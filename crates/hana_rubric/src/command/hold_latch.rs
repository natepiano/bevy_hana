use bevy::prelude::App;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Plugin;
use bevy_enhanced_input::prelude::ActionValue;
use bevy_enhanced_input::prelude::ActionsQuery;
use bevy_enhanced_input::prelude::ConditionKind;
use bevy_enhanced_input::prelude::ContextTime;
use bevy_enhanced_input::prelude::InputCondition;
use bevy_enhanced_input::prelude::InputConditionAppExt;
use bevy_enhanced_input::prelude::TriggerState;

/// Opens on an exact-modifier press and continues while the held action is
/// [`TriggerState::Ongoing`] or [`TriggerState::Fired`] and input is actuated.
#[derive(Component, Debug)]
pub struct HoldLatch {
    /// The one-shot action that opens the hold.
    pub press: Entity,
    hold:      Entity,
}

impl HoldLatch {
    /// Creates a latch for a shortcut's press and held actions.
    #[must_use]
    pub const fn new(press: Entity, hold: Entity) -> Self { Self { press, hold } }
}

impl InputCondition for HoldLatch {
    fn evaluate(
        &mut self,
        actions: &ActionsQuery,
        _: &ContextTime,
        value: ActionValue,
    ) -> TriggerState {
        if !value.as_bool() {
            return TriggerState::None;
        }

        let press_fired = matches!(actions.get(self.press), Ok((_, &TriggerState::Fired, ..)));
        let hold_active = matches!(
            actions.get(self.hold),
            Ok((_, &TriggerState::Ongoing | &TriggerState::Fired, ..))
        );
        if press_fired || hold_active {
            TriggerState::Fired
        } else {
            TriggerState::None
        }
    }

    fn kind(&self) -> ConditionKind { ConditionKind::Blocker }
}

/// Registers [`HoldLatch`] for held shortcuts.
pub struct HeldShortcutPlugin;

impl Plugin for HeldShortcutPlugin {
    fn build(&self, app: &mut App) { app.add_input_condition::<HoldLatch>(); }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::SystemState;
    use bevy::prelude::Time;
    use bevy::prelude::World;
    use bevy::time::Real;
    use bevy::time::Virtual;
    use bevy_enhanced_input::prelude::Action;
    use bevy_enhanced_input::prelude::InputAction;

    use super::*;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct TestPress;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct TestHold;

    #[test]
    fn opens_on_press_and_remains_open_until_release() {
        let mut world = World::new();
        world.init_resource::<Time>();
        world.init_resource::<Time<Real>>();
        world.init_resource::<Time<Virtual>>();
        let press = world
            .spawn((Action::<TestPress>::new(), TriggerState::None))
            .id();
        let hold = world
            .spawn((Action::<TestHold>::new(), TriggerState::None))
            .id();
        let mut query = SystemState::<(ContextTime, ActionsQuery)>::new(&mut world);
        let mut latch = HoldLatch::new(press, hold);

        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::None
        );
        world.entity_mut(press).insert(TriggerState::Fired);
        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::Fired
        );
        world.entity_mut(press).insert(TriggerState::None);
        world.entity_mut(hold).insert(TriggerState::Ongoing);
        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::Fired
        );
        world.entity_mut(hold).insert(TriggerState::Fired);
        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::Fired
        );
        assert_eq!(
            evaluate(&mut latch, &world, &mut query, false),
            TriggerState::None
        );
        world.entity_mut(hold).insert(TriggerState::None);
        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::None
        );
    }

    #[test]
    fn release_does_not_open_latch_even_if_press_fired() {
        let mut world = World::new();
        world.init_resource::<Time>();
        world.init_resource::<Time<Real>>();
        world.init_resource::<Time<Virtual>>();
        let press = world
            .spawn((Action::<TestPress>::new(), TriggerState::Fired))
            .id();
        let hold = world
            .spawn((Action::<TestHold>::new(), TriggerState::None))
            .id();
        let mut query = SystemState::<(ContextTime, ActionsQuery)>::new(&mut world);
        let mut latch = HoldLatch::new(press, hold);

        assert_eq!(
            evaluate(&mut latch, &world, &mut query, false),
            TriggerState::None
        );
    }

    #[test]
    fn stored_hold_state_closes_run_until_another_press_edge() {
        let mut world = World::new();
        world.init_resource::<Time>();
        world.init_resource::<Time<Real>>();
        world.init_resource::<Time<Virtual>>();
        let press = world
            .spawn((Action::<TestPress>::new(), TriggerState::Fired))
            .id();
        let hold = world
            .spawn((Action::<TestHold>::new(), TriggerState::None))
            .id();
        let mut query = SystemState::<(ContextTime, ActionsQuery)>::new(&mut world);
        let mut latch = HoldLatch::new(press, hold);

        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::Fired
        );
        world.entity_mut(press).insert(TriggerState::None);
        world.entity_mut(hold).insert(TriggerState::Ongoing);
        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::Fired
        );

        // Inactive contexts reset the stored hold action state without evaluating the latch.
        world.entity_mut(hold).insert(TriggerState::None);
        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::None
        );
        world.entity_mut(press).insert(TriggerState::Fired);
        assert_eq!(
            evaluate(&mut latch, &world, &mut query, true),
            TriggerState::Fired
        );
    }

    fn evaluate(
        latch: &mut HoldLatch,
        world: &World,
        query: &mut SystemState<(ContextTime, ActionsQuery)>,
        actuated: bool,
    ) -> TriggerState {
        let result = query.get(world);
        assert!(result.is_ok());
        let Ok((time, actions)) = result else {
            return TriggerState::None;
        };
        latch.evaluate(&actions, &time, actuated.into())
    }
}
