//! Opt-in real-time HTTP service over the actual Rust WorldModel.
//!
//! Default bind is loopback only. Public binds require WANXIANG_API_TOKEN.
//! For public use, terminate TLS at a trusted reverse proxy and restrict CORS origins.
//! External LLM calls use curl's TLS stack; provider secrets remain server-side.

use agent_runtime::MemoryBackedLlmPlanner;
use llm_adapter::{
    BaseLLMAdapter, JsonTransport, LLMError, Message, OpenAiCompatibleAdapter,
    OpenAiCompatibleConfig,
};
use memory_core::{NmmUnified, SdrEncoder};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wanxiang_agent_bridge::{run_rule_experiment, WorldAgent};
use wanxiang_sdk::WorldModel;
use wanxiang_types::{WorldRules, WorldType};

const MAX_BODY_BYTES: usize = 128 * 1024;
const MAX_TASK_CHARS: usize = 4_000;
const CACHE_TTL_SECS: u64 = 600;
const MAX_CACHE_ITEMS: usize = 64;
const STATUS_MARKER: &str = "\n__WANXIANG_HTTP_STATUS__:";

type LiveAdapter = OpenAiCompatibleAdapter<CurlJsonTransport>;

#[derive(Clone)]
struct CachedResponse {
    created_at: u64,
    request_hash: String,
    status: u16,
    body: Value,
}

struct AppState {
    world: Arc<Mutex<WorldModel>>,
    agent: Option<Mutex<WorldAgent<LiveAdapter>>>,
    api_token: Option<String>,
    allowed_origins: Vec<String>,
    data_file: PathBuf,
    cache: Mutex<BTreeMap<String, CachedResponse>>,
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn env_u64(name: &str, default: u64, min: u64, max: u64) -> u64 {
    env::var(name).ok().and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default).clamp(min, max)
}

fn curl_quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

/// HTTPS-capable JSON transport implemented by the system curl binary.
/// Configuration and API keys are sent to curl over stdin, not exposed in argv.
/// Install curl + a current CA bundle on the host/container.
struct CurlJsonTransport;

impl JsonTransport for CurlJsonTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        body: &Value,
        timeout_ms: u64,
    ) -> Result<(u16, Value), LLMError> {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(LLMError::Network("provider endpoint must use http:// or https://".into()));
        }
        let body_json = serde_json::to_string(body)
            .map_err(|e| LLMError::Network(format!("cannot encode request: {e}")))?;
        let timeout_secs = (timeout_ms.max(1) as f64 / 1_000.0).min(120.0);
        let connect_secs = timeout_secs.min(10.0).max(1.0);
        let mut config = format!(
            "url = {}\nrequest = \"POST\"\nsilent\nshow-error\nmax-time = {:.3}\nconnect-timeout = {:.3}\nheader = \"content-type: application/json\"\ndata-binary = {}\nwrite-out = {}\n",
            curl_quote(url),
            timeout_secs,
            connect_secs,
            curl_quote(&body_json),
            curl_quote(&format!("{STATUS_MARKER}%{{http_code}}")),
        );
        for (name, value) in headers {
            // Header names are controlled by the adapter. Reject CR/LF injection in values.
            if value.contains('\r') || value.contains('\n') {
                return Err(LLMError::Network("invalid newline in HTTP header".into()));
            }
            config.push_str(&format!("header = {}\n", curl_quote(&format!("{name}: {value}"))));
        }

        let mut child = Command::new("curl")
            .args(["--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| LLMError::Network(format!("could not start curl (install curl): {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(config.as_bytes())
                .map_err(|e| LLMError::Network(format!("could not send provider request: {e}")))?;
        }
        let output = child.wait_with_output()
            .map_err(|e| LLMError::Network(format!("curl failed: {e}")))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some((response_body, status)) = stdout.rsplit_once(STATUS_MARKER) {
            let status = status.trim().parse::<u16>()
                .map_err(|_| LLMError::Network("provider HTTP status was not parseable".into()))?;
            let raw = serde_json::from_str::<Value>(response_body)
                .unwrap_or_else(|_| json!({"error":{"message":response_body.trim()}}));
            return Ok((status, raw));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.to_ascii_lowercase().contains("timed out") || stderr.to_ascii_lowercase().contains("timeout") {
            return Err(LLMError::Timeout);
        }
        Err(LLMError::Network(format!("curl exited without an HTTP response ({}): {}", output.status, stderr.trim())))
    }
}

fn default_origins() -> Vec<String> {
    env::var("WANXIANG_CORS_ORIGIN")
        .unwrap_or_else(|_| "https://daohan003.github.io,http://localhost:8000,http://127.0.0.1:8000,http://localhost:8080,http://127.0.0.1:8080,http://localhost:5500,http://127.0.0.1:5500".into())
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn read_persisted(path: &Path) -> Result<(WorldModel, BTreeMap<String, CachedResponse>), String> {
    if !path.exists() {
        return Ok((WorldModel::new(WorldRules::for_genre(WorldType::Xianxia), 42), BTreeMap::new()));
    }
    let raw = fs::read_to_string(path).map_err(|e| format!("cannot read checkpoint {}: {e}", path.display()))?;
    let value: Value = serde_json::from_str(&raw).map_err(|e| format!("checkpoint JSON is invalid: {e}"))?;
    let checkpoint = value.get("world_checkpoint").and_then(Value::as_str)
        .ok_or_else(|| "checkpoint is missing world_checkpoint; refusing to overwrite it".to_string())?;
    let world = WorldModel::from_checkpoint_json(checkpoint)
        .map_err(|e| format!("world checkpoint cannot be restored: {e}"))?;
    let mut cache = BTreeMap::new();
    if let Some(records) = value.get("idempotency").and_then(Value::as_object) {
        for (key, record) in records {
            let created_at = record.get("created_at").and_then(Value::as_u64).unwrap_or(0);
            if now_secs().saturating_sub(created_at) > CACHE_TTL_SECS { continue; }
            let status = record.get("status").and_then(Value::as_u64).unwrap_or(200) as u16;
            let request_hash = record.get("request_hash").and_then(Value::as_str).unwrap_or("").to_string();
            let body = record.get("body").cloned().unwrap_or(Value::Null);
            cache.insert(key.clone(), CachedResponse { created_at, request_hash, status, body });
        }
    }
    Ok((world, cache))
}

fn persist_state(state: &AppState, cache: &BTreeMap<String, CachedResponse>) -> Result<(), String> {
    let checkpoint = {
        let world = state.world.lock().map_err(|_| "world lock poisoned".to_string())?;
        world.checkpoint_json().map_err(|e| format!("checkpoint serialization failed: {e}"))?
    };
    let records: BTreeMap<String, Value> = cache.iter().map(|(key, record)| {
        (key.clone(), json!({
            "created_at": record.created_at,
            "request_hash": record.request_hash,
            "status": record.status,
            "body": record.body
        }))
    }).collect();
    let payload = json!({
        "schema_version": 1,
        "saved_at_unix": now_secs(),
        "world_checkpoint": checkpoint,
        "idempotency": records
    });
    let raw = serde_json::to_vec(&payload).map_err(|e| format!("checkpoint JSON encoding failed: {e}"))?;
    if let Some(parent) = state.data_file.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create live data directory: {e}"))?;
    }
    let temp = state.data_file.with_extension("json.tmp");
    fs::write(&temp, raw).map_err(|e| format!("cannot write checkpoint temp file: {e}"))?;
    fs::rename(&temp, &state.data_file).map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("cannot atomically replace checkpoint {}: {e}", state.data_file.display())
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, (u16, String)> {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buffer = Vec::new();
    let mut expected_total: Option<usize> = None;
    loop {
        if buffer.len() > MAX_BODY_BYTES + 16 * 1024 {
            return Err((413, "request body too large".into()));
        }
        if let Some(header_end) = find_header_end(&buffer) {
            if expected_total.is_none() {
                let header = String::from_utf8_lossy(&buffer[..header_end]);
                let mut lines = header.lines();
                let line = lines.next().ok_or((400, "missing HTTP request line".into()))?;
                let mut parts = line.split_whitespace();
                let method = parts.next().ok_or((400, "missing HTTP method".into()))?.to_string();
                let path = parts.next().ok_or((400, "missing request path".into()))?.to_string();
                let mut headers = HashMap::new();
                for line in lines {
                    if let Some((k, v)) = line.split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                if headers.get("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
                    return Err((400, "chunked request bodies are not supported".into()));
                }
                let content_length = headers.get("content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                if content_length > MAX_BODY_BYTES {
                    return Err((413, "request body too large".into()));
                }
                expected_total = Some(header_end + 4 + content_length);
                if buffer.len() >= expected_total.unwrap() {
                    let body = String::from_utf8(buffer[header_end + 4..header_end + 4 + content_length].to_vec())
                        .map_err(|_| (400, "request body must be UTF-8".into()))?;
                    return Ok(HttpRequest { method, path, headers, body });
                }
            } else if buffer.len() >= expected_total.unwrap() {
                let header = String::from_utf8_lossy(&buffer[..header_end]);
                let mut lines = header.lines();
                let line = lines.next().ok_or((400, "missing HTTP request line".into()))?;
                let mut parts = line.split_whitespace();
                let method = parts.next().ok_or((400, "missing HTTP method".into()))?.to_string();
                let path = parts.next().ok_or((400, "missing request path".into()))?.to_string();
                let headers: HashMap<String, String> = lines.filter_map(|line| line.split_once(':'))
                    .map(|(k,v)|(k.trim().to_ascii_lowercase(),v.trim().to_string())).collect();
                let body_len = expected_total.unwrap() - header_end - 4;
                let body = String::from_utf8(buffer[header_end + 4..header_end + 4 + body_len].to_vec())
                    .map_err(|_| (400, "request body must be UTF-8".into()))?;
                return Ok(HttpRequest { method, path, headers, body });
            }
        }
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).map_err(|e| (400, format!("request read failed: {e}")))?;
        if n == 0 { return Err((400, "connection closed before request completed".into())); }
        buffer.extend_from_slice(&chunk[..n]);
    }
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK", 201 => "Created", 204 => "No Content", 400 => "Bad Request",
        401 => "Unauthorized", 403 => "Forbidden", 404 => "Not Found",
        409 => "Conflict", 413 => "Payload Too Large", 429 => "Too Many Requests",
        500 => "Internal Server Error", 503 => "Service Unavailable", _ => "OK"
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &Value, origin: Option<&str>, allowed: bool) {
    let raw = serde_json::to_vec(body).unwrap_or_else(|_| b"{\"error\":\"response serialization failed\"}".to_vec());
    let cors = if allowed {
        origin.map(|o| format!(
            "Access-Control-Allow-Origin: {o}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Idempotency-Key\r\nAccess-Control-Max-Age: 600\r\n"
        )).unwrap_or_default()
    } else { String::new() };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n{}\r\n",
        status, status_text(status), raw.len(), cors
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&raw);
    let _ = stream.flush();
}

fn request_hash(request: &HttpRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request.method.as_bytes());
    hasher.update(b"\n");
    hasher.update(request.path.as_bytes());
    hasher.update(b"\n");
    hasher.update(request.body.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn prune_cache(cache: &mut BTreeMap<String, CachedResponse>) {
    let now = now_secs();
    cache.retain(|_, record| now.saturating_sub(record.created_at) <= CACHE_TTL_SECS);
    while cache.len() > MAX_CACHE_ITEMS {
        if let Some(oldest) = cache.iter().min_by_key(|(_, r)| r.created_at).map(|(k,_)| k.clone()) {
            cache.remove(&oldest);
        } else { break; }
    }
}

fn state_payload(world: &WorldModel) -> Value {
    json!({
        "kind": "state",
        "backend": "wanxiang_sdk::WorldModel",
        "info": serde_json::from_str::<Value>(&world.info_json()).unwrap_or(Value::Null),
        "snapshot": serde_json::from_str::<Value>(&world.snapshot_json()).unwrap_or(Value::Null),
        "server_time_unix": now_secs(),
        "persistent_checkpoint": true
    })
}

fn route_request(state: &AppState, request: &HttpRequest) -> (u16, Value) {
    let path = request.path.split('?').next().unwrap_or(&request.path);
    match (request.method.as_str(), path) {
        ("GET", "/health") | ("GET", "/api/health") => (
            200,
            json!({
                "status":"ok",
                "service":"wanxiang-live",
                "backend":"wanxiang_sdk::WorldModel",
                "llm_configured":state.agent.is_some(),
                "api_version":1
            }),
        ),
        ("GET", "/api/state") => match state.world.lock() {
            Ok(world) => (200, state_payload(&world)),
            Err(_) => (500, json!({"error":"world lock poisoned"})),
        },
        ("POST", "/api/step") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let ticks = match args.get("ticks").and_then(Value::as_u64) {
                Some(ticks) if (1..=100).contains(&ticks) => ticks,
                _ => return (400, json!({"error":"ticks must be an integer in 1..=100; invalid input does not advance the world"})),
            };
            match state.world.lock() {
                Ok(mut world) => {
                    world.step(ticks);
                    let payload = state_payload(&world);
                    (200, json!({
                        "kind":"step",
                        "advanced_ticks":ticks,
                        "tick":world.current_tick(),
                        "info":payload["info"],
                        "snapshot":payload["snapshot"],
                        "server_time_unix":payload["server_time_unix"],
                        "persistent_checkpoint":true
                    }))
                }
                Err(_) => (500, json!({"error":"world lock poisoned"})),
            }
        }
        ("POST", "/api/experiment") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let rule = match args.get("rule").and_then(Value::as_str) {
                Some(v) => v,
                None => return (400, json!({"error":"rule must be a string"})),
            };
            let value = match args.get("value").and_then(Value::as_f64) {
                Some(v) => v,
                None => return (400, json!({"error":"value must be numeric"})),
            };
            let ticks = match args.get("ticks").and_then(Value::as_u64) {
                Some(v) => v,
                None => return (400, json!({"error":"ticks must be an integer"})),
            };
            match state.world.lock() {
                Ok(world) => match run_rule_experiment(&world, rule, value, ticks) {
                    Ok(result) => (200, result),
                    Err(error) => (400, json!({"error":error})),
                },
                Err(_) => (500, json!({"error":"world lock poisoned"})),
            }
        }
        ("POST", "/api/agent") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let task = match args.get("task").and_then(Value::as_str) {
                Some(task) if !task.trim().is_empty() && task.chars().count() <= MAX_TASK_CHARS => task,
                _ => return (400, json!({"error":format!("task must be a non-empty string of at most {MAX_TASK_CHARS} characters")})),
            };
            let agent_mutex = match &state.agent {
                Some(agent) => agent,
                None => return (503, json!({"error":"LLM agent is disabled; configure WANXIANG_LLM_ENDPOINT and WANXIANG_LLM_MODEL on the server"})),
            };
            match agent_mutex.lock() {
                Ok(mut agent) => {
                    let result = agent.run(task);
                    let steps: Vec<Value> = result.steps.iter().map(|s| json!({
                        "step_num":s.step_num,
                        "action":s.action,
                        "action_input":s.action_input,
                        "observation":s.observation,
                        "success":s.success,
                        "duration_ms":s.duration_ms
                    })).collect();
                    let world = match state.world.lock() {
                        Ok(world) => state_payload(&world),
                        Err(_) => json!({"error":"world lock poisoned"}),
                    };
                    (200, json!({
                        "kind":"agent_result",
                        "success":result.success,
                        "answer":result.answer,
                        "error":result.error,
                        "steps":steps,
                        "world":world,
                        "note":"The model's internal reasoning is not returned; tool inputs and observations are shown for audit."
                    }))
                }
                Err(_) => (500, json!({"error":"agent lock poisoned"})),
            }
        }
        ("OPTIONS", _) => (204, json!({"ok":true})),
        _ => (404, json!({"error":"not found","routes":["GET /health","GET /api/state","POST /api/step","POST /api/experiment","POST /api/agent"]})),
    }
}

fn handle_connection(mut stream: TcpStream, state: Arc<AppState>) {
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err((status, message)) => {
            respond(&mut stream, status, &json!({"error":message}), None, false);
            return;
        }
    };
    let origin = request.headers.get("origin").map(String::as_str);
    let allowed = origin.map(|o| state.allowed_origins.iter().any(|a| a == o)).unwrap_or(true);
    if !allowed {
        respond(&mut stream, 403, &json!({"error":"origin is not allowed by WANXIANG_CORS_ORIGIN"}), origin, false);
        return;
    }

    if request.method == "OPTIONS" {
        respond(&mut stream, 204, &json!({"ok":true}), origin, true);
        return;
    }
    if let Some(expected) = &state.api_token {
        let supplied = request.headers.get("authorization").map(String::as_str).unwrap_or("");
        let expected = format!("Bearer {expected}");
        if supplied != expected {
            respond(&mut stream, 401, &json!({"error":"missing or invalid bearer token"}), origin, allowed);
            return;
        }
    }

    let is_mutation = request.method == "POST"
        && matches!(request.path.split('?').next().unwrap_or(&request.path), "/api/step" | "/api/experiment" | "/api/agent");
    if !is_mutation {
        let (status, body) = route_request(&state, &request);
        respond(&mut stream, status, &body, origin, allowed);
        return;
    }

    // Serialize mutations, checkpoint writes, and idempotency-cache updates under one lock.
    let mut cache = match state.cache.lock() {
        Ok(cache) => cache,
        Err(_) => {
            respond(&mut stream, 500, &json!({"error":"idempotency lock poisoned"}), origin, allowed);
            return;
        }
    };
    prune_cache(&mut cache);
    let key = request.headers.get("idempotency-key").map(|s| s.trim()).filter(|s| !s.is_empty());
    if key.is_some_and(|key| key.len() > 128 || key.chars().any(char::is_control)) {
        respond(&mut stream, 400, &json!({"error":"Idempotency-Key must be at most 128 visible characters"}), origin, allowed);
        return;
    }
    let hash = request_hash(&request);
    if let Some(key) = key {
        if let Some(previous) = cache.get(key) {
            if previous.request_hash != hash {
                respond(&mut stream, 409, &json!({"error":"Idempotency-Key was already used with a different request"}), origin, allowed);
                return;
            }
            respond(&mut stream, previous.status, &previous.body, origin, allowed);
            return;
        }
    }

    let (mut status, mut body) = route_request(&state, &request);
    if status < 400 {
        if let Some(key) = key {
            cache.insert(key.to_string(), CachedResponse {
                created_at: now_secs(),
                request_hash: hash,
                status,
                body: body.clone(),
            });
            prune_cache(&mut cache);
        }
        if let Err(error) = persist_state(&state, &cache) {
            status = 500;
            body = json!({
                "error":"world operation completed in memory but durable checkpoint failed",
                "detail":error,
                "retry":"Check the checkpoint path before retrying; use the same Idempotency-Key to avoid re-executing a cached result within this process."
            });
        }
    }
    respond(&mut stream, status, &body, origin, allowed);
}

fn main() {
    let bind = env::var("WANXIANG_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let address: SocketAddr = bind.parse().expect("WANXIANG_BIND must be an IP:PORT socket address");
    let api_token = env::var("WANXIANG_API_TOKEN").ok().filter(|s| !s.trim().is_empty());
    if !address.ip().is_loopback() && api_token.is_none() {
        eprintln!("Refusing to bind a public interface without WANXIANG_API_TOKEN. Use 127.0.0.1:8787 or set a strong token.");
        std::process::exit(2);
    }

    let data_dir = env::var("WANXIANG_DATA_DIR").unwrap_or_else(|_| ".wanxiang-live-data".into());
    let data_file = PathBuf::from(data_dir).join("runtime-state.json");
    let (world, cache) = match read_persisted(&data_file) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let world = Arc::new(Mutex::new(world));

    let agent = match (env::var("WANXIANG_LLM_ENDPOINT").ok(), env::var("WANXIANG_LLM_MODEL").ok()) {
        (Some(endpoint), Some(model)) if !endpoint.trim().is_empty() && !model.trim().is_empty() => {
            let mut config = OpenAiCompatibleConfig::new(endpoint, model);
            config.timeout_ms = env_u64("WANXIANG_LLM_TIMEOUT_MS", 30_000, 1_000, 120_000);
            if let Ok(key) = env::var("WANXIANG_LLM_API_KEY") {
                if !key.is_empty() { config = config.with_api_key(key); }
            }
            let adapter = OpenAiCompatibleAdapter::new(config, CurlJsonTransport);
            let planner = MemoryBackedLlmPlanner::new(adapter, NmmUnified::new(SdrEncoder::default()))
                .with_retrieval(5, 0.0, true)
                .with_memory_limit(2_048);
            Some(Mutex::new(WorldAgent::with_shared_world(Arc::clone(&world), planner, 8)))
        }
        _ => None,
    };

    let state = Arc::new(AppState {
        world,
        agent,
        api_token,
        allowed_origins: default_origins(),
        data_file,
        cache: Mutex::new(cache),
    });
    if env::var("WANXIANG_LLM_ENDPOINT").is_ok() && state.agent.is_none() {
        eprintln!("LLM was not enabled: set both WANXIANG_LLM_ENDPOINT and WANXIANG_LLM_MODEL.");
    }
    let listener = TcpListener::bind(address).unwrap_or_else(|e| {
        eprintln!("cannot bind {address}: {e}");
        std::process::exit(2);
    });
    println!("Wanxiang live API listening at http://{address}");
    println!("Routes: GET /health, GET /api/state, POST /api/step, POST /api/experiment, POST /api/agent");
    println!("Persistent checkpoint: {}", state.data_file.display());
    if state.agent.is_some() {
        println!("LLM agent enabled; provider key stays on this server and curl provides HTTPS transport.");
    } else {
        println!("LLM agent disabled; world simulation and deterministic experiments work without a provider.");
    }
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || handle_connection(stream, state));
            }
            Err(error) => eprintln!("incoming connection failed: {error}"),
        }
    }
}
