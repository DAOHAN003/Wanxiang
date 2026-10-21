# 万象 · Wanxiang

**万象是一套以 Rust 编写的、由种子驱动的可复现世界模拟引擎，面向长程世界状态演化、规则对照实验与互动叙事。** 仓库还包含独立的因果图、记忆、Agent 运行时和模型适配研究组件。

> **项目阶段：早期实验性公开版本（0.1.x）。** 当前模块成熟度不一，接口可能调整。可复现的模拟结果不等于现实预测正确；因果工具不等于完整因果推断；安全辅助代码也不是生产级隔离边界。

> **直接体验：** [打开在线世界模拟实验室](https://daohan003.github.io/Wanxiang/)。静态演示由 Rust SDK 在网站构建时生成 seed 42、100 tick 的样例，可直接查看基准世界与规则变体，不需要登录、模型 API Key 或本地 Rust 环境。角色移动、矿点争夺、交易、生产、实时推进与 Agent 操作需要另行运行或部署 Rust API；仓库目前不提供默认的公共实时后端。

## 万象主要在做什么

万象的主工作区围绕一个具体问题构建：给定世界规则、初始设定和随机种子，能否推进一个结构化世界，并对输出快照、事件、资源账本及不同规则下的轨迹进行检查和比较？

- **可复现的世界模拟**：用显式 seed 创建世界，以 tick 推进模拟，并导出世界快照、事件和报告。
- **数据驱动的世界设定**：通过 `GenrePreset` 描述地点、势力、角色和参数；新增设定主要改配置，而不是把具体小说题材写死在引擎里。
- **规则对照与结构化比较**：在相同 seed 和相同推进步数下，比较基准规则与变更规则产生的模拟结果。
- **认知与 Agent 研究组件**：独立的引擎工作区包含因果图、记忆、行动选择、Agent 工具工作流、LLM 适配及叙事辅助等模块。

万象当前最值得验证的主张不是“拥有最多的模块”，而是**可复现的世界状态演化与可对照实验，能否为互动叙事、模拟和 Agent 应用带来可测量的帮助**。

第一阶段的产品目标是让开发者无需连接实时 API，就能在静态演示中比较基准轨迹与规则变体、检查事件和差异，并导出包含两份快照与结构化 diff 的离线 JSON 报告。产品验收门槛见 [产品闭环与验收标准](docs/PRODUCT_ACCEPTANCE.md)。

## 快速体验

**想先判断万象是否适合你的工作？** 打开[在线世界模拟实验室](https://daohan003.github.io/Wanxiang/)，在无需登录或连接 API 的静态模式下切换“基准世界 / 规则变体”，查看指标与事件，再点击“导出对照实验报告 JSON”。导出文件包含实验元数据、两份世界快照和结构化差异，便于离线检查和分享；它不是包含完整操作历史的 checkpoint 回放包。实时 tick、世界写操作和 Agent 仍需独立运行 Rust API。

要求：稳定版 Rust 工具链和 Cargo。

**这些世界模拟与示例不需要 LLM、模型下载或 API Key。** `wanxiang_sdk` 依赖本地的类型、世界和引擎 crate，不依赖 `engines/llm_adapter`；模拟逻辑会在本地根据规则和 seed 产生状态、事件和报告。首次构建时 Cargo 仍需要按依赖配置下载 Rust crate；运行示例本身不需要调用远程模型。

生成一份无需 LLM 的确定性事件/章节骨架：

```bash
cargo run --locked -p wanxiang_sdk --example gen_world -- --demo 7 100 12.0 4
```

参数依次为演示模式、seed、模拟 tick 数、吞噬强度参数和章节数。输出基于模拟事件整理成章节骨架与事件表；它是规则驱动的叙事素材，不是大模型写出的完整自然语言小说。

比较两条世界轨迹：

```bash
cargo run --locked -p wanxiang_sdk --example rule_compare
```

此示例使用同一 seed 和相同 tick 数，改变一个世界规则参数，再输出两个世界的概要与结构化差异。这展示的是**模拟模型内的规则敏感性实验**，不是对现实世界因果效应的证明。

运行两个工作区的测试：

```bash
cargo test --locked --workspace
cargo test --locked --workspace --manifest-path engines/Cargo.toml
```

也可以运行 `bash scripts/test-all.sh`，或在 Windows PowerShell 中运行 `./scripts/test-all.ps1`。请以干净 checkout 的 CI 结果为准。

## SDK 兼容性与性能基线

`wanxiang_sdk` 当前仍处于 0.1.x 预览阶段；Rust API 与未版本化 JSON/checkpoint 不承诺跨版本永久兼容。接口分级、升级检查和回放迁移边界见 [SDK 兼容策略](docs/SDK_COMPATIBILITY.md)。

运行小/中/大合成世界的 release 性能基线（构建、tick 推进、快照导出与 checkpoint 重建）：

```bash
python scripts/run-sdk-performance-baseline.py --ticks 10 --repeats 3 --output sdk-performance-baseline.json
python scripts/test-sdk-performance-baseline.py sdk-performance-baseline.json
```

参数分别是 tick 周期与重复次数。基线结果作为 CI artifact 保存；在同一 runner 与构建配置建立足够样本前，不会用不稳定的固定耗时阈值阻断 CI。更多说明见 [性能基线方案](docs/PERFORMANCE_BASELINES.md)。

## 工作区结构

| 路径 | 作用 |
|---|---|
| 根目录 `Cargo.toml` | 世界模拟工作区：类型、核心编排、认知组件、世界模拟、引擎与嵌入式 SDK |
| `wanxiang_sdk` | 面向下游程序的 `WorldModel` 公共入口、题材预设、快照和比较 API |
| `wanxiang_agent_bridge` | 把 Agent 的受限工具执行连接到真实 `WorldModel`，提供 world info、snapshot 和有步数上限的 step 工具 |
| `engines/Cargo.toml` | 独立的研究型 crate 工作区：`world_state`、`causal_core`、`memory_core`、`agent_runtime`、`llm_adapter`、`novel_writer_suite`、`security_suite` 等 |

根目录与 `engines/` 仍是独立 Cargo 工作区，但 `wanxiang_agent_bridge` 已提供一个明确的跨工作区集成点：Agent 可调用真实世界模拟器的只读查询和有界 tick 推进工具。`engines/agent_runtime` 的规划器每轮可检索 `memory_core` 记忆，并可使用关联因果图进行启发式排序。其他研究 crate 仍不能仅凭存在就视为已集成，具体边界见 [能力闭环审计](docs/CAPABILITY_CLOSURE_AUDIT.md)。

## 当前边界

- 相同 seed、规则和步数旨在让当前实现中的模拟轨迹可复现；这不保证跨版本或跨平台的长期兼容性，除非相应契约另有测试与承诺。
- 规则分支/比较会基于保存的初始化配方和公开操作记录重建世界，支持题材预设、调用方注入的初始实体以及实例级认知层开关。`fork` 会重放原有操作以恢复当前状态；`branch` 则会在修改后的规则下从初始条件重放整个操作历史。因此规则分支对比的是两条完整模拟轨迹，**不是只从当前 tick 开始生效的规则变更**。该机制是操作重放而非完整运行时内存快照，新增操作类型也必须同步维护重放逻辑与回归测试。
- `world_state` 的资源池是抽象账本，不是物理学中的能量守恒或热力学熵模型。
- `causal_core` 的部分传播与反事实计算采用简化模型；图抽象使用有规模限制的排列搜索，不是完整 VF2 实现。
- `memory_core` 的确定性稀疏编码不自动等于语义理解；检索质量仍需要带标签的数据集和基线对比。
- `security_suite` 提供辅助检查与策略代码。词法路径检查不能替代 OS 权限控制、符号链接处理或进程隔离。
- 叙事一致性与角色相关分数是启发式信号，不等于故事质量或可靠语义理解。经典算法本身也不应被描述为万象原创。

进一步阅读：[架构说明](docs/ARCHITECTURE.md)、[算法状态与限制](docs/ALGORITHM_STATUS.md)、[安全模型](docs/SECURITY_MODEL.md)、[能力闭环审计](docs/CAPABILITY_CLOSURE_AUDIT.md)、[工程路线图](docs/ROADMAP.md)。

## 参与方式

小而可验证的改动优先。请为行为修复添加回归测试；对算法改进提供适用假设、评测数据和可复现的基线。参见 [贡献指南](CONTRIBUTING.md)。

## 许可证

Apache License 2.0，详见 [LICENSE](LICENSE)。

## 实时运行：Rust API、自动推进、实验与 Agent

GitHub Pages 仍是静态托管。实时模式由独立 Rust 进程运行真实的 `wanxiang_sdk::WorldModel`，网页通过 HTTP 读取和控制该模拟器；这不是用 JavaScript 仿写的模拟，也不是静态 JSON 假装实时。仓库还提供 `Dockerfile.live` 供容器部署。

启动本地实时 API（不需要 LLM）：

```bash
cargo run --locked -p wanxiang_agent_bridge --example live_server
```

服务默认仅绑定 `127.0.0.1:8787`，首次启动默认创建中性 `Custom` 世界，不再默认绑定仙侠题材；可通过 `WANXIANG_WORLD_TYPE=xianxia|historical|scifi|urban|western` 选择题材，或用 `WANXIANG_WORLD_PRESET=shitian` 显式加载食天界剧本，并可用 `WANXIANG_WORLD_SEED` 设定初始种子。上述配置仅在首次创建世界时生效；如果运行目录已经有检查点，服务会优先恢复存档而不会静默覆盖它。并默认会独立于浏览器每秒推进一个 tick，关闭网页后模拟仍继续运行。设置 `WANXIANG_TICK_MS=0` 可关闭服务端自动推进，改用请求手动控制。打开在线实验室，展开“实时 Rust 引擎”，确认 endpoint 为 `http://127.0.0.1:8787`，点“连接”后可查看不断更新的 tick、手动额外推进、运行规则干预对照实验。运行实验会在独立分支里推进基准与变体，在线世界本身保持不变。页面没有连接后端时仍显示 GitHub Actions 生成的静态快照。

HTTP 接口：`GET /health`（进程存活）、`GET /ready`（存储状态正常时才返回 200；持久化降级时返回 503）、`GET /api/state`、`POST /api/step`（`ticks=1..100`）、`POST /api/experiment`（安全白名单规则、`value=0..1`、`ticks=1..1000`）和可选 `POST /api/agent`。为避免长期运行后每秒轮询的数据随事件历史无限增长，API 将事件数组限制为最近 200 条、实体数组限制为最近 500 个、势力数组限制为 100 个；完整总数和截断标志放在快照的 `meta.total_events`、`meta.total_entities`、`meta.alive_entities`、`meta.total_factions` 等字段中。请求 JSON 格式见 `scripts/evaluate-live-api.py`。服务端最多同时处理 64 个 TCP 请求连接；达到上限时返回 503，慢客户端不会无限创建线程。`/health` 与 `/api/state` 会返回 `persistence.degraded`，分别标识世界检查点、Agent 记忆与会话存储是否降级；`/ready` 在任一持久化环节降级时返回 503。出现降级时先检查服务日志和数据卷，不要把内存状态误认为已可靠落盘。成功的 POST 支持 `Idempotency-Key`；相同 key + 相同请求在 10 分钟窗口内返回缓存响应，不重复执行；同 key 不同 body 返回 409。缓存默认最多保留 512 条未过期记录，可通过 `WANXIANG_IDEMPOTENCY_MAX_ENTRIES` 在 1–100000 范围内调整；达到上限时，带新 key 的写请求返回 503 和按最早过期记录计算的 `Retry-After`，且不会执行操作或淘汰仍有效的幂等记录。待旧记录过期后，客户端可使用原 key 重试。运行状态和幂等记录保存在 `.wanxiang-live-data/runtime-state.json`，以原子替换写入。检查点保存世界初始化配方和可重放操作，启动时重新构造并重放；这不是内存对象的字节级转储。启用 Agent 时，NMM 的记忆记录、SDR 向量、调质状态、可选因果图与 ID 计数器会在每次 Agent 请求后单独原子写入 `.wanxiang-live-data/agent-memory.json`，并在下次启动时恢复。它不会随着每个 tick 重写，以免频繁写入较大的向量集合。



旅行 API 与网页按钮：

- `POST /api/plan-travel` 接收 `{"character_id":"...","destination_id":"..."}`，只返回估算，不推进 tick、不移动角色、不消耗补给；响应包含路线、预计 tick、风险分数、预测遭遇、补给可用量/需求量与可承担状态。
- `POST /api/travel` 执行模拟旅行：逐段推进 tick、扣减抽象补给，并将遭遇/移动事件和旅行摘要写入检查点。补给不足在出发前拒绝；相同 `Idempotency-Key` 在有效窗口内回放原响应。
- `POST /api/move` 保留即时移动语义，仅允许已声明的直连边，不推进模拟时间。
- `POST /api/production/order` 配置或更新持续生产单，接收 `faction_id`、`location_id`、`output_resource` 和 `workers`；传统 `input_resource` 仍兼容（每批消耗 1 单位），也可传 `inputs` 对象定义 1–16 种原料及每批用量，并用 `output_amount` 配置每批产量。`skill_level=0..10` 是抽象班组熟练度，每级提升 10% 批次吞吐，不等同于角色战斗境界；`wage_resource` 与 `wage_per_worker` 可为每名实际生产工人按 tick 发薪，工资从势力资源池扣除并转入地点库存。投入、工资或劳动力不足时订单暂停；`POST /api/production/cancel` 停止指定生产单。
- `GET /api/state` 返回 `production_orders`、`production_sites` 与 `production_settlements`。结算账本记录最近一个模拟 tick 中实际消耗的各类原料、产出数量、实际工人数、熟练度与工资转移；下一 tick 后它会更新，完整历史仍可从世界事件流追溯。配置/取消操作写入可重放检查点，支持 `Idempotency-Key`。
- 可分享回放包：`GET /api/checkpoint` 返回 `wanxiang-replay-v1` 包，包含 seed、完整操作序列、checkpoint 与导出时的最终 `WorldExport`。实验室可下载 JSON 文件，也可生成 gzip 压缩的 URL 片段；接收者不需要连接原 API 即可离线查看该次模拟的最终快照和事件流。链接展示的是导出时状态，完整重新计算可以使用仓库提供的验证示例：`cargo run --locked -p wanxiang_sdk --example verify_replay -- replay.json`；它会用 `WorldModel::from_checkpoint_json` 回放操作历史，并逐项比较重建后的检查点与导出快照。分享链接不携带 API token，也不会授予实时控制权限。
- 矿点争夺与开采：`GET /api/state` 返回 `mines`，每个矿点绑定地点已有的资源库存；`POST /api/mine/claim` 占领无人控制的矿点，`POST /api/mine/challenge` 按双方在场存活角色的 `max(power_rank, 0) + max(power_value, 0)` 合力和地点防御值计算争夺结果，`POST /api/mine/extract` 仅允许控制方把矿点库存转入所属势力资源池。三类操作均校验角色所在地点、写入事件、支持 `Idempotency-Key` 与 checkpoint/replay；占领和开采不推进 tick，开采前后资源总量必须守恒。当前矿点是抽象资源控制规则，不含 PvP 战斗过程、运载/物流成本或真实地理语义。
- Agent 同样提供只读 `world_plan_travel` 与执行工具 `world_travel_character`，便于先评估再执行。风险分数未校准为现实概率；旅行补给目前是独立的抽象预算，尚未和地点资源库存建立守恒联动。


Agent 可恢复会话：\`POST /api/sessions\` 用 \`{"task":"..."}\` 创建并返回 \`session_id\`；\`POST /api/sessions/{id}/drive\` 推进执行，待批准时状态会返回 \`awaiting_approval\`；提交 \`{"decision":"approve"}\` 或 \`{"decision":"reject"}\` 继续/终止待批准动作；\`GET /api/sessions/{id}\` 查询完整状态与历史。会话 JSON 原子存入 \`.wanxiang-live-data/sessions.json\`，启动时恢复，TTL 为 24 小时且最多保留 256 个会话。请求级 \`Idempotency-Key\` 可防止重复 drive。当前注册的世界工具不含 shell/任意文件等高风险动作，因此常规世界任务通常不会触发审批；会话接口为需要审批的工具工作流保留了显式暂停/恢复路径。

实时 LLM 流式接口（需要服务器配置 provider）：

\`\`\`bash
curl -N http://127.0.0.1:8787/api/llm/stream \\
  -H 'content-type: application/json' \\
  -d '{"prompt":"用一句话介绍万象世界模拟器"}'
\`\`\`

上游 SSE 文本块会经 HTTP Server-Sent Events 即时转发给浏览器；数据事件类型为 \`token\`、\`done\`、\`error\`、\`end\`。配置 \`WANXIANG_API_TOKEN\` 时还需要 \`Authorization: Bearer ...\`。该流式路由直接请求 provider，不调用世界工具；带工具循环的 Agent 保持使用同步 \`POST /api/agent\`。普通 JSON provider 调用执行最多两次 curl 短重试；SSE 请求不自动重放，防止部分响应后重复计费。

配置可选的服务端 LLM Agent（provider key 不要放进浏览器）：

```bash
export WANXIANG_LLM_ENDPOINT="https://api.openai.com/v1"
export WANXIANG_LLM_MODEL="your-model-name"
export WANXIANG_LLM_API_KEY="your-api-key"
cargo run --locked -p wanxiang_agent_bridge --example live_server
```

服务端使用系统 `curl` 处理 HTTPS/TLS；需要安装 curl 和 CA 证书。初始世界配置通过 `WANXIANG_WORLD_TYPE`（默认 `custom`）、`WANXIANG_WORLD_PRESET`（可选，当前内置 `shitian`）和 `WANXIANG_WORLD_SEED`（默认 `42`）指定。它们只在没有现存检查点的首次启动时使用；切换环境变量不会改写已有世界。若要从旧的仙侠存档换成中性新世界，请先备份数据卷，再按部署流程有意重置运行状态文件/数据卷，切勿把初始化配置变更当作存档迁移。服务器 tick cadence 可通过 `WANXIANG_TICK_MS` 设为 0–60,000 毫秒；默认 1,000 毫秒。CI 显式设置 0，以便验收请求的精确 tick 计数。也可指定 `WANXIANG_LLM_TIMEOUT_MS`（1,000–120,000）。如果没有配置 endpoint 和 model，模拟 API 仍可用，Agent 路由返回 503。该接入走 OpenAI-compatible Chat Completions JSON 请求；不同 provider 的兼容程度需要针对目标 provider 验证。

容器构建：

```bash
docker build -f Dockerfile.live -t wanxiang-live .
docker run --rm -p 8787:8787 \
  -e WANXIANG_BIND=0.0.0.0:8787 \
  -e WANXIANG_API_TOKEN="replace-with-a-long-random-secret" \
  -e WANXIANG_CORS_ORIGIN="https://daohan003.github.io" \
  -v wanxiang-live-data:/app/.wanxiang-live-data \
  wanxiang-live
```

非 loopback bind 必须设置 `WANXIANG_API_TOKEN`。在公开网络部署时，须在可信反向代理后提供 HTTPS、设置强随机 token、限制 CORS 来源并保护持久化卷；服务未实现用户账户、多租户授权或分布式锁，不能把 bearer token 当成完整生产身份系统。HTTP API 不开放 shell 或任意文件工具，规则干预只接受白名单中的五个归一化参数。


可选择 Agent 的记忆检索基线，便于做同任务对照：设置 \`WANXIANG_AGENT_RETRIEVAL=no-memory\`（不把记忆注入 planner）、\`memory\`（向量/记忆检索）或 \`causal\`（记忆 + 因果图融合，默认）。做对照实验时请用独立 \`WANXIANG_DATA_DIR\`，保持同一 seed、同一模型配置和 \`WANXIANG_TICK_MS=0\`，避免不同实例共享同一世界文件。

Agent 检索对照基准需要真实模型 API，会产生模型调用费用：

\`\`\`bash
python -m py_compile scripts/benchmark-agent-modes.py
python scripts/benchmark-agent-modes.py \\
  --no-memory-url http://127.0.0.1:8791 \\
  --memory-url http://127.0.0.1:8792 \\
  --causal-url http://127.0.0.1:8793
\`\`\`

在三个独立服务中分别设置 \`WANXIANG_AGENT_RETRIEVAL=no-memory|memory|causal\`，且各自使用独立端口和数据目录。基准会报告各模式的成功率、工具错误步骤、平均工具步数、平均/P95 延迟及任务执行前后是否意外改动在线世界；它是运行时对照而不是语义真值证明。

固定种子、多世界规则干预基准不需要 LLM：

\`\`\`bash
cargo run --locked -p wanxiang_sdk --example world_lab_benchmark -- 100
\`\`\`

它针对四个固定 seed 验证同 seed 回放确定性、规则干预差异及守恒账本。可以把 tick 数提高到 1000 检查长程敏感性。

运行实时 API 验收与复现实验：

```bash
python scripts/evaluate-live-api.py --base-url http://127.0.0.1:8787
# 如果已配置 LLM，可额外调用 Agent:
python scripts/evaluate-live-api.py --base-url http://127.0.0.1:8787 --agent
```

验收覆盖无效参数不推进、规则实验不改变基线、重复实验可复现、守恒账本检查以及相同幂等 key 不会重复推进。实验里的规则差异只描述万象模型内的干预结果，不构成现实世界的因果证明。


## Interactive world rendering

The browser lab renders a deterministic faction/character structure view, source-backed trade and war relationships, location routes, resident markers based on `location_id`, and an event-by-tick timeline. Generic seeded worlds use an explicit abstract path rule based on theme location order; presets can author their own links. See [World rendering](docs/WORLD_RENDERING.md). Location positions are still layout coordinates only: the schema has no physical coordinates, so the UI does not claim to be a geographic map.

## Reproducible experiment kernel

The wanxiang_experiment crate provides a WorldEnvironment protocol and SDK adapter, paired baseline/intervention experiments, named invariant checks, a deterministic linear SCM reference implementation, and typed paired Agent-trial comparisons. Its initial world adapter changes energy_density on a seeded Xianxia world and emits both snapshots, the model diff, and invariant results. Agent comparison records are a protocol/aggregation layer; adapters still need to execute real Agents and record outcomes.

Run cargo run --locked -p wanxiang_experiment --example run_experiment -- 42 100. An optional third argument sets energy_density in 0..=1. The report is reproducible for a fixed specification and simulator build. These checks do not establish real-world causal validity or physical conservation. See docs/EXPERIMENT_KERNEL.md and docs/WANXIANG_POSITIONING.md.


## VPS 部署（Docker Compose + 自动 HTTPS）

实时 API 现在提供可复现的 Compose 部署配置。GitHub Pages 仍然只负责静态前端；实时模拟必须运行在自己管理的 VPS/服务器上。公开访问使用 Caddy 自动申请/续期 HTTPS 证书，Rust API 只映射到宿主机 loopback，公网请求经反向代理进入。

先在服务器安装 Docker Engine 与 Docker Compose 插件，并将仓库检出到服务器。然后配置域名 DNS 的 A/AAAA 记录指向该服务器，确保公网 TCP 80、443 端口可达：

```bash
cp .env.live.example .env.live
# 编辑 .env.live：将 WANXIANG_DOMAIN 改为真实域名；
# 将 WANXIANG_API_TOKEN 设置为至少 32 字节随机值。
openssl rand -hex 32
python3 scripts/validate-live-deployment-env.py .env.live
docker compose --env-file .env.live -f compose.live.yml --profile public config --quiet
docker compose --env-file .env.live -f compose.live.yml --profile public up -d --build
docker compose --env-file .env.live -f compose.live.yml --profile public logs --tail=100 wanxiang-live caddy
```

将 GitHub Pages 实验室中的 **API endpoint** 填成 `https://你的域名`，Bearer token 填入部署时的 `WANXIANG_API_TOKEN`。该 token 只应由受信任的测试者持有；不要把它写进 `site/index.html`、提交到仓库或放入浏览器代码。服务持久化数据位于 Docker named volume `wanxiang_live_data`，Caddy 证书数据单独保存。

这套配置适合自托管测试和受控 beta，不等同于面向匿名公众的生产 SaaS：目前仍是共享 bearer token，没有用户/租户授权或分布式部署协调。实时 API 可对 Agent、规则实验、LLM 流式请求和会话驱动路由配置实例级限流，并在 /health 返回限流计数和配置；这不是按用户配额，多实例之间也不共享计数。若要公开给任意访客使用，应先加入用户身份、权限和跨实例配额协调，并对备份恢复、并发负载、provider 费用和故障恢复做实际验收。此配置不会自动创建 VPS、DNS 或替你启动公网服务。

完整配置校验、HTTPS/认证验收、数据备份、Token 轮换和故障处理步骤见 [实时 API 部署与运维手册](docs/OPERATIONS_RUNBOOK.md)。三种 Agent 检索模式的对比基准见 scripts/benchmark-agent-modes.py；它会实际调用模型，执行前会先确认 seed、tick 和完整初始快照一致，且可能产生 provider 费用。

## 在线演示与免费部署

万象提供一个基于静态页面的世界快照实验室。页面展示 GitHub Actions 在构建阶段调用 Rust SDK 生成的真实模拟结果，并对照基准规则与变更规则下的两条轨迹；浏览器只负责读取和展示 JSON，不会伪装成正在运行 Rust 模拟内核。

该方案不需要租用服务器或购买模型 API。启用公开网站的步骤：

1. 确认 `.github/workflows/static.yml` 中的 `Deploy Wanxiang static demo` 工作流已通过检查，并合并相关部署改动。
2. 打开 [仓库 Pages 设置](https://github.com/DAOHAN003/Wanxiang/settings/pages)，在 **Build and deployment → Source** 选择 **Deploy from a branch**；Branch 选 **gh-pages**，Folder 选 **/(root)**，然后保存。
3. 向 `main` 推送改动后，**Deploy Wanxiang static demo** 工作流会生成模拟 JSON 并更新 `gh-pages` 分支；工作流成功后访问 [https://daohan003.github.io/Wanxiang/](https://daohan003.github.io/Wanxiang/)。

本地生成自己的真实规则对照快照：

```bash
cargo run --locked -p wanxiang_sdk --example web_demo_data -- 42 100 > demo.json
```

页面支持从本机导入 JSON；数据只在浏览器中读取。静态页面展示构建阶段生成的真实模拟快照，并渲染地点网络、角色驻地和事件时间线。要推进在线世界、执行路线移动或运行 Agent，必须连接单独运行的 Rust API；GitHub Pages 本身不运行后端，也不提供多人实时共享。

## 事件学习到世界动力学的反馈闭环

`LivingCore::tick` 会从本 tick 的真实事件文本生成学习结果，再通过 `AdaptiveFeedback` 将事件重要性映射为有界压力项、将学习模式/推理映射为较小的韧性项，并更新 `LifeLayer.ecosystem_health`。反馈在快照与回放记录前应用，结果包含在 `TickResult.feedback` 中。控制器有上下界和固定 seed 集成测试；这属于可解释的模拟反馈策略，不证明文本学习是真实因果知识。

## Agent 与真实世界模拟的集成闭环

## 路线移动与确定性寻路

`WorldModel::move_character` 执行单段移动：角色必须存活、拥有当前位置，且起点的 `connected_ids` 必须包含目标。`WorldModel::plan_location_route` 使用按地点 ID 排序的广度优先搜索，返回稳定的最短路径；同长度路径会按排序邻居规则确定唯一结果。`WorldModel::travel_character` 按路径逐段执行移动，每段都有独立的 `WorldChange` 事件和操作记录，不会自动推进世界 tick。

实时 API 提供 `POST /api/move`（单段）和 `POST /api/travel`（自动寻路）。两者都写入 checkpoint，并支持 `Idempotency-Key`，防止网络重试重复移动。静态网页的路线控件必须连接实时 API 才会执行操作；离线 JSON 只用于观察。地点导出还包含可选的本地模拟坐标 `WorldPosition { x, y, elevation }` 与地形标签：题材预设可以显式指定；通用 seeded spawn 用已有位置索引、危险度和资源丰度派生确定性抽象坐标，不额外消耗 RNG。网页用高程做 2.5D SVG 投影，缺少坐标时回退到拓扑布局。**这些坐标不是经纬度，地形标签也不意味着已经模拟真实地形。** 路线规划目前以连接边数量求最短路径，不包含真实距离、地形代价、旅行耗时或风险消耗。

`wanxiang_agent_bridge::WorldAgent` 将真实 `WorldModel` 暴露为受限工具，包括 `world_info`、`world_snapshot`、`world_step`、`world_move_character`、`world_travel_character` 和规则对照实验。`world_step` 单次最多推进 100 tick；移动工具只沿已声明路线执行，`world_travel_character` 通过确定性最短路规划逐段移动，并把每一段作为事件和可重放操作记录。工具返回值会进入下一轮规划 prompt，固定 seed 的脚本化 adapter 测试覆盖移动成功、非法路线拒绝、状态回读和回放。

## Agent 工具执行与记忆增强规划器

`agent_runtime::AgentRunner::run_with_tools` 可以接收实现 `ActionPlanner` 的决策器，执行注册工具，并把工具返回值放入下一轮决策历史。`MemoryBackedLlmPlanner` 将 `BaseLLMAdapter`、`memory_core` 检索和可选因果图检索连到这一流程，并把真实工具观察结果写回记忆供后续决策检索。检索到的记忆和工具输出在提示中被标为不可信数据，不能覆盖系统指令。模型必须返回指定 JSON 动作；模型调用失败或响应格式错误会 fail closed，不会退化成成功的 `FINISH`。

该路径仍有边界：Agent 可通过 `run_with_approval` 注入同步审批回调；未配置回调或审批拒绝时会在 `pending` 处停止。`ResumableSession` 提供可序列化的审批状态，但持久化、TTL、幂等和分布式并发控制属于应用层责任。记忆检索使用实验性稀疏表示，不保证语义相关性；Agent 观察写回默认保留最近 2,048 条记忆，可通过 `with_memory_limit` 调整；因果分数是启发式工具，不证明真实因果。`OpenAiCompatibleAdapter` 将 OpenAI-compatible 请求通过 `JsonTransport` 发送并归一化响应；核心 crate 不捆绑 HTTP/TLS 客户端，部署方必须提供具备 TLS、超时和网络错误处理的 transport。通用适配器的 SSE 流式调用明确返回 `NotImplemented`，不能视为已验证的真实 provider 流式接入。当前 bridge 是最小工具层集成，不代表所有 `engines/` crate 已统一编排。

## English

Wanxiang is an experimental Rust project focused on **seeded, reproducible world simulation** for long-running state evolution, rule-comparison experiments, and interactive narrative. A separate engine workspace contains research components for causal graphs, memory, agent runtimes, model adapters, and narrative tooling.

Run the rule-comparison example with `cargo run --locked -p wanxiang_sdk --example rule_compare`, or test both workspaces with the commands above. The example compares trajectories produced by the current simulation model; it is not evidence of real-world causal validity.

This is an early 0.1.x snapshot with uneven maturity across modules. Reproducibility does not establish predictive accuracy, simplified causal utilities are not complete causal inference, and security helpers are not an OS sandbox. See the architecture, algorithm-status, and security-model documents for scope and limitations.


## Regression acceptance checks

The SDK test suite includes a 1,000-tick fixed-seed replay test that compares equivalent operation histories byte-for-byte, checks the resource-ledger invariant, and verifies that a changed rule creates a distinct replayed trajectory. CI runs positive/negative tests for the versioned static-demo JSON schema, validates the generated payload, checks that both exported worlds report a conserved ledger, and verifies that the rule-diff payload identifies the changed rule. These tests establish properties of the implemented simulation model; they do not validate real-world predictive or causal accuracy.
