mod adapter;
mod bindings;
mod gate;
mod routing;

use adapter::CancelWidgetAction;
use adapter::FirstWidgetAction;
use adapter::HeldWidgetActivations;
use adapter::LastWidgetAction;
use adapter::NextWidgetAction;
use adapter::PendingActivateEndings;
use adapter::PendingActivateStarts;
use adapter::PendingWidgetInputActions;
use adapter::PreviousWidgetAction;
use adapter::StepDownWidgetAction;
use adapter::StepLeftWidgetAction;
use adapter::StepRightWidgetAction;
use adapter::StepUpWidgetAction;
use adapter::WidgetInputContext;
pub use adapter::WidgetInputDisabled;
use adapter::classify_activate_endings;
use adapter::emit_widget_input;
use adapter::install_default_modes;
use adapter::queue_activate_cancel;
use adapter::queue_activate_complete;
use adapter::reconcile_input_installations;
use adapter::record_action_start;
use adapter::record_activate_start;
use adapter::record_step_fire;
use adapter::synchronize_context_activity;
use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::EnhancedInputSystems;
use bevy_enhanced_input::prelude::InputConditionAppExt;
use bevy_enhanced_input::prelude::InputContextAppExt;
pub use bindings::WidgetControlSummary;
pub use bindings::WidgetInputBindings;
pub use bindings::WidgetInputBindingsBuilder;
pub use bindings::WidgetInputBindingsError;
pub use bindings::WidgetInputMode;
use gate::WidgetBindingGate;
use gate::update_widget_binding_gates;
use hana_rubric::HeldShortcutPlugin;
pub(crate) use routing::SemanticWidgetIntent;
pub use routing::WidgetInput;
pub use routing::WidgetStepDirection;
pub(in crate::widgets) use routing::route_semantic_input;

use crate::ime::ImeSystemSet;
use crate::widgets::WidgetSystems;

/// Opt-in enhanced-input adapter for window-scoped widget controls.
pub struct WidgetInputPlugin;

impl Plugin for WidgetInputPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EnhancedInputPlugin>() {
            app.add_plugins(EnhancedInputPlugin);
        }
        if !app.is_plugin_added::<HeldShortcutPlugin>() {
            app.add_plugins(HeldShortcutPlugin);
        }
        app.add_input_context::<WidgetInputContext>()
            .add_input_condition::<WidgetBindingGate>()
            .init_resource::<PendingWidgetInputActions>()
            .init_resource::<HeldWidgetActivations>()
            .init_resource::<PendingActivateStarts>()
            .init_resource::<PendingActivateEndings>()
            .add_message::<WidgetInput>()
            .configure_sets(
                PreUpdate,
                (
                    WidgetInputSystems::Reconcile,
                    WidgetInputSystems::ActivateContexts,
                )
                    .chain()
                    .before(EnhancedInputSystems::Prepare),
            )
            .configure_sets(
                Update,
                WidgetInputSystems::EmitInput
                    .after(ImeSystemSet::PublishInputBlockers)
                    .before(WidgetSystems::SemanticInput),
            )
            .add_systems(
                PreUpdate,
                (install_default_modes, reconcile_input_installations)
                    .chain()
                    .in_set(WidgetInputSystems::Reconcile),
            )
            .add_systems(
                PreUpdate,
                (synchronize_context_activity, update_widget_binding_gates)
                    .after(ImeSystemSet::PublishInputBlockers)
                    .in_set(WidgetInputSystems::ActivateContexts),
            )
            .add_systems(
                PreUpdate,
                // Start/Complete/Cancel observers queue these edges during Trigger.
                classify_activate_endings.after(EnhancedInputSystems::Trigger),
            )
            .add_systems(
                Update,
                emit_widget_input.in_set(WidgetInputSystems::EmitInput),
            )
            .add_observer(record_action_start::<NextWidgetAction>)
            .add_observer(record_action_start::<PreviousWidgetAction>)
            .add_observer(record_action_start::<FirstWidgetAction>)
            .add_observer(record_action_start::<LastWidgetAction>)
            .add_observer(record_activate_start)
            .add_observer(queue_activate_complete)
            .add_observer(queue_activate_cancel)
            .add_observer(record_action_start::<CancelWidgetAction>)
            .add_observer(record_step_fire::<StepLeftWidgetAction>)
            .add_observer(record_step_fire::<StepRightWidgetAction>)
            .add_observer(record_step_fire::<StepUpWidgetAction>)
            .add_observer(record_step_fire::<StepDownWidgetAction>);
    }
}

#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum WidgetInputSystems {
    Reconcile,
    ActivateContexts,
    EmitInput,
}
