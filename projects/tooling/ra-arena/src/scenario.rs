use std::sync::Arc;

use ra_adaptor::runtime_definitions_from_ini_bytes;
use ra_engine::{BattleState, Session};
use ra_map::{MapEntity, MapEntityKind, MapInfo, OverlayCell};
use ra_types::{GameEdition, RuntimeDefinitions};
use serde::{Deserialize, Serialize};

use crate::ArenaResult;

pub(crate) const RULES: &[u8] = include_bytes!("../fixtures/rules-v1.ini");
pub(crate) const ECONOMY_RULES: &[u8] = include_bytes!("../fixtures/economy-v1.ini");
pub(crate) const HOUSES: [&str; 2] = ["AMERICANS", "ALLIANCE"];

/// 冻结合成场景。每局同种子开局一致；镜像比赛交换 NPC，地图保持不变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scenario {
    /// 双方各三辆坦克与无资金建造场；长局规则，测试作战控制。
    #[serde(rename = "duel-v1")]
    Duel,
    /// 双方各一辆 MCV 与 10000 资金；短局规则，测试展开、经济、建造、生产与进攻。
    #[serde(rename = "skirmish-v1")]
    Skirmish,
    /// MCV、真实矿石与矿场赠送矿车；评测卸矿收入和完整开局。
    #[serde(rename = "economy-v1")]
    Economy,
}

impl Scenario {
    /// 稳定场景 ID。
    pub fn id(self) -> &'static str {
        match self {
            Self::Duel => "duel-v1",
            Self::Skirmish => "skirmish-v1",
            Self::Economy => "economy-v1",
        }
    }

    /// 从 CLI / 配置使用的 ID 解析；不接受未知场景。
    pub fn parse(id: &str) -> ArenaResult<Self> {
        match id {
            "duel-v1" => Ok(Self::Duel),
            "skirmish-v1" => Ok(Self::Skirmish),
            "economy-v1" => Ok(Self::Economy),
            _ => Err(format!("未知场景: {id}")),
        }
    }
}

pub(crate) fn rules(scenario: Scenario) -> &'static [u8] {
    if scenario == Scenario::Economy { ECONOMY_RULES } else { RULES }
}

pub(crate) fn definitions(scenario: Scenario) -> ArenaResult<Arc<RuntimeDefinitions>> {
    runtime_definitions_from_ini_bytes(GameEdition::Ra2, rules(scenario), None).map(Arc::new).map_err(|e| e.to_string())
}

pub(crate) fn open(scenario: Scenario, seed: u64, defs: Arc<RuntimeDefinitions>) -> ArenaResult<Session> {
    let mut map = MapInfo::empty(GameEdition::Ra2, scenario.id());
    map.width = 40;
    map.height = 32;
    if scenario == Scenario::Economy {
        map.width = 64;
        map.height = 40;
    }
    let y = 10 + (seed % 8) as u16;
    let inset = 5 + ((seed >> 8) % 4) as u16;
    for (seat, house) in HOUSES.iter().enumerate() {
        let x = if seat == 0 { inset } else { map.width as u16 - 1 - inset };
        let types: &[(&str, u16)] = match scenario {
            Scenario::Duel => &[("MTNK", 0), ("MTNK", 2), ("MTNK", 4)],
            Scenario::Skirmish | Scenario::Economy => &[("AMCV", 0)],
        };
        for &(type_id, offset) in types {
            map.entities.push(MapEntity {
                kind: MapEntityKind::Unit,
                owner: (*house).into(),
                type_id: type_id.into(),
                health: 256,
                x,
                y: y + offset,
                facing: if seat == 0 { 64 } else { 192 },
                sub_cell: 0,
                mission: Default::default(),
                tag: Default::default(),
            });
        }
        if scenario == Scenario::Economy {
            // 两侧矿区按地图中线镜像；每格可供 255 次采集，不引入无限收入。
            for dx in 2..7 {
                for dy in 4..9 {
                    let ore_x = inset + dx;
                    map.overlays.push(OverlayCell {
                        x: if seat == 0 { ore_x } else { map.width as u16 - 1 - ore_x },
                        y: y + dy,
                        overlay_id: 0,
                        data: 255,
                    });
                }
            }
        }
        if scenario == Scenario::Duel {
            // 游戏内置 AI 仅在已有建造场时主动进攻；无资金场只提供这一前提。
            map.entities.push(MapEntity {
                kind: MapEntityKind::Structure,
                owner: (*house).into(),
                type_id: "GACNST".into(),
                health: 256,
                x,
                y: y + 7,
                facing: 0,
                sub_cell: 0,
                mission: Default::default(),
                tag: Default::default(),
            });
        }
    }
    let prepared = ra_engine::validate_map_for_battle(&map, &defs).map_err(|e| e.to_string())?;
    let mut world = BattleState::from_prepared(GameEdition::Ra2, defs, map, prepared).map_err(|e| e.to_string())?;
    // 使用 house 查询实际 PlayerId；不能假设地图投影中的槽位顺序。
    for house in HOUSES {
        if !world.set_house_funds(house, if scenario == Scenario::Duel { 0 } else { 10000 }) {
            return Err(format!("场景未生成玩家 {house}"));
        }
    }
    let mut session = Session::from_state(world, "ra-arena synthetic v1");
    let battle = session.expect_battle_mut();
    battle.ai_enabled = false;
    battle.set_match_seed(seed);
    battle.set_short_game(scenario != Scenario::Duel);
    Ok(session)
}
