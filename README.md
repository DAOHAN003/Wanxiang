# 万象 · Wanxiang

**万象是一套以 Rust 编写的、由种子驱动的可复现世界模拟引擎，面向长程世界状态演化、规则对照实验与互动叙事。** 仓库还包含独立的因果图、记忆、Agent 运行时和模型适配研究组件。

> **项目阶段：早期实验性公开版本（0.1.x）。** 当前模块成熟度不一，接口可能调整。可复现的模拟结果不等于现实预测正确；因果工具不等于完整因果推断；安全辅助代码也不是生产级隔离边界。

## 万象主要在做什么

万象的主工作区围绕一个具体问题构建：给定世界规则、初始设定和随机种子，能否推进一个结构化世界，并对输出快照、事件、资源账本及不同规则下的轨迹进行检查和比较？

- **可复现的世界模拟**：用显式 seed 创建世界，以 tick 推进模拟，并导出世界快照、事件和报告。
- **数据驱动的世界设定**：通过 `GenrePreset` 描述地点、势力、角色和参数；新增设定主要改配置，而不是把具体小说题材写死在引擎里。
- **规则对照与结构化比较**：在相同 seed 和相同推进步数下，比较基准规则与变更规则产生的模拟结果。
- **认知与 Agent 研究组件**：独立的引擎工作区包含因果图、记忆、行动选择、Agent 工具工作流、LLM 适配及叙事辅助等模块。

万象当前最值得验证的主张不是“拥有最多的模块”，而是**可复现的世界状态演化与可对照实验，能否为互动叙事、模拟和 Agent 应用带来可测量的帮助**。

## 快速体验

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

进一步阅读：[架构说明](docs/ARCHITECTURE.md)、[算法状态与限制](docs/ALGORITHM_STATUS.md)、[安全模型](docs/SECURITY_MODEL.md)、[能力闭环审计](docs/CAPABILITY_CLOSURE_AUDIT.md)。

## 参与方式

小而可验证的改动优先。请为行为修复添加回归测试；对算法改进提供适用假设、评测数据和可复现的基线。参见 [贡献指南](CONTRIBUTING.md)。

## 许可证

Apache License 2.0，详见 [LICENSE](LICENSE)。

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

页面支持从本机导入该 JSON；数据只在浏览器中读取。静态网站不提供持续运行的后台模拟、多人实时共享或云端存档。

## 事件学习到世界动力学的反馈闭环

`LivingCore::tick` 会从本 tick 的真实事件文本生成学习结果，再通过 `AdaptiveFeedback` 将事件重要性映射为有界压力项、将学习模式/推理映射为较小的韧性项，并更新 `LifeLayer.ecosystem_health`。反馈在快照与回放记录前应用，结果包含在 `TickResult.feedback` 中。控制器有上下界和固定 seed 集成测试；这属于可解释的模拟反馈策略，不证明文本学习是真实因果知识。

## Agent 与真实世界模拟的集成闭环

`wanxiang_agent_bridge::WorldAgent` 将 `WorldModel` 暴露为三个受限工具：`world_info`、`world_snapshot` 和 `world_step`。`world_step` 每次最多推进 100 tick，参数错误不会推进模拟。Agent 通过 `run_with_tools` 执行工具，真实返回值会进入下一轮规划 prompt。仓库测试使用固定 seed 和脚本化 adapter 验证“Agent 决策 → 世界推进 → 观察反馈 → 再决策”的链路，不需要外部 API。

## Agent 工具执行与记忆增强规划器

`agent_runtime::AgentRunner::run_with_tools` 可以接收实现 `ActionPlanner` 的决策器，执行注册工具，并把工具返回值放入下一轮决策历史。`MemoryBackedLlmPlanner` 将 `BaseLLMAdapter`、`memory_core` 检索和可选因果图检索连到这一流程，并把真实工具观察结果写回记忆供后续决策检索。检索到的记忆和工具输出在提示中被标为不可信数据，不能覆盖系统指令。模型必须返回指定 JSON 动作；模型调用失败或响应格式错误会 fail closed，不会退化成成功的 `FINISH`。

该路径仍有边界：Agent 可通过 `run_with_approval` 注入同步审批回调；未配置回调或审批拒绝时会在 `pending` 处停止。`ResumableSession` 提供可序列化的审批状态，但持久化、TTL、幂等和分布式并发控制属于应用层责任。记忆检索使用实验性稀疏表示，不保证语义相关性；Agent 观察写回默认保留最近 2,048 条记忆，可通过 `with_memory_limit` 调整；因果分数是启发式工具，不证明真实因果。`OpenAiCompatibleAdapter` 将 OpenAI-compatible 请求通过 `JsonTransport` 发送并归一化响应；核心 crate 不捆绑 HTTP/TLS 客户端，部署方必须提供具备 TLS、超时和网络错误处理的 transport。通用适配器的 SSE 流式调用明确返回 `NotImplemented`，不能视为已验证的真实 provider 流式接入。当前 bridge 是最小工具层集成，不代表所有 `engines/` crate 已统一编排。

## English

Wanxiang is an experimental Rust project focused on **seeded, reproducible world simulation** for long-running state evolution, rule-comparison experiments, and interactive narrative. A separate engine workspace contains research components for causal graphs, memory, agent runtimes, model adapters, and narrative tooling.

Run the rule-comparison example with `cargo run --locked -p wanxiang_sdk --example rule_compare`, or test both workspaces with the commands above. The example compares trajectories produced by the current simulation model; it is not evidence of real-world causal validity.

This is an early 0.1.x snapshot with uneven maturity across modules. Reproducibility does not establish predictive accuracy, simplified causal utilities are not complete causal inference, and security helpers are not an OS sandbox. See the architecture, algorithm-status, and security-model documents for scope and limitations.


## Regression acceptance checks

The SDK test suite includes a 1,000-tick fixed-seed replay test that compares equivalent operation histories byte-for-byte, checks the resource-ledger invariant, and verifies that a changed rule creates a distinct replayed trajectory. The CI demo validation also checks that both exported worlds report a conserved ledger and that the rule-diff payload identifies the changed rule. These tests establish properties of the implemented simulation model; they do not validate real-world predictive or causal accuracy.
