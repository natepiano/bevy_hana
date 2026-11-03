use core::cmp::Ordering;

use bevy::prelude::Component;
use hana_kana::ToF32;

use super::Slider;

/// Direction in which slider values increase.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SliderDirection {
    /// Values increase from left to right.
    #[default]
    LeftToRight,
    /// Values increase from right to left.
    RightToLeft,
    /// Values increase from bottom to top.
    BottomToTop,
    /// Values increase from top to bottom.
    TopToBottom,
}

/// How a slider distributes its values along the track.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SliderScale {
    Linear,
    Logarithmic,
}

/// Validated numeric range and track scale for a slider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderRange {
    start: f32,
    end:   f32,
    scale: SliderScale,
}

impl SliderRange {
    /// The `0.0..=1.0` range, used as the fallback for a slider declaration
    /// panel build already rejected.
    pub(super) const UNIT: Self = Self {
        start: 0.0,
        end:   1.0,
        scale: SliderScale::Linear,
    };

    /// Creates a finite slider range whose start is strictly less than its end.
    ///
    /// # Errors
    ///
    /// Returns [`SliderConfigError::NonFiniteRange`] when either endpoint is
    /// non-finite, or [`SliderConfigError::UnorderedRange`] when `start` is not
    /// strictly less than `end`.
    pub fn new(start: f32, end: f32) -> Result<Self, SliderConfigError> {
        if !start.is_finite() || !end.is_finite() {
            return Err(SliderConfigError::NonFiniteRange);
        }
        if start >= end {
            return Err(SliderConfigError::UnorderedRange);
        }
        Ok(Self {
            start,
            end,
            scale: SliderScale::Linear,
        })
    }

    /// Creates a logarithmic range with a strictly positive start.
    ///
    /// # Errors
    ///
    /// Returns the same range errors as [`Self::new`], or
    /// [`SliderConfigError::NonPositiveLogRange`] when the start is not positive.
    pub fn logarithmic(start: f32, end: f32) -> Result<Self, SliderConfigError> {
        let mut range = Self::new(start, end)?;
        if start <= 0.0 {
            return Err(SliderConfigError::NonPositiveLogRange);
        }
        range.scale = SliderScale::Logarithmic;
        Ok(range)
    }

    /// Returns the value's track fraction. Values outside the range extrapolate;
    /// a nonpositive value on a log range uses the lower endpoint.
    #[must_use]
    pub fn value_to_fraction(self, value: f32) -> f32 {
        let start = f64::from(self.start);
        let end = f64::from(self.end);
        let value = f64::from(value);
        let fraction = match self.scale {
            SliderScale::Linear => (value - start) / (end - start),
            SliderScale::Logarithmic if value > 0.0 => {
                (value.ln() - start.ln()) / (end.ln() - start.ln())
            },
            SliderScale::Logarithmic => 0.0,
        };
        fraction.to_f32()
    }

    /// Maps a track fraction to a real value. Fractions outside `0..=1`
    /// extrapolate for relative pointer travel, saturating at finite limits.
    /// Exact endpoints retain their authored bits.
    #[must_use]
    pub fn fraction_to_value(self, fraction: f32) -> f32 {
        if fraction == 0.0 {
            return self.start;
        }
        if fraction.to_bits() == 1.0_f32.to_bits() {
            return self.end;
        }
        let start = f64::from(self.start);
        let end = f64::from(self.end);
        let fraction = f64::from(fraction);
        let value = match self.scale {
            SliderScale::Linear => start.mul_add(1.0 - fraction, end * fraction),
            SliderScale::Logarithmic => (end.ln() - start.ln()).mul_add(fraction, start.ln()).exp(),
        };
        value
            .clamp(f64::from(f32::MIN), f64::from(f32::MAX))
            .to_f32()
    }

    /// Applies relative track travel to a real value without first rounding
    /// that value to an `f32` track fraction.
    pub(super) fn offset_value_by_fraction(self, value: f32, delta: f32) -> f32 {
        if delta == 0.0 {
            return value;
        }
        let start = f64::from(self.start);
        let end = f64::from(self.end);
        let value = f64::from(value);
        let delta = f64::from(delta);
        let next = match self.scale {
            SliderScale::Linear => (end - start).mul_add(delta, value),
            SliderScale::Logarithmic => value * ((end.ln() - start.ln()) * delta).exp(),
        };
        next.clamp(f64::from(f32::MIN), f64::from(f32::MAX))
            .to_f32()
    }

    /// Returns the inclusive lower endpoint.
    #[must_use]
    pub const fn start(self) -> f32 { self.start }

    /// Returns the inclusive upper endpoint.
    #[must_use]
    pub const fn end(self) -> f32 { self.end }
}

/// Validated step interval for a slider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderStep(f32);

impl SliderStep {
    /// Creates a finite, positive slider step.
    ///
    /// # Errors
    ///
    /// Returns [`SliderConfigError::NonPositiveStep`] when `step` is non-finite,
    /// zero, or negative.
    pub fn new(step: f32) -> Result<Self, SliderConfigError> {
        if !step.is_finite() || step <= 0.0 {
            return Err(SliderConfigError::NonPositiveStep);
        }
        Ok(Self(step))
    }

    /// Returns the step interval.
    #[must_use]
    pub const fn value(self) -> f32 { self.0 }
}

/// Whether a slider's applied value snaps to a step lattice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderStepping {
    /// Any value in the range is allowed.
    Stepless,
    /// Values snap to multiples of this interval from the range start.
    Stepped(SliderStep),
}

/// Applied runtime state of a reified slider widget.
///
/// First reification constructs this component from the authored [`Slider`];
/// later reifications preserve the live applied value and revalidate it when
/// the authored range, step, or direction changes. Application code applies
/// or rejects each [`SliderChangeRequested`](super::SliderChangeRequested) proposal with
/// [`SliderState::set_proposed_value`], or opts into the uncontrolled convenience by
/// observing with [`slider_self_update`](super::slider_self_update).
#[derive(Clone, Component, Debug, PartialEq)]
pub struct SliderState {
    range:     SliderRange,
    value:     f32,
    stepping:  SliderStepping,
    direction: SliderDirection,
    pushed:    SliderPushedValue,
}

/// Where a live signal pushes the setting relative to its authored handle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SliderPushedValue {
    /// Keep the authored fill and hide the reachable signal band.
    #[default]
    AtHandle,
    /// Fill from the authored handle to `value`, with a faint reachable band.
    PushedTo {
        /// Live value in the slider's raw numeric domain.
        value:      f32,
        /// Lower endpoint of the reachable interval in the raw numeric domain.
        band_start: f32,
        /// Upper endpoint of the reachable interval in the raw numeric domain.
        band_end:   f32,
    },
}

impl SliderState {
    /// Creates slider state with a finite applied value.
    ///
    /// The real value is stored exactly, clamped only to the range. The step
    /// lattice applies when accepting proposals, not when setting state in code.
    ///
    /// # Errors
    ///
    /// Returns [`SliderConfigError::NonFiniteValue`] when `value` is
    /// non-finite.
    pub const fn new(
        range: SliderRange,
        value: f32,
        stepping: SliderStepping,
        direction: SliderDirection,
    ) -> Result<Self, SliderConfigError> {
        if !value.is_finite() {
            return Err(SliderConfigError::NonFiniteValue);
        }
        Ok(Self::from_validated(range, value, stepping, direction))
    }

    pub(super) const fn from_validated(
        range: SliderRange,
        value: f32,
        stepping: SliderStepping,
        direction: SliderDirection,
    ) -> Self {
        Self {
            range,
            value: value.clamp(range.start(), range.end()),
            stepping,
            direction,
            pushed: SliderPushedValue::AtHandle,
        }
    }

    /// Stores a real `value` from code exactly, clamped only to the range.
    /// Accept interaction proposals with [`Self::set_proposed_value`] instead.
    ///
    /// Returns whether the applied value changed.
    ///
    /// # Errors
    ///
    /// Returns [`SliderConfigError::NonFiniteValue`] when `value` is
    /// non-finite; the applied value is left untouched.
    pub fn set_value(&mut self, value: f32) -> Result<bool, SliderConfigError> {
        if !value.is_finite() {
            return Err(SliderConfigError::NonFiniteValue);
        }
        let next = value.clamp(self.range.start(), self.range.end());
        let changed = next.partial_cmp(&self.value) != Some(Ordering::Equal);
        self.value = next;
        Ok(changed)
    }

    /// Accepts a finite interaction proposal, clamping it to the range.
    /// Motion and adjustment proposals snap to the step lattice; reset defaults
    /// and held values retain their exact real value.
    pub fn set_proposed_value(&mut self, value: SliderProposalValue) -> bool {
        let next = match value.stepping {
            SliderProposalStepping::SnapToStep => {
                normalized(self.range, self.stepping, value.get())
            },
            SliderProposalStepping::PreserveValue => {
                value.get().clamp(self.range.start(), self.range.end())
            },
        };
        let changed = next.partial_cmp(&self.value) != Some(Ordering::Equal);
        self.value = next;
        changed
    }

    /// Returns the validated range.
    #[must_use]
    pub const fn range(&self) -> SliderRange { self.range }

    /// Returns the applied raw-domain value.
    #[must_use]
    pub const fn value(&self) -> f32 { self.value }

    /// Sets the live fill and reachable band without moving the authored handle.
    pub const fn set_pushed(&mut self, pushed: SliderPushedValue) { self.pushed = pushed; }

    /// Returns the live fill and reachable band state.
    #[must_use]
    pub const fn pushed(&self) -> SliderPushedValue { self.pushed }

    /// Returns whether the slider snaps to a step lattice.
    #[must_use]
    pub const fn stepping(&self) -> SliderStepping { self.stepping }

    /// Returns the direction in which values increase.
    #[must_use]
    pub const fn direction(&self) -> SliderDirection { self.direction }

    /// Whether this state already carries `authored`'s range, step, and
    /// direction; the authored default does not alter an existing applied value
    /// and is therefore not compared.
    pub(in crate::widgets) fn matches_configuration(&self, authored: &Slider) -> bool {
        authored.validated().is_ok_and(|initial| {
            self.range == initial.range
                && self.stepping == initial.stepping
                && self.direction == initial.direction
        })
    }

    /// Rebuilds state around `authored`'s configuration while preserving
    /// `value`, clamping it to the range just like
    /// [`SliderState::set_value`].
    pub(in crate::widgets) fn with_configuration(authored: &Slider, value: f32) -> Self {
        let mut state = authored.initial_state();
        state.value = value.clamp(state.range.start(), state.range.end());
        state
    }

    /// Computes the finite raw-domain target for `adjustment` without
    /// normalizing it. Arithmetic overflow saturates at the finite `f32`
    /// limit. Normalization is deferred to [`SliderState::set_proposed_value`] when the
    /// application accepts the proposal.
    ///
    /// # Errors
    ///
    /// Returns [`SliderAdjustmentRejected::NonFiniteInput`] for a non-finite
    /// adjustment input, or [`SliderAdjustmentRejected::StepOnStepless`] when a
    /// relative step adjustment finds no step.
    pub(super) fn adjusted_value(
        &self,
        adjustment: SliderAdjustment,
    ) -> Result<SliderProposalValue, SliderAdjustmentRejected> {
        let target = match (adjustment, self.stepping) {
            (SliderAdjustment::RelativeSteps(_), SliderStepping::Stepless) => {
                return Err(SliderAdjustmentRejected::StepOnStepless);
            },
            (SliderAdjustment::Absolute(value), _) if value.is_finite() => value,
            (SliderAdjustment::Relative(delta), _) if delta.is_finite() => self.value + delta,
            (SliderAdjustment::RelativeSteps(steps), SliderStepping::Stepped(step))
                if steps.is_finite() =>
            {
                steps.mul_add(step.value(), self.value)
            },
            _ => return Err(SliderAdjustmentRejected::NonFiniteInput),
        };
        Ok(SliderProposalValue::snapping(
            target.clamp(f32::MIN, f32::MAX),
        ))
    }
}

/// Why [`SliderState::adjusted_value`] computed no target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SliderAdjustmentRejected {
    /// The adjustment's numeric input is non-finite.
    NonFiniteInput,
    /// A `RelativeSteps` adjustment on a `Stepless` slider.
    StepOnStepless,
}

fn normalized(range: SliderRange, stepping: SliderStepping, value: f32) -> f32 {
    let snapped = match stepping {
        SliderStepping::Stepless => value,
        SliderStepping::Stepped(step) => ((value - range.start()) / step.value())
            .round()
            .mul_add(step.value(), range.start()),
    };
    snapped.clamp(range.start(), range.end())
}

/// Finite raw-domain value carried by a slider proposal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderProposalValue {
    value:    f32,
    stepping: SliderProposalStepping,
}

/// Whether accepting a proposal may move it onto the step lattice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SliderProposalStepping {
    SnapToStep,
    PreserveValue,
}

impl SliderProposalValue {
    /// Creates a proposal value only for finite input.
    ///
    /// # Errors
    ///
    /// Returns [`SliderConfigError::NonFiniteValue`] for NaN or infinity.
    pub const fn new(value: f32) -> Result<Self, SliderConfigError> {
        if value.is_finite() {
            Ok(Self::snapping(value))
        } else {
            Err(SliderConfigError::NonFiniteValue)
        }
    }

    /// Returns the finite raw-domain value.
    #[must_use]
    pub const fn get(self) -> f32 { self.value }

    pub(super) const fn snapping(value: f32) -> Self {
        Self {
            value,
            stepping: SliderProposalStepping::SnapToStep,
        }
    }

    pub(super) const fn preserving(value: f32) -> Self {
        Self {
            value,
            stepping: SliderProposalStepping::PreserveValue,
        }
    }
}

/// Value computation for a [`RequestSliderAdjustment`](super::RequestSliderAdjustment).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderAdjustment {
    /// Proposes the given raw-domain value.
    Absolute(f32),
    /// Proposes the applied value plus the given raw-domain delta.
    Relative(f32),
    /// Proposes the applied value plus the given number of step intervals.
    ///
    /// Emits no proposal when the slider has no step.
    RelativeSteps(f32),
}

/// Invalid slider authoring configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SliderConfigError {
    /// A range endpoint was non-finite.
    #[error("slider range endpoints must be finite")]
    NonFiniteRange,
    /// The range start was not strictly less than the range end.
    #[error("slider range start must be less than its end")]
    UnorderedRange,
    /// A logarithmic range started at zero or below.
    #[error("logarithmic slider range start must be positive")]
    NonPositiveLogRange,
    /// A slider value was non-finite.
    #[error("slider value must be finite")]
    NonFiniteValue,
    /// A slider step was non-finite, zero, or negative.
    #[error("slider step must be finite and positive")]
    NonPositiveStep,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::EntityEvent;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;

    use super::super::*;
    use super::*;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;

    #[derive(Default, Resource)]
    struct RecordedProposals(Vec<(Entity, f32, bool, SliderProposalSource)>);

    fn record_proposal(change: On<SliderChangeRequested>, mut recorded: ResMut<RecordedProposals>) {
        recorded.0.push((
            change.event_target(),
            change.value.get(),
            change.is_final,
            change.source,
        ));
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .init_resource::<RecordedProposals>()
            .add_observer(record_proposal);
        app
    }

    fn range(start: f32, end: f32) -> SliderRange {
        SliderRange::new(start, end).expect("range should validate")
    }

    fn step(interval: f32) -> SliderStep {
        SliderStep::new(interval).expect("step should validate")
    }

    fn slider_tree(id: &str, slider: Slider) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(20.0, 10.0).widget(id, slider), |_| {});
        builder.build()
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build()
            .expect("panel should build");
        app.world_mut().spawn(panel).id()
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &str) -> Entity {
        let id = PanelElementId::named(id);
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .ok()
            .flatten()
            .expect("widget should be reified")
    }

    fn slider_value(app: &App, widget: Entity) -> f32 {
        app.world()
            .get::<SliderState>(widget)
            .expect("widget should carry slider state")
            .value()
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn logarithmic_slider_preserves_real_value_pushed_band_and_default_bits() {
        let mut app = test_app();
        let authored = Slider::new(0.01..=10.0)
            .logarithmic()
            .default_value(0.1)
            .value(0.42);
        let panel = spawn_panel(&mut app, slider_tree("size", authored.clone()));
        app.update();
        let widget = resolve_widget(&mut app, panel, "size");
        assert_eq!(slider_value(&app, widget).to_bits(), 0.42_f32.to_bits());
        let mut state = app
            .world_mut()
            .get_mut::<SliderState>(widget)
            .expect("log slider state");
        state.set_value(0.173).expect("finite authored value");
        state.set_pushed(SliderPushedValue::PushedTo {
            value:      0.75,
            band_start: 0.02,
            band_end:   2.0,
        });
        app.update();
        let state = app
            .world()
            .get::<SliderState>(widget)
            .expect("retained log state");
        assert_eq!(state.value().to_bits(), 0.173_f32.to_bits());
        assert_eq!(
            state.pushed(),
            SliderPushedValue::PushedTo {
                value:      0.75,
                band_start: 0.02,
                band_end:   2.0,
            },
            "live log signal must retain its real interval"
        );
        app.world_mut()
            .commands()
            .set_tree(panel, slider_tree("size", authored.value(3.2)))
            .expect("same slider rebuilt from a new authored value");
        app.update();
        assert_eq!(
            slider_value(&app, widget).to_bits(),
            0.173_f32.to_bits(),
            "rebuild preserves accepted real value"
        );
        assert_eq!(
            Slider::new(0.01..=10.0)
                .logarithmic()
                .default_value(0.1)
                .initial_state()
                .value()
                .to_bits(),
            0.1_f32.to_bits(),
            "authored reset target initializes without a log conversion round trip"
        );
    }

    #[test]
    fn range_rejects_non_finite_endpoints() {
        assert_eq!(
            SliderRange::new(f32::NAN, 1.0),
            Err(SliderConfigError::NonFiniteRange)
        );
        assert_eq!(
            SliderRange::new(0.0, f32::INFINITY),
            Err(SliderConfigError::NonFiniteRange)
        );
    }

    #[test]
    fn range_rejects_unordered_endpoints() {
        assert_eq!(
            SliderRange::new(1.0, 1.0),
            Err(SliderConfigError::UnorderedRange)
        );
        assert_eq!(
            SliderRange::new(2.0, 1.0),
            Err(SliderConfigError::UnorderedRange)
        );
    }

    #[test]
    fn step_rejects_non_positive_or_non_finite_values() {
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                SliderStep::new(invalid),
                Err(SliderConfigError::NonPositiveStep)
            );
        }
    }

    #[test]
    fn error_messages_are_stable() {
        let messages = [
            (
                SliderConfigError::NonFiniteRange,
                "slider range endpoints must be finite",
            ),
            (
                SliderConfigError::UnorderedRange,
                "slider range start must be less than its end",
            ),
            (
                SliderConfigError::NonFiniteValue,
                "slider value must be finite",
            ),
            (
                SliderConfigError::NonPositiveStep,
                "slider step must be finite and positive",
            ),
        ];

        for (error, message) in messages {
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn accepted_slider_proposal_snaps_to_lattice_anchored_at_range_start() {
        // The lattice for range start 1.0 and step 0.4 is 1.0, 1.4, 1.8; a
        // zero-anchored lattice would land on 1.6 instead.
        let mut state = SliderState::new(
            range(1.0, 2.0),
            1.55,
            SliderStepping::Stepped(step(0.4)),
            SliderDirection::LeftToRight,
        )
        .expect("state should validate");
        assert_eq!(
            state.value().to_bits(),
            1.55_f32.to_bits(),
            "code-set values remain off-lattice"
        );
        assert!(state.set_proposed_value(SliderProposalValue::new(1.55).expect("finite proposal")));
        assert_close(state.value(), 1.4);
    }

    #[test]
    fn slider_proposals_accept_only_finite_values_and_apply_infallibly() {
        assert_eq!(
            SliderProposalValue::new(f32::NAN),
            Err(SliderConfigError::NonFiniteValue)
        );
        assert_eq!(
            SliderProposalValue::new(f32::INFINITY),
            Err(SliderConfigError::NonFiniteValue)
        );
        let mut state = SliderState::new(
            range(0.0, 1.0),
            0.0,
            SliderStepping::Stepless,
            SliderDirection::LeftToRight,
        )
        .expect("finite slider state");
        let proposal = SliderProposalValue::new(0.75).expect("finite proposal");
        assert!(state.set_proposed_value(proposal));
        assert_close(state.value(), 0.75);
    }

    #[test]
    fn accepted_slider_proposals_clamp_after_snapping() {
        let mut state = SliderState::new(
            range(0.0, 1.0),
            0.5,
            SliderStepping::Stepped(step(0.4)),
            SliderDirection::LeftToRight,
        )
        .expect("state should validate");

        // Snapping 5.0 lands on 5.2's lattice point far above the range; the
        // final clamp keeps the range end even though it is off-lattice.
        assert!(state.set_proposed_value(SliderProposalValue::new(5.0).expect("finite proposal")));
        assert_close(state.value(), 1.0);
        assert!(state.set_proposed_value(SliderProposalValue::new(-3.0).expect("finite proposal")));
        assert_close(state.value(), 0.0);
    }

    #[test]
    fn state_rejects_non_finite_values_without_touching_applied() {
        assert_eq!(
            SliderState::new(
                range(0.0, 1.0),
                f32::NAN,
                SliderStepping::Stepless,
                SliderDirection::LeftToRight
            ),
            Err(SliderConfigError::NonFiniteValue)
        );

        let mut state = SliderState::new(
            range(0.0, 1.0),
            0.5,
            SliderStepping::Stepless,
            SliderDirection::LeftToRight,
        )
        .expect("state should validate");
        assert_eq!(
            state.set_value(f32::INFINITY),
            Err(SliderConfigError::NonFiniteValue)
        );
        assert_close(state.value(), 0.5);
    }

    #[test]
    fn adjusted_value_names_non_finite_and_stepless_rejections() {
        let state = SliderState::new(
            range(0.0, 1.0),
            0.5,
            SliderStepping::Stepless,
            SliderDirection::LeftToRight,
        )
        .expect("state should validate");

        assert_eq!(
            state.adjusted_value(SliderAdjustment::Absolute(f32::NAN)),
            Err(SliderAdjustmentRejected::NonFiniteInput)
        );
        assert_eq!(
            state.adjusted_value(SliderAdjustment::RelativeSteps(1.0)),
            Err(SliderAdjustmentRejected::StepOnStepless)
        );
    }

    #[test]
    fn set_value_reports_whether_the_applied_value_changed() {
        let mut state = SliderState::new(
            range(0.0, 1.0),
            0.5,
            SliderStepping::Stepped(step(0.25)),
            SliderDirection::LeftToRight,
        )
        .expect("state should validate");

        assert_eq!(state.set_value(0.55), Ok(true));
        assert_eq!(state.value().to_bits(), 0.55_f32.to_bits());
        assert_eq!(state.set_value(0.55), Ok(false));
        assert_eq!(state.set_value(0.7), Ok(true));
        assert_eq!(state.value().to_bits(), 0.7_f32.to_bits());

        // -0.0 equals 0.0 numerically, so replacing one with the other is
        // not a change.
        let mut state = SliderState::new(
            range(-1.0, 1.0),
            0.0,
            SliderStepping::Stepless,
            SliderDirection::LeftToRight,
        )
        .expect("state should validate");
        assert_eq!(state.set_value(-0.0), Ok(false));
    }

    #[test]
    fn reify_retains_each_authored_direction() {
        let directions = [
            ("ltr", SliderDirection::LeftToRight),
            ("rtl", SliderDirection::RightToLeft),
            ("btt", SliderDirection::BottomToTop),
            ("ttb", SliderDirection::TopToBottom),
        ];
        let mut app = test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        for (id, direction) in directions {
            let slider = Slider::new(0.0..=1.0).value(0.5).direction(direction);
            builder.with(El::new().size(20.0, 10.0).widget(id, slider), |_| {});
        }
        let panel = spawn_panel(&mut app, builder.build());
        app.update();

        for (id, direction) in directions {
            let widget = resolve_widget(&mut app, panel, id);
            let state = app
                .world()
                .get::<SliderState>(widget)
                .expect("slider should carry state");
            assert_eq!(state.direction(), direction);
        }
    }

    #[test]
    fn first_spawn_clamps_out_of_range_defaults_and_preserves_off_step_defaults() {
        let mut app = test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        for (id, default_value) in [("over", 25.0), ("off", 4.4)] {
            let slider = Slider::new(0.0..=10.0)
                .default_value(default_value)
                .step(3.0);
            builder.with(El::new().size(20.0, 10.0).widget(id, slider), |_| {});
        }
        let panel = spawn_panel(&mut app, builder.build());
        app.update();

        let over = resolve_widget(&mut app, panel, "over");
        assert_close(slider_value(&app, over), 10.0);
        let off = resolve_widget(&mut app, panel, "off");
        assert_eq!(slider_value(&app, off).to_bits(), 4.4_f32.to_bits());
    }

    #[test]
    fn reuse_preserves_live_value_and_revalidates_configuration_changes() {
        let mut app = test_app();
        let slider = Slider::new(0.0..=10.0).default_value(4.0);
        let panel = spawn_panel(&mut app, slider_tree("level", slider));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let applied = app
            .world_mut()
            .get_mut::<SliderState>(widget)
            .expect("widget should carry slider state")
            .set_value(9.0);
        assert_eq!(applied, Ok(true));
        let live_tick = app
            .world()
            .entity(widget)
            .get_ref::<SliderState>()
            .expect("widget should carry slider state")
            .last_changed();

        // An authored default-value change alone does not rewrite the reused
        // widget's live value or its state component.
        let replacement = Slider::new(0.0..=10.0).default_value(2.0);
        app.world_mut()
            .commands()
            .set_tree(panel, slider_tree("level", replacement))
            .expect("replacement tree should be accepted");
        app.update();
        assert_eq!(resolve_widget(&mut app, panel, "level"), widget);
        assert_close(slider_value(&app, widget), 9.0);
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<SliderState>()
                .expect("widget should retain slider state")
                .last_changed(),
            live_tick,
            "an unrelated reify must not rewrite slider state",
        );

        // An authored range change updates the configuration and revalidates
        // the preserved live value against it.
        let narrowed = Slider::new(0.0..=5.0).default_value(2.0);
        app.world_mut()
            .commands()
            .set_tree(panel, slider_tree("level", narrowed))
            .expect("narrowed tree should be accepted");
        app.update();
        let state = app
            .world()
            .get::<SliderState>(widget)
            .expect("widget should retain slider state");
        assert_close(state.range().end(), 5.0);
        assert_close(state.value(), 5.0);
    }
}
