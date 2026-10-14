use super::types::BattleState;
use crate::game::InputFrame;

impl BattleState {
    /// 推进一个逻辑 tick（使用默认 [`SystemSchedule`]）。
    pub fn advance_tick(&mut self) {
        self.advance_scheduled_tick(&crate::engine::SystemSchedule::standard());
    }

    /// 按调度表推进一个逻辑 tick（阶段顺序的唯一来源）。
    ///
    /// 只消费 `tick <= 当前 tick` 的待执行命令。更晚的调度命令留在队列中，禁止提前执行。
    pub fn advance_scheduled_tick(&mut self, schedule: &crate::engine::SystemSchedule) {
        use crate::engine::SystemPhase;

        #[cfg(feature = "profiling")]
        crate::profiling::begin_tick(self.tick.wrapping_add(1));
        #[cfg(feature = "profiling")]
        let _tick_timer = crate::profiling::span(crate::profiling::Stage::EngineTick);
        macro_rules! timed {
            ($stage:ident, $work:expr) => {{
                #[cfg(feature = "profiling")]
                let _timer = crate::profiling::span(crate::profiling::Stage::$stage);
                $work
            }};
        }

        let due = timed!(CommandQueue, {
            self.tick = self.tick.wrapping_add(1);
            let mut due = Vec::new();
            let mut deferred = Vec::new();
            for cmd in std::mem::take(&mut self.pending_commands) {
                if cmd.tick.0 <= self.tick {
                    due.push(cmd);
                }
                else {
                    deferred.push(cmd);
                }
            }
            self.pending_commands = deferred;
            self.last_input_frame = InputFrame { tick: self.tick, commands: due.clone() };
            self.last_rejects.clear();
            due
        });
        for phase in &schedule.phases {
            match *phase {
                SystemPhase::ApplyCommands => timed!(ApplyCommands, self.apply_commands(&due)),
                SystemPhase::Movement => {
                    timed!(Follow, self.resolve_follow());
                    timed!(Movement, self.advance_movement());
                }
                SystemPhase::HitFlash => timed!(HitFlash, self.tick_hit_flash()),
                SystemPhase::Combat => {
                    timed!(Combat, self.resolve_combat());
                    timed!(Infiltrate, self.resolve_infiltrate());
                    timed!(CaptureBuilding, self.resolve_capture_building());
                    timed!(Blackouts, self.tick_power_blackouts());
                }
                SystemPhase::Turrets => timed!(Turrets, self.advance_turrets()),
                SystemPhase::RefineryIncome => timed!(RefineryIncome, self.advance_refinery_income()),
                SystemPhase::Production => {
                    timed!(Production, self.advance_production());
                    timed!(Repairs, crate::gameplay::tick_repairs(self));
                    timed!(EvaAdvice, self.tick_eva_funds_nag());
                }
                SystemPhase::Powers => {
                    timed!(SuperWeapons, crate::gameplay::tick_super_weapon_charges(self));
                    timed!(LightningStorm, crate::gameplay::tick_lightning_storm(self));
                }
                SystemPhase::TerrainSpawn => timed!(TerrainSpawn, self.advance_terrain_spawners()),
                SystemPhase::Triggers => {
                    timed!(MapTriggers, crate::gameplay::tick_triggers(self));
                    timed!(AiTriggers, crate::gameplay::tick_ai_triggers(self));
                    timed!(TeamSpawns, crate::gameplay::flush_pending_team_spawns(self));
                    timed!(TeamScripts, crate::gameplay::tick_script_teams(self));
                    timed!(ScriptCrates, crate::gameplay::tick_script_crates(self));
                }
                SystemPhase::Rehash => {
                    self.sync_ecs_components();
                    self.rehash();
                }
            }
        }
    }
}
