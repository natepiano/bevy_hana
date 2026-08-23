//! Keyboard shortcuts routed through `bevy_enhanced_input` via the `hana_kana`
//! macros.
//!
//! Every showcase shortcut lives in one of two input contexts:
//!
//! - [`ShowcaseUiInput`] (always active): `Esc` pause, `G` log toggle, `Backspace` clear log, and
//!   `Up`/`Down` log scroll.
//! - [`ShowcaseGameplayInput`] (deactivated while paused via [`ContextActivity`]): `Y P O R 0 M I C
//!   K L`.
//!
//! Single keys are spawned through [`Keybindings::spawn_key`], which attaches a
//! `BlockBy` on every modifier. A bare key therefore stays quiet while a
//! modifier is held, so `fairy_dust`'s `Shift+C` preset cycle no longer also
//! fires the showcase's `C` conflict-policy shortcut.
//!
//! Discrete shortcuts are wired with [`bind_action_system!`]; the two scroll
//! keys are continuous and run from `On<Fire<…>>` observers that scale by frame
//! time.

use bevy::prelude::App;
use bevy::prelude::Component;
use bevy::prelude::Event;
use bevy::prelude::Has;
use bevy::prelude::KeyCode;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::SpawnRelated;
use bevy::prelude::SpawnWith;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::ContextActivity;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::Fire;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use hana_rubric::Keybindings;
use hana_rubric::action;
use hana_rubric::bind_action_system;
use hana_rubric::event;

use super::*;

// ui context — always active
action!(ClearLog);
action!(ScrollLogDown);
action!(ScrollLogUp);
action!(ToggleLog);
action!(TogglePause);

event!(ClearLogEvent);
event!(ToggleLogEvent);
event!(TogglePauseEvent);

// gameplay context — gated by pause
action!(AnimateCamera);
action!(LookAtAndFitHovered);
action!(LookAtHovered);
action!(RandomizeEasing);
action!(ResetEasing);
action!(SetOrthographic);
action!(SetPerspective);
action!(ToggleConflictPolicy);
action!(ToggleDebugOverlay);
action!(ToggleInterruptBehavior);

event!(AnimateCameraEvent);
event!(LookAtAndFitHoveredEvent);
event!(LookAtHoveredEvent);
event!(RandomizeEasingEvent);
event!(ResetEasingEvent);
event!(SetOrthographicEvent);
event!(SetPerspectiveEvent);
event!(ToggleConflictPolicyEvent);
event!(ToggleDebugOverlayEvent);
event!(ToggleInterruptBehaviorEvent);

// Modifier marker the `Keybindings` builder binds to `Shift`; the showcase
// never reads its state, it only drives the `BlockBy` wiring.
action!(ShowcaseShift);

/// Always-active context: pause, log toggle/clear, and log scrolling stay live
/// even while the simulation is paused.
#[derive(Component)]
struct ShowcaseUiInput;

/// Pause-gated context: camera and animation shortcuts only fire while the
/// simulation is running. [`gate_gameplay_on_pause`] toggles its
/// [`ContextActivity`].
#[derive(Component)]
struct ShowcaseGameplayInput;

pub(crate) struct ShowcaseInputPlugin;

impl Plugin for ShowcaseInputPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EnhancedInputPlugin>() {
            app.add_plugins(EnhancedInputPlugin);
        }
        app.add_input_context::<ShowcaseUiInput>()
            .add_input_context::<ShowcaseGameplayInput>()
            .add_systems(Startup, (spawn_ui_context, spawn_gameplay_context))
            .add_systems(Update, (gate_gameplay_on_pause, gate_camera_input_on_pause))
            .add_observer(scroll_log_up_on_fire)
            .add_observer(scroll_log_down_on_fire);

        bind_action_system!(app, TogglePause, TogglePauseEvent, ui::toggle_pause);
        bind_action_system!(app, ToggleLog, ToggleLogEvent, event_log::toggle_event_log);
        bind_action_system!(app, ClearLog, ClearLogEvent, event_log::clear_event_log);

        bind_action_system!(
            app,
            ToggleDebugOverlay,
            ToggleDebugOverlayEvent,
            animation_controls::toggle_debug_overlay
        );
        bind_action_system!(
            app,
            SetPerspective,
            SetPerspectiveEvent,
            animation_controls::set_perspective
        );
        bind_action_system!(
            app,
            SetOrthographic,
            SetOrthographicEvent,
            animation_controls::set_orthographic
        );
        bind_action_system!(
            app,
            RandomizeEasing,
            RandomizeEasingEvent,
            animation_controls::randomize_easing
        );
        bind_action_system!(
            app,
            ResetEasing,
            ResetEasingEvent,
            animation_controls::reset_easing
        );
        bind_action_system!(
            app,
            AnimateCamera,
            AnimateCameraEvent,
            animation_controls::animate_camera
        );
        bind_action_system!(
            app,
            ToggleInterruptBehavior,
            ToggleInterruptBehaviorEvent,
            animation_controls::toggle_interrupt_behavior
        );
        bind_action_system!(
            app,
            ToggleConflictPolicy,
            ToggleConflictPolicyEvent,
            animation_controls::toggle_animation_conflict_policy
        );
        bind_action_system!(
            app,
            LookAtHovered,
            LookAtHoveredEvent,
            pointer::look_at_hovered
        );
        bind_action_system!(
            app,
            LookAtAndFitHovered,
            LookAtAndFitHoveredEvent,
            pointer::look_at_and_zoom_to_fit_hovered
        );
    }
}

fn spawn_ui_context(mut commands: Commands) {
    commands.spawn((
        ShowcaseUiInput,
        Actions::<ShowcaseUiInput>::spawn(SpawnWith(spawn_ui_actions)),
    ));
}

fn spawn_gameplay_context(mut commands: Commands) {
    commands.spawn((
        ShowcaseGameplayInput,
        Actions::<ShowcaseGameplayInput>::spawn(SpawnWith(spawn_gameplay_actions)),
    ));
}

fn spawn_ui_actions(spawner: &mut ActionSpawner<ShowcaseUiInput>) {
    let keybindings = Keybindings::new::<ShowcaseShift>(spawner, ActionSettings::default());
    keybindings.spawn_key::<TogglePause>(spawner, KeyCode::Escape);
    keybindings.spawn_key::<ToggleLog>(spawner, KeyCode::KeyG);
    keybindings.spawn_key::<ClearLog>(spawner, KeyCode::Backspace);
    keybindings.spawn_key::<ScrollLogUp>(spawner, KeyCode::ArrowUp);
    keybindings.spawn_key::<ScrollLogDown>(spawner, KeyCode::ArrowDown);
}

fn spawn_gameplay_actions(spawner: &mut ActionSpawner<ShowcaseGameplayInput>) {
    let keybindings = Keybindings::new::<ShowcaseShift>(spawner, ActionSettings::default());
    keybindings.spawn_key::<ToggleDebugOverlay>(spawner, KeyCode::KeyY);
    keybindings.spawn_key::<SetPerspective>(spawner, KeyCode::KeyP);
    keybindings.spawn_key::<SetOrthographic>(spawner, KeyCode::KeyO);
    keybindings.spawn_key::<RandomizeEasing>(spawner, KeyCode::KeyR);
    keybindings.spawn_key::<ResetEasing>(spawner, KeyCode::Digit0);
    keybindings.spawn_key::<AnimateCamera>(spawner, KeyCode::KeyM);
    keybindings.spawn_key::<ToggleInterruptBehavior>(spawner, KeyCode::KeyI);
    keybindings.spawn_key::<ToggleConflictPolicy>(spawner, KeyCode::KeyC);
    keybindings.spawn_key::<LookAtHovered>(spawner, KeyCode::KeyK);
    keybindings.spawn_key::<LookAtAndFitHovered>(spawner, KeyCode::KeyL);
}

/// Mirrors the pause state onto the gameplay context. `ContextActivity` is an
/// immutable component, so a state flip is applied by re-inserting it.
fn gate_gameplay_on_pause(
    time: Res<Time<Virtual>>,
    mut commands: Commands,
    contexts: Query<(Entity, &ContextActivity<ShowcaseGameplayInput>)>,
) {
    let Ok((entity, activity)) = contexts.single() else {
        return;
    };
    let should_be_active = !time.is_paused();
    if **activity != should_be_active {
        commands
            .entity(entity)
            .insert(ContextActivity::<ShowcaseGameplayInput>::new(
                should_be_active,
            ));
    }
}

/// Mirrors the pause state onto the camera's input. While paused the camera
/// carries [`CameraInputDisabled`] so orbit/pan/zoom input is dropped instead
/// of writing into the target and replaying on unpause.
fn gate_camera_input_on_pause(
    time: Res<Time<Virtual>>,
    mut commands: Commands,
    cameras: Query<(Entity, Has<CameraInputDisabled>), With<OrbitCam>>,
) {
    let paused = time.is_paused();
    for (entity, disabled) in &cameras {
        match (paused, disabled) {
            (true, false) => {
                commands.entity(entity).insert(CameraInputDisabled);
            },
            (false, true) => {
                commands.entity(entity).remove::<CameraInputDisabled>();
            },
            (true, true) | (false, false) => {},
        }
    }
}

fn scroll_log_up_on_fire(
    _fire: On<Fire<ScrollLogUp>>,
    time: Res<Time>,
    mut log: ResMut<event_log::EventLog>,
) {
    event_log::scroll_log(&mut log, EVENT_LOG_SCROLL_SPEED * time.delta_secs());
}

fn scroll_log_down_on_fire(
    _fire: On<Fire<ScrollLogDown>>,
    time: Res<Time>,
    mut log: ResMut<event_log::EventLog>,
) {
    event_log::scroll_log(&mut log, -EVENT_LOG_SCROLL_SPEED * time.delta_secs());
}
