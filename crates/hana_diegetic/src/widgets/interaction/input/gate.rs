use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::With;
use bevy_enhanced_input::prelude::ActionValue;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::ActionsQuery;
use bevy_enhanced_input::prelude::ConditionKind;
use bevy_enhanced_input::prelude::ContextTime;
use bevy_enhanced_input::prelude::InputCondition;
use bevy_enhanced_input::prelude::TriggerState;

use super::adapter::WidgetInputContext;
use super::bindings::WidgetInputAction;
use super::routing;
use super::routing::WidgetActionTargets;
use crate::ime::ImeInputBlocker;
use crate::panel::ComputedDiegeticPanel;
use crate::widgets::PanelWidget;
use crate::widgets::PanelWidgets;
use crate::widgets::WidgetFocusAuthority;
use crate::widgets::WidgetFocusTarget;
use crate::widgets::WidgetFocusable;
use crate::widgets::WidgetOf;

/// Blocks a widget action unless it has a target or its window has an IME lease.
#[derive(Component, Debug)]
pub(super) struct WidgetBindingGate {
    pub(super) open: bool,
}

/// The focus target for which one step action's gate last opened.
#[derive(Component, Debug)]
pub(super) enum StepBindingFocus {
    AwaitingFirstOpen,
    OpenedFor(Entity),
}

impl StepBindingFocus {
    fn allows(&mut self, focus: WidgetFocusTarget, eligible: bool) -> bool {
        match *self {
            Self::OpenedFor(previous) if focus != WidgetFocusTarget::Widget(previous) => {
                *self = Self::AwaitingFirstOpen;
                false
            },
            Self::OpenedFor(_) => eligible,
            Self::AwaitingFirstOpen => {
                if let (true, WidgetFocusTarget::Widget(widget)) = (eligible, focus) {
                    *self = Self::OpenedFor(widget);
                    true
                } else {
                    false
                }
            },
        }
    }
}

impl InputCondition for WidgetBindingGate {
    fn evaluate(
        &mut self,
        _actions: &ActionsQuery,
        _time: &ContextTime,
        _value: ActionValue,
    ) -> TriggerState {
        if self.open {
            TriggerState::Fired
        } else {
            TriggerState::None
        }
    }

    fn kind(&self) -> ConditionKind { ConditionKind::Blocker }
}

pub(super) fn update_widget_binding_gates(
    contexts: Query<(Entity, &Actions<WidgetInputContext>)>,
    mut actions: Query<(
        &WidgetInputAction,
        &mut WidgetBindingGate,
        Option<&mut StepBindingFocus>,
    )>,
    authority: Option<Res<WidgetFocusAuthority>>,
    blocker: Option<Res<ImeInputBlocker>>,
    panels: Query<(&ComputedDiegeticPanel, &PanelWidgets)>,
    focusable_widgets: Query<(&PanelWidget, &WidgetOf), With<WidgetFocusable>>,
    action_targets: WidgetActionTargets,
) {
    for (window, context_actions) in contexts.iter() {
        for action_entity in context_actions.iter() {
            let Ok((action, mut gate, step_focus)) = actions.get_mut(*action_entity) else {
                continue;
            };
            let ime_blocked = blocker
                .as_deref()
                .is_some_and(|blocker| blocker.blocks_window(window));
            let eligible = authority.as_deref().is_some_and(|authority| {
                routing::can_widget_input_act(
                    *action,
                    window,
                    authority,
                    &panels,
                    &focusable_widgets,
                    &action_targets,
                )
            });
            let open = if ime_blocked {
                true
            } else if let (WidgetInputAction::Step(_), Some(mut step_focus), Some(authority)) =
                (*action, step_focus, authority.as_deref())
            {
                step_focus.allows(authority.focused_widget(window), eligible)
            } else {
                eligible
            };
            if gate.open != open {
                gate.open = open;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::SystemState;
    use bevy::prelude::Real;
    use bevy::prelude::Time;
    use bevy::prelude::Virtual;
    use bevy::prelude::World;

    use super::*;

    #[test]
    fn closed_gate_blocks_and_open_gate_allows_an_action() {
        let mut world = World::new();
        world.init_resource::<Time>();
        world.init_resource::<Time<Real>>();
        world.init_resource::<Time<Virtual>>();
        let mut state = SystemState::<(ContextTime, ActionsQuery)>::new(&mut world);
        let parameters = state.get(&world);
        assert!(parameters.is_ok());
        let Ok((time, actions)) = parameters else {
            return;
        };
        let mut gate = WidgetBindingGate { open: false };

        assert!(matches!(gate.kind(), ConditionKind::Blocker));
        assert_eq!(
            gate.evaluate(&actions, &time, true.into()),
            TriggerState::None
        );
        gate.open = true;
        assert_eq!(
            gate.evaluate(&actions, &time, true.into()),
            TriggerState::Fired
        );
    }
}
