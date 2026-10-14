use super::session::BattleSession;

impl BattleSession {
    /// 为所有非本地、非氛围阵营下发本 tick 的 AI 命令（经 `push_command`）。
    ///
    /// `Neutral` / `Civilian` 地图装饰房主不参与遭遇战 AI。
    /// 若该房主已由 `[AITriggerTypes]` 驱动产队：只做 MCV / 供电 / 矿场等经济基建，
    /// **不下发**兵营、车厂、工厂量产与全图追打（部队由 AITrigger → Create Team / Script 负责）。
    /// 否则走启发式基建，并对工厂量产做节流，避免空闲工厂每 tick 乱刷最便宜单位。
    /// `Easy`：奇数 tick 跳过生产与自动进攻，仅保留部署/建造节奏。
    /// `Normal`：每 4 个 tick 跳过一拍进攻/生产（略弱于 Hard）。
    /// `Hard`：每 tick 完整下发，并追加一轮生产尝试（仍受上述节流约束）。
    pub(super) fn push_ai_commands(&mut self) {
        let local_house = self.world.players.iter().find(|p| p.id == self.world.local_player).map(|p| p.house.clone());
        let opponents: Vec<ra_types::PlayerId> = self
            .world
            .players
            .iter()
            .filter(|p| !crate::gameplay::ai::is_ambient_house(p.house.as_ref()))
            .filter(|p| local_house.as_ref().map(|h| p.house.as_ref() != h.as_ref()).unwrap_or(true))
            .map(|p| p.id)
            .collect();
        for player in &opponents {
            for cmd in crate::heuristic_ai_commands(&self.world, *player, &self.difficulty) {
                self.world.push_player_command(*player, cmd);
            }
        }
    }
}

/// 按难度决定本 tick 是否跳过 AI 进攻/生产。
pub fn difficulty_skips_offensive(difficulty: &str, tick: u64) -> bool {
    if difficulty.eq_ignore_ascii_case("Easy") {
        tick % 2 == 1
    }
    else if difficulty.eq_ignore_ascii_case("Hard") {
        false
    }
    else {
        tick % 4 == 3
    }
}

/// Hard 是否追加一轮生产尝试。
pub fn difficulty_extra_produce(difficulty: &str) -> bool {
    difficulty.eq_ignore_ascii_case("Hard")
}
