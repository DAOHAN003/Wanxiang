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

运行一个规则对照示例：

```bash
cargo run --locked -p wanxiang_sdk --example rule_compare
```

示例用相同 seed 建立基准世界，将同一世界规则中的一个参数调整后重放至相同 tick，再输出两者的概要和结构化差异。它展示的是**模拟模型内的规则敏感性实验**，不是对现实世界的因果效应证明。

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

两个工作区目前不是一套经过整体验证的统一认知架构。不要假设 `engines/` 中的每个能力都已接入根目录的世界模拟主循环；请以对应 crate 的实际接口和测试为准。

## 当前边界

- 相同 seed、规则和步数旨在让当前实现中的模拟轨迹可复现；这不保证跨版本或跨平台的长期兼容性，除非相应契约另有测试与承诺。
- 当前 SDK 的规则分支/比较通过 seed、规则和 tick 重建并重放轨迹。修改规则后会从起点重放到当前 tick，不是从完整运行时快照恢复后只改变未来部分。当前变体句柄也没有保存 `GenrePreset`、调用方注入的初始实体或实例级认知层开关，因此对这些自定义初始化创建的世界，`fork` / `branch` 不保证保留完整设定；这需要作为后续 API 正确性改进项。
- `world_state` 的资源池是抽象账本，不是物理学中的能量守恒或热力学熵模型。
- `causal_core` 的部分传播与反事实计算采用简化模型；图抽象使用有规模限制的排列搜索，不是完整 VF2 实现。
- `memory_core` 的确定性稀疏编码不自动等于语义理解；检索质量仍需要带标签的数据集和基线对比。
- `security_suite` 提供辅助检查与策略代码。词法路径检查不能替代 OS 权限控制、符号链接处理或进程隔离。
- 叙事一致性与角色相关分数是启发式信号，不等于故事质量或可靠语义理解。经典算法本身也不应被描述为万象原创。

进一步阅读：[架构说明](docs/ARCHITECTURE.md)、[算法状态与限制](docs/ALGORITHM_STATUS.md)、[安全模型](docs/SECURITY_MODEL.md)。

## 参与方式

小而可验证的改动优先。请为行为修复添加回归测试；对算法改进提供适用假设、评测数据和可复现的基线。参见 [贡献指南](CONTRIBUTING.md)。

## 许可证

Apache License 2.0，详见 [LICENSE](LICENSE)。

## English

Wanxiang is an experimental Rust project focused on **seeded, reproducible world simulation** for long-running state evolution, rule-comparison experiments, and interactive narrative. A separate engine workspace contains research components for causal graphs, memory, agent runtimes, model adapters, and narrative tooling.

Run the rule-comparison example with `cargo run --locked -p wanxiang_sdk --example rule_compare`, or test both workspaces with the commands above. The example compares trajectories produced by the current simulation model; it is not evidence of real-world causal validity.

This is an early 0.1.x snapshot with uneven maturity across modules. Reproducibility does not establish predictive accuracy, simplified causal utilities are not complete causal inference, and security helpers are not an OS sandbox. See the architecture, algorithm-status, and security-model documents for scope and limitations.
