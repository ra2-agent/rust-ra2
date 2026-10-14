use super::{
    super::{
        entities::WorldEntity,
        entity_projection::{ProjectComponent, project_all, project_entities},
    },
    types::BattleState,
};
use crate::spatial::is_mobile;
use ra_types::EntityId;

impl BattleState {
    /// 分配下一枚稳定实体 ID（供后续生成建筑/单位使用）。
    pub fn alloc_entity_id(&mut self) -> EntityId {
        let id = EntityId(self.next_entity_id);
        self.next_entity_id = self.next_entity_id.saturating_add(1);
        id
    }

    /// 以 ECS 组件包生成实体：占投影槽 → 写权威组件 → 投影回槽位。
    pub(crate) fn spawn_from_bundle(&mut self, bundle: crate::state::components::EntitySpawnBundle) -> usize {
        let id = bundle.identity.entity_id;
        self.entities.push(WorldEntity::projection_slot(id));
        let index = self.entities.len() - 1;
        self.ecs.bind_bundle(id, bundle);
        self.project_entity_from_ecs(id);
        index
    }

    /// 将单个实体的全部 ECS 组件投影回 `WorldEntity`。
    pub(crate) fn project_entity_from_ecs(&mut self, id: EntityId) {
        let Some(index) = self.entity_index(id)
        else {
            return;
        };
        self.project_entity_at(index);
    }

    fn project_entity_at(&mut self, index: usize) {
        if let Some(handle) = self.ecs.resolve(self.entities[index].id) {
            project_all(self.ecs.world(), handle, &self.definitions, &mut self.entities[index]);
        }
    }

    /// 按稳定 ID 只读取 ECS 组件（玩法系统应优先走此路径，而非读投影）。
    pub(crate) fn ecs_get<T: ra_ecs::Component>(&self, id: EntityId) -> Option<&T> {
        let handle = self.ecs.resolve(id)?;
        self.ecs.world().get::<T>(handle)
    }

    /// 把 ECS 权威组件投影回 `WorldEntity`（兼容尚未改读组件的路径）。
    ///
    /// 不再从 `WorldEntity` 回写 ECS，避免双真相。
    pub(crate) fn sync_ecs_components(&mut self) {
        #[cfg(feature = "profiling")]
        let _timer = crate::profiling::span(crate::profiling::Stage::EcsSync);
        project_entities(&self.ecs, &self.definitions, &mut self.entities);
    }

    fn with_component_mut<T: ProjectComponent, R>(&mut self, id: EntityId, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        #[cfg(feature = "profiling")]
        let _timer = crate::profiling::span(T::PROFILE_STAGE);
        let handle = self.ecs.resolve(id)?;
        let index = self.entity_index(id);
        let component = self.ecs.world_mut().get_mut::<T>(handle)?;
        let result = f(component);
        if let Some(index) = index {
            #[cfg(feature = "profiling")]
            let _projection_timer = crate::profiling::span(crate::profiling::Stage::PartialProjection);
            component.project(&self.definitions, &mut self.entities[index]);
        }
        Some(result)
    }

    /// 以 ECS 为权威修改身份，并立即投影回 `WorldEntity`。
    pub(crate) fn with_identity_mut<R>(&mut self, id: EntityId, f: impl FnOnce(&mut crate::state::components::Identity) -> R) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改所属房主，并立即投影回 `WorldEntity`。
    pub(crate) fn with_owner_mut<R>(&mut self, id: EntityId, f: impl FnOnce(&mut crate::state::components::Owner) -> R) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改移动能力，并立即投影回 `WorldEntity`。
    pub(crate) fn with_locomotor_mut<R>(&mut self, id: EntityId, f: impl FnOnce(&mut crate::state::components::Locomotor) -> R) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改战斗参数，并立即投影回 `WorldEntity`。
    pub(crate) fn with_combat_stats_mut<R>(
        &mut self,
        id: EntityId,
        f: impl FnOnce(&mut crate::state::components::CombatStats) -> R,
    ) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改生命，并立即投影回 `WorldEntity`。
    pub(crate) fn with_health_mut<R>(&mut self, id: EntityId, f: impl FnOnce(&mut crate::state::components::Health) -> R) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改空间变换，并立即投影回 `WorldEntity`。
    pub(crate) fn with_transform_mut<R>(&mut self, id: EntityId, f: impl FnOnce(&mut crate::state::components::Transform) -> R) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改移动状态，并立即投影回 `WorldEntity`。
    pub(crate) fn with_movement_mut<R>(
        &mut self,
        id: EntityId,
        f: impl FnOnce(&mut crate::state::components::MovementState) -> R,
    ) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 为指定下标实体重算路径并写入 ECS `MovementState`。
    pub(crate) fn repath_entity_at(&mut self, index: usize) {
        let path = self.compute_repath_at(index);
        let id = self.entities[index].id;
        let _ = self.with_movement_mut(id, |movement| {
            movement.path = path;
        });
    }

    /// 以 ECS 为权威修改攻击状态，并立即投影回 `WorldEntity`。
    pub(crate) fn with_attack_mut<R>(&mut self, id: EntityId, f: impl FnOnce(&mut crate::state::components::AttackState) -> R) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改生产队列，并立即投影回 `WorldEntity`。
    pub(crate) fn with_production_mut<R>(
        &mut self,
        id: EntityId,
        f: impl FnOnce(&mut crate::state::components::ProductionQueue) -> R,
    ) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改采矿行程，并立即投影回 `WorldEntity`。
    pub(crate) fn with_harvester_mut<R>(
        &mut self,
        id: EntityId,
        f: impl FnOnce(&mut crate::state::components::HarvesterState) -> R,
    ) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 以 ECS 为权威修改动画桥接状态，并立即投影回 `WorldEntity`。
    pub(crate) fn with_animation_mut<R>(
        &mut self,
        id: EntityId,
        f: impl FnOnce(&mut crate::state::components::AnimationState) -> R,
    ) -> Option<R> {
        self.with_component_mut(id, f)
    }

    /// 测试 / 调试：写入 ECS 炮塔朝向并投影。
    pub fn set_ecs_turret_facing(&mut self, id: EntityId, turret_facing: u8) -> bool {
        self.with_transform_mut(id, |transform| {
            transform.turret_facing = turret_facing;
        })
        .is_some()
    }

    /// 测试 / 调试：写入 ECS 格子坐标并投影。
    pub fn set_ecs_cell(&mut self, id: EntityId, x: u16, y: u16) -> bool {
        self.with_transform_mut(id, |transform| {
            transform.x = x;
            transform.y = y;
        })
        .is_some()
    }

    /// 测试 / 调试：写入 ECS `Health` 并投影。
    pub fn set_ecs_health(&mut self, id: EntityId, current: u32, maximum: u32, dead: bool) -> bool {
        self.with_health_mut(id, |health| {
            health.current = current;
            health.maximum = maximum;
            health.dead = dead;
        })
        .is_some()
    }

    /// 测试 / 调试：写入 ECS 攻击参数并投影。
    pub fn set_ecs_attack_power(&mut self, id: EntityId, damage: u32, range: u32, cooldown_max: u32) -> bool {
        self.with_combat_stats_mut(id, |stats| {
            stats.attack_damage = damage;
            stats.attack_range = range;
            stats.attack_cooldown_max = cooldown_max;
        })
        .is_some()
    }

    /// 测试 / 调试：写入 ECS 身份类型并投影。
    pub fn set_ecs_type_id(&mut self, id: EntityId, type_id: &str, kind: ra_map::MapEntityKind) -> bool {
        let Some(def_id) = crate::gameplay::type_id_of(&self.definitions, type_id)
        else {
            return false;
        };
        self.with_identity_mut(id, |identity| {
            identity.type_id = def_id;
            identity.kind = kind;
        })
        .is_some()
    }

    /// 测试 / 调试：写入 ECS 移动速度并投影。
    pub fn set_ecs_speed(&mut self, id: EntityId, speed: u32) -> bool {
        self.with_locomotor_mut(id, |loco| {
            loco.speed = speed;
        })
        .is_some()
    }

    /// 测试 / 调试：清空 ECS 移动目的地与路径并投影。
    pub fn clear_ecs_movement(&mut self, id: EntityId) -> bool {
        self.with_movement_mut(id, |movement| {
            movement.destination_x = None;
            movement.destination_y = None;
            movement.waypoints.clear();
            movement.path.clear();
            movement.move_accum = 0;
        })
        .is_some()
    }

    /// 通行表变更后，为全部移动单位重算路径。
    pub fn repath_mobiles(&mut self) {
        use crate::state::components::Identity;

        for i in 0..self.entities.len() {
            let id = self.entities[i].id;
            if self.ecs_get::<Identity>(id).map(|identity| is_mobile(identity.kind)).unwrap_or(false) {
                self.repath_entity_at(i);
            }
        }
        self.rehash();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ra_map::MapInfo;
    use ra_types::GameEdition;
    use std::sync::Arc;

    fn world() -> (BattleState, EntityId) {
        let rules = b"[Countries]\n0=Americans\n1=Russians\n[Americans]\nSide=GDI\n[Russians]\nSide=Nod\n\
            [VehicleTypes]\n0=MTNK\n1=HTNK\n[MTNK]\nStrength=400\nSpeed=64\n[HTNK]\nStrength=600\nSpeed=32\n";
        let definitions = Arc::new(ra_adaptor::runtime_definitions_from_ini_bytes(GameEdition::Ra2, rules, None).unwrap());
        let mut map = MapInfo::empty(GameEdition::Ra2, "projection-storage");
        map.width = 16;
        map.height = 16;
        let mut world = BattleState::new(GameEdition::Ra2, definitions, map).unwrap();
        let id = world.spawn_unit_at("Americans", "MTNK", 2, 3).unwrap();
        (world, id)
    }

    #[test]
    fn scalar_updates_and_full_sync_preserve_cached_string_and_path_storage() {
        let (mut world, id) = world();
        world
            .with_movement_mut(id, |m| {
                m.destination_x = Some(5);
                m.destination_y = Some(3);
                m.path = vec![(3, 3), (4, 3), (5, 3)];
                m.waypoints = vec![(6, 3), (7, 3)];
            })
            .unwrap();
        let owner = Arc::clone(&world.entities[0].owner);
        let type_key = Arc::clone(&world.entities[0].type_id);
        let path_storage = world.entities[0].path.as_ptr();
        for tick in 0..100 {
            world.with_animation_mut(id, |a| a.hva_frame = tick).unwrap();
            world.with_attack_mut(id, |a| a.cooldown = u32::from(tick)).unwrap();
            world.with_health_mut(id, |h| h.current = 400 - u32::from(tick)).unwrap();
            world.sync_ecs_components();
            let projection = &world.entities[0];
            assert!(Arc::ptr_eq(&projection.owner, &owner));
            assert!(Arc::ptr_eq(&projection.type_id, &type_key));
            assert_eq!(projection.path.as_ptr(), path_storage);
            assert_eq!(projection.path, vec![(3, 3), (4, 3), (5, 3)]);
        }
    }

    #[test]
    fn type_owner_and_production_changes_refresh_only_the_affected_cache() {
        let (mut world, id) = world();
        let target = world.definitions.techno.get("HTNK").unwrap().id;
        let russian = world.definitions.houses.get("Russians").unwrap().id;
        world.with_identity_mut(id, |i| i.type_id = target).unwrap();
        world.with_owner_mut(id, |o| o.house = russian).unwrap();
        world.with_production_mut(id, |q| q.item = Some((target, 20))).unwrap();
        assert_eq!(world.entities[0].type_id.as_ref(), "HTNK");
        assert_eq!(world.entities[0].owner.as_ref(), "RUSSIANS");
        let queued_key = Arc::clone(&world.entities[0].produce_queue.as_ref().unwrap().0);
        world.with_production_mut(id, |q| q.item = Some((target, 19))).unwrap();
        assert!(Arc::ptr_eq(&world.entities[0].produce_queue.as_ref().unwrap().0, &queued_key));
        assert_eq!(world.entities[0].produce_queue.as_ref().unwrap().1, 19);
        world.with_production_mut(id, |q| q.clear_production()).unwrap();
        assert!(world.entities[0].produce_queue.is_none());
    }
}
