use std::collections::BTreeMap;

use ra_engine::{BattleState, GameCommand};
use ra_types::PlayerId;

use crate::ArenaResult;

/// 一拍只读观测。v1 为全图信息协议，双方权限相同。
pub struct BotContext<'a> {
    /// 当前权威世界；NPC 不能通过此引用修改它。
    pub world: &'a BattleState,
    /// 此 NPC 的玩家席位；命令始终以该玩家身份提交。
    pub player: PlayerId,
}

/// 可替换的 Rust NPC；每局使用全新实例，状态不能串局。
pub trait Npc {
    /// 初始化局内状态与私有随机数；相同镜像对的种子相同。
    fn reset(&mut self, _seed: u64, _player: PlayerId) {}

    /// 对当前世界生成命令；双方观察同一 tick 后才统一入队。
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand>;
}

/// 内置 AI 的评测包装，与游戏中的启发式策略共用代码。
pub struct HeuristicBot {
    difficulty: &'static str,
}

impl HeuristicBot {
    /// 包装 Easy / Normal / Hard 难度策略。
    pub fn new(difficulty: &'static str) -> Self {
        Self { difficulty }
    }
}

impl Npc for HeuristicBot {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        ra_engine::heuristic_ai_commands(context.world, context.player, self.difficulty)
    }
}

/// 不主动发出命令的基线；引擎的被动接敌与防守仍正常工作。
#[derive(Default)]
pub struct IdleBot;

impl Npc for IdleBot {
    fn commands(&mut self, _context: &BotContext<'_>) -> Vec<GameCommand> {
        Vec::new()
    }
}

/// NPC 工厂目录；ID 应包含策略版本，修改策略后注册新 ID。
#[derive(Default)]
pub struct BotRegistry {
    factories: BTreeMap<String, Box<dyn Fn() -> Box<dyn Npc> + Send + Sync>>,
}

impl BotRegistry {
    /// 注册可跨线程调用、每局创建新 NPC 的工厂，拒绝空 ID 与重复 ID。
    pub fn register(&mut self, id: &str, factory: impl Fn() -> Box<dyn Npc> + Send + Sync + 'static) -> ArenaResult<()> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)) {
            return Err(format!("非法 NPC ID: {id:?}；只允许字母、数字、-、_"));
        }
        if self.factories.contains_key(id) {
            return Err(format!("NPC ID 已注册: {id}"));
        }
        self.factories.insert(id.into(), Box::new(factory));
        Ok(())
    }

    /// 创建指定 NPC；未知 ID 返回错误。
    pub fn create(&self, id: &str) -> ArenaResult<Box<dyn Npc>> {
        self.factories.get(id).map(|factory| factory()).ok_or_else(|| format!("未知 NPC: {id}"))
    }

    /// 按 ID 排序的可用 NPC 清单。
    pub fn ids(&self) -> Vec<String> {
        self.factories.keys().cloned().collect()
    }

    /// 是否存在指定 NPC。
    pub fn contains(&self, id: &str) -> bool {
        self.factories.contains_key(id)
    }

    /// 空闲基线与游戏现有的三档启发式策略。
    pub fn builtins() -> Self {
        let mut registry = Self::default();
        registry.register("idle-v1", || Box::new(IdleBot)).unwrap();
        for (id, difficulty) in [("easy-v1", "Easy"), ("normal-v1", "Normal"), ("hard-v1", "Hard")] {
            registry.register(id, move || Box::new(HeuristicBot::new(difficulty))).unwrap();
        }
        registry.register("hard-baseline-v1", || Box::new(CopiedHardBot { candidate: false })).unwrap();
        registry.register("candidate", || Box::new(CopiedHardBot { candidate: true })).unwrap();
        registry.register("reference-v1", || Box::new(ReferenceBot(1))).unwrap();
        registry.register("reference-v2", || Box::new(ReferenceBot(2))).unwrap();
        registry.register("reference-v3", || Box::new(ReferenceBot(3))).unwrap();
        registry.register("reference-v4", || Box::new(ReferenceBot(4))).unwrap();
        registry.register("reference-v5", || Box::new(ReferenceBot(5))).unwrap();
        registry.register("reference-v6", || Box::new(ReferenceBot(6))).unwrap();
        registry.register("reference-v7", || Box::new(ReferenceBot(7))).unwrap();
        registry.register("counter-retreat-v1", || Box::new(ReferenceBot(8))).unwrap();
        for (id, variant) in [
            ("tune-cap8", 1),
            ("tune-cap16", 2),
            ("tune-cap64", 3),
            ("tune-infantry", 4),
            ("tune-vehicles", 5),
            ("tune-harvester2", 6),
            ("tune-refinery2", 7),
            ("tune-factory-first", 8),
            ("tune-micro0", 9),
            ("tune-micro2", 10),
            ("tune-micro10", 11),
            ("tune-threat", 12),
            ("tune-kite", 13),
            ("tune-queue2", 14),
            ("tune-forward", 15),
            ("tune-kite-threat", 16),
            ("tune-kite-forward", 17),
            ("tune-all3", 18),
            ("tune-kite-queue2", 19),
            ("tune-kite-micro10", 20),
            ("tune-retreat", 21),
            ("tune-reserve", 22),
            ("tune-adaptive", 23),
            ("tune-mixed-retreat", 24),
            ("tune-mixed-defensive", 25),
            ("tune-adaptive-hp", 26),
            ("tune-kite-late", 27),
            ("tune-kite-early", 28),
            ("tune-forward2", 29),
            ("tune-forward6", 30),
            ("tune-health25", 31),
            ("tune-health75", 32),
            ("tune-final-early", 33),
            ("tune-final-queue2", 34),
            ("tune-final-cap16", 35),
            ("tune-final-cap64", 36),
            ("tune-final-forward2", 37),
            ("tune-final-forward6", 38),
            ("tune-final-reserve", 39),
            ("tune-final-micro10", 40),
            ("tune-final-hp", 41),
        ] {
            registry.register(id, move || Box::new(VariantBot(variant))).unwrap();
        }
        registry
    }
}

struct ReferenceBot(u32);

struct VariantBot(u32);

impl Npc for VariantBot {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        ra_engine::candidate_variant_npc_commands(context.world, context.player, self.0)
    }
}

impl Npc for ReferenceBot {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        ra_engine::reference_npc_commands(context.world, context.player, self.0)
    }
}

struct CopiedHardBot {
    candidate: bool,
}

impl Npc for CopiedHardBot {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        if self.candidate {
            ra_engine::candidate_npc_commands(context.world, context.player)
        }
        else {
            ra_engine::hard_baseline_v1_commands(context.world, context.player)
        }
    }
}
