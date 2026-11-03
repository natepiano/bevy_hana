//! Anchored content keeps a rigid world pose beneath stretched owners.

#![allow(clippy::expect_used, reason = "test fixtures require their components")]

use bevy::app::App;
use bevy::app::PostUpdate;
use bevy::ecs::schedule::IntoScheduleConfigs as _;
use bevy::math::Quat;
use bevy::math::Vec3;
use bevy::prelude::ChildOf;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Transform;
use bevy::transform::TransformPlugin;
use bevy::transform::TransformSystems;
use hana_valence::AnchorFrame;
use hana_valence::AnchorPose;
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchorResolveSkip;
use hana_valence::AnchorSite;
use hana_valence::AnchoredTo;
use hana_valence::Displacement;
use hana_valence::Orientation;
use hana_valence::Position;
use hana_valence::ResolvedAnchorGeometry;
use hana_valence::resolve_anchors;

fn geometry() -> ResolvedAnchorGeometry {
    let frame =
        AnchorFrame::try_new(Position::from(Vec3::X), Orientation::default()).expect("valid frame");
    ResolvedAnchorGeometry::try_new([(AnchorSite::Center, frame)], []).expect("valid geometry")
}

fn fixture(
    scale: Vec3,
    parent_rotation: Quat,
    target_rotation: Quat,
) -> (App, Entity, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins(TransformPlugin)
        .init_resource::<AnchorResolveDiagnostics>()
        .add_systems(
            PostUpdate,
            resolve_anchors.before(TransformSystems::Propagate),
        );
    // Finish startup propagation before installing deliberately invalid parents.
    app.update();
    let parent_transform = Transform::from_xyz(1.0, 2.0, 3.0)
        .with_rotation(parent_rotation)
        .with_scale(scale);
    let parent_global = GlobalTransform::from(parent_transform);
    let parent = app
        .world_mut()
        .spawn((parent_transform, parent_global))
        .id();
    let target_transform = Transform::from_xyz(-2.0, 3.0, 5.0).with_rotation(target_rotation);
    let target = app
        .world_mut()
        .spawn((
            geometry(),
            target_transform,
            GlobalTransform::from(target_transform),
        ))
        .id();
    let source = app
        .world_mut()
        .spawn((
            geometry(),
            Transform::IDENTITY,
            parent_global,
            ChildOf(parent),
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
        ))
        .id();
    (app, parent, target, source)
}

fn assert_rigid_matches(app: &App, source: Entity, expected: GlobalTransform) {
    assert!(
        app.world()
            .resource::<AnchorResolveDiagnostics>()
            .current()
            .next()
            .is_none(),
        "stretched owners must resolve"
    );
    let actual = app
        .world()
        .get::<GlobalTransform>(source)
        .expect("source global");
    for point in [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z] {
        assert!(
            (actual.transform_point(point) - expected.transform_point(point)).length() < 1e-4,
            "actual {actual:?}, expected {expected:?}"
        );
    }
    let axes = [Vec3::X, Vec3::Y, Vec3::Z].map(|axis| actual.affine().transform_vector3(axis));
    for axis in axes {
        assert!((axis.length() - 2.0).abs() < 1e-4, "uniform world scale");
    }
    assert!(axes[0].dot(axes[1]).abs() < 1e-4);
    assert!(axes[0].dot(axes[2]).abs() < 1e-4);
    assert!(axes[1].dot(axes[2]).abs() < 1e-4);
}

#[test]
fn stretched_parent_places_a_rigid_source_matching_a_uniform_parent() {
    for (parent_rotation, target_rotation) in [
        (Quat::IDENTITY, Quat::IDENTITY),
        (Quat::from_rotation_y(0.7), Quat::from_rotation_y(0.7)),
        (
            Quat::from_rotation_y(0.7),
            Quat::from_rotation_y(0.7) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        ),
    ] {
        let (mut reference, _, _, reference_source) =
            fixture(Vec3::splat(2.0), parent_rotation, target_rotation);
        reference.update();
        let expected = *reference
            .world()
            .get::<GlobalTransform>(reference_source)
            .expect("reference");
        let (mut app, parent, _, source) =
            fixture(Vec3::new(2.0, 3.0, 4.0), parent_rotation, target_rotation);
        app.update();
        assert_rigid_matches(&app, source, expected);
        assert_eq!(
            app.world().get::<ChildOf>(source).map(ChildOf::parent),
            Some(parent)
        );
        app.update();
        assert_rigid_matches(&app, source, expected);
        // Ordinary hierarchy ownership is preserved.
        app.world_mut().despawn(parent);
        assert!(
            app.world().get_entity(source).is_err(),
            "owner despawns its source"
        );
    }
}

#[test]
fn stretched_parent_pose_changes_keep_the_source_rigid() {
    let (mut app, _, _, source) = fixture(Vec3::new(2.0, 3.0, 4.0), Quat::IDENTITY, Quat::IDENTITY);
    app.update();
    app.update();
    app.world_mut().entity_mut(source).insert(AnchorPose {
        translation: Displacement::from(Vec3::Y),
        rotation:    Orientation::try_from(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
            .expect("rotation"),
    });
    app.update();
    let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let expected = GlobalTransform::from(Transform {
        translation: Vec3::new(-1.0, 4.0, 5.0) - rotation * (2.0 * Vec3::X),
        rotation,
        scale: Vec3::splat(2.0),
    });
    assert_rigid_matches(&app, source, expected);
    app.update();
    assert_rigid_matches(&app, source, expected);
    app.world_mut().entity_mut(source).insert(AnchorPose {
        translation: Displacement::from(Vec3::Y * 2.0),
        rotation:    Orientation::try_from(rotation).expect("rotation"),
    });
    app.update();
    let moved = GlobalTransform::from(Transform {
        translation: expected.translation() + Vec3::Y,
        rotation,
        scale: Vec3::splat(2.0),
    });
    assert_rigid_matches(&app, source, moved);
}

fn assert_unsupported_parent(app: &App, parent: Entity, target: Entity, source: Entity) {
    assert!(
        app.world()
            .resource::<AnchorResolveDiagnostics>()
            .current()
            .any(|entry| entry.source == source
                && entry.target == target
                && entry.reason == AnchorResolveSkip::UnsupportedParentTransform),
        "a local pose requiring shear must skip"
    );
    assert_eq!(
        app.world().get::<ChildOf>(source).map(ChildOf::parent),
        Some(parent)
    );
    assert_eq!(
        app.world().get::<Transform>(source),
        Some(&Transform::IDENTITY)
    );
}

#[test]
fn nonaligned_rotation_skips_without_changing_parent_or_local_transform() {
    let (mut app, parent, target, source) = fixture(
        Vec3::new(2.0, 3.0, 4.0),
        Quat::from_rotation_y(0.7),
        Quat::from_rotation_z(0.4),
    );
    app.update();
    assert_unsupported_parent(&app, parent, target, source);
    app.update();
    assert_unsupported_parent(&app, parent, target, source);
}

#[test]
fn sheared_parent_skips_without_changing_parent_or_local_transform() {
    let rotation = Quat::from_rotation_y(0.5);
    let (mut app, parent, target, source) = fixture(Vec3::splat(2.0), Quat::IDENTITY, rotation);
    let stretch = Transform::from_scale(Vec3::new(2.0, 3.0, 4.0));
    let grandparent = app
        .world_mut()
        .spawn((stretch, GlobalTransform::from(stretch)))
        .id();
    let local = Transform::from_rotation(Quat::from_rotation_z(0.4));
    let sheared = GlobalTransform::from(stretch).mul_transform(local);
    app.world_mut()
        .entity_mut(parent)
        .insert((local, sheared, ChildOf(grandparent)));
    app.update();
    assert_unsupported_parent(&app, parent, target, source);
    app.update();
    assert_unsupported_parent(&app, parent, target, source);
    app.world_mut().despawn(grandparent);
    assert!(app.world().get_entity(source).is_err());
}

#[test]
fn degenerate_parent_still_skips() {
    for scale in [
        Vec3::new(2.0, 0.0, 4.0),
        Vec3::splat(f32::INFINITY),
        Vec3::splat(f32::NAN),
    ] {
        let (mut app, _, target, source) = fixture(scale, Quat::IDENTITY, Quat::IDENTITY);
        // Keep the source valid so this tests the parent diagnostic itself.
        app.world_mut()
            .entity_mut(source)
            .insert(GlobalTransform::IDENTITY);
        app.update();
        assert!(
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .any(|entry| entry.source == source
                    && entry.target == target
                    && entry.reason == AnchorResolveSkip::UnsupportedParentTransform)
        );
    }
}
