//! Opt-in real-time HTTP service over the actual Rust WorldModel.
//!
//! Default bind is loopback only. Public binds require WANXIANG_API_TOKEN.
//! For public use, terminate TLS at a trusted reverse proxy and restrict CORS origins.
//! External LLM calls use curl's TLS stack; provider secrets remain server-side.

use agent_runtime::{ApprovalDecision, MemoryBackedLlmPlanner, ResumableSession, SessionStatus};
use llm_adapter::{
    authorization_headers, JsonTransport, LLMError, Message,
    OpenAiCompatibleAdapter, OpenAiCompatibleConfig, OpenAiCompatibleRequest,
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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wanxiang_agent_bridge::{run_rule_experiment, WorldAgent};
use wanxiang_sdk::{GenrePreset, MarketTradeSide, WorldModel};
use wanxiang_types::{WorldRules, WorldType};

const MAX_BODY_BYTES: usize = 128 * 1024;
const MAX_ACTIVE_CONNECTIONS: usize = 64;
const MAX_TASK_CHARS: usize = 4_000;
const CACHE_TTL_SECS: u64 = 600;
const MAX_CACHE_ITEMS: usize = 64;
const MAX_PERSISTED_SESSIONS: usize = 256;
const SESSION_TTL_SECS: u64 = 24 * 60 * 60;
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(0);
const STATUS_MARKER: &str = "__WANXIANG_HTTP_STATUS__:";

type LiveAdapter = OpenAiCompatibleAdapter<CurlJsonTransport>;

#[derive(Clone)]
struct CachedResponse {
    created_at: u64,
    request_hash: String,
    status: u16,
    body: Value,
}

#[derive(Debug, Clone)]
struct StoredSession {
    updated_at: u64,
    session: ResumableSession,
}

struct AppState {
    world: Arc<Mutex<WorldModel>>,
    agent: Option<Mutex<WorldAgent<LiveAdapter>>>,
    api_token: Option<String>,
    allowed_origins: Vec<String>,
    data_file: PathBuf,
    memory_file: PathBuf,
    sessions_file: PathBuf,
    sessions: Mutex<BTreeMap<String, StoredSession>>,
    cache: Mutex<BTreeMap<String, CachedResponse>>,
    slow_mutation_gate: Mutex<()>,
    stream_gate: Mutex<()>,
    world_checkpoint_degraded: AtomicBool,
    agent_memory_degraded: AtomicBool,
    sessions_checkpoint_degraded: AtomicBool,
    auto_tick_ms: u64,
    agent_retrieval_mode: String,
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

/// Validate provider URLs before sending a configured API key.
/// Public providers must use HTTPS; plaintext HTTP is reserved for loopback development.
fn endpoint_authority<'a>(endpoint: &'a str, scheme: &str) -> Option<&'a str> {
    let rest = endpoint.strip_prefix(scheme)?;
    let authority = rest.split(|ch| ch == '/' || ch == '?' || ch == '#').next()?;
    if authority.is_empty()
        || authority.contains('@')
        || authority.chars().any(char::is_control)
        || authority.contains(' ')
    {
        return None;
    }
    Some(authority)
}

fn endpoint_host(authority: &str) -> Option<&str> {
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let suffix = &authority[end + 1..];
        if !suffix.is_empty() && !suffix.starts_with(':') {
            return None;
        }
        let host = &authority[1..end];
        return (!host.is_empty()).then_some(host);
    }
    if authority.matches(':').count() > 1 {
        return None; // IPv6 literals must be bracketed in URLs.
    }
    let host = authority.split_once(':').map_or(authority, |(host, _)| host);
    (!host.is_empty()).then_some(host)
}

fn validate_provider_endpoint(endpoint: &str) -> Result<(), String> {
    if endpoint.trim() != endpoint || endpoint.chars().any(char::is_control) {
        return Err("provider endpoint contains leading/trailing whitespace or control characters".into());
    }
    if let Some(authority) = endpoint_authority(endpoint, "https://") {
        return endpoint_host(authority)
            .map(|_| ())
            .ok_or_else(|| "provider endpoint has an invalid hostname".into());
    }
    if let Some(authority) = endpoint_authority(endpoint, "http://") {
        let host = endpoint_host(authority)
            .ok_or_else(|| "provider endpoint has an invalid hostname".to_string())?;
        let loopback_name = host.eq_ignore_ascii_case("localhost");
        let loopback_ip = host.parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
        if loopback_name || loopback_ip {
            return Ok(());
        }
        return Err("provider API keys may only be sent over HTTPS; HTTP is allowed only for loopback development endpoints".into());
    }
    Err("provider endpoint must start with https:// (or loopback http:// for development)".into())
}


impl CurlJsonTransport {
    /// True streaming transport used by both the adapter's SSE contract and the live HTTP route.
    fn stream_chat<F>(
        url: &str,
        headers: &BTreeMap<String, String>,
        body: &Value,
        timeout_ms: u64,
        mut on_line: F,
    ) -> Result<u16, LLMError>
    where
        F: FnMut(&str) -> Result<(), LLMError>,
    {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(LLMError::Network("provider endpoint must use http:// or https://".into()));
        }
        let body_json = serde_json::to_string(body)
            .map_err(|e| LLMError::Network(format!("cannot encode stream request: {e}")))?;
        let timeout_secs = (timeout_ms.max(1) as f64 / 1_000.0).min(120.0);
        let connect_secs = timeout_secs.min(10.0).max(1.0);
        let mut config = format!(
            "url = {}\nrequest = \"POST\"\nsilent\nshow-error\nmax-time = {:.3}\nconnect-timeout = {:.3}\nheader = \"content-type: application/json\"\ndata-binary = {}\nwrite-out = {}\n",
            curl_quote(url), timeout_secs, connect_secs, curl_quote(&body_json),
            curl_quote(&format!("\n{STATUS_MARKER}%{{http_code}}\n")),
        );
        for (name, value) in headers {
            if value.contains('\r') || value.contains('\n') {
                return Err(LLMError::Network("invalid newline in HTTP header".into()));
            }
            config.push_str(&format!("header = {}\n", curl_quote(&format!("{name}: {value}"))));
        }
        let mut child = Command::new("curl")
            .args(["--disable", "--no-buffer", "--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| LLMError::Network(format!("could not start curl for SSE (install curl): {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(config.as_bytes())
                .map_err(|e| LLMError::Network(format!("could not send SSE request: {e}")))?;
        }
        let stdout = child.stdout.take()
            .ok_or_else(|| LLMError::Network("curl SSE stdout is unavailable".into()))?;
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let mut status = None;
        loop {
            line.clear();
            let read = std::io::BufRead::read_line(&mut reader, &mut line)
                .map_err(|e| LLMError::Network(format!("could not read SSE response: {e}")))?;
            if read == 0 { break; }
            let current = line.trim_end_matches(&['\r', '\n'][..]);
            if let Some(code) = current.strip_prefix(STATUS_MARKER) {
                status = code.trim().parse::<u16>().ok();
                break;
            }
            if let Err(error) = on_line(current) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        let exit = child.wait()
            .map_err(|e| LLMError::Network(format!("curl SSE process failed: {e}")))?;
        match status {
            Some(status) => Ok(status),
            None => Err(LLMError::Network(format!("curl ended without an HTTP status (exit code {})", exit.code().unwrap_or(-1)))),
        }
    }

}
 
fn stream_delta(line: &str) -> Result<Option<String>, String> {
        let Some(data) = line.strip_prefix("data:") else { return Ok(None) };
        let data = data.trim();
        if data == "[DONE]" { return Ok(None); }
        let value: Value = serde_json::from_str(data).map_err(|e| format!("malformed SSE data: {e}"))?;
        if let Some(error) = value.get("error") {
            return Err(error.get("message").and_then(Value::as_str).unwrap_or("provider stream error").to_string());
        }
        Ok(value.get("choices")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string))
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
            curl_quote(&format!("\n{STATUS_MARKER}%{{http_code}}\n")),
        );
        for (name, value) in headers {
            // Header names are controlled by the adapter. Reject CR/LF injection in values.
            if value.contains('\r') || value.contains('\n') {
                return Err(LLMError::Network("invalid newline in HTTP header".into()));
            }
            config.push_str(&format!("header = {}\n", curl_quote(&format!("{name}: {value}"))));
        }

        let mut child = Command::new("curl")
            .args(["--disable", "--retry", "2", "--retry-delay", "1", "--retry-max-time", "15", "--retry-connrefused", "--config", "-"])
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

    fn post_sse(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        body: &Value,
        timeout_ms: u64,
    ) -> Result<(u16, String), LLMError> {
        let mut raw = String::new();
        let status = Self::stream_chat(url, headers, body, timeout_ms, |line| {
            raw.push_str(line);
            raw.push('\n');
            Ok(())
        })?;
        Ok((status, raw))
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


fn prune_sessions(sessions: &mut BTreeMap<String, StoredSession>) {
    let now = now_secs();
    sessions.retain(|_, entry| now.saturating_sub(entry.updated_at) <= SESSION_TTL_SECS);
    while sessions.len() > MAX_PERSISTED_SESSIONS {
        if let Some(oldest) = sessions.iter()
            .min_by_key(|(_, entry)| entry.updated_at)
            .map(|(id, _)| id.clone())
        {
            sessions.remove(&oldest);
        } else {
            break;
        }
    }
}

fn read_sessions(path: &Path) -> Result<BTreeMap<String, StoredSession>, String> {
    if !path.exists() { return Ok(BTreeMap::new()); }
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("cannot read persisted sessions {}: {e}", path.display()))?;
    let records = serde_json::from_str::<BTreeMap<String, Value>>(&raw)
        .map_err(|e| format!("persisted session JSON is invalid: {e}"))?;
    let mut sessions = BTreeMap::new();
    for (id, record) in records {
        let updated_at = record.get("updated_at").and_then(Value::as_u64).unwrap_or(0);
        let session_value = record.get("session").cloned()
            .ok_or_else(|| format!("persisted session {id} is missing its session field"))?;
        let session = serde_json::from_value::<ResumableSession>(session_value)
            .map_err(|e| format!("persisted session {id} cannot be restored: {e}"))?;
        sessions.insert(id, StoredSession { updated_at, session });
    }
    prune_sessions(&mut sessions);
    Ok(sessions)
}

fn persist_sessions_to(path: &Path, sessions: &BTreeMap<String, StoredSession>) -> Result<(), String> {
    let records: BTreeMap<String, Value> = sessions.iter().map(|(id, entry)| {
        (id.clone(), json!({"updated_at":entry.updated_at,"session":entry.session}))
    }).collect();
    let raw = serde_json::to_vec(&records)
        .map_err(|e| format!("session checkpoint serialization failed: {e}"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create session checkpoint directory: {e}"))?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, raw).map_err(|e| format!("cannot write session checkpoint temp file: {e}"))?;
    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("cannot atomically replace session checkpoint {}: {e}", path.display())
    })
}

fn persist_sessions(state: &AppState, sessions: &BTreeMap<String, StoredSession>) -> Result<(), String> {
    let result = persist_sessions_to(&state.sessions_file, sessions);
    state.sessions_checkpoint_degraded.store(result.is_err(), Ordering::Release);
    result
}

fn session_payload(entry: &StoredSession) -> Value {
    let session = &entry.session;
    let status = if session.finished {
        "finished"
    } else if session.pending_approval.is_some() {
        "awaiting_approval"
    } else {
        "ready"
    };
    json!({
        "session_id": session.session_id,
        "status": status,
        "updated_at": entry.updated_at,
        "user_input": session.user_input,
        "history": session.history,
        "pending_approval": session.pending_approval,
        "finished": session.finished,
        "success": session.success,
        "error": session.error,
        "max_steps": session.max_steps
    })
}

fn session_outcome(status: &SessionStatus) -> Value {
    match status {
        SessionStatus::Running => json!({"status":"running"}),
        SessionStatus::AwaitingApproval(pending) => json!({"status":"awaiting_approval","pending_approval":pending}),
        SessionStatus::Finished { success, answer } => json!({"status":"finished","success":success,"answer":answer}),
    }
}

fn session_id_for_route(path: &str, drive: bool) -> Option<&str> {
    let rest = path.strip_prefix("/api/sessions/")?;
    let id = if drive { rest.strip_suffix("/drive")? } else { rest };
    if id.is_empty() || id.contains('/') || id.chars().any(char::is_control) {
        None
    } else {
        Some(id)
    }
}


/// Parse a configured world genre. Missing configuration intentionally means a neutral world.
fn parse_world_type(value: Option<&str>) -> Result<WorldType, String> {
    let Some(raw) = value.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(WorldType::Custom);
    };
    match raw.to_ascii_lowercase().as_str() {
        "custom" | "generic" | "neutral" | "自定义" | "通用" | "中性" => Ok(WorldType::Custom),
        "xianxia" | "cultivation" | "仙侠" => Ok(WorldType::Xianxia),
        "historical" | "history" | "历史" => Ok(WorldType::Historical),
        "scifi" | "sci-fi" | "science-fiction" | "科幻" => Ok(WorldType::Scifi),
        "urban" | "city" | "都市" => Ok(WorldType::Urban),
        "western" | "western_fantasy" | "fantasy" | "西方奇幻" => Ok(WorldType::Western),
        _ => Err(format!(
            "unsupported WANXIANG_WORLD_TYPE '{raw}'; choose custom, xianxia, historical, scifi, urban, or western"
        )),
    }
}

/// Select a first-boot world from explicit configuration. An authored preset overrides genre generation.
fn initial_world_for_settings(
    world_type: Option<&str>,
    preset_id: Option<&str>,
    seed: u64,
) -> Result<WorldModel, String> {
    if let Some(id) = preset_id.map(str::trim).filter(|id| !id.is_empty()) {
        if id.eq_ignore_ascii_case("generic") || id.eq_ignore_ascii_case("custom") || id.eq_ignore_ascii_case("none") {
            let kind = parse_world_type(world_type)?;
            return Ok(WorldModel::new(WorldRules::for_genre(kind), seed));
        }
        let preset = GenrePreset::registry().into_iter().find(|preset| {
            preset.genre_id.eq_ignore_ascii_case(id) || preset.title.eq_ignore_ascii_case(id)
        }).ok_or_else(|| format!(
            "unknown WANXIANG_WORLD_PRESET '{id}'; available presets: {}",
            GenrePreset::registry().iter().map(|preset| preset.genre_id.clone()).collect::<Vec<_>>().join(", ")
        ))?;
        return Ok(WorldModel::from_preset(preset, seed));
    }
    let kind = parse_world_type(world_type)?;
    Ok(WorldModel::new(WorldRules::for_genre(kind), seed))
}

fn initial_world_from_env() -> Result<WorldModel, String> {
    let world_type = env::var("WANXIANG_WORLD_TYPE").ok();
    let preset_id = env::var("WANXIANG_WORLD_PRESET").ok();
    let seed = match env::var("WANXIANG_WORLD_SEED") {
        Ok(raw) => raw.parse::<u64>().map_err(|_| {
            format!("WANXIANG_WORLD_SEED must be an unsigned 64-bit integer, got '{raw}'")
        })?,
        Err(_) => 42,
    };
    initial_world_for_settings(world_type.as_deref(), preset_id.as_deref(), seed)
}

fn read_persisted(path: &Path) -> Result<(WorldModel, BTreeMap<String, CachedResponse>), String> {
    if !path.exists() {
        return Ok((initial_world_from_env()?, BTreeMap::new()));
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
            // 425 is an in-memory in-flight marker and must never survive a restart.
            if status == 425 { continue; }
            let request_hash = record.get("request_hash").and_then(Value::as_str).unwrap_or("").to_string();
            let body = record.get("body").cloned().unwrap_or(Value::Null);
            cache.insert(key.clone(), CachedResponse { created_at, request_hash, status, body });
        }
    }
    Ok((world, cache))
}


fn read_agent_memory(path: &Path) -> Result<Option<NmmUnified>, String> {
    if !path.exists() { return Ok(None); }
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("cannot read Agent memory {}: {e}", path.display()))?;
    serde_json::from_str::<NmmUnified>(&raw)
        .map(Some)
        .map_err(|e| format!("Agent memory checkpoint cannot be restored: {e}"))
}

fn persist_agent_memory(state: &AppState) -> Result<(), String> {
    let result = persist_agent_memory_inner(state);
    state.agent_memory_degraded.store(result.is_err(), Ordering::Release);
    result
}

fn persist_agent_memory_inner(state: &AppState) -> Result<(), String> {
    let Some(agent) = &state.agent else { return Ok(()); };
    let guard = agent.lock().map_err(|_| "agent lock poisoned while saving memory".to_string())?;
    let bytes = serde_json::to_vec(guard.planner().memory())
        .map_err(|e| format!("Agent memory serialization failed: {e}"))?;
    if let Some(parent) = state.memory_file.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create Agent memory directory: {e}"))?;
    }
    let temp = state.memory_file.with_extension("json.tmp");
    fs::write(&temp, bytes).map_err(|e| format!("cannot write Agent memory temp file: {e}"))?;
    fs::rename(&temp, &state.memory_file).map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("cannot atomically replace Agent memory {}: {e}", state.memory_file.display())
    })
}

fn persist_state(state: &AppState, cache: &BTreeMap<String, CachedResponse>) -> Result<(), String> {
    let result = persist_state_inner(state, cache);
    state.world_checkpoint_degraded.store(result.is_err(), Ordering::Release);
    result
}

fn persist_state_inner(state: &AppState, cache: &BTreeMap<String, CachedResponse>) -> Result<(), String> {
    let checkpoint = {
        let world = state.world.lock().map_err(|_| "world lock poisoned".to_string())?;
        world.checkpoint_json().map_err(|e| format!("checkpoint serialization failed: {e}"))?
    };
    let records: BTreeMap<String, Value> = cache.iter()
        .filter(|(_, record)| record.status != 425)
        .map(|(key, record)| {
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


fn advance_background_tick(state: &AppState) {
    // All writes follow the same lock order: idempotency cache, then world, then checkpoint.
    let mut cache = match state.cache.lock() {
        Ok(cache) => cache,
        Err(_) => {
            eprintln!("background tick skipped: idempotency lock poisoned");
            return;
        }
    };
    prune_cache(&mut cache);
    match state.world.lock() {
        Ok(mut world) => world.step(1),
        Err(_) => {
            eprintln!("background tick skipped: world lock poisoned");
            return;
        }
    }
    if let Err(error) = persist_state(state, &cache) {
        eprintln!("world advanced, but background checkpoint failed: {error}");
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

#[derive(Debug)]
struct ParsedRequestHead {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    content_length: usize,
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
        })
}

fn parse_request_head(header: &[u8]) -> Result<ParsedRequestHead, (u16, String)> {
    let text = std::str::from_utf8(header)
        .map_err(|_| (400, "HTTP headers must be valid UTF-8".to_string()))?;
    let lines: Vec<&str> = text.split("\r\n").collect();
    if lines.iter().any(|line| line.contains('\r') || line.contains('\n')) {
        return Err((400, "HTTP header lines must use CRLF separators".into()));
    }
    let request_line = lines.first().copied().filter(|line| !line.is_empty())
        .ok_or((400, "missing HTTP request line".to_string()))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or((400, "missing HTTP method".to_string()))?;
    let path = parts.next().ok_or((400, "missing request path".to_string()))?;
    let version = parts.next().ok_or((400, "missing HTTP version".to_string()))?;
    if parts.next().is_some() || method.bytes().any(|b| !b.is_ascii_alphabetic()) {
        return Err((400, "malformed HTTP request line".into()));
    }
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err((505, "only HTTP/1.0 and HTTP/1.1 are supported".into()));
    }
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err((400, "invalid request target".into()));
    }

    let mut headers = HashMap::new();
    for line in lines.iter().skip(1) {
        if line.is_empty() {
            return Err((400, "unexpected empty HTTP header line".into()));
        }
        let (name, value) = line.split_once(':')
            .ok_or((400, "malformed HTTP header field".to_string()))?;
        if !valid_header_name(name) {
            return Err((400, "invalid HTTP header name".into()));
        }
        let key = name.to_ascii_lowercase();
        if headers.contains_key(&key) {
            return Err((400, format!("duplicate HTTP header is not supported: {key}")));
        }
        let value = value.trim();
        if value.chars().any(|ch| ch == '\r' || ch == '\n' || ch == '\0') {
            return Err((400, "invalid control character in HTTP header".into()));
        }
        headers.insert(key, value.to_string());
    }

    if headers.contains_key("transfer-encoding") {
        return Err((400, "Transfer-Encoding is not supported; send a Content-Length body".into()));
    }
    let content_length = match headers.get("content-length") {
        None => 0,
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse::<usize>().map_err(|_| (400, "invalid Content-Length".into()))?
        }
        Some(_) => return Err((400, "invalid Content-Length".into())),
    };
    if content_length > MAX_BODY_BYTES {
        return Err((413, "request body too large".into()));
    }

    Ok(ParsedRequestHead {
        method: method.to_string(),
        path: path.to_string(),
        headers,
        content_length,
    })
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, (u16, String)> {
    const MAX_HEADER_BYTES: usize = 16 * 1024;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buffer = Vec::new();
    let header_end = loop {
        if let Some(end) = find_header_end(&buffer) {
            if end > MAX_HEADER_BYTES {
                return Err((431, "HTTP headers exceed the size limit".into()));
            }
            break end;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err((431, "HTTP headers exceed the size limit".into()));
        }
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk)
            .map_err(|e| (400, format!("request read failed: {e}")))?;
        if n == 0 {
            return Err((400, "connection closed before HTTP headers completed".into()));
        }
        buffer.extend_from_slice(&chunk[..n]);
    };

    let parsed = parse_request_head(&buffer[..header_end])?;
    let body_start = header_end + 4;
    let total = body_start + parsed.content_length;
    if buffer.len() > total {
        return Err((400, "unexpected bytes after the HTTP request body".into()));
    }
    while buffer.len() < total {
        let remaining = total - buffer.len();
        let mut chunk = [0u8; 8192];
        let read_limit = remaining.min(chunk.len());
        let n = stream.read(&mut chunk[..read_limit])
            .map_err(|e| (400, format!("request body read failed: {e}")))?;
        if n == 0 {
            return Err((400, "connection closed before request body completed".into()));
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8(buffer[body_start..total].to_vec())
        .map_err(|_| (400, "request body must be UTF-8".into()))?;
    Ok(HttpRequest {
        method: parsed.method,
        path: parsed.path,
        headers: parsed.headers,
        body,
    })
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK", 201 => "Created", 204 => "No Content", 400 => "Bad Request",
        401 => "Unauthorized", 403 => "Forbidden", 404 => "Not Found",
        409 => "Conflict", 413 => "Payload Too Large", 429 => "Too Many Requests",
        431 => "Request Header Fields Too Large", 505 => "HTTP Version Not Supported",
        500 => "Internal Server Error", 503 => "Service Unavailable", _ => "OK"
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &Value, origin: Option<&str>, allowed: bool) {
    let raw = if status == 204 {
        Vec::new()
    } else {
        serde_json::to_vec(body).unwrap_or_else(|_| b"{\"error\":\"response serialization failed\"}".to_vec())
    };
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


const MAX_RESPONSE_EVENTS: usize = 200;
const MAX_RESPONSE_ENTITIES: usize = 500;
const MAX_RESPONSE_FACTIONS: usize = 100;

fn truncate_tail(value: &mut Value, keep: usize) {
    if let Some(items) = value.as_array_mut() {
        if items.len() > keep {
            let tail = items.split_off(items.len() - keep);
            *items = tail;
        }
    }
}

fn compact_live_snapshot(mut snapshot: Value) -> Value {
    let total_events = snapshot.get("events").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    let (total_entities, alive_entities) = snapshot.get("entities").and_then(Value::as_array)
        .map(|items| (items.len(), items.iter().filter(|e| e.get("alive").and_then(Value::as_bool).unwrap_or(false)).count()))
        .unwrap_or((0, 0));
    let total_factions = snapshot.get("factions").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    truncate_tail(&mut snapshot["events"], MAX_RESPONSE_EVENTS);
    truncate_tail(&mut snapshot["entities"], MAX_RESPONSE_ENTITIES);
    truncate_tail(&mut snapshot["factions"], MAX_RESPONSE_FACTIONS);
    if !snapshot.get("meta").is_some_and(Value::is_object) {
        snapshot["meta"] = json!({});
    }
    snapshot["meta"]["total_events"] = json!(total_events);
    snapshot["meta"]["event_window_start"] = json!(total_events.saturating_sub(MAX_RESPONSE_EVENTS));
    snapshot["meta"]["events_truncated"] = json!(total_events > MAX_RESPONSE_EVENTS);
    snapshot["meta"]["total_entities"] = json!(total_entities);
    snapshot["meta"]["alive_entities"] = json!(alive_entities);
    snapshot["meta"]["entities_truncated"] = json!(total_entities > MAX_RESPONSE_ENTITIES);
    snapshot["meta"]["total_factions"] = json!(total_factions);
    snapshot["meta"]["factions_truncated"] = json!(total_factions > MAX_RESPONSE_FACTIONS);
    snapshot
}

#[cfg(test)]
mod live_snapshot_tests {
    use super::*;

    #[test]
    fn live_snapshot_window_is_bounded_and_keeps_total_counts() {
        let events: Vec<Value> = (0..250).map(|i| json!({"id":i})).collect();
        let entities: Vec<Value> = (0..600).map(|i| json!({"id":i,"alive":i%2==0})).collect();
        let factions: Vec<Value> = (0..110).map(|i| json!({"id":i})).collect();
        let compact = compact_live_snapshot(json!({"meta":{"tick":99},"events":events,"entities":entities,"factions":factions}));
        assert_eq!(compact["events"].as_array().unwrap().len(), MAX_RESPONSE_EVENTS);
        assert_eq!(compact["entities"].as_array().unwrap().len(), MAX_RESPONSE_ENTITIES);
        assert_eq!(compact["factions"].as_array().unwrap().len(), MAX_RESPONSE_FACTIONS);
        assert_eq!(compact["meta"]["total_events"], 250);
        assert_eq!(compact["meta"]["event_window_start"], 50);
        assert_eq!(compact["meta"]["total_entities"], 600);
        assert_eq!(compact["meta"]["alive_entities"], 300);
        assert_eq!(compact["meta"]["total_factions"], 110);
        assert_eq!(compact["meta"]["tick"], 99);
    }
}

fn persistence_status(state: &AppState) -> Value {
    let world_degraded = state.world_checkpoint_degraded.load(Ordering::Acquire);
    let memory_degraded = state.agent_memory_degraded.load(Ordering::Acquire);
    let sessions_degraded = state.sessions_checkpoint_degraded.load(Ordering::Acquire);
    json!({
        "degraded": world_degraded || memory_degraded || sessions_degraded,
        "world_checkpoint_degraded": world_degraded,
        "agent_memory_degraded": memory_degraded,
        "sessions_checkpoint_degraded": sessions_degraded
    })
}

fn state_payload(state: &AppState, world: &WorldModel) -> Value {
    let info = serde_json::from_str::<Value>(&world.info_json()).unwrap_or(Value::Null);
    let snapshot = serde_json::from_str::<Value>(&world.snapshot_json()).unwrap_or(Value::Null);
    json!({
        "kind": "state",
        "backend": "wanxiang_sdk::WorldModel",
        "info": info,
        "snapshot": compact_live_snapshot(snapshot),
        "production_orders": world.production_orders(),
        "production_sites": world.production_sites(),
        "server_time_unix": now_secs(),
        "persistent_checkpoint": true,
        "persistence": persistence_status(state),
        "snapshot_window": {
            "max_events": MAX_RESPONSE_EVENTS,
            "max_entities": MAX_RESPONSE_ENTITIES,
            "max_factions": MAX_RESPONSE_FACTIONS
        }
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
                "auto_tick_ms":state.auto_tick_ms,
                "agent_retrieval_mode":state.agent_retrieval_mode,
                "persistence":persistence_status(state),
                "api_version":1
            }),
        ),
        ("GET", "/ready") => {
            let persistence = persistence_status(state);
            if persistence.get("degraded").and_then(Value::as_bool).unwrap_or(true) {
                (503, json!({"status":"degraded","persistence":persistence}))
            } else {
                (200, json!({
                    "status":"ready",
                    "backend":"wanxiang_sdk::WorldModel",
                    "persistence":persistence
                }))
            }
        },
        ("GET", "/api/state") => match state.world.lock() {
            Ok(world) => (200, state_payload(state, &world)),
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
                    let payload = state_payload(state, &world);
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
        ("POST", "/api/move") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let character_id = match args.get("character_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                _ => return (400, json!({"error":"character_id must be a non-empty string of at most 128 characters"})),
            };
            let destination_id = match args.get("destination_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                _ => return (400, json!({"error":"destination_id must be a non-empty string of at most 128 characters"})),
            };
            match state.world.lock() {
                Ok(mut world) => match world.move_character(character_id, destination_id) {
                    Ok(()) => {
                        let payload = state_payload(state, &world);
                        (200, json!({
                            "kind":"move",
                            "moved":true,
                            "character_id":character_id,
                            "destination_id":destination_id,
                            "tick":world.current_tick(),
                            "info":payload["info"],
                            "snapshot":payload["snapshot"],
                            "server_time_unix":payload["server_time_unix"],
                            "persistent_checkpoint":true
                        }))
                    }
                    Err(error) => (400, json!({"error":error,"moved":false})),
                },
                Err(_) => (500, json!({"error":"world lock poisoned"})),
            }
        }
        ("POST", "/api/gather") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let character_id = match args.get("character_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                _ => return (400, json!({"error":"character_id must be a non-empty string of at most 128 characters"})),
            };
            let resource = match args.get("resource").and_then(Value::as_str) {
                Some(name) if !name.trim().is_empty() && name.trim().chars().count() <= 128 && !name.chars().any(char::is_control) => name.trim(),
                _ => return (400, json!({"error":"resource must be a non-empty string of at most 128 characters without control characters"})),
            };
            let amount = match args.get("amount").and_then(Value::as_f64) {
                Some(amount) if amount.is_finite() && amount > 0.0 && amount <= 1_000_000.0 => amount,
                _ => return (400, json!({"error":"amount must be finite and in (0, 1000000]"})),
            };
            match state.world.lock() {
                Ok(mut world) => match world.gather_resource(character_id, resource, amount) {
                    Ok(report) => {
                        let payload = state_payload(state, &world);
                        (200, json!({"kind":"gather","gathered":true,"report":report,"tick":world.current_tick(),"snapshot":payload["snapshot"],"persistent_checkpoint":true}))
                    }
                    Err(error) => (400, json!({"error":error,"gathered":false})),
                },
                Err(_) => (500, json!({"error":"world lock poisoned"})),
            }
        }
        ("POST", "/api/trade") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let character_id = match args.get("character_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                _ => return (400, json!({"error":"character_id must be a non-empty string of at most 128 characters"})),
            };
            let resource = match args.get("resource").and_then(Value::as_str) {
                Some(name) if !name.trim().is_empty() && name.trim().chars().count() <= 128 && !name.chars().any(char::is_control) => name.trim(),
                _ => return (400, json!({"error":"resource must be a non-empty string of at most 128 characters without control characters"})),
            };
            let payment_resource = match args.get("payment_resource").and_then(Value::as_str) {
                Some(name) if !name.trim().is_empty() && name.trim().chars().count() <= 128 && !name.chars().any(char::is_control) => name.trim(),
                _ => return (400, json!({"error":"payment_resource must be a non-empty string of at most 128 characters without control characters"})),
            };
            let side = match args.get("side").and_then(Value::as_str) {
                Some("buy") => MarketTradeSide::Buy,
                Some("sell") => MarketTradeSide::Sell,
                _ => return (400, json!({"error":"side must be 'buy' or 'sell'"})),
            };
            let amount = match args.get("amount").and_then(Value::as_f64) {
                Some(amount) if amount.is_finite() && amount > 0.0 && amount <= 1_000_000.0 => amount,
                _ => return (400, json!({"error":"amount must be finite and in (0, 1000000]"})),
            };
            match state.world.lock() {
                Ok(mut world) => match world.trade_resource(character_id, side, resource, payment_resource, amount) {
                    Ok(report) => {
                        let payload = state_payload(state, &world);
                        (200, json!({"kind":"trade","traded":true,"report":report,"tick":world.current_tick(),"snapshot":payload["snapshot"],"persistent_checkpoint":true}))
                    }
                    Err(error) => (400, json!({"error":error,"traded":false})),
                },
                Err(_) => (500, json!({"error":"world lock poisoned"})),
            }
        }
        ("POST", "/api/production/order") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let required_id = |key: &str| -> Result<String, Value> {
                match args.get(key).and_then(Value::as_str) {
                    Some(value) if !value.trim().is_empty() && value.chars().count() <= 128
                        && !value.chars().any(char::is_control) => Ok(value.trim().to_string()),
                    _ => Err(json!({"error":format!("{key} must be a non-empty string of at most 128 characters without control characters")})),
                }
            };
            let faction_id = match required_id("faction_id") { Ok(v) => v, Err(e) => return (400, e) };
            let location_id = match required_id("location_id") { Ok(v) => v, Err(e) => return (400, e) };
            let output_resource = match required_id("output_resource") { Ok(v) => v, Err(e) => return (400, e) };
            let workers = match args.get("workers").and_then(Value::as_u64) {
                Some(value) if (1..=1000).contains(&value) => value as usize,
                _ => return (400, json!({"error":"workers must be an integer in 1..=1000"})),
            };
            let inputs: std::collections::BTreeMap<String, f64> = if let Some(raw_inputs) = args.get("inputs") {
                let object = match raw_inputs.as_object() {
                    Some(object) => object,
                    None => return (400, json!({"error":"inputs must be a JSON object mapping resource names to positive amounts"})),
                };
                let mut parsed = std::collections::BTreeMap::new();
                for (resource, amount_value) in object {
                    if resource.trim().is_empty() || resource.chars().count() > 128
                        || resource.chars().any(char::is_control) {
                        return (400, json!({"error":"ingredient names must be non-empty and at most 128 characters without control characters"}));
                    }
                    let amount = match amount_value.as_f64() {
                        Some(amount) if amount.is_finite() && amount > 0.0 && amount <= 1_000_000.0 => amount,
                        _ => return (400, json!({"error":format!("ingredient '{resource}' amount must be finite and in (0, 1000000]")})),
                    };
                    parsed.insert(resource.trim().to_string(), amount);
                }
                parsed
            } else {
                let input_resource = match required_id("input_resource") { Ok(v) => v, Err(e) => return (400, e) };
                std::collections::BTreeMap::from([(input_resource, 1.0)])
            };
            let optional_number = |key: &str, default: f64| -> Result<f64, Value> {
                match args.get(key) {
                    None => Ok(default),
                    Some(value) => match value.as_f64() {
                        Some(number) if number.is_finite() => Ok(number),
                        _ => Err(json!({"error":format!("{key} must be a finite number")})),
                    },
                }
            };
            let output_amount = match optional_number("output_amount", 1.0) {
                Ok(v) => v, Err(e) => return (400, e),
            };
            let wage_per_worker = match optional_number("wage_per_worker", 0.0) {
                Ok(v) => v, Err(e) => return (400, e),
            };
            let skill_level = match optional_number("skill_level", 0.0) {
                Ok(v) => v, Err(e) => return (400, e),
            };
            let wage_resource = match args.get("wage_resource") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if !value.trim().is_empty()
                    && value.chars().count() <= 128 && !value.chars().any(char::is_control) => {
                    Some(value.trim().to_string())
                }
                _ => return (400, json!({"error":"wage_resource must be null or a non-empty string of at most 128 characters"})),
            };
            match state.world.lock() {
                Ok(mut world) => match world.set_production_recipe(
                    &faction_id, &location_id, inputs, &output_resource, output_amount, workers,
                    wage_resource.as_deref(), wage_per_worker, skill_level,
                ) {
                    Ok(order) => {
                        let payload = state_payload(state, &world);
                        (200, json!({
                            "kind":"production_order","configured":true,"order":order,
                            "tick":world.current_tick(),"snapshot":payload["snapshot"],
                            "production_orders":world.production_orders(),"production_sites":world.production_sites(),
                            "persistent_checkpoint":true
                        }))
                    }
                    Err(error) => (400, json!({"error":error,"configured":false})),
                },
                Err(_) => (500, json!({"error":"world lock poisoned"})),
            }
        }
        ("POST", "/api/production/cancel") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let order_id = match args.get("order_id").and_then(Value::as_str) {
                Some(value) if !value.trim().is_empty() && value.chars().count() <= 128
                    && !value.chars().any(char::is_control) => value.trim(),
                _ => return (400, json!({"error":"order_id must be a non-empty string of at most 128 characters"})),
            };
            match state.world.lock() {
                Ok(mut world) => match world.cancel_production_order(order_id) {
                    Ok(order) => {
                        let payload = state_payload(state, &world);
                        (200, json!({
                            "kind":"production_cancelled","cancelled":true,"order":order,
                            "tick":world.current_tick(),"snapshot":payload["snapshot"],
                            "production_orders":world.production_orders(),"production_sites":world.production_sites(),
                            "persistent_checkpoint":true
                        }))
                    }
                    Err(error) => (400, json!({"error":error,"cancelled":false})),
                },
                Err(_) => (500, json!({"error":"world lock poisoned"})),
            }
        }
        ("POST", "/api/plan-travel") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let character_id = match args.get("character_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                _ => return (400, json!({"error":"character_id must be a non-empty string of at most 128 characters"})),
            };
            let destination_id = match args.get("destination_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                _ => return (400, json!({"error":"destination_id must be a non-empty string of at most 128 characters"})),
            };
            match state.world.lock() {
                Ok(world) => match world.plan_travel_character(character_id, destination_id) {
                    Ok(plan) => (200, json!({"kind":"travel_plan","tick":world.current_tick(),"plan":plan})),
                    Err(error) => (400, json!({"error":error,"planned":false})),
                },
                Err(_) => (500, json!({"error":"world lock poisoned"})),
            }
        }
        ("POST", "/api/travel") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(v) => v,
                Err(e) => return (400, json!({"error":format!("invalid JSON: {e}")})),
            };
            let character_id = match args.get("character_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                _ => return (400, json!({"error":"character_id must be a non-empty string of at most 128 characters"})),
            };
            let destination_id = match args.get("destination_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() && id.chars().count() <= 128 => id,
                _ => return (400, json!({"error":"destination_id must be a non-empty string of at most 128 characters"})),
            };
            match state.world.lock() {
                Ok(mut world) => match world.simulate_travel_character(character_id, destination_id) {
                    Ok(report) => {
                        let payload = state_payload(state, &world);
                        let travel = serde_json::to_value(&report).unwrap_or(Value::Null);
                        (200, json!({
                            "kind":"travel",
                            "moved":report.completed,
                            "character_id":character_id,
                            "destination_id":destination_id,
                            "path":report.path,
                            "route_cost":report.route_cost,
                            "travel":travel,
                            "tick":world.current_tick(),
                            "info":payload["info"],
                            "snapshot":payload["snapshot"],
                            "server_time_unix":payload["server_time_unix"],
                            "persistent_checkpoint":true
                        }))
                    }
                    Err(error) => (400, json!({"error":error,"moved":false})),
                },
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
                Ok(world_guard) => {
                    let source_snapshot = world_guard.fork();
                    drop(world_guard);
                    match run_rule_experiment(&source_snapshot, rule, value, ticks) {
                        Ok(mut result) => {
                            for key in ["baseline", "altered"] {
                                if let Some(snapshot) = result.get(key).cloned() {
                                    result[key] = compact_live_snapshot(snapshot);
                                }
                            }
                            (200, result)
                        }
                        Err(error) => (400, json!({"error":error})),
                    }
                }
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
                        Ok(world) => state_payload(state, &world),
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

        ("POST", "/api/sessions") => {
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(value) => value,
                Err(error) => return (400, json!({"error":format!("invalid JSON: {error}")})),
            };
            let task = match args.get("task").and_then(Value::as_str) {
                Some(task) if !task.trim().is_empty() && task.chars().count() <= MAX_TASK_CHARS => task,
                _ => return (400, json!({"error":format!("task must be a non-empty string of at most {MAX_TASK_CHARS} characters")})),
            };
            let Some(agent_mutex) = &state.agent else {
                return (503, json!({"error":"LLM agent is disabled; configure WANXIANG_LLM_ENDPOINT and WANXIANG_LLM_MODEL"}));
            };
            // Keep one lock order across all session operations: sessions, then Agent.
            let mut sessions = match state.sessions.lock() {
                Ok(sessions) => sessions,
                Err(_) => return (500, json!({"error":"session store lock poisoned"})),
            };
            prune_sessions(&mut sessions);
            if sessions.len() >= MAX_PERSISTED_SESSIONS {
                return (429, json!({"error":format!("session capacity reached ({MAX_PERSISTED_SESSIONS}); wait for TTL expiry or restart after archiving old sessions")}));
            }
            let session_id = loop {
                let candidate = format!("session_{}_{}", now_secs(), NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed));
                if !sessions.contains_key(&candidate) { break candidate; }
            };
            let agent = match agent_mutex.lock() {
                Ok(agent) => agent,
                Err(_) => return (500, json!({"error":"agent lock poisoned"})),
            };
            let session = agent.start_session(session_id.clone(), task);
            drop(agent);
            sessions.insert(session_id.clone(), StoredSession { updated_at: now_secs(), session });
            if let Err(error) = persist_sessions(state, &sessions) {
                sessions.remove(&session_id);
                return (500, json!({"error":"could not persist new Agent session","detail":error}));
            }
            (201, json!({"session":session_payload(sessions.get(&session_id).unwrap())}))
        }
        ("GET", path) if path.starts_with("/api/sessions/") => {
            let Some(session_id) = session_id_for_route(path, false) else {
                return (400, json!({"error":"invalid session path"}));
            };
            match state.sessions.lock() {
                Ok(mut sessions) => {
                    let previous_len = sessions.len();
                    prune_sessions(&mut sessions);
                    if sessions.len() != previous_len {
                        if let Err(error) = persist_sessions(state, &sessions) {
                            return (500, json!({"error":"could not persist session TTL cleanup","detail":error}));
                        }
                    }
                    match sessions.get(session_id) {
                        Some(entry) => (200, json!({"session":session_payload(entry)})),
                        None => (404, json!({"error":"session not found or expired"})),
                    }
                }
                Err(_) => (500, json!({"error":"session store lock poisoned"})),
            }
        }
        ("POST", path) if path.starts_with("/api/sessions/") && path.ends_with("/drive") => {
            let Some(session_id) = session_id_for_route(path, true) else {
                return (400, json!({"error":"invalid session path"}));
            };
            let args = match serde_json::from_str::<Value>(&request.body) {
                Ok(value) => value,
                Err(error) => return (400, json!({"error":format!("invalid JSON: {error}")})),
            };
            let decision = match args.get("decision").and_then(Value::as_str) {
                None => None,
                Some("approve") => Some(ApprovalDecision::Approve),
                Some("reject") => Some(ApprovalDecision::Reject),
                Some(_) => return (400, json!({"error":"decision must be 'approve', 'reject', or omitted"})),
            };
            let Some(agent_mutex) = &state.agent else {
                return (503, json!({"error":"LLM agent is disabled"}));
            };
            let mut sessions = match state.sessions.lock() {
                Ok(sessions) => sessions,
                Err(_) => return (500, json!({"error":"session store lock poisoned"})),
            };
            let previous_len = sessions.len();
            prune_sessions(&mut sessions);
            if sessions.len() != previous_len {
                if let Err(error) = persist_sessions(state, &sessions) {
                    return (500, json!({"error":"could not persist session TTL cleanup","detail":error}));
                }
            }
            let Some(entry) = sessions.get_mut(session_id) else {
                return (404, json!({"error":"session not found or expired"}));
            };
            let outcome = {
                let mut agent = match agent_mutex.lock() {
                    Ok(agent) => agent,
                    Err(_) => return (500, json!({"error":"agent lock poisoned"})),
                };
                agent.drive_session(&mut entry.session, decision)
            };
            entry.updated_at = now_secs();
            let outcome_json = session_outcome(&outcome);
            let session_json = session_payload(entry);
            if let Err(error) = persist_agent_memory(state) {
                return (500, json!({"error":"Agent session advanced but durable memory write failed","detail":error,"session":session_json,"outcome":outcome_json}));
            }
            if let Err(error) = persist_sessions(state, &sessions) {
                return (500, json!({"error":"Agent session advanced but durable session write failed","detail":error,"session":session_json,"outcome":outcome_json}));
            }
            (200, json!({"session":session_json,"outcome":outcome_json}))
        }
        ("OPTIONS", _) => (204, json!({"ok":true})),
        _ => (404, json!({"error":"not found","routes":["GET /health","GET /ready","GET /api/state","POST /api/step","POST /api/gather","POST /api/trade","POST /api/production/order","POST /api/production/cancel","POST /api/experiment","POST /api/agent","POST /api/llm/stream","POST /api/sessions","GET /api/sessions/{id}","POST /api/sessions/{id}/drive"]})),
    }
}


fn respond_sse_start(stream: &mut TcpStream, origin: Option<&str>, allowed: bool) -> std::io::Result<()> {
    let cors = if allowed {
        origin.map(|o| format!(
            "Access-Control-Allow-Origin: {o}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Idempotency-Key\r\n"
        )).unwrap_or_default()
    } else { String::new() };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache, no-store\r\nX-Content-Type-Options: nosniff\r\nX-Accel-Buffering: no\r\nConnection: close\r\n{}\r\n",
        cors
    );
    stream.write_all(header.as_bytes())?;
    stream.flush()
}

fn send_sse_event(stream: &mut TcpStream, event: &str, payload: &Value) -> std::io::Result<()> {
    let json = serde_json::to_string(payload)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(format!("event: {event}\ndata: {json}\n\n").as_bytes())?;
    stream.flush()
}

fn handle_llm_stream(
    stream: &mut TcpStream,
    state: &AppState,
    request: &HttpRequest,
    origin: Option<&str>,
    allowed: bool,
) {
    let args = match serde_json::from_str::<Value>(&request.body) {
        Ok(value) => value,
        Err(error) => {
            respond(stream, 400, &json!({"error":format!("invalid JSON: {error}")}), origin, allowed);
            return;
        }
    };
    let prompt = match args.get("prompt").and_then(Value::as_str) {
        Some(prompt) if !prompt.trim().is_empty() && prompt.chars().count() <= MAX_TASK_CHARS => prompt,
        _ => {
            respond(stream, 400, &json!({"error":format!("prompt must be a non-empty string of at most {MAX_TASK_CHARS} characters")}), origin, allowed);
            return;
        }
    };
    let agent_mutex = match &state.agent {
        Some(agent) => agent,
        None => {
            respond(stream, 503, &json!({"error":"LLM is disabled; configure WANXIANG_LLM_ENDPOINT and WANXIANG_LLM_MODEL on the server"}), origin, allowed);
            return;
        }
    };
    // Copy immutable provider configuration while holding the planner lock only briefly.
    // Keep slow streaming requests serialized independently from Agent memory/approval work.
    let config = match agent_mutex.lock() {
        Ok(guard) => guard.planner().adapter().config().clone(),
        Err(_) => {
            respond(stream, 500, &json!({"error":"agent lock poisoned"}), origin, allowed);
            return;
        }
    };
    let _stream_guard = match state.stream_gate.lock() {
        Ok(guard) => guard,
        Err(_) => {
            respond(stream, 500, &json!({"error":"LLM stream gate poisoned"}), origin, allowed);
            return;
        }
    };
    let endpoint = config.endpoint.trim_end_matches('/');
    let url = if endpoint.ends_with("/chat/completions") {
        endpoint.to_string()
    } else {
        format!("{endpoint}/chat/completions")
    };
    let messages = [Message::user(prompt)];
    let body = match serde_json::to_value(OpenAiCompatibleRequest::stream(&config, &messages)) {
        Ok(body) => body,
        Err(error) => {
            respond(stream, 500, &json!({"error":format!("cannot encode stream request: {error}")}), origin, allowed);
            return;
        }
    };
    let headers = authorization_headers(&config);
    if let Err(error) = respond_sse_start(stream, origin, allowed) {
        eprintln!("cannot start SSE response: {error}");
        return;
    }

    let mut content_chunks = 0usize;
    let mut provider_error: Option<String> = None;
    let result = CurlJsonTransport::stream_chat(&url, &headers, &body, config.timeout_ms, |line| {
        match stream_delta(line) {
            Ok(Some(content)) => {
                content_chunks += 1;
                send_sse_event(stream, "token", &json!({"content":content}))
                    .map_err(|error| LLMError::Network(format!("client stream closed: {error}")))?;
            }
            Ok(None) => {}
            Err(error) => { provider_error = Some(error); }
        }
        Ok(())
    });

    match result {
        Err(error) => {
            let _ = send_sse_event(stream, "error", &json!({"message":error.to_string()}));
        }
        Ok(status) if !(200..300).contains(&status) => {
            let message = match status {
                401 | 403 => "provider authentication failed".to_string(),
                429 => "provider rate limit exceeded".to_string(),
                other => format!("provider returned HTTP {other}"),
            };
            let _ = send_sse_event(stream, "error", &json!({"status":status,"message":provider_error.unwrap_or(message)}));
        }
        Ok(_) => {
            if let Some(error) = provider_error {
                let _ = send_sse_event(stream, "error", &json!({"message":error}));
            } else if content_chunks == 0 {
                let _ = send_sse_event(stream, "error", &json!({"message":"provider stream returned no text content"}));
            } else {
                let _ = send_sse_event(stream, "done", &json!({"chunks":content_chunks}));
            }
        }
    }
    let _ = send_sse_event(stream, "end", &json!({"done":true}));
}


/// Run slow, world-related POSTs without holding the global idempotency-cache lock
/// during provider waits or counterfactual simulation. The gate serializes slow requests;
/// the short-lived status=425 entry prevents other routes reusing the same key concurrently.
fn handle_slow_mutation(
    stream: &mut TcpStream,
    state: &AppState,
    request: &HttpRequest,
    origin: Option<&str>,
    allowed: bool,
) {
    let gate = match state.slow_mutation_gate.lock() {
        Ok(gate) => gate,
        Err(_) => {
            respond(stream, 500, &json!({"error":"slow mutation gate poisoned"}), origin, allowed);
            return;
        }
    };
    let key = request.headers.get("idempotency-key").map(|s| s.trim()).filter(|s| !s.is_empty());
    if key.is_some_and(|key| key.len() > 128 || key.chars().any(char::is_control)) {
        respond(stream, 400, &json!({"error":"Idempotency-Key must be at most 128 visible characters"}), origin, allowed);
        return;
    }
    let hash = request_hash(request);
    {
        let mut cache = match state.cache.lock() {
            Ok(cache) => cache,
            Err(_) => {
                respond(stream, 500, &json!({"error":"idempotency lock poisoned"}), origin, allowed);
                return;
            }
        };
        prune_cache(&mut cache);
        if let Some(key) = key {
            if let Some(previous) = cache.get(key) {
                if previous.request_hash != hash {
                    respond(stream, 409, &json!({"error":"Idempotency-Key was already used with a different request"}), origin, allowed);
                    return;
                }
                respond(stream, previous.status, &previous.body, origin, allowed);
                return;
            }
            cache.insert(key.to_string(), CachedResponse {
                created_at: now_secs(),
                request_hash: hash.clone(),
                status: 425,
                body: json!({"error":"request is already in progress; retry with the same Idempotency-Key"}),
            });
        }
    }

    let path = request.path.split('?').next().unwrap_or(&request.path);
    let (mut status, mut body) = route_request(state, request);
    if status < 400 && path == "/api/agent" {
        if let Err(error) = persist_agent_memory(state) {
            status = 500;
            body = json!({"error":"Agent ran but durable memory checkpoint failed","detail":error});
        }
    }

    let mut cache = match state.cache.lock() {
        Ok(cache) => cache,
        Err(_) => {
            respond(stream, 500, &json!({"error":"idempotency lock poisoned after operation"}), origin, allowed);
            return;
        }
    };
    if status < 400 {
        if let Some(key) = key {
            cache.insert(key.to_string(), CachedResponse {
                created_at: now_secs(),
                request_hash: hash,
                status,
                body: body.clone(),
            });
        }
        prune_cache(&mut cache);
        if let Err(error) = persist_state(state, &cache) {
            status = 500;
            body = json!({
                "error":"operation completed in memory but durable checkpoint failed",
                "detail":error,
                "retry":"Check the checkpoint path before retrying; do not create a new Idempotency-Key."
            });
            if let Some(key) = key {
                cache.insert(key.to_string(), CachedResponse {
                    created_at: now_secs(),
                    request_hash: request_hash(request),
                    status,
                    body: body.clone(),
                });
            }
        }
    } else if let Some(key) = key {
        if cache.get(key).is_some_and(|entry| entry.status == 425 && entry.request_hash == hash) {
            if status >= 500 {
                // The operation may have changed world or Agent state before a later
                // persistence step failed. Retain the failure response so an identical
                // client retry cannot execute those side effects a second time.
                cache.insert(key.to_string(), CachedResponse {
                    created_at: now_secs(),
                    request_hash: hash.clone(),
                    status,
                    body: body.clone(),
                });
                prune_cache(&mut cache);
                if let Err(error) = persist_state(state, &cache) {
                    eprintln!("could not persist failed idempotency result: {error}");
                }
            } else {
                cache.remove(key);
            }
        }
    }
    drop(cache);
    drop(gate);
    respond(stream, status, &body, origin, allowed);
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

    if request.method == "POST" && request.path.split('?').next().unwrap_or(&request.path) == "/api/llm/stream" {
        handle_llm_stream(&mut stream, &state, &request, origin, allowed);
        return;
    }

    let mutation_path = request.path.split('?').next().unwrap_or(&request.path);
    let is_slow_mutation = request.method == "POST"
        && (matches!(mutation_path, "/api/agent" | "/api/experiment" | "/api/sessions")
            || (mutation_path.starts_with("/api/sessions/") && mutation_path.ends_with("/drive")));
    if is_slow_mutation {
        handle_slow_mutation(&mut stream, &state, &request, origin, allowed);
        return;
    }
    let is_mutation = request.method == "POST"
        && (matches!(mutation_path, "/api/step" | "/api/move" | "/api/gather" | "/api/trade" | "/api/production/order" | "/api/production/cancel" | "/api/travel" | "/api/experiment" | "/api/agent" | "/api/sessions")
            || (mutation_path.starts_with("/api/sessions/") && mutation_path.ends_with("/drive")));
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
    if status < 400 && request.path.split('?').next().unwrap_or(&request.path) == "/api/agent" {
        if let Err(error) = persist_agent_memory(&state) {
            status = 500;
            body = json!({"error":"Agent ran but durable memory checkpoint failed","detail":error});
        }
    }
    if status < 400 {
        if let Some(key) = key {
            cache.insert(key.to_string(), CachedResponse {
                created_at: now_secs(),
                request_hash: hash.clone(),
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
                "retry":"Check the checkpoint path before retrying; reuse the same Idempotency-Key to avoid re-executing this operation."
            });
            if let Some(key) = key {
                cache.insert(key.to_string(), CachedResponse {
                    created_at: now_secs(),
                    request_hash: hash.clone(),
                    status,
                    body: body.clone(),
                });
                prune_cache(&mut cache);
                if let Err(retry_error) = persist_state(&state, &cache) {
                    eprintln!("could not persist failure response for idempotency key: {retry_error}");
                }
            }
        }
    }
    respond(&mut stream, status, &body, origin, allowed);
}


/// Bounds thread-per-connection resource usage. A guard releases the slot even
/// when request parsing or routing returns early.
struct ActiveConnectionGuard(Arc<AtomicUsize>);

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn try_acquire_connection(active: &AtomicUsize) -> bool {
    active.try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        (current < MAX_ACTIVE_CONNECTIONS).then_some(current + 1)
    }).is_ok()
}

const MIN_PUBLIC_TOKEN_BYTES: usize = 32;

fn validate_public_bind_token(address: SocketAddr, token: Option<&str>) -> Result<(), &'static str> {
    if address.ip().is_loopback() {
        return Ok(());
    }
    match token {
        None => Err("public binds require WANXIANG_API_TOKEN"),
        Some(value) if value.as_bytes().len() < MIN_PUBLIC_TOKEN_BYTES => {
            Err("WANXIANG_API_TOKEN must be at least 32 bytes for non-loopback binds")
        }
        Some(_) => Ok(()),
    }
}

fn main() {
    let bind = env::var("WANXIANG_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let address: SocketAddr = bind.parse().expect("WANXIANG_BIND must be an IP:PORT socket address");
    let api_token = env::var("WANXIANG_API_TOKEN").ok().filter(|s| !s.trim().is_empty());
    if let Err(error) = validate_public_bind_token(address, api_token.as_deref()) {
        eprintln!("Refusing to bind {address}: {error}. Use a loopback bind for local development or provide a strong random token.");
        std::process::exit(2);
    }

    let data_dir = env::var("WANXIANG_DATA_DIR").unwrap_or_else(|_| ".wanxiang-live-data".into());
    let data_root = PathBuf::from(data_dir);
    let data_file = data_root.join("runtime-state.json");
    let memory_file = data_root.join("agent-memory.json");
    let sessions_file = data_root.join("sessions.json");
    let sessions = match read_sessions(&sessions_file) {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let restored_agent_memory = match read_agent_memory(&memory_file) {
        Ok(memory) => memory,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    // Real-time mode advances independently of connected browsers by default.
    // Set WANXIANG_TICK_MS=0 for request-driven/manual stepping and deterministic smoke tests.
    let auto_tick_ms = env_u64("WANXIANG_TICK_MS", 1_000, 0, 60_000);
    let (world, cache) = match read_persisted(&data_file) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let world = Arc::new(Mutex::new(world));
    let agent_retrieval_mode = env::var("WANXIANG_AGENT_RETRIEVAL")
        .unwrap_or_else(|_| "causal".to_string()).to_ascii_lowercase();
    let (retrieval_top_k, use_causal) = match agent_retrieval_mode.as_str() {
        "none" | "no-memory" => (0, false),
        "memory" => (5, false),
        "causal" => (5, true),
        _ => {
            eprintln!("WANXIANG_AGENT_RETRIEVAL must be no-memory, memory, or causal");
            std::process::exit(2);
        }
    };

    let agent = match (env::var("WANXIANG_LLM_ENDPOINT").ok(), env::var("WANXIANG_LLM_MODEL").ok()) {
        (Some(endpoint), Some(model)) if !endpoint.trim().is_empty() && !model.trim().is_empty() => {
            if let Err(error) = validate_provider_endpoint(&endpoint) {
                eprintln!("Refusing provider configuration: {error}");
                std::process::exit(2);
            }
            let mut config = OpenAiCompatibleConfig::new(endpoint, model);
            config.timeout_ms = env_u64("WANXIANG_LLM_TIMEOUT_MS", 30_000, 1_000, 120_000);
            if let Ok(key) = env::var("WANXIANG_LLM_API_KEY") {
                if !key.is_empty() { config = config.with_api_key(key); }
            }
            let adapter = OpenAiCompatibleAdapter::new(config, CurlJsonTransport);
            let memory = restored_agent_memory.unwrap_or_else(|| NmmUnified::new(SdrEncoder::default()));
            let planner = MemoryBackedLlmPlanner::new(adapter, memory)
                .with_retrieval(retrieval_top_k, 0.0, use_causal)
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
        memory_file,
        sessions_file,
        sessions: Mutex::new(sessions),
        cache: Mutex::new(cache),
        slow_mutation_gate: Mutex::new(()),
        stream_gate: Mutex::new(()),
        world_checkpoint_degraded: AtomicBool::new(false),
        agent_memory_degraded: AtomicBool::new(false),
        sessions_checkpoint_degraded: AtomicBool::new(false),
        auto_tick_ms,
        agent_retrieval_mode: agent_retrieval_mode.clone(),
    });
    if env::var("WANXIANG_LLM_ENDPOINT").is_ok() && state.agent.is_none() {
        eprintln!("LLM was not enabled: set both WANXIANG_LLM_ENDPOINT and WANXIANG_LLM_MODEL.");
    }
    let listener = TcpListener::bind(address).unwrap_or_else(|e| {
        eprintln!("cannot bind {address}: {e}");
        std::process::exit(2);
    });
    println!("Wanxiang live API listening at http://{address}");
    println!("Routes: GET /health, GET /api/state, POST /api/step, POST /api/move, POST /api/travel, POST /api/experiment, POST /api/agent, POST /api/llm/stream, POST /api/sessions, GET /api/sessions/{{id}}, POST /api/sessions/{{id}}/drive");
    println!("Persistent world checkpoint: {}", state.data_file.display());
    println!("Persistent Agent memory: {}", state.memory_file.display());
    println!("Persistent Agent sessions: {}", state.sessions_file.display());
    println!("Agent retrieval mode: {} (set WANXIANG_AGENT_RETRIEVAL=none|memory|causal)", state.agent_retrieval_mode);
    if state.agent.is_some() {
        println!("LLM agent enabled; provider key stays on this server and curl provides HTTPS transport.");
    } else {
        println!("LLM agent disabled; world simulation and deterministic experiments work without a provider.");
    }
    if auto_tick_ms > 0 {
        println!("Autonomous simulation tick every {auto_tick_ms} ms (set WANXIANG_TICK_MS=0 to disable).");
        let ticker_state = Arc::clone(&state);
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(ticker_state.auto_tick_ms));
            advance_background_tick(&ticker_state);
        });
    } else {
        println!("Autonomous ticking disabled; use POST /api/step to advance the world.");
    }
    // Cap concurrent sockets so slow clients cannot create unbounded threads.
    let active_connections = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        match incoming {
            Ok(mut stream) => {
                if !try_acquire_connection(&active_connections) {
                    respond(
                        &mut stream,
                        503,
                        &json!({"error":"server is at connection capacity; retry shortly"}),
                        None,
                        false,
                    );
                    continue;
                }
                let state = Arc::clone(&state);
                let active = Arc::clone(&active_connections);
                let spawn = thread::Builder::new()
                    .name("wanxiang-http".to_string())
                    .spawn(move || {
                        let _guard = ActiveConnectionGuard(active);
                        handle_connection(stream, state);
                    });
                if let Err(error) = spawn {
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                    eprintln!("could not start request thread: {error}");
                }
            }
            Err(error) => eprintln!("incoming connection failed: {error}"),
        }
    }
}

#[cfg(test)]
mod session_checkpoint_tests {
    use super::*;
    use agent_runtime::AgentRunner;

    #[test]
    fn durable_session_store_round_trip_preserves_pending_session_state() {
        let runner = AgentRunner::new(4);
        let mut session = ResumableSession::new("persisted-session", "test task", &runner);
        session.pending_approval = Some(agent_runtime::PendingApproval {
            tool_name: "sensitive_tool".to_string(),
            arguments: json!({"confirm":true}),
        });
        let mut sessions = BTreeMap::new();
        sessions.insert("persisted-session".to_string(), StoredSession {
            updated_at: now_secs(),
            session,
        });
        let path = std::env::temp_dir().join(format!("wanxiang-session-{}-{}.json", std::process::id(), now_secs()));
        persist_sessions_to(&path, &sessions).expect("sessions persist");
        let restored = read_sessions(&path).expect("sessions restore");
        let _ = fs::remove_file(&path);
        let entry = restored.get("persisted-session").expect("session survived restore");
        assert_eq!(entry.session.user_input, "test task");
        assert_eq!(entry.session.pending_approval.as_ref().unwrap().tool_name, "sensitive_tool");
        assert_eq!(entry.session.pending_approval.as_ref().unwrap().arguments["confirm"], true);
    }
}


#[cfg(test)]
mod connection_limit_tests {
    use super::*;

    #[test]
    fn active_connection_limit_is_enforced() {
        let active = AtomicUsize::new(0);
        for expected in 1..=MAX_ACTIVE_CONNECTIONS {
            assert!(try_acquire_connection(&active));
            assert_eq!(active.load(Ordering::Acquire), expected);
        }
        assert!(!try_acquire_connection(&active));
        assert_eq!(active.load(Ordering::Acquire), MAX_ACTIVE_CONNECTIONS);
    }

    #[test]
    fn active_connection_guard_releases_capacity_on_drop() {
        let active = Arc::new(AtomicUsize::new(1));
        {
            let _guard = ActiveConnectionGuard(Arc::clone(&active));
            assert_eq!(active.load(Ordering::Acquire), 1);
        }
        assert_eq!(active.load(Ordering::Acquire), 0);
        assert!(try_acquire_connection(&active));
    }
}


#[cfg(test)]
mod public_bind_auth_tests {
    use super::*;

    fn address(value: &str) -> SocketAddr {
        value.parse().expect("valid socket address")
    }

    #[test]
    fn loopback_bind_does_not_require_public_token() {
        assert!(validate_public_bind_token(address("127.0.0.1:8787"), None).is_ok());
    }

    #[test]
    fn non_loopback_bind_requires_a_long_token() {
        assert!(validate_public_bind_token(address("0.0.0.0:8787"), None).is_err());
        assert!(validate_public_bind_token(address("0.0.0.0:8787"), Some("short-token")).is_err());
        assert!(validate_public_bind_token(
            address("0.0.0.0:8787"),
            Some("0123456789abcdef0123456789abcdef")
        ).is_ok());
    }
}

#[cfg(test)]
mod request_parser_tests {
    use super::*;

    #[test]
    fn parses_simple_http_1_1_request_and_header_case_insensitively() {
        let parsed = parse_request_head(
            b"POST /api/step HTTP/1.1\r\ncontent-length: 12\r\nauthorization: Bearer token"
        ).expect("valid request head");
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.path, "/api/step");
        assert_eq!(parsed.content_length, 12);
        assert_eq!(parsed.headers.get("authorization").map(String::as_str), Some("Bearer token"));
    }

    #[test]
    fn rejects_duplicate_length_and_transfer_encoding_headers() {
        assert!(parse_request_head(
            b"POST /api/step HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 3"
        ).is_err());
        assert!(parse_request_head(
            b"POST /api/step HTTP/1.1\r\nTransfer-Encoding: chunked"
        ).is_err());
    }

    #[test]
    fn rejects_invalid_length_header_name_and_http_version() {
        assert!(parse_request_head(b"POST /api/step HTTP/1.1\r\nContent-Length: nope").is_err());
        assert!(parse_request_head(b"GET /health HTTP/1.1\r\nBad Header: x").is_err());
        let version_error = parse_request_head(b"GET /health HTTP/2.0").unwrap_err();
        assert_eq!(version_error.0, 505);
    }

    #[test]
    fn request_body_limit_is_checked_before_reading_body() {
        let header = format!("POST /api/step HTTP/1.1\r\nContent-Length: {}", MAX_BODY_BYTES + 1);
        assert_eq!(parse_request_head(header.as_bytes()).unwrap_err().0, 413);
    }
}


#[cfg(test)]
mod provider_endpoint_validation_tests {
    use super::*;

    #[test]
    fn accepts_https_public_providers_and_loopback_http_development() {
        assert!(validate_provider_endpoint("https://api.example.com/v1").is_ok());
        assert!(validate_provider_endpoint("https://[2001:db8::1]:8443/v1").is_ok());
        assert!(validate_provider_endpoint("http://localhost:11434/v1").is_ok());
        assert!(validate_provider_endpoint("http://127.0.0.1:11434/v1").is_ok());
        assert!(validate_provider_endpoint("http://[::1]:11434/v1").is_ok());
    }

    #[test]
    fn rejects_plain_http_public_hosts_and_non_http_schemes() {
        assert!(validate_provider_endpoint("http://api.example.com/v1").is_err());
        assert!(validate_provider_endpoint("ftp://api.example.com/v1").is_err());
        assert!(validate_provider_endpoint("https:///v1").is_err());
        assert!(validate_provider_endpoint("https://user:password@api.example.com/v1").is_err());
        assert!(validate_provider_endpoint(" https://api.example.com/v1").is_err());
    }
}


#[cfg(test)]
mod world_type_config_tests {
    use super::{initial_world_for_settings, parse_world_type};
    use wanxiang_types::WorldType;

    #[test]
    fn no_world_type_configuration_defaults_to_neutral_custom_world() {
        assert_eq!(parse_world_type(None).unwrap(), WorldType::Custom);
        assert_eq!(initial_world_for_settings(None, None, 42).unwrap().snapshot().meta.world_type, "Custom");
    }

    #[test]
    fn supported_genres_can_be_selected_without_changing_default() {
        assert_eq!(parse_world_type(Some("scifi")).unwrap(), WorldType::Scifi);
        assert_eq!(parse_world_type(Some("都市")).unwrap(), WorldType::Urban);
        assert_eq!(parse_world_type(Some("western_fantasy")).unwrap(), WorldType::Western);
        assert_eq!(initial_world_for_settings(Some("xianxia"), None, 42).unwrap().snapshot().meta.world_type, "Xianxia");
    }

    #[test]
    fn authored_preset_is_opt_in_and_unknown_settings_fail_clearly() {
        let preset_world = initial_world_for_settings(None, Some("shitian"), 42).unwrap();
        assert_eq!(preset_world.snapshot().locations.len(), 3);
        assert!(parse_world_type(Some("not-a-genre")).is_err());
        assert!(initial_world_for_settings(None, Some("missing-preset"), 42).is_err());
    }
}
