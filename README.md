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
| `engines/Cargo.toml` | 独立的研究型 crate 工作区：`world_state`、`causal_core`、`memory_core`、`agent_runtime`、`llm_adapter`、`novel_writer_suite`、`security_suite` 等 |

两个工作区仍是独立 Cargo 工作区。`engines/agent_runtime` 现在提供可注入 LLM adapter 的工具执行路径，并可在每轮决策前检索 `memory_core` 记忆、使用关联因果图进行启发式排序；这条路径不等于它已经接入根目录的 `wanxiang_sdk` 世界模拟循环。其他研究 crate 也不能仅凭存在就视为已集成，具体边界见 [能力闭环审计](docs/CAPABILITY_CLOSURE_AUDIT.md)。

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
2. 打开 [仓库 Pages 设置](https://github.com/DAOHAN003/Wanxiang/settings/pages)，在 **Build and deployment → Source** 选择 **GitHub Actions**。
3. 等待主分支的 **Deploy Wanxiang static demo** 工作流成功。成功后访问 [https://daohan003.github.io/Wanxiang/](https://daohan003.github.io/Wanxiang/)。

本地生成自己的真实规则对照快照：

```bash
cargo run --locked -p wanxiang_sdk --example web_demo_data -- 42 100 > demo.json
```

页面支持从本机导入该 JSON；数据只在浏览器中读取。静态网站不提供持续运行的后台模拟、多人实时共享或云端存档。

## Agent 工具执行与记忆增强规划器

`agent_runtime::AgentRunner::run_with_tools` 可以接收实现 `ActionPlanner` 的决策器，执行注册工具，并把工具返回值放入下一轮决策历史。`MemoryBackedLlmPlanner` 将 `BaseLLMAdapter`、`memory_core` 检索和可选因果图检索连到这一流程。模型必须返回指定 JSON 动作；模型调用失败或响应格式错误会 fail closed，不会退化成成功的 `FINISH`。

该路径仍有边界：人工审批目前以 `pending` 结果停止，尚未提供交互式审批处理器；记忆检索使用实验性稀疏表示，不保证语义相关性；因果分数是启发式工具，不证明真实因果；真实 provider 的认证、超时、重试与流式契约仍需单独集成测试。它尚未与根工作区的世界模拟器统一编排。

## English

Wanxiang is an experimental Rust project focused on **seeded, reproducible world simulation** for long-running state evolution, rule-comparison experiments, and interactive narrative. A separate engine workspace contains research components for causal graphs, memory, agent runtimes, model adapters, and narrative tooling.

Run the rule-comparison example with `cargo run --locked -p wanxiang_sdk --example rule_compare`, or test both workspaces with the commands above. The example compares trajectories produced by the current simulation model; it is not evidence of real-world causal validity.

This is an early 0.1.x snapshot with uneven maturity across modules. Reproducibility does not establish predictive accuracy, simplified causal utilities are not complete causal inference, and security helpers are not an OS sandbox. See the architecture, algorithm-status, and security-model documents for scope and limitations.
