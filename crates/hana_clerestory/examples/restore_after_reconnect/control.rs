use std::collections::BTreeMap;

use bevy::diagnostic::FrameCount;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::IVec2;
use bevy::prelude::Mut;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::window::MonitorSelection;
use bevy::window::VideoModeSelection;
use bevy::window::WindowMode;
use bevy::window::WindowPosition;
use bevy::window::WindowResolution;
use hana_clerestory::ManagedWindowName;
use hana_clerestory::RecoverOnRequest;
use hana_clerestory::managed_window_role;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::LiveRoleChange;
use hana_rigging::prelude::LiveRoleChanged;
use hana_rigging::prelude::ReapplyConfiguration;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::RetireRole;
use hana_rigging::prelude::RoleStatusView;
use hana_rigging::prelude::WaitingStatusView;
use serde::Deserialize;
use serde::Serialize;

use super::constants::APPLICATION_WINDOW_KEY;
use super::constants::APPLICATION_WINDOW_TITLE;
use super::constants::AUTOMATIC_WINDOW_KEY;
use super::constants::FIELD_RECOVERY_CYCLE;
use super::constants::FIELD_WINDOW;
use super::constants::FIELD_WINDOW_KEY;
use super::constants::KIND_RECOVERY_CANCELLATION_REQUESTED;
use super::constants::KIND_RECOVERY_RESTORE_REQUESTED;
use super::constants::PRODUCER_APPLICATION_RECOVERY_CANCELLATION_REQUESTED;
use super::constants::PRODUCER_AUTOMATIC_RECOVERY_CANCELLATION_REQUESTED;
use super::constants::PRODUCER_RECOVERY_RESTORE_REQUESTED;
use super::constants::SECOND_RECOVERY_CYCLE;
use super::setup;
use super::setup::ProbeWindowRegistrationComplete;
use super::setup::ProbeWindowRole;
use super::setup::ProbeWindowScenario;
use super::trace::ProbeTrace;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ProbeWindowSelector {
    Primary,
    Automatic,
    Application,
    RestoreOnly,
    Control,
}

impl ProbeWindowSelector {
    /// The name this selector is written as in a command, for naming it back in a receipt.
    const fn key(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Automatic => "automatic",
            Self::Application => "application",
            Self::RestoreOnly => "restore-only",
            Self::Control => "control",
        }
    }

    const fn matches(self, scenario: ProbeWindowScenario) -> bool {
        matches!(
            (self, scenario),
            (Self::Primary, ProbeWindowScenario::PrimaryAutomaticReturn)
                | (Self::Automatic, ProbeWindowScenario::ManagedAutomaticReturn)
                | (
                    Self::Application,
                    ProbeWindowScenario::ApplicationRequestedReturn
                )
                | (Self::RestoreOnly, ProbeWindowScenario::RestoreOnly)
                | (Self::Control, ProbeWindowScenario::UnmanagedControl)
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum RequestedWindowMode {
    Windowed,
    Borderless,
    Exclusive,
}

impl RequestedWindowMode {
    const fn window_mode(self) -> WindowMode {
        match self {
            Self::Windowed => WindowMode::Windowed,
            Self::Borderless => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            Self::Exclusive => {
                WindowMode::Fullscreen(MonitorSelection::Current, VideoModeSelection::Current)
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum ProbeCommand {
    SetMode {
        window: ProbeWindowSelector,
        mode:   RequestedWindowMode,
    },
    Move {
        window:   ProbeWindowSelector,
        position: [i32; 2],
    },
    Resize {
        window: ProbeWindowSelector,
        size:   [u32; 2],
    },
    CancelRecovery,
    ReplaceApplication,
    Close {
        window: ProbeWindowSelector,
    },
}

#[derive(Clone, Debug, Event)]
pub(super) struct ProbeCommandIntent {
    pub(super) command_id: String,
    pub(super) command:    ProbeCommand,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum CommandStatus {
    Applied,
    Rejected,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct CommandReceipt {
    pub(super) command_id: String,
    pub(super) status:     CommandStatus,
    pub(super) detail:     String,
}

#[derive(Default, Resource)]
pub(super) struct CommandReceipts(pub(super) BTreeMap<String, CommandReceipt>);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ApplicationRecoveryPhase {
    #[default]
    AwaitingFirstDeparture,
    FirstReturnOwed,
    AwaitingSecondDeparture,
    Retired,
}

/// Consumer-owned lifecycle for the one application-controlled probe role.
#[derive(Default, Resource)]
pub(super) struct ApplicationRecoveryLifecycle {
    phase: ApplicationRecoveryPhase,
}

type WindowQuery<'world, 'state> =
    Query<'world, 'state, (Entity, &'static mut Window, &'static ProbeWindowScenario)>;

type SelectedWindow<'a> = (Entity, Mut<'a, Window>, ProbeWindowScenario);
type ProbeCommandResult = Result<&'static str, String>;

enum ProbeWindowSelection<'a> {
    Available(SelectedWindow<'a>),
    Unavailable,
}

fn select_window<'a>(
    windows: &'a mut WindowQuery,
    selector: ProbeWindowSelector,
) -> ProbeWindowSelection<'a> {
    windows
        .iter_mut()
        .find(|(_, _, scenario)| selector.matches(**scenario))
        .map_or(
            ProbeWindowSelection::Unavailable,
            |(entity, window, scenario)| {
                ProbeWindowSelection::Available((entity, window, *scenario))
            },
        )
}

fn mutate_window(
    windows: &mut WindowQuery,
    selector: ProbeWindowSelector,
    apply: impl FnOnce(&mut Window) -> &'static str,
) -> ProbeCommandResult {
    match select_window(windows, selector) {
        ProbeWindowSelection::Available((_, mut target, _)) => Ok(apply(&mut target)),
        ProbeWindowSelection::Unavailable => Err(format!(
            "no {} window is open, so there was nothing to change",
            selector.key()
        )),
    }
}

/// Marks a window for removal in `Update`, while Bevy still owns its platform window.
#[derive(Component)]
pub(super) struct CloseRequested;

/// Despawns every window carrying [`CloseRequested`], which holds the close until after the
/// remote observer has returned its receipt.
pub(super) fn despawn_requested_windows(
    mut commands: Commands,
    requested: Query<Entity, With<CloseRequested>>,
) {
    for entity in requested.iter() {
        commands.entity(entity).despawn();
    }
}

pub(super) fn apply_probe_command(
    event: On<ProbeCommandIntent>,
    mut commands: Commands,
    mut windows: WindowQuery,
    mut receipts: ResMut<CommandReceipts>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    if receipts.0.contains_key(&event.command_id) {
        return;
    }
    let result = match event.command.clone() {
        ProbeCommand::SetMode { window, mode } => mutate_window(&mut windows, window, |target| {
            target.mode = mode.window_mode();
            "window mode updated"
        }),
        ProbeCommand::Move { window, position } => mutate_window(&mut windows, window, |target| {
            target.position = WindowPosition::At(IVec2::from_array(position));
            "window position updated"
        }),
        ProbeCommand::Resize { window, size } => mutate_window(&mut windows, window, |target| {
            target.resolution = WindowResolution::new(size[0], size[1]);
            "window size updated"
        }),
        ProbeCommand::CancelRecovery => {
            match select_window(&mut windows, ProbeWindowSelector::Automatic) {
                ProbeWindowSelection::Available((_, _, scenario)) => scenario
                    .role()
                    .map_err(|error| {
                        format!(
                            "`{AUTOMATIC_WINDOW_KEY}` did not make a usable role key, so the \
                             automatic window's role could not be retired: {error}"
                        )
                    })
                    .and_then(|role| match role {
                        ProbeWindowRole::KernelRole(role) => {
                            commands.trigger(RetireRole { role });
                            trace.record(
                                frame_count.0,
                                PRODUCER_AUTOMATIC_RECOVERY_CANCELLATION_REQUESTED,
                                KIND_RECOVERY_CANCELLATION_REQUESTED,
                                vec![(FIELD_WINDOW_KEY.into(), AUTOMATIC_WINDOW_KEY.into())],
                            );
                            Ok("automatic role retired")
                        },
                        ProbeWindowRole::UnmanagedControl => Err(String::from(
                            "the automatic selector matched a window holding no kernel role. \
                             Only the control window holds none, and the automatic selector \
                             does not match it, so either the selector or the scenario it \
                             maps to has changed",
                        )),
                    }),
                ProbeWindowSelection::Unavailable => Err(String::from(
                    "no managed automatic window is open, so there is no recovery to cancel",
                )),
            }
        },
        ProbeCommand::ReplaceApplication => {
            let application_exists = windows.iter().any(|(_, _, scenario)| {
                *scenario == ProbeWindowScenario::ApplicationRequestedReturn
            });
            if application_exists {
                Ok("application-controlled window already exists")
            } else {
                commands.spawn((
                    setup::probe_window(APPLICATION_WINDOW_TITLE, WindowPosition::Automatic),
                    ProbeWindowScenario::ApplicationRequestedReturn,
                    ManagedWindowName(APPLICATION_WINDOW_KEY.into()),
                    RecoverOnRequest,
                    ProbeWindowRegistrationComplete,
                ));
                Ok("application-controlled replacement requested")
            }
        },
        ProbeCommand::Close { window } => match select_window(&mut windows, window) {
            ProbeWindowSelection::Available((entity, _, scenario)) => match scenario.role() {
                Ok(window_role) => {
                    if let ProbeWindowRole::KernelRole(role) = window_role {
                        commands.trigger(RetireRole { role });
                    }
                    commands.entity(entity).insert(CloseRequested);
                    Ok("window close requested")
                },
                Err(error) => Err(format!(
                    "`{}` did not make a usable role key, so the {} window still holds its \
                     role and was not closed: {error}",
                    scenario.key(),
                    window.key()
                )),
            },
            ProbeWindowSelection::Unavailable => Err(format!(
                "no {} window is open, so there was nothing to close",
                window.key()
            )),
        },
    };
    let (status, detail) = match result {
        Ok(detail) => (CommandStatus::Applied, String::from(detail)),
        Err(detail) => (CommandStatus::Rejected, detail),
    };
    receipts.0.insert(
        event.command_id.clone(),
        CommandReceipt {
            command_id: event.command_id.clone(),
            status,
            detail,
        },
    );
}

/// Advances application-controlled recovery on reporter-wait entry and exit status edges.
pub(super) fn on_application_role_status_changed(
    event: On<LiveRoleChanged>,
    mut lifecycle: ResMut<ApplicationRecoveryLifecycle>,
    bindings: Res<Bindings>,
    windows: Query<(Entity, &ProbeWindowScenario)>,
    mut commands: Commands,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let LiveRoleChange::Status { from, to } = &event.change else {
        return;
    };
    let was_waiting = matches!(
        from.view(),
        RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
    );
    let is_waiting = matches!(
        to.view(),
        RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
    );
    let application_role = match managed_window_role(APPLICATION_WINDOW_KEY) {
        Ok(application_role) => application_role,
        Err(error) => {
            error!(
                "[on_application_role_status_changed] application role invariant failed: {error}"
            );
            return;
        },
    };
    if event.role != application_role {
        return;
    }
    match (was_waiting, is_waiting) {
        (false, true) => match lifecycle.phase {
            ApplicationRecoveryPhase::AwaitingFirstDeparture => {
                lifecycle.phase = ApplicationRecoveryPhase::FirstReturnOwed;
            },
            ApplicationRecoveryPhase::AwaitingSecondDeparture => {
                for (entity, scenario) in &windows {
                    if *scenario == ProbeWindowScenario::ApplicationRequestedReturn {
                        commands.entity(entity).insert(CloseRequested);
                    }
                }
                commands.trigger(RetireRole {
                    role: event.role.clone(),
                });
                trace.record(
                    frame_count.0,
                    PRODUCER_APPLICATION_RECOVERY_CANCELLATION_REQUESTED,
                    KIND_RECOVERY_CANCELLATION_REQUESTED,
                    vec![
                        (FIELD_WINDOW_KEY.into(), APPLICATION_WINDOW_KEY.into()),
                        (FIELD_RECOVERY_CYCLE.into(), SECOND_RECOVERY_CYCLE.into()),
                    ],
                );
                lifecycle.phase = ApplicationRecoveryPhase::Retired;
            },
            ApplicationRecoveryPhase::FirstReturnOwed | ApplicationRecoveryPhase::Retired => {},
        },
        (true, false) => {
            if lifecycle.phase != ApplicationRecoveryPhase::FirstReturnOwed {
                return;
            }
            let Ok(binding) = bindings.binding(&event.role) else {
                return;
            };
            if binding.recovery != RecoveryPolicy::ReapplyOnRequest {
                return;
            }
            let Ok(binding) = bindings.role_entity(&event.role) else {
                return;
            };
            commands.trigger(ReapplyConfiguration { binding });
            let window = windows.iter().find_map(|(entity, scenario)| {
                (*scenario == ProbeWindowScenario::ApplicationRequestedReturn).then_some(entity)
            });
            let mut fields = vec![(FIELD_WINDOW_KEY.into(), APPLICATION_WINDOW_KEY.into())];
            if let Some(window) = window {
                fields.push((FIELD_WINDOW.into(), format!("{window:?}")));
            }
            trace.record(
                frame_count.0,
                PRODUCER_RECOVERY_RESTORE_REQUESTED,
                KIND_RECOVERY_RESTORE_REQUESTED,
                fields,
            );
            lifecycle.phase = ApplicationRecoveryPhase::AwaitingSecondDeparture;
        },
        (false, false) | (true, true) => {},
    }
}
