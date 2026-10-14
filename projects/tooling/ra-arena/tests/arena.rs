use ra_arena::{
    ArenaConfig, BotContext, BotRegistry, EloConfig, EloTable, HeuristicBot, IdleBot, MatchSpec, Npc, Scenario, Score, Termination, play_match,
    replay_match, run_tournament,
};
use ra_engine::GameCommand;

fn spec(scene: Scenario, a: &str, b: &str) -> MatchSpec {
    MatchSpec { scenario: scene, seed: 42, seats: [a.into(), b.into()], max_ticks: 1500, max_commands_per_tick: 64, trace: true }
}

#[test]
fn elo_equal_rating_win_and_zero_sum() {
    let mut table = EloTable::new(EloConfig::default(), &["a".into(), "b".into()]).unwrap();
    table.rate_series("a", "b", &[Score::Win, Score::Win]).unwrap();
    let rows = table.leaderboard();
    assert_eq!(rows[0].elo, 1516.0);
    assert_eq!(rows[1].elo, 1484.0);
    assert_eq!(rows[0].wins, 2);
    assert_eq!(rows[1].losses, 2);
    assert_eq!(rows.iter().map(|row| row.elo).sum::<f64>(), 3000.0);
}

#[test]
fn mirrored_results_have_no_update_order_bias() {
    let mut first = EloTable::new(EloConfig::default(), &["a".into(), "b".into()]).unwrap();
    let mut second = EloTable::new(EloConfig::default(), &["a".into(), "b".into()]).unwrap();
    first.rate_series("a", "b", &[Score::Win, Score::Loss]).unwrap();
    second.rate_series("a", "b", &[Score::Loss, Score::Win]).unwrap();
    assert_eq!(first.leaderboard(), second.leaderboard());
    assert!(first.leaderboard().iter().all(|row| row.elo == 1500.0));
}

#[test]
fn unexpected_win_is_worth_more_and_draw_moves_toward_equality() {
    let config = EloConfig::default();
    assert!((config.expected(1900.0, 1500.0) - 10.0 / 11.0).abs() < 1e-12);
    assert!((config.expected(1500.0, 1900.0) - 1.0 / 11.0).abs() < 1e-12);
    let mut table = EloTable::new(config, &["a".into(), "b".into()]).unwrap();
    table.rate_series("a", "b", &[Score::Win]).unwrap();
    table.rate_series("a", "b", &[Score::Draw]).unwrap();
    assert!(table.leaderboard()[0].elo < 1516.0);
    assert!(table.leaderboard()[0].elo > 1500.0);
}

#[test]
fn elo_bad_input_does_not_mutate_table() {
    let mut table = EloTable::new(EloConfig::default(), &["a".into(), "b".into()]).unwrap();
    let before = table.leaderboard();
    assert!(table.rate_series("a", "a", &[Score::Win]).is_err());
    assert!(table.rate_series("a", "missing", &[Score::Win]).is_err());
    assert!(table.rate_series("a", "b", &[]).is_err());
    assert_eq!(before, table.leaderboard());
    for bad in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
        assert!(EloConfig { k: bad, ..EloConfig::default() }.validate().is_err());
        assert!(EloConfig { scale: bad, ..EloConfig::default() }.validate().is_err());
    }
}

#[test]
fn idle_skirmish_times_out_without_hidden_builtin_ai() {
    let registry = BotRegistry::builtins();
    let record = play_match(&registry, MatchSpec { max_ticks: 200, ..spec(Scenario::Skirmish, "idle-v1", "idle-v1") }).unwrap();
    assert_eq!(record.termination, Termination::TickLimit);
    assert_eq!(record.score, Score::Draw);
    assert_eq!(record.ticks, 200);
    assert!(record.frames.as_ref().unwrap().iter().all(|frame| frame.commands.is_empty()));
    replay_match(&record).unwrap();
}

#[test]
fn actual_winner_is_correct_even_when_local_player_loses() {
    let registry = BotRegistry::builtins();
    for scene in [Scenario::Duel, Scenario::Skirmish] {
        let right_wins = play_match(&registry, spec(scene, "idle-v1", "hard-v1")).unwrap();
        assert_eq!(right_wins.score, Score::Loss, "{scene:?}");
        assert_eq!(right_wins.termination, Termination::Elimination);
        replay_match(&right_wins).unwrap();
        let left_wins = play_match(&registry, spec(scene, "hard-v1", "idle-v1")).unwrap();
        assert_eq!(left_wins.score, Score::Win, "{scene:?}");
        replay_match(&left_wins).unwrap();
    }
}

#[test]
fn matches_are_repeatable_across_new_worlds_and_serialization() {
    let registry = BotRegistry::builtins();
    for scene in [Scenario::Duel, Scenario::Skirmish] {
        let first = play_match(&registry, spec(scene, "normal-v1", "hard-v1")).unwrap();
        let second = play_match(&registry, spec(scene, "normal-v1", "hard-v1")).unwrap();
        assert_eq!(first, second, "{scene:?}");
        let saved = serde_json::to_vec(&first).unwrap();
        let loaded = serde_json::from_slice(&saved).unwrap();
        replay_match(&loaded).unwrap();
    }
}

#[test]
fn replay_detects_result_hash_and_command_tampering() {
    let registry = BotRegistry::builtins();
    let first = play_match(&registry, spec(Scenario::Duel, "hard-v1", "idle-v1")).unwrap();
    let mut modified = first.clone();
    modified.final_hash = "0000000000000000".into();
    assert!(replay_match(&modified).is_err());
    let mut modified = first.clone();
    modified.score = modified.score.reversed();
    assert!(replay_match(&modified).is_err());
    let mut modified = first.clone();
    modified.frames.as_mut().unwrap()[0].commands[0].push_str("00");
    assert!(replay_match(&modified).is_err());
    let mut modified = first;
    modified.frames = None;
    assert!(replay_match(&modified).is_err());
}

struct Cheater;
impl Npc for Cheater {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        let enemy = context.world.players.iter().find(|p| p.id != context.player).unwrap();
        let entity = context.world.find_entity_id_by_owner_type(enemy.house.as_ref(), "AMCV").unwrap();
        let type_id = context.world.definitions.techno.get("GAPOWR").unwrap().id;
        vec![GameCommand::Deploy { entity }, GameCommand::Produce { player: enemy.id, type_id }]
    }
}

#[test]
fn npc_cannot_issue_orders_or_spend_funds_as_opponent() {
    let mut registry = BotRegistry::builtins();
    registry.register("cheater", || Box::new(Cheater)).unwrap();
    let record = play_match(&registry, MatchSpec { max_ticks: 1, ..spec(Scenario::Skirmish, "cheater", "idle-v1") }).unwrap();
    assert_eq!(record.submitted, [2, 0]);
    assert_eq!(record.rejected, [2, 0]);
    assert_eq!(record.score, Score::Draw);
    replay_match(&record).unwrap();
}

struct Spam;
impl Npc for Spam {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        let house = &context.world.players.iter().find(|p| p.id == context.player).unwrap().house;
        let entity = context.world.find_entity_id_by_owner_type(house.as_ref(), "AMCV").unwrap();
        vec![GameCommand::Stop { entity }; 100]
    }
}

#[test]
fn commands_are_capped_equally_and_dropped_count_is_visible() {
    let mut registry = BotRegistry::builtins();
    registry.register("spam", || Box::new(Spam)).unwrap();
    let record =
        play_match(&registry, MatchSpec { max_ticks: 1, max_commands_per_tick: 3, ..spec(Scenario::Skirmish, "spam", "spam") }).unwrap();
    assert_eq!(record.submitted, [3, 3]);
    assert_eq!(record.dropped, [97, 97]);
    assert_eq!(record.rejected, [0, 0]);
    replay_match(&record).unwrap();
}

struct Broken;
impl Npc for Broken {
    fn commands(&mut self, _: &BotContext<'_>) -> Vec<GameCommand> {
        panic!("broken npc")
    }
}

#[test]
fn npc_failure_is_an_error_and_never_a_draw() {
    let mut registry = BotRegistry::builtins();
    registry.register("broken", || Box::new(Broken)).unwrap();
    let error = play_match(&registry, spec(Scenario::Duel, "broken", "idle-v1")).unwrap_err();
    assert!(error.contains("panic"));
    assert!(error.contains("broken"));
}

#[test]
fn mirrored_round_robin_is_balanced_and_identical_bots_stay_equal() {
    let mut registry = BotRegistry::default();
    for id in ["clone-a", "clone-b", "clone-c"] {
        registry.register(id, || Box::new(HeuristicBot::new("Hard"))).unwrap();
    }
    let config = ArenaConfig { npcs: registry.ids(), rounds: 2, max_ticks: 300, trace: true, ..ArenaConfig::default() };
    let report = run_tournament(&registry, config.clone()).unwrap();
    assert_eq!(report.matches.len(), 24);
    assert!(report.leaderboard.iter().all(|row| row.games() == 16 && row.elo == 1500.0));
    for games in report.matches.as_chunks::<2>().0 {
        assert_eq!(games[0].spec.seed, games[1].spec.seed);
        assert_eq!(games[0].spec.scenario, games[1].spec.scenario);
        assert_eq!(games[0].spec.seats[0], games[1].spec.seats[1]);
        assert_eq!(games[0].spec.seats[1], games[1].spec.seats[0]);
    }
    let second = run_tournament(&registry, config).unwrap();
    for (i, (a, b)) in report.matches.iter().zip(&second.matches).enumerate() {
        assert_eq!(a.spec, b.spec, "match {i}");
        assert_eq!(a.final_hash, b.final_hash, "match {i}");
        assert_eq!(a.frames, b.frames, "match {i}");
    }
    assert_eq!(report.leaderboard, second.leaderboard);
    assert!(report.by_scenario.values().all(|rows| rows.iter().all(|row| row.games() == 8)));
}

#[test]
fn invalid_configs_are_rejected_before_running() {
    let registry = BotRegistry::builtins();
    let mut config = ArenaConfig::default();
    config.npcs[0] = "missing".into();
    assert!(config.validate(&registry).is_err());
    config = ArenaConfig::default();
    config.npcs[1] = config.npcs[0].clone();
    assert!(config.validate(&registry).is_err());
    config = ArenaConfig::default();
    config.scenarios.push(Scenario::Duel);
    assert!(config.validate(&registry).is_err());
    config = ArenaConfig::default();
    config.max_ticks = 0;
    assert!(config.validate(&registry).is_err());
    assert!(Scenario::parse("missing").is_err());
    assert!(registry.create("missing").is_err());
    let mut custom = BotRegistry::default();
    custom.register("bot-v1", || Box::new(IdleBot)).unwrap();
    assert!(custom.register("bot-v1", || Box::new(IdleBot)).is_err());
}

#[test]
fn skirmish_ai_disable_switch_is_respected_and_can_be_reenabled() {
    let mut case = ra_testing::ai_skirmish_open();
    case.session.expect_battle_mut().ai_enabled = false;
    case.advance(5);
    assert!(case.session.expect_battle().world.find_entity_id_by_owner_type("Russians", "SMCV").is_some());
    assert!(case.session.expect_battle().world.find_entity_id_by_owner_type("Russians", "NACNST").is_none());
    case.session.expect_battle_mut().ai_enabled = true;
    case.advance(1);
    assert!(case.session.expect_battle().world.find_entity_id_by_owner_type("Russians", "NACNST").is_some());
}

#[test]
fn shared_ai_entrypoint_preserves_the_builtin_driver_commands() {
    use ra_testing::HeadlessCase;
    let mut automatic = ra_testing::ai_skirmish_open();
    let mut manual = HeadlessCase::new(ra_engine::Session::from_state(automatic.session.expect_battle().world.clone(), "manual AI"));
    let opponent = manual.session.expect_battle().world.players.iter().find(|p| p.house.eq_ignore_ascii_case("Russians")).unwrap().id;
    for _ in 0..150 {
        let commands = ra_engine::heuristic_ai_commands(&manual.session.expect_battle().world, opponent, "Normal");
        for command in commands {
            manual.session.expect_battle_mut().world.push_player_command(opponent, command);
        }
        automatic.advance(1);
        manual.advance(1);
        assert_eq!(automatic.session.expect_battle().world.last_input_frame(), manual.session.expect_battle().world.last_input_frame());
        assert_eq!(automatic.observe().state_hash, manual.observe().state_hash);
    }
}

#[test]
fn parallel_matches_preserve_schedule_traces_hashes_and_elo() {
    let registry = BotRegistry::builtins();
    let config = ArenaConfig {
        rounds: 2,
        max_ticks: 300,
        trace: true,
        scenarios: vec![Scenario::Duel, Scenario::Skirmish, Scenario::Economy],
        ..ArenaConfig::default()
    };
    let serial = run_tournament(&registry, config.clone()).unwrap();
    for jobs in [2, 64] {
        let parallel = run_tournament(&registry, ArenaConfig { jobs, ..config.clone() }).unwrap();
        assert_eq!(serial.matches, parallel.matches, "jobs={jobs}");
        assert_eq!(serial.leaderboard, parallel.leaderboard);
        assert_eq!(serial.by_scenario, parallel.by_scenario);
        assert_eq!(serial.statistics(), parallel.statistics());
        // 并发记录仍可由引擎命令路径回放。
        replay_match(&parallel.matches[0]).unwrap();
    }
}

#[test]
fn summary_retains_scores_matchups_and_command_totals_without_records() {
    let registry = BotRegistry::builtins();
    let config =
        ArenaConfig { rounds: 2, max_ticks: 400, jobs: 3, scenarios: vec![Scenario::Duel, Scenario::Economy], ..ArenaConfig::default() };
    let full = run_tournament(&registry, config.clone()).unwrap();
    let summary = run_tournament(&registry, ArenaConfig { summary_only: true, ..config.clone() }).unwrap();
    assert!(summary.matches.is_empty());
    assert_eq!(summary.leaderboard, full.leaderboard);
    assert_eq!(summary.by_scenario, full.by_scenario);
    assert_eq!(summary.statistics(), full.statistics());
    assert_eq!(summary.statistics().games, config.match_count().unwrap());
    let stats = summary.statistics();
    assert_eq!(stats.ticks, full.matches.iter().map(|m| m.ticks).sum::<u64>());
    assert_eq!(stats.submitted, full.matches.iter().flat_map(|m| m.submitted).sum::<u64>());
    for a in &config.npcs {
        for b in &config.npcs {
            let scores: Vec<_> = full
                .matches
                .iter()
                .filter_map(|m| {
                    if &m.spec.seats[0] == a && &m.spec.seats[1] == b {
                        Some(m.score.points())
                    }
                    else if &m.spec.seats[1] == a && &m.spec.seats[0] == b {
                        Some(m.score.reversed().points())
                    }
                    else {
                        None
                    }
                })
                .collect();
            if a == b {
                assert!(!stats.head_to_head[a].contains_key(b));
            }
            else {
                assert_eq!(stats.head_to_head[a][b].games, scores.len() as u64);
                assert_eq!(stats.head_to_head[a][b].points, scores.iter().sum::<f64>());
            }
        }
    }
    let loaded: ra_arena::Report = serde_json::from_slice(&serde_json::to_vec(&summary).unwrap()).unwrap();
    assert!(loaded.matches.is_empty());
    assert_eq!(loaded.statistics(), stats);
}

#[test]
fn old_reports_and_configs_remain_readable() {
    let registry = BotRegistry::builtins();
    let report = run_tournament(&registry, ArenaConfig { rounds: 1, max_ticks: 2, ..ArenaConfig::default() }).unwrap();
    let mut json = serde_json::to_value(&report).unwrap();
    json.as_object_mut().unwrap().remove("summary");
    json.as_object_mut().unwrap().remove("profiling");
    let config = json["config"].as_object_mut().unwrap();
    config.remove("jobs");
    config.remove("summary_only");
    config.remove("profile");
    config.remove("profile_sample_every");
    let loaded: ra_arena::Report = serde_json::from_value(json).unwrap();
    assert_eq!(loaded.config.jobs, 1);
    assert!(!loaded.config.summary_only);
    assert!(!loaded.config.profile);
    assert_eq!(loaded.config.profile_sample_every, 1);
    assert!(loaded.profiling.is_none());
    assert_eq!(loaded.statistics(), report.statistics());
}

#[test]
fn invalid_resource_options_and_parallel_npc_failures_are_errors() {
    let mut registry = BotRegistry::builtins();
    assert!(ArenaConfig { jobs: 0, ..ArenaConfig::default() }.validate(&registry).is_err());
    assert!(ArenaConfig { trace: true, summary_only: true, ..ArenaConfig::default() }.validate(&registry).is_err());
    registry.register("broken", || Box::new(Broken)).unwrap();
    registry.register("factory-panic", || panic!("factory failed")).unwrap();
    for id in ["broken", "factory-panic"] {
        let error = run_tournament(
            &registry,
            ArenaConfig {
                npcs: vec![id.into(), "idle-v1".into(), "hard-v1".into()],
                rounds: 1,
                jobs: 2,
                max_ticks: 2,
                ..ArenaConfig::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("panic"));
    }
}

#[test]
fn worker_count_bounds_live_npc_instances() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Tracked {
        live: Arc<AtomicUsize>,
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl Npc for Tracked {
        fn commands(&mut self, _: &BotContext<'_>) -> Vec<GameCommand> {
            std::thread::sleep(std::time::Duration::from_millis(2));
            Vec::new()
        }
    }
    let live = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut registry = BotRegistry::default();
    for id in ["a", "b", "c", "d"] {
        let live = Arc::clone(&live);
        let peak = Arc::clone(&peak);
        registry
            .register(id, move || {
                let count = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                Box::new(Tracked { live: Arc::clone(&live) })
            })
            .unwrap();
    }
    let report = run_tournament(
        &registry,
        ArenaConfig {
            npcs: registry.ids(),
            scenarios: vec![Scenario::Duel],
            rounds: 2,
            max_ticks: 1,
            jobs: 2,
            summary_only: true,
            ..ArenaConfig::default()
        },
    )
    .unwrap();
    assert_eq!(report.statistics().games, 24);
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert!(peak.load(Ordering::SeqCst) > 2, "比赛应实际并发");
    assert!(peak.load(Ordering::SeqCst) <= 4, "两名 worker 每个最多两个 NPC");
}

#[test]
fn frozen_hard_copy_preserves_original_strategy_commands() {
    let mut case = ra_testing::ai_skirmish_open();
    case.session.expect_battle_mut().ai_enabled = false;
    let player = case.session.expect_battle().world.players.iter().find(|p| p.house.eq_ignore_ascii_case("Russians")).unwrap().id;
    for _ in 0..250 {
        let world = &case.session.expect_battle().world;
        let frozen = ra_engine::hard_baseline_v1_commands(world, player);
        assert_eq!(frozen, ra_engine::heuristic_ai_commands(world, player, "Hard"));
        for command in frozen {
            case.session.expect_battle_mut().world.push_player_command(player, command);
        }
        case.advance(1);
    }
}

#[test]
fn economy_measures_real_harvesting_even_while_spending_and_replays_metrics() {
    let record = play_match(&BotRegistry::builtins(), spec(Scenario::Economy, "hard-baseline-v1", "idle-v1")).unwrap();
    let metrics = record.metrics.as_ref().unwrap();
    let hard = &metrics[0];
    let idle = &metrics[1];
    assert!(hard.harvested_credits > 0);
    assert!(hard.spent > 0);
    assert_eq!(i64::from(hard.final_funds) + i64::from(hard.spent) - i64::from(hard.initial_funds), hard.harvested_credits as i64);
    assert!(hard.first_refinery_tick.unwrap() < hard.first_income_tick.unwrap());
    assert_eq!(hard.opening_ticks, record.ticks.min(900));
    assert_eq!(idle.harvested_credits, 0);
    assert_eq!(idle.first_income_tick, None);
    assert_eq!(idle.first_refinery_tick, None);
    replay_match(&record).unwrap();
    let mut tampered = record;
    tampered.metrics.as_mut().unwrap()[0].harvested_credits += 1;
    assert!(replay_match(&tampered).unwrap_err().contains("遥测"));
}

#[test]
fn telemetry_aggregation_weights_duration_and_keeps_legacy_unknown() {
    let config = ArenaConfig {
        npcs: vec!["hard-baseline-v1".into(), "idle-v1".into()],
        scenarios: vec![Scenario::Economy],
        rounds: 2,
        max_ticks: 1200,
        ..ArenaConfig::default()
    };
    let report = run_tournament(&BotRegistry::builtins(), config).unwrap();
    let stats = report.statistics();
    let hard = &stats.metrics["hard-baseline-v1"];
    let mut credits = 0;
    let mut ticks = 0;
    let mut score = 0;
    for record in &report.matches {
        let seat = record.spec.seats.iter().position(|id| id == "hard-baseline-v1").unwrap();
        let m = &record.metrics.as_ref().unwrap()[seat];
        credits += m.harvested_credits;
        ticks += record.ticks;
        score += i64::from(m.battle_score);
    }
    assert_eq!(hard.harvested_credits, credits);
    assert_eq!(hard.ticks, ticks);
    assert_eq!(hard.battle_score, score);
    assert_eq!(hard.income_per_minute(), credits as f64 * 900.0 / ticks as f64);
    assert_eq!(hard.average_battle_score(), score as f64 / 4.0);
    assert_eq!(stats.metrics_by_scenario["economy-v1"]["hard-baseline-v1"], *hard);
    let mut legacy = serde_json::to_value(&report).unwrap();
    legacy.as_object_mut().unwrap().remove("summary");
    legacy.as_object_mut().unwrap().remove("metrics_version");
    for record in legacy["matches"].as_array_mut().unwrap() {
        record.as_object_mut().unwrap().remove("metrics");
    }
    let loaded: ra_arena::Report = serde_json::from_value(legacy).unwrap();
    assert!(loaded.metrics_version.is_none());
    assert!(loaded.statistics().metrics.is_empty());
    assert_eq!(loaded.leaderboard, report.leaderboard);
}

#[test]
fn engine_optimizations_preserve_frozen_match_results() {
    // 由底层优化之前的二进制生成；规则/玩法有意改变时才应更新。
    let expected: Vec<ra_arena::MatchRecord> = serde_json::from_str(include_str!("../fixtures/golden-matches-v1.json")).unwrap();
    let registry = BotRegistry::builtins();
    assert_eq!(expected.len(), 24);
    for record in expected {
        let mut actual = play_match(&registry, record.spec.clone()).unwrap();
        // 旧金样只比较游戏行为；新增的观测字段不更改历史基准。
        if record.metrics.is_none() {
            actual.metrics = None;
        }
        assert_eq!(actual, record, "{} {:?} seed={}", record.spec.scenario.id(), record.spec.seats, record.spec.seed);
    }
}

#[test]
fn invalid_profiling_options_fail_before_the_tournament() {
    let registry = BotRegistry::builtins();
    assert!(ArenaConfig { profile_sample_every: 0, ..ArenaConfig::default() }.validate(&registry).is_err());
    #[cfg(not(feature = "profiling"))]
    assert!(ArenaConfig { profile: true, ..ArenaConfig::default() }.validate(&registry).unwrap_err().contains("--features profiling"));
}

#[cfg(feature = "profiling")]
#[test]
fn profiling_preserves_records_and_aggregates_sampled_ticks_across_workers() {
    let registry = BotRegistry::builtins();
    let config = ArenaConfig { rounds: 1, max_ticks: 7, trace: true, ..ArenaConfig::default() };
    let plain = run_tournament(&registry, config.clone()).unwrap();
    assert!(plain.profiling.is_none());
    for jobs in [1, 3] {
        let profiled = run_tournament(&registry, ArenaConfig { profile: true, profile_sample_every: 3, jobs, ..config.clone() }).unwrap();
        assert_eq!(profiled.matches, plain.matches);
        assert_eq!(profiled.leaderboard, plain.leaderboard);
        assert_eq!(profiled.by_scenario, plain.by_scenario);
        assert_eq!(profiled.statistics(), plain.statistics());
        let profile = profiled.profiling.as_ref().unwrap();
        let measured = plain.matches.iter().map(|m| m.ticks.div_ceil(3)).sum::<u64>();
        assert_eq!(profile.overall.games, plain.matches.len() as u64);
        assert_eq!(profile.overall.observed_ticks, plain.statistics().ticks);
        assert_eq!(profile.overall.sampled_ticks, measured);
        assert_eq!(profile.overall.timings["apply_commands"].calls, measured);
        assert_eq!(profile.overall.timings["ecs_full_sync"].calls, measured);
        assert_eq!(profile.overall.timings["trace_encode"].calls, measured);
        assert_eq!(profile.overall.timings["npc_decision"].calls, 2 * measured);
        assert_eq!(profile.npc_decisions.values().map(|row| row.calls).sum::<u64>(), 2 * measured);
        let total_own = profile.overall.timings.values().map(|row| u128::from(row.self_ns)).sum::<u128>();
        assert!(total_own <= u128::from(profile.overall.match_wall_ns));
        for row in profile.overall.timings.values() {
            assert_eq!(row.self_histogram.iter().sum::<u64>(), row.calls);
            assert!(row.self_ns <= row.inclusive_ns);
        }
        for (scene, group) in &profile.by_scenario {
            assert_eq!(group.games, plain.matches.iter().filter(|m| m.spec.scenario.id() == scene).count() as u64);
        }
        let loaded: ra_arena::Report = serde_json::from_slice(&serde_json::to_vec(&profiled).unwrap()).unwrap();
        assert_eq!(loaded.profiling.as_ref().unwrap().overall.timings, profile.overall.timings);
        assert!(profile.csv().contains("scenario,skirmish-v1,"));
        assert!(profile.markdown().contains("P95"));
        replay_match(&profiled.matches[0]).unwrap();
    }
    let summary = run_tournament(
        &registry,
        ArenaConfig { profile: true, summary_only: true, rounds: 1, max_ticks: 3, jobs: 2, ..ArenaConfig::default() },
    )
    .unwrap();
    assert!(summary.matches.is_empty());
    assert_eq!(summary.profiling.unwrap().overall.games, summary.config.match_count().unwrap());
}

#[cfg(feature = "profiling")]
#[test]
fn npc_prediction_ticks_are_not_counted_as_authoritative_ticks() {
    struct Planner;
    impl Npc for Planner {
        fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
            let mut prediction = context.world.clone();
            prediction.advance_tick();
            prediction.advance_tick();
            Vec::new()
        }
    }
    let mut registry = BotRegistry::builtins();
    registry.register("planner", || Box::new(Planner)).unwrap();
    let report = run_tournament(
        &registry,
        ArenaConfig {
            npcs: vec!["planner".into(), "idle-v1".into()],
            scenarios: vec![Scenario::Duel],
            rounds: 1,
            max_ticks: 5,
            profile: true,
            profile_sample_every: 2,
            ..ArenaConfig::default()
        },
    )
    .unwrap();
    let profile = report.profiling.unwrap();
    assert_eq!(profile.overall.observed_ticks, 10);
    assert_eq!(profile.overall.sampled_ticks, 6);
    assert_eq!(profile.overall.timings["engine_tick"].calls, 18);
    assert_eq!(profile.npc_decisions["planner"].calls, 6);
}
