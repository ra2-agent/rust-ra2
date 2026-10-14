use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ArenaResult;

/// 首位 NPC 的单局结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Score {
    /// 首位胜。
    Win,
    /// 平局。
    Draw,
    /// 首位负。
    Loss,
}

impl Score {
    /// 胜 / 和 / 负分别为 1 / 0.5 / 0 分。
    pub fn points(self) -> f64 {
        match self {
            Self::Win => 1.0,
            Self::Draw => 0.5,
            Self::Loss => 0.0,
        }
    }

    /// 从另一方视角看同一局。
    pub fn reversed(self) -> Self {
        match self {
            Self::Win => Self::Loss,
            Self::Draw => Self::Draw,
            Self::Loss => Self::Win,
        }
    }
}

/// Elo 参数。每个镜像双局作为一次更新，使用两局平均得分。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EloConfig {
    /// 新 NPC 的起始分，默认 1500。
    pub initial: f64,
    /// 每个双局的更新系数，默认 32。
    pub k: f64,
    /// 十倍胜算对应的分差，默认 400。
    pub scale: f64,
    /// 少于此局数显示为暂定分，默认 20。
    pub provisional_games: u64,
}

impl Default for EloConfig {
    fn default() -> Self {
        Self { initial: 1500.0, k: 32.0, scale: 400.0, provisional_games: 20 }
    }
}

impl EloConfig {
    /// 检查有限数字与正系数。
    pub fn validate(&self) -> ArenaResult<()> {
        if !self.initial.is_finite() || !self.k.is_finite() || self.k <= 0.0 || !self.scale.is_finite() || self.scale <= 0.0 {
            return Err("Elo initial 必须有限，k 和 scale 必须为有限正数".into());
        }
        Ok(())
    }

    /// 首位玩家对次位玩家的预期得分。
    pub fn expected(&self, a: f64, b: f64) -> f64 {
        1.0 / (1.0 + 10.0_f64.powf((b - a) / self.scale))
    }
}

/// 一个版本的 NPC 在一个评测池中的累计记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    /// 注册的策略版本 ID。
    pub npc: String,
    /// 未取整的 Elo。
    pub elo: f64,
    /// 胜局数。
    pub wins: u64,
    /// 和局数。
    pub draws: u64,
    /// 负局数。
    pub losses: u64,
}

impl Rating {
    /// 实际完成的局数。
    pub fn games(&self) -> u64 {
        self.wins + self.draws + self.losses
    }

    /// 包含平局半分的得分率；无比赛时为 0。
    pub fn score_rate(&self) -> f64 {
        if self.games() == 0 { 0.0 } else { (self.wins as f64 + 0.5 * self.draws as f64) / self.games() as f64 }
    }

    fn add(&mut self, score: Score) {
        match score {
            Score::Win => self.wins += 1,
            Score::Draw => self.draws += 1,
            Score::Loss => self.losses += 1,
        }
    }
}

/// 固定参数的 Elo 排名表。
pub struct EloTable {
    config: EloConfig,
    ratings: BTreeMap<String, Rating>,
}

impl EloTable {
    /// 所有 NPC 从相同起始分开始；拒绝空列表和重复 ID。
    pub fn new(config: EloConfig, ids: &[String]) -> ArenaResult<Self> {
        config.validate()?;
        let mut ratings = BTreeMap::new();
        for npc in ids {
            let rating = Rating { npc: npc.clone(), elo: config.initial, wins: 0, draws: 0, losses: 0 };
            if npc.is_empty() || ratings.insert(npc.clone(), rating).is_some() {
                return Err(format!("空或重复 NPC ID: {npc:?}"));
            }
        }
        if ratings.is_empty() {
            return Err("排名表至少需要一个 NPC".into());
        }
        Ok(Self { config, ratings })
    }

    /// 使用同一个赛前预期得分，按系列平均分零和更新；输入结果均从 a 视角给出。
    ///
    /// 交换出生位的两局一起调用，防止先输后赢与先赢后输产生人为分差。
    pub fn rate_series(&mut self, a: &str, b: &str, scores: &[Score]) -> ArenaResult<()> {
        if a == b || scores.is_empty() {
            return Err("系列需要两个不同 NPC 和至少一个结果".into());
        }
        let first = self.ratings.get(a).ok_or_else(|| format!("未注册排名: {a}"))?;
        let second = self.ratings.get(b).ok_or_else(|| format!("未注册排名: {b}"))?;
        let actual = scores.iter().map(|s| s.points()).sum::<f64>() / scores.len() as f64;
        let delta = self.config.k * (actual - self.config.expected(first.elo, second.elo));
        let new_a = first.elo + delta;
        let new_b = second.elo - delta;
        if !new_a.is_finite() || !new_b.is_finite() {
            return Err("Elo 更新溢出；请检查参数".into());
        }
        let first = self.ratings.get_mut(a).unwrap();
        first.elo = new_a;
        for score in scores {
            first.add(*score);
        }
        let second = self.ratings.get_mut(b).unwrap();
        second.elo = new_b;
        for score in scores {
            second.add(score.reversed());
        }
        Ok(())
    }

    /// 分数降序，平分时按策略 ID 排序。
    pub fn leaderboard(&self) -> Vec<Rating> {
        let mut rows: Vec<_> = self.ratings.values().cloned().collect();
        rows.sort_by(|a, b| b.elo.total_cmp(&a.elo).then(a.npc.cmp(&b.npc)));
        rows
    }
}
