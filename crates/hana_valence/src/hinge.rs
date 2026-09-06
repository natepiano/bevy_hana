//! Resting fold endpoints and their conversion into an anchor pose.

use bevy_app::App;
use bevy_app::Plugin;
use bevy_app::PostUpdate;
use bevy_ecs::change_detection::DetectChanges;
use bevy_ecs::change_detection::DetectChangesMut;
use bevy_ecs::change_detection::Mut;
use bevy_ecs::entity::Entity;
use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::Component;
use bevy_ecs::prelude::ReflectComponent;
use bevy_ecs::prelude::Resource;
use bevy_ecs::query::Has;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::system::Commands;
use bevy_ecs::system::Local;
use bevy_ecs::system::Query;
use bevy_ecs::system::SystemChangeTick;
use bevy_ecs::world::DeferredWorld;
use bevy_math::Dir3;
use bevy_math::Quat;
use bevy_reflect::Reflect;
use hana_kana::Angle;
use hana_kana::Displacement;
use hana_kana::Orientation;
use hana_kana::ToF32;
use thiserror::Error;

use crate::AnchorPose;
use crate::AnchorSite;
use crate::AnchorSystems;
use crate::AnchoredTo;
use crate::EasedFoldFraction;
use crate::Edge;
use crate::FoldEvaluationError;
use crate::FoldFractionScratch;
use crate::FoldMemberFraction;
use crate::FoldSequencePlayback;
use crate::GeometryError;
use crate::ResolvedAnchorGeometry;

/// Proof that [`HingePlugin`] installed the hinge driver.
///
/// A [`Hinge`] is inert without that driver: the component sits on the entity,
/// the resolver never sees a fold, and nothing warns. A pintle is the pin a
/// hinge turns on, and [`Hinge::require_driver`] demands one of every hinge
/// entering a world, so a hinge cannot reach a world holding nothing to move
/// it.
///
/// The resource is crate-private, so an application cannot file the proof by
/// hand; the private field, the crate-private constructor, and
/// `#[reflect(opaque)]` close the literal and the dynamic reflected tuple as
/// well. [`HingePlugin`] is the sole issuer, which is what makes the hook the
/// single place a missing driver is ever reported.
#[derive(Resource, Clone, Copy, Debug, Reflect)]
#[reflect(opaque)]
pub(crate) struct Pintle(());

impl Pintle {
    /// Creates the proof that the hinge driver is registered.
    pub(crate) const fn installed() -> Self { Self(()) }
}

/// Registers the hinge driver, without which no [`Hinge`] may enter the world.
///
/// [`crate::ArrangementPlugin`] adds this plugin, so an application that builds
/// arrangements needs nothing further. Add it directly when hinges are authored
/// by hand: it requires no assets and no scene support, so it composes into a
/// headless app that has neither.
///
/// This is the sole registrar of the hinge driver, which it places in
/// [`AnchorSystems::HingeToPose`] nested inside [`AnchorSystems::AnimatePose`].
/// The driver itself is private: order against the set instead.
#[derive(Default)]
pub struct HingePlugin;

impl Plugin for HingePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Pintle::installed())
            .configure_sets(
                PostUpdate,
                AnchorSystems::HingeToPose.in_set(AnchorSystems::AnimatePose),
            )
            .add_systems(PostUpdate, hinge_to_pose.in_set(AnchorSystems::HingeToPose));
    }
}

/// The two resting fold endpoints of one arrangement connection.
///
/// A hinge stores where its member rests when the fold is fully unfolded
/// ([`Self::base_angle`]) and where it rests when the fold is complete
/// ([`Self::folded_angle`]), never a mutable current angle. Playback position
/// supplies everything between them, so nothing outside this crate has to keep
/// a second copy of the live angle in sync.
///
/// [`Self::pivot_offset`] is the source-anchor-frame displacement from the
/// hinge axis to the physical pivot the member must turn about. It is
/// calibrated against `base_angle`: the pose translation is zero at the base
/// endpoint and grows with the angular delta away from it.
///
/// Fields are private and reflection is opaque, because [`Self::try_new`] is
/// the only place a degenerate edge or a non-finite pivot is rejected.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, PartialEq, Debug, Clone, opaque)]
#[require(AnchorPose)]
#[component(on_add = Self::require_driver)]
pub struct Hinge {
    edge:         Edge,
    base_angle:   Angle,
    folded_angle: Angle,
    pivot_offset: Displacement,
}

impl Hinge {
    /// Creates a hinge from its authored edge, resting endpoints, and pivot.
    ///
    /// A hinge that never reaches a world moves nothing and harms nothing, so
    /// construction asks only whether the authored values describe a hinge.
    /// Whether anything will actually pose it is settled where that becomes
    /// answerable — when the component is inserted, by an `on_add` hook that
    /// rejects a world [`HingePlugin`] never reached.
    ///
    /// Both [`Angle`] endpoints are finite by construction, so only the edge
    /// and the pivot displacement are validated here.
    ///
    /// # Errors
    ///
    /// Returns [`HingeError::DegenerateEdge`] when `edge` names the same
    /// anchor site twice, or [`HingeError::NonFinitePivotOffset`] when
    /// `pivot_offset` has a NaN or infinite coordinate.
    pub fn try_new(
        edge: Edge,
        base_angle: Angle,
        folded_angle: Angle,
        pivot_offset: Displacement,
    ) -> Result<Self, HingeError> {
        if edge.start == edge.end {
            return Err(HingeError::DegenerateEdge { edge });
        }
        if !pivot_offset.is_finite() {
            return Err(HingeError::NonFinitePivotOffset { pivot_offset });
        }

        Ok(Self {
            edge,
            base_angle,
            folded_angle,
            pivot_offset,
        })
    }

    /// Rejects a hinge entering a world that holds nothing to pose it.
    ///
    /// Every route a hinge takes into a world ends in this insertion —
    /// [`Self::try_new`] followed by an insert, [`Self::resting`] through
    /// arrangement materialization, a scene, a clone — so this is the one
    /// place a missing [`HingePlugin`] is reported, and it names the entity
    /// that would otherwise have carried a pose nobody ever wrote.
    ///
    /// # Panics
    ///
    /// Panics when [`Pintle`] is absent, which happens only when the
    /// application added no [`HingePlugin`]. That is a wiring mistake fixed
    /// once at startup, never a runtime condition an application can recover
    /// from, and the alternative is the silence this hook exists to end.
    fn require_driver(world: DeferredWorld<'_>, context: HookContext) {
        assert!(
            world.get_resource::<Pintle>().is_some(),
            "entity {} received a Hinge in an app with no HingePlugin, so the \
             hinge driver is unregistered and this hinge would hold an \
             unwritten pose forever. Add hana_valence::HingePlugin, or \
             ArrangementPlugin, which adds it.",
            context.entity,
        );
    }

    /// Creates the unfolded hinge one validated plan connection starts from.
    ///
    /// Both endpoints are the connection's resting angle and the pivot offset
    /// is zero, so a freshly materialized arrangement is unfolded and every
    /// later fold recipe replaces a known base endpoint. Neither check in
    /// [`Self::try_new`] can reject these inputs:
    /// [`crate::ArrangementPlan::try_new`] already rejects a connection whose
    /// member edge names one anchor site twice, and the zero pivot is finite.
    pub(crate) const fn resting(edge: Edge, base_angle: Angle) -> Self {
        Self {
            edge,
            base_angle,
            folded_angle: base_angle,
            pivot_offset: Displacement::new(0.0, 0.0, 0.0),
        }
    }

    /// Returns the ordered source-local edge this hinge turns about.
    #[must_use]
    pub const fn edge(&self) -> Edge { self.edge }

    /// Returns the resting angle at the unfolded endpoint.
    #[must_use]
    pub const fn base_angle(&self) -> Angle { self.base_angle }

    /// Returns the resting angle at the folded endpoint.
    #[must_use]
    pub const fn folded_angle(&self) -> Angle { self.folded_angle }

    /// Returns the source-anchor-frame displacement to the physical pivot.
    #[must_use]
    pub const fn pivot_offset(&self) -> Displacement { self.pivot_offset }

    /// Replaces the folded endpoint and the pivot, keeping the authored edge
    /// and `base_angle`.
    pub(crate) const fn refolded(
        mut self,
        folded_angle: Angle,
        pivot_offset: Displacement,
    ) -> Self {
        self.folded_angle = folded_angle;
        self.pivot_offset = pivot_offset;
        self
    }

    /// Returns whether both resting endpoints are the same angle.
    pub(crate) fn rests_at_base(&self) -> bool { self.base_angle == self.folded_angle }

    /// Interpolates the two resting endpoints at an eased fold fraction.
    ///
    /// Interpolation runs in `f64` so finite easing overshoot and reversal
    /// survive wherever the resulting angle is representable. This is the sole
    /// place a fold fraction becomes an angle: both
    /// [`evaluate_fold_angle`](crate::evaluate_fold_angle) and
    /// [`hinge_to_pose`] reach it.
    ///
    /// # Errors
    ///
    /// Returns [`FoldEvaluationError::UnrepresentableAngle`] when the
    /// interpolated angle falls outside the finite range.
    pub(crate) fn angle_at(
        &self,
        fraction: EasedFoldFraction,
    ) -> Result<Angle, FoldEvaluationError> {
        let fraction = fraction.value();
        let base = f64::from(self.base_angle.radians());
        let folded = f64::from(self.folded_angle.radians());
        Angle::from_radians(base.mul_add(1.0 - fraction, folded * fraction).to_f32())
            .map_err(|_| FoldEvaluationError::UnrepresentableAngle)
    }

    fn pose(
        &self,
        current_angle: Angle,
        attachment: Option<&AnchoredTo>,
        geometry: &ResolvedAnchorGeometry,
    ) -> Result<AnchorPose, HingePoseSkip> {
        let attachment = attachment.ok_or(HingePoseSkip::MissingRelationship)?;
        let axis = self
            .edge
            .axis(geometry)
            .map_err(HingePoseSkip::UnavailableEdgeAxis)?;
        let source_frame = geometry
            .frame(attachment.source_anchor())
            .map_err(|_| HingePoseSkip::MissingSourceAnchor(attachment.source_anchor()))?
            .orientation();
        let axis_in_source_frame = Dir3::new(source_frame.inverse() * *axis)
            .map_err(|_| HingePoseSkip::InvalidSourceFrame)?;
        let rotation = Quat::from_axis_angle(*axis_in_source_frame, current_angle.radians());
        let delta = current_angle.radians() - self.base_angle.radians();
        let pivot = self.pivot_offset.into_inner();
        let translation = pivot - Quat::from_axis_angle(*axis_in_source_frame, delta) * pivot;
        let rotation =
            Orientation::try_from(rotation).map_err(|_| HingePoseSkip::NonFiniteInput)?;
        if !translation.is_finite() {
            return Err(HingePoseSkip::NonFiniteInput);
        }

        Ok(AnchorPose {
            rotation,
            translation: Displacement::from(translation),
        })
    }
}

/// Invalid input supplied to [`Hinge::try_new`].
#[derive(Clone, Copy, Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum HingeError {
    /// The authored edge used one anchor site as both endpoints.
    #[error("hinge edge {edge:?} uses the same anchor site twice")]
    DegenerateEdge {
        /// Ordered edge whose endpoints were equal.
        edge: Edge,
    },
    /// The pivot displacement had a NaN or infinite coordinate.
    #[error("hinge pivot offset {pivot_offset:?} is not finite")]
    NonFinitePivotOffset {
        /// Displacement whose coordinates were not finite.
        pivot_offset: Displacement,
    },
}

/// Why one hinge left its existing [`AnchorPose`] and authoring unchanged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum HingePoseSkip {
    MissingRelationship,
    MissingSourceAnchor(AnchorSite),
    MissingSourceGeometry,
    UnavailableEdgeAxis(GeometryError),
    InvalidSourceFrame,
    NonFiniteInput,
    UnrepresentableAngle,
}

impl From<FoldEvaluationError> for HingePoseSkip {
    fn from(_: FoldEvaluationError) -> Self { Self::UnrepresentableAngle }
}

/// Marks a hinge whose unavailable pose the hinge driver already reported.
///
/// The library inserts this on the offending entity and removes it once that
/// entity poses successfully, which is how one authoring mistake warns once.
/// Keeping the flag on the entity rather than in a system-local set means a
/// despawn drops it along with the entity, so an application that respawns
/// misauthored hinges accumulates nothing per spawn. Applications may query it
/// to list the hinges currently holding a stale pose; they should not insert or
/// remove it.
#[derive(Component, Clone, Copy, Debug)]
pub struct HingePoseReported;

/// Writes each hinge's current rotation and pivot compensation into [`AnchorPose`].
///
/// [`HingePlugin`] is this system's sole registrar; it runs in
/// [`AnchorSystems::HingeToPose`], nested inside
/// [`AnchorSystems::AnimatePose`], before [`crate::resolve_anchors`].
///
/// A hinge whose entity is tracked by a retained [`FoldSequencePlayback`] takes
/// its angle from the eased fraction that playback cached for it in `Update`.
/// A hinge no sequence stages rests at [`Hinge::base_angle`], and a staged one
/// whose fraction `Update` could not resolve keeps the pose it already has.
/// Because the angle is derived here from the live [`Hinge`], replacing that
/// component in `PostUpdate` before this system changes the pose in the same
/// frame.
///
/// Unavailable geometry, a missing relationship or source site, an unusable
/// edge axis, an unrepresentable angle, and a non-finite runtime value each
/// preserve the previous pose and every authored component, so repairing the
/// ECS state resumes evaluation on the next run.
///
/// Unfilled geometry is an ordinary not-ready state and stays silent. The
/// remaining states name an authoring mistake, so each warns once per entity
/// and warns again only after that entity has posed successfully in between.
///
/// Fold-evaluation failures are not reported here: `Update` owns every
/// [`FoldEvaluationError`], so a held fold leaves its cached state unchanged
/// and this system either recomputes the same pose from the last resolved
/// fraction or, with none resolved yet, writes no pose at all.
fn hinge_to_pose(
    mut commands: Commands,
    system_tick: SystemChangeTick,
    mut fractions: Local<FoldFractionScratch>,
    playbacks: Query<&FoldSequencePlayback>,
    mut hinges: Query<(
        Entity,
        &Hinge,
        Option<&AnchoredTo>,
        Option<&ResolvedAnchorGeometry>,
        &mut AnchorPose,
        Has<HingePoseReported>,
    )>,
) {
    fractions.reload(&playbacks);
    for (entity, hinge, attachment, geometry, mut pose, reported) in &mut hinges {
        let fraction = match fractions.fraction(entity) {
            // A staged member with no fraction resolved yet holds every pose it
            // already has, including the one it entered the frame with.
            FoldMemberFraction::Unresolved => continue,
            FoldMemberFraction::Untracked => EasedFoldFraction::BASE,
            FoldMemberFraction::Eased(fraction) => fraction,
        };
        let next_pose = hinge
            .angle_at(fraction)
            .map_err(HingePoseSkip::from)
            .and_then(|current_angle| {
                geometry
                    .ok_or(HingePoseSkip::MissingSourceGeometry)
                    .and_then(|geometry| hinge.pose(current_angle, attachment, geometry))
            });
        let next_pose = match next_pose {
            Ok(next_pose) => next_pose,
            Err(HingePoseSkip::MissingSourceGeometry) => continue,
            Err(skipped) => {
                if !reported {
                    tracing::warn!(entity = ?entity, error = ?skipped, "hinge pose unavailable");
                    commands.entity(entity).insert(HingePoseReported);
                }
                continue;
            },
        };
        if reported {
            commands.entity(entity).remove::<HingePoseReported>();
        }
        let overwrites_same_frame_write = pose_changed_this_frame(&pose, system_tick);
        if pose.set_if_neq(next_pose) && overwrites_same_frame_write {
            warn_of_overwritten_pose(entity);
        }
    }
}

/// Returns whether something else wrote this pose earlier in the current frame.
fn pose_changed_this_frame(pose: &Mut<AnchorPose>, system_tick: SystemChangeTick) -> bool {
    let last_run = system_tick.last_run();
    let this_run = system_tick.this_run();

    pose.last_changed().is_newer_than(last_run, this_run) && pose.last_changed() != pose.added()
}

/// Reports one same-frame pose write this system replaced.
fn warn_of_overwritten_pose(entity: Entity) {
    #[cfg(debug_assertions)]
    tracing::warn!(
        entity = ?entity,
        "hinge overwrote an AnchorPose changed earlier this frame"
    );
    #[cfg(not(debug_assertions))]
    {
        let _ = entity;
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use core::fmt::Debug;

    use bevy_ecs::entity::Entity;
    use bevy_ecs::prelude::Component;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use bevy_ecs::schedule::Schedule;
    use bevy_ecs::world::World;
    use bevy_math::Quat;
    use bevy_math::Vec3;
    use bevy_reflect::PartialReflect;
    use bevy_reflect::ReflectRef;
    use bevy_transform::prelude::GlobalTransform;
    use bevy_transform::prelude::Transform;
    use hana_kana::Angle;
    use hana_kana::Displacement;
    use hana_kana::Orientation;

    use super::Hinge;
    use super::HingeError;
    use super::Pintle;
    use super::hinge_to_pose;
    use crate::AnchorFrame;
    use crate::AnchorPose;
    use crate::AnchorSite;
    use crate::AnchorSystems;
    use crate::AnchoredTo;
    use crate::Edge;
    use crate::Position;
    use crate::ResolvedAnchorGeometry;
    use crate::resolve;

    const ASSERT_EPSILON: f32 = 1e-4;
    const BASE_ANGLE: f32 = core::f32::consts::FRAC_PI_4;
    const FOLD_ANGLE: f32 = core::f32::consts::FRAC_PI_2;
    const PIVOT_OFFSET: Vec3 = Vec3::new(0.0, 0.0, 0.25);
    const UNCHANGED_POSE_TRANSLATION: Vec3 = Vec3::new(1.0, 2.0, 3.0);

    #[derive(Component)]
    struct Unhinged;

    /// Unwraps a fixture, naming the rejected input and the error on failure.
    fn valid<T, E: Debug>(input: impl Debug, result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("test fixture {input:?} was rejected: {error:?}"),
        }
    }

    fn angle(radians: f32) -> Angle { valid(radians, Angle::from_radians(radians)) }

    const fn top_edge() -> Edge {
        Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        }
    }

    const fn reversed_top_edge() -> Edge {
        Edge {
            start: AnchorSite::Vertex(1),
            end:   AnchorSite::Vertex(0),
        }
    }

    fn hinge(edge: Edge, base: f32, pivot: Vec3) -> Hinge {
        valid(
            (edge, base, pivot),
            Hinge::try_new(edge, angle(base), angle(base), Displacement::from(pivot)),
        )
    }

    #[test]
    fn construction_rejects_a_degenerate_edge_and_a_non_finite_pivot() {
        let same_site_edge = Edge {
            start: AnchorSite::Center,
            end:   AnchorSite::Center,
        };

        assert_eq!(
            Hinge::try_new(
                same_site_edge,
                angle(0.0),
                angle(FOLD_ANGLE),
                Displacement::default(),
            ),
            Err(HingeError::DegenerateEdge {
                edge: same_site_edge,
            }),
        );
        // A NaN coordinate never equals itself, so the reported pivot is
        // matched rather than compared.
        assert!(matches!(
            Hinge::try_new(
                top_edge(),
                angle(0.0),
                angle(FOLD_ANGLE),
                Displacement::new(f32::NAN, 0.0, 0.0),
            ),
            Err(HingeError::NonFinitePivotOffset { pivot_offset }) if pivot_offset.into_inner().x.is_nan()
        ));
    }

    #[test]
    fn accessors_return_the_authored_endpoints_and_reflection_stays_opaque() {
        let authored = valid(
            (top_edge(), BASE_ANGLE, FOLD_ANGLE),
            Hinge::try_new(
                top_edge(),
                angle(BASE_ANGLE),
                angle(FOLD_ANGLE),
                Displacement::from(PIVOT_OFFSET),
            ),
        );

        assert_eq!(authored.edge(), top_edge());
        assert_eq!(authored.base_angle(), angle(BASE_ANGLE));
        assert_eq!(authored.folded_angle(), angle(FOLD_ANGLE));
        assert_eq!(authored.pivot_offset(), Displacement::from(PIVOT_OFFSET));
        assert!(matches!(authored.reflect_ref(), ReflectRef::Opaque(_)));
    }

    #[test]
    fn a_nonzero_base_angle_poses_without_a_fold_sequence() {
        let mut world = hinge_world();
        let entity = spawn_hinged(&mut world, hinge(top_edge(), BASE_ANGLE, Vec3::ZERO));

        run_hinge_driver(&mut world);

        assert_pose(
            &world,
            entity,
            Quat::from_rotation_x(BASE_ANGLE),
            Vec3::ZERO,
        );
    }

    #[test]
    fn endpoint_order_flips_the_rotation_sense() {
        let mut world = hinge_world();
        let forward = spawn_hinged(&mut world, hinge(top_edge(), FOLD_ANGLE, Vec3::ZERO));
        let reversed = spawn_hinged(
            &mut world,
            hinge(reversed_top_edge(), FOLD_ANGLE, Vec3::ZERO),
        );

        run_hinge_driver(&mut world);

        assert_pose(
            &world,
            forward,
            Quat::from_rotation_x(FOLD_ANGLE),
            Vec3::ZERO,
        );
        assert_pose(
            &world,
            reversed,
            Quat::from_rotation_x(-FOLD_ANGLE),
            Vec3::ZERO,
        );
    }

    #[test]
    fn pivot_compensation_is_zero_while_the_member_rests_at_base() {
        let mut world = hinge_world();
        let away_from_base = hinge(top_edge(), BASE_ANGLE, PIVOT_OFFSET)
            .refolded(angle(FOLD_ANGLE), Displacement::from(PIVOT_OFFSET));
        let at_base = spawn_hinged(&mut world, hinge(top_edge(), BASE_ANGLE, PIVOT_OFFSET));
        let folded = spawn_hinged(&mut world, away_from_base);

        run_hinge_driver(&mut world);

        assert_pose(
            &world,
            at_base,
            Quat::from_rotation_x(BASE_ANGLE),
            Vec3::ZERO,
        );
        // Without a fold sequence the member still rests at its base endpoint,
        // so a different folded endpoint changes neither rotation nor pivot.
        assert_pose(
            &world,
            folded,
            Quat::from_rotation_x(BASE_ANGLE),
            Vec3::ZERO,
        );
    }

    #[test]
    fn the_edge_axis_is_converted_into_the_source_anchor_frame() {
        let mut world = hinge_world();
        let target = resolve::spawn_quad(&mut world, Transform::default());
        let unframed = spawn_framed_hinge(&mut world, target, Quat::IDENTITY);
        let framed = spawn_framed_hinge(&mut world, target, Quat::from_rotation_z(FOLD_ANGLE));

        run_hinge_driver(&mut world);

        // The authored edge runs from Vertex(0) to Vertex(1), along +Y.
        assert_pose(
            &world,
            unframed,
            Quat::from_axis_angle(Vec3::Y, BASE_ANGLE),
            Vec3::ZERO,
        );
        // A source frame turned a quarter turn about +Z carries that edge onto +X.
        assert_pose(
            &world,
            framed,
            Quat::from_axis_angle(Vec3::X, BASE_ANGLE),
            Vec3::ZERO,
        );
    }

    #[test]
    fn unavailable_state_preserves_pose_and_authoring_then_resumes_on_repair() {
        let mut world = hinge_world();
        let target = resolve::spawn_quad(&mut world, Transform::default());
        let authored = hinge(top_edge(), FOLD_ANGLE, PIVOT_OFFSET);

        let without_relationship = resolve::spawn_quad(&mut world, Transform::default());
        world
            .entity_mut(without_relationship)
            .insert((unchanged_pose(), authored));

        let without_source_site = resolve::spawn_quad(&mut world, Transform::default());
        world.entity_mut(without_source_site).insert((
            AnchoredTo::new(
                target,
                AnchorSite::EdgeMidpoint(u32::MAX),
                AnchorSite::Center,
            ),
            unchanged_pose(),
            authored,
        ));

        let without_geometry = world
            .spawn((
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
                unchanged_pose(),
                authored,
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();

        let without_edge = world
            .spawn((
                ResolvedAnchorGeometry::default(),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
                unchanged_pose(),
                authored,
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();

        run_hinge_driver(&mut world);

        for entity in [
            without_relationship,
            without_source_site,
            without_geometry,
            without_edge,
        ] {
            assert_eq!(pose(&world, entity), unchanged_pose());
            assert_eq!(world.get::<Hinge>(entity), Some(&authored));
        }

        world
            .entity_mut(without_relationship)
            .insert(AnchoredTo::new(
                target,
                AnchorSite::Center,
                AnchorSite::Center,
            ));

        run_hinge_driver(&mut world);

        assert_pose(
            &world,
            without_relationship,
            Quat::from_rotation_x(FOLD_ANGLE),
            Vec3::ZERO,
        );
    }

    #[test]
    fn an_entity_without_a_hinge_receives_no_pose_write() {
        let mut world = hinge_world();
        let unhinged = resolve::spawn_quad(&mut world, Transform::default());
        world
            .entity_mut(unhinged)
            .insert((unchanged_pose(), Unhinged));

        run_hinge_driver(&mut world);

        assert_eq!(pose(&world, unhinged), unchanged_pose());
    }

    /// A world already carrying the pintle [`HingePlugin`] would have issued,
    /// so these tests insert hinges without building an `App`.
    fn hinge_world() -> World {
        let mut world = resolve::world_with_diagnostics();
        world.insert_resource(Pintle::installed());
        world
    }

    fn spawn_hinged(world: &mut World, hinge: Hinge) -> Entity {
        let target = resolve::spawn_quad(world, Transform::default());
        let source = resolve::spawn_quad(world, Transform::default());
        world.entity_mut(source).insert((
            AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            AnchorPose::default(),
            hinge,
        ));
        source
    }

    fn run_hinge_driver(world: &mut World) {
        let mut schedule = Schedule::default();
        schedule.add_systems(hinge_to_pose.in_set(AnchorSystems::AnimatePose));
        schedule.run(world);
    }

    fn spawn_framed_hinge(world: &mut World, target: Entity, source_frame: Quat) -> Entity {
        world
            .spawn((
                framed_hinge_geometry(source_frame),
                Transform::default(),
                GlobalTransform::default(),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
                AnchorPose::default(),
                hinge(top_edge(), BASE_ANGLE, PIVOT_OFFSET),
            ))
            .id()
    }

    fn framed_hinge_geometry(source_frame: Quat) -> ResolvedAnchorGeometry {
        valid(
            source_frame,
            ResolvedAnchorGeometry::try_new(
                [
                    (AnchorSite::Center, oriented_frame(Vec3::ZERO, source_frame)),
                    (AnchorSite::Vertex(0), frame(Vec3::NEG_Y)),
                    (AnchorSite::Vertex(1), frame(Vec3::Y)),
                ],
                [top_edge()],
            ),
        )
    }

    fn frame(position: Vec3) -> AnchorFrame {
        valid(
            position,
            AnchorFrame::try_new(Position::from(position), Orientation::default()),
        )
    }

    fn oriented_frame(position: Vec3, orientation: Quat) -> AnchorFrame {
        valid(
            (position, orientation),
            AnchorFrame::try_new(
                Position::from(position),
                valid(orientation, Orientation::try_from(orientation)),
            ),
        )
    }

    fn unchanged_pose() -> AnchorPose {
        let rotation = Quat::from_rotation_y(FOLD_ANGLE);

        AnchorPose {
            rotation:    valid(rotation, Orientation::try_from(rotation)),
            translation: Displacement::from(UNCHANGED_POSE_TRANSLATION),
        }
    }

    fn pose(world: &World, entity: Entity) -> AnchorPose {
        world.get::<AnchorPose>(entity).copied().unwrap_or_default()
    }

    fn assert_pose(world: &World, entity: Entity, rotation: Quat, translation: Vec3) {
        let pose = pose(world, entity);
        assert_close_quat(pose.rotation.into_inner(), rotation);
        assert_close_vec3(pose.translation.into_inner(), translation);
    }

    fn assert_close_vec3(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() <= ASSERT_EPSILON,
            "actual {actual:?}, expected {expected:?}",
        );
    }

    fn assert_close_quat(actual: Quat, expected: Quat) {
        assert!(
            actual.dot(expected).abs() >= 1.0 - ASSERT_EPSILON,
            "actual {actual:?}, expected {expected:?}",
        );
    }
}
