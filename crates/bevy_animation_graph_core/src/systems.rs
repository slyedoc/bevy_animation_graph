use std::collections::VecDeque;

use bevy::{
    ecs::prelude::*,
    gizmos::gizmos::Gizmos,
    log::{info_span, warn_once},
    math::prelude::*,
    mesh::morph::MorphWeights,
    platform::collections::HashMap,
    reflect::prelude::*,
    time::prelude::*,
    transform::prelude::*,
};

use crate::{
    animated_scene::AnimatedSceneInstance,
    animation_clip::EntityPath,
    animation_graph::TimeUpdate,
    animation_graph_player::{AnimationGraphPlayer, PlaybackState},
    context::system_resources::SystemResources,
    pose::BoneId,
};

/// Component that receives root motion deltas each frame from the animation system.
///
/// Add this component to an entity with an [`AnimationGraphPlayer`], or to a parent
/// entity that has an [`AnimatedSceneInstance`] (the common case when using
/// [`AnimatedSceneHandle`]).
///
/// The built-in [`extract_root_motion`] system populates this component each frame
/// with the root motion delta converted to the entity's local space. The delta accounts
/// for the player entity's transform (armature scale and rotation).
///
/// **The system does NOT apply the delta to the entity's [`Transform`].** Your game
/// code is responsible for reading the delta and applying it however you see fit
/// (direct transform manipulation, physics impulse, character controller, etc.).
///
/// # Example
/// ```rust,ignore
/// fn apply_root_motion(
///     mut query: Query<(&RootMotionOutput, &mut Transform)>,
/// ) {
///     for (rm, mut transform) in &mut query {
///         let world_delta = transform.rotation * rm.translation_delta;
///         transform.translation += world_delta;
///         transform.rotation *= rm.rotation_delta;
///     }
/// }
/// ```
///
/// [`AnimatedSceneHandle`]: crate::animated_scene::AnimatedSceneHandle
#[derive(Component, Default, Reflect)]
pub struct RootMotionOutput {
    /// Translation displacement this frame, in the entity's local space.
    /// Accounts for the armature's scale and rotation.
    pub translation_delta: Vec3,
    /// Rotation displacement this frame.
    pub rotation_delta: Quat,
}

/// System that extracts root motion data from the animation graph player and
/// stores it in [`RootMotionOutput`], converted to the entity's local space.
///
/// Does **not** apply the delta to the entity's [`Transform`]. That is left
/// to user code so that the application strategy (direct transform, physics,
/// character controller) can be chosen freely.
///
/// Supports two layouts:
/// 1. `RootMotionOutput` on the same entity as `AnimationGraphPlayer`
/// 2. `RootMotionOutput` on a parent entity with `AnimatedSceneInstance` (the player
///    is on a child entity)
pub fn extract_root_motion(
    mut with_player: Query<(&AnimationGraphPlayer, &mut RootMotionOutput)>,
    mut with_scene: Query<
        (&AnimatedSceneInstance, &mut RootMotionOutput),
        Without<AnimationGraphPlayer>,
    >,
    players: Query<(&AnimationGraphPlayer, &Transform), Without<RootMotionOutput>>,
) {
    // Case 1: RootMotionOutput on the same entity as the player
    for (player, mut output) in &mut with_player {
        extract_delta(player, &mut output, &Transform::IDENTITY);
    }

    // Case 2: RootMotionOutput on a parent entity with AnimatedSceneInstance
    for (scene, mut output) in &mut with_scene {
        let Ok((player, player_transform)) = players.get(scene.player_entity()) else {
            continue;
        };
        extract_delta(player, &mut output, player_transform);
    }
}

fn extract_delta(
    player: &AnimationGraphPlayer,
    output: &mut RootMotionOutput,
    player_local: &Transform,
) {
    if let Some(delta) = player.get_root_motion() {
        // Convert delta from armature-local space to the output entity's local space.
        // This handles armature scale (e.g., 0.01 for Mixamo cm-authored models) and
        // armature rotation (e.g., axis conversion from FBX import).
        let converted = player_local.rotation * (delta.translation * player_local.scale);

        output.translation_delta = converted;
        output.rotation_delta = delta.rotation;
    } else {
        output.translation_delta = Vec3::ZERO;
        output.rotation_delta = Quat::IDENTITY;
    }
}

/// Whether the entity map built from `visited` no longer stands for the hierarchy under `root`:
/// never built, an entity it visited gone or unnamed (the root excepted: an unnamed root maps
/// nothing until it is named), or one whose name or children changed since the system last ran.
/// `visited` holds every entity the build walked, including a sibling whose path a later sibling
/// with the same name overwrote in the map, so the hidden one is still watched. A visited bone's
/// own parent link is checked too: when it leaves a parent as the last child, that parent's
/// `Children` is removed rather than changed. And a visited entity's children are checked for a
/// new name: an unnamed child is skipped by the build, so naming it changes nothing visited.
fn entity_map_stale(visited: &[Entity], root: Entity, resources: &SystemResources) -> bool {
    visited.first() != Some(&root)
        || visited
            .iter()
            .any(|&entity| match resources.hierarchy_changes.get(entity) {
                Ok((name, child_of, children)) => {
                    name.map_or(entity != root, |name| name.is_changed())
                        || (entity != root && child_of.is_none_or(|child_of| child_of.is_changed()))
                        || children.is_some_and(|children| {
                            children.is_changed()
                                || children.iter().any(|child| {
                                    resources.hierarchy_changes.get(child).is_ok_and(
                                        |(name, _, _)| name.is_some_and(|name| name.is_changed()),
                                    )
                                })
                        })
                }
                Err(_) => true,
            })
}

/// Maps every named entity under `root_entity` by its path from the root, and returns every
/// entity it walked (the root first) for [`entity_map_stale`]. An unnamed root maps nothing: bone
/// paths start at the root's name. Of two siblings with the same name, the later one in
/// `Children` order holds the path.
fn build_entity_map(
    root_entity: Entity,
    resources: &SystemResources,
) -> (HashMap<BoneId, Entity>, Vec<Entity>) {
    let mut entity_map = HashMap::default();
    let mut visited = vec![root_entity];

    let Ok(root_name) = resources.names_query.get(root_entity) else {
        warn_once!(
            "AnimationGraphPlayer on {root_entity} has no Name: bone paths start at the root's \
             name, so nothing is mapped until it is named"
        );
        return (entity_map, visited);
    };
    let root_path = EntityPath {
        parts: vec![root_name.clone()],
    };

    entity_map.insert(root_path.id(), root_entity);

    let mut queue: VecDeque<(Entity, EntityPath)> = VecDeque::new();

    if let Ok(root_children) = resources.children_query.get(root_entity) {
        for child in root_children {
            queue.push_back((*child, root_path.clone()));
        }
    }

    while let Some((entity, parent_path)) = queue.pop_front() {
        let Ok(name) = resources.names_query.get(entity) else {
            continue;
        };
        let path = parent_path.child(name.clone());
        entity_map.insert(path.id(), entity);
        visited.push(entity);

        if let Ok(children) = resources.children_query.get(entity) {
            for child in children {
                queue.push_back((*child, path.clone()));
            }
        }
    }

    (entity_map, visited)
}

/// System that will play all animations, using any entity with a [`AnimationGraphPlayer`]
/// and a [`Handle<AnimationClip>`] as an animation root
#[allow(clippy::too_many_arguments)]
pub fn animation_player(
    time: Res<Time>,
    mut animation_players: Query<(Entity, &mut AnimationGraphPlayer)>,
    sysres: SystemResources,
) {
    animation_players.par_iter_mut().for_each(|(root, player)| {
        run_animation_player(root, player, &time, &sysres);
    });
    animation_players.par_iter_mut().for_each(|(root, player)| {
        debug_draw_animation_players(player, root, &sysres);
    });
}

/// System that will draw deferred gizmo commands called during graph evaluation
#[allow(clippy::too_many_arguments)]
pub fn animation_player_deferred_gizmos(
    mut animation_players: Query<&mut AnimationGraphPlayer>,
    mut gizmos: Gizmos,
) {
    for mut player in &mut animation_players {
        player.deferred_gizmos.apply(&mut gizmos);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_animation_player(
    root: Entity,
    mut player: Mut<AnimationGraphPlayer>,
    time: &Time,
    system_resources: &SystemResources,
) {
    let _run_animation_player_span = info_span!("run_animation_player").entered();

    // The entity map is updated regardless of animation type.
    // Important as the entity map is relied upon for e.g. debug bone drawing
    // Rebuilt only when the hierarchy under the root changed: a full rebuild hashes every bone's
    // whole path, each tick, for every player.
    if entity_map_stale(&player.entity_map_visited, root, system_resources) {
        let _entity_map_span = info_span!("build_entity_map").entered();
        (player.entity_map, player.entity_map_visited) = build_entity_map(root, system_resources);
    }

    if !player.animation.is_graph() {
        return;
    }

    if !player.is_paused() {
        player.queue_time_update(TimeUpdate::Delta(time.delta_secs()));
    }

    {
        let _update_span = info_span!("player_update").entered();
        player.update(system_resources, root);
    }

    if matches!(player.playback_state(), PlaybackState::PlayOneFrame) {
        player.pause();
    }
}

pub fn apply_animation_to_targets(
    mut animation_targets: Query<(
        &mut Transform,
        Option<&mut MorphWeights>,
        &bevy::animation::AnimationTargetId,
        &bevy::animation::AnimatedBy,
    )>,
    graph_players: Query<&AnimationGraphPlayer>,
) {
    for (mut target_transform, target_morphs, target_id, animated_by) in &mut animation_targets {
        let target_bone_id = BoneId::from(*target_id);
        let Ok(player) = graph_players.get(animated_by.0) else {
            continue;
        };
        let Some(pose) = player.get_default_output_pose() else {
            continue;
        };

        let Some(bone_index) = pose.paths.get(&target_bone_id) else {
            continue;
        };

        let bone_pose = &pose.bones[*bone_index];
        if let Some(rotation) = bone_pose.rotation {
            target_transform.rotation = rotation;
        }
        if let Some(translation) = bone_pose.translation {
            target_transform.translation = translation;
        }
        if let Some(scale) = bone_pose.scale {
            target_transform.scale = scale;
        }
        if let Some(weights) = &bone_pose.weights
            && let Some(mut morphs) = target_morphs
        {
            apply_morph_weights(morphs.weights_mut(), weights);
        }
    }
}

pub fn debug_draw_animation_players(
    mut player: Mut<AnimationGraphPlayer>,
    root_entity: Entity,
    system_resources: &SystemResources,
) {
    player.debug_draw_bones(system_resources, root_entity);
}

/// Update `weights` based on weights in `keyframe` with a linear interpolation
/// on `key_lerp`.
fn apply_morph_weights(weights: &mut [f32], new_weights: &[f32]) {
    let zipped = weights.iter_mut().zip(new_weights);
    for (morph_weight, keyframe) in zipped {
        *morph_weight = *keyframe;
    }
}

#[cfg(test)]
mod tests {
    use bevy::{asset::Assets, ecs::system::RunSystemOnce, prelude::*};

    use super::*;
    use crate::{
        animation_clip::GraphClip, animation_graph::AnimationGraph, bone_mask_asset::BoneMaskAsset,
        skeleton::Skeleton, state_machine::high_level::StateMachine,
    };

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Assets<GraphClip>>();
        world.init_resource::<Assets<AnimationGraph>>();
        world.init_resource::<Assets<StateMachine>>();
        world.init_resource::<Assets<Skeleton>>();
        world.init_resource::<Assets<BoneMaskAsset>>();
        world
    }

    fn build(world: &mut World, root: Entity) -> (HashMap<BoneId, Entity>, Vec<Entity>) {
        world
            .run_system_once(move |resources: SystemResources| build_entity_map(root, &resources))
            .unwrap()
    }

    /// A staleness check that keeps its change ticks between calls, as the player's system does;
    /// primed once, since a new system sees everything as changed.
    fn stale_check(
        world: &mut World,
        root: Entity,
        visited: Vec<Entity>,
    ) -> impl FnMut(&mut World) -> bool + use<> {
        let system = world.register_system(move |resources: SystemResources| {
            entity_map_stale(&visited, root, &resources)
        });
        world.run_system(system).unwrap();
        move |world| world.run_system(system).unwrap()
    }

    fn path(parts: &[&'static str]) -> BoneId {
        EntityPath {
            parts: parts.iter().map(|part| Name::new(*part)).collect(),
        }
        .id()
    }

    #[test]
    fn a_sibling_hidden_by_a_namesake_is_still_watched() {
        let mut world = world();
        let root = world.spawn(Name::new("root")).id();
        let hidden = world.spawn((Name::new("arm"), ChildOf(root))).id();
        let shown = world.spawn((Name::new("arm"), ChildOf(root))).id();

        let (map, visited) = build(&mut world, root);
        assert_eq!(map.get(&path(&["root", "arm"])), Some(&shown));
        assert_eq!(visited, vec![root, hidden, shown]);

        let mut stale = stale_check(&mut world, root, visited);
        assert!(!stale(&mut world));

        world.spawn((Name::new("hand"), ChildOf(hidden)));
        assert!(stale(&mut world));
    }

    #[test]
    fn a_root_without_children_maps_itself() {
        let mut world = world();
        let root = world.spawn(Name::new("root")).id();

        let (map, visited) = build(&mut world, root);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&path(&["root"])), Some(&root));

        let mut stale = stale_check(&mut world, root, visited);
        assert!(!stale(&mut world));
        world.spawn((Name::new("hips"), ChildOf(root)));
        assert!(stale(&mut world));
    }

    #[test]
    fn an_unnamed_root_maps_nothing_until_named() {
        let mut world = world();
        let root = world.spawn_empty().id();
        world.spawn((Name::new("hips"), ChildOf(root)));

        let (map, visited) = build(&mut world, root);
        assert!(map.is_empty());
        assert_eq!(visited, vec![root]);

        let mut stale = stale_check(&mut world, root, visited);
        assert!(!stale(&mut world));
        world.entity_mut(root).insert(Name::new("root"));
        assert!(stale(&mut world));

        let (map, _) = build(&mut world, root);
        assert_eq!(map.len(), 2);
    }
}
