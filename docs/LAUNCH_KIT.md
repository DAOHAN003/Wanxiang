# Wanxiang Launch Kit

This document is a practical launch and demo plan for the public repository. Keep all claims aligned with the implementation and the hosted demo. This is an early-stage project, not a validated real-world prediction or causal-inference system.

## One-line positioning

**中文：** 同一个世界，只改一条规则，看看它会变成什么样。

**English:** Run the same world twice. Change one rule. See exactly what changed.

Wanxiang is an experimental Rust framework for seeded, reproducible world simulation and controlled rule-comparison experiments. Its useful loop is: fixed seed → baseline and intervention → inspect state/event differences → export a report.

## Honest demo description

The hosted GitHub Pages demo displays simulation results generated during the build process. It is a static viewer, not a live Rust simulation backend. It does not require login or an LLM API key. Live world ticking, state mutations, and Agent execution require a separately running Rust API. Do not imply the hosted static page provides live multi-user simulation.

Reproducibility within the implemented simulator does not establish real-world predictive accuracy or causal validity. The separate `engines/` workspace contains research components with uneven maturity and should not be described as a single fully integrated production Agent platform.

## 30-second demo storyboard

Record actual product footage; do not replace it with mock UI.

| Time | Show | Narration / caption |
|---|---|---|
| 0–5 s | Open the real static demo and show the world structure or event timeline | “A world simulation with reproducible, seeded runs.” |
| 5–12 s | Identify the seed and baseline run | “Start with the same initial conditions.” |
| 12–19 s | Show the actual rule setting and intervention result | “Change one rule and compare the resulting trajectory.” |
| 19–25 s | Show actual state/event differences and report JSON | “Inspect the differences and export the result.” |
| 25–30 s | Show repo, demo, and first-experiment guide links | “Wanxiang: reproducible world simulation in Rust.” |

Before recording, verify the deployed page loads, the displayed result is real build-generated output, report export works if shown, and all links work. If a step cannot be demonstrated reliably, remove it rather than simulating success.

## Chinese community post

### 我在用 Rust 做一个可复现的世界模拟实验框架：Wanxiang

如果保持初始世界和随机种子不变，只修改一条规则，世界的演化结果会发生什么变化？

Wanxiang 目前围绕这个实验流程构建：

1. 使用固定 seed 初始化模拟世界。
2. 运行基线与修改规则后的实验分支。
3. 对比世界状态、事件记录和结果差异。
4. 导出实验结果，方便复查和复现。

项目使用 Rust 构建模拟内核，并提供浏览器静态演示。在线页面展示的是构建阶段生成的模拟结果；实时推进世界或运行 Agent 需要单独启动 Rust API。

- Repository: https://github.com/DAOHAN003/Wanxiang
- Demo: https://daohan003.github.io/Wanxiang/
- First experiment: https://github.com/DAOHAN003/Wanxiang/blob/main/docs/FIRST_EXPERIMENT.md

我想优先听到具体反馈：第一次打开能否看懂实验流程？规则差异是否清晰、可检查、可复现？这个工作流对模拟系统、Agent 环境或交互式世界开发有没有实际用途？

这是一个仍在开发中的实验框架，不是现实世界预测系统。欢迎指出文档、演示和实现之间的落差。

## English Show HN draft

### Show HN: Wanxiang — Reproducible World Simulation in Rust

I’m building Wanxiang, an experimental Rust framework for reproducible world simulation and controlled rule-comparison experiments.

The core idea is simple: run the same seeded world twice, change one rule, and inspect how the resulting trajectories differ.

The project focuses on reproducibility, inspectable state and event histories, and exportable experiment reports. The browser demo shows build-generated simulation results; running live simulations or Agents requires a separately hosted Rust API.

- Demo: https://daohan003.github.io/Wanxiang/
- Repository: https://github.com/DAOHAN003/Wanxiang
- First experiment: https://github.com/DAOHAN003/Wanxiang/blob/main/docs/FIRST_EXPERIMENT.md

I’m especially interested in feedback on whether this rule-comparison workflow is useful for simulation tooling, Agent environments, or interactive-world development. This is an early-stage project; results describe the implemented simulator and do not validate real-world causal effects.

Post only after personally checking that the demo and first-experiment guide work end-to-end. Explain what you built and what feedback would be useful. Do not ask for upvotes or stars, coordinate engagement, or cross-post identical promotional text indiscriminately.

## Release checklist

### Product and documentation
- [ ] A first-time visitor can understand the project's purpose from the first screen.
- [ ] The README links to the demo, first experiment, architecture/positioning, algorithm status, and roadmap.
- [ ] Chinese and English navigation links resolve to existing files.
- [ ] The demo is labelled as a static, build-generated snapshot viewer.
- [ ] The first experiment can be followed from a clean checkout using the documented commands.
- [ ] The demo's JSON/report export is tested before being shown in a recording.
- [ ] Known limitations are visible and do not rely on users discovering them later.

### Technical quality
- [ ] Required CI checks pass on the exact PR head commit.
- [ ] No lockfiles or dependencies were changed without a reason.
- [ ] Root and `engines/` workspaces are tested independently.
- [ ] No API keys, bearer tokens, local state, or private-version files are committed.
- [ ] Live API claims match the deployed configuration and authentication model.

### Launch assets
- [ ] Record a real 20–30 second screen capture.
- [ ] Prepare one screenshot showing baseline vs. intervention.
- [ ] Prepare one real exported report sample, with seed and simulator version/build noted.
- [ ] Check all public links from a logged-out browser.
- [ ] Publish a technical write-up explaining one reproducible experiment, including inputs, rule change, observed differences, and reproduction steps.
- [ ] Prepare to answer technical feedback; do not schedule a launch if nobody can monitor replies.

## Four-week rollout

### Week 1 — Trust and first-run clarity
Verify the static demo and quickstart, finish documentation CI, record actual footage, and fix the first confusing step encountered by a new developer.

### Week 2 — Small technical audience
Share with relevant Rust, simulation, and Agent-environment developers. Ask for feedback on a specific workflow, not general praise. Record what users tried, where they stopped, and which result they found useful.

### Week 3 — Publish a reproducible case study
Publish one fixed-seed experiment with the initial conditions, exact rule change, baseline and intervention outputs, report artifact, and reproduction commands. State clearly that conclusions apply to the simulator, not necessarily to the real world.

### Week 4 — Iterate, then consider a broader launch
Address the strongest usability or correctness issue first. Consider Show HN or Product Hunt only if the demo is easy to try, the project page is accurate, launch assets are ready, and someone can respond to comments.

## Metrics

Prefer useful evidence over a single star count:

- Demo page visits and outbound clicks, using privacy-conscious analytics if added.
- First-experiment guide visits and successful completion feedback.
- Report exports or downloads, if observable without intrusive tracking.
- Substantive issues, discussions, forks, external references, and repeat contributors.
- Reproducible bug reports and time-to-first-success.

Do not add account creation or tracking barriers solely to measure interest. Do not invent metrics where instrumentation does not exist.

## Operating rules for automation

- Check CI and open PR status on a regular cadence.
- Retry only clearly transient failures; inspect failed logs before changing code.
- Never merge a PR solely because most jobs passed; require all required checks to complete successfully.
- Never publish to external communities, solicit votes, or trigger potentially billable LLM benchmarks without explicit approval.
- Keep public-repository work separate from any local/private advanced version.
- Report verified state, blockers, and the next concrete action; do not claim background monitoring unless an automation is actually enabled.
