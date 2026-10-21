# 第一次实验 / First Experiment

这份指南面向第一次接触万象的开发者。建议先完成在线静态实验；它不需要登录、Rust 环境、LLM 或模型 API Key。

This guide is for first-time Wanxiang users. Start with the hosted static experiment: no login, local Rust setup, LLM, or model API key is required.

## 1. 在线体验（约 3–5 分钟） / Try the hosted lab (about 3–5 minutes)

打开 [万象世界模拟实验室](https://daohan003.github.io/Wanxiang/)。

1. 在“基准世界 / 规则变体”之间切换，查看两条轨迹的指标、世界结构和事件。
2. 留意两条轨迹使用相同的 seed 和推进步数；实验变体改变的是模型内的一项规则。
3. 查看差异和事件时间线，确认规则变化后哪些模拟状态发生了变化。
4. 点击“导出对照实验报告 JSON”，保存结果供离线检查或分享。

Open the [Wanxiang World Simulation Lab](https://daohan003.github.io/Wanxiang/).

1. Switch between the baseline world and the rule variant; inspect metrics, world structure, and events.
2. The paired trajectories use the same seed and tick horizon while changing a model rule.
3. Inspect the diff and event timeline to see which simulated outcomes changed.
4. Export the comparison experiment as JSON for offline inspection or sharing.

静态报告使用版本化格式 `wanxiang-experiment-report-v1`，包含实验元数据、基准与变体快照、结构化差异和解释边界。格式契约见 [JSON Schema](../schemas/wanxiang-experiment-report-v1.schema.json)。此报告是实验比较档案，不等同于包含完整操作历史的 checkpoint/replay 包。

The static report uses the versioned `wanxiang-experiment-report-v1` format. It includes experiment metadata, baseline and altered snapshots, a structured diff, and interpretation limits. See the [JSON Schema](../schemas/wanxiang-experiment-report-v1.schema.json). It is a comparison report, not a checkpoint/replay bundle containing the full operation history.

## 2. 在本地运行一次实验（可选） / Run an experiment locally (optional)

需要稳定版 Rust 工具链和 Cargo。从仓库根目录运行：

With a stable Rust toolchain and Cargo, run this from the repository root:

```bash
cargo run --locked -p wanxiang_experiment --example run_experiment -- 42 100 0.95
```

该示例用 seed `42` 推进 `100` 个 tick，并将 `energy_density` 设为 `0.95`。它会输出包含实验规格、两份快照、模型差异、不变量检查和 `passed` 状态的 JSON 报告。此命令不需要 LLM 或模型 API Key；首次构建仍需要 Cargo 按依赖配置获取 Rust crates。

This runs seed `42` for `100` ticks and sets `energy_density` to `0.95`. The JSON report contains the experiment specification, both snapshots, the model diff, invariant checks, and a `passed` status. No LLM or model API key is required; Cargo still needs to obtain Rust dependencies on the first build.

## 3. 如何解读结果 / Interpret the result

- 相同实验规格、模拟器版本和构建配置旨在产生相同的结构化结果。
- 规则干预说明的是**当前模拟模型内**的轨迹敏感性；它不证明现实世界的因果效应，也不代表模型可以准确预测现实。
- 资源账本是模拟中的抽象约束，不是物理能量守恒模型。
- SDK 目前处于 `0.1.x` 预览阶段；请查看 [SDK 兼容策略](SDK_COMPATIBILITY.md)，不要假定所有旧存档都能跨版本永久兼容。

- The same experiment specification, simulator version, and build configuration are designed to produce the same structured result.
- A rule intervention measures trajectory sensitivity **inside this simulation model**. It does not establish real-world causal effects or real-world predictive accuracy.
- The resource ledger is an abstract simulation invariant, not a physical energy-conservation model.
- The SDK is currently a `0.1.x` preview. Read the [SDK compatibility policy](SDK_COMPATIBILITY.md); do not assume every old save is permanently compatible across versions.

## 4. 什么时候需要实时 API？ / When do you need the live API?

静态实验室足以查看样例、比较规则和导出报告。若要真实推进实时世界、执行写操作或运行 Agent，需要自行启动或部署 Rust API：

The static lab is enough to inspect samples, compare rules, and export reports. To advance a live world, execute write operations, or run an Agent, start or deploy the Rust API yourself:

```bash
cargo run --locked -p wanxiang_agent_bridge --example live_server
```

默认服务仅绑定本机回环地址。仓库没有默认提供公共实时 API。公开部署前请先阅读 [部署与运维手册](OPERATIONS_RUNBOOK.md)，配置认证、HTTPS、精确 CORS 来源和持久化保护。

By default, the server binds to loopback. The repository does not provide a public live API by default. Before public deployment, follow the [operations runbook](OPERATIONS_RUNBOOK.md) and configure authentication, HTTPS, a precise CORS origin, and persistent-data protection.

## 进一步阅读 / Next steps

- [产品闭环与验收标准 / Product acceptance](PRODUCT_ACCEPTANCE.md)
- [万象定位 / Project positioning](WANXIANG_POSITIONING.md)
- [架构说明 / Architecture](ARCHITECTURE.md)
- [工程路线图 / Roadmap](ROADMAP.md)
