//! 自 `engine/ra-types/src/definition/structure.rs` 迁出的单元测试（集成测试 crate）。

// 自 engine/ra-types/src/definition/structure.rs :: tests
use ra_types::definition::structure::BuildCat;

#[test]
fn build_cat_combat_is_defense_tab() {
    assert_eq!(BuildCat::parse("Combat"), BuildCat::Combat);
    assert_eq!(BuildCat::parse("combat"), BuildCat::Combat);
    assert!(BuildCat::Combat.is_defense_tab());
}

#[test]
fn build_cat_missing_or_other_goes_to_building_tab() {
    assert_eq!(BuildCat::parse(""), BuildCat::Building);
    assert_eq!(BuildCat::parse("Tech"), BuildCat::Building);
    assert_eq!(BuildCat::parse("Power"), BuildCat::Building);
    assert_eq!(BuildCat::parse("Resource"), BuildCat::Building);
    assert!(!BuildCat::Building.is_defense_tab());
}

#[test]
fn borrowed_definition_queries_preserve_trim_and_ascii_case_rules() {
    use ra_types::{HouseDefinition, HouseDefinitions, HouseId, HouseName, SideName};
    let mut houses = HouseDefinitions::default();
    for (id, key) in [(7, "France"), (9, "苏联_é")] {
        houses.insert(HouseDefinition {
            id: HouseId(id),
            type_key: HouseName::parse(key),
            side: SideName::parse("GDI"),
            stolen_tech: None,
            multiplay: true,
        });
    }
    for query in ["FRANCE", "france", "  fRaNcE  ", "\u{2003}FRANCE\u{2003}"] {
        assert_eq!(houses.get(query).map(|h| h.id), Some(HouseId(7)));
    }
    assert_eq!(houses.get("  苏联_é  ").map(|h| h.id), Some(HouseId(9)));
    assert!(houses.get("苏联_É").is_none(), "INI 查询只规范 ASCII 大小写");
    for query in ["", "  ", "Unknown"] {
        assert!(houses.get(query).is_none());
    }
}
