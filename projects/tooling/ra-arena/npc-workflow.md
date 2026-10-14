# NPC 改进与一键评测

在仓库根目录运行：

```powershell
.\test-npc.cmd
# 或在 Windows / Linux / macOS 上：
node scripts/npc-test.mjs
# 或：
pnpm npc:test
```

需要 Node.js 20+ 和仓库声明的 Rust nightly；无需安装 Node 依赖、窗口、GPU 或零售游戏资源。Windows 脚本会自动找到用户目录中的 Cargo，不要求先修改 PATH。

默认先跑报告对比测试、NPC 回归和矿车记账测试，再使用 `arena` 优化增量构建。六个参赛者 `candidate`、`hard-baseline-v1`、`reference-v4`、`reference-v5`、`reference-v7`、`counter-retreat-v1` 在 `duel-v1`、`skirmish-v1`、`economy-v1` 各打四轮镜像循环赛，共 360 局，每个 NPC 120 局。诊断另外保存候选版对 Hard 的 6 局命令轨迹，再从轨迹回放并核对命令、终局哈希、胜负和全部遥测。

当前候选已完成多轮改进，实测结果和停止继续调参的依据见 [optimization-report.md](optimization-report.md)。七代改进对手的冻结文件摘要见 [baselines/champions-v1.json](baselines/champions-v1.json)。脚本同时检查原始 Hard 和这些冻结对手。

```mermaid
flowchart LR
    edit[修改 candidate.rs] --> frozen[核验冻结基线]
    frozen --> tests[回归与指标测试]
    tests --> build[优化增量构建]
    build --> league[固定种子镜像联赛]
    league --> replay[诊断比赛与回放核验]
    replay --> report[JSON / Markdown / CSV]
    report --> compare[对比上次相同赛程]
    compare --> edit
```

每次在 `arena-results/npc/时间-label/` 新建目录，保留 `index.md` 总报告、`league.json` 原始结果、`league.csv` 指标、`candidate.json` 候选和 Hard 对照、`comparison.md/json` 历史差值、`replay.json` 诊断轨迹、`steps.json` 各阶段耗时和 `logs/` 日志。`source/` 保存工作区构建源码和测试资源快照，`manifest.json` 保存文件 SHA-256、编译器版本、Git 提交、工作区是否有改动和二进制指纹；`bin/` 保留本次评测二进制，可直接回放本次 `replay.json`。

失败立即退出并写入 `failure.json`，不更新 `latest.json`。即使未提交 Git，候选版也有精确的源码快照；如果运行期间源码或二进制发生变化，本次报告标记失败，重跑即可。

## 如何修改

复制出的代码清单见 [baselines/hard-v1/readme.md](baselines/hard-v1/readme.md)。候选策略文件在 [`ra-engine/src/gameplay/npc/candidate.rs`](../../engine/ra-engine/src/gameplay/npc/candidate.rs)，注册入口在 [`src/bot.rs`](src/bot.rs)。

修改候选文件中的一个函数后，执行相同命令。脚本自动读取最近成功的记录；只有共同引擎、指标版本、规则、对手及其顺序、场景、种子、轮次、预算和 Elo 参数一致时才计算历史差值。不同参赛池的 Elo 不作差；并发数和存储选项可以不同。每次同时挑战冻结 Hard、强版本和反制策略。

一键流程的 `candidate` 是实验席位，具体策略版本由源码 SHA-256 和快照区分。候选与基线即使代码相同，也可能因各自对其他对手使用的地图种子和在线 Elo 结算顺序不同，出现小幅 Elo 差异；以复制一致性测试、直接交手与同赛程历史对比判断变化。

```powershell
# release 构建和引擎/适配器/类型/ECS/测试工具回归，默认 30 轮、2700 局
.\test-npc.cmd --full --label economy-change
# 新种子检查泛化：同一轮仍有冻结 Hard 对照，历史对比会提示赛程不同
.\test-npc.cmd --seed 20261014 --rounds 20
# 指定历史记录；路径是运行目录，不是 JSON 文件
.\test-npc.cmd --compare arena-results/npc/某次运行目录
# 资源占用控制
.\test-npc.cmd --jobs 1
```

固定训练种子用于迭代，另选未用于调参的种子验收。不要降低 tick 预算来声称策略变强；超时一律按平局计分。测试不强制候选版始终等于 Hard，所以策略修改后无需删掉基线。

## 指标口径

| 指标 | 定义与解释 |
|---|---|
| 资源获取速度 | 实际卸矿到账累计金额 / 累计仿真时长 × 60；单位资金/游戏分钟，15 tick = 1 秒。不含初始资金、退款、出售和间谍转账，不受同时生产扣款干扰 |
| 前 60 秒资源速度 | 每局前 900 tick 的实际收入 / 这些窗口累计时长；提前结束使用实际时长 |
| 首次收入时间 | 有收入局的首次卸矿 tick 均值 / 15；同时显示有收入局数，没收入的局不填 0 |
| 平均对局得分 | `(胜 + 和/2) / 总局数`，范围 0–1 |
| 平均游戏积分 | 引擎临时积分 `max(0, 支出/100 + 击杀×10 − 损失×5)` 的局均值；该公式会奖励花钱，不能单凭它判断变强 |
| Elo | 初始 1500、K=32、尺度 400；每个镜像双局一次更新，具体数学见 [readme.md](readme.md) |
| 命令拒绝率 | 引擎拒绝命令 / 实际提交命令；有助于发现重复下单、建造竞争、资金不足或越权 |
| 其他 CSV 字段 | 平均支出、余额、击杀、损失、生产完成数、低电占比和超预算命令数 |

报告保留各场景指标。`duel-v1` 没有经济，资源指标为 0；评价经济优先看 `economy-v1`。总体资源平均包含无矿场景，不能当作纯运营效率。旧报告无遥测字段时显示未知，不补成 0。历史差值描述固定样本中的变化，不宣称统计显著性。

经济场景使用对称平地、同科技阵营、双方 10000 开局资金、MCV、镜像矿石区、矿场 `FreeUnit=CMIN`，矿车按现有引擎自动寻矿、采集、返场并卸货。矿石是有限密度；没有虚拟被动收入。原有 `duel-v1` 和 `skirmish-v1` 规则保留。

当前评测是 Rust 重写引擎的全图信息协议，不含战争迷雾、完整原版规则和真实地图，不代表原版 RA2 天梯强度。当前候选在这个场景池中已进入局部瓶颈；要验证更广泛的强度，需要扩展地图、完整科技与视野协议，并重新训练和验收。

## 原始 Hard 代码审查与实验方向

| 位置 | 当前行为 | 可验证的候选改动 | 重点指标 |
|---|---|---|---|
| `heuristic_ai_commands` / `build_or_place` | 电厂、矿场、兵营和车厂都读取同一拍旧状态并生成命令，可能竞争同一空闲建造场或同一份余额 | 明确建造优先级，一拍只生成可执行的经济/建筑队列决策，预留军费与建造资金 | 拒绝率、矿场时间、首次收入、得分 |
| `heuristic_should_produce_army` | 活着的战斗单位上限 8；首批后每 45 tick 下单；不计已排队单位 | 将在建与队列纳入数量预算，按资金、工厂能力和威胁调整节奏 | 得分、Elo、支出、损失 |
| Hard 额外生产尝试 | 从同一旧状态再生成一次生产请求；引擎可把第二次请求放入待生产队列，可能一次扣两份钱 | 显式管理期望队列深度，避免盲目重复，不预先假定删掉就更强 | 拒绝率、经济窗口、直接交手得分 |
| `pick_techno` | 按科技、便宜程度、类型名选单位；未以输出、克制、存活率衡量，也未明确排除无武器经济载具 | 先区分矿车与战斗兵种，再按对手组成和资源预算选兵 | 击杀/损失、经济收入、Elo |
| `house_has_power` / `place_power_commands` | 只检查有没有电厂；不按实际电力缺口补电 | 检查当前/未来耗电，必要时增建电厂 | 低电占比、生产完成数、得分 |
| `place_refinery_commands` / 放置搜索 | 有一座矿场后停止扩张；建筑按建造场附近开放格搜索，没有针对矿位和运输路径优化 | 按矿车效率和矿区距离选择落点，补充矿车/矿场 | 首次收入、前60秒和全局采矿速度 |
| `auto_attack_commands` / `nearest_enemy` | 单位逐个找最近敌人；有目标就不重选；没建造场不主动进攻 | 集火、威胁与经济目标排序、撤退和重新接敌；明确残军行为 | 战斗场景得分、损失、超时率 |

该表记录原始 Hard 的问题。当前候选已经实现预算和队列管理、持续生产、动态补电、矿区布局、协同选敌、伤害分配、射程拉扯和混编部队残血撤退。额外矿车/矿场、队列加深、单一兵种等方案经过实验后未成为默认策略；具体正负结果见迭代报告。
