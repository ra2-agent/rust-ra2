//! 使用真实 `ra-engine` tick 的 NPC 评测；内置合成规则，不需要窗口或零售资源。
#![deny(missing_docs)]

mod bot;
mod elo;
mod metrics;
mod profile;
mod runner;
mod scenario;

pub use bot::{BotContext, BotRegistry, HeuristicBot, IdleBot, Npc};
pub use elo::{EloConfig, EloTable, Rating, Score};
pub use metrics::{METRICS_VERSION, NpcMetrics, PlayerMetrics};
pub use profile::{ArenaProfile, ProfileGroup, TimingStats};
pub use runner::{
    ArenaConfig, MatchRecord, MatchSpec, MatchupStats, Report, Termination, TournamentSummary, TraceFrame, play_match, replay_match,
    run_tournament,
};
pub use scenario::Scenario;

/// 评测错误；失败不计为平局，也不更新排名。
pub type ArenaResult<T> = Result<T, String>;

/// 评测协议版本；规则、观测权限或调度语义变化时应升级。
pub const PROTOCOL: &str = "ra-arena-full-info-v1";
