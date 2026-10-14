//! 基础 AI：只经 `GameCommand` 下发，不直接改写世界。
//!
//! 候选建筑 / 单位由冻结定义 + Owner 过滤选出，不硬编码外部类型名。
//!
//! 有 `[AITriggerTypes]` 覆盖的房主：经济基建可由本模块补齐，作战部队交给
//! `tick_ai_triggers` → Create Team / ScriptTypes，避免与启发式工厂量产叠刷。

use crate::{
    BattleState, GameCommand,
    gameplay::{
        TechTreePlayer, deploy_into_type, factory_matches_category, is_construction_yard, is_power_plant, is_refinery, is_type_eligible_id,
        living_structure_keys,
    },
    state::components::{AttackState, CombatStats, Health, Identity, Owner, ProductionQueue, Transform},
};
use ra_map::MapEntityKind;
use ra_types::{PlayerId, ProductionCategory, TechnoCategory};

/// 为指定玩家生成一拍内置启发式 AI 命令；不修改世界，也不依赖本地玩家席位。
///
/// 遭遇战与无窗口 NPC 评测共用此入口。`difficulty` 接受 Easy / Normal / Hard；
/// AITrigger 覆盖的房主只补经济基建，军队仍交给地图脚本。
pub fn heuristic_ai_commands(world: &BattleState, player: PlayerId, difficulty: &str) -> Vec<GameCommand> {
    let Some(state) = world.players.iter().find(|p| p.id == player)
    else {
        return Vec::new();
    };
    let house = state.house.as_ref();
    if is_ambient_house(house) || !world.house_production_begun(house) {
        return Vec::new();
    }
    let mut cmds = Vec::new();
    cmds.extend(deploy_mcv_commands(world, house));
    cmds.extend(place_power_commands(world, house, player));
    cmds.extend(place_refinery_commands(world, house, player));
    if house_army_driven_by_ai_triggers(world, house) {
        return cmds;
    }
    cmds.extend(place_barracks_commands(world, house, player));
    cmds.extend(place_war_factory_commands(world, house, player));
    if !difficulty_skips_offensive(difficulty, world.tick) {
        if heuristic_should_produce_army(world, house) {
            cmds.extend(produce_infantry_commands(world, house, player));
            cmds.extend(produce_vehicle_commands(world, house, player));
            if difficulty_extra_produce(difficulty) {
                cmds.extend(produce_infantry_commands(world, house, player));
                cmds.extend(produce_vehicle_commands(world, house, player));
            }
        }
        cmds.extend(auto_attack_commands(world, house));
    }
    cmds
}

/// 启发式量产：同房主存活机动作战单位上限（达到后停刷）。
const HEURISTIC_ARMY_CAP: usize = 8;
/// 启发式量产：首批单位之后的下单间隔（逻辑 tick）。
const HEURISTIC_PRODUCE_PERIOD: u64 = 45;

/// 该房主的作战部队是否应由 `[AITriggerTypes]` 驱动（而非工厂启发式量产）。
pub fn house_army_driven_by_ai_triggers(world: &BattleState, house: &str) -> bool {
    if world.prepared.ai_triggers.is_empty() {
        return false;
    }
    for at in &world.prepared.ai_triggers {
        if let Some(house_id) = at.owner_house {
            if world.definitions.houses.get_by_id(house_id).is_some_and(|h| h.type_key.as_str().eq_ignore_ascii_case(house)) {
                return true;
            }
            continue;
        }
        if let Some(team) = world.prepared.team_types.iter().find(|t| t.id == at.team) {
            match team.house {
                None => return true,
                Some(team_house) => {
                    if world.definitions.houses.get_by_id(team_house).is_some_and(|h| h.type_key.as_str().eq_ignore_ascii_case(house)) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// 无 AITrigger 覆盖时：是否允许本 tick 启发式工厂量产（首批放行，其后节流并封顶）。
pub fn heuristic_should_produce_army(world: &BattleState, house: &str) -> bool {
    let army = count_mobile_combatants(world, house);
    if army >= HEURISTIC_ARMY_CAP {
        return false;
    }
    if army == 0 {
        return true;
    }
    world.tick % HEURISTIC_PRODUCE_PERIOD == 0
}

fn count_mobile_combatants(world: &BattleState, house: &str) -> usize {
    world
        .entities
        .iter()
        .filter(|e| {
            let id = e.id;
            if world.ecs_get::<Health>(id).map(|h| h.dead).unwrap_or(true) {
                return false;
            }
            if !world.ecs_get::<Owner>(id).map(|o| crate::gameplay::house_id_of(&world.definitions, house) == Some(o.house)).unwrap_or(false) {
                return false;
            }
            let Some(identity) = world.ecs_get::<Identity>(id)
            else {
                return false;
            };
            matches!(identity.kind, MapEntityKind::Unit | MapEntityKind::Infantry | MapEntityKind::Aircraft)
                && world.ecs_get::<CombatStats>(id).map(|s| s.attack_damage > 0).unwrap_or(false)
        })
        .count()
}

/// 地图氛围房主（平民装饰 / 多人被动），不参与遭遇战 AI，也不计入胜负作战力量。
pub fn is_ambient_house(house: &str) -> bool {
    house.eq_ignore_ascii_case("Neutral") || house.eq_ignore_ascii_case("Civilian") || house.eq_ignore_ascii_case("Special")
}

/// 两 house 是否同盟（同名，或任一方 `PlayerState.allies` 列出对方）。
pub fn houses_are_allied(world: &BattleState, a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    let a_lists_b = world
        .players
        .iter()
        .find(|p| p.house.as_ref().eq_ignore_ascii_case(a))
        .map(|p| p.allies.iter().any(|x| x.eq_ignore_ascii_case(b)))
        .unwrap_or(false);
    if a_lists_b {
        return true;
    }
    world
        .players
        .iter()
        .find(|p| p.house.as_ref().eq_ignore_ascii_case(b))
        .map(|p| p.allies.iter().any(|x| x.eq_ignore_ascii_case(a)))
        .unwrap_or(false)
}

/// 为本阵营未部署的可部署单位生成 `Deploy`（已有建造场则跳过）。
pub fn deploy_mcv_commands(world: &BattleState, house: &str) -> Vec<GameCommand> {
    if house_has_yard(world, house) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for e in world.entities.iter() {
        let id = e.id;
        if world.ecs_get::<Health>(id).map(|h| h.dead).unwrap_or(true) {
            continue;
        }
        if !world.ecs_get::<Owner>(id).map(|o| crate::gameplay::house_id_of(&world.definitions, house) == Some(o.house)).unwrap_or(false) {
            continue;
        }
        let Some(identity) = world.ecs_get::<Identity>(id)
        else {
            continue;
        };
        if deploy_into_type(&world.definitions, identity.type_id).is_none() {
            continue;
        }
        if identity.kind != MapEntityKind::Unit {
            continue;
        }
        out.push(GameCommand::Deploy { entity: id });
    }
    out
}

/// 有建造场且无供电时：若电厂已完工则落位，否则排队建造。
pub fn place_power_commands(world: &BattleState, house: &str, player: PlayerId) -> Vec<GameCommand> {
    if !house_has_yard(world, house) || house_has_power(world, house) {
        return Vec::new();
    }
    let Some(power) = pick_structure(world, house, |s| s.power.output > 0)
    else {
        return Vec::new();
    };
    build_or_place(world, house, player, power)
}

/// 有供电且无兵营时：若兵营已完工则落位，否则排队建造。
pub fn place_barracks_commands(world: &BattleState, house: &str, player: PlayerId) -> Vec<GameCommand> {
    if !house_has_power(world, house) || house_has_factory(world, house, ProductionCategory::Infantry) {
        return Vec::new();
    }
    let Some(barracks) = pick_structure(world, house, |s| s.production.as_ref().is_some_and(|p| p.category == ProductionCategory::Infantry))
    else {
        return Vec::new();
    };
    build_or_place(world, house, player, barracks)
}

/// 有供电且无战车工厂时：若车厂已完工则落位，否则排队建造。
pub fn place_war_factory_commands(world: &BattleState, house: &str, player: PlayerId) -> Vec<GameCommand> {
    if !house_has_power(world, house) || house_has_factory(world, house, ProductionCategory::Vehicle) {
        return Vec::new();
    }
    let Some(wf) = pick_structure(world, house, |s| s.production.as_ref().is_some_and(|p| p.category == ProductionCategory::Vehicle))
    else {
        return Vec::new();
    };
    build_or_place(world, house, player, wf)
}

/// 有供电且无矿场时：若矿场已完工则落位，否则排队建造。
pub fn place_refinery_commands(world: &BattleState, house: &str, player: PlayerId) -> Vec<GameCommand> {
    if !house_has_power(world, house) || house_has_refinery(world, house) {
        return Vec::new();
    }
    let Some(refinery) = pick_structure(world, house, |s| s.refinery)
    else {
        return Vec::new();
    };
    build_or_place(world, house, player, refinery)
}

/// 有空闲兵营时生产一名步兵。
pub fn produce_infantry_commands(world: &BattleState, house: &str, player: PlayerId) -> Vec<GameCommand> {
    if !house_has_idle_factory(world, house, ProductionCategory::Infantry) {
        return Vec::new();
    }
    let Some(unit) = pick_techno(world, house, ProductionCategory::Infantry)
    else {
        return Vec::new();
    };
    produce_unit(world, house, player, unit)
}

/// 有空闲战车工厂时生产一辆载具。
pub fn produce_vehicle_commands(world: &BattleState, house: &str, player: PlayerId) -> Vec<GameCommand> {
    if !house_has_idle_factory(world, house, ProductionCategory::Vehicle) {
        return Vec::new();
    }
    let Some(unit) = pick_techno(world, house, ProductionCategory::Vehicle)
    else {
        return Vec::new();
    };
    produce_unit(world, house, player, unit)
}

fn produce_unit(world: &BattleState, house: &str, player: PlayerId, unit: &ra_types::TechnoDefinition) -> Vec<GameCommand> {
    let cost = unit.cost.max(0) as u32;
    let Some(funds) = world.house_funds(house)
    else {
        return Vec::new();
    };
    if funds < cost as i32 {
        return Vec::new();
    }
    vec![GameCommand::Produce { player, type_id: unit.id }]
}

/// 建造场已有该类型完工件则落位，否则在空闲建造场排队 `Produce`。
fn build_or_place(world: &BattleState, house: &str, player: PlayerId, structure: &ra_types::StructureDefinition) -> Vec<GameCommand> {
    if let Some(ready) = world.house_ready_building(house) {
        if ready != structure.id {
            // 另有完工建筑待落位，先不插队。
            return Vec::new();
        }
        return place_near_yard(world, house, player, structure);
    }
    if !house_has_idle_yard(world, house) {
        return Vec::new();
    }
    let cost = if structure.cost > 0 {
        structure.cost.max(0) as u32
    }
    else {
        world.definitions.techno.get_by_id(structure.id).map(|t| t.cost.max(0) as u32).unwrap_or(0)
    };
    let Some(funds) = world.house_funds(house)
    else {
        return Vec::new();
    };
    if funds < cost as i32 {
        return Vec::new();
    }
    vec![GameCommand::Produce { player, type_id: structure.id }]
}

fn place_near_yard(world: &BattleState, house: &str, player: PlayerId, structure: &ra_types::StructureDefinition) -> Vec<GameCommand> {
    let Some((yx, yy)) = yard_cell(world, house)
    else {
        return Vec::new();
    };
    let foundation = &structure.foundation;
    let Some((x, y)) = find_open_near(world, yx, yy, foundation.width, foundation.height)
    else {
        return Vec::new();
    };
    vec![GameCommand::PlaceBuilding { player, type_id: structure.id, x, y }]
}

/// 动态集火：优先射程内威胁与残血，预留本拍伤害以免重复击杀。
pub fn auto_attack_commands(world: &BattleState, house: &str) -> Vec<GameCommand> {
    let owner = crate::gameplay::house_id_of(&world.definitions, house);
    let mut out = Vec::new();
    let mut assigned = std::collections::BTreeMap::<ra_types::EntityId, u32>::new();
    for attacker in &world.entities {
        let id = attacker.id;
        if !world.ecs_get::<Health>(id).is_some_and(|h| !h.dead) || !world.ecs_get::<Owner>(id).is_some_and(|o| Some(o.house) == owner) {
            continue;
        }
        let (Some(stats), Some(position), Some(attack), Some(identity)) =
            (world.ecs_get::<CombatStats>(id), world.ecs_get::<Transform>(id), world.ecs_get::<AttackState>(id), world.ecs_get::<Identity>(id))
        else {
            continue;
        };
        if stats.attack_damage == 0
            || !matches!(identity.kind, MapEntityKind::Unit | MapEntityKind::Infantry | MapEntityKind::Aircraft)
            || identity.mission == Some(ra_types::MissionKind::Guard)
            || attack.capture_target.is_some()
            || attack.infiltrate_target.is_some()
        {
            continue;
        }
        let target = world
            .entities
            .iter()
            .filter_map(|enemy| {
                let hp = world.ecs_get::<Health>(enemy.id)?;
                let enemy_owner = world.ecs_get::<Owner>(enemy.id)?;
                let enemy_house = crate::gameplay::house_key_of(&world.definitions, enemy_owner.house);
                if hp.dead || Some(enemy_owner.house) == owner || is_ambient_house(enemy_house) || houses_are_allied(world, house, enemy_house)
                {
                    return None;
                }
                let enemy_position = world.ecs_get::<Transform>(enemy.id)?;
                let enemy_stats = world.ecs_get::<CombatStats>(enemy.id)?;
                let damage = (u64::from(stats.attack_damage) * u64::from(stats.attack_verses[enemy_stats.armor.index()]) / 100) as u32;
                if damage == 0 {
                    return None;
                }
                let distance = (i32::from(position.x) - i32::from(enemy_position.x)).unsigned_abs()
                    + (i32::from(position.y) - i32::from(enemy_position.y)).unsigned_abs();
                let reserved = assigned.get(&enemy.id).copied().unwrap_or(0);
                let key = (
                    distance > stats.attack_range,
                    reserved >= hp.current,
                    enemy_stats.attack_damage == 0,
                    hp.current.saturating_sub(reserved).div_ceil(damage),
                    distance,
                    enemy.id,
                );
                Some((key, enemy.id, damage, distance))
            })
            .min_by_key(|item| item.0);
        let Some((_, target, damage, distance)) = target
        else {
            continue;
        };
        if attack.cooldown == 0 && distance <= stats.attack_range {
            *assigned.entry(target).or_default() += damage;
        }
        if attack.target != Some(target) {
            out.push(GameCommand::Attack { attacker: id, target });
        };
    }
    out
}

fn pick_structure<'a, F>(world: &'a BattleState, house: &str, pred: F) -> Option<&'a ra_types::StructureDefinition>
where
    F: Fn(&ra_types::StructureDefinition) -> bool,
{
    let living = living_structure_keys(world, house);
    let Some(player) = world.players.iter().find(|p| p.house.eq_ignore_ascii_case(house))
    else {
        return None;
    };
    let tech = TechTreePlayer::from_player(player);
    world
        .definitions
        .structures
        .iter()
        .find(|s| pred(s) && !s.construction_yard && is_type_eligible_id(&world.definitions, tech, &living, s.id))
}

fn pick_techno<'a>(world: &'a BattleState, house: &str, category: ProductionCategory) -> Option<&'a ra_types::TechnoDefinition> {
    let living = living_structure_keys(world, house);
    let Some(player) = world.players.iter().find(|p| p.house.eq_ignore_ascii_case(house))
    else {
        return None;
    };
    let tech = TechTreePlayer::from_player(player);
    world
        .definitions
        .techno
        .iter()
        .filter(|t| {
            t.class.production_category() == Some(category)
                && is_type_eligible_id(&world.definitions, tech, &living, t.id)
                && t.class != ra_types::TechnoClass::Building
                // 陆地工厂不造海军单位（否则 DEST 等会从战车厂刷出）。
                && !t.naval
                // 警犬等 Category=Dog 不进常规量产。
                && t.category != TechnoCategory::Dog
                && deploy_into_type(&world.definitions, t.id).is_none()
        })
        // 同科技等级下优先较便宜的基础单位；再按类型键稳定排序。
        .min_by_key(|t| (t.tech_level, t.cost, t.type_key.as_str()))
}

fn living_house_structure<'a, F>(world: &'a BattleState, house: &str, pred: F) -> bool
where
    F: Fn(&BattleState, &Identity) -> bool,
{
    world.entities.iter().any(|e| {
        let id = e.id;
        !world.ecs_get::<Health>(id).map(|h| h.dead).unwrap_or(true)
            && world.ecs_get::<Owner>(id).map(|o| crate::gameplay::house_id_of(&world.definitions, house) == Some(o.house)).unwrap_or(false)
            && world.ecs_get::<Identity>(id).map(|i| i.kind == MapEntityKind::Structure && pred(world, i)).unwrap_or(false)
    })
}

fn house_has_yard(world: &BattleState, house: &str) -> bool {
    living_house_structure(world, house, |w, i| is_construction_yard(&w.definitions, i.type_id))
}

fn house_has_idle_yard(world: &BattleState, house: &str) -> bool {
    world.entities.iter().any(|e| {
        let id = e.id;
        !world.ecs_get::<Health>(id).map(|h| h.dead).unwrap_or(true)
            && world.ecs_get::<Owner>(id).map(|o| crate::gameplay::house_id_of(&world.definitions, house) == Some(o.house)).unwrap_or(false)
            && world
                .ecs_get::<Identity>(id)
                .map(|i| i.kind == MapEntityKind::Structure && is_construction_yard(&world.definitions, i.type_id))
                .unwrap_or(false)
            && world.ecs_get::<ProductionQueue>(id).map(|q| q.item.is_none() && q.ready.is_none()).unwrap_or(false)
    })
}

fn house_has_power(world: &BattleState, house: &str) -> bool {
    living_house_structure(world, house, |w, i| is_power_plant(&w.definitions, i.type_id))
}

fn house_has_factory(world: &BattleState, house: &str, category: ProductionCategory) -> bool {
    living_house_structure(world, house, |w, i| factory_matches_category(&w.definitions, i.type_id, category))
}

fn house_has_idle_factory(world: &BattleState, house: &str, category: ProductionCategory) -> bool {
    world.entities.iter().any(|e| {
        let id = e.id;
        !world.ecs_get::<Health>(id).map(|h| h.dead).unwrap_or(true)
            && world.ecs_get::<Owner>(id).map(|o| crate::gameplay::house_id_of(&world.definitions, house) == Some(o.house)).unwrap_or(false)
            && world.ecs_get::<Identity>(id).map(|i| i.kind == MapEntityKind::Structure).unwrap_or(false)
            && world.ecs_get::<ProductionQueue>(id).map(|q| q.item.is_none()).unwrap_or(true)
            && world.ecs_get::<Identity>(id).map(|i| factory_matches_category(&world.definitions, i.type_id, category)).unwrap_or(false)
    })
}

fn house_has_refinery(world: &BattleState, house: &str) -> bool {
    living_house_structure(world, house, |w, i| is_refinery(&w.definitions, i.type_id))
}

fn yard_cell(world: &BattleState, house: &str) -> Option<(u16, u16)> {
    world.entities.iter().find_map(|e| {
        let id = e.id;
        if world.ecs_get::<Health>(id).map(|h| h.dead).unwrap_or(true) {
            return None;
        }
        if !world.ecs_get::<Owner>(id).map(|o| crate::gameplay::house_id_of(&world.definitions, house) == Some(o.house)).unwrap_or(false) {
            return None;
        }
        let identity = world.ecs_get::<Identity>(id)?;
        if identity.kind != MapEntityKind::Structure || !is_construction_yard(&world.definitions, identity.type_id) {
            return None;
        }
        world.ecs_get::<Transform>(id).map(|t| (t.x, t.y))
    })
}

/// 在建造场附近按完整占地找可放置格（左上角）。
///
/// 旧逻辑只查邻格 `1x1`，真实 `Foundation=2x2` / `3x4` 会在 `PlaceBuilding` 被拒，
/// 表现为 AI 有钱却造不出建筑、只刷已有兵营的大兵。
fn find_open_near(world: &BattleState, fx: u16, fy: u16, width: u16, height: u16) -> Option<(u16, u16)> {
    let width = width.max(1);
    let height = height.max(1);
    const MAX_RADIUS: i32 = 16;
    for radius in 1..=MAX_RADIUS {
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let x = i32::from(fx) + dx;
                let y = i32::from(fy) + dy;
                if x < 0 || y < 0 {
                    continue;
                }
                let (x, y) = (x as u16, y as u16);
                if world.can_place_structure_footprint(x, y, width, height) {
                    return Some((x, y));
                }
            }
        }
    }
    None
}

fn nearest_enemy(world: &BattleState, from: usize, house: &str) -> Option<usize> {
    let from_id = world.entities[from].id;
    let from_xf = world.ecs_get::<Transform>(from_id)?;
    let mut best: Option<(u32, usize)> = None;
    for (i, e) in world.entities.iter().enumerate() {
        if i == from {
            continue;
        }
        let id = e.id;
        if world.ecs_get::<Health>(id).map(|h| h.dead).unwrap_or(true) {
            continue;
        }
        if world.ecs_get::<Owner>(id).map(|o| crate::gameplay::house_id_of(&world.definitions, house) == Some(o.house)).unwrap_or(false) {
            continue;
        }
        if world
            .ecs_get::<Owner>(id)
            .map(|o| houses_are_allied(world, house, crate::gameplay::house_key_of(&world.definitions, o.house)))
            .unwrap_or(false)
        {
            continue;
        }
        if world.ecs_get::<Owner>(id).map(|o| is_ambient_house(crate::gameplay::house_key_of(&world.definitions, o.house))).unwrap_or(false) {
            continue;
        }
        let Some(xf) = world.ecs_get::<Transform>(id)
        else {
            continue;
        };
        let dist = (i32::from(from_xf.x) - i32::from(xf.x)).unsigned_abs() + (i32::from(from_xf.y) - i32::from(xf.y)).unsigned_abs();
        if best.map(|(d, _)| dist < d).unwrap_or(true) {
            best = Some((dist, i));
        }
    }
    best.map(|(_, i)| i)
}

/// 按难度决定本 tick 是否跳过 AI 进攻/生产。
pub fn difficulty_skips_offensive(difficulty: &str, tick: u64) -> bool {
    if difficulty.eq_ignore_ascii_case("Easy") {
        tick % 2 == 1
    }
    else if difficulty.eq_ignore_ascii_case("Hard") {
        false
    }
    else {
        tick % 4 == 3
    }
}

/// Hard 是否追加一轮生产尝试。
pub fn difficulty_extra_produce(difficulty: &str) -> bool {
    difficulty.eq_ignore_ascii_case("Hard")
}
