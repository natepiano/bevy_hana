use hana_kana::Easing;
use hana_kana::EasingSample;
use hana_kana::SequenceEasingError;
use hana_kana::SequenceEasingSample;
use thiserror::Error;

use super::FoldMemberSample;
use super::FoldTarget;
use crate::Angle;
use crate::Hinge;

/// Evaluates one member's hinge angle at a sampled sequence position.
///
/// `select_easing` receives the member's raw segment progress and answers with
/// the producer's easing decision for that position: the ECS caller forwards
/// [`SequenceEasingSampler::sample`](hana_kana::SequenceEasingSampler::sample)
/// and a value test answers with a [`SequenceEasingSample`] directly. Each
/// variant carries the position it applies to, so a curve that replaced
/// authored easing cannot also run it, and one that fed authored easing cannot
/// skip it.
/// Raw stage evaluation answers
/// [`SequenceEasingSample::AuthoredEasingSuppressed`] with the raw position.
///
/// `sample_authored_easing` supplies the eased output of the segment's own
/// authored curve; the ECS caller passes
/// [`EasingSampler`](hana_kana::EasingSampler) and a value test passes a stub.
/// It runs only for [`SequenceEasingSample::AuthoredEasingApplies`].
///
/// Interpolation runs in `f64` so finite easing overshoot and reversal survive
/// wherever the resulting angle is representable. A member resting on an
/// endpoint evaluates to that endpoint exactly.
///
/// # Errors
///
/// Returns [`FoldEvaluationError::ExternalCurveRejected`] when the producer's
/// curve cannot ease its scope,
/// [`FoldEvaluationError::NonFiniteEasing`] when either easing path produced a
/// NaN or infinite output, and
/// [`FoldEvaluationError::UnrepresentableAngle`] when the interpolated angle
/// falls outside the finite range. The caller holds its current pose in every
/// case; this layer never substitutes linear easing.
pub fn evaluate_fold_angle(
    hinge: &Hinge,
    member_sample: &FoldMemberSample<'_>,
    select_easing: impl FnOnce(f32) -> SequenceEasingSample,
    sample_authored_easing: impl FnOnce(&Easing, f32) -> EasingSample,
) -> Result<Angle, FoldEvaluationError> {
    hinge.angle_at(fold_fraction(
        member_sample,
        select_easing,
        sample_authored_easing,
    )?)
}

/// One member's eased travel toward its folded endpoint.
///
/// `0.0` holds [`Hinge::base_angle`](crate::Hinge::base_angle) and `1.0` holds
/// [`Hinge::folded_angle`](crate::Hinge::folded_angle). The value is finite but
/// deliberately unclamped: a single-stage `ReplacedBy` curve may carry eased
/// output past either endpoint, and that overshoot is the authored result
/// rather than an invalid position.
///
/// This is what evaluation caches per member, so it is distinct from
/// [`FoldSegmentProgress`](crate::FoldSegmentProgress), which is a clamped
/// position *inside* one authored segment and is never eased output.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct EasedFoldFraction(f64);

impl EasedFoldFraction {
    /// The unfolded resting endpoint.
    pub const BASE: Self = Self(0.0);
    /// The fully folded endpoint.
    pub const FOLDED: Self = Self(1.0);

    /// Creates an eased fraction from finite eased output.
    ///
    /// # Errors
    ///
    /// Returns [`FoldEvaluationError::NonFiniteEasing`] for a NaN or infinite
    /// value.
    pub const fn try_new(fraction: f64) -> Result<Self, FoldEvaluationError> {
        if fraction.is_finite() {
            Ok(Self(fraction))
        } else {
            Err(FoldEvaluationError::NonFiniteEasing)
        }
    }

    /// Returns the eased fraction.
    #[must_use]
    pub const fn value(self) -> f64 { self.0 }
}

impl Default for EasedFoldFraction {
    fn default() -> Self { Self::BASE }
}

/// Reasons one member's fold output cannot be evaluated.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum FoldEvaluationError {
    /// The producer's external curve cannot ease the scope it selected.
    #[error("the external fold easing curve was rejected: {0}")]
    ExternalCurveRejected(SequenceEasingError),
    /// An easing path produced a NaN or infinite output.
    #[error("the fold easing curve produced a non-finite output")]
    NonFiniteEasing,
    /// The interpolated hinge angle fell outside the finite range.
    #[error("the interpolated fold angle is not representable")]
    UnrepresentableAngle,
}

/// Returns the sampled position as a fraction of the member's folded endpoint.
///
/// This is the entry point retained playback evaluates once per member per
/// update and caches, and the one [`evaluate_fold_angle`] delegates to, so both
/// paths share a single implementation. `select_easing` and
/// `sample_authored_easing` mean exactly what they mean there.
///
/// # Errors
///
/// Returns the same [`FoldEvaluationError`] values [`evaluate_fold_angle`]
/// documents, except [`FoldEvaluationError::UnrepresentableAngle`], which only
/// interpolating an angle can produce.
pub fn fold_fraction(
    member_sample: &FoldMemberSample<'_>,
    select_easing: impl FnOnce(f32) -> SequenceEasingSample,
    sample_authored_easing: impl FnOnce(&Easing, f32) -> EasingSample,
) -> Result<EasedFoldFraction, FoldEvaluationError> {
    let (segment, raw_progress) = match *member_sample {
        FoldMemberSample::Unauthored => {
            return EasedFoldFraction::try_new(f64::from(FoldTarget::BASE.fraction()));
        },
        FoldMemberSample::Resting { target } => {
            return EasedFoldFraction::try_new(f64::from(target.fraction()));
        },
        FoldMemberSample::Moving {
            segment,
            raw_progress,
        } => (segment, raw_progress),
    };

    let eased = match select_easing(raw_progress.normalized()) {
        SequenceEasingSample::AuthoredEasingApplies { progress } => {
            match sample_authored_easing(&segment.timing().easing, progress) {
                EasingSample::Eased(eased) => finite_easing(eased)?,
                EasingSample::NonFinite => return Err(FoldEvaluationError::NonFiniteEasing),
            }
        },
        SequenceEasingSample::AuthoredEasingSuppressed { eased } => finite_easing(eased)?,
        SequenceEasingSample::CurveRejected(error) => {
            return Err(FoldEvaluationError::ExternalCurveRejected(error));
        },
    };
    let from = f64::from(segment.from().fraction());
    let to = f64::from(segment.to().fraction());

    EasedFoldFraction::try_new(from.mul_add(1.0 - eased, to * eased))
}

/// Widens a finite eased output to `f64` and rejects a NaN or infinite one.
fn finite_easing(eased: f32) -> Result<f64, FoldEvaluationError> {
    if eased.is_finite() {
        Ok(f64::from(eased))
    } else {
        Err(FoldEvaluationError::NonFiniteEasing)
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::time::Duration;

    use bevy_ecs::entity::Entity;
    use bevy_ecs::world::World;
    use bevy_math::curve::EaseFunction;
    use hana_kana::EasingCurve;
    use hana_kana::EasingInput;
    use hana_kana::EasingOutput;

    use super::*;
    use crate::AnchorSite;
    use crate::Displacement;
    use crate::Edge;
    use crate::FoldSequence;
    use crate::FoldSequenceBuilder;
    use crate::FoldStage;
    use crate::FoldTiming;

    const FOLDED_RADIANS: f32 = 1.0;
    const HALF_WAY: f64 = 0.5;
    const SECOND: Duration = Duration::from_secs(1);

    fn angle(radians: f32) -> Angle {
        match Angle::from_radians(radians) {
            Ok(angle) => angle,
            Err(error) => panic!("test fixture angle {radians} was rejected: {error:?}"),
        }
    }

    fn hinge(folded_radians: f32) -> Hinge {
        let edge = Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        };
        match Hinge::try_new(
            edge,
            angle(0.0),
            angle(folded_radians),
            Displacement::new(0.0, 0.0, 0.0),
        ) {
            Ok(hinge) => hinge,
            Err(error) => panic!("test fixture hinge was rejected: {error:?}"),
        }
    }

    fn folding(easing: impl Into<Easing>) -> (FoldSequence, Entity) {
        let mut world = World::new();
        let member_entity = world.spawn_empty().id();
        let sequence = FoldSequenceBuilder::new(FoldTiming::new(SECOND, easing))
            .stage(FoldStage::from(member_entity))
            .build();
        (sequence, member_entity)
    }

    fn input(value: f32) -> EasingInput {
        match EasingInput::try_new(value) {
            Ok(input) => input,
            Err(error) => panic!("test easing input {value} was rejected: {error:?}"),
        }
    }

    fn output(value: f32) -> EasingOutput {
        match EasingOutput::try_new(value) {
            Ok(output) => output,
            Err(error) => panic!("test easing output {value} was rejected: {error:?}"),
        }
    }

    /// Stands in for the stock `QuadraticIn` curve the ECS sampler would supply.
    fn quadratic(easing: &Easing, progress: f32) -> EasingSample {
        match *easing {
            Easing::Bevy(EaseFunction::QuadraticIn) => EasingSample::Eased(progress * progress),
            _ => panic!("the segment carried an unexpected easing: {easing:?}"),
        }
    }

    /// Selects the plain authored path: the segment's own curve is applied to
    /// the raw position.
    const fn authored(progress: f32) -> SequenceEasingSample {
        SequenceEasingSample::AuthoredEasingApplies { progress }
    }

    /// Selects raw stage evaluation: the raw position with no authored easing.
    const fn raw_stage(eased: f32) -> SequenceEasingSample {
        SequenceEasingSample::AuthoredEasingSuppressed { eased }
    }

    /// Stands in for a single-stage `ReplacedBy` curve that produced `eased`.
    fn replacing(eased: f32) -> impl FnOnce(f32) -> SequenceEasingSample {
        move |_| SequenceEasingSample::AuthoredEasingSuppressed { eased }
    }

    /// Stands in for a single-stage `ComposedWith` curve that produced
    /// `progress`.
    fn composing(progress: f32) -> impl FnOnce(f32) -> SequenceEasingSample {
        move |_| SequenceEasingSample::AuthoredEasingApplies { progress }
    }

    fn easing_decision(sample: SequenceEasingSample) -> impl FnOnce(f32) -> SequenceEasingSample {
        move |_| sample
    }

    fn easing_output(sample: EasingSample) -> impl FnOnce(&Easing, f32) -> EasingSample {
        move |_, _| sample
    }

    #[test]
    fn a_member_holding_a_target_evaluates_to_that_endpoint_under_both_paths() {
        let fold_hinge = hinge(FOLDED_RADIANS);
        let folded = FoldMemberSample::Resting {
            target: FoldTarget::FOLDED,
        };
        let unauthored = FoldMemberSample::Unauthored;
        let paths: [fn(f32) -> SequenceEasingSample; 2] = [authored, raw_stage];

        for select_easing in paths {
            assert_eq!(
                evaluate_fold_angle(&fold_hinge, &folded, select_easing, quadratic),
                Ok(angle(FOLDED_RADIANS))
            );
            assert_eq!(
                evaluate_fold_angle(&fold_hinge, &unauthored, select_easing, quadratic),
                Ok(angle(0.0))
            );
        }
    }

    #[test]
    fn raw_and_authored_evaluation_agree_at_a_segment_boundary() {
        let fold_hinge = hinge(FOLDED_RADIANS);
        let (sequence, member_entity) = folding(EaseFunction::QuadraticIn);
        let entering = sequence.sample_member(member_entity, 0.0);

        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &entering, authored, quadratic),
            Ok(angle(0.0))
        );
        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &entering, raw_stage, quadratic),
            Ok(angle(0.0))
        );
    }

    #[test]
    fn stock_authored_easing_shapes_the_output_and_raw_evaluation_does_not() {
        let fold_hinge = hinge(FOLDED_RADIANS);
        let (sequence, member_entity) = folding(EaseFunction::QuadraticIn);
        let midway = sequence.sample_member(member_entity, HALF_WAY);

        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &midway, authored, quadratic),
            Ok(angle(0.25))
        );
        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &midway, raw_stage, quadratic),
            Ok(angle(0.5))
        );
    }

    #[test]
    fn an_owned_lookup_curve_shapes_the_output() {
        let Ok(curve) = EasingCurve::builder()
            .linear()
            .knot(input(0.0), output(0.0))
            .knot(input(0.5), output(0.25))
            .knot(input(1.0), output(1.0))
            .try_build()
        else {
            panic!("test fixture lookup curve was rejected");
        };
        let fold_hinge = hinge(FOLDED_RADIANS);
        let (sequence, member_entity) = folding(curve.clone());
        let midway = sequence.sample_member(member_entity, HALF_WAY);
        let looked_up = |easing: &Easing, progress: f32| match easing {
            Easing::Curve(sampled) if *sampled == curve => {
                EasingSample::Eased(curve.sample(progress))
            },
            _ => panic!("the segment carried an unexpected easing: {easing:?}"),
        };

        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &midway, authored, looked_up),
            Ok(angle(curve.sample(0.5)))
        );
    }

    #[test]
    fn a_replaced_or_composed_stage_curve_leaves_the_authored_sequence_alone() {
        let fold_hinge = hinge(FOLDED_RADIANS);
        let (sequence, member_entity) = folding(EaseFunction::QuadraticIn);
        let midway = sequence.sample_member(member_entity, HALF_WAY);

        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &midway, replacing(0.25), quadratic),
            Ok(angle(0.25))
        );
        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &midway, composing(0.25), quadratic),
            Ok(angle(0.0625))
        );
    }

    #[test]
    fn finite_overshoot_and_reversal_survive_both_easing_paths() {
        let fold_hinge = hinge(FOLDED_RADIANS);
        let (sequence, member_entity) = folding(EaseFunction::QuadraticIn);
        let midway = sequence.sample_member(member_entity, HALF_WAY);

        assert_eq!(
            evaluate_fold_angle(
                &fold_hinge,
                &midway,
                authored,
                easing_output(EasingSample::Eased(1.25))
            ),
            Ok(angle(1.25))
        );
        assert_eq!(
            evaluate_fold_angle(
                &fold_hinge,
                &midway,
                authored,
                easing_output(EasingSample::Eased(-0.25))
            ),
            Ok(angle(-0.25))
        );
        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &midway, replacing(1.25), quadratic),
            Ok(angle(1.25))
        );
        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &midway, replacing(-0.25), quadratic),
            Ok(angle(-0.25))
        );
    }

    #[test]
    fn a_non_finite_curve_or_unrepresentable_angle_reports_its_own_error() {
        let fold_hinge = hinge(FOLDED_RADIANS);
        let (sequence, member_entity) = folding(EaseFunction::QuadraticIn);
        let midway = sequence.sample_member(member_entity, HALF_WAY);

        assert_eq!(
            evaluate_fold_angle(
                &fold_hinge,
                &midway,
                authored,
                easing_output(EasingSample::NonFinite)
            ),
            Err(FoldEvaluationError::NonFiniteEasing)
        );
        assert_eq!(
            evaluate_fold_angle(
                &fold_hinge,
                &midway,
                authored,
                easing_output(EasingSample::Eased(f32::NAN))
            ),
            Err(FoldEvaluationError::NonFiniteEasing)
        );
        assert_eq!(
            evaluate_fold_angle(
                &fold_hinge,
                &midway,
                easing_decision(SequenceEasingSample::CurveRejected(
                    SequenceEasingError::MappingNotBoundedMonotonic
                )),
                quadratic
            ),
            Err(FoldEvaluationError::ExternalCurveRejected(
                SequenceEasingError::MappingNotBoundedMonotonic
            ))
        );
        assert_eq!(
            evaluate_fold_angle(&fold_hinge, &midway, replacing(f32::NAN), quadratic),
            Err(FoldEvaluationError::NonFiniteEasing)
        );
        assert_eq!(
            evaluate_fold_angle(
                &hinge(f32::MAX),
                &midway,
                authored,
                easing_output(EasingSample::Eased(3.0))
            ),
            Err(FoldEvaluationError::UnrepresentableAngle)
        );
    }
}
