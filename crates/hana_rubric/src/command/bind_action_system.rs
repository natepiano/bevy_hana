/// Wires an input action to a command function through an intermediate event.
///
/// Registers two observers:
/// 1. `On<Start<Action>>` triggers the event
/// 2. `On<Event>` runs the command via `run_system_cached`
///
/// The intermediate event decouples keyboard input from command execution,
/// allowing the same command to be invoked by a keybinding, programmatically
/// via `commands.trigger(MyEvent)`, or through the Bevy Remote Protocol's
/// `world.trigger_event`.
///
/// Use with [`action!`](crate::action) and [`event!`](crate::event) to generate the action and
/// event structs.
///
/// # Examples
///
/// ```ignore
/// use hana_rubric::action;
/// use hana_rubric::bind_action_system;
/// use hana_rubric::event;
///
/// action!(PauseToggle);
/// event!(PauseEvent);
///
/// fn setup(app: &mut App) {
///     bind_action_system!(app, PauseToggle, PauseEvent, pause_command);
/// }
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! bind_action_system {
    ($app:expr, $action:ty, $event:ty, $command:path) => {{
        use bevy_enhanced_input::action::events::Start;

        $app.add_observer(|_: On<Start<$action>>, mut commands: Commands| {
            commands.trigger(<$event>::default());
        })
        .add_observer(|_: On<$event>, mut commands: Commands| {
            commands.run_system_cached($command);
        })
    }};
}
