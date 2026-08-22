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
use bevy::window::MonitorSelection;
use bevy::window::VideoModeSelection;
use bevy::window::WindowMode;
use bevy::window::WindowPosition;
use bevy::window::WindowResolution;
use hana_clerestory::ManagedWindow;
use hana_clerestory::ManagedWindowReapplyOnRequest;
use hana_rigging::prelude::BindingEntities;
use hana_rigging::prelude::BindingEntityLookup;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::ReapplyConfiguration;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::RetireRole;
use hana_rigging::prelude::RoleAvailable;
use hana_rigging::prelude::RoleAwaiting;
use hana_rigging::prelude::RoleKey;
use serde::Deserialize;
use serde::Serialize;

use super::constants::APPLICATION_WINDOW_KEY;
use super::constants::APPLICATION_WINDOW_ROLE;
use super::constants::APPLICATION_WINDOW_TITLE;
use super::constants::AUTOMATIC_WINDOW_KEY;
use super::constants::AUTOMATIC_WINDOW_ROLE;
use super::constants::FIELD_RECOVERY_CYCLE;
use super::constants::FIELD_WINDOW;
use super::constants::FIELD_WINDOW_KEY;
use super::constants::KIND_RECOVERY_CANCELLATION_REQUESTED;
use super::constants::KIND_RECOVERY_RESTORE_REQUESTED;
use super::constants::PRIMARY_WINDOW_ROLE;
use super::constants::PRODUCER_APPLICATION_RECOVERY_CANCELLATION_REQUESTED;
use super::constants::PRODUCER_AUTOMATIC_RECOVERY_CANCELLATION_REQUESTED;
use super::constants::PRODUCER_RECOVERY_RESTORE_REQUESTED;
use super::constants::SECOND_RECOVERY_CYCLE;
use super::setup;
use super::setup::ProbeWindowRegistrationComplete;
use super::setup::ProbeWindowRole;
use super::trace::ProbeTrace;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ProbeWindowSelector {
    Primary,
    Automatic,
    Application,
    Control,
}

impl ProbeWindowSelector {
    const fn matches(self, role: ProbeWindowRole) -> bool {
        matches!(
            (self, role),
            (Self::Primary, ProbeWindowRole::Primary)
                | (Self::Automatic, ProbeWindowRole::Automatic)
                | (Self::Application, ProbeWindowRole::Application)
                | (Self::Control, ProbeWindowRole::Control)
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
    Query<'world, 'state, (Entity, &'static mut Window, &'static ProbeWindowRole)>;

type SelectedWindow<'a> = (Entity, Mut<'a, Window>, ProbeWindowRole);

fn select_window<'a>(
    windows: &'a mut WindowQuery,
    selector: ProbeWindowSelector,
) -> Option<SelectedWindow<'a>> {
    windows
        .iter_mut()
        .find(|(_, _, role)| selector.matches(**role))
        .map(|(entity, window, role)| (entity, window, *role))
}

fn mutate_window(
    windows: &mut WindowQuery,
    selector: ProbeWindowSelector,
    apply: impl FnOnce(&mut Window) -> &'static str,
) -> Result<&'static str, &'static str> {
    select_window(windows, selector)
        .map_or(Err("target window is unavailable"), |(_, mut target, _)| {
            Ok(apply(&mut target))
        })
}

fn kernel_role(role: ProbeWindowRole) -> Result<RoleKey, &'static str> {
    let role = match role {
        ProbeWindowRole::Primary => PRIMARY_WINDOW_ROLE,
        ProbeWindowRole::Automatic => AUTOMATIC_WINDOW_ROLE,
        ProbeWindowRole::Application => APPLICATION_WINDOW_ROLE,
        ProbeWindowRole::Control => return Err("control window has no managed kernel role"),
    };
    RoleKey::new(role).map_err(|_| "probe window role is invalid")
}

/// Marks a window for removal in `Update`, while Bevy still owns its platform window.
#[derive(Component)]
pub(super) struct CloseRequested;

/// Removes windows which a probe command closed after the remote observer returned.
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
        ProbeCommand::CancelRecovery => select_window(&mut windows, ProbeWindowSelector::Automatic)
            .map_or(
                Err("managed automatic window is unavailable"),
                |(_, _, role)| {
                    kernel_role(role).map(|role| {
                        commands.trigger(RetireRole { role });
                        trace.record(
                            frame_count.0,
                            PRODUCER_AUTOMATIC_RECOVERY_CANCELLATION_REQUESTED,
                            KIND_RECOVERY_CANCELLATION_REQUESTED,
                            vec![(FIELD_WINDOW_KEY.into(), AUTOMATIC_WINDOW_KEY.into())],
                        );
                        "automatic role retired"
                    })
                },
            ),
        ProbeCommand::ReplaceApplication => {
            let application_exists = windows
                .iter()
                .any(|(_, _, role)| *role == ProbeWindowRole::Application);
            if application_exists {
                Ok("application-controlled window already exists")
            } else {
                commands.spawn((
                    setup::probe_window(APPLICATION_WINDOW_TITLE, WindowPosition::Automatic),
                    ProbeWindowRole::Application,
                    ManagedWindow {
                        name: APPLICATION_WINDOW_KEY.into(),
                    },
                    ManagedWindowReapplyOnRequest,
                    ProbeWindowRegistrationComplete,
                ));
                Ok("application-controlled replacement requested")
            }
        },
        ProbeCommand::Close { window } => select_window(&mut windows, window).map_or(
            Err("target window is unavailable"),
            |(entity, _, role)| {
                if let Ok(role) = kernel_role(role) {
                    commands.trigger(RetireRole { role });
                }
                commands.entity(entity).insert(CloseRequested);
                Ok("window close requested")
            },
        ),
    };
    let (status, detail) = match result {
        Ok(detail) => (CommandStatus::Applied, detail),
        Err(detail) => (CommandStatus::Rejected, detail),
    };
    receipts.0.insert(
        event.command_id.clone(),
        CommandReceipt {
            command_id: event.command_id.clone(),
            status,
            detail: detail.into(),
        },
    );
}

/// Records the first departure debt and retires the application role on its second departure.
pub(super) fn on_application_role_awaiting(
    event: On<RoleAwaiting>,
    mut lifecycle: ResMut<ApplicationRecoveryLifecycle>,
    windows: Query<(Entity, &ProbeWindowRole)>,
    mut commands: Commands,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    if event.role.as_str() != APPLICATION_WINDOW_ROLE {
        return;
    }
    match lifecycle.phase {
        ApplicationRecoveryPhase::AwaitingFirstDeparture => {
            lifecycle.phase = ApplicationRecoveryPhase::FirstReturnOwed;
        },
        ApplicationRecoveryPhase::AwaitingSecondDeparture => {
            for (entity, role) in &windows {
                if *role == ProbeWindowRole::Application {
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
    }
}

/// Reapplies the application role only after its first recorded departure debt.
pub(super) fn on_application_role_available(
    event: On<RoleAvailable>,
    mut lifecycle: ResMut<ApplicationRecoveryLifecycle>,
    bindings: Res<Bindings>,
    binding_entities: Res<BindingEntities>,
    windows: Query<(Entity, &ProbeWindowRole)>,
    mut commands: Commands,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    if event.role.as_str() != APPLICATION_WINDOW_ROLE
        || lifecycle.phase != ApplicationRecoveryPhase::FirstReturnOwed
    {
        return;
    }
    let Ok(binding) = bindings.binding(&event.role) else {
        return;
    };
    if binding.recovery != RecoveryPolicy::ReapplyOnRequest {
        return;
    }
    let BindingEntityLookup::Registered(binding) = binding_entities.entity(&event.role) else {
        return;
    };
    commands.trigger(ReapplyConfiguration { binding });
    let window = windows
        .iter()
        .find_map(|(entity, role)| (*role == ProbeWindowRole::Application).then_some(entity));
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
}
