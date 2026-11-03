use bevy::camera::primitives::Aabb;
use bevy::mesh::Mesh3d;
use bevy::platform::collections::HashSet;
#[cfg(test)]
use bevy::prelude::App;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
#[cfg(test)]
use bevy::prelude::GlobalTransform;
use bevy::prelude::In;
use bevy::prelude::Or;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::RemovedComponents;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
#[cfg(test)]
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::warn;
use hana_valence::AnchorFrame;
use hana_valence::AnchorSite;
use hana_valence::AnchoredHere;
use hana_valence::Edge;
use hana_valence::Orientation;
use hana_valence::Position;
use hana_valence::ResolvedAnchorGeometry;

use super::entity::TooltipTargetEntity;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::layout::Anchor;
use crate::panel::World;
use crate::widgets::constants::QUAD_ANCHOR_COUNT;
use crate::widgets::constants::QUAD_EDGE_COUNT;

/// Axis-aligned face selected as a rectangular anchor provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeshFace {
    /// Face with a positive local X normal.
    PositiveX,
    /// Face with a negative local X normal.
    NegativeX,
    /// Face with a positive local Y normal.
    PositiveY,
    /// Face with a negative local Y normal.
    NegativeY,
    /// Face with a positive local Z normal.
    PositiveZ,
    /// Face with a negative local Z normal.
    NegativeZ,
}

/// Extension methods for checked general mesh anchor authoring.
pub trait MeshAnchorCommandsExt {
    /// Authors a persistent mesh-face anchor target.
    fn mesh_anchor_target(&mut self, entity: Entity, face: MeshFace) -> TooltipTargetEntity<World>;
}

impl MeshAnchorCommandsExt for Commands<'_, '_> {
    fn mesh_anchor_target(&mut self, entity: Entity, face: MeshFace) -> TooltipTargetEntity<World> {
        self.run_system_cached_with(author_mesh_anchor_target, (entity, face));
        TooltipTargetEntity::from_validated(entity)
    }
}

#[derive(Clone, Copy, Component)]
pub(crate) struct MeshAnchorTarget {
    face: MeshFace,
}

#[derive(Clone, Copy, Component)]
pub(in crate::widgets) struct MeshAnchorGeometry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MeshAnchorPendingCause {
    WaitingForMesh,
    WaitingForBounds,
    UnusableBounds,
}

#[derive(Clone, Copy, Component)]
pub(in crate::widgets) struct MeshAnchorGeometryPending {
    cause: MeshAnchorPendingCause,
}

#[derive(Default, Resource)]
pub(crate) struct MeshAnchorWarnings(HashSet<Entity>);

fn author_mesh_anchor_target(
    In((entity, face)): In<(Entity, MeshFace)>,
    meshes: Query<(), With<Mesh3d>>,
    mut commands: Commands,
) {
    if meshes.contains(entity) {
        commands.entity(entity).insert(MeshAnchorTarget { face });
    }
}

pub(in crate::widgets) fn update_mesh_anchor_geometry(
    changed_targets: Query<
        Entity,
        (
            With<MeshAnchorTarget>,
            With<AnchoredHere>,
            Or<(
                Changed<AnchoredHere>,
                Changed<MeshAnchorTarget>,
                Changed<Mesh3d>,
                Changed<Aabb>,
            )>,
        ),
    >,
    demanded_targets: Query<
        (
            Entity,
            Ref<MeshAnchorTarget>,
            Option<Ref<Mesh3d>>,
            Option<Ref<Aabb>>,
            Option<&MeshAnchorGeometry>,
            Option<&MeshAnchorGeometryPending>,
        ),
        (With<MeshAnchorTarget>, With<AnchoredHere>),
    >,
    mut removed_meshes: RemovedComponents<Mesh3d>,
    mut removed_bounds: RemovedComponents<Aabb>,
    mut warnings: ResMut<MeshAnchorWarnings>,
    mut commands: Commands,
) {
    let mut candidates = changed_targets.iter().collect::<HashSet<_>>();
    candidates.extend(removed_meshes.read());
    candidates.extend(removed_bounds.read());

    for entity in candidates {
        let Ok((_, target, mesh, aabb, geometry, pending)) = demanded_targets.get(entity) else {
            continue;
        };
        if mesh.is_none() {
            mark_mesh_anchor_pending(
                entity,
                MeshAnchorPendingCause::WaitingForMesh,
                &mut commands,
            );
            continue;
        }
        let Some(aabb) = aabb else {
            mark_mesh_anchor_pending(
                entity,
                MeshAnchorPendingCause::WaitingForBounds,
                &mut commands,
            );
            continue;
        };
        if geometry.is_none() || pending.is_some() || target.is_changed() || aabb.is_changed() {
            write_mesh_anchor_geometry(entity, target.face, *aabb, &mut warnings, &mut commands);
        }
    }
}

fn mark_mesh_anchor_pending(
    entity: Entity,
    cause: MeshAnchorPendingCause,
    commands: &mut Commands,
) {
    commands
        .entity(entity)
        .remove::<(ResolvedAnchorGeometry, MeshAnchorGeometry)>()
        .insert(MeshAnchorGeometryPending { cause });
}

fn write_mesh_anchor_geometry(
    entity: Entity,
    face: MeshFace,
    aabb: Aabb,
    warnings: &mut MeshAnchorWarnings,
    commands: &mut Commands,
) {
    let Some(geometry) = mesh_face_geometry(aabb, face) else {
        mark_mesh_anchor_pending(entity, MeshAnchorPendingCause::UnusableBounds, commands);
        return;
    };
    warnings.0.remove(&entity);
    commands
        .entity(entity)
        .remove::<MeshAnchorGeometryPending>()
        .insert((geometry, MeshAnchorGeometry));
}

pub(in crate::widgets) fn warn_pending_mesh_anchor_geometry(
    targets: Query<
        (Entity, &MeshAnchorTarget, &MeshAnchorGeometryPending),
        (With<AnchoredHere>, Changed<MeshAnchorGeometryPending>),
    >,
    mut warnings: ResMut<MeshAnchorWarnings>,
) {
    for (entity, target, pending) in &targets {
        if warnings.0.insert(entity) {
            match pending.cause {
                MeshAnchorPendingCause::WaitingForMesh => {
                    warn!(target: DIAGNOSTIC_LOG_TARGET, "mesh anchor target {entity:?} is waiting for Mesh3d");
                },
                MeshAnchorPendingCause::WaitingForBounds => {
                    warn!(target: DIAGNOSTIC_LOG_TARGET, "mesh anchor target {entity:?} is waiting for Aabb");
                },
                MeshAnchorPendingCause::UnusableBounds => {
                    warn!(
                        target: DIAGNOSTIC_LOG_TARGET, "mesh anchor target {entity:?} has no usable bounds for face {:?}",
                        target.face
                    );
                },
            }
        }
    }
}

pub(in crate::widgets) fn remove_mesh_anchor_geometry(
    targets: Query<
        (
            Entity,
            Option<&MeshAnchorGeometry>,
            Option<&MeshAnchorGeometryPending>,
        ),
        (With<MeshAnchorTarget>, Without<AnchoredHere>),
    >,
    mut warnings: ResMut<MeshAnchorWarnings>,
    mut commands: Commands,
) {
    for (entity, geometry, pending) in &targets {
        warnings.0.remove(&entity);
        if geometry.is_some() || pending.is_some() {
            commands.entity(entity).remove::<(
                ResolvedAnchorGeometry,
                MeshAnchorGeometry,
                MeshAnchorGeometryPending,
            )>();
        }
    }
}

pub(in crate::widgets::tooltip) fn mesh_face_geometry(
    aabb: Aabb,
    face: MeshFace,
) -> Option<ResolvedAnchorGeometry> {
    let center = Vec3::from(aabb.center);
    let half_extents = Vec3::from(aabb.half_extents);
    if !center.is_finite() || !half_extents.is_finite() {
        return None;
    }
    let frame = mesh_face_frame(face);
    let right = frame * Vec3::X;
    let up = frame * Vec3::Y;
    let normal = frame * Vec3::Z;
    let right_extent = right.abs().dot(half_extents);
    let up_extent = up.abs().dot(half_extents);
    let normal_extent = normal.abs().dot(half_extents);
    if right_extent <= 0.0 || up_extent <= 0.0 {
        return None;
    }
    let face_center = center + normal * normal_extent;
    let mut frames = Vec::with_capacity(QUAD_ANCHOR_COUNT);
    let orientation = Orientation::try_from(frame).ok()?;
    for anchor in [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::BottomLeft,
        Anchor::TopCenter,
        Anchor::CenterRight,
        Anchor::BottomCenter,
        Anchor::CenterLeft,
        Anchor::Center,
    ] {
        let (x_fraction, y_fraction) = anchor.offset_fraction();
        let x = x_fraction.mul_add(2.0, -1.0) * right_extent;
        let y = y_fraction.mul_add(-2.0, 1.0) * up_extent;
        let anchor_frame = AnchorFrame::try_new(
            Position::from(face_center + right * x + up * y),
            orientation,
        )
        .ok()?;
        frames.push((AnchorSite::from(anchor), anchor_frame));
    }
    let edges = vec![
        Edge {
            start: AnchorSite::from(Anchor::TopLeft),
            end:   AnchorSite::from(Anchor::TopRight),
        },
        Edge {
            start: AnchorSite::from(Anchor::TopRight),
            end:   AnchorSite::from(Anchor::BottomRight),
        },
        Edge {
            start: AnchorSite::from(Anchor::BottomRight),
            end:   AnchorSite::from(Anchor::BottomLeft),
        },
        Edge {
            start: AnchorSite::from(Anchor::BottomLeft),
            end:   AnchorSite::from(Anchor::TopLeft),
        },
    ];
    debug_assert_eq!(edges.len(), QUAD_EDGE_COUNT);
    ResolvedAnchorGeometry::try_new(frames, edges).ok()
}

fn mesh_face_frame(face: MeshFace) -> Quat {
    match face {
        MeshFace::PositiveX => Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        MeshFace::NegativeX => Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
        MeshFace::PositiveY => Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        MeshFace::NegativeY => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        MeshFace::PositiveZ => Quat::IDENTITY,
        MeshFace::NegativeZ => Quat::from_rotation_y(std::f32::consts::PI),
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Vec3A;
    use hana_valence::AnchoredTo;

    use super::*;
    use crate::HeadlessLayoutPlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;

    const GEOMETRY_EPSILON: f32 = 1e-4;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn center_anchor_geometry() -> ResolvedAnchorGeometry {
        let frame = AnchorFrame::try_new(Position::from(Vec3::ZERO), Orientation::default());
        assert!(
            frame.is_ok(),
            "the center anchor uses finite identity geometry"
        );
        let geometry = ResolvedAnchorGeometry::try_new(
            [(AnchorSite::Center, frame.unwrap_or_default())],
            Vec::<Edge>::new(),
        );
        assert!(
            geometry.is_ok(),
            "a single center anchor has no invalid edges"
        );
        geometry.unwrap_or_default()
    }

    fn demand_mesh_target(app: &mut App, target: Entity) -> Entity {
        app.world_mut()
            .spawn((
                center_anchor_geometry(),
                Transform::default(),
                GlobalTransform::default(),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            ))
            .id()
    }

    fn settle_mesh_anchor_geometry(app: &mut App) {
        app.update();
        app.update();
    }

    fn mesh_anchor_center(app: &App, target: Entity) -> Option<Vec3> {
        app.world()
            .get::<ResolvedAnchorGeometry>(target)
            .and_then(|geometry| geometry.frame(AnchorSite::Center).ok())
            .map(|point| point.position().into_inner())
    }

    #[track_caller]
    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, GEOMETRY_EPSILON),
            "expected {expected:?}, got {actual:?}",
        );
    }

    #[track_caller]
    fn assert_quat_close(actual: Quat, expected: Quat) {
        assert!(
            actual.abs_diff_eq(expected, GEOMETRY_EPSILON)
                || actual.abs_diff_eq(-expected, GEOMETRY_EPSILON),
            "expected {expected:?}, got {actual:?}",
        );
    }

    fn assert_positive_z_mesh_geometry(geometry: &ResolvedAnchorGeometry) {
        assert_eq!(geometry.frames().count(), QUAD_ANCHOR_COUNT);
        for (anchor, expected) in [
            (Anchor::TopLeft, Vec3::new(-1.0, 2.0, 3.0)),
            (Anchor::TopRight, Vec3::new(1.0, 2.0, 3.0)),
            (Anchor::BottomRight, Vec3::new(1.0, -2.0, 3.0)),
            (Anchor::BottomLeft, Vec3::new(-1.0, -2.0, 3.0)),
            (Anchor::TopCenter, Vec3::new(0.0, 2.0, 3.0)),
            (Anchor::CenterRight, Vec3::new(1.0, 0.0, 3.0)),
            (Anchor::BottomCenter, Vec3::new(0.0, -2.0, 3.0)),
            (Anchor::CenterLeft, Vec3::new(-1.0, 0.0, 3.0)),
            (Anchor::Center, Vec3::new(0.0, 0.0, 3.0)),
        ] {
            let point = geometry.frame(AnchorSite::from(anchor));
            assert!(
                point.is_ok(),
                "mesh geometry contains every named quad anchor"
            );
            let Some(point) = point.ok() else {
                continue;
            };
            assert_vec3_close(point.position().into_inner(), expected);
            assert_quat_close(point.orientation().into_inner(), Quat::IDENTITY);
        }
        assert_eq!(
            geometry.edges(),
            &[
                Edge {
                    start: AnchorSite::from(Anchor::TopLeft),
                    end:   AnchorSite::from(Anchor::TopRight),
                },
                Edge {
                    start: AnchorSite::from(Anchor::TopRight),
                    end:   AnchorSite::from(Anchor::BottomRight),
                },
                Edge {
                    start: AnchorSite::from(Anchor::BottomRight),
                    end:   AnchorSite::from(Anchor::BottomLeft),
                },
                Edge {
                    start: AnchorSite::from(Anchor::BottomLeft),
                    end:   AnchorSite::from(Anchor::TopLeft),
                },
            ]
        );
    }

    #[test]
    fn mesh_face_provider_tracks_bounds_without_transform_rebuilds() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        let source = app
            .world_mut()
            .spawn((
                center_anchor_geometry(),
                Transform::default(),
                GlobalTransform::default(),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            ))
            .id();
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());

        app.world_mut().entity_mut(target).insert(Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::new(1.0, 2.0, 3.0),
        });
        app.update();
        let geometry = app.world().get::<ResolvedAnchorGeometry>(target);
        assert!(geometry.is_some());
        let Some(geometry) = geometry else {
            return;
        };
        assert_positive_z_mesh_geometry(geometry);
        assert_eq!(
            app.world()
                .get::<Transform>(source)
                .map(|transform| transform.translation),
            Some(Vec3::new(0.0, 0.0, 3.0))
        );
        let before_tick = app
            .world()
            .entity(target)
            .get_ref::<ResolvedAnchorGeometry>()
            .map(|geometry| geometry.last_changed());
        assert!(before_tick.is_some());

        let translated = Transform::from_translation(Vec3::X);
        app.world_mut()
            .entity_mut(target)
            .insert((translated, GlobalTransform::from(translated)));
        app.update();
        let after_transform_tick = app
            .world()
            .entity(target)
            .get_ref::<ResolvedAnchorGeometry>()
            .map(|geometry| geometry.last_changed());
        assert_eq!(after_transform_tick, before_tick);

        app.world_mut().entity_mut(target).insert(Aabb {
            center:       Vec3A::new(0.0, 0.0, 1.75),
            half_extents: Vec3A::splat(0.25),
        });
        app.update();
        assert_eq!(
            app.world()
                .get::<ResolvedAnchorGeometry>(target)
                .and_then(|geometry| geometry.frame(AnchorSite::Center).ok())
                .map(|point| point.position().into_inner()),
            Some(Vec3::new(0.0, 0.0, 2.0))
        );
        assert_eq!(
            app.world()
                .get::<Transform>(source)
                .map(|transform| transform.translation),
            Some(Vec3::new(1.0, 0.0, 2.0))
        );

        app.world_mut().despawn(source);
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
    }

    #[test]
    fn transform_only_changes_do_not_rebuild_mesh_anchor_geometry() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        demand_mesh_target(&mut app, target);
        settle_mesh_anchor_geometry(&mut app);
        let before = app
            .world()
            .entity(target)
            .get_ref::<ResolvedAnchorGeometry>()
            .map(|geometry| geometry.last_changed());
        assert!(before.is_some());

        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_translation(Vec3::X));
        app.update();

        let after = app
            .world()
            .entity(target)
            .get_ref::<ResolvedAnchorGeometry>()
            .map(|geometry| geometry.last_changed());
        assert_eq!(after, before);
    }

    #[test]
    fn replacing_attachment_keeps_mesh_target_geometry() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        let source = demand_mesh_target(&mut app, target);
        settle_mesh_anchor_geometry(&mut app);
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_some());

        app.world_mut().entity_mut(source).insert(AnchoredTo::new(
            target,
            AnchorSite::Center,
            AnchorSite::Center,
        ));
        app.update();
        assert!(app.world().get::<AnchoredHere>(target).is_some());
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_some());
    }

    #[test]
    fn mesh_component_removal_retires_geometry_until_readdition_recovers() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        demand_mesh_target(&mut app, target);
        settle_mesh_anchor_geometry(&mut app);
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );

        app.world_mut().entity_mut(target).remove::<Mesh3d>();
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
        assert!(app.world().get::<MeshAnchorGeometry>(target).is_none());
        assert_eq!(
            app.world()
                .get::<MeshAnchorGeometryPending>(target)
                .map(|pending| pending.cause),
            Some(MeshAnchorPendingCause::WaitingForMesh)
        );

        app.world_mut().entity_mut(target).insert(Mesh3d::default());
        app.update();
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn mesh_component_removed_before_first_demand_does_not_publish_retained_bounds() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        app.update();
        assert!(app.world().get::<Aabb>(target).is_some());

        app.world_mut().entity_mut(target).remove::<Mesh3d>();
        app.update();
        assert!(app.world().get::<Aabb>(target).is_some());
        demand_mesh_target(&mut app, target);
        app.update();

        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
        assert!(app.world().get::<MeshAnchorGeometry>(target).is_none());
        assert_eq!(
            app.world()
                .get::<MeshAnchorGeometryPending>(target)
                .map(|pending| pending.cause),
            Some(MeshAnchorPendingCause::WaitingForMesh)
        );
    }

    #[test]
    fn bounds_removal_retires_geometry_until_current_bounds_return() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        let source = app
            .world_mut()
            .spawn((
                center_anchor_geometry(),
                Transform::default(),
                GlobalTransform::default(),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            ))
            .id();
        settle_mesh_anchor_geometry(&mut app);
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );

        app.world_mut().entity_mut(target).remove::<Aabb>();
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
        assert!(app.world().get::<MeshAnchorGeometry>(target).is_none());

        app.world_mut().entity_mut(target).insert(Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::splat(3.0),
        });
        app.update();
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 3.0))
        );
        assert_eq!(
            app.world()
                .get::<Transform>(source)
                .map(|transform| transform.translation),
            Some(Vec3::new(0.0, 0.0, 3.0))
        );
    }

    #[test]
    fn selected_face_change_refreshes_local_geometry() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::new(1.0, 2.0, 3.0),
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        demand_mesh_target(&mut app, target);
        settle_mesh_anchor_geometry(&mut app);
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 3.0))
        );

        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveX);
        app.update();

        assert!(
            mesh_anchor_center(&app, target)
                .is_some_and(|center| center.abs_diff_eq(Vec3::X, 1e-5))
        );
    }
}
