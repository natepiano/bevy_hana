use bevy::prelude::Component;
use bevy::prelude::KeyCode;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::InputModKeys;
use bevy_enhanced_input::prelude::ModKeys;
use thiserror::Error;

use super::routing::WidgetStepDirection;

/// Per-window selection of the widget input bindings [`super::WidgetInputPlugin`]
/// installs.
#[derive(Component, Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Component, Default)]
#[require(WidgetInputModeInitialized)]
pub enum WidgetInputMode {
    /// Installs Tab, Shift+Tab, Enter or Space, Escape, and the four arrows.
    #[default]
    Default,
    /// Installs the supplied bindings for this window.
    Bindings(WidgetInputBindings),
    /// Leaves input context ownership to the application.
    Manual,
}

impl WidgetInputMode {
    /// Returns display labels for the controls selected by this mode.
    #[must_use]
    pub fn control_summary(&self) -> WidgetControlSummary {
        match self {
            Self::Default => WidgetInputBindings::default().control_summary(),
            Self::Bindings(bindings) => bindings.control_summary(),
            Self::Manual => WidgetControlSummary::default(),
        }
    }
}

/// Validated per-action bindings used by [`WidgetInputMode::Bindings`].
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct WidgetInputBindings {
    pub(super) next:     Vec<Binding>,
    pub(super) previous: Vec<Binding>,
    pub(super) first:    Vec<Binding>,
    pub(super) last:     Vec<Binding>,
    pub(super) activate: Vec<Binding>,
    pub(super) cancel:   Vec<Binding>,
    pub(super) steps:    Vec<WidgetStepBinding>,
}

impl WidgetInputBindings {
    /// Starts a custom widget-input binding definition.
    #[must_use]
    pub fn builder() -> WidgetInputBindingsBuilder { WidgetInputBindingsBuilder::default() }

    fn control_summary(&self) -> WidgetControlSummary {
        WidgetControlSummary {
            next:       binding_labels(&self.next),
            previous:   binding_labels(&self.previous),
            first:      binding_labels(&self.first),
            last:       binding_labels(&self.last),
            activate:   binding_labels(&self.activate),
            cancel:     binding_labels(&self.cancel),
            step_left:  step_binding_labels(&self.steps, WidgetStepDirection::Left),
            step_right: step_binding_labels(&self.steps, WidgetStepDirection::Right),
            step_up:    step_binding_labels(&self.steps, WidgetStepDirection::Up),
            step_down:  step_binding_labels(&self.steps, WidgetStepDirection::Down),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub(super) struct WidgetStepBinding {
    pub(super) direction: WidgetStepDirection,
    pub(super) binding:   Binding,
}

impl Default for WidgetInputBindings {
    fn default() -> Self {
        Self {
            next:     vec![KeyCode::Tab.into()],
            previous: vec![KeyCode::Tab.with_mod_keys(ModKeys::SHIFT)],
            first:    Vec::new(),
            last:     Vec::new(),
            activate: vec![KeyCode::Enter.into(), KeyCode::Space.into()],
            cancel:   vec![KeyCode::Escape.into()],
            steps:    vec![
                WidgetStepBinding {
                    direction: WidgetStepDirection::Left,
                    binding:   KeyCode::ArrowLeft.into(),
                },
                WidgetStepBinding {
                    direction: WidgetStepDirection::Right,
                    binding:   KeyCode::ArrowRight.into(),
                },
                WidgetStepBinding {
                    direction: WidgetStepDirection::Up,
                    binding:   KeyCode::ArrowUp.into(),
                },
                WidgetStepBinding {
                    direction: WidgetStepDirection::Down,
                    binding:   KeyCode::ArrowDown.into(),
                },
            ],
        }
    }
}

/// Consuming builder for validated per-window widget bindings.
#[derive(Clone, Debug, Default)]
pub struct WidgetInputBindingsBuilder {
    bindings: WidgetInputBindingsBuilderFields,
}

impl WidgetInputBindingsBuilder {
    /// Adds a binding alternative for focus-next.
    #[must_use]
    pub fn next(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.next.push(binding.into());
        self
    }

    /// Adds a binding alternative for focus-previous.
    #[must_use]
    pub fn previous(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.previous.push(binding.into());
        self
    }

    /// Adds a binding alternative for focus-first.
    #[must_use]
    pub fn first(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.first.push(binding.into());
        self
    }

    /// Adds a binding alternative for focus-last.
    #[must_use]
    pub fn last(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.last.push(binding.into());
        self
    }

    /// Adds a binding alternative for activation.
    #[must_use]
    pub fn activate(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.activate.push(binding.into());
        self
    }

    /// Adds a binding alternative for cancellation.
    #[must_use]
    pub fn cancel(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.cancel.push(binding.into());
        self
    }

    /// Adds a binding alternative for a left slider step.
    #[must_use]
    pub fn step_left(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.step_left.push(binding.into());
        self
    }

    /// Adds a binding alternative for a right slider step.
    #[must_use]
    pub fn step_right(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.step_right.push(binding.into());
        self
    }

    /// Adds a binding alternative for an upward slider step.
    #[must_use]
    pub fn step_up(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.step_up.push(binding.into());
        self
    }

    /// Adds a binding alternative for a downward slider step.
    #[must_use]
    pub fn step_down(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.step_down.push(binding.into());
        self
    }

    /// Validates and returns the custom binding collection.
    ///
    /// # Errors
    ///
    /// Returns [`WidgetInputBindingsError::NoneBinding`] for [`Binding::None`]
    /// or [`WidgetInputBindingsError::ConflictingBinding`] when two actions use
    /// the same exact binding.
    pub fn build(self) -> Result<WidgetInputBindings, WidgetInputBindingsError> {
        let mut fields = self.bindings;
        let mut assigned = Vec::new();
        validate_action_bindings(&mut fields.next, &mut assigned)?;
        validate_action_bindings(&mut fields.previous, &mut assigned)?;
        validate_action_bindings(&mut fields.first, &mut assigned)?;
        validate_action_bindings(&mut fields.last, &mut assigned)?;
        validate_action_bindings(&mut fields.activate, &mut assigned)?;
        validate_action_bindings(&mut fields.cancel, &mut assigned)?;
        validate_action_bindings(&mut fields.step_left, &mut assigned)?;
        validate_action_bindings(&mut fields.step_right, &mut assigned)?;
        validate_action_bindings(&mut fields.step_up, &mut assigned)?;
        validate_action_bindings(&mut fields.step_down, &mut assigned)?;
        Ok(WidgetInputBindings {
            next:     fields.next,
            previous: fields.previous,
            first:    fields.first,
            last:     fields.last,
            activate: fields.activate,
            cancel:   fields.cancel,
            steps:    [
                (WidgetStepDirection::Left, fields.step_left),
                (WidgetStepDirection::Right, fields.step_right),
                (WidgetStepDirection::Up, fields.step_up),
                (WidgetStepDirection::Down, fields.step_down),
            ]
            .into_iter()
            .flat_map(|(direction, bindings)| {
                bindings
                    .into_iter()
                    .map(move |binding| WidgetStepBinding { direction, binding })
            })
            .collect(),
        })
    }
}

#[derive(Clone, Debug, Default)]
struct WidgetInputBindingsBuilderFields {
    next:       Vec<Binding>,
    previous:   Vec<Binding>,
    first:      Vec<Binding>,
    last:       Vec<Binding>,
    activate:   Vec<Binding>,
    cancel:     Vec<Binding>,
    step_left:  Vec<Binding>,
    step_right: Vec<Binding>,
    step_up:    Vec<Binding>,
    step_down:  Vec<Binding>,
}

/// Validation error from [`WidgetInputBindingsBuilder::build`].
#[derive(Clone, Debug, Error, PartialEq)]
pub enum WidgetInputBindingsError {
    /// A binding was the non-input sentinel.
    #[error("widget input bindings cannot contain Binding::None")]
    NoneBinding,
    /// One exact binding was assigned to more than one semantic action.
    #[error("widget input binding `{0}` is assigned to multiple actions")]
    ConflictingBinding(Binding),
}

/// Point-in-time labels for one window's effective widget controls.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WidgetControlSummary {
    /// Focus-next binding labels.
    pub next:       Vec<String>,
    /// Focus-previous binding labels.
    pub previous:   Vec<String>,
    /// Focus-first binding labels.
    pub first:      Vec<String>,
    /// Focus-last binding labels.
    pub last:       Vec<String>,
    /// Activation binding labels.
    pub activate:   Vec<String>,
    /// Cancellation binding labels.
    pub cancel:     Vec<String>,
    /// Left-step binding labels.
    pub step_left:  Vec<String>,
    /// Right-step binding labels.
    pub step_right: Vec<String>,
    /// Up-step binding labels.
    pub step_up:    Vec<String>,
    /// Down-step binding labels.
    pub step_down:  Vec<String>,
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub(super) struct WidgetInputModeInitialized;

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WidgetInputAction {
    Next,
    Previous,
    First,
    Last,
    Activate,
    Cancel,
    Step(WidgetStepDirection),
}

fn validate_action_bindings(
    bindings: &mut Vec<Binding>,
    assigned: &mut Vec<Binding>,
) -> Result<(), WidgetInputBindingsError> {
    let mut unique = Vec::with_capacity(bindings.len());
    for binding in bindings.drain(..) {
        if binding == Binding::None {
            return Err(WidgetInputBindingsError::NoneBinding);
        }
        if unique.contains(&binding) {
            continue;
        }
        if assigned.contains(&binding) {
            return Err(WidgetInputBindingsError::ConflictingBinding(binding));
        }
        assigned.push(binding);
        unique.push(binding);
    }
    *bindings = unique;
    Ok(())
}

fn binding_labels(bindings: &[Binding]) -> Vec<String> {
    bindings.iter().map(ToString::to_string).collect()
}

fn step_binding_labels(steps: &[WidgetStepBinding], direction: WidgetStepDirection) -> Vec<String> {
    steps
        .iter()
        .filter(|step| step.direction == direction)
        .map(|step| step.binding.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use bevy::prelude::KeyCode;
    use bevy_enhanced_input::prelude::Binding;
    use bevy_enhanced_input::prelude::InputModKeys;
    use bevy_enhanced_input::prelude::ModKeys;

    use super::WidgetControlSummary;
    use super::WidgetInputBindings;
    use super::WidgetInputBindingsError;
    use super::WidgetInputMode;
    use super::WidgetStepBinding;
    use super::WidgetStepDirection;

    #[test]
    fn binding_builder_adds_alternatives_deduplicates_and_permits_omissions() {
        let result = WidgetInputBindings::builder()
            .next(KeyCode::Tab)
            .activate(KeyCode::Enter)
            .activate(KeyCode::Enter)
            .activate(KeyCode::Space)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };

        assert_eq!(bindings.next, vec![Binding::from(KeyCode::Tab)]);
        assert_eq!(bindings.previous, []);
        assert_eq!(bindings.first, []);
        assert_eq!(bindings.last, []);
        assert_eq!(
            bindings.activate,
            vec![Binding::from(KeyCode::Enter), Binding::from(KeyCode::Space),]
        );
        assert_eq!(bindings.cancel, []);
        assert_eq!(bindings.steps, []);
    }

    #[test]
    fn binding_builder_rejects_none_and_cross_action_conflicts() {
        assert_eq!(
            WidgetInputBindings::builder().next(Binding::None).build(),
            Err(WidgetInputBindingsError::NoneBinding)
        );

        let tab = Binding::from(KeyCode::Tab);
        assert_eq!(
            WidgetInputBindings::builder()
                .next(tab)
                .activate(tab)
                .build(),
            Err(WidgetInputBindingsError::ConflictingBinding(tab))
        );
    }

    #[test]
    fn step_bindings_validate_like_other_widget_actions() {
        let left = Binding::from(KeyCode::ArrowLeft);
        assert_eq!(
            WidgetInputBindings::builder()
                .step_left(left)
                .step_right(left)
                .build(),
            Err(WidgetInputBindingsError::ConflictingBinding(left))
        );
        assert_eq!(
            WidgetInputBindings::builder()
                .step_up(Binding::None)
                .build(),
            Err(WidgetInputBindingsError::NoneBinding)
        );
        let result = WidgetInputBindings::builder()
            .step_down(KeyCode::ArrowDown)
            .step_down(KeyCode::ArrowDown)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else { return };
        assert_eq!(
            bindings.steps,
            [WidgetStepBinding {
                direction: WidgetStepDirection::Down,
                binding:   Binding::from(KeyCode::ArrowDown),
            }]
        );
    }

    #[test]
    fn binding_error_messages_are_stable() {
        assert_eq!(
            WidgetInputBindingsError::NoneBinding.to_string(),
            "widget input bindings cannot contain Binding::None"
        );
        assert_eq!(
            WidgetInputBindingsError::ConflictingBinding(KeyCode::Tab.into()).to_string(),
            "widget input binding `Tab` is assigned to multiple actions"
        );
    }

    #[test]
    fn control_summary_describes_defaults_custom_bindings_and_manual_mode() {
        let defaults = WidgetInputMode::Default.control_summary();
        assert_eq!(defaults.next, vec![Binding::from(KeyCode::Tab).to_string()]);
        assert_eq!(
            defaults.previous,
            vec![KeyCode::Tab.with_mod_keys(ModKeys::SHIFT).to_string()]
        );
        assert_eq!(defaults.first.len(), 0);
        assert_eq!(defaults.last.len(), 0);
        assert_eq!(
            defaults.step_left,
            vec![Binding::from(KeyCode::ArrowLeft).to_string()]
        );
        assert_eq!(
            defaults.step_right,
            vec![Binding::from(KeyCode::ArrowRight).to_string()]
        );
        assert_eq!(
            defaults.step_up,
            vec![Binding::from(KeyCode::ArrowUp).to_string()]
        );
        assert_eq!(
            defaults.step_down,
            vec![Binding::from(KeyCode::ArrowDown).to_string()]
        );
        assert_eq!(
            defaults.activate,
            vec![
                Binding::from(KeyCode::Enter).to_string(),
                Binding::from(KeyCode::Space).to_string(),
            ]
        );

        let result = WidgetInputBindings::builder()
            .next(KeyCode::ArrowDown)
            .cancel(KeyCode::Backspace)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let custom = WidgetInputMode::Bindings(bindings).control_summary();
        assert_eq!(
            custom.next,
            vec![Binding::from(KeyCode::ArrowDown).to_string()]
        );
        assert_eq!(
            custom.cancel,
            vec![Binding::from(KeyCode::Backspace).to_string()]
        );
        assert_eq!(custom.previous.len(), 0);
        assert_eq!(
            WidgetInputMode::Manual.control_summary(),
            WidgetControlSummary::default()
        );
    }
}
