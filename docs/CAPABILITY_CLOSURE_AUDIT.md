# 能力闭环审计

本文按可验证的数据流记录当前集成状态。“闭环”不仅指 crate、trait 或算法存在，还要求输入实际进入组件、动作真实执行、结果反馈到后续步骤，并由自动化端到端测试保护。

## 状态表

| 链路 | 当前状态 | 仓库证据 | 未完成项 |
|---|---|---|---|
| 规则/seed → world ticks → 状态/事件/快照 | 主工作区已实现 | `wanxiang_sdk::WorldModel`、`wanxiang_engine::LivingCore::tick`、`wanxiang_world::WorldSimulator` | 增加一条面向用户场景的验收测试，固定关键不变量 |
| 相同初始条件 → 规则变更 → 重放 → 轨迹比较 | SDK 层已实现 | `WorldModel::branch`、`rule_compare`、`web_demo_data` | 覆盖所有公开操作类型和自定义初始实体的回归测试；结果只说明模型内敏感性，不证明现实因果 |
| 事件 → 学习结果 → 有界反馈 → 生态状态 → 后续 tick | 已新增确定性反馈控制，等待 CI 验证 | `wanxiang_engine::feedback::apply_event_learning_feedback` 依据事件重要性和学习到的 patterns/inferences 更新 `LifeLayer.ecosystem_health`；`TickResult.feedback` 暴露记录；单元与固定 seed 集成测试覆盖边界/确定性 | 当前是显式启发式反馈，不是经实证校准的生态模型；仍需敏感性实验、稳定性分析，并验证长期轨迹没有违反守恒不变量 |
| Agent 决策 → 工具执行 → 观察 → 下一轮决策 | 新增真实执行路径 | `AgentRunner::run_with_tools` 调用 `ToolRegistry::execute` 并把结果加入下一轮历史；测试覆盖成功与审批 pending | 工具注册表现已提供显式审批决策执行 API，且审批不能覆盖硬阻止策略；Agent 主循环仍会在 pending 处停止，暂停/恢复式审批编排尚未实现 |
| LLM adapter → 结构化动作解析 → Agent 执行 | 现有接口已接入可注入规划器 | `MemoryBackedLlmPlanner` 使用 `BaseLLMAdapter`，要求 JSON 动作，解析/适配器错误 fail closed | provider 特定认证、超时、重试、流式、限流与 fallback 合约仍未验证 |
| Agent + 记忆 + 因果检索 + 工具执行 | 新增端到端测试路径 | `MemoryBackedLlmPlanner` 每轮检索 `NmmUnified`；记忆可选使用 `CausalGraph`；测试断言工具执行结果进入下一次 LLM prompt | 当前检索是实验性 SDR/启发式排序，不保证语义检索质量；因果图只影响检索评分，不构成真实因果证明 |
| Agent/记忆/因果 → 根工作区世界模拟 | 已有最小工具桥接，等待 CI 验证 | `wanxiang_agent_bridge::WorldAgent` 注册 `world_info`、`world_snapshot` 和有界 `world_step` 工具；脚本化 adapter 的端到端测试验证推进结果进入下一轮 prompt | 目前是受限工具桥，不是世界模型与 Agent 的双向语义规划；仍需暂停/恢复式审批、真实 provider 合约测试和更丰富的状态/事件观测契约 |
| Rust SDK → JSON → schema 验证 → 静态网页 | 有生成与展示流程 | `web_demo_data`、`.github/workflows/static.yml`、`site/index.html` | 根工作区 PR CI 现在生成并校验真实 SDK JSON、基准/变更轨迹元数据和浏览器脚本语法；仍需浏览器端有效/无效 fixture 渲染测试 |
| 整个仓库 | 不是单一闭环 | 根工作区与研究型引擎工作区分别构建和测试 | 应明确集成边界；独立 crate 测试通过不代表全仓库功能端到端成立 |

## 当前可以准确宣称的能力

- 主工作区提供带 seed 的世界模拟 SDK，可推进世界、导出快照并对比规则分支。
- Agent runtime 提供真实的注册工具执行路径，并能把执行结果反馈到下一轮规划器。
- 可注入的 LLM 规划器会在每轮决策前检索记忆，并将实际工具观察写回记忆；可选因果图参与启发式记忆评分。提示把检索记忆和工具输出视为不可信数据。端到端测试使用确定性测试 adapter，不访问外部网络。
- LLM 规划器对无效 JSON 或 adapter 错误 fail closed，不把错误当作任务完成。
- `wanxiang_agent_bridge` 把独立 `engines/` 工作区中的 Agent runtime 与根工作区的 `WorldModel` 通过受限工具连接；这是一条最小集成路径，不代表所有研究 crate 都已统一编排。
- 独立 crate 编译或单元测试通过，并不能证明模型输出质量、provider 可靠性、语义记忆效果或生产级安全。

## 下一步闭环顺序

1. 让 Agent 工具执行与记忆增强规划器的端到端测试在 CI 通过。
2. 补齐工具失败、策略阻止、审批待处理、步数上限和 session trace 隔离的测试。
3. 为 SDK 写一条固定 seed、明确规则、固定步数、可断言结果的端到端验收测试。
4. 在最小桥接通过 CI 后，定义稳定的世界观测与动作契约；避免让研究 crate 直接耦合模拟器内部实现。
5. 只有当学习/进化参数能以可测量且可复现的方式影响后续动力学后，才宣称“事件驱动的自适应世界闭环”完成。
6. 对真实 LLM provider 分别做认证、超时、限流、重试、流式和 fallback 合约测试。

## 每条闭环的验收标准

每条能力都必须记录：(a) 入口；(b) 相连组件；(c) 反馈到下一轮的状态/输出；(d) 停止和错误行为；(e) 一项连接断开时会失败的自动化测试。
