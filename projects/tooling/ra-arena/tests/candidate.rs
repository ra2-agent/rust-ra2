use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use ra_arena::{BotContext, BotRegistry, MatchSpec, Npc, Scenario, Score, play_match, replay_match};
use ra_engine::GameCommand;

fn spec(scene: Scenario, a: &str, b: &str) -> MatchSpec {
    MatchSpec { scenario: scene, seed: 42, seats: [a.into(), b.into()], max_ticks: 1500, max_commands_per_tick: 64, trace: true }
}

#[test]
fn candidate_defeats_frozen_hard_from_both_seats_and_replays() {
    let registry = BotRegistry::builtins();
    for scene in [Scenario::Duel, Scenario::Skirmish, Scenario::Economy] {
        for (a, b, expected) in [("candidate", "hard-baseline-v1", Score::Win), ("hard-baseline-v1", "candidate", Score::Loss)] {
            let record = play_match(&registry, spec(scene, a, b)).unwrap();
            assert_eq!(record.score, expected, "{scene:?}: {a}");
            replay_match(&record).unwrap();
        }
    }
}

#[test]
fn candidate_refinery_placement_improves_first_income() {
    let registry = BotRegistry::builtins();
    let original = play_match(&registry, spec(Scenario::Economy, "hard-baseline-v1", "idle-v1")).unwrap();
    let candidate = play_match(&registry, spec(Scenario::Economy, "candidate", "idle-v1")).unwrap();
    let original = &original.metrics.as_ref().unwrap()[0];
    let candidate = &candidate.metrics.as_ref().unwrap()[0];
    assert!(candidate.first_income_tick.unwrap() < original.first_income_tick.unwrap());
    assert_eq!(candidate.initial_funds, original.initial_funds);
}

#[test]
fn candidate_does_not_overcommit_funds_or_compete_for_one_building_queue() {
    let mut case = ra_testing::ai_skirmish_open();
    case.session.expect_battle_mut().ai_enabled = false;
    let player = case.session.expect_battle().world.players.iter().find(|p| p.house.eq_ignore_ascii_case("Russians")).unwrap().id;
    for _ in 0..180 {
        let world = &case.session.expect_battle().world;
        let commands = ra_engine::candidate_npc_commands(world, player);
        let mut costs = 0;
        let mut buildings = 0;
        for command in &commands {
            if let GameCommand::Produce { player: owner, type_id } = command {
                assert_eq!(*owner, player);
                let t = world.definitions.techno.get_by_id(*type_id).unwrap();
                costs += t.cost;
                buildings += usize::from(t.class == ra_types::TechnoClass::Building);
                if t.class != ra_types::TechnoClass::Building {
                    assert!(!t.harvester, "ordinary army production must not choose economic units");
                    assert!(t.primary_id.and_then(|w| world.definitions.weapons.get_by_id(w)).is_some_and(|w| w.damage > 0));
                }
            }
        }
        assert!(costs <= world.players.iter().find(|p| p.id == player).unwrap().funds);
        assert!(buildings <= 1);
        for command in commands {
            case.session.expect_battle_mut().world.push_player_command(player, command);
        }
        case.advance(1);
    }
}

struct CampCapped {
    peak: Arc<AtomicUsize>,
}

impl Npc for CampCapped {
    fn commands(&mut self, context: &BotContext<'_>) -> Vec<GameCommand> {
        let world = context.world;
        let house = &world.players.iter().find(|p| p.id == context.player).unwrap().house;
        let mut units = 0;
        let mut queued = 0;
        for id in (0..world.entity_count()).filter_map(|i| world.entity_id_at(i)) {
            if !world.ecs_owner(id).is_some_and(|h| h == *house) || !world.ecs_health(id).is_some_and(|h| !h.2) {
                continue;
            }
            let (key, kind) = world.ecs_identity(id).unwrap();
            units += usize::from(kind != ra_map::MapEntityKind::Structure && world.ecs_combat_view(id).is_some_and(|s| s.attack_damage > 0));
            if world.definitions.structures.get(key.as_ref()).is_some_and(|s| {
                s.production
                    .as_ref()
                    .is_some_and(|p| matches!(p.category, ra_types::ProductionCategory::Infantry | ra_types::ProductionCategory::Vehicle))
            }) {
                let (active, pending) = world.ecs_produce_unit_queue_len(id).unwrap();
                queued += usize::from(active) + pending;
            }
        }
        assert!(units + queued <= 8, "alive={units}, queued={queued}");
        self.peak.fetch_max(units, Ordering::SeqCst);
        ra_engine::candidate_variant_npc_commands(world, context.player, 1)
            .into_iter()
            .filter(|c| !matches!(c, GameCommand::Attack { .. } | GameCommand::AttackMove { .. }))
            .collect()
    }
}

#[test]
fn army_cap_counts_both_alive_units_and_in_flight_production() {
    let peak = Arc::new(AtomicUsize::new(0));
    let mut registry = BotRegistry::builtins();
    let observed = Arc::clone(&peak);
    registry.register("camp-cap8", move || Box::new(CampCapped { peak: Arc::clone(&observed) })).unwrap();
    let record = play_match(&registry, MatchSpec { max_ticks: 500, trace: false, ..spec(Scenario::Skirmish, "camp-cap8", "idle-v1") }).unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), 8);
    assert_eq!(record.rejected[0], 0);
}
