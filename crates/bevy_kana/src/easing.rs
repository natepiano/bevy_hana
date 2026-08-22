use bevy::app::App;
use bevy::app::Plugin;
use bevy::math::Vec2;
use bevy::math::curve::Curve;
use bevy::math::curve::EaseFunction;
use bevy::reflect::Reflect;
use bevy_lookup_curve::Knot;
use bevy_lookup_curve::KnotInterpolation;
use bevy_lookup_curve::LookupCurve;
use bevy_lookup_curve::Tangent;
use thiserror::Error;

/// Registers shared easing reflection types.
///
/// Every type this module defines is non-generic and uses Bevy's automatic
/// reflection registration. The plugin remains repeatable for consumer plugin
/// composition, but owns no assets, resources, or runtime state.
pub struct EasingPlugin;

impl Plugin for EasingPlugin {
    fn build(&self, _: &mut App) {}

    fn is_unique(&self) -> bool { false }
}

/// A finite normalized coordinate at which an [`EasingCurve`] is sampled.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Reflect)]
#[reflect(opaque)]
pub struct EasingInput(f32);

impl EasingInput {
    /// Creates a finite coordinate in the normalized easing interval.
    ///
    /// # Errors
    ///
    /// Returns [`EasingCurveError::NonFiniteInput`] for NaN or infinity and
    /// [`EasingCurveError::InputOutOfRange`] outside `0.0..=1.0`.
    pub const fn try_new(input: f32) -> Result<Self, EasingCurveError> {
        if !input.is_finite() {
            return Err(EasingCurveError::NonFiniteInput { input });
        }
        if input < EasingCurve::START_INPUT || input > EasingCurve::END_INPUT {
            return Err(EasingCurveError::InputOutOfRange { input });
        }
        Ok(Self(input))
    }

    /// Returns the normalized coordinate.
    #[must_use]
    pub const fn value(self) -> f32 { self.0 }
}

/// A finite authored easing output.
///
/// Outputs deliberately retain anticipation, overshoot, and reversal values.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Reflect)]
#[reflect(opaque)]
pub struct EasingOutput(f32);

impl EasingOutput {
    /// Creates a finite authored output.
    ///
    /// # Errors
    ///
    /// Returns [`EasingCurveError::NonFiniteOutput`] for NaN or infinity.
    pub const fn try_new(output: f32) -> Result<Self, EasingCurveError> {
        if output.is_finite() {
            Ok(Self(output))
        } else {
            Err(EasingCurveError::NonFiniteOutput { output })
        }
    }

    /// Returns the authored output.
    #[must_use]
    pub const fn value(self) -> f32 { self.0 }
}

/// A finite tangent slope for cubic [`EasingInterpolation`].
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Reflect)]
#[reflect(opaque)]
pub struct EasingSlope(f32);

impl EasingSlope {
    /// Creates a finite tangent slope.
    ///
    /// # Errors
    ///
    /// Returns [`EasingCurveError::NonFiniteSlope`] for NaN or infinity.
    pub const fn try_new(slope: f32) -> Result<Self, EasingCurveError> {
        if slope.is_finite() {
            Ok(Self(slope))
        } else {
            Err(EasingCurveError::NonFiniteSlope { slope })
        }
    }

    /// Returns the tangent slope.
    #[must_use]
    pub const fn value(self) -> f32 { self.0 }
}

/// Interpolation applied between one [`EasingKnot`] and the next.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Reflect)]
pub enum EasingInterpolation {
    /// Hold the knot's output until the next knot.
    Constant,
    /// Straight line to the next knot.
    #[default]
    Linear,
    /// Cubic segment whose curvature comes from both knots' tangent slopes.
    Cubic,
}

/// Incoming and outgoing tangent slopes used by cubic interpolation.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct EasingSlopes {
    incoming: EasingSlope,
    outgoing: EasingSlope,
}

impl Default for EasingSlopes {
    fn default() -> Self {
        Self {
            incoming: EasingSlope(0.0),
            outgoing: EasingSlope(0.0),
        }
    }
}

impl EasingSlopes {
    /// Replaces this knot's incoming tangent slope.
    #[must_use]
    pub const fn with_incoming(mut self, incoming: EasingSlope) -> Self {
        self.incoming = incoming;
        self
    }

    /// Replaces this knot's outgoing tangent slope.
    #[must_use]
    pub const fn with_outgoing(mut self, outgoing: EasingSlope) -> Self {
        self.outgoing = outgoing;
        self
    }

    /// Returns the incoming tangent slope.
    #[must_use]
    pub const fn incoming(self) -> EasingSlope { self.incoming }

    /// Returns the outgoing tangent slope.
    #[must_use]
    pub const fn outgoing(self) -> EasingSlope { self.outgoing }
}

/// One validated authored point of an [`EasingCurve`].
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct EasingKnot {
    input:         EasingInput,
    output:        EasingOutput,
    interpolation: EasingInterpolation,
    slopes:        EasingSlopes,
}

impl EasingKnot {
    /// Creates a knot from already validated semantic coordinates.
    #[must_use]
    pub const fn new(
        input: EasingInput,
        output: EasingOutput,
        interpolation: EasingInterpolation,
    ) -> Self {
        Self {
            input,
            output,
            interpolation,
            slopes: EasingSlopes {
                incoming: EasingSlope(0.0),
                outgoing: EasingSlope(0.0),
            },
        }
    }

    /// Returns this knot with the given tangent slopes.
    #[must_use]
    pub const fn with_slopes(mut self, slopes: EasingSlopes) -> Self {
        self.slopes = slopes;
        self
    }

    /// Returns the normalized input progress.
    #[must_use]
    pub const fn input(self) -> EasingInput { self.input }

    /// Returns the authored output value.
    #[must_use]
    pub const fn output(self) -> EasingOutput { self.output }

    /// Returns the interpolation used toward the next knot.
    #[must_use]
    pub const fn interpolation(self) -> EasingInterpolation { self.interpolation }

    /// Returns the tangent slopes used by cubic interpolation.
    #[must_use]
    pub const fn slopes(self) -> EasingSlopes { self.slopes }
}

/// Whether a curve may remap whole-sequence or multi-stage progress.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
pub enum EasingMapping {
    /// Output stays within `0..=1` and never decreases, so the curve can remap
    /// raw sequence progress.
    BoundedMonotonic,
    /// Output leaves `0..=1` or reverses, so the curve is valid only as one
    /// stage's output easing.
    OvershootingOrReversing,
    /// The curve produces a NaN or infinite output, so it eases nothing at any
    /// scope.
    NonFiniteOutput,
}

impl EasingMapping {
    /// Ordinal of the final sample taken across `0..=1`, held in a width that
    /// converts to `f32` without losing precision.
    const LAST_SAMPLE: u16 = 64;

    /// Classifies a stock easing from its evaluation.
    fn from_samples(sample: impl Fn(f32) -> f32) -> Self {
        let mut previous = f32::NEG_INFINITY;
        for step in 0..=Self::LAST_SAMPLE {
            let progress = f32::from(step) / f32::from(Self::LAST_SAMPLE);
            let output = sample(progress);
            if !output.is_finite() {
                return Self::NonFiniteOutput;
            }
            if !(EasingCurve::START_OUTPUT..=EasingCurve::END_OUTPUT).contains(&output)
                || output < previous
            {
                return Self::OvershootingOrReversing;
            }
            previous = output;
        }
        Self::BoundedMonotonic
    }

    /// Classifies authored knots by bounding every segment between them.
    fn from_knots(knots: &[EasingKnot]) -> Self {
        knots
            .windows(2)
            .map(|segment| segment_mapping(segment[0], segment[1]))
            .find(|mapping| !matches!(mapping, Self::BoundedMonotonic))
            .unwrap_or(Self::BoundedMonotonic)
    }
}

/// Bounds one authored segment between consecutive knots.
fn segment_mapping(start: EasingKnot, end: EasingKnot) -> EasingMapping {
    match start.interpolation() {
        EasingInterpolation::Constant | EasingInterpolation::Linear => {
            endpoint_mapping(start.output().value(), end.output().value())
        },
        EasingInterpolation::Cubic => cubic_segment_mapping(start, end),
    }
}

/// Bounds a segment whose extreme values are its two authored outputs.
fn endpoint_mapping(start: f32, end: f32) -> EasingMapping {
    let outputs = EasingCurve::START_OUTPUT..=EasingCurve::END_OUTPUT;
    if outputs.contains(&start) && outputs.contains(&end) && end >= start {
        EasingMapping::BoundedMonotonic
    } else {
        EasingMapping::OvershootingOrReversing
    }
}

/// Bounds a cubic Hermite segment from its endpoints and tangent slopes.
fn cubic_segment_mapping(start: EasingKnot, end: EasingKnot) -> EasingMapping {
    let width = f64::from(end.input().value()) - f64::from(start.input().value());
    let first = f64::from(start.output().value());
    let last = f64::from(end.output().value());
    let outgoing = f64::from(start.slopes().outgoing().value()) * width;
    let incoming = f64::from(end.slopes().incoming().value()) * width;
    let quadratic = 3.0f64.mul_add(last - first, -2.0f64.mul_add(outgoing, incoming));
    let cubic = 2.0f64.mul_add(first - last, outgoing + incoming);

    if slope_stays_nonnegative(outgoing, quadratic, cubic) {
        endpoint_mapping(start.output().value(), end.output().value())
    } else {
        EasingMapping::OvershootingOrReversing
    }
}

/// Whether `3 * cubic * s^2 + 2 * quadratic * s + linear` stays at or above zero
/// across `s` in `0..=1`.
fn slope_stays_nonnegative(linear: f64, quadratic: f64, cubic: f64) -> bool {
    let curvature = 3.0 * cubic;
    if linear < 0.0 || 2.0f64.mul_add(quadratic, curvature + linear) < 0.0 {
        return false;
    }
    if curvature <= 0.0 {
        return true;
    }
    let turning_point = -quadratic / curvature;
    if !(0.0..1.0).contains(&turning_point) {
        return true;
    }
    curvature
        .mul_add(turning_point, 2.0 * quadratic)
        .mul_add(turning_point, linear)
        >= 0.0
}

/// Immutable validated lookup curve authored by value.
///
/// The private representation is a `bevy_lookup_curve` spline. Every accepted
/// curve keeps at least two knots, strictly increasing normalized inputs, and
/// exact `0` and `1` endpoints. A future editable or asset-backed source must
/// use a distinct [`Easing`] variant with its own availability contract.
#[derive(Clone, Debug, Reflect)]
#[reflect(opaque)]
pub struct EasingCurve {
    knots:   Vec<EasingKnot>,
    mapping: EasingMapping,
    lookup:  LookupCurve,
}

impl PartialEq for EasingCurve {
    /// Compares immutable authored knots. The private spline is derived from them.
    fn eq(&self, other: &Self) -> bool { self.knots == other.knots }
}

impl EasingCurve {
    /// Normalized input of the final knot.
    pub const END_INPUT: f32 = 1.0;
    /// Required output of the final knot.
    pub const END_OUTPUT: f32 = 1.0;
    /// Fewest knots an authored curve can describe.
    pub const MINIMUM_KNOTS: usize = 2;
    /// Normalized input of the first knot.
    pub const START_INPUT: f32 = 0.0;
    /// Required output of the first knot.
    pub const START_OUTPUT: f32 = 0.0;

    /// Starts programmatic construction.
    #[must_use]
    pub fn builder() -> EasingCurveBuilder { EasingCurveBuilder::default() }

    /// Creates a curve from ordered authored knots.
    ///
    /// # Errors
    ///
    /// Returns the exact [`EasingCurveError`] identifying too few knots, a
    /// repeated or out-of-order input, or a missing exact endpoint.
    pub fn try_new(knots: impl IntoIterator<Item = EasingKnot>) -> Result<Self, EasingCurveError> {
        let knots: Vec<EasingKnot> = knots.into_iter().collect();
        Self::validate(&knots)?;
        Ok(Self {
            lookup: LookupCurve::new(knots.iter().copied().map(Knot::from).collect()),
            mapping: EasingMapping::from_knots(&knots),
            knots,
        })
    }

    /// Returns the authored knots in input order.
    #[must_use]
    pub fn knots(&self) -> &[EasingKnot] { &self.knots }

    /// Samples the curve at normalized `progress`.
    #[must_use]
    pub fn sample(&self, progress: f32) -> f32 { self.lookup.lookup(progress) }

    /// Returns whether this curve can remap whole-sequence progress.
    #[must_use]
    pub const fn mapping(&self) -> EasingMapping { self.mapping }

    fn validate(knots: &[EasingKnot]) -> Result<(), EasingCurveError> {
        if knots.len() < Self::MINIMUM_KNOTS {
            return Err(EasingCurveError::TooFewKnots { knots: knots.len() });
        }
        for (ordinal, knot) in knots.iter().enumerate().skip(1) {
            if knot.input.value() <= knots[ordinal - 1].input.value() {
                return Err(EasingCurveError::KnotInputOutOfOrder { ordinal });
            }
        }

        let first = knots[0];
        let last = knots[knots.len() - 1];
        if first.input.value() != Self::START_INPUT || first.output.value() != Self::START_OUTPUT {
            return Err(EasingCurveError::EndpointNotPreserved {
                ordinal: 0,
                input:   first.input,
                output:  first.output,
            });
        }
        if last.input.value() != Self::END_INPUT || last.output.value() != Self::END_OUTPUT {
            return Err(EasingCurveError::EndpointNotPreserved {
                ordinal: knots.len() - 1,
                input:   last.input,
                output:  last.output,
            });
        }
        Ok(())
    }
}

/// Programmatic [`EasingCurve`] construction for examples, tests, and generated
/// motion.
#[derive(Clone, Debug, Default)]
pub struct EasingCurveBuilder {
    knots:         Vec<(EasingInput, EasingOutput)>,
    interpolation: EasingInterpolation,
}

impl EasingCurveBuilder {
    /// Appends a knot from its semantic coordinate roles.
    #[must_use]
    pub fn knot(mut self, input: EasingInput, output: EasingOutput) -> Self {
        self.knots.push((input, output));
        self
    }

    /// Uses constant interpolation between every knot.
    #[must_use]
    pub const fn constant(mut self) -> Self {
        self.interpolation = EasingInterpolation::Constant;
        self
    }

    /// Uses linear interpolation between every knot.
    #[must_use]
    pub const fn linear(mut self) -> Self {
        self.interpolation = EasingInterpolation::Linear;
        self
    }

    /// Uses cubic interpolation between every knot.
    #[must_use]
    pub const fn cubic(mut self) -> Self {
        self.interpolation = EasingInterpolation::Cubic;
        self
    }

    /// Validates every knot and builds the curve.
    ///
    /// # Errors
    ///
    /// Returns the exact [`EasingCurveError`] for an invalid overall curve.
    pub fn try_build(self) -> Result<EasingCurve, EasingCurveError> {
        let knots = self
            .knots
            .into_iter()
            .map(|(input, output)| EasingKnot::new(input, output, self.interpolation));
        EasingCurve::try_new(knots)
    }
}

/// Authored choice between a stock Bevy easing and an immutable lookup curve.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
pub enum Easing {
    /// One of Bevy's built-in easing functions.
    Bevy(EaseFunction),
    /// An owned validated lookup curve.
    Curve(EasingCurve),
}

impl Default for Easing {
    fn default() -> Self { Self::Bevy(EaseFunction::Linear) }
}

impl From<EaseFunction> for Easing {
    fn from(ease_function: EaseFunction) -> Self { Self::Bevy(ease_function) }
}

impl From<EasingCurve> for Easing {
    fn from(easing_curve: EasingCurve) -> Self { Self::Curve(easing_curve) }
}

/// Result of sampling an [`Easing`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EasingSample {
    /// The curve produced this eased output.
    Eased(f32),
    /// The curve produced a NaN or infinite output for this input.
    NonFinite,
}

/// Samples either immutable easing source without runtime state.
#[derive(Clone, Copy, Debug, Default)]
pub struct EasingSampler;

impl EasingSampler {
    /// Samples `easing` at normalized `progress`.
    #[must_use]
    pub fn sample(&self, easing: &Easing, progress: f32) -> EasingSample {
        let eased = match easing {
            Easing::Bevy(ease_function) => ease_function.sample_clamped(progress),
            Easing::Curve(easing_curve) => easing_curve.sample(progress),
        };
        if eased.is_finite() {
            EasingSample::Eased(eased)
        } else {
            EasingSample::NonFinite
        }
    }

    /// Classifies whether `easing` may remap whole-sequence progress.
    #[must_use]
    pub fn mapping(&self, easing: &Easing) -> EasingMapping {
        match easing {
            Easing::Bevy(ease_function) => {
                EasingMapping::from_samples(|progress| ease_function.sample_clamped(progress))
            },
            Easing::Curve(easing_curve) => easing_curve.mapping(),
        }
    }
}

/// Errors returned while authoring an [`EasingCurve`].
#[derive(Clone, Copy, Debug, Error, PartialEq)]
pub enum EasingCurveError {
    /// An input coordinate was NaN or infinite.
    #[error("easing input {input} must be finite")]
    NonFiniteInput {
        /// Invalid normalized input.
        input: f32,
    },
    /// An input coordinate left the normalized interval.
    #[error("easing input {input} must be within 0..=1")]
    InputOutOfRange {
        /// Invalid normalized input.
        input: f32,
    },
    /// An output coordinate was NaN or infinite.
    #[error("easing output {output} must be finite")]
    NonFiniteOutput {
        /// Invalid authored output.
        output: f32,
    },
    /// A tangent slope was NaN or infinite.
    #[error("easing tangent slope {slope} must be finite")]
    NonFiniteSlope {
        /// Invalid tangent slope.
        slope: f32,
    },
    /// The curve described fewer than [`EasingCurve::MINIMUM_KNOTS`] knots.
    #[error("easing curve needs at least 2 knots but described {knots}")]
    TooFewKnots {
        /// Number of authored knots.
        knots: usize,
    },
    /// A knot input repeated or decreased.
    #[error("easing knot {ordinal} input must be greater than its predecessor")]
    KnotInputOutOfOrder {
        /// Ordinal of the first out-of-order knot.
        ordinal: usize,
    },
    /// An endpoint knot did not map exactly `0` to `0` or `1` to `1`.
    #[error("easing endpoint knot {ordinal} ({input:?}, {output:?}) must preserve 0 and 1")]
    EndpointNotPreserved {
        /// Ordinal of the invalid endpoint knot.
        ordinal: usize,
        /// Endpoint normalized input.
        input:   EasingInput,
        /// Endpoint authored output.
        output:  EasingOutput,
    },
}

impl From<EasingKnot> for Knot {
    fn from(easing_knot: EasingKnot) -> Self {
        Self {
            position: Vec2::new(easing_knot.input.value(), easing_knot.output.value()),
            interpolation: match easing_knot.interpolation {
                EasingInterpolation::Constant => KnotInterpolation::Constant,
                EasingInterpolation::Linear => KnotInterpolation::Linear,
                EasingInterpolation::Cubic => KnotInterpolation::Cubic,
            },
            left_tangent: Tangent {
                slope: easing_knot.slopes.incoming().value(),
                ..Tangent::default()
            },
            right_tangent: Tangent {
                slope: easing_knot.slopes.outgoing().value(),
                ..Tangent::default()
            },
            ..Self::default()
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare authored knot values that round-trip exactly"
)]
mod tests {
    use bevy::reflect::FromReflect;
    use bevy::reflect::Reflect;
    use bevy::reflect::ReflectKind;
    use bevy::reflect::TypePath;
    use bevy::reflect::structs::DynamicStruct;

    use super::*;

    const DECREASING_RUN_KNOTS: [(f32, f32); 4] = [(0.0, 0.0), (0.3, 0.8), (0.6, 0.2), (1.0, 1.0)];
    const LATCH_KNOTS: [(f32, f32); 4] = [(0.0, 0.0), (0.2, 0.02), (0.8, 0.92), (1.0, 1.0)];
    const OVERSHOOTING_KNOTS: [(f32, f32); 3] = [(0.0, 0.0), (0.5, 1.4), (1.0, 1.0)];
    const SPIKE_END_OUTPUT: f32 = 0.01;
    const SPIKE_END_INPUT: f32 = 1.0 / 64.0;
    const SPIKE_INTERIOR: f32 = 0.0033;
    const SPIKE_SLOPE: f32 = 1000.0;

    fn input(value: f32) -> EasingInput { EasingInput::try_new(value).unwrap() }

    fn output(value: f32) -> EasingOutput { EasingOutput::try_new(value).unwrap() }

    fn slope(value: f32) -> EasingSlope { EasingSlope::try_new(value).unwrap() }

    fn latch_curve() -> EasingCurve { curve_from(&LATCH_KNOTS) }

    fn spiking_curve() -> EasingCurve {
        EasingCurve::try_new([
            EasingKnot::new(input(0.0), output(0.0), EasingInterpolation::Cubic)
                .with_slopes(EasingSlopes::default().with_outgoing(slope(SPIKE_SLOPE))),
            EasingKnot::new(
                input(SPIKE_END_INPUT),
                output(SPIKE_END_OUTPUT),
                EasingInterpolation::Cubic,
            )
            .with_slopes(EasingSlopes::default().with_incoming(slope(SPIKE_SLOPE))),
            EasingKnot::new(input(1.0), output(1.0), EasingInterpolation::Cubic),
        ])
        .unwrap()
    }

    fn curve_from(knots: &[(f32, f32)]) -> EasingCurve {
        knots
            .iter()
            .copied()
            .fold(
                EasingCurve::builder(),
                |builder, (input_value, output_value)| {
                    builder.knot(input(input_value), output(output_value))
                },
            )
            .cubic()
            .try_build()
            .unwrap()
    }

    #[test]
    fn semantic_values_report_role_specific_rejections() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(matches!(
                EasingInput::try_new(value),
                Err(EasingCurveError::NonFiniteInput { input }) if input.to_bits() == value.to_bits()
            ));
            assert!(matches!(
                EasingOutput::try_new(value),
                Err(EasingCurveError::NonFiniteOutput { output }) if output.to_bits() == value.to_bits()
            ));
            assert!(matches!(
                EasingSlope::try_new(value),
                Err(EasingCurveError::NonFiniteSlope { slope }) if slope.to_bits() == value.to_bits()
            ));
        }
        assert_eq!(
            EasingInput::try_new(-0.01),
            Err(EasingCurveError::InputOutOfRange { input: -0.01 })
        );
        assert_eq!(
            EasingInput::try_new(1.01),
            Err(EasingCurveError::InputOutOfRange { input: 1.01 })
        );
    }

    #[test]
    fn semantic_values_preserve_boundaries_overshoot_reversal_and_tangents() {
        assert_eq!(input(0.0).value(), 0.0);
        assert_eq!(input(1.0).value(), 1.0);
        assert_eq!(output(1.5).value(), 1.5);
        assert_eq!(output(-0.25).value(), -0.25);
        assert_eq!(slope(-2.0).value(), -2.0);
    }

    #[test]
    fn knots_and_slopes_retain_their_semantic_roles() {
        let incoming = slope(-2.0);
        let outgoing = slope(3.0);
        let slopes = EasingSlopes::default()
            .with_incoming(incoming)
            .with_outgoing(outgoing);
        let knot = EasingKnot::new(input(0.25), output(1.25), EasingInterpolation::Cubic)
            .with_slopes(slopes);

        assert_eq!(knot.input(), input(0.25));
        assert_eq!(knot.output(), output(1.25));
        assert_eq!(knot.slopes().incoming(), incoming);
        assert_eq!(knot.slopes().outgoing(), outgoing);
    }

    #[test]
    fn curve_construction_reports_every_exact_error_variant() {
        assert_eq!(
            EasingCurve::try_new([]),
            Err(EasingCurveError::TooFewKnots { knots: 0 })
        );
        assert_eq!(
            EasingCurve::try_new([
                EasingKnot::new(input(0.0), output(0.0), EasingInterpolation::Linear),
                EasingKnot::new(input(0.0), output(0.5), EasingInterpolation::Linear),
                EasingKnot::new(input(1.0), output(1.0), EasingInterpolation::Linear),
            ]),
            Err(EasingCurveError::KnotInputOutOfOrder { ordinal: 1 })
        );
        assert_eq!(
            EasingCurve::try_new([
                EasingKnot::new(input(0.0), output(0.1), EasingInterpolation::Linear),
                EasingKnot::new(input(1.0), output(1.0), EasingInterpolation::Linear),
            ]),
            Err(EasingCurveError::EndpointNotPreserved {
                ordinal: 0,
                input:   input(0.0),
                output:  output(0.1),
            })
        );
    }

    #[test]
    fn stock_and_lookup_sources_sample_without_assets() {
        let easing_sampler = EasingSampler;
        let curve = latch_curve();
        let stock = easing_sampler.sample(&Easing::from(EaseFunction::CubicOut), 0.5);
        let curve_start = easing_sampler.sample(&Easing::Curve(curve.clone()), 0.0);
        let curve_end = easing_sampler.sample(&Easing::Curve(curve), 1.0);

        assert_eq!(
            stock,
            EasingSample::Eased(EaseFunction::CubicOut.sample_clamped(0.5))
        );
        assert_eq!(curve_start, EasingSample::Eased(EasingCurve::START_OUTPUT));
        assert_eq!(curve_end, EasingSample::Eased(EasingCurve::END_OUTPUT));
    }

    #[test]
    fn mapping_classification_separates_bounded_monotonic_from_overshooting() {
        assert_eq!(latch_curve().mapping(), EasingMapping::BoundedMonotonic);
        assert_eq!(
            curve_from(&OVERSHOOTING_KNOTS).mapping(),
            EasingMapping::OvershootingOrReversing
        );
        assert_eq!(
            curve_from(&DECREASING_RUN_KNOTS).mapping(),
            EasingMapping::OvershootingOrReversing
        );
        assert_eq!(
            EasingMapping::from_samples(
                |progress| EaseFunction::CubicInOut.sample_clamped(progress)
            ),
            EasingMapping::BoundedMonotonic
        );
        assert_eq!(
            EasingMapping::from_samples(|progress| EaseFunction::BackInOut.sample_clamped(progress)),
            EasingMapping::OvershootingOrReversing
        );
    }

    #[test]
    fn knot_classification_catches_a_spike_that_falls_between_samples() {
        let spiking = spiking_curve();

        assert_eq!(
            EasingMapping::from_samples(|progress| spiking.sample(progress)),
            EasingMapping::BoundedMonotonic
        );
        assert!(spiking.sample(SPIKE_INTERIOR) > EasingCurve::END_OUTPUT);
        assert_eq!(spiking.mapping(), EasingMapping::OvershootingOrReversing);
    }

    #[test]
    fn a_non_finite_stock_sample_reports_non_finite_output() {
        assert_eq!(
            EasingSampler.sample(&Easing::Bevy(EaseFunction::Elastic(f32::NAN)), 0.5),
            EasingSample::NonFinite
        );
    }

    fn assert_opaque_reflection<T>(mut value: T, structural_patch: &DynamicStruct)
    where
        T: Clone + FromReflect + PartialEq + Reflect + std::fmt::Debug,
    {
        let validated_value = value.clone();

        assert_eq!(value.reflect_kind(), ReflectKind::Opaque);
        assert!(value.try_apply(structural_patch).is_err());
        assert!(T::from_reflect(structural_patch).is_none());
        assert_eq!(value, validated_value);
    }

    #[test]
    fn authored_easing_values_reject_structural_reflection_mutation() {
        let input = input(0.25);
        let output = output(0.75);
        let slope = slope(-1.0);
        let slopes = EasingSlopes::default()
            .with_incoming(slope)
            .with_outgoing(slope);
        let knot = EasingKnot::new(input, output, EasingInterpolation::Cubic).with_slopes(slopes);
        let curve = latch_curve();
        let easing = Easing::Curve(curve.clone());
        let mut structural_patch = DynamicStruct::default();
        structural_patch.insert("value", f32::NAN);

        assert_opaque_reflection(input, &structural_patch);
        assert_opaque_reflection(output, &structural_patch);
        assert_opaque_reflection(slope, &structural_patch);
        assert_opaque_reflection(knot, &structural_patch);
        assert_opaque_reflection(slopes, &structural_patch);
        assert_opaque_reflection(curve, &structural_patch);
        assert_opaque_reflection(easing, &structural_patch);
    }

    #[test]
    fn easing_curve_type_path_is_owned_by_bevy_kana() {
        assert!(EasingCurve::type_path().starts_with("bevy_kana::"));
    }
}
