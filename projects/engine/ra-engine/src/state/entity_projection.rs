//! 按组件更新兼容投影，避免一次标量写入复制整份实体。

use std::sync::Arc;

use ra_ecs::{EcsEntity, EcsWorld};
use ra_types::RuntimeDefinitions;

use super::{components::*, ecs_registry::EcsRegistry, entities::WorldEntity};

/// 更新组件在实体投影中对应的字段，不读取或复制其他组件。
pub(super) trait ProjectComponent: ra_ecs::Component {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage;
    fn project(&self, definitions: &RuntimeDefinitions, entity: &mut WorldEntity);
}

fn copy_key(target: &mut Arc<str>, key: &str) {
    if target.as_ref() != key {
        *target = Arc::from(key);
    }
}

impl ProjectComponent for Identity {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsIdentity;
    fn project(&self, definitions: &RuntimeDefinitions, entity: &mut WorldEntity) {
        if let Some(definition) = definitions.techno.get_by_id(self.type_id) {
            copy_key(&mut entity.type_id, definition.type_key.as_str());
        }
        entity.kind = self.kind;
    }
}

impl ProjectComponent for Owner {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsOwner;
    fn project(&self, definitions: &RuntimeDefinitions, entity: &mut WorldEntity) {
        if let Some(definition) = definitions.houses.get_by_id(self.house) {
            copy_key(&mut entity.owner, definition.type_key.as_str());
        }
    }
}

impl ProjectComponent for Health {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsHealth;
    fn project(&self, _: &RuntimeDefinitions, entity: &mut WorldEntity) {
        entity.health = self.current;
        entity.max_health = self.maximum;
        entity.dead = self.dead;
    }
}

impl ProjectComponent for Transform {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsTransform;
    fn project(&self, _: &RuntimeDefinitions, entity: &mut WorldEntity) {
        entity.x = self.x;
        entity.y = self.y;
        entity.facing = self.facing;
        entity.turret_facing = self.turret_facing;
        entity.sub_cell = self.sub_cell;
    }
}

impl ProjectComponent for Locomotor {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsLocomotor;
    fn project(&self, _: &RuntimeDefinitions, entity: &mut WorldEntity) {
        entity.speed = self.speed;
    }
}

impl ProjectComponent for MovementState {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsMovement;
    fn project(&self, _: &RuntimeDefinitions, entity: &mut WorldEntity) {
        entity.target_x = self.destination_x;
        entity.target_y = self.destination_y;
        entity.path.clone_from(&self.path);
        entity.move_accum = self.move_accum;
    }
}

impl ProjectComponent for CombatStats {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsCombatStats;
    fn project(&self, _: &RuntimeDefinitions, entity: &mut WorldEntity) {
        entity.armor = self.armor;
        entity.attack_range = self.attack_range;
        entity.attack_damage = self.attack_damage;
        entity.attack_cooldown_max = self.attack_cooldown_max;
        entity.attack_verses = self.attack_verses;
        entity.techno_class = self.techno_class;
    }
}

impl ProjectComponent for AttackState {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsAttack;
    fn project(&self, _: &RuntimeDefinitions, entity: &mut WorldEntity) {
        entity.attack_target = self.target;
        entity.attack_cooldown = self.cooldown;
    }
}

impl ProjectComponent for ProductionQueue {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsProduction;
    fn project(&self, definitions: &RuntimeDefinitions, entity: &mut WorldEntity) {
        match self.item.and_then(|(id, remaining)| definitions.techno.get_by_id(id).map(|t| (t.type_key.as_str(), remaining))) {
            Some((key, remaining)) => {
                if let Some((current_key, current_remaining)) = &mut entity.produce_queue {
                    copy_key(current_key, key);
                    *current_remaining = remaining;
                }
                else {
                    entity.produce_queue = Some((Arc::from(key), remaining));
                }
            }
            None => entity.produce_queue = None,
        }
        entity.rally_x = self.rally_x;
        entity.rally_y = self.rally_y;
    }
}

impl ProjectComponent for HarvesterState {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsHarvester;
    fn project(&self, _: &RuntimeDefinitions, entity: &mut WorldEntity) {
        entity.ore_trip_accum = self.ore_trip_accum;
    }
}

impl ProjectComponent for AnimationState {
    #[cfg(feature = "profiling")]
    const PROFILE_STAGE: crate::profiling::Stage = crate::profiling::Stage::EcsAnimation;
    fn project(&self, _: &RuntimeDefinitions, entity: &mut WorldEntity) {
        entity.hva_frame = self.hva_frame;
        entity.hit_flash = self.hit_flash;
        entity.fire_flash = self.fire_flash;
    }
}

pub(super) fn project_all(world: &EcsWorld, handle: EcsEntity, definitions: &RuntimeDefinitions, entity: &mut WorldEntity) {
    macro_rules! project {
        ($($component:ty),*) => { $(
            if let Some(component) = world.get::<$component>(handle) {
                component.project(definitions, entity);
            }
        )* };
    }
    project!(
        Identity,
        Owner,
        Health,
        Transform,
        Locomotor,
        MovementState,
        CombatStats,
        AttackState,
        ProductionQueue,
        HarvesterState,
        AnimationState
    );
}

pub(super) fn project_entities(registry: &EcsRegistry, definitions: &RuntimeDefinitions, entities: &mut [WorldEntity]) {
    macro_rules! views {
        ($($name:ident: $component:ty),*) => {
            $(let $name = registry.world().view::<$component>();)*
            for entity in entities {
                if let Some(handle) = registry.resolve(entity.id) {
                    $(if let Some(component) = $name.get(handle) {
                        component.project(definitions, entity);
                    })*
                }
            }
        };
    }
    views!(identity: Identity, owner: Owner, health: Health, transform: Transform, locomotor: Locomotor,
        movement: MovementState, combat: CombatStats, attack: AttackState, production: ProductionQueue,
        harvester: HarvesterState, animation: AnimationState);
}
