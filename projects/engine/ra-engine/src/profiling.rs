//! 可选分项墙钟计时。每线程一次捕获，固定大小直方图，无逐 tick 历史。
//! 嵌套范围同时记录 inclusive 与 self 时间；时钟和统计开销包含在测量中。

use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

macro_rules! stages {
    ($($variant:ident => $key:literal),* $(,)?) => {
        /// 稳定的计时分类；不影响系统执行顺序。
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(usize)]
        pub enum Stage { $(#[doc = $key] $variant,)* }
        impl Stage {
            /// 全部分类，按声明顺序。
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
            /// 用于 JSON / CSV 的稳定分类名。
            pub fn key(self) -> &'static str { match self { $(Self::$variant => $key,)* } }
        }
    };
}

stages! {
    MatchSetup => "match_setup",
    NpcLeft => "npc_left",
    NpcRight => "npc_right",
    CommandSubmit => "command_submit",
    CommandAudit => "command_audit",
    Telemetry => "telemetry",
    TraceEncode => "trace_encode",
    MatchFinish => "match_finish",
    ArenaOutcome => "arena_outcome_check",
    SessionHints => "session_hints",
    AutomaticAi => "automatic_ai",
    Outcome => "engine_outcome_check",
    EngineTick => "engine_tick",
    CommandQueue => "command_queue",
    ApplyCommands => "apply_commands",
    Follow => "follow",
    Movement => "movement",
    HitFlash => "hit_flash",
    Combat => "combat",
    Infiltrate => "infiltrate",
    CaptureBuilding => "capture_building",
    Blackouts => "power_blackouts",
    Turrets => "turrets",
    RefineryIncome => "refinery_income",
    Production => "production",
    Repairs => "repairs",
    EvaAdvice => "eva_advice",
    SuperWeapons => "super_weapons",
    LightningStorm => "lightning_storm",
    TerrainSpawn => "terrain_spawn",
    MapTriggers => "map_triggers",
    AiTriggers => "ai_triggers",
    TeamSpawns => "team_spawns",
    TeamScripts => "team_scripts",
    ScriptCrates => "script_crates",
    EcsSync => "ecs_full_sync",
    StateHash => "state_hash",
    EcsIdentity => "ecs_mutation.identity",
    EcsOwner => "ecs_mutation.owner",
    EcsHealth => "ecs_mutation.health",
    EcsTransform => "ecs_mutation.transform",
    EcsLocomotor => "ecs_mutation.locomotor",
    EcsMovement => "ecs_mutation.movement",
    EcsCombatStats => "ecs_mutation.combat_stats",
    EcsAttack => "ecs_mutation.attack",
    EcsProduction => "ecs_mutation.production",
    EcsHarvester => "ecs_mutation.harvester",
    EcsAnimation => "ecs_mutation.animation",
    PartialProjection => "ecs_partial_projection",
    Pathfinding => "pathfinding",
}

/// 固定大小统计；直方图为 self 纳秒数的 log2 桶，分位数返回桶上界。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingStats {
    /// 实际测量的调用数。
    pub calls: u64,
    /// 包含已插桩子调用的累计纳秒数。
    pub inclusive_ns: u64,
    /// 扣除已插桩子调用的累计纳秒数。
    pub self_ns: u64,
    /// 单次 inclusive 时间的最大值。
    pub max_inclusive_ns: u64,
    /// 单次 self 时间的最大值。
    pub max_self_ns: u64,
    /// self 时间直方图；0 桶涵盖 0–1 ns，之后按二的幂划分。
    pub histogram: [u64; 64],
}

impl Default for TimingStats {
    fn default() -> Self {
        Self { calls: 0, inclusive_ns: 0, self_ns: 0, max_inclusive_ns: 0, max_self_ns: 0, histogram: [0; 64] }
    }
}

impl TimingStats {
    fn record(&mut self, inclusive: u64, own: u64) {
        self.calls += 1;
        self.inclusive_ns = self.inclusive_ns.saturating_add(inclusive);
        self.self_ns = self.self_ns.saturating_add(own);
        self.max_inclusive_ns = self.max_inclusive_ns.max(inclusive);
        self.max_self_ns = self.max_self_ns.max(own);
        let bucket = if own <= 1 { 0 } else { (64 - (own - 1).leading_zeros()) as usize };
        self.histogram[bucket.min(63)] += 1;
    }
}

/// 一次捕获结果；每局各自捕获，多个线程由消费方合并。
#[derive(Debug, Clone)]
pub struct ProfileData {
    /// 捕获的实际墙钟纳秒数，包含未插桩与统计开销。
    pub elapsed_ns: u64,
    /// 实际观察 tick 数（同一 tick 的重复 begin 不重复计数）。
    pub observed_ticks: u64,
    /// 被抽样的 tick 数。
    pub sampled_ticks: u64,
    /// 分类统计，与 `Stage::ALL` 顺序一致。
    pub stages: Vec<TimingStats>,
}

struct ActiveSpan {
    children_ns: u64,
}

struct State {
    id: u64,
    every: u64,
    active: bool,
    last_tick: Option<u64>,
    started: Instant,
    observed_ticks: u64,
    sampled_ticks: u64,
    stages: Vec<TimingStats>,
    stack: Vec<ActiveSpan>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// 单线程捕获范围；panic / 提前返回时自动清理上下文，不能跨线程移动。
pub struct Capture {
    id: u64,
    _thread: PhantomData<Rc<()>>,
}

impl Capture {
    /// 开始一次捕获；拒绝零采样间隔和同线程重叠的捕获。
    pub fn start(sample_every: u64) -> Result<Self, &'static str> {
        if sample_every == 0 {
            return Err("profile_sample_every 必须大于零");
        }
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.is_some() {
                return Err("同一线程已有 profiling 捕获");
            }
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            *state = Some(State {
                id,
                every: sample_every,
                active: true,
                last_tick: None,
                started: Instant::now(),
                observed_ticks: 0,
                sampled_ticks: 0,
                stages: vec![TimingStats::default(); Stage::ALL.len()],
                stack: Vec::with_capacity(8),
            });
            Ok(Self { id, _thread: PhantomData })
        })
    }

    /// 结束捕获；调用前须让所有 span 离开作用域。
    pub fn finish(self) -> ProfileData {
        STATE.with(|state| {
            let state = state.borrow_mut().take().expect("捕获上下文应存在");
            assert_eq!(state.id, self.id);
            assert!(state.stack.is_empty(), "finish 前必须释放所有计时范围");
            ProfileData {
                elapsed_ns: nanos(state.started),
                observed_ticks: state.observed_ticks,
                sampled_ticks: state.sampled_ticks,
                stages: state.stages,
            }
        })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.as_ref().is_some_and(|state| state.id == self.id) {
                *state = None;
            }
        });
    }
}

/// 选择本 tick 是否计时。总是从 tick 1 开始，每 `every` tick 采样一次。
pub fn begin_tick(tick: u64) {
    STATE.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            // NPC 中的预测世界由外层范围控制采样，不计为真实比赛 tick。
            if !state.stack.is_empty() {
                return;
            }
            state.active = tick.saturating_sub(1) % state.every == 0;
            if state.last_tick != Some(tick) {
                state.last_tick = Some(tick);
                state.observed_ticks += 1;
                state.sampled_ticks += u64::from(state.active);
            }
        }
    });
}

/// 对局装载和结算在 tick 采样之外，始终测量。
pub fn outside_tick() {
    STATE.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            if !state.stack.is_empty() {
                return;
            }
            state.active = true;
        }
    });
}

/// 嵌套计时范围；无捕获或未采样时不读取时钟，不能跨线程移动。
pub struct Span {
    active: Option<(u64, usize, Stage, Option<Instant>)>,
    _thread: PhantomData<Rc<()>>,
}

/// 开始分类计时；结果须按栈顺序释放。
pub fn span(stage: Stage) -> Span {
    let active = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = state.as_mut()?;
        let depth = state.stack.len();
        state.stack.push(ActiveSpan { children_ns: 0 });
        Some((state.id, depth, stage, state.active.then(Instant::now)))
    });
    Span { active, _thread: PhantomData }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some((id, depth, stage, started)) = self.active
        else {
            return;
        };
        let elapsed = started.map(nanos);
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let Some(state) = state.as_mut().filter(|state| state.id == id)
            else {
                return;
            };
            assert_eq!(state.stack.len(), depth + 1, "span 必须按栈顺序释放");
            let frame = state.stack.pop().unwrap();
            if let Some(elapsed) = elapsed {
                state.stages[stage as usize].record(elapsed, elapsed.saturating_sub(frame.children_ns));
                if let Some(parent) = state.stack.last_mut() {
                    parent.children_ns = parent.children_ns.saturating_add(elapsed);
                }
            }
        });
    }
}

fn nanos(started: Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_spans_account_for_child_time_once() {
        let capture = Capture::start(1).unwrap();
        {
            let _parent = span(Stage::Movement);
            let _child = span(Stage::Pathfinding);
        }
        let data = capture.finish();
        let parent = data.stages[Stage::Movement as usize];
        let child = data.stages[Stage::Pathfinding as usize];
        assert_eq!(parent.calls, 1);
        assert_eq!(child.calls, 1);
        assert_eq!(parent.self_ns + child.inclusive_ns, parent.inclusive_ns);
        assert_eq!(parent.histogram.iter().sum::<u64>(), 1);
    }

    #[test]
    fn sampling_counts_unique_ticks_and_cleanup_survives_unwind() {
        let capture = Capture::start(3).unwrap();
        for tick in 1..=5 {
            begin_tick(tick);
            begin_tick(tick);
            let _timer = span(Stage::Combat);
        }
        let data = capture.finish();
        assert_eq!((data.observed_ticks, data.sampled_ticks), (5, 2));
        assert_eq!(data.stages[Stage::Combat as usize].calls, 2);
        let _ = std::panic::catch_unwind(|| {
            let _capture = Capture::start(1).unwrap();
            let _timer = span(Stage::Combat);
            panic!("cleanup");
        });
        let empty = Capture::start(1).unwrap().finish();
        assert!(empty.stages.iter().all(|s| s.calls == 0));
        assert!(Capture::start(0).is_err());
    }

    #[test]
    fn prediction_worlds_inside_npc_do_not_change_sampling_or_actual_tick_counts() {
        let capture = Capture::start(3).unwrap();
        for tick in 1..=2 {
            begin_tick(tick);
            let _npc = span(Stage::NpcLeft);
            for future in [100, 101, 102] {
                begin_tick(future);
                let _simulation = span(Stage::Combat);
            }
        }
        let data = capture.finish();
        assert_eq!((data.observed_ticks, data.sampled_ticks), (2, 1));
        assert_eq!(data.stages[Stage::Combat as usize].calls, 3);
        assert_eq!(data.stages[Stage::NpcLeft as usize].calls, 1);
    }
}
