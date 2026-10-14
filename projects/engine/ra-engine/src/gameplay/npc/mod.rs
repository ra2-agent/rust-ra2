//! NPC 实验代码：冻结基线与独立候选副本共用引擎，策略实现互不调用。

mod candidate;
mod hard_v1;
#[allow(dead_code)]
mod reference_v1;
#[allow(dead_code)]
mod reference_v2;
#[allow(dead_code)]
mod reference_v3;
#[allow(dead_code)]
mod reference_v4;
#[allow(dead_code)]
mod reference_v5;
#[allow(dead_code)]
mod reference_v6;
#[allow(dead_code)]
mod reference_v7;

/// 冻结的各轮改进与反制策略，供候选版自我对战使用；版本 1–7 和反制版本 8。
pub fn reference_npc_commands(world: &crate::BattleState, player: ra_types::PlayerId, revision: u32) -> Vec<crate::GameCommand> {
    match revision {
        1 => reference_v1::heuristic_ai_commands(world, player, "Hard"),
        2 => reference_v2::heuristic_ai_commands(world, player, "Hard"),
        3 => reference_v3::heuristic_ai_commands(world, player, "Hard"),
        4 => reference_v4::heuristic_ai_commands(world, player, "Hard"),
        5 => reference_v5::variant_commands(world, player, 16),
        6 => reference_v6::variant_commands(world, player, 23),
        7 => reference_v7::variant_commands(world, player, 24),
        8 => reference_v5::variant_commands(world, player, 21),
        _ => hard_v1::heuristic_ai_commands(world, player, "Hard"),
    }
}

/// 同一策略实现的明确参数变体，用于固定种子的批量调参；0 为当前候选配置。
pub fn candidate_variant_npc_commands(world: &crate::BattleState, player: ra_types::PlayerId, variant: u32) -> Vec<crate::GameCommand> {
    candidate::variant_commands(world, player, variant)
}

/// 2026-10-14 复制的 Hard 策略；修改候选版时保持此实现不变。
pub fn hard_baseline_v1_commands(world: &crate::BattleState, player: ra_types::PlayerId) -> Vec<crate::GameCommand> {
    hard_v1::heuristic_ai_commands(world, player, "Hard")
}

/// 待改进 NPC 的入口；在 `npc/candidate.rs` 修改建造、经济、生产与战斗函数。
pub fn candidate_npc_commands(world: &crate::BattleState, player: ra_types::PlayerId) -> Vec<crate::GameCommand> {
    candidate::heuristic_ai_commands(world, player, "Hard")
}
