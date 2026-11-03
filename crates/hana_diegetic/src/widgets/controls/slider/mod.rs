mod adjustment;
mod drag;
mod face;
mod presentation;
mod projection;
mod slider_state;

use std::ops::RangeInclusive;

pub use adjustment::RequestSliderAdjustment;
pub use adjustment::SliderChangeRequested;
pub use adjustment::SliderProposalSource;
use adjustment::handle_adjustment_request;
pub use adjustment::slider_self_update;
use bevy::picking::PickingSystems;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::ResMut;
pub use drag::SliderCancelCause;
pub use drag::SliderCanceled;
pub(crate) use drag::SliderCaptures;
pub(crate) use drag::SliderDrag;
pub use drag::SliderGrabbed;
pub use drag::SliderReleased;
pub(in crate::widgets) use drag::SliderWidget;
use drag::cancel_before_widget_despawn;
use drag::cancel_from_disabled;
use drag::cancel_from_pointer;
use drag::cancel_from_pointer_removal;
use drag::cancel_from_widget_removal;
pub(in crate::widgets) use drag::cancel_slider_drag;
pub(in crate::widgets) use drag::capture_reconciled_slider_press;
use drag::drag_from_pointer;
pub(crate) use drag::finalize_panel_sliders;
use drag::grab_from_pointer;
use drag::handle_semantic_intent;
use drag::release_from_drag_end;
use drag::release_from_pointer;
use drag::reset_from_pointer;
pub(in crate::widgets) use drag::resolve_release;
pub use face::SliderReachableBand;
pub(in crate::widgets) use presentation::present_slider_state;
pub(in crate::widgets) use projection::SliderProjection;
pub use slider_state::SliderAdjustment;
pub use slider_state::SliderConfigError;
pub use slider_state::SliderDirection;
pub use slider_state::SliderProposalValue;
pub use slider_state::SliderPushedValue;
pub use slider_state::SliderRange;
use slider_state::SliderScale;
pub use slider_state::SliderState;
pub use slider_state::SliderStep;
pub use slider_state::SliderStepping;

/// Refuses authoring while retaining the slider's full-colour live presentation.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SliderAuthoringLock;

fn cancel_locked_slider(
    lock: On<Add<SliderAuthoringLock>>,
    mut captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    captures.clear_reset_widget(lock.entity);
    cancel_slider_drag(
        lock.entity,
        SliderCancelCause::AuthoringLocked,
        &mut captures,
        &mut commands,
    );
}

/// Registers slider runtime state, pointer capture, and adjustment handling.
pub(in crate::widgets) struct SliderPlugin;

impl Plugin for SliderPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::FineAdjustmentPlugin>() {
            app.add_plugins(crate::FineAdjustmentPlugin);
        }
        app.init_resource::<SliderCaptures>()
            .add_systems(
                PreUpdate,
                drag::preempt_for_orbit
                    .before(PickingSystems::Hover)
                    .run_if(
                        bevy::prelude::resource_exists::<
                            bevy::prelude::Messages<bevy::picking::pointer::PointerInput>,
                        >,
                    ),
            )
            .add_systems(
                PreUpdate,
                drag::reanchor_precision
                    .after(crate::FineAdjustmentSystems)
                    .before(PickingSystems::ProcessInput),
            )
            .add_observer(handle_adjustment_request)
            .add_observer(grab_from_pointer)
            .add_observer(drag_from_pointer)
            .add_observer(release_from_pointer)
            .add_observer(reset_from_pointer)
            .add_observer(release_from_drag_end)
            .add_observer(cancel_from_pointer)
            .add_observer(cancel_from_pointer_removal)
            .add_observer(cancel_from_disabled)
            .add_observer(cancel_locked_slider)
            .add_observer(cancel_from_widget_removal)
            .add_observer(cancel_before_widget_despawn)
            .add_observer(handle_semantic_intent);
    }
}

/// The step as declared on a [`Slider`], before [`Slider::validated`] checks it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SliderStepDeclaration {
    /// No step was declared: any value in the range is allowed.
    Stepless,
    /// The interval passed to [`Slider::step`], not yet checked by [`SliderStep::new`].
    Interval(f32),
}

/// The initial live value, separate from the authored reset target.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SliderInitialValue {
    AuthoredDefault,
    Value(f32),
}

/// Authored configuration for a panel slider.
///
/// Declare one inline with [`El::slider`](crate::El::slider), or build one here
/// and attach it with [`El::widget`](crate::El::widget). Reified sliders store
/// their applied value in [`SliderState`] and propose changes through
/// [`SliderChangeRequested`]. A slider carries behavior; the root's resting
/// look stays on the element's ordinary [`El::background`](crate::El::background),
/// [`El::border`](crate::El::border), and [`El::material`](crate::El::material)
/// declarations, and its per-state look on the element's state builders.
///
/// Range, value, and step are stored as authored and validated when the owning
/// panel builds, so every builder here is infallible and the first invalid
/// value surfaces as [`PanelBuildError::SliderConfig`](crate::PanelBuildError).
#[must_use]
#[derive(Clone, Debug, PartialEq)]
pub struct Slider {
    range:         RangeInclusive<f32>,
    scale:         SliderScale,
    default_value: f32,
    initial_value: SliderInitialValue,
    step:          SliderStepDeclaration,
    direction:     SliderDirection,
}

impl Slider {
    /// Creates a slider declaration over `range` whose default value is the
    /// range start.
    ///
    /// The default initializes the first runtime [`SliderState`] unless
    /// [`Self::value`] supplies a current value. A primary-button double-click
    /// on the thumb always proposes the default.
    pub fn new(range: RangeInclusive<f32>) -> Self {
        let default_value = *range.start();
        Self {
            range,
            scale: SliderScale::Linear,
            default_value,
            initial_value: SliderInitialValue::AuthoredDefault,
            step: SliderStepDeclaration::Stepless,
            direction: SliderDirection::default(),
        }
    }

    /// Distributes values logarithmically along the track. Panel build rejects
    /// a range whose start is not strictly positive.
    pub const fn logarithmic(mut self) -> Self {
        self.scale = SliderScale::Logarithmic;
        self
    }

    /// Sets the stable target for a thumb double-click. Unless [`Self::value`]
    /// supplies a current value, this also initializes the first runtime state.
    pub const fn default_value(mut self, value: f32) -> Self {
        self.default_value = value;
        self
    }

    /// Sets the initial live value without changing the reset default.
    ///
    /// Use this when rebuilding a panel from saved or current values, together
    /// with [`Self::default_value`] for the authored reset target. Reification
    /// preserves an existing slider's accepted live value.
    pub const fn value(mut self, value: f32) -> Self {
        self.initial_value = SliderInitialValue::Value(value);
        self
    }

    /// Sets the step interval.
    pub const fn step(mut self, step: f32) -> Self {
        self.step = SliderStepDeclaration::Interval(step);
        self
    }

    /// Sets the direction in which values increase.
    pub const fn direction(mut self, direction: SliderDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Validates the authored range, default value, and step together, building
    /// the [`SliderState`] they describe.
    ///
    /// Panel build calls this once per authored slider and keeps only the
    /// `Result`; every later read goes through [`Slider::initial_state`], which
    /// repeats the same conversion and falls back to the authored range's start
    /// when a value cannot be validated.
    pub(crate) fn validated(&self) -> Result<SliderState, SliderConfigError> {
        let range = match self.scale {
            SliderScale::Linear => SliderRange::new(*self.range.start(), *self.range.end())?,
            SliderScale::Logarithmic => {
                SliderRange::logarithmic(*self.range.start(), *self.range.end())?
            },
        };
        let value = match self.initial_value {
            SliderInitialValue::AuthoredDefault => self.default_value,
            SliderInitialValue::Value(value) => value,
        };
        if !self.default_value.is_finite() || !value.is_finite() {
            return Err(SliderConfigError::NonFiniteValue);
        }
        let stepping = match self.step {
            SliderStepDeclaration::Stepless => SliderStepping::Stepless,
            SliderStepDeclaration::Interval(interval) => {
                SliderStepping::Stepped(SliderStep::new(interval)?)
            },
        };
        Ok(SliderState::from_validated(
            range,
            value,
            stepping,
            self.direction,
        ))
    }

    /// Builds the first-spawn runtime state for this authored slider.
    ///
    /// The initial live value (or the default when none is supplied) initializes
    /// only the first state. Later reifications preserve the accepted live value
    /// while retaining the independently declared default for reset proposals.
    /// Panel build rejects an unvalidatable declaration, so the fallback state
    /// here is never reached by a reified slider.
    pub(in crate::widgets) fn initial_state(&self) -> SliderState {
        self.validated().unwrap_or_else(|_| {
            SliderState::from_validated(
                SliderRange::UNIT,
                0.0,
                SliderStepping::Stepless,
                self.direction,
            )
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    fn range(start: f32, end: f32) -> SliderRange {
        SliderRange::new(start, end).expect("range should validate")
    }

    fn step(interval: f32) -> SliderStep {
        SliderStep::new(interval).expect("step should validate")
    }

    #[test]
    fn slider_defers_a_non_finite_default_value_to_validation() {
        assert_eq!(
            Slider::new(0.0..=1.0).default_value(f32::NAN).validated(),
            Err(SliderConfigError::NonFiniteValue)
        );
    }

    #[test]
    fn slider_defers_an_unordered_range_to_validation() {
        assert_eq!(
            Slider::new(2.0..=1.0).validated(),
            Err(SliderConfigError::UnorderedRange)
        );
    }

    #[test]
    fn slider_defers_a_non_positive_step_to_validation() {
        assert_eq!(
            Slider::new(0.0..=1.0).step(0.0).validated(),
            Err(SliderConfigError::NonPositiveStep)
        );
    }

    #[test]
    fn valid_slider_retains_authored_configuration() {
        let slider = Slider::new(-1.0..=2.0)
            .default_value(0.5)
            .value(1.0)
            .step(0.25)
            .direction(SliderDirection::TopToBottom);
        let config = slider.validated().expect("authored slider validates");

        assert_eq!(config.range(), range(-1.0, 2.0));
        assert!((config.value() - 1.0).abs() <= f32::EPSILON);
        assert!((slider.default_value - 0.5).abs() <= f32::EPSILON);
        assert_eq!(config.stepping(), SliderStepping::Stepped(step(0.25)));
        assert_eq!(slider.direction, SliderDirection::TopToBottom);
    }

    #[test]
    fn current_value_never_replaces_the_reset_default() {
        let first = Slider::new(0.0..=10.0).default_value(2.0).value(8.0);
        let rebuilt = first.clone().value(9.0);
        assert!((rebuilt.default_value - 2.0).abs() <= f32::EPSILON);
        assert!((rebuilt.initial_state().value() - 9.0).abs() <= f32::EPSILON);
        let reversed = Slider::new(0.0..=10.0).value(8.0).default_value(2.0);
        assert_eq!(first, reversed);
        let from_default = Slider::new(0.0..=10.0).default_value(2.0).initial_state();
        assert!((from_default.value() - 2.0).abs() <= f32::EPSILON);
        for slider in [
            Slider::new(0.0..=10.0).default_value(f32::NAN).value(1.0),
            Slider::new(0.0..=10.0).default_value(2.0).value(f32::NAN),
        ] {
            assert_eq!(slider.validated(), Err(SliderConfigError::NonFiniteValue));
        }
    }
}
