# 实时 API 部署与运维手册

本文用于把仓库里的自托管 Rust API 运行在自己的主机上。**仓库配置本身不代表已经部署公共 API**；目前没有由仓库自动创建域名、主机、TLS 证书、监控或多租户账户的流程。没有完成本手册中的验证前，不要将服务视为生产级 SaaS。

## 1. 部署前置条件

需要一台由你管理、运行 Docker Engine 和 Docker Compose v2 的 Linux 主机，以及一个指向该主机的 DNS 域名。对外服务需允许入站 TCP 80/443，以便 Caddy 获取和续订 HTTPS 证书。不要直接将 Rust API 的 8787 端口暴露到公网；Compose 默认只将它绑定到宿主机的 127.0.0.1，公开流量应经过 Caddy。

当前服务使用单个共享 Bearer Token 认证，并提供实例级、按路由共享的限流。它适用于受控、自托管 beta，不提供用户身份、细粒度授权或多租户隔离。在公开开放注册或承载不可信用户前，还需要用户级授权、跨实例配额协调、集中日志、告警和经过验证的备份/恢复策略。

## 2. 配置并执行部署前检查

在仓库根目录执行：

    cp .env.live.example .env.live
    openssl rand -hex 32

把生成的随机字符串填入 .env.live 的 WANXIANG_API_TOKEN，然后配置：

- WANXIANG_DOMAIN：你控制的真实主机名，不要保留 api.example.com。
- WANXIANG_CORS_ORIGIN：可信浏览器前端的精确 origin，例如 https://daohan003.github.io。不要写通配符，也不要附加路径。
- WANXIANG_TICK_MS：0 表示仅手动推进；生产演示可选择一个明确的正值。
- WANXIANG_WORLD_SEED 和 WANXIANG_WORLD_TYPE / WANXIANG_WORLD_PRESET：首次创建世界时使用的配置。
- WANXIANG_RATE_LIMIT_AGENT_PER_MIN、WANXIANG_RATE_LIMIT_EXPERIMENT_PER_MIN、WANXIANG_RATE_LIMIT_STREAM_PER_MIN、WANXIANG_RATE_LIMIT_SESSIONS_PER_MIN：各高成本路由的每分钟 token-bucket 补充速率，默认依次为 12、30、6、12。
- WANXIANG_RATE_LIMIT_BURST：每个路由桶的突发容量，默认 3。
- WANXIANG_LLM_ENDPOINT、WANXIANG_LLM_MODEL、WANXIANG_LLM_API_KEY：若需启用模型驱动 Agent，三项必须完整填写。API Key 只放在服务器环境文件，绝不能写入静态页面或提交到 Git。

运行本地配置检查。它会显示检查结果，但不会打印 API Token 或模型密钥：

    python3 scripts/validate-live-deployment-env.py .env.live
    docker compose --env-file .env.live -f compose.live.yml --profile public config --quiet

任一检查失败都应先修复，不要通过关闭认证、放宽 CORS 或使用占位域名来绕过。

## 3. 启动和确认服务

配置检查通过、DNS 已生效且主机防火墙已开放 80/443 后，在仓库根目录执行：

    docker compose --env-file .env.live -f compose.live.yml --profile public up -d --build
    docker compose --env-file .env.live -f compose.live.yml ps
    docker compose --env-file .env.live -f compose.live.yml logs --tail=100 wanxiang-live caddy

然后从可信客户端验证 HTTPS、认证和 readiness：

    export WANXIANG_API_TOKEN='从 .env.live 中读取的值'
    export WANXIANG_API_BASE='https://你配置的域名'
    curl --fail --silent --show-error -H "Authorization: Bearer $WANXIANG_API_TOKEN" "$WANXIANG_API_BASE/health"
    curl --fail --silent --show-error -H "Authorization: Bearer $WANXIANG_API_TOKEN" "$WANXIANG_API_BASE/ready"
    python3 scripts/evaluate-live-api.py --base-url "$WANXIANG_API_BASE" --token "$WANXIANG_API_TOKEN"

预期 /health 报告 status=ok、backend=wanxiang_sdk::WorldModel；/ready 报告 status=ready 且 persistence 未降级；确定性 API evaluator 应通过。额外从无 Token 的客户端请求 /health，应得到 HTTP 401。不要只检查容器处于 running 状态。

/health 还提供不含凭据的 telemetry：服务运行时间、通过认证门的请求计数、触发限流的请求计数，以及限流配置。超出 Agent、规则实验、LLM 流式请求或会话路由限额时，API 返回 HTTP 429 和 Retry-After。当前限额是**单实例、按路由共享**：由于服务仍使用共享 Bearer Token，它不是按用户或跨多实例的全局配额。扩容或多租户前，需要在可信反向代理或共享存储层实现全局限额。

evaluate-live-api.py 会执行确定性世界检查和规则实验，不需要配置模型 API；只有显式传入其 --agent 参数才会调用模型。模型调用可能产生费用。

## 4. Agent 检索模式评测

scripts/benchmark-agent-modes.py 会调用配置的 LLM provider，因此不应无条件加入 CI。为三种模式分别启动独立实例和数据目录，并保证 provider、模型、初始 preset/type、seed 与起始 tick 一致；三套实例均设置 WANXIANG_TICK_MS=0，且分别配置 WANXIANG_AGENT_RETRIEVAL=no-memory、memory、causal。

示例（这里假设三个实例分别监听 8787、8788、8789）：

    python3 scripts/benchmark-agent-modes.py \
      --no-memory-url http://127.0.0.1:8787 \
      --memory-url http://127.0.0.1:8788 \
      --causal-url http://127.0.0.1:8789 \
      --token "$WANXIANG_API_TOKEN" \
      --expected-seed 42 --expected-tick 0 \
      --provider-label openai-compatible \
      --model-label '填写实际模型标识' \
      --repetitions 1 --confirm-provider-costs \
      --output artifacts/agent-retrieval-benchmark.json

脚本要求显式加上 --confirm-provider-costs 才会执行可能产生费用的请求；默认只执行一轮任务，需先从小规模评测开始。脚本会先检查三种模式的 readiness、Agent 是否启用、自动 tick 是否关闭，以及 seed、tick 和**完整初始快照**是否一致；这些检查必须在第一次模型调用前完成。报告包含每项任务的运行时完成率、确定性任务正确率、结构化答案合规率、工具策略合规率、证据一致率、工具错误、平均工具步数、平均/P95 延迟、世界是否保持不变，以及三种模式之间按任务和重复序号配对的描述性差异。任务集含跨任务记忆召回；召回任务禁止实时工具，答案由预先读取的权威状态和工具证据评分。Provider/model label 是操作员填写的记录字段，不由服务器认证；不要把密钥放在参数或报告内。

报告中保留了 Agent 回答用于人工复核。它不是对语义正确性、通用智能、记忆有效性或因果推断能力的证明，也不自动反映实际账单；费用与 Token 用量需从相应 provider 的控制台单独核对。

## 5. 运行三模式 Agent 配对评测（GitHub Actions）

仓库的 Actions 中提供手动工作流 **Agent retrieval-mode A/B benchmark**。先在 Settings → Secrets and variables → Actions 配置：

- `WANXIANG_LLM_ENDPOINT`：可从 GitHub-hosted runner 访问的 HTTPS OpenAI-compatible endpoint。
- `WANXIANG_LLM_MODEL`：该 endpoint 支持的模型标识。
- `WANXIANG_LLM_API_KEY`：如 provider 需要认证则设置；无密钥模式可留空。

不要将密钥写入仓库、workflow 参数或评测报告。工作流不会打印上述配置值；若 endpoint 或模型未配置，会在任何 Agent 任务请求前失败。它在同一个 CI runner 内启动三套使用隔离数据目录的 API 实例，统一 seed 与 `WANXIANG_TICK_MS=0)，分别设置 `no-memory`、`memory` 和 `causal`，预检三套世界后才开始模型请求。

从 Actions 页面手动运行 **Agent retrieval-mode A/B benchmark**。费用确认复选框默认关闭；只有在确认 provider 可能产生费用后才勾选。首次使用建议 `repetitions=1`（每种模式四个固定任务，即 12 次 Agent 任务请求；每个任务可能触发多次模型请求），seed 默认是 42。工作流将配对评测 JSON 作为 `agent-mode-benchmark-report` artifact 保存 14 天。任务请求数不等同于模型调用数或实际费用；完成后应到 provider 控制台核对 Token、调用量和账单。若未勾选费用确认，不会启动服务器或调用模型。

## 6. 备份、恢复和凭据轮换

世界状态、Agent 记忆和会话检查点保存在 Docker named volume 中。升级前先安排维护窗口，并对数据目录做一致性备份。推荐步骤：

1. 用 docker compose ... stop wanxiang-live 暂停写入。
2. 获取停止后的容器 ID：docker compose ... ps -aq wanxiang-live。
3. 使用临时容器通过 --volumes-from 只读挂载该容器的卷，把 /app/.wanxiang-live-data 打包到独立、加密且受访问控制的备份目录。
4. 备份成功后重新启动 API，并验证 /ready、/api/state 和 checkpoint 导出。
5. 至少在隔离环境实际演练一次恢复；只确认备份文件存在并不算恢复验证。

备份可能包含模拟状态、Agent 记忆和用户提交的内容，必须限制访问并设置保留期限。请勿把状态卷或备份推送到公共 Git 仓库。

轮换 WANXIANG_API_TOKEN 时，先在受控维护窗口更新服务器 .env.live，重新创建 API 容器，然后验证新 Token 成功、旧 Token 失败，并同步更新所有受信调用方。由于当前是共享 Token，轮换会影响所有客户端。

## 7. 发布阻断项与故障处理

- /ready 返回 503：先查看 persistence 状态和容器日志；不要把 readiness 降级当作健康服务继续承载写操作。
- HTTPS 失败：检查 DNS A/AAAA、主机入站 80/443、防火墙及 Caddy 日志。
- API 返回 401：检查 Bearer Token 是否传入、是否与当前服务器配置一致；不要为了排障临时关闭认证。
- 浏览器 CORS 错误：将前端的 scheme + host + port 与 WANXIANG_CORS_ORIGIN 精确比较；不要改成 *。
- HTTP 429：查看响应中的 route、limit_per_minute 和 Retry-After；适当降低调用频率或调整限额，不要通过反复重启来绕过配额。
- 模型操作失败：检查服务端 provider 配置、模型权限、速率/额度和网络超时；不要在日志或 issue 中粘贴密钥。
- 持久化写入失败：停止进一步变更操作，保全日志和备份，按恢复流程在隔离副本上验证后再恢复服务。

当前 Compose 模板包含非 root 容器、受限能力和 readiness 检查，但还不等同于完整的生产平台。公开面向多用户之前，必须另行补足和验证用户级授权、请求限流/资源配额、集中访问日志与告警、provider 费用治理、凭据管理及恢复演练。
