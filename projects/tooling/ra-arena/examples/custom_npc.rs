//! 自定义 NPC 接入示例：目前沿用 Hard 策略，可在 `commands` 内逐步替换决策。
use ra_arena::{ArenaConfig, ArenaResult, BotContext, BotRegistry, Npc, run_tournament};
use ra_engine::GameCommand;

struct MyNpc;

impl Npc for MyNpc {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        ra_engine::heuristic_ai_commands(context.world, context.player, "Hard")
    }
}

fn main() -> ArenaResult<()> {
    let mut registry = BotRegistry::builtins();
    registry.register("my-npc-v1", || Box::new(MyNpc))?;
    let mut config = ArenaConfig { rounds: 2, ..ArenaConfig::default() };
    config.npcs.push("my-npc-v1".into());
    let report = run_tournament(&registry, config)?;
    for row in &report.leaderboard {
        println!("{}: {:.1} Elo，{} 局", row.npc, row.elo, row.games());
    }
    std::fs::create_dir_all("arena-results").map_err(|e| e.to_string())?;
    std::fs::write("arena-results/custom.json", serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(())
}
