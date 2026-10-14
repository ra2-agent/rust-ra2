# Hard 策略代码副本

这是 2026-10-14 当前工作区中 Rust 引擎 NPC 的副本。不是 Westwood 原版闭源 `ra2.exe` 的反编译源码。

- 可执行冻结副本：[hard_v1.rs](../../../../engine/ra-engine/src/gameplay/npc/hard_v1.rs)。已复制完整启发式逻辑和难度节奏函数，不再调用游戏中可变化的 AI 实现。
- 可编辑候选副本：[candidate.rs](../../../../engine/ra-engine/src/gameplay/npc/candidate.rs)。首次交付与冻结副本相同；之后只在这里改进策略。
- 原始启发式源码：[source/heuristic_ai.rs](source/heuristic_ai.rs)。
- 难度驱动：[source/difficulty_driver.rs](source/difficulty_driver.rs)。
- 地图队伍触发器：[source/ai_triggers.rs](source/ai_triggers.rs)。
- 队伍脚本执行：[source/script_teams.rs](source/script_teams.rs)。
- 冻结文件 SHA-256：[manifest.json](manifest.json)。一键脚本会验证这些文件；不自动更新基线。

触发器和队伍脚本副本供阅读；当前合成评测地图没有 AITrigger，不执行这些副本。基线冻结的是决策代码，科技条件、生产、移动、伤害、经济等底层规则仍来自共同引擎。更改共同引擎后，一键脚本会重新建立历史参照，不将结果变化归因于候选版。

原始 `ai.rs` 中调用难度辅助函数的两处引用在可执行副本中改为本模块辅助函数；其余逻辑保持复制时行为。候选与基线各有自己的函数，不共享策略函数。回归测试检查冻结副本的命令和原始 Hard 相同，历史金样检查引擎对局哈希、胜负和命令统计。
