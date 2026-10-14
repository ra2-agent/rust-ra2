# ra-arena：NPC 对战与 Elo 评测

**NPC 改进一键入口：在仓库根目录运行 `.\test-npc.cmd` 或 `node scripts/npc-test.mjs`。** 已复制最高难度 Hard 为冻结基线和独立候选版，增加真实矿车经济场景、资源速度、游戏积分、CSV、历史差值、源码快照与回放核验。完整用法与代码审查见 [npc-workflow.md](npc-workflow.md)，冻结源码见 [baselines/hard-v1](baselines/hard-v1/readme.md)。

使用仓库现有 `ra-engine` 的权威命令、经济、生产、战斗与胜负路径运行 NPC 对战。无需窗口、GPU 或零售游戏资源。Rust workspace 的 `projects/tooling/*` 已自动包含本 crate。

## 项目已有的 NPC 代码

| 代码 | 当前能力 |
|---|---|
| [`ra-engine/src/gameplay/ai.rs`](../../engine/ra-engine/src/gameplay/ai.rs) | 部署 MCV、建电厂/矿场/兵营/车厂、量产步兵和载具、寻找敌人并进攻 |
| [`ra-engine/src/game/ai.rs`](../../engine/ra-engine/src/game/ai.rs) | 对非本地玩家驱动 AI，Easy / Normal / Hard 控制进攻和生产节奏 |
| [`ra-engine/src/gameplay/ai_triggers.rs`](../../engine/ra-engine/src/gameplay/ai_triggers.rs) | 地图 AITrigger 条件检查和队伍创建 |
| [`ra-engine/src/gameplay/script_teams.rs`](../../engine/ra-engine/src/gameplay/script_teams.rs) | TeamTypes / ScriptTypes 队伍脚本执行 |
| [`ra-testing/src/headless.rs`](../ra-testing/src/headless.rs) | 无窗口对战、MCV、建造和 AI 遭遇战夹具 |

这是启发式与地图脚本 AI 基础。当前没有现成的强化学习训练器、外部 NPC 插件协议或证明达到“最强”的策略。内置启发式军队上限为 8 个作战单位，首批之后每 45 tick 尝试量产；选兵主要按科技等级和造价。困难模式的额外生产尝试不保证它在所有场景中都优于其他难度。

本次增加的 `ra_engine::heuristic_ai_commands(&world, player, difficulty)` 对任意席位返回内置 AI 命令，与正常游戏 AI 共用实现。评测时关闭自动 AI，由参赛 NPC 分别接管双方。

## 快速运行

在仓库根目录运行：

```powershell
cargo run -p ra-arena -- list
cargo run -p ra-arena -- run --rounds 4 --seed 42 --trace --out arena-results/baseline.json
cargo run -p ra-arena -- leaderboard arena-results/baseline.json
cargo run -p ra-arena -- replay arena-results/baseline.json
```

默认四个 NPC、两个场景、四轮，共 **96 局**；每个 NPC 48 局。报告保存 JSON 比赛记录和同名 Markdown 排名、分场景排名与交手得分率矩阵。`arena-results/` 已加入 gitignore。

如当前 PowerShell 找不到 `cargo`，本机已有 Rust 工具链，可在本次会话加入路径后使用上述命令：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

更多轮次与配置：

```powershell
cargo run -p ra-arena -- run --config projects/tooling/ra-arena/arena.example.json --out arena-results/league.json
cargo run -p ra-arena -- run --bots easy-v1,normal-v1,hard-v1 --scenarios skirmish-v1 --rounds 10
```

CLI 选项覆盖 JSON 配置，无论 `--config` 在参数中的位置。全部选项见 `--help`。默认每局 3000 仿真 tick（15 Hz 下相当于 200 秒游戏时间），计算速度与墙钟无关。批量评测可使用 `cargo run --release -p ra-arena -- ...`。

## 批量加速与资源控制

先构建优化二进制，批量任务直接启动它，避免每次调用 Cargo：

```powershell
cargo build --release -p ra-arena
# 控制资源：最多两个镜像双局同时执行，只保留排名与统计。
.\target\release\ra-arena.exe run --rounds 1000 --jobs 2 --summary --out arena-results/large.json
# 吞吐优先：按系统可用逻辑 CPU 数选择并发。
.\target\release\ra-arena.exe run --rounds 1000 --jobs auto --summary --out arena-results/fast.json
# 少量诊断局保存完整回放。
.\target\release\ra-arena.exe run --rounds 1 --jobs 2 --trace --out arena-results/debug.json
```

- `jobs` 默认 **1**，旧配置仍可读取。固定数量的线程各执行一个镜像双局，两局顺序执行；每局世界与 NPC 独立。并发提升吞吐，也会提高瞬时 CPU 与内存占用，低占用可选择 1–2。
- 队列与乱序结果按批限制为最多 `jobs` 个双局，不为所有轮次预先创建任务。规则与只读引擎在一次循环赛中只构建一次并共享。无论线程何时完成，都按原来的固定种子赛程更新 Elo；确定性 NPC 的种子、命令、终局哈希和分数保持一致。
- `--summary` / `summary_only: true` 保留综合榜、场景榜、交手得分率、总局数、总 tick、超时和命令统计，及时丢弃逐局记录。固定 NPC 数、场景数和并发数时，内存与报告体积不随轮次线性增长。需要逐局种子、终局哈希或回放时去掉该选项。
- 批量评测默认不保存轨迹。`--trace` 的占用随总 tick 与命令量增长，不能与 `--summary` 同用。配置中已有 `trace: true` 时，可通过 `--no-trace` 关闭。
- JSON 默认紧凑输出；`--pretty` 恢复缩进格式。报告直接写入缓冲文件，不额外构造整份 JSON 字节数组。两种格式都可读取和回放；精简报告可以用 `leaderboard` 查看，不能回放。
- CLI 显示仿真耗时、局/秒与 tick/秒，耗时不包含编译与报告写入。不要为了加速跳过仿真 tick 或降低 NPC 决策频率，这会改变比赛语义；缩短 `max_ticks` 也会改变超时平局比例。

`arena.example.json` 使用 4 并发、精简报告。NPC 工厂需满足 `Send + Sync`，因为工厂会从多个线程调用；NPC 实例在线程内创建和使用，无须实现 `Send`。工厂与 NPC 不应通过全局可变状态跨局交换信息。workspace 的 release 配置使用 `panic=abort`：release 中 NPC panic 会终止进程并使本次运行失败；调试构建可捕获 panic 并返回错误。

Windows 可用随附脚本比较串行、并行和精简模式，记录耗时、累计 CPU 时间、峰值工作集、采样私有内存与报告大小，并核对排名一致性：

```powershell
.\projects\tooling\ra-arena\scripts\benchmark.ps1 -Rounds 10 -Jobs 4 -Repeats 3
# 如果保留了优化前的二进制，还可加入同赛程对照：
.\projects\tooling\ra-arena\scripts\benchmark.ps1 -BaselineBinary .\arena-results\perf\ra-arena-before.exe
```

脚本耗时包含启动与报告写入，排除编译；报告与指标写入独立的 `arena-results/benchmark-*` 目录。先在自己机器上比较 1、2、4 和 auto 并发，再选吞吐与资源占用合适的配置。

并发与报告优化阶段的本机实测（Ryzen 7 9800X3D，系统可用 8 个逻辑 CPU，约 31 GiB 内存，release，seed=42，原四个内置 NPC、两个场景、每局最多 3000 tick）：

| 模式 | 240 局耗时，三次中位数 | JSON 大小 |
|---|---:|---:|
| 优化前串行、缩进完整报告 | 4.94 秒 | 156134 字节 |
| 优化后串行、紧凑完整报告 | 4.93 秒 | 90815 字节 |
| 优化后 4 并发、紧凑完整报告 | 2.64 秒 | 90815 字节 |
| 优化后 4 并发、精简报告 | 2.65 秒 | 2041 字节 |

四并发吞吐约为原来的 **1.9 倍**，精简 JSON 比原报告小 **98.7%**。逐局记录（包括种子、胜负、终局哈希、命令统计）与优化前相同；综合 Elo、场景 Elo 和精简统计也一致。串行计算耗时改善很小，主要加速来自并发，主要容量节约来自不保留逐局数据。

该阶段另一次 2400 局测试：串行 47.98 秒，4 并发完整报告 24.85 秒，4 并发精简报告 24.89 秒。完整 JSON 为 889636 字节，精简 JSON 为 2087 字节。精简模式峰值工作集从 240 局的约 6.3 MiB 到 2400 局的约 6.7 MiB；后者完整模式约 7.8 MiB。CPU 时间读数有波动，单靠并发未据此认定总 CPU 计算量显著下降。更复杂规则、地图或 NPC 的每局资源需求可能更高，应在实际场景重新测量。

### 减少总 CPU 工作量

引擎底层优化自动生效，不需要改变 NPC 决策频率或比赛 tick：

- 一个组件修改只更新对应的兼容投影；完整投影复用字符串和路径缓冲，不再克隆生产候补与航点等无关数据。
- tick 末全量投影从逐 ID 线性查找槽位改为直接扫描槽位，移除这一同步路径的 O(N²) 搜索。
- ECS 全量投影与哈希使用只读组件视图，每种组件存储每次扫描只定位一次；哈希每个实体只解析一次句柄，同一组件各字段复用查询结果。
- 规范化的规则键查询不分配字符串；生产系统跳过空队列，移动查询借用路径，战斗事件使用稳定 `HouseId`。

随附分配计数基准固定使用原四个 NPC、`duel-v1` / `skirmish-v1`、seed=42 和单线程：

```powershell
cargo run --release -p ra-arena --example cpu_bench -- 10
# 普通二进制测累计 CPU 时间，固定串行，排除分配计数器和编译的影响。
.\projects\tooling\ra-arena\scripts\benchmark.ps1 -Rounds 100 -Repeats 3 -IncludeModes serial
```

分配计数器只测 `run_tournament`，不包含 JSON 输出、CLI Git 查询或注册表构建；累计申请字节数包含每次重新分配请求，**不是峰值内存**。CPU 脚本记录进程用户态与内核态时间之和及每仿真 tick 的 CPU 微秒数，耗时包含普通报告写入。

240 局、232339 tick 的计数结果：堆分配从 **29654208** 次降到 **4697136** 次，减少 **84.2%**；累计申请字节从 **658846421** 降到 **302467784**，减少 **54.1%**。新构建包含同时新增的经济遥测，所以这些是完整版本的对照结果，不能据此把收益精确归因到某一个改动。

普通 release 二进制的最终 CPU 对照：原四个 NPC、原两个场景，固定 **2400 局 / 2331394 tick / jobs=1**，优化前后各运行三次，取中位数。

| 指标 | 底层优化前 | 底层优化后 | 减少 |
|---|---:|---:|---:|
| 累计进程 CPU 时间 | 11.81 秒 | 8.34 秒 | 29.4% |
| 每仿真 tick 的 CPU 时间 | 5.07 微秒 | 3.58 微秒 | 29.4% |
| 实际耗时，含启动和写报告 | 47.12 秒 | 30.90 秒 | 34.4% |

这组使用相同串行参数的结果显示本机总 CPU 开销下降，不能把分配次数减少 84% 解读成 CPU 也减少 84%。CPU 时间仍有系统计时与调度波动，其他地图、规则和自定义策略应另测。底层优化后 2400 局逐局胜负、tick、终局哈希、命令统计及 Elo 与原版本一致。

正确性使用优化前二进制的 `golden-matches-v1.json` 检查 24 个固定开局，另外核对 96 局完整命令轨迹和逐局结果，并用新引擎回放旧报告。玩法有意改变时才更新基线；新增遥测字段由独立测试验证。

### 分组件性能计时

使用可选 `profiling` feature 和 `--profile` 开启，常规构建不编译任何引擎计时插桩。先用串行 release 定位热点：

```powershell
cargo run --release -p ra-arena --features profiling -- run --profile --jobs 1 --rounds 10 --summary --out arena-results/profile.json
# 大赛程可每 10 tick 抽样一次，减少读取时钟与更新统计的次数。
cargo run --release -p ra-arena --features profiling -- run --profile --profile-every 10 --jobs 1 --rounds 100 --summary --out arena-results/profile-sampled.json
# 读取计时报告无需 profiling feature。
cargo run -p ra-arena -- profile arena-results/profile.json
```

`profile: true` 与 `profile_sample_every: 1` 也可在 JSON 配置中指定。`--profile-every` 只设置采样间隔，需配合 `--profile`。未编译 feature 时启用计时会在赛前报错并给出构建命令；旧配置默认关闭，旧报告没有计时数据时不会补零。

JSON 的 `profiling` 字段和同名 Markdown 保存完整计时；另输出 `profile.profile.csv` 长表，包含 `overall`、`scenario`、`npc` 三种 scope。精简模式同样保留计时。统计按线程收集并按比赛合并，不保存逐 tick 历史，空间随组件、场景与 NPC 数增长，不随比赛轮次增长。

| 范围 | 分类 |
|---|---|
| NPC / 评测器 | 单独按 NPC ID 记录决策；装载、命令提交/拒绝审计、胜负检查、经济遥测、轨迹编码、结算 |
| 命令 / 移动 / 战斗 | 输入队列、命令应用、跟随、移动、寻路、闪白、攻击、渗透、占领、停电、炮塔 |
| 经济 / 脚本 | 矿车收入、生产、修理、EVA、超武、闪电、矿柱、地图触发器、AI 触发器、队伍创建和脚本 |
| ECS / 摘要 | 全量同步、局部投影、哈希；修改按 identity、owner、health、transform、locomotor、movement、combat_stats、attack、production、harvester、animation 分开统计 |

每项保存实测调用数、累计 inclusive / self 纳秒数、单次最大值和固定 64 桶 self 直方图。Markdown 按 **self 累计耗时** 降序，显示 self 占比、平均微秒、近似 P95 上界、最大微秒及 inclusive 总时间。CSV 使用毫秒 / 微秒列；JSON 原始时间统一为纳秒。跨比赛的均值和 P95 按调用数合并，避免把少量长调用与大量短调用等权平均。

口径与使用限制：

- 这是 `Instant` 的单调**墙钟耗时**，包含调度暂停和等待；不是每组件的 CPU 时间。并发累计对局时间可能大于循环赛墙钟时间。总 CPU 成本仍用普通构建的进程 CPU 基准验证。
- `inclusive` 包含已插桩子调用，`self` 扣除它们；同一分组内以 self 总和计算占比。不同 scope、inclusive 和 self 不能再次相加。`engine_tick` 的 self 含阶段调度、未单独计时工作及计时统计开销，不应直接当作某个游戏系统的成本。
- P95 使用 log2 桶上界，最多约两倍精度，属于近似值。抽样调用数和累计时间都是**实测样本**，没有乘采样间隔外推总量；周期抽样可能与 AI/生产周期混叠，确认热点时用 `--profile-every 1` 或多个间隔复测。
- 装载和结算始终测量，tick 从 1 起抽样。NPC 内部的预测世界可计入其 inclusive 决策耗时，但不会增加真实比赛 tick 计数或改变采样节奏。
- 插桩有开销，特别是纳秒级 ECS 修改；实际优化收益必须在不启用 `profiling` feature 的相同赛程上验证。循环赛总墙钟还包含规则解析、调度和计时汇总，最终文件写入不包含在其中；组件明细聚焦每局运行。

引擎消费方也可使用 `ra_engine::profiling::Capture` 与 `span(Stage)` 添加新的分类计时。捕获和范围不能跨线程移动，范围按栈顺序释放；错误或 panic 自动清理，不污染下一局。计时状态位于线程局部存储，不进入权威世界、命令协议、状态哈希或回放轨迹。

## 评测协议与公平性

协议 ID：`ra-arena-full-info-v1`。

- 每个 NPC 组合、每个场景、每轮都打两局；使用同一地图种子和初始状态，第二局交换 NPC 的左右出生位。
- 两边用相同单位科技和资金，避免将阵营差异当作 NPC 智力差异。
- 双方先读取同一 tick 的只读世界，再统一提交命令。每 tick 轮换两边入队的先后顺序。
- 每边每 tick 最多提交 64 条命令；可配置。超额截断，报告记录 `dropped`。
- 命令信封中的玩家由评测器指定，不能被 NPC 的载荷覆盖。引擎检查单位所有权、造价、科技、生产与放置条件；拒绝数计入 `rejected`。
- 每局创建新的 NPC 实例，并调用 `reset(seed, player)`。NPC 应使用传入种子初始化自身随机数，不能依赖墙钟或系统随机源。
- 胜者由引擎的 `sole_victor()` 判定，不用本地玩家的 Victory / Defeat 文案猜测获胜方。
- 到达 tick 上限记平局；双方同时被消灭记平局。NPC panic 是评测错误，整次运行失败，不写出成功报告，也不作为平局增加分数。
- 赛程使用主种子确定洗牌顺序，避免一直按注册顺序执行。相同源码、配置和确定性 NPC 会复现相同结果与终局哈希。

v1 允许双方读取完整 `BattleState`，包括敌人和经济，**没有战争迷雾过滤**。NPC 是进程内 Rust 实现，命令上限不限制 NPC 思考耗时。若后续需要公平的视野和 CPU 预算，应增加过滤后的观测协议、独立进程或超时机制，并使用新的协议版本重新评测。

## 场景池

规则位于 [`fixtures/rules-v1.ini`](fixtures/rules-v1.ini)，使用合成内容。

| 场景 | 双方开局 | 胜负规则 | 重点 |
|---|---|---|---|
| `duel-v1` | 3 辆坦克、1 座建造场、0 资金 | 长局：消灭所有保活单位/建筑 | 接敌、移动、集火、目标选择 |
| `skirmish-v1` | 1 辆 MCV、10000 资金 | 短局：消灭建筑与 MCV | 展开、建造、简化矿场经济、生产、进攻 |

作战场景保留建造场，是因为现有游戏 AI 在无建造场时不会主动追击。它没有资金可用于扩张。地图种子改变开局纵坐标和双方间距；当前地图为对称平地，尚未覆盖复杂地形、完整科技与模组机制。原有 `skirmish-v1` 没有矿车和矿石，无法衡量采矿效率；新增 `economy-v1` 使用矿场赠送矿车和镜像矿区，执行实际采集、运输与卸货。原有两个场景规则保持不变。

分数只对同一引擎、协议、规则和场景池成立，不能直接解释为原版 RA2 的 Elo。扩展真实规则/地图时，在 `Scenario` 和 `scenario::open` 增加版本化场景，并扩展报告的规则指纹；视野等协议变化时升级 `PROTOCOL`。

## Elo 如何计分

新 NPC 默认 **1500 分**，分差尺度为 **400**，K 为 **32**。

```text
E(A) = 1 / (1 + 10 ^ ((R(B) - R(A)) / 400))
S(A) = (第一局得分 + 第二局得分) / 2
Δ = K × (S(A) - E(A))
R'(A) = R(A) + Δ
R'(B) = R(B) - Δ
```

胜 / 和 / 负分别为 1 / 0.5 / 0。两局得分都先换算为同一个 NPC 的视角，用同一个赛前预期一起更新，因此先胜后负与先负后胜没有结算顺序偏差。K 是**每个镜像双局**的系数，不是每个单局；同分双方赢下两局增加 16 分，一胜一负不变。每组更新保持总分不变。

综合榜每个镜像双局更新一次；场景榜仅使用该场景的双局单独更新。报告同时显示胜和负、得分率及交手矩阵，帮助识别策略之间的克制关系。少于 20 局显示为“暂定”；达到阈值仅说明样本数量，不表示已收敛或有统计显著性。

每次 `run` 是一个独立评测池，从初始分重新计算；保存报告可供读取、比较和回放，当前没有跨运行续分。比较策略版本时，将新旧版本与相同基线放进**同一个循环赛**，固定种子和场景，并检查交手得分率；不要把不同参赛池的绝对 Elo 直接相减。

## 接入你自己的 NPC

实现 `Npc` 后注册工厂即可。NPC 通过只读观测决定行为，通过 `GameCommand` 操作，不直接修改权威世界。

```rust,no_run
use ra_arena::{ArenaConfig, BotContext, BotRegistry, Npc, run_tournament};
use ra_engine::GameCommand;

struct MyNpc;

impl Npc for MyNpc {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        // 从现有策略起步，再替换建造顺序、单位组合和作战控制。
        ra_engine::heuristic_ai_commands(context.world, context.player, "Hard")
    }
}

let mut registry = BotRegistry::builtins();
registry.register("my-npc-v1", || Box::new(MyNpc))?;
let mut config = ArenaConfig::default();
config.npcs.push("my-npc-v1".into());
let report = run_tournament(&registry, config)?;
for row in report.leaderboard {
    println!("{}: {:.1}", row.npc, row.elo);
}
# Ok::<(), String>(())
```

可直接运行的接入示例：[`examples/custom_npc.rs`](examples/custom_npc.rs)。CLI 的参赛目录在 `BotRegistry::builtins()`；自定义程序可以使用自己的目录。外部可执行程序和原版游戏 AI 暂不能直接作为参赛者。

```powershell
cargo run -p ra-arena --example custom_npc
```

策略改动后使用新版本 ID，如 `my-npc-v2`，避免同一个名字混合不同策略。先优化作战场景里的目标选择与部队控制，再优化运营场景里的生产和经济；每一步都让新旧版本互打，保留旧版和固定基线，防止只对一个对手过拟合。

## 报告与回放

完整 JSON 包含协议、规则摘要、引擎版本、源码提交和工作区改动标志、全部配置、总榜/分榜、累计统计、每局种子、出生位、结束原因、命令统计和终局哈希。精简模式不保存逐局记录。`--trace` 额外保存每 tick 实际进入引擎的编码命令信封。

`replay` 不执行 NPC，重新创建场景后将记录的命令送入引擎。它检查连续 tick、命令编码和顺序、命令统计、胜负、结束原因及终局哈希。当前引擎源码改变后可能无法回放旧报告，请保留对应源码版本。

## 验证

```powershell
cargo test -p ra-arena
cargo test -p ra-adaptor -p ra-engine -p ra-testing -p ra-arena
cargo test -p ra-engine -p ra-arena --features ra-arena/profiling
```

测试覆盖 Elo 数学、镜像公平性、NPC 接管和内置 AI 开关、双方胜负、固定种子复现、命令越权和预算、panic 错误、并发与串行结果一致、活跃 NPC 数受并发上限约束、精简统计等价、旧报告兼容，以及命令轨迹回放和篡改检测。计时测试覆盖嵌套自身耗时、采样、预测世界隔离、跨比赛统计合并，以及开启计时前后的比赛和回放一致性。现有 CI 分别测试普通构建与 profiling 构建。
