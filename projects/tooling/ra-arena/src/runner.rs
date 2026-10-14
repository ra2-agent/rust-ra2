use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, mpsc},
    thread,
};

use ra_engine::{BattleSession, Engine, EngineConfig, decode_scheduled, encode_scheduled};
use ra_types::PlayerId;
use serde::{Deserialize, Serialize};

use crate::{
    ArenaProfile, ArenaResult, BotContext, BotRegistry, EloConfig, EloTable, METRICS_VERSION, NpcMetrics, PROTOCOL, PlayerMetrics, Rating,
    Scenario, Score, metrics::MatchTelemetry, scenario,
};

/// 循环赛配置；所有 NPC 使用相同 tick 与命令预算。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaConfig {
    /// 参赛策略版本；顺序参与固定种子赛程的生成。
    pub npcs: Vec<String>,
    /// 评测场景池，每个场景等权参加循环赛。
    pub scenarios: Vec<Scenario>,
    /// 每个 NPC 组合、每个场景的镜像双局次数。
    pub rounds: u32,
    /// 主随机种子。
    pub seed: u64,
    /// 每局最大仿真 tick；到达后记平局。
    pub max_ticks: u64,
    /// 每 NPC 每 tick 的命令上限；超额截断并记录。
    pub max_commands_per_tick: usize,
    /// 是否保存每 tick 的命令信封以便回放。
    pub trace: bool,
    /// 同时运行的镜像双局数；默认 1，限制 CPU 与活跃世界数量。
    #[serde(default = "default_jobs")]
    pub jobs: usize,
    /// 只保存累计统计和排名；内存、磁盘占用不随轮次线性增长，不支持回放。
    #[serde(default)]
    pub summary_only: bool,
    /// 开启分项耗时统计；须用 `profiling` feature 构建。
    #[serde(default)]
    pub profile: bool,
    /// 每多少仿真 tick 计时一次，默认 1；统计不外推未测量的 tick。
    #[serde(default = "default_profile_sample_every")]
    pub profile_sample_every: u64,
    /// Elo 参数。
    pub elo: EloConfig,
}

impl Default for ArenaConfig {
    fn default() -> Self {
        Self {
            npcs: vec!["idle-v1".into(), "easy-v1".into(), "normal-v1".into(), "hard-v1".into()],
            scenarios: vec![Scenario::Duel, Scenario::Skirmish],
            rounds: 4,
            seed: 42,
            max_ticks: 3000,
            max_commands_per_tick: 64,
            trace: false,
            jobs: 1,
            summary_only: false,
            profile: false,
            profile_sample_every: 1,
            elo: EloConfig::default(),
        }
    }
}

fn default_jobs() -> usize {
    1
}

fn default_profile_sample_every() -> u64 {
    1
}

impl ArenaConfig {
    /// 赛前验证；未知 NPC、重复场景与零预算会被拒绝。
    pub fn validate(&self, registry: &BotRegistry) -> ArenaResult<()> {
        self.elo.validate()?;
        if self.profile_sample_every == 0 {
            return Err("profile_sample_every 必须大于零".into());
        }
        if self.profile && !cfg!(feature = "profiling") {
            return Err("分项计时需要 profiling 构建：cargo run --release -p ra-arena --features profiling -- run --profile".into());
        }
        if self.jobs == 0 {
            return Err("jobs 必须大于零；CLI 可用 --jobs auto 自动选择".into());
        }
        if self.summary_only && self.trace {
            return Err("summary_only 与 trace 不能同时开启；精简评测请关闭 trace".into());
        }
        if self.npcs.len() < 2 || self.scenarios.is_empty() || self.rounds == 0 || self.max_ticks == 0 || self.max_commands_per_tick == 0 {
            return Err("需要至少两个 NPC、一个场景，且 rounds、max_ticks、命令上限都必须大于零".into());
        }
        for (index, id) in self.npcs.iter().enumerate() {
            if !registry.contains(id) {
                return Err(format!("未知 NPC: {id}"));
            }
            if self.npcs[..index].contains(id) {
                return Err(format!("重复 NPC: {id}"));
            }
        }
        for (index, scene) in self.scenarios.iter().enumerate() {
            if self.scenarios[..index].contains(scene) {
                return Err(format!("重复场景: {}", scene.id()));
            }
        }
        self.match_count()?;
        Ok(())
    }

    /// 计划单局数；拒绝会溢出的赛程。
    pub fn match_count(&self) -> ArenaResult<u64> {
        let n = self.npcs.len() as u64;
        n.checked_mul(n.saturating_sub(1))
            .and_then(|n| n.checked_mul(self.scenarios.len() as u64))
            .and_then(|n| n.checked_mul(u64::from(self.rounds)))
            .ok_or_else(|| "赛程局数溢出".into())
    }
}

/// 单局开局与运行参数；数组顺序是固定的左右出生位。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchSpec {
    /// 场景。
    pub scenario: Scenario,
    /// 地图与 NPC 私有随机源的种子。
    pub seed: u64,
    /// 左 / 右出生位的 NPC ID。
    pub seats: [String; 2],
    /// 最大 tick 数。
    pub max_ticks: u64,
    /// 每 NPC 每 tick 的命令上限。
    pub max_commands_per_tick: usize,
    /// 保存命令轨迹。
    pub trace: bool,
}

/// 结束原因；超时和双方同时出局都独立于引擎的本地胜负文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Termination {
    /// 一方被消灭。
    Elimination,
    /// 双方均不再保活。
    MutualElimination,
    /// 到达最大 tick 数。
    TickLimit,
}

/// 一拍实际进入引擎的命令；包含信封 ID、玩家、tick 与编码后的载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceFrame {
    /// 消费命令的仿真 tick。
    pub tick: u64,
    /// `encode_scheduled` 的十六进制字节串，保留实际顺序。
    pub commands: Vec<String>,
}

/// 可审计的单局比赛记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchRecord {
    /// 评测协议版本。
    pub protocol: String,
    /// 冻结合成规则的字节摘要。
    pub rules_hash: String,
    /// 开局参数。
    pub spec: MatchSpec,
    /// 从左出生位 NPC 视角给出的结果。
    pub score: Score,
    /// 结束原因。
    pub termination: Termination,
    /// 实际持续 tick。
    pub ticks: u64,
    /// 结束时引擎状态摘要（十六进制，避免 JSON 数字精度问题）。
    pub final_hash: String,
    /// 各出生位提交至引擎的命令数。
    pub submitted: [u64; 2],
    /// 各出生位被引擎拒绝的命令数（包括越权命令）。
    pub rejected: [u64; 2],
    /// 各出生位超预算被截断的命令数。
    pub dropped: [u64; 2],
    /// 回放轨迹；关闭 trace 时为 null。
    pub frames: Option<Vec<TraceFrame>>,
    /// 两席位的实测指标；旧报告为 null。
    #[serde(default)]
    pub metrics: Option<[PlayerMetrics; 2]>,
}

/// 循环赛报告。每次循环赛从相同起始分重新建立评测池。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// 分项耗时；未启用时省略，旧报告默认无计时。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profiling: Option<ArenaProfile>,
    /// 指标口径；旧报告无指标时为 null。
    #[serde(default)]
    pub metrics_version: Option<String>,
    /// 协议版本。
    pub protocol: String,
    /// 引擎 crate 版本。
    pub engine_version: String,
    /// CLI 写入的源码提交；库调用时为空。
    pub source_revision: Option<String>,
    /// CLI 写入的工作区是否含未提交改动；库调用时为空。
    pub source_dirty: Option<bool>,
    /// 冻结规则字节摘要。
    pub rules_hash: String,
    /// 完整赛程配置。
    pub config: ArenaConfig,
    /// 综合排名，降序。
    pub leaderboard: Vec<Rating>,
    /// 场景独立排名，防止微操与运营水平被综合分掩盖。
    pub by_scenario: BTreeMap<String, Vec<Rating>>,
    /// 累计统计与交手矩阵；旧报告可由 `statistics()` 从比赛记录重建。
    #[serde(default)]
    pub summary: Option<TournamentSummary>,
    /// 按赛程顺序保存的比赛结果；连续两局构成镜像双局。精简模式为空。
    pub matches: Vec<MatchRecord>,
}

/// 两个 NPC 的交手累计分；从行 NPC 的视角计分。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchupStats {
    /// 交手局数。
    pub games: u64,
    /// 胜局加半数和局的累计分。
    pub points: f64,
}

/// 与轮次数无关的报告摘要。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TournamentSummary {
    /// 实际完成的比赛数。
    pub games: u64,
    /// 所有比赛累计推进的仿真 tick。
    pub ticks: u64,
    /// 到达 tick 上限的比赛数。
    pub timeouts: u64,
    /// 实际提交的命令总数。
    pub submitted: u64,
    /// 引擎拒绝的命令总数。
    pub rejected: u64,
    /// 超过预算截断的命令总数。
    pub dropped: u64,
    /// 行 NPC 对列 NPC 的交手统计。
    pub head_to_head: BTreeMap<String, BTreeMap<String, MatchupStats>>,
    /// 每个 NPC 的遥测总量；旧报告缺少此字段时为空。
    #[serde(default)]
    pub metrics: BTreeMap<String, NpcMetrics>,
    /// 按场景区分经济与作战指标。
    #[serde(default)]
    pub metrics_by_scenario: BTreeMap<String, BTreeMap<String, NpcMetrics>>,
}

impl TournamentSummary {
    fn record(&mut self, record: &MatchRecord) {
        self.games += 1;
        self.ticks += record.ticks;
        self.timeouts += u64::from(record.termination == Termination::TickLimit);
        self.submitted += record.submitted.iter().sum::<u64>();
        self.rejected += record.rejected.iter().sum::<u64>();
        self.dropped += record.dropped.iter().sum::<u64>();
        for seat in 0..2 {
            let row = self.head_to_head.entry(record.spec.seats[seat].clone()).or_default();
            let matchup = row.entry(record.spec.seats[1 - seat].clone()).or_default();
            matchup.games += 1;
            matchup.points += if seat == 0 { record.score } else { record.score.reversed() }.points();
            if let Some(metrics) = &record.metrics {
                let id = &record.spec.seats[seat];
                self.metrics.entry(id.clone()).or_default().record(
                    &metrics[seat],
                    record.ticks,
                    record.submitted[seat],
                    record.rejected[seat],
                    record.dropped[seat],
                );
                self.metrics_by_scenario.entry(record.spec.scenario.id().into()).or_default().entry(id.clone()).or_default().record(
                    &metrics[seat],
                    record.ticks,
                    record.submitted[seat],
                    record.rejected[seat],
                    record.dropped[seat],
                );
            }
        }
    }
}

impl Report {
    /// 获取累计统计；兼容未保存摘要的旧版完整报告。
    pub fn statistics(&self) -> TournamentSummary {
        if let Some(summary) = &self.summary {
            return summary.clone();
        }
        let mut summary = TournamentSummary::default();
        for record in &self.matches {
            summary.record(record);
        }
        summary
    }
}

fn new_engine() -> ArenaResult<Engine> {
    Engine::new(Arc::new(ra_types::RuntimeDefinitions::default()), EngineConfig).map_err(|e| e.to_string())
}

fn players(battle: &BattleSession) -> ArenaResult<[PlayerId; 2]> {
    let mut ids = [PlayerId(0); 2];
    for (seat, house) in scenario::HOUSES.iter().enumerate() {
        ids[seat] = battle
            .world
            .players
            .iter()
            .find(|p| p.house.eq_ignore_ascii_case(house))
            .map(|p| p.id)
            .ok_or_else(|| format!("场景缺少玩家 {house}"))?;
    }
    Ok(ids)
}

fn alive(battle: &BattleSession, seat: usize) -> bool {
    battle.house_keeps_alive(scenario::HOUSES[seat])
}

fn finished(battle: &BattleSession, max_ticks: u64) -> Option<(Score, Termination)> {
    #[cfg(feature = "profiling")]
    let _timer = ra_engine::profiling::span(ra_engine::profiling::Stage::ArenaOutcome);
    if let Some(house) = battle.sole_victor() {
        return Some((if house.eq_ignore_ascii_case(scenario::HOUSES[0]) { Score::Win } else { Score::Loss }, Termination::Elimination));
    }
    if !alive(battle, 0) && !alive(battle, 1) {
        return Some((Score::Draw, Termination::MutualElimination));
    }
    (battle.world.tick >= max_ticks).then_some((Score::Draw, Termination::TickLimit))
}

/// 运行一局真实引擎比赛；NPC panic 返回错误，禁止当作正常平局计分。
pub fn play_match(registry: &BotRegistry, spec: MatchSpec) -> ArenaResult<MatchRecord> {
    let definitions = scenario::definitions(spec.scenario)?;
    play_match_prepared(registry, spec, &new_engine()?, &definitions)
}

fn play_match_prepared(
    registry: &BotRegistry,
    spec: MatchSpec,
    engine: &Engine,
    definitions: &Arc<ra_types::RuntimeDefinitions>,
) -> ArenaResult<MatchRecord> {
    #[cfg(feature = "profiling")]
    let setup_timer = ra_engine::profiling::span(ra_engine::profiling::Stage::MatchSetup);
    if spec.max_ticks == 0 || spec.max_commands_per_tick == 0 {
        return Err("单局 tick 与命令预算必须大于零".into());
    }
    let mut session = scenario::open(spec.scenario, spec.seed, Arc::clone(definitions))?;
    let ids = players(session.expect_battle())?;
    let mut telemetry = MatchTelemetry::new(&session.expect_battle().world, ids);
    let mut bots = [registry.create(&spec.seats[0])?, registry.create(&spec.seats[1])?];
    for seat in 0..2 {
        catch_unwind(AssertUnwindSafe(|| bots[seat].reset(spec.seed, ids[seat])))
            .map_err(|_| format!("NPC {} reset panic", spec.seats[seat]))?;
    }
    let mut submitted = [0; 2];
    let mut rejected = [0; 2];
    let mut dropped = [0; 2];
    let mut frames = spec.trace.then(Vec::new);
    #[cfg(feature = "profiling")]
    drop(setup_timer);
    let (score, termination) = loop {
        let battle = session.expect_battle();
        if let Some(result) = finished(battle, spec.max_ticks) {
            break result;
        }
        #[cfg(feature = "profiling")]
        ra_engine::profiling::begin_tick(battle.world.tick.wrapping_add(1));
        let mut commands = [Vec::new(), Vec::new()];
        for seat in 0..2 {
            #[cfg(feature = "profiling")]
            let npc_timer = ra_engine::profiling::span(if seat == 0 {
                ra_engine::profiling::Stage::NpcLeft
            }
            else {
                ra_engine::profiling::Stage::NpcRight
            });
            commands[seat] = catch_unwind(AssertUnwindSafe(|| bots[seat].commands(&BotContext { world: &battle.world, player: ids[seat] })))
                .map_err(|_| format!("NPC {} 在 tick {} panic", spec.seats[seat], battle.world.tick))?;
            #[cfg(feature = "profiling")]
            drop(npc_timer);
            dropped[seat] += commands[seat].len().saturating_sub(spec.max_commands_per_tick) as u64;
            commands[seat].truncate(spec.max_commands_per_tick);
            submitted[seat] += commands[seat].len() as u64;
        }
        // 每拍轮换入队顺序；两方决策完成前世界完全不变。
        let first = (battle.world.tick % 2) as usize;
        {
            #[cfg(feature = "profiling")]
            let _timer = ra_engine::profiling::span(ra_engine::profiling::Stage::CommandSubmit);
            for seat in [first, 1 - first] {
                for command in std::mem::take(&mut commands[seat]) {
                    session.expect_battle_mut().world.push_player_command(ids[seat], command);
                }
            }
        }
        session.tick(&engine.runtime());
        let world = &session.expect_battle().world;
        {
            #[cfg(feature = "profiling")]
            let _timer = ra_engine::profiling::span(ra_engine::profiling::Stage::Telemetry);
            telemetry.observe(world);
        }
        {
            #[cfg(feature = "profiling")]
            let _timer = ra_engine::profiling::span(ra_engine::profiling::Stage::CommandAudit);
            for reject in world.last_rejects() {
                let command = &world.last_input_frame().commands[reject.command_index];
                let seat = ids.iter().position(|id| *id == command.player).ok_or("出现未注册玩家的命令")?;
                rejected[seat] += 1;
            }
        }
        if let Some(frames) = &mut frames {
            #[cfg(feature = "profiling")]
            let _timer = ra_engine::profiling::span(ra_engine::profiling::Stage::TraceEncode);
            frames.push(TraceFrame {
                tick: world.tick,
                commands: world.last_input_frame().commands.iter().map(|cmd| hex(&encode_scheduled(cmd))).collect(),
            });
        }
    };
    #[cfg(feature = "profiling")]
    ra_engine::profiling::outside_tick();
    #[cfg(feature = "profiling")]
    let _timer = ra_engine::profiling::span(ra_engine::profiling::Stage::MatchFinish);
    let world = &session.expect_battle().world;
    Ok(MatchRecord {
        protocol: PROTOCOL.into(),
        rules_hash: rules_hash(scenario::rules(spec.scenario)),
        spec,
        score,
        termination,
        ticks: world.tick,
        final_hash: format!("{:016x}", world.state_hash()),
        submitted,
        rejected,
        dropped,
        frames,
        metrics: Some(telemetry.finish(session.expect_battle())),
    })
}

/// 无需 NPC 实现，通过命令轨迹重放并核对终局哈希、结束原因、胜负和实际命令。
pub fn replay_match(record: &MatchRecord) -> ArenaResult<()> {
    if record.protocol != PROTOCOL || record.rules_hash != rules_hash(scenario::rules(record.spec.scenario)) {
        return Err("回放协议或规则版本不匹配".into());
    }
    let frames = record.frames.as_ref().ok_or("此记录未开启 trace，不能回放")?;
    if frames.len() as u64 != record.ticks || record.ticks == 0 || record.ticks > record.spec.max_ticks {
        return Err("回放 tick 数与记录不一致".into());
    }
    let engine = new_engine()?;
    let mut session = scenario::open(record.spec.scenario, record.spec.seed, scenario::definitions(record.spec.scenario)?)?;
    let ids = players(session.expect_battle())?;
    let mut telemetry = MatchTelemetry::new(&session.expect_battle().world, ids);
    let mut submitted = [0; 2];
    let mut rejected = [0; 2];
    for frame in frames {
        let world = &mut session.expect_battle_mut().world;
        if frame.tick != world.tick + 1 {
            return Err("回放帧必须从 tick 1 连续排列".into());
        }
        for bytes in &frame.commands {
            let bytes = unhex(bytes)?;
            let cmd = decode_scheduled(&bytes).ok_or("回放命令无法解码")?;
            let seat = ids.iter().position(|id| *id == cmd.player).ok_or("回放含未知玩家")?;
            submitted[seat] += 1;
            if cmd.tick.0 != frame.tick || encode_scheduled(&cmd) != bytes {
                return Err("回放命令信封 tick 或字节不合法".into());
            }
            world.push_player_command(cmd.player, cmd.body);
        }
        session.tick(&engine.runtime());
        let battle = session.expect_battle();
        telemetry.observe(&battle.world);
        let actual: Vec<_> = battle.world.last_input_frame().commands.iter().map(|cmd| hex(&encode_scheduled(cmd))).collect();
        if battle.world.tick != frame.tick || actual != frame.commands {
            return Err(format!("回放在 tick {} 出现命令或时钟差异", frame.tick));
        }
        for reject in battle.world.last_rejects() {
            let player = battle.world.last_input_frame().commands[reject.command_index].player;
            rejected[ids.iter().position(|id| *id == player).unwrap()] += 1;
        }
        if frame.tick < record.ticks && finished(battle, record.spec.max_ticks).is_some() {
            return Err("回放在记录结束前提前终局".into());
        }
    }
    let battle = session.expect_battle();
    if record.metrics.as_ref().is_some_and(|metrics| *metrics != telemetry.finish(battle)) {
        return Err("回放遥测与记录不一致".into());
    }
    if format!("{:016x}", battle.world.state_hash()) != record.final_hash
        || finished(battle, record.spec.max_ticks) != Some((record.score, record.termination))
        || submitted != record.submitted
        || rejected != record.rejected
    {
        return Err(format!(
            "回放终局不一致: hash={:016x}/{} result={:?}/{:?} submitted={:?}/{:?} rejected={:?}/{:?}",
            battle.world.state_hash(),
            record.final_hash,
            finished(battle, record.spec.max_ticks),
            (record.score, record.termination),
            submitted,
            record.submitted,
            rejected,
            record.rejected
        ));
    }
    Ok(())
}

/// 完整镜像循环赛；赛程用固定种子洗牌，双方每个场景的比赛数量完全相同。
pub fn run_tournament(registry: &BotRegistry, config: ArenaConfig) -> ArenaResult<Report> {
    config.validate(registry)?;
    let profile_started = config.profile.then(std::time::Instant::now);
    let profile_every = config.profile.then_some(config.profile_sample_every);
    let mut profiling = config.profile.then(|| ArenaProfile::new(config.profile_sample_every));
    // 冻结规则与只读引擎仅构建一次；每局仍创建独立世界与 NPC。
    let definitions =
        config.scenarios.iter().map(|scene| Ok((*scene, scenario::definitions(*scene)?))).collect::<ArenaResult<BTreeMap<_, _>>>()?;
    let engine = new_engine()?;
    let mut overall = EloTable::new(config.elo.clone(), &config.npcs)?;
    let mut per_scene = BTreeMap::new();
    for scene in &config.scenarios {
        per_scene.insert(scene.id().to_string(), EloTable::new(config.elo.clone(), &config.npcs)?);
    }
    let mut random = Random(config.seed);
    let mut matches = Vec::new();
    let mut summary = TournamentSummary::default();
    let workers = config.jobs.min((config.match_count()? / u64::from(config.rounds) / 2) as usize);
    thread::scope(|scope| -> ArenaResult<()> {
        let (result_tx, result_rx) = mpsc::sync_channel(workers);
        let mut senders = Vec::new();
        if workers > 1 {
            for _ in 0..workers {
                let (task_tx, task_rx) = mpsc::sync_channel::<(usize, [MatchSpec; 2])>(1);
                senders.push(task_tx);
                let result_tx = result_tx.clone();
                let engine = &engine;
                let definitions = &definitions;
                thread::Builder::new()
                    .name("ra-arena".into())
                    .spawn_scoped(scope, move || {
                        while let Ok((index, specs)) = task_rx.recv() {
                            let result = play_series(registry, specs, engine, definitions, profile_every);
                            if result_tx.send((index, result)).is_err() {
                                break;
                            }
                        }
                    })
                    .map_err(|e| format!("无法创建测评线程: {e}"))?;
            }
        }
        drop(result_tx);
        for _ in 0..config.rounds {
            let mut schedule = Vec::new();
            for a in 0..config.npcs.len() {
                for b in a + 1..config.npcs.len() {
                    for scene in &config.scenarios {
                        schedule.push((a, b, *scene));
                    }
                }
            }
            for i in (1..schedule.len()).rev() {
                let j = (random.next() % (i as u64 + 1)) as usize;
                schedule.swap(i, j);
            }
            // 每批最多 workers 个双局；不积攒全部任务或乱序完成的结果。
            for batch in schedule.chunks(workers) {
                let mut results: Vec<Option<ArenaResult<PlayedSeries>>> = (0..batch.len()).map(|_| None).collect();
                for (index, &(a, b, scene)) in batch.iter().enumerate() {
                    let seed = random.next();
                    let first = MatchSpec {
                        scenario: scene,
                        seed,
                        seats: [config.npcs[a].clone(), config.npcs[b].clone()],
                        max_ticks: config.max_ticks,
                        max_commands_per_tick: config.max_commands_per_tick,
                        trace: config.trace,
                    };
                    let second = MatchSpec { seats: [first.seats[1].clone(), first.seats[0].clone()], ..first.clone() };
                    if workers == 1 {
                        results[index] = Some(play_series(registry, [first, second], &engine, &definitions, profile_every));
                    }
                    else {
                        senders[index].send((index, [first, second])).map_err(|_| "测评线程已退出")?;
                    }
                }
                if workers > 1 {
                    for _ in 0..batch.len() {
                        let (index, result) = result_rx.recv().map_err(|_| "测评结果通道已关闭")?;
                        results[index] = Some(result);
                    }
                }
                // Elo 按原赛程顺序结算，避免线程完成顺序改变分数。
                for (&(a, b, scene), result) in batch.iter().zip(results) {
                    let series = result.ok_or("缺少测评结果")??;
                    #[cfg(feature = "profiling")]
                    if let (Some(profiling), Some(profiles)) = (&mut profiling, series.profiles) {
                        for (record, data) in series.records.iter().zip(profiles) {
                            profiling.record(record.spec.scenario.id(), &record.spec.seats, data);
                        }
                    }
                    let [first, second] = series.records;
                    let scores = [first.score, second.score.reversed()];
                    overall.rate_series(&config.npcs[a], &config.npcs[b], &scores)?;
                    per_scene.get_mut(scene.id()).unwrap().rate_series(&config.npcs[a], &config.npcs[b], &scores)?;
                    summary.record(&first);
                    summary.record(&second);
                    if !config.summary_only {
                        matches.extend([first, second]);
                    }
                }
            }
        }
        Ok(())
    })?;
    if let (Some(profiling), Some(started)) = (&mut profiling, profile_started) {
        profiling.tournament_wall_ns = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    }
    Ok(Report {
        profiling,
        metrics_version: Some(METRICS_VERSION.into()),
        protocol: PROTOCOL.into(),
        engine_version: env!("CARGO_PKG_VERSION").into(),
        source_revision: None,
        source_dirty: None,
        rules_hash: rules_hash(&[scenario::RULES, scenario::ECONOMY_RULES].concat()),
        leaderboard: overall.leaderboard(),
        by_scenario: per_scene.into_iter().map(|(id, table)| (id, table.leaderboard())).collect(),
        summary: Some(summary),
        config,
        matches,
    })
}

struct PlayedSeries {
    records: [MatchRecord; 2],
    #[cfg(feature = "profiling")]
    profiles: Option<[ra_engine::profiling::ProfileData; 2]>,
}

fn play_series(
    registry: &BotRegistry,
    specs: [MatchSpec; 2],
    engine: &Engine,
    definitions: &BTreeMap<Scenario, Arc<ra_types::RuntimeDefinitions>>,
    _profile_every: Option<u64>,
) -> ArenaResult<PlayedSeries> {
    catch_unwind(AssertUnwindSafe(|| {
        let [first, second] = specs;
        let definitions = &definitions[&first.scenario];
        #[cfg(feature = "profiling")]
        let first_capture = _profile_every.map(ra_engine::profiling::Capture::start).transpose().map_err(str::to_string)?;
        let first = play_match_prepared(registry, first, engine, definitions)?;
        #[cfg(feature = "profiling")]
        let first_profile = first_capture.map(ra_engine::profiling::Capture::finish);
        #[cfg(feature = "profiling")]
        let second_capture = _profile_every.map(ra_engine::profiling::Capture::start).transpose().map_err(str::to_string)?;
        let second = play_match_prepared(registry, second, engine, definitions)?;
        #[cfg(feature = "profiling")]
        let second_profile = second_capture.map(ra_engine::profiling::Capture::finish);
        Ok(PlayedSeries {
            records: [first, second],
            #[cfg(feature = "profiling")]
            profiles: first_profile.zip(second_profile).map(|(a, b)| [a, b]),
        })
    }))
    .map_err(|_| "测评执行 panic（包括 NPC 工厂），本次评测失败".to_string())?
}

// SplitMix64；只用于赛程和种子生成，不依赖平台随机源。
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
}

fn rules_hash(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
    format!("{hash:016x}")
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").unwrap();
    }
    result
}

fn unhex(value: &str) -> ArenaResult<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("无效回放十六进制字节".into());
    }
    (0..value.len()).step_by(2).map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|e| e.to_string())).collect()
}
