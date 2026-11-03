//! Public held shortcut behavior across physical keyboard frames.

use std::time::Duration;

use bevy::app::FixedPreUpdate;
use bevy::app::PreUpdate;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::Key;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::keyboard::NativeKey;
use bevy::prelude::App;
use bevy::prelude::ButtonInput;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::SpawnRelated;
use bevy::prelude::SpawnWith;
use bevy::prelude::default;
use bevy::prelude::related;
use bevy::time::Fixed;
use bevy::time::Time;
use bevy::time::TimeUpdateStrategy;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::Bindings;
use bevy_enhanced_input::prelude::Complete;
use bevy_enhanced_input::prelude::ContextActivity;
use bevy_enhanced_input::prelude::ContextPriority;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::Fire;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use bevy_enhanced_input::prelude::InputModKeys;
use bevy_enhanced_input::prelude::ModKeys;
use bevy_enhanced_input::prelude::Pulse;
use bevy_enhanced_input::prelude::Start;
use hana_rubric::HeldShortcutPlugin;
use hana_rubric::Keybindings;

#[derive(Component)]
struct HeldContext;

#[derive(Component)]
struct LowerContext;

#[derive(InputAction)]
#[action_output(bool)]
struct TestShift;

#[derive(InputAction)]
#[action_output(bool)]
struct HeldAction;

#[derive(InputAction)]
#[action_output(bool)]
struct LowerAction;

#[derive(Default, Resource)]
struct RecordedEvents {
    starts:       usize,
    fires:        usize,
    completes:    usize,
    lower_starts: usize,
    lower_fires:  usize,
}

fn record_start(_start: On<Start<HeldAction>>, mut events: ResMut<RecordedEvents>) {
    events.starts += 1;
}

fn record_fire(_fire: On<Fire<HeldAction>>, mut events: ResMut<RecordedEvents>) {
    events.fires += 1;
}

fn record_complete(_complete: On<Complete<HeldAction>>, mut events: ResMut<RecordedEvents>) {
    events.completes += 1;
}

fn record_lower_start(_start: On<Start<LowerAction>>, mut events: ResMut<RecordedEvents>) {
    events.lower_starts += 1;
}

fn record_lower_fire(_fire: On<Fire<LowerAction>>, mut events: ResMut<RecordedEvents>) {
    events.lower_fires += 1;
}

fn test_app() -> App { test_app_with_held_context::<PreUpdate>() }

fn test_app_with_held_context<S: ScheduleLabel + Default>() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        InputPlugin,
        EnhancedInputPlugin,
        HeldShortcutPlugin,
    ))
    .add_input_context_to::<S, HeldContext>()
    .add_input_context::<LowerContext>()
    .init_resource::<RecordedEvents>()
    .add_observer(record_start)
    .add_observer(record_fire)
    .add_observer(record_complete)
    .add_observer(record_lower_start)
    .add_observer(record_lower_fire);
    app.finish();
    app
}

#[test]
fn skipped_fixed_evaluation_keeps_hold_until_primary_key_release() {
    let mut app = test_app_with_held_context::<FixedPreUpdate>();
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(Duration::from_millis(10));
    spawn_held_context(&mut app, KeyCode::Space.into());
    app.update();

    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            10,
        )));
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Space);
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().starts, 1);
    assert_eq!(app.world().resource::<RecordedEvents>().fires, 1);

    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(5)));
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().fires, 1);
    assert_eq!(app.world().resource::<RecordedEvents>().completes, 0);

    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.starts, 1);
    assert_eq!(events.fires, 2);
    assert_eq!(events.completes, 0);

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::Space);
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            10,
        )));
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().completes, 1);
}

fn spawn_held_context(app: &mut App, binding: bevy_enhanced_input::prelude::Binding) -> Entity {
    app.world_mut()
        .spawn((
            HeldContext,
            Actions::<HeldContext>::spawn(SpawnWith(move |spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(
                    spawner,
                    ActionSettings {
                        consume_input: true,
                        ..default()
                    },
                );
                keybindings.spawn_held_shortcut::<HeldAction>(spawner, binding);
            })),
        ))
        .id()
}

fn key_input(app: &mut App, window: Entity, key_code: KeyCode, state: ButtonState) {
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state,
        text: None,
        repeat: false,
        window,
    });
}

fn press_key(app: &mut App, window: Entity, key_code: KeyCode) {
    key_input(app, window, key_code, ButtonState::Pressed);
}

fn release_key(app: &mut App, window: Entity, key_code: KeyCode) {
    key_input(app, window, key_code, ButtonState::Released);
}

#[test]
fn press_edge_requires_exact_declared_modifiers() {
    let mut app = test_app();
    spawn_held_context(&mut app, KeyCode::Space.with_mod_keys(ModKeys::CONTROL));
    let window = app.world_mut().spawn_empty().id();
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().starts, 0);
    assert_eq!(app.world().resource::<RecordedEvents>().fires, 0);
    release_key(&mut app, window, KeyCode::Space);
    app.update();

    press_key(&mut app, window, KeyCode::ControlLeft);
    press_key(&mut app, window, KeyCode::ShiftLeft);
    press_key(&mut app, window, KeyCode::Space);
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().starts, 0);
    assert_eq!(app.world().resource::<RecordedEvents>().fires, 0);
    release_key(&mut app, window, KeyCode::Space);
    release_key(&mut app, window, KeyCode::ShiftLeft);
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.starts, 1);
    assert_eq!(events.fires, 1);
    assert_eq!(events.completes, 0);
}

#[test]
fn releasing_modifier_does_not_end_primary_key_hold() {
    let mut app = test_app();
    spawn_held_context(&mut app, KeyCode::Space.with_mod_keys(ModKeys::CONTROL));
    let window = app.world_mut().spawn_empty().id();
    app.update();

    press_key(&mut app, window, KeyCode::ControlLeft);
    press_key(&mut app, window, KeyCode::Space);
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().starts, 1);
    assert_eq!(app.world().resource::<RecordedEvents>().fires, 1);

    release_key(&mut app, window, KeyCode::ControlLeft);
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.starts, 1);
    assert_eq!(events.fires, 2);
    assert_eq!(events.completes, 0);
}

#[test]
fn releasing_primary_key_completes_hold_on_that_frame() {
    let mut app = test_app();
    spawn_held_context(&mut app, KeyCode::Space.into());
    let window = app.world_mut().spawn_empty().id();
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    app.update();
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().fires, 2);
    assert_eq!(app.world().resource::<RecordedEvents>().completes, 0);

    release_key(&mut app, window, KeyCode::Space);
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.starts, 1);
    assert_eq!(events.fires, 2);
    assert_eq!(events.completes, 1);
}

#[test]
fn consuming_hold_blocks_lower_priority_space_for_entire_hold() {
    let mut app = test_app();
    app.world_mut().spawn((
        HeldContext,
        ContextPriority::<HeldContext>::new(1),
        Actions::<HeldContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
            let keybindings = Keybindings::new::<TestShift>(
                spawner,
                ActionSettings {
                    consume_input: true,
                    ..default()
                },
            );
            keybindings.spawn_held_shortcut::<HeldAction>(spawner, KeyCode::Space.into());
        })),
    ));
    app.world_mut().spawn((
        LowerContext,
        Actions::<LowerContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
            spawner.spawn((
                Action::<LowerAction>::new(),
                related!(Bindings[Binding::from(KeyCode::Space)]),
            ));
        })),
    ));
    let window = app.world_mut().spawn_empty().id();
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.starts, 1);
    assert_eq!(events.fires, 1);
    assert_eq!(events.lower_starts, 0);
    assert_eq!(events.lower_fires, 0);

    app.update();
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.fires, 3);
    assert_eq!(events.lower_starts, 0);
    assert_eq!(events.lower_fires, 0);
}

#[test]
fn pulsing_hold_repeats_and_consumes_space_until_release() {
    let mut app = test_app();
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            60,
        )));
    app.world_mut().spawn((
        HeldContext,
        ContextPriority::<HeldContext>::new(1),
        Actions::<HeldContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
            let keybindings = Keybindings::new::<TestShift>(
                spawner,
                ActionSettings {
                    consume_input: true,
                    ..default()
                },
            );
            let shortcut =
                keybindings.spawn_held_shortcut::<HeldAction>(spawner, KeyCode::Space.into());
            spawner
                .world_mut()
                .entity_mut(shortcut.hold)
                .insert(Pulse::new(0.15).with_initial_delay(0.4));
        })),
    ));
    app.world_mut().spawn((
        LowerContext,
        Actions::<LowerContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
            spawner.spawn((
                Action::<LowerAction>::new(),
                related!(Bindings[Binding::from(KeyCode::Space)]),
            ));
        })),
    ));
    let window = app.world_mut().spawn_empty().id();
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!((events.starts, events.fires, events.completes), (1, 1, 0));
    assert_eq!((events.lower_starts, events.lower_fires), (0, 0));

    for _ in 0..5 {
        app.update();
        let events = app.world().resource::<RecordedEvents>();
        assert_eq!((events.starts, events.fires, events.completes), (1, 1, 0));
        assert_eq!((events.lower_starts, events.lower_fires), (0, 0));
    }
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!((events.starts, events.fires, events.completes), (1, 2, 0));
    assert_eq!((events.lower_starts, events.lower_fires), (0, 0));

    for expected_fires in [3, 4] {
        for _ in 0..2 {
            app.update();
            let events = app.world().resource::<RecordedEvents>();
            assert_eq!(events.fires, expected_fires - 1);
            assert_eq!(events.completes, 0);
            assert_eq!((events.lower_starts, events.lower_fires), (0, 0));
        }
        app.update();
        let events = app.world().resource::<RecordedEvents>();
        assert_eq!(events.fires, expected_fires);
        assert_eq!(events.completes, 0);
        assert_eq!((events.lower_starts, events.lower_fires), (0, 0));
    }

    release_key(&mut app, window, KeyCode::Space);
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!((events.starts, events.fires, events.completes), (1, 4, 1));
    assert_eq!((events.lower_starts, events.lower_fires), (0, 0));
}

#[test]
fn key_held_before_context_activation_does_not_start_hold() {
    let mut app = test_app();
    let window = app.world_mut().spawn_empty().id();
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    app.update();
    spawn_held_context(&mut app, KeyCode::Space.into());
    app.update();
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.starts, 0);
    assert_eq!(events.fires, 0);
    assert_eq!(events.completes, 0);

    release_key(&mut app, window, KeyCode::Space);
    app.update();
    press_key(&mut app, window, KeyCode::Space);
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.starts, 1);
    assert_eq!(events.fires, 1);
}

#[test]
fn inactive_context_gap_ends_hold_without_restarting_on_reactivation() {
    let mut app = test_app();
    let context = spawn_held_context(&mut app, KeyCode::Space.into());
    let window = app.world_mut().spawn_empty().id();
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().starts, 1);

    app.world_mut()
        .entity_mut(context)
        .insert(ContextActivity::<HeldContext>::INACTIVE);
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().completes, 1);

    release_key(&mut app, window, KeyCode::Space);
    app.update();
    press_key(&mut app, window, KeyCode::Space);
    app.update();
    app.world_mut()
        .entity_mut(context)
        .insert(ContextActivity::<HeldContext>::ACTIVE);
    app.update();
    let events = app.world().resource::<RecordedEvents>();
    assert_eq!(events.starts, 1, "re-activation must not start a new hold");
    assert_eq!(events.completes, 1);

    release_key(&mut app, window, KeyCode::Space);
    app.update();
    press_key(&mut app, window, KeyCode::Space);
    app.update();
    assert_eq!(app.world().resource::<RecordedEvents>().starts, 2);
}
