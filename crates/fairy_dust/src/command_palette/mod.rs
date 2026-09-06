//! Capability: a searchable box listing every declared keymap command, with the
//! keymap failures that broke the bindings rendered above its query field.
//!
//! [`SprinkleBuilder::with_command_palette`](crate::SprinkleBuilder::with_command_palette)
//! adds the box alone. The keymap runtime it lists is installed by every Fairy
//! Dust app whether or not the box is asked for, because Fairy Dust's own
//! capabilities are reached through it — see [`crate::keymap`]. The query field
//! is an editable panel field driven by `hana_diegetic`'s IME session, so the
//! platform candidate window is positioned at the caret and Fairy Dust's own
//! keyboard shortcuts do not run while the session holds the window's input
//! lease.

mod constants;
mod failure_row;
mod panel;

use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::process::Command;

use bevy::ecs::change_detection::DetectChanges;
use bevy::input::ButtonInput;
use bevy::input::InputSystems;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::Added;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mut;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use hana_diegetic::Anchor;
use hana_diegetic::ButtonClicked;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::ImeAcceptCommit;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeAppliedResult;
use hana_diegetic::ImeCanceled;
use hana_diegetic::ImeCommitAuthority;
use hana_diegetic::ImeCommitCause;
use hana_diegetic::ImeCommitRequested;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeInputBlocker;
use hana_diegetic::ImeOpenSession;
use hana_diegetic::ImeRejectCommit;
use hana_diegetic::ImeRejection;
use hana_diegetic::ImeReplacePanelTree;
use hana_diegetic::ImeTarget;
use hana_diegetic::ImeTextChanged;
use hana_diegetic::LayoutTree;
use hana_diegetic::PanelPicking;
use hana_diegetic::PanelSystems;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_rubric::ActiveKeymapContext;
use hana_rubric::CommandId;
use hana_rubric::CommandInvocationOutcome;
use hana_rubric::CommandRegistry;
use hana_rubric::ContextDimensionName;
use hana_rubric::EffectiveKeymapStatus;
use hana_rubric::KeyboardClaim;
use hana_rubric::KeyboardOwner;
use hana_rubric::KeyboardRelease;
use hana_rubric::KeymapBindingUnavailability;
use hana_rubric::KeymapBindings;
use hana_rubric::KeymapLoadFailures;
use hana_rubric::KeymapPathAvailability;
use hana_rubric::KeymapSystems;
use hana_rubric::KeystrokeRouting;
use hana_rubric::PaletteBinding;
use hana_rubric::PaletteSelectionOutcome;
use hana_rubric::ReflectKeymapCommand;
use hana_rubric::command;
use hana_rubric::query_command_palette;

use self::constants::FIELD_ID;
use self::failure_row::KeymapFailureAction;
use self::failure_row::keymap_failure_rows;
use self::panel::FailureActionRow;
use self::panel::failure_action_row_index;
use self::panel::palette_panel_origin;
use self::panel::palette_panel_width;
use self::panel::palette_tree_awaiting_assembly;
use self::panel::palette_tree_for_query;
use crate::ensure_plugin;
use crate::keymap;

command! {
    action:      OpenCommandPalette,
    event:       OpenCommandPaletteEvent,
    id:          "palette::open",
    title:       "Open Command Palette",
    description: "Search every declared command and run the selected one.",
    capability:  Unremappable,
}

pub(crate) fn recovery_command_id() -> CommandId {
    CommandId::declared::<OpenCommandPaletteEvent>()
}

/// Installs the searchable box. The keymap runtime it reads is installed by the
/// baseline (see [`crate::keymap`]), so an application that never opens the
/// palette still dispatches every declared command.
struct CommandPalettePlugin;

impl Plugin for CommandPalettePlugin {
    fn build(&self, app: &mut App) {
        ensure_plugin(app, EnhancedInputPlugin);
        app.add_systems(
            PreUpdate,
            (
                toggle_command_palette_from_recovery_keystroke
                    .after(InputSystems)
                    .before(KeymapSystems::Route),
                hand_keyboard_to_query_field.before(KeymapSystems::Route),
            ),
        )
        .add_systems(
            Update,
            (
                refresh_open_palette_after_keymap_changes,
                open_palette_ime_after_panel_layout
                    .after(PanelSystems::ComputeLayout)
                    .after(PanelSystems::PositionScreenSpace),
            ),
        );
        app.add_observer(toggle_command_palette)
            .add_observer(update_query)
            .add_observer(dispatch_selected_command)
            .add_observer(close_on_cancel)
            .add_observer(run_failure_action);
    }
}

/// Text the command palette and examples use to describe one semantic binding result.
///
/// The value keeps `PaletteBinding`'s typed result at the query boundary and owns only the
/// user-facing wording needed by a panel, title bar, or status surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaletteBindingPresentation(String);

impl PaletteBindingPresentation {
    /// Returns the user-facing text for this command's current binding result.
    #[must_use]
    pub fn text(&self) -> &str { &self.0 }
}

impl Display for PaletteBindingPresentation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result { self.0.fmt(formatter) }
}

/// Presents one typed Rubric palette binding with the wording Fairy Dust uses everywhere.
#[must_use]
pub fn palette_binding_presentation(
    palette_binding: PaletteBinding<'_, '_>,
) -> PaletteBindingPresentation {
    let text = match palette_binding {
        PaletteBinding::ApplicationRecovery(keystroke) => format!("protected {keystroke}"),
        PaletteBinding::BoundTo(keystroke_sequence) => keystroke_sequence.to_string(),
        PaletteBinding::Unbound => String::from("unbound"),
        PaletteBinding::KeymapUnavailable(unavailability) => {
            keymap_unavailability_label(unavailability).to_owned()
        },
        PaletteBinding::AwaitingStateDimensions => String::from("state loading"),
        PaletteBinding::StateDimensionsUnavailable(missing) => {
            format!("state unavailable: {}", missing_dimension_names(missing))
        },
        PaletteBinding::UnmaterializableStateDimensions => String::from("invalid state snapshot"),
    };
    PaletteBindingPresentation(text)
}

/// Joins Rubric's sorted missing-dimension names for one concurrent recovery context.
fn missing_dimension_names(missing: &[ContextDimensionName]) -> String {
    missing
        .iter()
        .map(hana_rubric::ContextDimensionName::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The terse user-facing state shown when a committed keymap table is unavailable.
const fn keymap_unavailability_label(unavailability: KeymapBindingUnavailability) -> &'static str {
    match unavailability {
        KeymapBindingUnavailability::AwaitingInitialLoad => "keymap loading",
        KeymapBindingUnavailability::Unconfigured => "keymap unconfigured",
        KeymapBindingUnavailability::MissingDefault => "embedded defaults missing",
        KeymapBindingUnavailability::InvalidDefault => "embedded defaults invalid",
    }
}

/// Marks the spawned palette box so the observers can rebuild or despawn it.
#[derive(Component)]
struct CommandPalettePanel;

/// The one open palette panel whose IME session may update, commit, or close it.
#[derive(Clone, Copy)]
struct OpenPalettePanel(Entity);

/// Whether an IME target is owned by the current [`OpenPalettePanel`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaletteImeTargetOwnership {
    /// The target names the open palette panel and its query field.
    OwnedByOpenPanel,
    /// The target belongs to another panel or an application-owned field.
    NotOwnedByOpenPanel,
}

impl OpenPalettePanel {
    /// Classifies the target against this panel's screen or app-owned query field.
    fn ime_target_ownership(self, target: &ImeTarget) -> PaletteImeTargetOwnership {
        match target {
            ImeTarget::ScreenPanelField { panel, field_id }
                if *panel == self.0 && field_id.to_string() == FIELD_ID =>
            {
                PaletteImeTargetOwnership::OwnedByOpenPanel
            },
            ImeTarget::AppOwned { owner, field_id }
                if *owner == self.0 && field_id.to_string() == FIELD_ID =>
            {
                PaletteImeTargetOwnership::OwnedByOpenPanel
            },
            ImeTarget::WorldPanelField { .. }
            | ImeTarget::ScreenPanelField { .. }
            | ImeTarget::AppOwned { .. } => PaletteImeTargetOwnership::NotOwnedByOpenPanel,
        }
    }
}

/// Keeps the window that owns the palette's IME session with the panel until
/// the panel is despawned.
#[derive(Component)]
struct PaletteImeSessionWindow(Entity);

/// The query text that the palette entity has committed through its IME
/// session. Resource-driven refreshes reuse it without fabricating another IME
/// text event.
#[derive(Component)]
struct PaletteCommittedQuery(String);

/// Whether Rubric has assembled the registry needed to query the palette.
///
/// This is separate from a keymap availability result: once the registry is
/// assembled, a terminal `KeymapBindings` state is still queried and rendered
/// through Rubric's typed `PaletteBinding::KeymapUnavailable`.
enum PaletteRegistryAssembly<'registry> {
    /// Plugin assembly has not yet published command metadata.
    AwaitingAssembly,
    /// The registry can produce a borrowed shared palette result.
    Assembled(&'registry CommandRegistry),
}

/// UI data produced by one consumer-owned palette rebuild.
struct PaletteTreePresentation {
    /// The diegetic tree that represents the current shared query and
    /// application-owned failure rows.
    layout_tree:            LayoutTree,
    /// Click actions in the same order as the displayed failure rows.
    keymap_failure_actions: Vec<KeymapFailureAction>,
}

/// The repair actions the palette's failure rows currently offer, in row order.
///
/// The rows are rebuilt on every keystroke, so the click observer resolves a
/// clicked action against the rows that were rendered rather than recomputing
/// the diagnostics.
#[derive(Component)]
struct PaletteFailureActions(Vec<KeymapFailureAction>);

/// Adds the palette box to `app` once. The keymap it lists is recorded
/// separately by [`crate::keymap::configure`].
pub(crate) fn install(app: &mut App) {
    if app.is_plugin_added::<CommandPalettePlugin>() {
        return;
    }
    app.add_plugins(CommandPalettePlugin);
}

/// Hands the keyboard to the query field for as long as its IME session holds
/// the window's input lease, and hands it back when the session ends.
///
/// The keymap keeps routing throughout, so it never loses a release edge. The
/// palette's recovery chord is read directly outside that routing path, which
/// lets the same chord close an editing palette even when no bindings route.
fn hand_keyboard_to_query_field(
    ime_input_blocker: Option<Res<ImeInputBlocker>>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut keystroke_routing: ResMut<KeystrokeRouting>,
) {
    let query_field_owns_the_keyboard = match (ime_input_blocker, windows.single()) {
        (Some(ime_input_blocker), Ok(window)) => ime_input_blocker.blocks_window(window),
        (None, _) | (_, Err(_)) => false,
    };

    match (query_field_owns_the_keyboard, keystroke_routing.as_ref()) {
        (true, KeystrokeRouting::EveryBinding { .. }) => {
            let keyboard_claim = keystroke_routing.take_for_text_entry(
                KeyboardOwner::of::<CommandPalettePlugin>(),
                [CommandId::declared::<OpenCommandPaletteEvent>()],
            );
            if keyboard_claim == KeyboardClaim::HeldByAnother {
                warn!(
                    "fairy_dust: the command palette query field opened while another party holds \
                     the keyboard; its keystrokes still route as commands"
                );
            }
        },
        (false, KeystrokeRouting::TextEntry { .. }) => {
            let keyboard_release =
                keystroke_routing.release(KeyboardOwner::of::<CommandPalettePlugin>());
            if keyboard_release == KeyboardRelease::HeldByAnother {
                warn!(
                    "fairy_dust: the command palette query field closed while another party holds \
                     the keyboard; that party's text entry is still in force"
                );
            }
        },
        (true, KeystrokeRouting::TextEntry { .. })
        | (false, KeystrokeRouting::EveryBinding { .. }) => {},
    }
}

/// Opens or closes the palette from Fairy Dust's physical recovery chord.
///
/// This bypasses Rubric's matcher, so it cancels any existing partial match
/// before the observer below opens the panel or closes it.
fn toggle_command_palette_from_recovery_keystroke(world: &mut World) {
    let recovery_pressed = world
        .get_resource::<ButtonInput<KeyCode>>()
        .is_some_and(keymap::recovery_keystroke_pressed);
    if recovery_pressed {
        hana_rubric::cancel_pending_sequences(world);
        world.trigger(OpenCommandPaletteEvent);
    }
}

/// Opens the palette, or closes it when the same command fires while it is open.
fn toggle_command_palette(
    _open: On<OpenCommandPaletteEvent>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    panels: Query<Entity, With<CommandPalettePanel>>,
    keymap_load_failures: Res<KeymapLoadFailures>,
    keymap_path_availability: Res<KeymapPathAvailability>,
    keymap_bindings: Res<KeymapBindings>,
    active_context: Res<ActiveKeymapContext>,
    effective_keymap_status: Res<EffectiveKeymapStatus>,
    command_registry: Option<Res<CommandRegistry>>,
    mut commands: Commands,
) {
    if let Ok(panel) = panels.single() {
        commands.entity(panel).despawn();
        return;
    }
    let Ok((window_entity, window)) = windows.single() else {
        return;
    };
    let registry_assembly = palette_registry_assembly(command_registry.as_deref());
    let presentation = build_palette_presentation(
        registry_assembly,
        &keymap_load_failures,
        &keymap_path_availability,
        &keymap_bindings,
        &active_context,
        &effective_keymap_status,
        "",
        palette_panel_width(window),
    );
    let origin = palette_panel_origin(window);
    let panel_width = palette_panel_width(window);
    let panel = DiegeticPanel::screen()
        .size(Sizing::fixed(Px(panel_width)), Sizing::FIT)
        .anchor(Anchor::TopLeft)
        .screen_position(origin.x, origin.y)
        .picking(PanelPicking::INTERACTIVE)
        .with_tree(presentation.layout_tree)
        .build();
    let panel = match panel {
        Ok(panel) => panel,
        Err(error) => {
            error!("fairy_dust: failed to build the command palette: {error}");
            return;
        },
    };

    commands.spawn((
        CommandPalettePanel,
        PaletteImeSessionWindow(window_entity),
        PaletteCommittedQuery(String::new()),
        PaletteFailureActions(presentation.keymap_failure_actions),
        panel,
        Transform::default(),
    ));
}

/// Starts every newly spawned palette session after its field has been laid out.
fn open_palette_ime_after_panel_layout(
    added_panels: Query<(Entity, &PaletteImeSessionWindow), Added<PaletteImeSessionWindow>>,
    mut commands: Commands,
) {
    for (panel, session_window) in &added_panels {
        commands.trigger(ImeOpenSession {
            target:       palette_ime_target(panel),
            window:       session_window.0,
            initial_text: String::new(),
            field_spec:   ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new(FIELD_ID)),
            anchor:       None,
        });
    }
}

/// Selects the palette's inline field in applications and a stable app-owned
/// target in headless tests, which omit screen-panel positioning.
fn palette_ime_target(panel: Entity) -> ImeTarget {
    #[cfg(test)]
    {
        ImeTarget::AppOwned {
            owner:    panel,
            field_id: FIELD_ID.into(),
        }
    }
    #[cfg(not(test))]
    {
        ImeTarget::ScreenPanelField {
            panel,
            field_id: FIELD_ID.into(),
        }
    }
}

/// Re-renders the palette for each keystroke the IME session commits to its buffer.
fn update_query(
    changed: On<ImeTextChanged>,
    windows: Query<&Window, With<PrimaryWindow>>,
    panels: Query<Entity, With<CommandPalettePanel>>,
    keymap_load_failures: Res<KeymapLoadFailures>,
    keymap_path_availability: Res<KeymapPathAvailability>,
    keymap_bindings: Res<KeymapBindings>,
    active_context: Res<ActiveKeymapContext>,
    effective_keymap_status: Res<EffectiveKeymapStatus>,
    command_registry: Option<Res<CommandRegistry>>,
    mut commands: Commands,
) {
    let Ok(panel) = panels.single() else {
        return;
    };
    let open_palette_panel = OpenPalettePanel(panel);
    if open_palette_panel.ime_target_ownership(&changed.target)
        != PaletteImeTargetOwnership::OwnedByOpenPanel
    {
        return;
    }
    let registry_assembly = palette_registry_assembly(command_registry.as_deref());
    rebuild_palette(
        &windows,
        panel,
        &keymap_load_failures,
        &keymap_path_availability,
        &keymap_bindings,
        &active_context,
        &effective_keymap_status,
        registry_assembly,
        &changed.snapshot.committed_text,
        &mut commands,
    );
    commands.entity(panel).insert(PaletteCommittedQuery(
        changed.snapshot.committed_text.clone(),
    ));
}

/// Rebuilds an open palette after Rubric commits new context, binding, or
/// diagnostics resources. `Update` runs after Rubric's `PreUpdate` lifecycle,
/// so this reads the committed public snapshots rather than matcher internals.
fn refresh_open_palette_after_keymap_changes(
    windows: Query<&Window, With<PrimaryWindow>>,
    panels: Query<(Entity, &PaletteCommittedQuery), With<CommandPalettePanel>>,
    keymap_load_failures: Res<KeymapLoadFailures>,
    keymap_path_availability: Res<KeymapPathAvailability>,
    keymap_bindings: Res<KeymapBindings>,
    active_context: Res<ActiveKeymapContext>,
    effective_keymap_status: Res<EffectiveKeymapStatus>,
    command_registry: Option<Res<CommandRegistry>>,
    mut commands: Commands,
) {
    let Ok((panel, palette_query)) = panels.single() else {
        return;
    };
    if !keymap_bindings.is_changed()
        && !active_context.is_changed()
        && !effective_keymap_status.is_changed()
        && !keymap_load_failures.is_changed()
    {
        return;
    }

    let registry_assembly = palette_registry_assembly(command_registry.as_deref());
    rebuild_palette(
        &windows,
        panel,
        &keymap_load_failures,
        &keymap_path_availability,
        &keymap_bindings,
        &active_context,
        &effective_keymap_status,
        registry_assembly,
        &palette_query.0,
        &mut commands,
    );
}

/// Completes palette blur commits or runs the selected explicit submission.
fn dispatch_selected_command(
    commit: On<ImeCommitRequested>,
    authority: Res<ImeCommitAuthority>,
    panels: Query<Entity, With<CommandPalettePanel>>,
    keymap_bindings: Res<KeymapBindings>,
    active_context: Res<ActiveKeymapContext>,
    effective_keymap_status: Res<EffectiveKeymapStatus>,
    command_registry: Option<Res<CommandRegistry>>,
    mut commands: Commands,
) {
    let commit = commit.event();
    let Ok(panel) = panels.single() else {
        return;
    };
    let open_palette_panel = OpenPalettePanel(panel);
    if open_palette_panel.ime_target_ownership(&commit.target)
        != PaletteImeTargetOwnership::OwnedByOpenPanel
        || !authority.is_current(commit.session_id, commit.attempt_id)
    {
        return;
    }
    if commit.cause == ImeCommitCause::Blur {
        accept_palette_commit(commit, &mut commands);
        commands.entity(panel).despawn();
        return;
    }
    let registry_assembly = palette_registry_assembly(command_registry.as_deref());
    let selection = match registry_assembly {
        PaletteRegistryAssembly::AwaitingAssembly => {
            commands.trigger(ImeRejectCommit {
                session_id: commit.session_id,
                attempt_id: commit.attempt_id,
                reason:     ImeRejection::AppOwned(String::from(
                    "the command registry is still assembling",
                )),
            });
            return;
        },
        PaletteRegistryAssembly::Assembled(command_registry) => query_command_palette(
            command_registry,
            &active_context,
            &effective_keymap_status,
            &keymap_bindings,
            &commit.text,
        )
        .selection(),
    };
    let command_id = match selection {
        PaletteSelectionOutcome::Selected(command) => command.id().clone(),
        PaletteSelectionOutcome::EmptyQuery
        | PaletteSelectionOutcome::NoMatch
        | PaletteSelectionOutcome::NotPaletteInvocable => {
            commands.trigger(ImeRejectCommit {
                session_id: commit.session_id,
                attempt_id: commit.attempt_id,
                reason:     ImeRejection::AppOwned(
                    selection_rejection_reason(selection).to_owned(),
                ),
            });
            return;
        },
    };

    accept_palette_commit(commit, &mut commands);
    commands.entity(panel).despawn();
    commands.queue(move |world: &mut World| invoke_command(&command_id, world));
}

/// Completes a palette IME attempt after the palette has accepted its text.
fn accept_palette_commit(commit: &ImeCommitRequested, commands: &mut Commands) {
    commands.trigger(ImeAcceptCommit {
        session_id: commit.session_id,
        attempt_id: commit.attempt_id,
        result:     ImeAppliedResult::AppOwned {
            display_text:   None,
            value_revision: None,
        },
    });
}

/// Describes why the shared palette selection cannot invoke a command.
const fn selection_rejection_reason(selection: PaletteSelectionOutcome<'_>) -> &'static str {
    match selection {
        PaletteSelectionOutcome::Selected(_) => "selected commands are invocable",
        PaletteSelectionOutcome::EmptyQuery => "type a command title or id",
        PaletteSelectionOutcome::NoMatch => "no command matches this text",
        PaletteSelectionOutcome::NotPaletteInvocable => {
            "that command runs from its binding, not here"
        },
    }
}

/// Closes the palette when the IME session is canceled, which is what Escape does.
fn close_on_cancel(
    canceled: On<ImeCanceled>,
    panels: Query<Entity, With<CommandPalettePanel>>,
    mut commands: Commands,
) {
    let Ok(panel) = panels.single() else {
        return;
    };
    let open_palette_panel = OpenPalettePanel(panel);
    if open_palette_panel.ime_target_ownership(&canceled.target)
        == PaletteImeTargetOwnership::OwnedByOpenPanel
    {
        commands.entity(panel).despawn();
    }
}

/// Opens the file or reveals the directory the clicked failure row names.
fn run_failure_action(
    clicked: On<ButtonClicked>,
    palette_failure_actions: Query<&PaletteFailureActions, With<CommandPalettePanel>>,
) {
    let FailureActionRow::Row(row_index) = failure_action_row_index(&clicked.id.to_string()) else {
        return;
    };
    let Ok(palette_failure_actions) = palette_failure_actions.single() else {
        return;
    };
    match palette_failure_actions.0.get(row_index) {
        Some(KeymapFailureAction::OpenFile(path)) => open_path(path, RevealInParent::No),
        Some(KeymapFailureAction::RevealDirectory(path)) => open_path(path, RevealInParent::Yes),
        Some(KeymapFailureAction::NoAction) | None => {},
    }
}

/// Whether the platform opener selects the path inside its parent rather than
/// opening the path itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RevealInParent {
    Yes,
    No,
}

/// Hands `path` to the platform's file handler.
fn open_path(path: &Path, reveal_in_parent: RevealInParent) {
    #[cfg(target_os = "macos")]
    let mut opener = {
        let mut opener = Command::new("open");
        if reveal_in_parent == RevealInParent::Yes {
            opener.arg("-R");
        }
        opener
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut opener = {
        let _ = reveal_in_parent;
        Command::new("xdg-open")
    };
    #[cfg(windows)]
    let mut opener = {
        let _ = reveal_in_parent;
        Command::new("explorer")
    };

    if let Err(error) = opener.arg(path).spawn() {
        error!(
            "fairy_dust: the command palette could not open {}: {error}",
            path.display()
        );
    }
}

fn rebuild_palette(
    windows: &Query<&Window, With<PrimaryWindow>>,
    panel: Entity,
    keymap_load_failures: &KeymapLoadFailures,
    keymap_path_availability: &KeymapPathAvailability,
    keymap_bindings: &KeymapBindings,
    active_context: &ActiveKeymapContext,
    effective_keymap_status: &EffectiveKeymapStatus,
    registry_assembly: PaletteRegistryAssembly<'_>,
    palette_query: &str,
    commands: &mut Commands,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let origin = palette_panel_origin(window);
    let panel_width = palette_panel_width(window);
    let presentation = build_palette_presentation(
        registry_assembly,
        keymap_load_failures,
        keymap_path_availability,
        keymap_bindings,
        active_context,
        effective_keymap_status,
        palette_query,
        panel_width,
    );
    commands
        .entity(panel)
        .insert(PaletteFailureActions(presentation.keymap_failure_actions));
    commands
        .entity(panel)
        .entry::<DiegeticPanel>()
        .and_modify(move |mut palette_panel| {
            let _ = palette_panel.set_screen_size(Sizing::fixed(Px(panel_width)), Sizing::FIT);
            let _ = palette_panel.set_screen_position(origin);
        });
    commands.trigger(ImeReplacePanelTree {
        panel,
        tree: presentation.layout_tree,
    });
}

/// Converts an optional shared registry into the palette's semantic assembly
/// state at its consumer boundary.
const fn palette_registry_assembly(
    command_registry: Option<&CommandRegistry>,
) -> PaletteRegistryAssembly<'_> {
    match command_registry {
        Some(command_registry) => PaletteRegistryAssembly::Assembled(command_registry),
        None => PaletteRegistryAssembly::AwaitingAssembly,
    }
}

/// Queries Rubric and builds one renderer-owned tree without retaining the
/// borrowed query result after this rebuild returns.
fn build_palette_presentation(
    registry_assembly: PaletteRegistryAssembly<'_>,
    keymap_load_failures: &KeymapLoadFailures,
    keymap_path_availability: &KeymapPathAvailability,
    keymap_bindings: &KeymapBindings,
    active_context: &ActiveKeymapContext,
    effective_keymap_status: &EffectiveKeymapStatus,
    palette_query: &str,
    panel_width: f32,
) -> PaletteTreePresentation {
    let keymap_failures = keymap_failure_rows(keymap_load_failures, keymap_path_availability);
    let keymap_failure_actions = keymap_failures
        .iter()
        .map(|keymap_failure| keymap_failure.action.clone())
        .collect();
    let layout_tree = match registry_assembly {
        PaletteRegistryAssembly::AwaitingAssembly => {
            palette_tree_awaiting_assembly(palette_query, &keymap_failures, panel_width)
        },
        PaletteRegistryAssembly::Assembled(command_registry) => {
            let query_result = query_command_palette(
                command_registry,
                active_context,
                effective_keymap_status,
                keymap_bindings,
                palette_query,
            );
            palette_tree_for_query(palette_query, &query_result, &keymap_failures, panel_width)
        },
    };

    PaletteTreePresentation {
        layout_tree,
        keymap_failure_actions,
    }
}

fn invoke_command(command_id: &CommandId, world: &mut World) {
    let outcome = world.resource_scope(|world, command_registry: Mut<CommandRegistry>| {
        command_registry.invoke(command_id, world)
    });
    match outcome {
        CommandInvocationOutcome::Invoked => {},
        CommandInvocationOutcome::UnknownCommand => {
            error!("fairy_dust: the command palette selected an unknown command {command_id}");
        },
        CommandInvocationOutcome::HeldCommandRequiresPhase => {
            error!("fairy_dust: the command palette selected the hold-to-act command {command_id}");
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod test_support {
    use std::str::FromStr;

    use bevy::prelude::App;
    use bevy::prelude::Event;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy_enhanced_input::prelude::InputAction;
    use hana_rubric::CommandId;
    use hana_rubric::KeymapPlugin;
    use hana_rubric::ReflectKeymapCommand;
    use hana_rubric::command;

    use crate::keymap::FAIRY_DUST_DEFAULT_KEYMAP;

    pub(super) const DISPATCH_COMMAND_ID: &str = "palette_test::dispatch";
    pub(super) const DISPATCH_COMMAND_TITLE: &str = "Palette Test Dispatch";
    pub(super) const HELD_COMMAND_ID: &str = "palette_test::hold";
    pub(super) const OPEN_COMMAND_ID: &str = "palette::open";

    macro_rules! canonical_context_command {
        ($action:ident, $event:ident, $id:literal) => {
            command! {
                action:      $action,
                event:       $event,
                id:          $id,
                title:       $id,
                description: "Headless registration for the canonical keymap-contexts document.",
            }
        };
    }

    canonical_context_command!(
        CanonicalEnterMainMenuAction,
        CanonicalEnterMainMenu,
        "example::enter_main_menu"
    );
    canonical_context_command!(
        CanonicalEnterRunningAction,
        CanonicalEnterRunning,
        "example::enter_running"
    );
    canonical_context_command!(
        CanonicalEnterRestingAction,
        CanonicalEnterResting,
        "example::enter_resting"
    );
    canonical_context_command!(
        CanonicalEnterDimensionLockAction,
        CanonicalEnterDimensionLock,
        "example::enter_dimension_lock"
    );
    canonical_context_command!(
        CanonicalShowGlobalRouteAction,
        CanonicalShowGlobalRoute,
        "example::show_global_route"
    );
    canonical_context_command!(
        CanonicalShowMainMenuRouteAction,
        CanonicalShowMainMenuRoute,
        "example::show_main_menu_route"
    );
    canonical_context_command!(
        CanonicalShowRunningRouteAction,
        CanonicalShowRunningRoute,
        "example::show_running_route"
    );
    canonical_context_command!(
        CanonicalShowRestingRouteAction,
        CanonicalShowRestingRoute,
        "example::show_resting_route"
    );
    canonical_context_command!(
        CanonicalShowDimensionLockRouteAction,
        CanonicalShowDimensionLockRoute,
        "example::show_dimension_lock_route"
    );
    canonical_context_command!(
        CanonicalShowCombinedRouteAction,
        CanonicalShowCombinedRoute,
        "example::show_combined_route"
    );
    canonical_context_command!(
        CanonicalShowCombinedRouteBeforeOverrideAction,
        CanonicalShowCombinedRouteBeforeOverride,
        "example::show_combined_route_before_override"
    );
    canonical_context_command!(
        CanonicalShowCombinedRouteAfterOverrideAction,
        CanonicalShowCombinedRouteAfterOverride,
        "example::show_combined_route_after_override"
    );
    canonical_context_command!(
        CanonicalShowTombstonedRouteAction,
        CanonicalShowTombstonedRoute,
        "example::show_tombstoned_route"
    );

    command! {
        action:      PaletteTestDispatch,
        event:       PaletteTestDispatchEvent,
        id:          "palette_test::dispatch",
        title:       "Palette Test Dispatch",
        description: "Dispatched by the palette selection test.",
    }

    command! {
        held,
        action:      PaletteTestHold,
        event:       PaletteTestHoldEvent,
        id:          "palette_test::hold",
        title:       "Palette Test Hold",
        description: "Hold-to-act, so the palette must not list it.",
    }

    command! {
        action:      PaletteTestRecover,
        event:       PaletteTestRecoverEvent,
        id:          "palette_test::recover",
        title:       "Palette Test Recover",
        description: "Permanently bound, and still reachable from the palette.",
        capability:  Unremappable,
    }

    /// Builds a finished app whose `CommandRegistry` holds every command this
    /// crate declares, from the document Fairy Dust actually ships. No
    /// application name is configured, so no disk worker starts and no
    /// configuration directory is touched.
    pub(super) fn palette_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(KeymapPlugin::new().with_defaults(FAIRY_DUST_DEFAULT_KEYMAP));
        app.finish();
        app
    }

    pub(super) fn command_id(text: &str) -> CommandId {
        CommandId::from_str(text).expect("test command ids are valid")
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;

    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyCode;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::NextState;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::States;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::state::app::AppExtStates;
    use bevy::state::app::StatesPlugin;
    use bevy::window::Ime;
    use bevy::window::PrimaryWindow;
    use hana_diegetic::DiegeticPanel;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::HeadlessDiegeticUiPlugin;
    use hana_diegetic::ImeApplied;
    use hana_diegetic::ImeRequestCommit;
    use hana_diegetic::ImeSessionId;
    use hana_diegetic::ImeStarted;
    use hana_diegetic::ImeValidationRejected;
    use hana_rubric::CommandKeystroke;
    use hana_rubric::Diagnostic;
    use hana_rubric::DiagnosticKind;
    use hana_rubric::DiagnosticOrigin;
    use hana_rubric::DiagnosticSeverity;
    use hana_rubric::KeymapBindingUnavailability;
    use hana_rubric::KeymapPlugin;
    use hana_rubric::Keystroke;
    use hana_rubric::PaletteBinding;
    use hana_rubric::PaletteSelectionOutcome;
    use strum::AsRefStr;
    use strum::EnumIter;
    use strum::EnumMessage;

    use self::test_support::CanonicalShowGlobalRoute;
    use self::test_support::DISPATCH_COMMAND_ID;
    use self::test_support::DISPATCH_COMMAND_TITLE;
    use self::test_support::HELD_COMMAND_ID;
    use self::test_support::OPEN_COMMAND_ID;
    use self::test_support::PaletteTestDispatchEvent;
    use self::test_support::command_id;
    use self::test_support::palette_test_app;
    use super::*;

    const LOADED_COMMAND_KEYSTROKE: &str = "ctrl-shift-r";
    const LOADED_COMMAND_TITLE: &str = "Restart Example";
    const REVEAL_FAILURE_ACTION_LABEL: &str = "Reveal";
    const CANONICAL_CONTEXT_DEFAULTS: &str =
        include_str!("../../examples/keymap_contexts.keymap.jsonc");
    const CANONICAL_PENDING_SEQUENCE_DEFAULTS: &str = r#"{
        "bindings": [{ "bindings": { "g h": "example::show_global_route" } }]
    }"#;
    const CANONICAL_CONTEXT_APPLICATION_NAME: &str = "fairy_dust_keymap_contexts";
    const CANONICAL_EXTRA_PROTECTED_KEYSTROKES: [&str; 2] = ["ctrl-shift-q", "alt-shift-x"];
    const CANONICAL_SHOW_GLOBAL_ROUTE_TITLE: &str = "example::show_global_route";
    const INVALID_CANONICAL_DEFAULTS: &str = "{ invalid default";

    #[derive(Default, Resource)]
    struct DispatchedCommands(usize);

    #[derive(Default, Resource)]
    struct CanonicalRouteInvocations(usize);

    #[derive(Default, Resource)]
    struct PaletteTreeReplacements(usize);

    #[derive(Default, Resource)]
    struct PaletteImeSessions(Vec<ImeSessionId>);

    /// Supplies a real awaiting typed-dimension context for palette refresh tests.
    #[derive(AsRefStr, Clone, Copy, Debug, EnumIter, EnumMessage, Eq, Hash, PartialEq, States)]
    #[strum(serialize_all = "snake_case")]
    enum AwaitingPaletteContext {
        #[strum(message = "While the palette refresh fixture is active")]
        Active,
    }

    /// The first simultaneously observed application dimension used by the refresh regression.
    #[derive(AsRefStr, Clone, Copy, Debug, EnumIter, EnumMessage, Eq, Hash, PartialEq, States)]
    #[strum(serialize_all = "snake_case")]
    enum PaletteApplicationState {
        #[strum(message = "While the palette application is ready")]
        Ready,
    }

    /// The second simultaneously observed interaction dimension used by the refresh regression.
    #[derive(AsRefStr, Clone, Copy, Debug, EnumIter, EnumMessage, Eq, Hash, PartialEq, States)]
    #[strum(serialize_all = "snake_case")]
    enum PaletteInteractionState {
        #[strum(message = "While the palette interaction is resting")]
        Resting,
        #[strum(message = "While the palette interaction is editing")]
        Editing,
    }

    /// The two state dimensions the canonical `keymap_contexts` document observes together.
    #[derive(
        AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
    )]
    #[strum(serialize_all = "snake_case")]
    enum CanonicalApplicationState {
        #[default]
        #[strum(message = "While the canonical example is at its main menu")]
        MainMenu,
        #[strum(message = "While the canonical example is running")]
        Running,
    }

    /// The interaction state the canonical document combines with application state.
    #[derive(
        AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
    )]
    #[strum(serialize_all = "snake_case")]
    enum CanonicalInteractionState {
        #[default]
        #[strum(message = "While the canonical example is resting")]
        Resting,
        #[strum(message = "While the canonical example holds dimension lock")]
        DimensionLock,
    }

    /// Whether the headless canonical app supplies every state dimension the document requires.
    #[derive(Clone, Copy)]
    enum CanonicalContextDimensions {
        Complete,
        InteractionUnavailable,
    }

    #[derive(Default, Resource)]
    struct PaletteImeCommitOutcomes {
        accepted: usize,
        rejected: usize,
    }

    #[derive(Default, Resource)]
    struct PaletteToggleEvents(usize);

    #[derive(Default, Resource)]
    struct AppOwnedImeSessions(Vec<ImeSessionId>);

    struct UnrelatedKeyboardOwner;

    fn palette_runtime_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((InputPlugin, HeadlessDiegeticUiPlugin))
            .init_resource::<PaletteImeSessions>()
            .add_observer(record_palette_ime_session);
        crate::keymap::install(&mut app);
        install(&mut app);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.finish();
        app
    }

    fn palette_runtime_app_with_defaults(defaults: &'static str) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((InputPlugin, HeadlessDiegeticUiPlugin))
            .init_resource::<PaletteImeSessions>()
            .add_observer(record_palette_ime_session);
        crate::keymap::configure(&mut app, crate::CommandPaletteKeymap::new(defaults));
        crate::keymap::install(&mut app);
        install(&mut app);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.finish();
        app
    }

    fn palette_runtime_app_with_two_state_dimensions() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((InputPlugin, HeadlessDiegeticUiPlugin, StatesPlugin))
            .insert_state(PaletteApplicationState::Ready)
            .insert_state(PaletteInteractionState::Resting)
            .init_resource::<PaletteImeSessions>()
            .add_observer(record_palette_ime_session)
            .add_plugins(
                KeymapPlugin::new()
                    .with_defaults(crate::keymap::FAIRY_DUST_DEFAULT_KEYMAP)
                    .with_protected_command_binding(
                        crate::command_palette_recovery_command_id(),
                        crate::command_palette_recovery_keystroke(),
                    )
                    .with_state_dimension::<PaletteApplicationState>("application")
                    .with_state_dimension::<PaletteInteractionState>("interaction"),
            );
        crate::keymap::configure(&mut app, crate::CommandPaletteKeymap::default());
        crate::keymap::install(&mut app);
        install(&mut app);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.finish();
        app.update();
        app
    }

    /// Builds the canonical contextual document against the same two state dimensions as the
    /// public example, without starting Winit.
    fn canonical_context_palette_runtime_app(
        defaults: &'static str,
        context_dimensions: CanonicalContextDimensions,
    ) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((InputPlugin, HeadlessDiegeticUiPlugin, StatesPlugin))
            .insert_state(CanonicalApplicationState::MainMenu)
            .init_resource::<CanonicalRouteInvocations>()
            .init_resource::<PaletteImeSessions>()
            .add_observer(record_palette_ime_session)
            .add_observer(
                |_route: On<CanonicalShowGlobalRoute>,
                 mut invocations: ResMut<CanonicalRouteInvocations>| {
                    invocations.0 += 1;
                },
            );
        if matches!(context_dimensions, CanonicalContextDimensions::Complete) {
            app.insert_state(CanonicalInteractionState::Resting);
        }
        app.add_plugins(canonical_context_keymap_plugin(defaults));
        crate::keymap::configure(&mut app, canonical_context_command_palette_keymap(defaults));
        crate::keymap::install(&mut app);
        install(&mut app);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.finish();
        app.update();
        app
    }

    fn canonical_context_keymap_plugin(defaults: &'static str) -> KeymapPlugin {
        let [first_keystroke, second_keystroke] = canonical_context_protected_keystrokes();
        KeymapPlugin::new()
            .with_defaults(defaults)
            .with_app_name(CANONICAL_CONTEXT_APPLICATION_NAME)
            .with_protected_command_binding(
                crate::command_palette_recovery_command_id(),
                crate::command_palette_recovery_keystroke(),
            )
            .with_protected_keystroke(first_keystroke)
            .with_protected_keystroke(second_keystroke)
            .with_state_dimension::<CanonicalApplicationState>("application")
            .with_state_dimension::<CanonicalInteractionState>("interaction")
    }

    fn canonical_context_command_palette_keymap(
        defaults: &'static str,
    ) -> crate::CommandPaletteKeymap {
        let [first_keystroke, second_keystroke] = canonical_context_protected_keystrokes();
        crate::CommandPaletteKeymap::new(defaults)
            .for_application(CANONICAL_CONTEXT_APPLICATION_NAME)
            .with_protected_keystroke(first_keystroke)
            .with_protected_keystroke(second_keystroke)
    }

    fn canonical_context_protected_keystrokes() -> [Keystroke; 2] {
        CANONICAL_EXTRA_PROTECTED_KEYSTROKES.map(|source| {
            source
                .parse()
                .expect("the canonical example's protected keystroke parses")
        })
    }

    fn awaiting_keymap_context() -> ActiveKeymapContext {
        let mut app = App::new();
        app.add_plugins(
            KeymapPlugin::new().with_state_dimension::<AwaitingPaletteContext>("palette-refresh"),
        );
        app.finish();
        app.world_mut()
            .remove_resource::<ActiveKeymapContext>()
            .unwrap_or_default()
    }

    fn terminal_palette_runtime_app(keymap_plugin: KeymapPlugin) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((InputPlugin, HeadlessDiegeticUiPlugin))
            .init_resource::<PaletteImeSessions>()
            .add_observer(record_palette_ime_session)
            .add_plugins(keymap_plugin);
        install(&mut app);
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.finish();
        app
    }

    fn record_palette_ime_session(
        started: On<ImeStarted>,
        panels: Query<Entity, With<CommandPalettePanel>>,
        mut palette_sessions: ResMut<PaletteImeSessions>,
    ) {
        let Ok(panel) = panels.single() else {
            return;
        };
        if OpenPalettePanel(panel).ime_target_ownership(&started.target)
            == PaletteImeTargetOwnership::OwnedByOpenPanel
        {
            palette_sessions.0.push(started.session_id);
        }
    }

    fn record_palette_ime_commit_acceptance(
        _applied: On<ImeApplied>,
        mut outcomes: ResMut<PaletteImeCommitOutcomes>,
    ) {
        outcomes.accepted += 1;
    }

    fn record_palette_ime_commit_rejection(
        _rejected: On<ImeValidationRejected>,
        mut outcomes: ResMut<PaletteImeCommitOutcomes>,
    ) {
        outcomes.rejected += 1;
    }

    fn record_app_owned_ime_session(
        started: On<ImeStarted>,
        mut app_owned_sessions: ResMut<AppOwnedImeSessions>,
    ) {
        if matches!(started.target, ImeTarget::AppOwned { .. }) {
            app_owned_sessions.0.push(started.session_id);
        }
    }

    fn recovery_modifier() -> KeyCode {
        if cfg!(target_os = "macos") {
            KeyCode::SuperLeft
        } else {
            KeyCode::ControlLeft
        }
    }

    fn press_recovery_keystroke(app: &mut App) {
        // A physical chord arrives as the modifier first, then the character
        // key while the modifier stays held. Advancing the schedule between
        // those inputs proves recovery reads the `ButtonInput` update Bevy
        // performs in `InputSystems`; a recovery system scheduled before that
        // set would miss the second frame's just-pressed P.
        press_keys(app, [recovery_modifier()]);
        app.update();
        press_keys(app, [KeyCode::KeyP]);
        // The recovery system triggers the observer in `PreUpdate`; the
        // observer's deferred panel spawn becomes visible on the next frame.
        app.update();
        app.world_mut().flush();
        app.update();
    }

    fn press_recovery_keystroke_again(app: &mut App) {
        release_key(app, KeyCode::KeyP);
        app.update();
        press_keys(app, [KeyCode::KeyP]);
        app.update();
        app.world_mut().flush();
        app.update();
    }

    fn release_recovery_keystroke(app: &mut App) {
        release_key(app, KeyCode::KeyP);
        release_key(app, recovery_modifier());
        app.update();
    }

    fn press_keys(app: &mut App, keys: impl IntoIterator<Item = KeyCode>) {
        let window = primary_window(app);
        for key_code in keys {
            app.world_mut().write_message(KeyboardInput {
                key_code,
                logical_key: Key::Character(String::new().into()),
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window,
            });
        }
    }

    fn release_key(app: &mut App, key_code: KeyCode) {
        let window = primary_window(app);
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Character(String::new().into()),
            state: ButtonState::Released,
            text: None,
            repeat: false,
            window,
        });
    }

    fn primary_window(app: &mut App) -> Entity {
        let world = app.world_mut();
        let mut windows = world.query_filtered::<Entity, With<PrimaryWindow>>();
        windows.single(world).expect("one primary window")
    }

    fn commit_ime_text(app: &mut App, text: &str) {
        let window = primary_window(app);
        app.world_mut().write_message(Ime::Commit {
            window,
            value: text.to_owned(),
        });
        app.update();
        app.world_mut().flush();
    }

    fn request_palette_ime_commit(app: &mut App, cause: ImeCommitCause) {
        let session_id = *app
            .world()
            .resource::<PaletteImeSessions>()
            .0
            .last()
            .expect("recovery opening creates a palette IME session");
        app.world_mut()
            .trigger(ImeRequestCommit { session_id, cause });
        app.world_mut().flush();
        app.update();
    }

    fn palette_panel_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut panels = world.query_filtered::<Entity, With<CommandPalettePanel>>();
        panels.iter(world).count()
    }

    fn palette_panel(app: &mut App) -> Entity {
        let world = app.world_mut();
        let mut panels = world.query_filtered::<Entity, With<CommandPalettePanel>>();
        panels.single(world).expect("one open command palette")
    }

    fn palette_tree_text(app: &mut App) -> String {
        let panel = palette_panel(app);
        format!(
            "{:?}",
            app.world()
                .get::<DiegeticPanel>(panel)
                .expect("open palette carries its diegetic presentation")
                .tree()
        )
    }

    fn palette_committed_query(app: &mut App) -> String {
        let panel = palette_panel(app);
        app.world()
            .get::<PaletteCommittedQuery>(panel)
            .expect("open palette keeps its committed query")
            .0
            .clone()
    }

    fn rejected_reload_diagnostic(message: &str) -> Diagnostic {
        Diagnostic {
            origin:             DiagnosticOrigin::KeymapDirectory(PathBuf::from(
                "/tmp/fairy-dust-rejected-reload",
            )),
            byte_range:         0..0,
            line:               0,
            column:             0,
            block_index:        0,
            context:            String::new(),
            original_keystroke: String::new(),
            command_id:         String::new(),
            kind:               DiagnosticKind::Command,
            severity:           DiagnosticSeverity::Failure,
            message:            message.to_owned(),
            suggestions:        Vec::new(),
        }
    }

    /// The palette dispatches through its own selection and invocation path,
    /// not by reaching into the registry, so the test drives the same code the
    /// Enter key does.
    #[test]
    fn selecting_a_row_dispatches_through_the_palette() {
        let mut app = palette_test_app();
        app.init_resource::<DispatchedCommands>().add_observer(
            |_dispatched: On<PaletteTestDispatchEvent>, mut counted: ResMut<DispatchedCommands>| {
                counted.0 += 1;
            },
        );

        let selected = {
            let query_result = query_command_palette(
                app.world().resource(),
                app.world().resource(),
                app.world().resource(),
                app.world().resource(),
                DISPATCH_COMMAND_TITLE,
            );
            let PaletteSelectionOutcome::Selected(command) = query_result.selection() else {
                panic!("the shared palette query did not select the dispatch command");
            };
            assert_eq!(command.id().as_str(), DISPATCH_COMMAND_ID);
            command.id().clone()
        };

        invoke_command(&selected, app.world_mut());

        assert_eq!(app.world().resource::<DispatchedCommands>().0, 1);
    }

    /// The shipped document leaves the recovery chord out of authored
    /// bindings, so opening remains available only through the direct Fairy Dust
    /// recovery system.
    #[test]
    fn the_shipped_defaults_leave_the_recovery_command_unbound() {
        let app = palette_test_app();

        let keymap_bindings = app.world().resource::<KeymapBindings>();

        assert!(matches!(
            keymap_bindings.keystroke(&command_id(OPEN_COMMAND_ID)),
            CommandKeystroke::Unbound
        ));
    }

    /// A row for a command the keymap binds nothing to prints no keystroke, so
    /// the column stays empty rather than falling back to the command id.
    #[test]
    fn an_unbound_command_row_carries_no_keystroke() {
        let app = palette_test_app();
        let query_result = query_command_palette(
            app.world().resource(),
            app.world().resource(),
            app.world().resource(),
            app.world().resource(),
            DISPATCH_COMMAND_TITLE,
        );

        assert_eq!(query_result.rows().len(), 1);
        assert_eq!(query_result.rows()[0].binding(), PaletteBinding::Unbound);
    }

    /// Two builder methods can each ask for the box; Bevy panics on a duplicate
    /// plugin add, so the second request has to be a no-op.
    #[test]
    fn asking_for_the_palette_twice_installs_it_once() {
        let mut app = App::new();

        install(&mut app);
        install(&mut app);

        assert!(app.is_plugin_added::<CommandPalettePlugin>());
    }

    #[test]
    fn recovery_keystroke_reads_bevy_keyboard_events_after_input_updates() {
        let mut app = palette_runtime_app();

        press_recovery_keystroke(&mut app);
        assert_eq!(app.world().resource::<PaletteImeSessions>().0.len(), 1);
        assert_eq!(palette_panel_count(&mut app), 1);
    }

    #[test]
    fn recovery_keystroke_opens_and_closes_the_palette_outside_keymap_routing() {
        let mut app = palette_runtime_app();

        press_recovery_keystroke(&mut app);
        app.update();

        assert_eq!(palette_panel_count(&mut app), 1);

        press_recovery_keystroke_again(&mut app);
        app.update();

        assert_eq!(palette_panel_count(&mut app), 0);
    }

    #[test]
    fn canonical_context_recovery_keeps_ime_visible_rows_and_palette_invocation_working() {
        let mut app = canonical_context_palette_runtime_app(
            CANONICAL_CONTEXT_DEFAULTS,
            CanonicalContextDimensions::Complete,
        );
        assert!(
            matches!(
                app.world().resource::<KeymapBindings>().authored(),
                hana_rubric::AuthoredKeymapBindings::Loaded(_)
            ),
            "{:?}",
            app.world().resource::<KeymapLoadFailures>()
        );

        press_recovery_keystroke(&mut app);
        assert_eq!(app.world().resource::<PaletteImeSessions>().0.len(), 1);
        assert_eq!(palette_panel_count(&mut app), 1);
        commit_ime_text(&mut app, CANONICAL_SHOW_GLOBAL_ROUTE_TITLE);
        assert!(palette_tree_text(&mut app).contains(CANONICAL_SHOW_GLOBAL_ROUTE_TITLE));

        press_recovery_keystroke_again(&mut app);
        assert_eq!(palette_panel_count(&mut app), 0);
        release_recovery_keystroke(&mut app);

        press_recovery_keystroke(&mut app);
        commit_ime_text(&mut app, CANONICAL_SHOW_GLOBAL_ROUTE_TITLE);
        request_palette_ime_commit(&mut app, ImeCommitCause::Request);

        assert_eq!(app.world().resource::<CanonicalRouteInvocations>().0, 1);
        assert_eq!(palette_panel_count(&mut app), 0);
    }

    #[test]
    fn canonical_context_recovery_cancels_a_pending_sequence_before_toggling() {
        let mut app = canonical_context_palette_runtime_app(
            CANONICAL_PENDING_SEQUENCE_DEFAULTS,
            CanonicalContextDimensions::Complete,
        );

        press_keys(&mut app, [KeyCode::KeyG]);
        app.update();
        release_key(&mut app, KeyCode::KeyG);
        app.update();
        press_recovery_keystroke(&mut app);
        press_recovery_keystroke_again(&mut app);
        release_recovery_keystroke(&mut app);
        press_keys(&mut app, [KeyCode::KeyH]);
        app.update();

        assert_eq!(palette_panel_count(&mut app), 0);
        assert_eq!(app.world().resource::<CanonicalRouteInvocations>().0, 0);
    }

    #[test]
    fn canonical_context_recovery_stays_usable_for_invalid_defaults_and_unavailable_dimensions() {
        for (defaults, context_dimensions, expected_binding_text) in [
            (
                INVALID_CANONICAL_DEFAULTS,
                CanonicalContextDimensions::Complete,
                "embedded defaults invalid",
            ),
            (
                CANONICAL_CONTEXT_DEFAULTS,
                CanonicalContextDimensions::InteractionUnavailable,
                "Context unavailable: interaction",
            ),
        ] {
            let mut app = canonical_context_palette_runtime_app(defaults, context_dimensions);

            press_recovery_keystroke(&mut app);
            assert_eq!(palette_panel_count(&mut app), 1);
            commit_ime_text(&mut app, CANONICAL_SHOW_GLOBAL_ROUTE_TITLE);
            let presentation = palette_tree_text(&mut app);
            assert!(
                presentation.contains(expected_binding_text),
                "expected `{expected_binding_text}` in {presentation}"
            );
            request_palette_ime_commit(&mut app, ImeCommitCause::Request);

            assert_eq!(app.world().resource::<CanonicalRouteInvocations>().0, 1);
            assert_eq!(palette_panel_count(&mut app), 0);
        }
    }

    #[test]
    fn recovery_keystroke_cancels_pending_sequences_before_the_palette_closes() {
        let mut app = palette_runtime_app_with_defaults(
            r#"{ "bindings": [{ "bindings": { "g h": "palette_test::dispatch" } }] }"#,
        );
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            hana_rubric::AuthoredKeymapBindings::Loaded(_)
        ));
        app.init_resource::<PaletteToggleEvents>().add_observer(
            |_open: On<OpenCommandPaletteEvent>, mut events: ResMut<PaletteToggleEvents>| {
                events.0 += 1;
            },
        );

        let keyboard_claim = app
            .world_mut()
            .resource_mut::<KeystrokeRouting>()
            .take_for_text_entry(
                KeyboardOwner::of::<UnrelatedKeyboardOwner>(),
                [CommandId::declared::<OpenCommandPaletteEvent>()],
            );
        assert_eq!(keyboard_claim, KeyboardClaim::Granted);
        app.update();

        press_keys(&mut app, [KeyCode::KeyG]);
        app.update();
        release_key(&mut app, KeyCode::KeyG);
        app.update();

        press_recovery_keystroke(&mut app);
        press_recovery_keystroke_again(&mut app);
        release_key(&mut app, KeyCode::KeyP);
        release_key(&mut app, recovery_modifier());
        app.update();
        press_keys(&mut app, [KeyCode::KeyH]);
        app.update();
        app.world_mut().flush();
        app.update();

        assert_eq!(palette_panel_count(&mut app), 0);
        assert_eq!(app.world().resource::<PaletteToggleEvents>().0, 2);
    }

    #[test]
    fn authored_secondary_shift_p_does_not_toggle_direct_recovery() {
        let mut app = palette_runtime_app_with_defaults(
            r#"{ "bindings": [{ "bindings": { "secondary-shift-p": "palette_test::dispatch" } }] }"#,
        );
        app.init_resource::<DispatchedCommands>().add_observer(
            |_dispatched: On<PaletteTestDispatchEvent>, mut counted: ResMut<DispatchedCommands>| {
                counted.0 += 1;
            },
        );

        press_keys(
            &mut app,
            [recovery_modifier(), KeyCode::ShiftLeft, KeyCode::KeyP],
        );
        app.update();

        assert_eq!(app.world().resource::<DispatchedCommands>().0, 1);
        assert_eq!(palette_panel_count(&mut app), 0);
    }

    #[test]
    fn terminal_keymap_states_render_and_invoke_through_the_palette_ime_boundary() {
        for (keymap_plugin, unavailability, unavailable_label) in [
            (
                KeymapPlugin::new(),
                KeymapBindingUnavailability::Unconfigured,
                "keymap unconfigured",
            ),
            (
                KeymapPlugin::new().with_protected_command_binding(
                    crate::command_palette_recovery_command_id(),
                    crate::command_palette_recovery_keystroke(),
                ),
                KeymapBindingUnavailability::MissingDefault,
                "embedded defaults missing",
            ),
            (
                KeymapPlugin::new().with_defaults("{ invalid default"),
                KeymapBindingUnavailability::InvalidDefault,
                "embedded defaults invalid",
            ),
        ] {
            let mut app = terminal_palette_runtime_app(keymap_plugin);
            app.init_resource::<DispatchedCommands>().add_observer(
                |_dispatched: On<PaletteTestDispatchEvent>,
                 mut counted: ResMut<DispatchedCommands>| {
                    counted.0 += 1;
                },
            );

            press_recovery_keystroke(&mut app);
            app.update();

            assert_eq!(palette_panel_count(&mut app), 1);
            assert!(matches!(
                app.world().resource::<KeymapBindings>().authored(),
                hana_rubric::AuthoredKeymapBindings::Unavailable(actual) if actual == unavailability
            ));
            assert!(palette_tree_text(&mut app).contains(unavailable_label));
            let panel = palette_panel(&mut app);
            assert_eq!(
                app.world()
                    .get::<PaletteFailureActions>(panel)
                    .expect("terminal keymap presentation retains its repair rows")
                    .0,
                vec![KeymapFailureAction::NoAction]
            );

            commit_ime_text(&mut app, DISPATCH_COMMAND_TITLE);
            assert_eq!(palette_committed_query(&mut app), DISPATCH_COMMAND_TITLE);
            request_palette_ime_commit(&mut app, ImeCommitCause::Request);

            assert_eq!(app.world().resource::<DispatchedCommands>().0, 1);
            assert_eq!(palette_panel_count(&mut app), 0);
        }
    }

    #[test]
    fn blurring_an_empty_palette_query_completes_without_validation_or_dispatch_and_closes() {
        let mut app = palette_runtime_app();
        app.init_resource::<PaletteImeCommitOutcomes>()
            .init_resource::<DispatchedCommands>()
            .add_observer(record_palette_ime_commit_acceptance)
            .add_observer(record_palette_ime_commit_rejection)
            .add_observer(
                |_dispatched: On<PaletteTestDispatchEvent>,
                 mut counted: ResMut<DispatchedCommands>| {
                    counted.0 += 1;
                },
            );

        press_recovery_keystroke(&mut app);
        request_palette_ime_commit(&mut app, ImeCommitCause::Blur);

        let outcomes = app.world().resource::<PaletteImeCommitOutcomes>();
        assert_eq!(outcomes.accepted, 1);
        assert_eq!(outcomes.rejected, 0);
        assert_eq!(app.world().resource::<DispatchedCommands>().0, 0);
        assert_eq!(palette_panel_count(&mut app), 0);
    }

    #[test]
    fn blurring_a_matching_palette_query_never_invokes_the_command() {
        let mut app = palette_runtime_app();
        app.init_resource::<PaletteImeCommitOutcomes>()
            .init_resource::<DispatchedCommands>()
            .add_observer(record_palette_ime_commit_acceptance)
            .add_observer(record_palette_ime_commit_rejection)
            .add_observer(
                |_dispatched: On<PaletteTestDispatchEvent>,
                 mut counted: ResMut<DispatchedCommands>| {
                    counted.0 += 1;
                },
            );

        press_recovery_keystroke(&mut app);
        commit_ime_text(&mut app, DISPATCH_COMMAND_TITLE);
        request_palette_ime_commit(&mut app, ImeCommitCause::Blur);

        let outcomes = app.world().resource::<PaletteImeCommitOutcomes>();
        assert_eq!(outcomes.accepted, 1);
        assert_eq!(outcomes.rejected, 0);
        assert_eq!(app.world().resource::<DispatchedCommands>().0, 0);
        assert_eq!(palette_panel_count(&mut app), 0);
    }

    #[test]
    fn shared_selection_outcomes_keep_their_consumer_meanings_distinct() {
        let app = palette_test_app();
        let selection = |query| {
            query_command_palette(
                app.world().resource(),
                app.world().resource(),
                app.world().resource(),
                app.world().resource(),
                query,
            )
            .selection()
        };

        assert!(matches!(selection(""), PaletteSelectionOutcome::EmptyQuery));
        assert!(matches!(
            selection("this command does not exist"),
            PaletteSelectionOutcome::NoMatch
        ));
        assert!(matches!(
            selection(HELD_COMMAND_ID),
            PaletteSelectionOutcome::NotPaletteInvocable
        ));
        assert!(matches!(
            selection(DISPATCH_COMMAND_TITLE),
            PaletteSelectionOutcome::Selected(_)
        ));
    }

    #[test]
    fn unrelated_app_owned_query_session_cannot_invoke_a_palette_command_without_an_open_panel() {
        let mut app = palette_runtime_app();
        app.init_resource::<AppOwnedImeSessions>()
            .init_resource::<DispatchedCommands>()
            .add_observer(record_app_owned_ime_session)
            .add_observer(
                |_dispatched: On<PaletteTestDispatchEvent>,
                 mut counted: ResMut<DispatchedCommands>| {
                    counted.0 += 1;
                },
            );
        let owner = app.world_mut().spawn_empty().id();
        let window = primary_window(&mut app);
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::AppOwned {
                owner,
                field_id: FIELD_ID.into(),
            },
            window,
            initial_text: DISPATCH_COMMAND_TITLE.to_owned(),
            field_spec: ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new(FIELD_ID)),
            anchor: None,
        });
        app.world_mut().flush();
        let session_id = *app
            .world()
            .resource::<AppOwnedImeSessions>()
            .0
            .last()
            .expect("the unrelated app-owned session starts");

        app.world_mut().trigger(ImeRequestCommit {
            session_id,
            cause: ImeCommitCause::Request,
        });
        app.world_mut().flush();
        app.update();

        assert_eq!(palette_panel_count(&mut app), 0);
        assert_eq!(app.world().resource::<DispatchedCommands>().0, 0);
    }

    #[test]
    fn distinct_same_origin_rejections_refresh_once_each_without_replacing_bindings() {
        let mut app = palette_runtime_app();
        app.init_resource::<PaletteTreeReplacements>().add_observer(
            |_replacement: On<ImeReplacePanelTree>, mut count: ResMut<PaletteTreeReplacements>| {
                count.0 += 1;
            },
        );
        press_recovery_keystroke(&mut app);
        commit_ime_text(&mut app, LOADED_COMMAND_TITLE);
        app.update();
        app.world_mut().resource_mut::<PaletteTreeReplacements>().0 = 0;

        app.update();
        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 0);

        app.world_mut()
            .resource_mut::<KeymapLoadFailures>()
            .diagnostics = vec![rejected_reload_diagnostic("first rejected document")];
        app.update();
        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 1);
        let first_presentation = palette_tree_text(&mut app);
        assert!(first_presentation.contains("first rejected document"));
        assert!(first_presentation.contains(REVEAL_FAILURE_ACTION_LABEL));
        assert!(first_presentation.contains(LOADED_COMMAND_KEYSTROKE));
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            hana_rubric::AuthoredKeymapBindings::Loaded(_)
        ));

        app.world_mut()
            .resource_mut::<KeymapLoadFailures>()
            .diagnostics = vec![rejected_reload_diagnostic("second rejected document")];
        app.update();
        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 2);
        let second_presentation = palette_tree_text(&mut app);
        assert!(second_presentation.contains("second rejected document"));
        assert!(!second_presentation.contains("first rejected document"));
        assert!(second_presentation.contains(REVEAL_FAILURE_ACTION_LABEL));
        assert!(second_presentation.contains(LOADED_COMMAND_KEYSTROKE));
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            hana_rubric::AuthoredKeymapBindings::Loaded(_)
        ));

        app.update();
        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 2);
    }

    #[test]
    fn keymap_binding_refresh_reuses_the_open_palette_query_once() {
        let mut app = palette_runtime_app();
        app.init_resource::<PaletteTreeReplacements>().add_observer(
            |_replacement: On<ImeReplacePanelTree>, mut count: ResMut<PaletteTreeReplacements>| {
                count.0 += 1;
            },
        );
        press_recovery_keystroke(&mut app);
        app.update();
        commit_ime_text(&mut app, DISPATCH_COMMAND_TITLE);
        app.world_mut().resource_mut::<PaletteTreeReplacements>().0 = 0;

        app.world_mut().insert_resource(KeymapBindings::from(
            KeymapBindingUnavailability::MissingDefault,
        ));
        app.update();

        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 1);
        assert_eq!(palette_committed_query(&mut app), DISPATCH_COMMAND_TITLE);
        assert!(palette_tree_text(&mut app).contains("embedded defaults missing"));

        app.update();
        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 1);
    }

    #[test]
    fn active_keymap_context_refresh_reuses_the_open_palette_query_once() {
        let mut app = palette_runtime_app();
        app.init_resource::<PaletteTreeReplacements>().add_observer(
            |_replacement: On<ImeReplacePanelTree>, mut count: ResMut<PaletteTreeReplacements>| {
                count.0 += 1;
            },
        );
        press_recovery_keystroke(&mut app);
        app.update();
        commit_ime_text(&mut app, DISPATCH_COMMAND_TITLE);
        app.world_mut().resource_mut::<PaletteTreeReplacements>().0 = 0;

        app.world_mut().insert_resource(awaiting_keymap_context());
        app.update();

        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 1);
        assert_eq!(palette_committed_query(&mut app), DISPATCH_COMMAND_TITLE);
        assert!(palette_tree_text(&mut app).contains("state loading"));

        app.update();
        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 1);
    }

    #[test]
    fn two_state_dimensions_refresh_the_retained_query_with_both_resolved_values() {
        let mut app = palette_runtime_app_with_two_state_dimensions();
        app.init_resource::<PaletteTreeReplacements>().add_observer(
            |_replacement: On<ImeReplacePanelTree>, mut count: ResMut<PaletteTreeReplacements>| {
                count.0 += 1;
            },
        );
        press_recovery_keystroke(&mut app);
        commit_ime_text(&mut app, LOADED_COMMAND_TITLE);
        app.world_mut().resource_mut::<PaletteTreeReplacements>().0 = 0;

        app.world_mut()
            .resource_mut::<NextState<PaletteInteractionState>>()
            .set(PaletteInteractionState::Editing);
        app.update();
        app.update();

        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 1);
        assert_eq!(palette_committed_query(&mut app), LOADED_COMMAND_TITLE);
        let presentation = palette_tree_text(&mut app);
        assert!(presentation.contains("Context: application=ready, interaction=editing"));

        app.update();
        assert_eq!(app.world().resource::<PaletteTreeReplacements>().0, 1);
    }
}
