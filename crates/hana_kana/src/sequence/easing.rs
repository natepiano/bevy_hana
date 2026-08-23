use bevy::reflect::Reflect;
use thiserror::Error;

use super::stages::SequenceScope;
use crate::easing::Easing;
use crate::easing::EasingMapping;
use crate::easing::EasingSample;
use crate::easing::EasingSampler;

/// How a producer's external curve relates to the domain's authored stage
/// easing.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub enum SequenceEasing {
    /// Only the domain's authored stage easing applies.
    #[default]
    Authored,
    /// The external curve replaces authored stage easing over the scope.
    ReplacedBy(Easing),
    /// The external curve feeds authored stage easing over the scope.
    ComposedWith(Easing),
}

impl SequenceEasing {
    /// Returns the external curve, when the producer publishes one.
    #[must_use]
    pub const fn curve(&self) -> SequenceEasingCurve<'_> {
        match self {
            Self::Authored => SequenceEasingCurve::AuthoredOnly,
            Self::ReplacedBy(easing) => SequenceEasingCurve::Replacing(easing),
            Self::ComposedWith(easing) => SequenceEasingCurve::Composing(easing),
        }
    }
}

/// The external curve a [`SequenceEasing`] publishes and what it does to
/// authored stage easing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequenceEasingCurve<'a> {
    /// The producer publishes no external curve.
    AuthoredOnly,
    /// This curve replaces authored stage easing.
    Replacing(&'a Easing),
    /// This curve feeds authored stage easing.
    Composing(&'a Easing),
}

/// Result of sampling a producer's [`SequenceEasing`] at one scope-relative
/// input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequenceEasingSample {
    /// The domain applies its own authored stage easing to this input. Either
    /// the producer publishes no external curve, or its curve composed with
    /// authored easing and produced this input.
    AuthoredEasingApplies {
        /// Scope-relative input for authored stage easing.
        progress: f32,
    },
    /// The external curve replaced authored stage easing and produced this
    /// output directly.
    AuthoredEasingSuppressed {
        /// Final eased output for the scope.
        eased: f32,
    },
    /// The external curve is not usable at this scope.
    CurveRejected(SequenceEasingError),
}

/// Reasons an external curve cannot ease a scope.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq, Reflect)]
#[reflect(opaque)]
pub enum SequenceEasingError {
    /// The scope spans more than one stage, so the curve must map the unit
    /// interval onto itself without reversing or overshooting.
    #[error(
        "a curve easing more than one stage must be bounded and monotonic, but it \
         overshoots or reverses"
    )]
    MappingNotBoundedMonotonic,
    /// The curve produced a NaN or infinite output for this input.
    #[error("the curve produced a non-finite output")]
    NonFiniteOutput,
}

/// Samples a producer's [`SequenceEasing`] without world or asset access.
#[derive(Clone, Copy, Debug, Default)]
pub struct SequenceEasingSampler;

impl SequenceEasingSampler {
    /// Samples `easing` at scope-relative `progress`.
    #[must_use]
    pub fn sample(
        &self,
        scope: SequenceScope,
        easing: &SequenceEasing,
        progress: f32,
    ) -> SequenceEasingSample {
        let (curve, authored_easing) = match easing.curve() {
            SequenceEasingCurve::AuthoredOnly => {
                return SequenceEasingSample::AuthoredEasingApplies { progress };
            },
            SequenceEasingCurve::Replacing(curve) => (curve, AuthoredEasing::Suppressed),
            SequenceEasingCurve::Composing(curve) => (curve, AuthoredEasing::Applies),
        };

        if scope_remaps_progress(scope) {
            match EasingSampler.mapping(curve) {
                EasingMapping::BoundedMonotonic => {},
                EasingMapping::OvershootingOrReversing => {
                    return SequenceEasingSample::CurveRejected(
                        SequenceEasingError::MappingNotBoundedMonotonic,
                    );
                },
                EasingMapping::NonFiniteOutput => {
                    return SequenceEasingSample::CurveRejected(
                        SequenceEasingError::NonFiniteOutput,
                    );
                },
            }
        }

        match EasingSampler.sample(curve, progress) {
            EasingSample::Eased(eased) => match authored_easing {
                AuthoredEasing::Suppressed => {
                    SequenceEasingSample::AuthoredEasingSuppressed { eased }
                },
                AuthoredEasing::Applies => {
                    SequenceEasingSample::AuthoredEasingApplies { progress: eased }
                },
            },
            EasingSample::NonFinite => {
                SequenceEasingSample::CurveRejected(SequenceEasingError::NonFiniteOutput)
            },
        }
    }
}

/// Whether the domain's authored stage easing still runs after the external
/// curve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthoredEasing {
    Applies,
    Suppressed,
}

const fn scope_remaps_progress(scope: SequenceScope) -> bool {
    match scope {
        SequenceScope::Stage(_) => false,
        SequenceScope::WholeSequence | SequenceScope::StageRange { .. } => true,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::math::curve::EaseFunction;

    use super::*;
    use crate::easing::EasingCurve;
    use crate::easing::EasingInput;
    use crate::easing::EasingOutput;
    use crate::sequence::stages::SequenceStages;

    const HALFWAY: f32 = 0.5;

    fn input(value: f32) -> EasingInput {
        EasingInput::try_new(value).expect("test input is normalized")
    }

    fn output(value: f32) -> EasingOutput {
        EasingOutput::try_new(value).expect("test output is finite")
    }

    fn sample(
        scope: SequenceScope,
        easing: &SequenceEasing,
        progress: f32,
    ) -> SequenceEasingSample {
        SequenceEasingSampler.sample(scope, easing, progress)
    }

    fn one_stage_scope() -> SequenceScope {
        let stages = SequenceStages::new([Duration::from_secs(1), Duration::from_secs(1)]);
        SequenceScope::Stage(
            stages
                .stage_id(0)
                .expect("a two stage description has an ordinal zero"),
        )
    }

    #[test]
    fn authored_easing_passes_progress_through_unchanged() {
        assert_eq!(
            sample(
                SequenceScope::WholeSequence,
                &SequenceEasing::Authored,
                HALFWAY
            ),
            SequenceEasingSample::AuthoredEasingApplies { progress: HALFWAY }
        );
    }

    #[test]
    fn a_replacing_curve_suppresses_authored_easing() {
        assert_eq!(
            sample(
                SequenceScope::WholeSequence,
                &SequenceEasing::ReplacedBy(Easing::Bevy(EaseFunction::Linear)),
                HALFWAY,
            ),
            SequenceEasingSample::AuthoredEasingSuppressed { eased: HALFWAY }
        );
    }

    #[test]
    fn a_composing_curve_feeds_authored_easing() {
        assert_eq!(
            sample(
                SequenceScope::WholeSequence,
                &SequenceEasing::ComposedWith(Easing::Bevy(EaseFunction::QuadraticIn)),
                HALFWAY,
            ),
            SequenceEasingSample::AuthoredEasingApplies {
                progress: HALFWAY * HALFWAY,
            }
        );
    }

    #[test]
    fn a_multi_stage_scope_rejects_an_overshooting_curve() {
        assert_eq!(
            sample(
                SequenceScope::WholeSequence,
                &SequenceEasing::ReplacedBy(Easing::Bevy(EaseFunction::BackInOut)),
                HALFWAY,
            ),
            SequenceEasingSample::CurveRejected(SequenceEasingError::MappingNotBoundedMonotonic)
        );
    }

    #[test]
    fn a_one_stage_scope_accepts_an_overshooting_curve() {
        let sampled = sample(
            one_stage_scope(),
            &SequenceEasing::ReplacedBy(Easing::Bevy(EaseFunction::BackInOut)),
            0.25,
        );

        let SequenceEasingSample::AuthoredEasingSuppressed { eased } = sampled else {
            panic!("an anticipating curve eases a single stage");
        };
        assert!(eased < 0.0, "BackInOut anticipates below zero at 0.25");
    }

    #[test]
    fn a_non_finite_curve_reports_non_finite_output_rather_than_overshoot() {
        assert_eq!(
            sample(
                SequenceScope::WholeSequence,
                &SequenceEasing::ReplacedBy(Easing::Bevy(EaseFunction::Elastic(f32::NAN))),
                HALFWAY,
            ),
            SequenceEasingSample::CurveRejected(SequenceEasingError::NonFiniteOutput)
        );
    }

    #[test]
    fn an_owned_lookup_curve_eases_a_multi_stage_scope() {
        let easing_curve = EasingCurve::builder()
            .linear()
            .knot(input(0.0), output(0.0))
            .knot(input(1.0), output(1.0))
            .try_build()
            .expect("a two knot linear ramp is valid");

        assert_eq!(
            sample(
                SequenceScope::WholeSequence,
                &SequenceEasing::ComposedWith(Easing::Curve(easing_curve)),
                HALFWAY,
            ),
            SequenceEasingSample::AuthoredEasingApplies { progress: HALFWAY }
        );
    }
}
