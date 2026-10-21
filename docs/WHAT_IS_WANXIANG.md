# 万象是什么？ | What Is Wanxiang?

[English version below](#what-is-wanxiang)

## 简短定义

**万象是一个早期的、基于 Rust 的可复现世界模拟框架与实验平台。** 它的核心用途是创建结构化模拟世界，在明确的规则与随机种子下推进世界状态，检查事件和快照，并对受控规则变化产生的轨迹进行比较。

从软件工程意义上说，万象可以称为框架：它提供可复用的领域类型、模拟引擎、面向嵌入的 SDK，以及供应用程序使用的集成入口。现阶段更准确的定位是“可复现世界模拟框架”，而不是成熟的通用 AI Agent 框架。

## 框架能力

- **可复用的执行模型：** 应用可以创建 `WorldModel`、推进 tick、检查状态、导出快照，并通过 SDK 比较不同规则分支。
- **明确的实验输入：** seed、初始配置、规则和 tick 数使模型内部实验可检查、可复现；可复现范围仍受具体模型和版本兼容性约束。
- **结构化输出：** 世界快照、事件记录和对比报告可以供下游应用检查或导出。
- **集成入口：** `wanxiang_sdk` 是预期的嵌入 API；`wanxiang_agent_bridge` 将有限的一组 Agent 工具连接到世界模型。
- **研究扩展组件：** 仓库还包含记忆、因果图工具、Agent 运行时、模型适配器和叙事工具等研究模块。

## 当前不应声称什么

- 不能把所有模块描述为成熟、统一的 AI Agent 框架。根目录世界模型工作区与 `engines/` 下的独立工作区有不同的集成程度和成熟度。
- 不能仅凭存在记忆、因果和 Agent crate，就声称已验证一套通用认知架构。
- 不能把可复现性或规则分支比较等同于真实世界预测有效性或因果推断结论。
- 不能把仓库等同于已运营的生产级、多租户托管服务。构建成功不代表公共实时服务已部署。
- 不能声称它可直接替代现有引擎或 Agent 框架；这需要兼容性、工作负载和对比评测证据。

## 当前产品边界

最清晰的首要用途是**构建和评估可复现的模拟世界**。应用可以定义世界，在固定 seed 和规则下运行，对比基准规则与变更规则，再检查或导出结构化结果。Agent 执行是可选的世界交互方式，而不是整个产品的唯一定位。

静态网页 Demo 可展示构建时生成的样例轨迹，不需要 API Key 或本地 Rust 环境。实时 tick、状态写入和 Agent 执行需要单独运行或部署 Rust API。

## 推荐的一句话介绍

> **万象是一个用于可复现世界模拟与受控规则实验的早期 Rust 框架，并附带可选的 Agent、记忆、因果图和叙事研究组件。**

随着项目成熟，更强的能力声明应建立在稳定公共接口、端到端集成测试、可复现基准数据集、明确的兼容性保证和外部用户证据之上。

---

# What Is Wanxiang?

## Short definition

**Wanxiang is an early-stage Rust framework and experimentation platform for reproducible world simulation.** It creates structured simulated worlds, advances state under explicit rules and seeds, exposes events and snapshots, and compares trajectories produced by controlled rule changes.

In the broad software-engineering sense, Wanxiang qualifies as a framework: it offers reusable domain types, a simulation engine, an embedding-facing SDK, and integration points for applications. Its most precise current positioning is “reproducible world-simulation framework,” not a mature general-purpose AI-agent framework.

## Framework capabilities

- **Reusable execution model:** applications can create a `WorldModel`, advance ticks, inspect state, export snapshots, and compare rule branches through the SDK.
- **Explicit experiment inputs:** seeds, initial configuration, rules, and tick counts make model-internal experiments inspectable and reproducible, within model and version-compatibility limits.
- **Structured outputs:** snapshots, event records, and comparison reports can be inspected or exported by downstream applications.
- **Integration surface:** `wanxiang_sdk` is the intended embedding API; `wanxiang_agent_bridge` connects a bounded set of Agent tools to the world model.
- **Research extensions:** the repository also contains research components for memory, causal-graph utilities, Agent runtime, model adapters, and narrative tooling.

## What it should not claim yet

- It is not a single, uniformly mature AI-agent framework. The root world-model workspace and the separate `engines/` workspace have different maturity and integration levels.
- The presence of memory, causal, and Agent crates does not by itself validate a general cognitive architecture.
- Reproducibility and rule-branch comparison do not establish real-world predictive validity or causal inference.
- The repository is not equivalent to an operated production multi-tenant service. A successful build does not mean a public live backend is deployed.
- It is not a drop-in replacement for existing engines or Agent frameworks without compatibility, workload, and comparative evaluation evidence.

## Current product boundary

The clearest first use case is **building and evaluating reproducible simulated worlds**. Applications can define a world, run it with a fixed seed and rule set, compare a baseline with a changed rule, and inspect or export structured results. Agent execution is an optional way to interact with a world, not the definition of the whole product.

The static web demo presents build-generated sample trajectories without an API key or local Rust installation. Live ticking, state-changing operations, and Agent execution require the Rust API to be run or deployed separately.

## Recommended one-sentence description

> **Wanxiang is an early-stage Rust framework for reproducible world simulation and controlled rule experiments, with optional Agent, memory, causal-graph, and narrative research components.**

As the project matures, stronger claims should be supported by stable public interfaces, end-to-end integration tests, reproducible benchmark datasets, documented compatibility guarantees, and evidence from external users.
