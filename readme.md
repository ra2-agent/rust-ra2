# rust-ra2 · ra2-agent 研究分支

这是 [ra2-agent](https://github.com/ra2-agent) 社区使用的红色警戒 Rust 引擎分支，重点是 **NPC / Agent 接入、可复现评测和仿真性能优化**。

本项目 Fork 自 [rust-alert/ra2.exe](https://github.com/rust-alert/ra2.exe)，本轮改造基于上游提交 [c61fbf21](https://github.com/rust-alert/ra2.exe/commit/c61fbf2105ae6d741b2e0e324de9fd06de3013e9)。感谢上游作者与贡献者建立跨平台引擎、资源适配、渲染和工具链。本分支保留上游 Git 历史，沿用 [Apache-2.0 许可证](License.md) 和原有版权声明。

本仓库的开发分支及默认入口为 **ra2-agent**；上游 master 分支保留来源关系。引擎源码集中在这里，社区 Agent 和正式 Elo 战绩分别维护在独立仓库。

## 我们关注什么

| 方向 | 当前工作 |
|---|---|
| Agent 接入 | 可替换的 Rust Npc 接口、只读观测、命令提交、逐局初始化、策略注册与版本化基线 |
| 可复现评测 | 无窗口镜像双局、固定种子、场景分榜、经济指标、命令轨迹与终局哈希回放核验 |
| 策略研究 | 冻结 Hard 基线、独立候选策略、多代参照策略、运营/生产/目标选择/集火与撤退实验 |
| 性能优化 | 减少 ECS 投影和哈希开销、临时分配与字符串复制；有界并发、精简报告、组件耗时分析 |

评测沿用 ra-engine 的命令、经济、生产、移动、战斗和胜负判定路径。性能改动需要检查同赛程的命令、胜负和状态哈希；策略改动则使用新版本与固定对手比较。

## 快速开始

需要 Git、Rust nightly（以 rust-toolchain.toml 为准）；一键评测脚本另需 Node.js 20+。无窗口合成场景不需要零售游戏资源、GPU 或图形窗口。

```sh
git clone --branch ra2-agent https://github.com/ra2-agent/rust-ra2.git
cd rust-ra2

# 接口、引擎与回放回归检查，不产生社区正式排名
cargo test --locked -p ra-ecs -p ra-types -p ra-adaptor -p ra-engine -p ra-testing -p ra-arena
node --test scripts/npc/report.test.mjs

# 查看已有策略
cargo run --locked -p ra-arena -- list
```

首次运行会按工作区工具链配置安装 Rust 组件。仓库保留 Cargo.lock 以固定依赖解析；正式评测请同时记录 rustc 版本和完整 Git 提交。

维护者需要运行实验时，可选择一键流水线或直接调用 Arena：

```sh
# 回归、构建、联赛、诊断回放、报告和历史对比
node scripts/npc-test.mjs
# Windows 也可使用 .\test-npc.cmd

# 指定一组策略与合成场景，保留逐局记录
cargo run --locked --profile arena -p ra-arena -- run --bots candidate,hard-baseline-v1 --scenarios duel-v1,economy-v1 --rounds 4 --seed 42 --jobs 1 --out arena-results/example.json

# 诊断时保存命令轨迹并核对回放
cargo run --locked --profile arena -p ra-arena -- run --rounds 1 --trace --out arena-results/replay.json
cargo run --locked --profile arena -p ra-arena -- replay arena-results/replay.json
```

输出写入被 Git 忽略的 arena-results/。本仓库中的实验 Elo 是当次参赛池的结果；社区正式分数由维护者测试、确认并提交 [leaderboard](https://github.com/ra2-agent/leaderboard)，不自动同步实验报告。NPC evaluation 工作流只支持维护者手动触发；社区 Agent PR 不触发正式比赛。

## 接入自己的 Agent

当前支持 **Rust 进程内 Npc**。Agent 读取 BotContext 中的世界与玩家席位，通过 GameCommand 返回操作；评测器负责玩家信封、合法性校验和推进仿真。

```rust
use ra_arena::{BotContext, Npc};
use ra_engine::GameCommand;

pub struct MyNpc;

impl Npc for MyNpc {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        // 从现有 Hard 策略开始，再替换运营与作战决策。
        ra_engine::heuristic_ai_commands(context.world, context.player, "Hard")
    }
}
```

在评测驱动中注册工厂：registry.register("my-npc-v1", || Box::new(MyNpc))?，并把该注册名加入 ArenaConfig.npcs。每局创建新实例，涉及随机数时通过 reset(seed, player) 初始化。完整示例见 [custom_npc.rs](projects/tooling/ra-arena/examples/custom_npc.rs)，社区开发流程见 [agent-template](https://github.com/ra2-agent/agent-template)。

协议为 ra-arena-full-info-v1：双方都能读取完整世界，使用合成规则与对称场景，尚无战争迷雾过滤、独立进程隔离或逐 Agent 的可靠 CPU/内存预算。Python、HTTP、Wasm 和 LLM Agent 接口属于后续研究方向。当前分数只衡量本协议下的重写引擎表现。

源码位置：

- [NPC 接口与注册表](projects/tooling/ra-arena/src/bot.rs)
- [候选策略](projects/engine/ra-engine/src/gameplay/npc/candidate.rs) 与 [冻结 Hard 基线](projects/tooling/ra-arena/baselines/hard-v1/readme.md)
- [一键工作流](projects/tooling/ra-arena/npc-workflow.md) 与 [已有策略实验记录](projects/tooling/ra-arena/optimization-report.md)
- [Arena 使用说明、指标与公平性](projects/tooling/ra-arena/readme.md)

## 性能优化

我们的目标是在保持仿真和回放语义的前提下，减少单局 CPU 成本和内存占用，并提高批量研究的吞吐。

- ECS 变更只更新相关投影，移除全量同步中的重复实体查找；摘要路径直接使用只读组件视图。
- 规则键已规范化时直接借用，减少每 tick 的临时字符串、路径和候选列表复制。
- 镜像双局使用有界线程队列，按固定赛程顺序汇总结算；jobs 控制并发与资源需求。
- summary 模式及时丢弃逐局数据，只保留累计统计；完整记录与 trace 用于需要回放的诊断。
- profiling feature 按 NPC、引擎阶段、ECS 和场景记录耗时，普通构建不包含这些计时插桩。
- arena Cargo profile 保留优化和增量编译，减少策略迭代时全量 LTO 的等待。

已有本机对照记录中，相同串行赛程的进程 CPU 时间从 11.81 秒降至 8.34 秒（约29.4%）；固定样本的分配次数减少约84.2%。这是特定硬件、规则和参赛池的测量结果，分配次数、墙钟时间和 CPU 时间分别统计，不能互相替代。条件和复测方法见 [性能说明](projects/tooling/ra-arena/readme.md#减少总-cpu-工作量)；这些历史测量不是对所有 Agent 的性能保证。

```sh
# 带组件计时的诊断构建
cargo run --locked --release -p ra-arena --features profiling -- run --profile --jobs 1 --rounds 10 --summary --out arena-results/profile.json
# 分配计数基准；最终优化收益应再用无计时插桩的普通构建验证
cargo run --locked --release -p ra-arena --example cpu_bench -- 10
```

Windows 的 [benchmark.ps1](projects/tooling/ra-arena/scripts/benchmark.ps1) 可记录进程 CPU 时间、墙钟时间、峰值工作集与报告大小。组件 profiling 使用单调墙钟，含等待与调度影响，不能当作各组件的实际 CPU 时间。

## 社区仓库如何协作

| 仓库 | 职责 |
|---|---|
| [rust-ra2](https://github.com/ra2-agent/rust-ra2) | 本分支的引擎、接入接口、评测工具和性能改造 |
| [agent-spec](https://github.com/ra2-agent/agent-spec) | Agent 清单、源码版本与评测协议 |
| [agent-template](https://github.com/ra2-agent/agent-template) | Agent 开发与打包起点 |
| [agents](https://github.com/ra2-agent/agents) | 社区 PR 提交的不可覆盖源码版本 |
| [leaderboard](https://github.com/ra2-agent/leaderboard) | 维护者提交的正式战绩与可复算 Elo |
| [research](https://github.com/ra2-agent/research) | 研究路线、实验设计与结论 |

社区贡献者提交源码与接入说明；维护者审核、测试并录入正式战绩。正式赛季必须固定引擎完整提交、Agent 版本、规则/场景指纹和预算，保持干净工作区；更换条件需按社区协议另开赛季。

## 图形客户端与上游能力

上游包含 RA2 / YR / Mental Omega 数据适配、原生窗口、GPU 渲染、N-API、TypeScript CLI 和 Wasm 相关工作。本分支保留这些源码，研究入口优先使用无窗口 ra-arena；图形与跨平台能力的实际完整程度以各模块文档和对应验证为准。

```sh
pnpm install --frozen-lockfile
pnpm run build
pnpm exec ra2 emulate --path "C:/Games/RA2" --edition ra2
```

客户端运行需要用户自行准备合法取得的游戏安装目录。本仓库不包含零售 MIX、音频、地图或其他原版资源。产品安装与架构背景可查阅 [上游说明](https://github.com/rust-alert/ra2.exe/tree/c61fbf2105ae6d741b2e0e324de9fd06de3013e9) 和各 crate 文档。上游 @game-gpt/red-alert2 npm 包不代表已经包含本研究分支的改动，本分支没有自动向上游 npm 包发布的流程。

欢迎针对 Agent 接口、仿真正确性、回放兼容性和性能瓶颈提交 PR；策略源码请优先提交到 agents 仓库。讨论与提交约定见 [贡献指南](CONTRIBUTING.md)。
