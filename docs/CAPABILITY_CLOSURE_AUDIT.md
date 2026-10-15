# 能力闭环审计

本文按可验证的数据流记录当前集成状态。“闭环”不仅指 crate、trait 或算法存在，还要求输入实际进入组件、动作真实执行、结果反馈到后续步骤，并由自动化端到端测试保护。对外部 provider、持久化和现实因果有效性的声明严格区分“接口已实现”和“环境已验证”。

## 状态表

| 链路 | 当前状态 | 仓库证据 | 未完成项 |
|---|---|---|---|
| 规则/seed → world ticks → 状态/事件/快照 | 主工作区已实现 | `wanxiang_sdk::WorldModel`、`wanxiang_engine::LivingCore::tick`、`wanxiang_world::WorldSimulator` | 增加更多面向用户场景的验收测试，固定关键不变量 |
| 相同初始条件 → 规则变更 → 重放 → 轨迹比较 | SDK 层已实现 | `WorldModel::branch`、`rule_compare`、`web_demo_data` | 覆盖所有公开操作类型和自定义初始实体的回归测试；结果只说明模型内敏感性，不证明现实因果 |
| 事件 → 学习结果 → 有界反馈 → 生态状态 → 后续 tick | 已实现并受测试保护 | `wanxiang_engine::feedback::apply_event_learning_feedback`；`TickResult.feedback`；固定 seed 集成测试 | 当前是显式启发式反馈，不是经实证校准的生态模型；仍需敏感性实验、稳定性分析，并验证长期轨迹不违反既有不变量 |
| Agent 决策 → 工具执行 → 观察 → 下一轮决策 | 已实现 | `AgentRunner::run_with_tools`、`run_with_approval` | 生产环境仍需根据业务定义持久化 trace、超时和并发策略 |
| 审批 → pending → 跨请求恢复 → 执行/拒绝 | 已实现为可序列化状态边界 | `ResumableSession`、`ApprovalDecision`；序列化后恢复的单测；硬安全阻止不可被审批覆盖 | durable session store、分布式锁、TTL/幂等策略属于应用层；当前不是分布式审批服务 |
| LLM adapter → 结构化动作解析 → Agent 执行 | 已接入可注入规划器 | `MemoryBackedLlmPlanner` 使用 `BaseLLMAdapter`，要求 JSON 动作，解析/适配器错误 fail closed | 真实 provider 的网络、SSE、quota、TLS、retry/fallback 仍需环境级测试 |
| OpenAI-compatible provider → 请求/认证 → 响应校验 →统一 ChatResponse | 契约已实现 | `OpenAiCompatibleAdapter` + `JsonTransport`；请求序列化、Bearer auth、401/403/429/非 2xx、空响应和 usage 解析测试 | 核心 crate 不绑定具体 HTTP runtime；应用需提供实际 HTTP transport。生产 provider 行为仍需真实环境验证 |
| Agent + 记忆 + 因果检索 + 工具执行 | 已实现并有端到端测试 | `MemoryBackedLlmPlanner` 每轮检索 `NmmUnified`，可使用 `CausalGraph`；工具观察写回记忆并进入下一轮 prompt | SDR/启发式排序不保证语义检索质量；因果图只影响启发式评分，不构成真实因果证明 |
| Agent + world_state + causal + memory + LLM + tools | 已形成单一确定性研究闭环 | `agent_runtime::ResearchLoop`；测试从 `XianxiaWorld` 世界定义，经 causal graph、NMM memory、scripted LLM、`world_info` tool 到 Agent 结果 | 这是研究工作区的组合边界；根工作区 `WorldModel` 仍通过 `wanxiang_agent_bridge` 单独集成，两者不应被描述成同一个 runtime |
| Agent/记忆/因果 → 根工作区世界模拟 | 已有受限工具桥 | `wanxiang_agent_bridge::WorldAgent` 注册 `world_info`、`world_snapshot`、有界 `world_step`；脚本化 adapter 验证推进结果进入下一轮 prompt | 尚非双向语义规划器；真实 provider 和生产审批存储仍由部署层负责 |
| Rust SDK → JSON → schema/metadata 校验 → 静态网页 | 已有生成与 CI 展示流程 | `web_demo_data`、`.github/workflows/ci.yml`、`site/index.html` | 仍缺真实浏览器自动化 fixture；当前 CI 校验 JSON 内容和 JS 语法，不模拟浏览器交互 |
| 整个仓库 | 双工作区、明确边界 | 根工作区和 `engines/` 工作区分别构建；`agent_runtime::ResearchLoop` 和 `wanxiang_agent_bridge` 提供两条明确集成路径 | 不是单一 Cargo workspace；不能把独立 crate 的测试结果夸大为整个系统的生产级端到端证明 |

## 当前可以准确宣称的能力

- 主工作区提供带 seed 的世界模拟 SDK，可推进世界、导出快照并对比规则分支。
- Agent runtime 提供真实注册工具执行、工具观察反馈、审批策略、trace 记录和可序列化的跨请求审批状态。
- `MemoryBackedLlmPlanner` 每轮决策前检索 `memory_core`，并把实际工具观察写回记忆；可选 `causal_core` 参与启发式评分。检索记忆和工具输出在 prompt 中被标记为不可信数据。
- 无效 JSON、adapter 调用错误和工具执行错误均 fail closed，不会被转换成成功的 `FINISH`。
- `OpenAiCompatibleAdapter` 定义了可替换的 provider 请求/响应契约，但没有伪装成已经完成的真实网络接入。
- `agent_runtime::ResearchLoop` 提供一条确定性研究闭环：world definition → causal graph → NMM memory → LLM action planner → guarded tool execution → observation memory。
- `wanxiang_agent_bridge` 将 Agent 与根工作区真实 `WorldModel` 连接起来；这是一条受限工具边界，不代表所有研究 crate 都运行在同一个进程或同一个 Cargo workspace。
- 独立 crate 的 CI 通过不能证明模型输出质量、provider 可靠性、语义记忆效果、现实因果有效性或生产级安全隔离。

## 剩余工程化边界

1. 为真实 provider transport 增加可选的生产实现，并在隔离环境验证 TLS、超时、重试、429、SSE、quota 和 fallback；当前核心库保持 provider-neutral。
2. 为 `ResumableSession` 提供应用级 durable store、TTL、幂等键和并发锁；核心库只负责可序列化状态机。
3. 对 world feedback 做长程敏感性/稳定性实验，并明确哪些不变量必须始终成立。
4. 为 SDK/world bridge 增加更多固定 seed 的验收 fixture；浏览器层如需要真实 UI 保障，再增加 Playwright 等环境级测试。
5. 为 memory retrieval 建立带标签的评测集和基线，避免把 SDR 相似度描述成语义理解。
6. 因果组件继续明确“模型内干预计算”与“现实世界因果识别”的边界。

## 每条闭环的验收标准

每条能力都必须记录：(a) 入口；(b) 相连组件；(c) 反馈到下一轮的状态/输出；(d) 停止和错误行为；(e) 一项连接断开时会失败的自动化测试。外部依赖还必须记录“契约已实现”与“真实环境已验证”两个不同状态。
