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
use bevy::prelude::ResMut;
use bevy::prelude::Window;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy::window::OnMonitor;
use bevy::window::PrimaryWindow;
use bevy::window::WindowPosition;
use bevy::window::WindowResolution;
use hana_clerestory::ManagedWindowName;
use hana_clerestory::Monitors;
use hana_clerestory::RecoverOnRequest;
use hana_clerestory::RecoverOnReturn;
use hana_clerestory::managed_window_role;
use hana_clerestory::primary_window_role;
use hana_kana::ToI32;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleKeyError;

use super::ProbeExitBehavior;
use super::ProbeMonitorSelection;
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
use super::constants::RESTORE_ONLY_WINDOW_KEY;
use super::constants::RESTORE_ONLY_WINDOW_TITLE;
use super::control::ProbeCommand;
use super::control::ProbeCommandIntent;
use super::control::ProbeWindowSelector;
use super::control::RequestedWindowMode;
use super::trace::ProbeTrace;

/// Which recovery path one probe window covers: an automatic return, an application-requested
/// return, restore only, or the unmanaged control. Attached to the window entity before
/// clerestory registers a kernel window role for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
pub(super) enum ProbeWindowScenario {
    PrimaryAutomaticReturn,
    ManagedAutomaticReturn,
    ApplicationRequestedReturn,
    RestoreOnly,
    UnmanagedControl,
}

impl ProbeWindowScenario {
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::PrimaryAutomaticReturn => "primary",
            Self::ManagedAutomaticReturn => AUTOMATIC_WINDOW_KEY,
            Self::ApplicationRequestedReturn => APPLICATION_WINDOW_KEY,
            Self::RestoreOnly => RESTORE_ONLY_WINDOW_KEY,
            Self::UnmanagedControl => "control",
        }
    }

    /// Builds the kernel role key this scenario registers, or reports that it registers none.
    pub(super) fn role(self) -> Result<ProbeWindowRole, RoleKeyError> {
        match self {
            Self::PrimaryAutomaticReturn => primary_window_role().map(ProbeWindowRole::KernelRole),
            Self::ManagedAutomaticReturn => {
                managed_window_role(AUTOMATIC_WINDOW_KEY).map(ProbeWindowRole::KernelRole)
            },
            Self::ApplicationRequestedReturn => {
                managed_window_role(APPLICATION_WINDOW_KEY).map(ProbeWindowRole::KernelRole)
            },
            Self::RestoreOnly => {
                managed_window_role(RESTORE_ONLY_WINDOW_KEY).map(ProbeWindowRole::KernelRole)
            },
            Self::UnmanagedControl => Ok(ProbeWindowRole::UnmanagedControl),
        }
    }
}

/// Whether a probe window holds a kernel role, or is the control window that holds none.
pub(super) enum ProbeWindowRole {
    KernelRole(RoleKey),
    UnmanagedControl,
}

/// Marks a probe window that `register_probe_windows_on_selected_monitor` has already handled,
/// once winit put it on the selected monitor. The registration query filters on its absence, so
/// each window is registered once.
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
    mut monitor_selection: ResMut<ProbeMonitorSelection>,
    monitors: Res<Monitors>,
    mut commands: Commands,
) {
    let requested = monitor_selection.requested_monitor_index();
    let requested_descriptor = monitors
        .iter()
        .find(|monitor| monitor.descriptor.index.adapter_value() == requested)
        .map(|monitor| *monitor.descriptor);
    let target_descriptor = if let Some(target_descriptor) = requested_descriptor {
        target_descriptor
    } else {
        let Some(target_descriptor) = monitors
            .iter()
            .min_by_key(|monitor| monitor.descriptor.index.adapter_value())
            .map(|monitor| *monitor.descriptor)
        else {
            error!("[spawn_probe_windows] no live monitor is available");
            return;
        };
        let active_index = target_descriptor.index.adapter_value();
        warn!(
            "requested monitor index {requested} is unavailable; using monitor index {active_index}"
        );
        monitor_selection.use_fallback(active_index);
        target_descriptor
    };
    spawn_probe_windows_on_monitor(
        &mut commands,
        *startup_mode,
        monitor_selection.selected_monitor_index(),
        target_descriptor,
    );
}

fn spawn_probe_windows_on_monitor(
    commands: &mut Commands,
    startup_mode: ProbeStartupMode,
    monitor_index: usize,
    target_descriptor: hana_clerestory::MonitorDescriptor,
) {
    let position = WindowPosition::At(centered_window_position(&target_descriptor));
    commands.spawn((
        probe_window(PRIMARY_WINDOW_TITLE, position),
        ProbeWindowScenario::PrimaryAutomaticReturn,
    ));
    commands.spawn((
        Window {
            mode: startup_mode.automatic_window_mode(monitor_index),
            ..probe_window(AUTOMATIC_WINDOW_TITLE, position)
        },
        ProbeWindowScenario::ManagedAutomaticReturn,
    ));
    commands.spawn((
        probe_window(APPLICATION_WINDOW_TITLE, position),
        ProbeWindowScenario::ApplicationRequestedReturn,
    ));
    commands.spawn((
        probe_window(RESTORE_ONLY_WINDOW_TITLE, position),
        ProbeWindowScenario::RestoreOnly,
    ));
    commands.spawn((
        probe_window(CONTROL_WINDOW_TITLE, position),
        ProbeWindowScenario::UnmanagedControl,
    ));
}

/// Registers a window role only after winit reports the selected monitor for that native window.
pub(super) fn register_probe_windows_on_selected_monitor(
    startup_mode: Res<ProbeStartupMode>,
    monitor_selection: Res<ProbeMonitorSelection>,
    monitors: Res<Monitors>,
    windows: Query<
        (Entity, &ProbeWindowScenario, &OnMonitor),
        Without<ProbeWindowRegistrationComplete>,
    >,
    mut commands: Commands,
) {
    let Some(target) = monitors.iter().find(|monitor| {
        monitor.descriptor.index.adapter_value() == monitor_selection.selected_monitor_index()
    }) else {
        return;
    };
    for (entity, role, on_monitor) in &windows {
        if on_monitor.0 != target.entity {
            continue;
        }
        let mut entity_commands = commands.entity(entity);
        match role {
            ProbeWindowScenario::PrimaryAutomaticReturn => {
                entity_commands.insert((PrimaryWindow, ProbeWindowRegistrationComplete));
            },
            ProbeWindowScenario::ManagedAutomaticReturn => match *startup_mode {
                ProbeStartupMode::Exclusive => {
                    entity_commands.insert(ProbeWindowRegistrationComplete);
                },
                ProbeStartupMode::Windowed | ProbeStartupMode::Borderless => {
                    entity_commands.insert((
                        ManagedWindowName(AUTOMATIC_WINDOW_KEY.into()),
                        RecoverOnReturn,
                        ProbeWindowRegistrationComplete,
                    ));
                },
            },
            ProbeWindowScenario::ApplicationRequestedReturn => {
                entity_commands.insert((
                    ManagedWindowName(APPLICATION_WINDOW_KEY.into()),
                    RecoverOnRequest,
                    ProbeWindowRegistrationComplete,
                ));
            },
            ProbeWindowScenario::RestoreOnly => {
                entity_commands.insert((
                    ManagedWindowName(RESTORE_ONLY_WINDOW_KEY.into()),
                    ProbeWindowRegistrationComplete,
                ));
            },
            ProbeWindowScenario::UnmanagedControl => {
                entity_commands.insert(ProbeWindowRegistrationComplete);
            },
        }
    }
}

/// Turns a `B` or `W` press into a borderless or windowed [`ProbeCommand::SetMode`] intent, and
/// only while the [`ProbeWindowScenario::ManagedAutomaticReturn`] window is focused. Both keys at
/// once send nothing.
pub(super) fn control_automatic_window_mode(
    keyboard: Res<ButtonInput<KeyCode>>,
    windows: Query<(&ProbeWindowScenario, &Window)>,
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
    if windows.iter().any(|(scenario, window)| {
        *scenario == ProbeWindowScenario::ManagedAutomaticReturn && window.focused
    }) {
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
    windows: Query<(&ProbeWindowScenario, &Window)>,
    frame_count: Res<FrameCount>,
    mut commands: Commands,
) {
    let shift_pressed = keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if !shift_pressed || !keyboard.just_pressed(KeyCode::KeyC) {
        return;
    }
    if windows.iter().any(|(scenario, window)| {
        *scenario == ProbeWindowScenario::ManagedAutomaticReturn && window.focused
    }) {
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
    let exit_behavior = exit_frame.map_or(ProbeExitBehavior::Continue, |exit_frame| {
        ProbeExitBehavior::ExitAfter(*exit_frame)
    });
    if matches!(
        exit_behavior,
        ProbeExitBehavior::ExitAfter(exit_frame) if frame_count.0 >= exit_frame.0
    ) {
        app_exit.write(AppExit::Success);
    }
}

/// Records the selected monitor index and the startup mode as the run's frame-0 trace record,
/// before the kernel authors any display binding.
pub(super) fn trace_probe_session(
    startup_mode: Res<ProbeStartupMode>,
    monitor_selection: Res<ProbeMonitorSelection>,
    trace: Res<ProbeTrace>,
) {
    trace.record(
        0,
        super::constants::PRODUCER_STARTUP_SESSION,
        super::constants::KIND_PROBE_SESSION,
        vec![
            (
                super::constants::FIELD_SELECTED_MONITOR_INDEX.into(),
                monitor_selection.selected_monitor_index().to_string(),
            ),
            (
                super::constants::FIELD_STARTUP_MODE.into(),
                startup_mode.selector().into(),
            ),
        ],
    );
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use bevy::prelude::App;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::With;
    use bevy::window::Monitor;
    use bevy::winit::WinitMonitors;
    use hana_clerestory::WindowManagerPlugin;

    use super::*;
    use crate::constants::DEFAULT_EXTERNAL_MONITOR_INDEX;
    use crate::constants::PROBE_WINDOW_COUNT;

    #[test]
    fn five_probe_scenarios_spawn_on_a_single_display() -> Result<(), Box<dyn Error>> {
        let persistence_directory = tempfile::tempdir()?;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::prelude::WindowPlugin {
                primary_window: None,
                ..default()
            },
        ))
        .insert_resource(WinitMonitors::default())
        .insert_resource(ProbeStartupMode::Windowed)
        .insert_resource(ProbeMonitorSelection::Requested(
            DEFAULT_EXTERNAL_MONITOR_INDEX,
        ))
        .add_plugins(WindowManagerPlugin::with_path(
            persistence_directory.path().join("windows.ron"),
        ))
        .add_systems(bevy::prelude::Startup, spawn_probe_windows);
        app.world_mut().spawn(Monitor {
            name:                    None,
            physical_height:         1_080,
            physical_width:          1_920,
            physical_position:       IVec2::ZERO,
            refresh_rate_millihertz: None,
            scale_factor:            1.0,
            video_modes:             Vec::new(),
        });

        app.update();

        let monitors = app.world().resource::<Monitors>();
        assert_eq!(monitors.iter().len(), 1);
        assert_eq!(monitors.first().index.adapter_value(), 0);
        let probe_monitor_selection = *app.world().resource::<ProbeMonitorSelection>();
        assert_eq!(
            probe_monitor_selection.requested_monitor_index(),
            DEFAULT_EXTERNAL_MONITOR_INDEX
        );
        assert_eq!(
            probe_monitor_selection,
            ProbeMonitorSelection::Fallback {
                requested: DEFAULT_EXTERNAL_MONITOR_INDEX,
                active:    0,
            }
        );
        let mut windows = app
            .world_mut()
            .query_filtered::<&ProbeWindowScenario, With<Window>>();
        let scenarios: Vec<_> = windows.iter(app.world()).copied().collect();
        assert_eq!(scenarios.len(), PROBE_WINDOW_COUNT);
        assert!(
            [
                ProbeWindowScenario::PrimaryAutomaticReturn,
                ProbeWindowScenario::ManagedAutomaticReturn,
                ProbeWindowScenario::ApplicationRequestedReturn,
                ProbeWindowScenario::RestoreOnly,
                ProbeWindowScenario::UnmanagedControl,
            ]
            .into_iter()
            .all(|scenario| scenarios.contains(&scenario))
        );
        Ok(())
    }
}
