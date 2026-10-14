//! 确定性状态摘要（锁步校验用）。

use crate::{
    game::GameCommand,
    state::{
        BattleState,
        components::{
            AnimationState, AttackState, CombatStats, HarvesterState, Health, Identity, Locomotor, Owner, ProductionQueue, Transform,
        },
    },
};
use ra_types::ScheduledCommand;

impl BattleState {
    pub(crate) fn rehash(&mut self) {
        #[cfg(feature = "profiling")]
        let _timer = crate::profiling::span(crate::profiling::Stage::StateHash);
        let mut h = self.tick;
        h = h.wrapping_mul(1099511628211).wrapping_add(self.edition.as_str().len() as u64);
        h = h.wrapping_mul(1099511628211).wrapping_add(self.entities.len() as u64);
        h = h.wrapping_mul(1099511628211).wrapping_add(self.last_input_frame.tick).wrapping_add(self.last_input_frame.commands.len() as u64);
        for cmd in &self.last_input_frame.commands {
            h = hash_scheduled(h, cmd);
        }
        let transforms = self.ecs.world().view::<Transform>();
        let healths = self.ecs.world().view::<Health>();
        let locomotors = self.ecs.world().view::<Locomotor>();
        let attacks = self.ecs.world().view::<AttackState>();
        let animations = self.ecs.world().view::<AnimationState>();
        let harvesters = self.ecs.world().view::<HarvesterState>();
        let combat_stats = self.ecs.world().view::<CombatStats>();
        let queues = self.ecs.world().view::<ProductionQueue>();
        let identities = self.ecs.world().view::<Identity>();
        let owners = self.ecs.world().view::<Owner>();
        for entity in &self.entities {
            let id = entity.id;
            let handle = self.ecs.resolve(id);
            macro_rules! get {
                ($view:ident) => {
                    handle.and_then(|h| $view.get(h))
                };
            }
            let xf = get!(transforms).copied();
            let health = get!(healths).copied();
            let loco = get!(locomotors).copied();
            let attack = get!(attacks).copied();
            let anim = get!(animations).copied();
            let harvester = get!(harvesters).copied();
            let stats = get!(combat_stats);
            let armor = stats.map(|s| s.armor).unwrap_or_default();
            let attack_verses = stats.map(|s| s.attack_verses).unwrap_or([0; 11]);
            let attack_range = stats.map(|s| s.attack_range).unwrap_or(0);
            let attack_damage = stats.map(|s| s.attack_damage).unwrap_or(0);
            let attack_cooldown_max = stats.map(|s| s.attack_cooldown_max).unwrap_or(0);
            let queue = get!(queues);
            let produce_item = queue.and_then(|p| p.item);
            let produce_ready = queue.and_then(|p| p.ready);
            let rally_x = queue.and_then(|p| p.rally_x);
            let rally_y = queue.and_then(|p| p.rally_y);
            let type_id = get!(identities).map(|i| i.type_id);
            let owner = get!(owners).map(|o| o.house);

            let x = xf.map(|t| t.x).unwrap_or(0);
            let y = xf.map(|t| t.y).unwrap_or(0);
            let facing = xf.map(|t| t.facing).unwrap_or(0);
            let turret_facing = xf.map(|t| t.turret_facing).unwrap_or(0);
            let current_health = health.map(|h| h.current).unwrap_or(0);
            let max_health = health.map(|h| h.maximum).unwrap_or(0);
            let dead = health.map(|h| h.dead).unwrap_or(false);
            let speed = loco.map(|l| l.speed).unwrap_or(0);
            let hva_frame = anim.map(|a| a.hva_frame).unwrap_or(0);
            let hit_flash = anim.map(|a| a.hit_flash).unwrap_or(0);
            let fire_flash = anim.map(|a| a.fire_flash).unwrap_or(0);
            let attack_cooldown = attack.map(|a| a.cooldown).unwrap_or(0);
            let attack_target = attack.and_then(|a| a.target);
            let ore_trip_accum = harvester.map(|h| h.ore_trip_accum).unwrap_or(0);
            let ore_cargo = harvester.map(|h| h.cargo).unwrap_or(0);

            h = h
                .wrapping_mul(1099511628211)
                .wrapping_add(id.0)
                .wrapping_add(x as u64)
                .wrapping_add((y as u64) << 16)
                .wrapping_add((facing as u64) << 32)
                .wrapping_add((turret_facing as u64) << 40)
                .wrapping_add(u64::from(current_health))
                .wrapping_add(u64::from(max_health).wrapping_shl(1))
                .wrapping_add(u64::from(speed).wrapping_shl(2))
                .wrapping_add(u64::from(attack_range).wrapping_shl(3))
                .wrapping_add(u64::from(attack_damage).wrapping_shl(4))
                .wrapping_add(u64::from(attack_cooldown_max).wrapping_shl(5))
                .wrapping_add(u64::from(dead))
                .wrapping_add(u64::from(hva_frame) << 8)
                .wrapping_add(u64::from(attack_cooldown) << 24)
                .wrapping_add(u64::from(ore_trip_accum) << 8)
                .wrapping_add(u64::from(ore_cargo) << 12)
                .wrapping_add(u64::from(hit_flash) << 16)
                .wrapping_add(u64::from(fire_flash) << 20)
                .wrapping_add(attack_target.map(|tid| tid.0).unwrap_or(0) << 32);
            for b in armor.as_str().as_bytes() {
                h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(*b));
            }
            for v in attack_verses {
                h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(v));
            }
            if let Some((qid, rem)) = produce_item {
                h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(rem));
                for b in qid.0.to_le_bytes() {
                    h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(b));
                }
            }
            if let Some(ready) = produce_ready {
                h = h.wrapping_mul(1099511628211).wrapping_add(3);
                for b in ready.0.to_le_bytes() {
                    h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(b));
                }
            }
            h = h
                .wrapping_mul(1099511628211)
                .wrapping_add(rally_x.map(u64::from).unwrap_or(0))
                .wrapping_add(rally_y.map(|v| u64::from(v) << 16).unwrap_or(0));
            if let Some(type_id) = type_id {
                for b in type_id.0.to_le_bytes() {
                    h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(b));
                }
            }
            if let Some(owner) = owner {
                for b in owner.0.to_le_bytes() {
                    h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(b));
                }
            }
        }
        for p in &self.players {
            h = h
                .wrapping_mul(1099511628211)
                .wrapping_add(u64::from(p.id.0))
                .wrapping_add(p.funds as u64)
                .wrapping_add(p.funds_spent as u64)
                .wrapping_add((p.power_output as u64) << 16)
                .wrapping_add((p.power_drain as u64) << 32)
                .wrapping_add(u64::from(p.eva_funds_nag_ticks))
                .wrapping_add(u64::from(p.kills) << 8)
                .wrapping_add(u64::from(p.built) << 16);
            for b in p.house.as_bytes() {
                h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(*b));
            }
        }
        h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(self.speak_delay_ticks));
        for g in &self.eva_base_under_attack {
            h = h
                .wrapping_mul(1099511628211)
                .wrapping_add(u64::from(g.x))
                .wrapping_add((u64::from(g.y)) << 16)
                .wrapping_add(g.expire_tick.wrapping_mul(0x9E37_79B9));
            for b in g.house.as_bytes() {
                h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(*b));
            }
        }
        h = h.wrapping_mul(1099511628211).wrapping_add(match self.map.lighting_profile {
            ra_map::LightingProfile::Normal => 0,
            ra_map::LightingProfile::Ion => 1,
        });
        if let Some(storm) = &self.lightning_storm {
            h = h
                .wrapping_mul(1099511628211)
                .wrapping_add(1)
                .wrapping_add(u64::from(storm.target_x))
                .wrapping_add((u64::from(storm.target_y)) << 16)
                .wrapping_add((storm.deferment_remaining as u64) << 32)
                .wrapping_add(storm.duration_remaining as u64);
        }
        self.state_hash = h;
    }
}

pub(crate) fn hash_scheduled(mut h: u64, cmd: &ScheduledCommand) -> u64 {
    h = h.wrapping_mul(1099511628211).wrapping_add(cmd.id.0);
    h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(cmd.player.0));
    h = h.wrapping_mul(1099511628211).wrapping_add(cmd.tick.0);
    hash_command(h, &cmd.body)
}

pub(crate) fn hash_command(mut h: u64, cmd: &GameCommand) -> u64 {
    match *cmd {
        GameCommand::MoveTo { entity, x, y } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(1);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0).wrapping_add((x as u64) << 16).wrapping_add((y as u64) << 32);
        }
        GameCommand::MovePath { entity, ref points } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(13);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0);
            for &(x, y) in points {
                h = h.wrapping_mul(1099511628211).wrapping_add((x as u64) << 16).wrapping_add((y as u64) << 32);
            }
        }
        GameCommand::Attack { attacker, target } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(2);
            h = h.wrapping_mul(1099511628211).wrapping_add(attacker.0).wrapping_add(target.0 << 16);
        }
        GameCommand::Deploy { entity } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(3);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0);
        }
        GameCommand::PlaceBuilding { player, type_id, x, y } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(4);
            h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(player.0)).wrapping_add((x as u64) << 8).wrapping_add((y as u64) << 24);
            h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(type_id.0));
        }
        GameCommand::Produce { player, type_id } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(5);
            h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(player.0));
            h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(type_id.0));
        }
        GameCommand::SetRallyPoint { factory, x, y } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(6);
            h = h.wrapping_mul(1099511628211).wrapping_add(factory.0).wrapping_add((x as u64) << 16).wrapping_add((y as u64) << 32);
        }
        GameCommand::Infiltrate { agent, building } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(7);
            h = h.wrapping_mul(1099511628211).wrapping_add(agent.0).wrapping_add(building.0 << 16);
        }
        GameCommand::CancelProduce { player, type_id } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(8);
            h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(player.0));
            h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(type_id.0));
        }
        GameCommand::CaptureBuilding { engineer, building } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(9);
            h = h.wrapping_mul(1099511628211).wrapping_add(engineer.0).wrapping_add(building.0 << 16);
        }
        GameCommand::Guard { entity } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(10);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0);
        }
        GameCommand::Stop { entity } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(15);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0);
        }
        GameCommand::AttackMove { entity, x, y } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(16);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0).wrapping_add((x as u64) << 16).wrapping_add((y as u64) << 32);
        }
        GameCommand::Follow { entity, target } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(19);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0).wrapping_add(target.0 << 16);
        }
        GameCommand::Scatter { entity } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(17);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0);
        }
        GameCommand::Delete { entity } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(18);
            h = h.wrapping_mul(1099511628211).wrapping_add(entity.0);
        }
        GameCommand::SellBuilding { player, building } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(11);
            h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(player.0)).wrapping_add(building.0 << 8);
        }
        GameCommand::RepairBuilding { player, building } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(12);
            h = h.wrapping_mul(1099511628211).wrapping_add(u64::from(player.0)).wrapping_add(building.0 << 8);
        }
        GameCommand::FireSuperWeapon { player, type_id, x, y } => {
            h = h.wrapping_mul(1099511628211).wrapping_add(14);
            h = h
                .wrapping_mul(1099511628211)
                .wrapping_add(u64::from(player.0))
                .wrapping_add(u64::from(type_id.0) << 8)
                .wrapping_add((x as u64) << 40)
                .wrapping_add((y as u64) << 56);
        }
    }
    h
}
