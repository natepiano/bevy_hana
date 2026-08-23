use bevy::diagnostic::FrameCount;
use bevy::prelude::AppExit;
use bevy::prelude::ButtonInput;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IVec2;
use bevy::prelude::KeyCode;
use bevy::prelude::MessageWriter;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Window;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy::window::OnMonitor;
use bevy::window::PrimaryWindow;
use bevy::window::WindowPosition;
use bevy::window::WindowResolution;
use hana_clerestory::ManagedWindow;
use hana_clerestory::ManagedWindowReapplyOnRequest;
use hana_clerestory::Monitors;
use hana_kana::ToI32;

use super::ProbeMonitorIndex;
use super::ProbeStartupMode;
use super::SmokeExitFrame;
use super::constants::APPLICATION_WINDOW_KEY;
use super::constants::APPLICATION_WINDOW_TITLE;
use super::constants::AUTOMATIC_WINDOW_KEY;
use super::constants::AUTOMATIC_WINDOW_TITLE;
use super::constants::CONTROL_WINDOW_TITLE;
use super::constants::KEYBOARD_COMMAND_ID_PREFIX;
use super::constants::PRIMARY_WINDOW_TITLE;
use super::constants::PROBE_WINDOW_HEIGHT;
use super::constants::PROBE_WINDOW_WIDTH;
use super::control::ProbeCommand;
use super::control::ProbeCommandIntent;
use super::control::ProbeWindowSelector;
use super::control::RequestedWindowMode;
use super::trace::ProbeTrace;

/// Stable probe role attached before Clerestory registers a kernel window role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
pub(super) enum ProbeWindowRole {
    Primary,
    Automatic,
    Application,
    Control,
}

impl ProbeWindowRole {
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Automatic => AUTOMATIC_WINDOW_KEY,
            Self::Application => APPLICATION_WINDOW_KEY,
            Self::Control => "control",
        }
    }
}

/// Marks a probe window whose native monitor association authorized its final registration.
#[derive(Component)]
pub(super) struct ProbeWindowRegistrationComplete;

/// Builds a probe window with the size shared by each independently controlled role.
pub(super) fn probe_window(title: &str, position: WindowPosition) -> Window {
    Window {
        title: title.into(),
        position,
        resolution: WindowResolution::new(PROBE_WINDOW_WIDTH, PROBE_WINDOW_HEIGHT),
        ..default()
    }
}

fn centered_window_position(target: &hana_clerestory::MonitorDescriptor) -> IVec2 {
    let available_width = target.physical_size.x.to_i32() - PROBE_WINDOW_WIDTH.to_i32();
    let available_height = target.physical_size.y.to_i32() - PROBE_WINDOW_HEIGHT.to_i32();
    target.physical_position + IVec2::new(available_width.max(0) / 2, available_height.max(0) / 2)
}

/// Starts every probe window at the controller-selected monitor.
pub(super) fn spawn_probe_windows(
    startup_mode: Res<ProbeStartupMode>,
    monitor_index: Res<ProbeMonitorIndex>,
    monitors: Res<Monitors>,
    mut commands: Commands,
) {
    let Some(target) = monitors
        .iter()
        .find(|monitor| monitor.descriptor.index.adapter_value() == monitor_index.0)
    else {
        error!(
            "[spawn_probe_windows] selected monitor index {} is unavailable",
            monitor_index.0
        );
        return;
    };
    let position = WindowPosition::At(centered_window_position(target.descriptor));
    commands.spawn((
        probe_window(PRIMARY_WINDOW_TITLE, position),
        ProbeWindowRole::Primary,
    ));
    commands.spawn((
        Window {
            mode: startup_mode.automatic_window_mode(monitor_index.0),
            ..probe_window(AUTOMATIC_WINDOW_TITLE, position)
        },
        ProbeWindowRole::Automatic,
    ));
    commands.spawn((
        probe_window(APPLICATION_WINDOW_TITLE, position),
        ProbeWindowRole::Application,
    ));
    commands.spawn((
        probe_window(CONTROL_WINDOW_TITLE, position),
        ProbeWindowRole::Control,
    ));
}

/// Registers a window role only after winit reports the selected monitor for that native window.
pub(super) fn register_probe_windows_on_selected_monitor(
    startup_mode: Res<ProbeStartupMode>,
    monitor_index: Res<ProbeMonitorIndex>,
    monitors: Res<Monitors>,
    windows: Query<
        (Entity, &ProbeWindowRole, &OnMonitor),
        Without<ProbeWindowRegistrationComplete>,
    >,
    mut commands: Commands,
) {
    let Some(target) = monitors
        .iter()
        .find(|monitor| monitor.descriptor.index.adapter_value() == monitor_index.0)
    else {
        return;
    };
    for (entity, role, on_monitor) in &windows {
        if on_monitor.0 != target.entity {
            continue;
        }
        let mut entity_commands = commands.entity(entity);
        match role {
            ProbeWindowRole::Primary => {
                entity_commands.insert((PrimaryWindow, ProbeWindowRegistrationComplete));
            },
            ProbeWindowRole::Automatic => match *startup_mode {
                ProbeStartupMode::Exclusive => {
                    entity_commands.insert(ProbeWindowRegistrationComplete);
                },
                ProbeStartupMode::Windowed | ProbeStartupMode::Borderless => {
                    entity_commands.insert((
                        ManagedWindow {
                            name: AUTOMATIC_WINDOW_KEY.into(),
                        },
                        ProbeWindowRegistrationComplete,
                    ));
                },
            },
            ProbeWindowRole::Application => {
                entity_commands.insert((
                    ManagedWindow {
                        name: APPLICATION_WINDOW_KEY.into(),
                    },
                    ManagedWindowReapplyOnRequest,
                    ProbeWindowRegistrationComplete,
                ));
            },
            ProbeWindowRole::Control => {
                entity_commands.insert(ProbeWindowRegistrationComplete);
            },
        }
    }
}

/// Allows a focused automatic window to demonstrate normal window-mode input.
pub(super) fn control_automatic_window_mode(
    keyboard: Res<ButtonInput<KeyCode>>,
    windows: Query<(&ProbeWindowRole, &Window)>,
    frame_count: Res<FrameCount>,
    mut commands: Commands,
) {
    let requested_window_mode = match (
        keyboard.just_pressed(KeyCode::KeyB),
        keyboard.just_pressed(KeyCode::KeyW),
    ) {
        (true, false) => RequestedWindowMode::Borderless,
        (false, true) => RequestedWindowMode::Windowed,
        (true, true) | (false, false) => return,
    };
    if windows
        .iter()
        .any(|(role, window)| *role == ProbeWindowRole::Automatic && window.focused)
    {
        commands.trigger(ProbeCommandIntent {
            command_id: format!(
                "{KEYBOARD_COMMAND_ID_PREFIX}-mode-{}-{requested_window_mode:?}",
                frame_count.0
            ),
            command:    ProbeCommand::SetMode {
                window: ProbeWindowSelector::Automatic,
                mode:   requested_window_mode,
            },
        });
    }
}

/// Routes Shift+C through the same retirement command used by the authenticated controller.
pub(super) fn retire_automatic_window(
    keyboard: Res<ButtonInput<KeyCode>>,
    windows: Query<(&ProbeWindowRole, &Window)>,
    frame_count: Res<FrameCount>,
    mut commands: Commands,
) {
    let shift_pressed = keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if !shift_pressed || !keyboard.just_pressed(KeyCode::KeyC) {
        return;
    }
    if windows
        .iter()
        .any(|(role, window)| *role == ProbeWindowRole::Automatic && window.focused)
    {
        commands.trigger(ProbeCommandIntent {
            command_id: format!("{KEYBOARD_COMMAND_ID_PREFIX}-cancel-{}", frame_count.0),
            command:    ProbeCommand::CancelRecovery,
        });
    }
}

/// Ends a smoke run at its configured frame.
pub(super) fn exit_after_smoke_frame(
    exit_frame: Option<Res<SmokeExitFrame>>,
    frame_count: Res<FrameCount>,
    mut app_exit: MessageWriter<AppExit>,
) {
    if exit_frame.is_some_and(|exit_frame| frame_count.0 >= exit_frame.0) {
        app_exit.write(AppExit::Success);
    }
}

/// Records the run identity before the kernel begins authoring display bindings.
pub(super) fn trace_probe_session(
    startup_mode: Res<ProbeStartupMode>,
    monitor_index: Res<ProbeMonitorIndex>,
    trace: Res<ProbeTrace>,
) {
    trace.record(
        0,
        super::constants::PRODUCER_STARTUP_SESSION,
        super::constants::KIND_PROBE_SESSION,
        vec![
            (
                super::constants::FIELD_SELECTED_MONITOR_INDEX.into(),
                monitor_index.0.to_string(),
            ),
            (
                super::constants::FIELD_STARTUP_MODE.into(),
                startup_mode.selector().into(),
            ),
        ],
    );
}
