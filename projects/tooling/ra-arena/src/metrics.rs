//! 确定性的对局遥测；时间单位来自引擎仿真 tick，与机器运行速度无关。

use ra_engine::{BattleSession, BattleState, DEFAULT_TICK_HZ};
use ra_types::PlayerId;
use serde::{Deserialize, Serialize};

/// 指标口径版本；改变采样或聚合含义时升级。
pub const METRICS_VERSION: &str = "npc-metrics-v1";

/// 单局单席位的实测指标；旧报告缺少此结构时表示未知，不能补零。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerMetrics {
    /// 开局资金，不计入采矿收入。
    pub initial_funds: i32,
    /// 终局剩余资金。
    pub final_funds: i32,
    /// 矿车卸货的实际到账金额，不含初始资金、退款和转账。
    pub harvested_credits: u64,
    /// 前 60 游戏秒实际观察到的采矿收入；提前结束时窗口也提前结束。
    pub opening_harvested: u64,
    /// 前 60 秒窗口的实际观察 tick 数。
    pub opening_ticks: u64,
    /// 引擎累计支出。
    pub spent: i32,
    /// 引擎当前结算积分，和 Elo 的胜/和/负得分分开。
    pub battle_score: i32,
    /// 击杀数。
    pub kills: u32,
    /// 损失数。
    pub losses: u32,
    /// 建造和生产完成数。
    pub built: u32,
    /// 首次采矿到账时刻；整局没有收入时为 null。
    pub first_income_tick: Option<u64>,
    /// 首座矿场存活的时刻；始终没有矿场时为 null。
    pub first_refinery_tick: Option<u64>,
    /// 低电 tick 数。
    pub low_power_ticks: u64,
}

/// 一个 NPC 的累计指标；保留总量与分母，支持精简模式和正确的时长加权速率。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcMetrics {
    /// 有遥测的局数。
    pub games: u64,
    /// 累计观察 tick。
    pub ticks: u64,
    /// 实际采矿收入总额。
    pub harvested_credits: u64,
    /// 前 60 秒窗口收入总额。
    pub opening_harvested: u64,
    /// 前 60 秒窗口的累计 tick。
    pub opening_ticks: u64,
    /// 累计支出。
    pub spent: i64,
    /// 累计终局余额。
    pub final_funds: i64,
    /// 累计引擎结算积分。
    pub battle_score: i64,
    /// 累计击杀。
    pub kills: u64,
    /// 累计损失。
    pub losses: u64,
    /// 累计建造和生产完成数。
    pub built: u64,
    /// 有采矿收入的局数；计算首次收入均值时的分母。
    pub income_games: u64,
    /// 有收入局的首次到账 tick 之和。
    pub first_income_ticks: u64,
    /// 出现矿场的局数。
    pub refinery_games: u64,
    /// 有矿场局的首次矿场 tick 之和。
    pub first_refinery_ticks: u64,
    /// 低电 tick 总数。
    pub low_power_ticks: u64,
    /// 提交至引擎的命令数。
    pub submitted: u64,
    /// 被引擎拒绝的命令数。
    pub rejected: u64,
    /// 超预算截断的命令数。
    pub dropped: u64,
}

impl NpcMetrics {
    pub(crate) fn record(&mut self, m: &PlayerMetrics, ticks: u64, submitted: u64, rejected: u64, dropped: u64) {
        self.games += 1;
        self.ticks += ticks;
        self.harvested_credits += m.harvested_credits;
        self.opening_harvested += m.opening_harvested;
        self.opening_ticks += m.opening_ticks;
        self.spent += i64::from(m.spent);
        self.final_funds += i64::from(m.final_funds);
        self.battle_score += i64::from(m.battle_score);
        self.kills += u64::from(m.kills);
        self.losses += u64::from(m.losses);
        self.built += u64::from(m.built);
        self.low_power_ticks += m.low_power_ticks;
        self.submitted += submitted;
        self.rejected += rejected;
        self.dropped += dropped;
        if let Some(tick) = m.first_income_tick {
            self.income_games += 1;
            self.first_income_ticks += tick;
        }
        if let Some(tick) = m.first_refinery_tick {
            self.refinery_games += 1;
            self.first_refinery_ticks += tick;
        }
    }

    /// 总采矿收入 / 总游戏时长，单位：资金 / 游戏分钟。
    pub fn income_per_minute(&self) -> f64 {
        rate(self.harvested_credits, self.ticks)
    }

    /// 各局前 60 秒窗口合并后的资金 / 游戏分钟。
    pub fn opening_income_per_minute(&self) -> f64 {
        rate(self.opening_harvested, self.opening_ticks)
    }

    /// 平均引擎结算积分；无样本返回 0。
    pub fn average_battle_score(&self) -> f64 {
        if self.games == 0 { 0.0 } else { self.battle_score as f64 / self.games as f64 }
    }
}

fn rate(credits: u64, ticks: u64) -> f64 {
    if ticks == 0 { 0.0 } else { credits as f64 * f64::from(DEFAULT_TICK_HZ) * 60.0 / ticks as f64 }
}

pub(crate) struct MatchTelemetry {
    players: [PlayerId; 2],
    initial_harvested: [u64; 2],
    metrics: [PlayerMetrics; 2],
}

impl MatchTelemetry {
    pub(crate) fn new(world: &BattleState, players: [PlayerId; 2]) -> Self {
        let mut result = Self { players, initial_harvested: [0; 2], metrics: Default::default() };
        for (seat, id) in players.iter().enumerate() {
            let p = world.players.iter().find(|p| p.id == *id).unwrap();
            result.initial_harvested[seat] = p.harvested_credits;
            result.metrics[seat].initial_funds = p.funds;
        }
        result.observe(world);
        result
    }

    pub(crate) fn observe(&mut self, world: &BattleState) {
        for seat in 0..2 {
            let p = world.players.iter().find(|p| p.id == self.players[seat]).unwrap();
            let m = &mut self.metrics[seat];
            m.harvested_credits = p.harvested_credits - self.initial_harvested[seat];
            if world.tick <= u64::from(DEFAULT_TICK_HZ) * 60 {
                m.opening_harvested = m.harvested_credits;
                m.opening_ticks = world.tick;
            }
            if m.harvested_credits > 0 && m.first_income_tick.is_none() {
                m.first_income_tick = Some(world.tick);
            }
            m.low_power_ticks += u64::from(world.tick > 0 && p.low_power());
            if m.first_refinery_tick.is_none() {
                let has_refinery = (0..world.entity_count()).filter_map(|i| world.entity_id_at(i)).any(|id| {
                    world.ecs_owner(id).is_some_and(|o| o == p.house)
                        && world.ecs_health(id).is_some_and(|(_, _, dead)| !dead)
                        && world
                            .ecs_identity(id)
                            .is_some_and(|(key, _)| world.definitions.structures.get(key.as_ref()).is_some_and(|s| s.refinery))
                });
                if has_refinery {
                    m.first_refinery_tick = Some(world.tick);
                }
            }
        }
    }

    pub(crate) fn finish(mut self, battle: &BattleSession) -> [PlayerMetrics; 2] {
        let stats = battle.current_battle_stats();
        for seat in 0..2 {
            let p = battle.world.players.iter().find(|p| p.id == self.players[seat]).unwrap();
            let row = stats.players.iter().find(|s| s.house == p.house.as_ref()).unwrap();
            let m = &mut self.metrics[seat];
            m.final_funds = p.funds;
            m.spent = p.funds_spent;
            m.battle_score = row.score;
            m.kills = row.kills;
            m.losses = row.losses;
            m.built = row.built;
        }
        self.metrics
    }
}
