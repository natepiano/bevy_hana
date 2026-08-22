//! Capability: example-owned keyboard shortcuts that never collide with Fairy
//! Dust's own chords.
//!
//! Examples register a key with
//! [`SprinkleBuilder::with_shortcut`](crate::SprinkleBuilder::with_shortcut)
//! (runs once per press) or
//! [`SprinkleBuilder::with_held_shortcut`](crate::SprinkleBuilder::with_held_shortcut)
//! (runs every frame while held). Each registers a `(key, system)` pair; the
//! example never names an input type, so its only imports stay `bevy` and
//! `fairy_dust`.
//!
//! [`run_shortcuts`] runs a registered system only when its key fires **and no
//! modifier is held**. Fairy Dust's own chords (`Ctrl+Shift+L` and friends)
//! fire only *with* their modifiers, so a bare example key and a Fairy Dust
//! chord on the same letter never both fire — the modifier guard is what the
//! original raw-input examples were missing.
//!
//! Bare keys Fairy Dust already binds (`H` home, `P` cube spin or fold play)
//! register into [`ReservedKeys`]. A second capability is rejected immediately,
//! while [`assert_no_reserved_collisions`] rejects an example shortcut that
//! reuses one at startup.

use std::any::TypeId;

use bevy::ecs::system::SystemId;
use bevy::prelude::App;
use bevy::prelude::ButtonInput;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Update;
use bevy::prelude::With;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use hana_diegetic::ImeInputBlocker;
use hana_rubric::AuthoredKeymapBindings;
use hana_rubric::CommandKeystroke;
use hana_rubric::CommandRegistry;
use hana_rubric::KeymapBindingUnavailability;
use hana_rubric::KeymapBindings;
use hana_rubric::Modifiers;
use hana_rubric::PrimaryTrigger;

use crate::constants::MODIFIER_KEYS;

/// When a registered shortcut's system runs relative to the key press.
#[derive(Clone, Copy)]
enum ShortcutTiming {
    /// Once, on the frame the key goes down.
    Press,
    /// Every frame the key is held.
    Held,
}

/// Marks that the shortcuts capability has been installed, so repeated
/// `with_shortcut` calls add the `Startup`/`Update` systems only once.
#[derive(Resource)]
struct ShortcutsInstalled;

/// Whether every example shortcut has been checked against the authored
/// keymap bindings that can route beside it.
#[derive(Default, Resource)]
enum ShortcutCollisionCheckLifecycle {
    /// Startup did not yet have a loaded authored binding table to inspect.
    #[default]
    WaitingForAuthoredBindings,
    /// The check ran against the loaded table, or no keymap plugin exists.
    Complete,
}

/// A Fairy Dust bare-key binding that example shortcuts must not reuse.
struct ReservedKey {
    key:        KeyCode,
    owner:      TypeId,
    owner_name: &'static str,
    label:      &'static str,
}

/// Bare keys already bound by Fairy Dust capabilities. Populated and checked at
/// capability install, then read by [`assert_no_reserved_collisions`].
#[derive(Resource, Default)]
struct ReservedKeys(Vec<ReservedKey>);

struct ShortcutRegistration {
    key:       KeyCode,
    timing:    ShortcutTiming,
    system_id: SystemId,
}

/// Example shortcuts recorded during builder construction, run by
/// [`run_shortcuts`].
#[derive(Resource, Default)]
struct ShortcutRegistrations(Vec<ShortcutRegistration>);

/// Adds the shortcut registry, reserved-key check, and runner exactly once.
/// Idempotent — called by every `with_shortcut` / `with_held_shortcut`.
pub(crate) fn install(app: &mut App) {
    app.init_resource::<ShortcutRegistrations>();
    app.init_resource::<ReservedKeys>();
    app.init_resource::<ShortcutCollisionCheckLifecycle>();
    if app.world().contains_resource::<ShortcutsInstalled>() {
        return;
    }
    app.insert_resource(ShortcutsInstalled);
    app.add_systems(Startup, assert_no_reserved_collisions);
    app.add_systems(Update, run_shortcuts);
}

/// Whether the window is free of a text editor, which is the condition every
/// registered shortcut runs under.
///
/// While the command palette's query field holds the window's IME lease, its
/// keystrokes are text. Running them as shortcuts too would home the camera
/// every time the reader typed `h`.
fn no_text_entry_in_progress(
    ime_input_blocker: Option<Res<ImeInputBlocker>>,
    windows: Query<Entity, With<PrimaryWindow>>,
) -> bool {
    let (Some(ime_input_blocker), Ok(window)) = (ime_input_blocker, windows.single()) else {
        return true;
    };
    !ime_input_blocker.blocks_window(window)
}

/// Records `key` to run `system_id` once each time it is pressed.
pub(crate) fn register_press(app: &mut App, key: KeyCode, system_id: SystemId) {
    push(app, key, ShortcutTiming::Press, system_id);
}

/// Records `key` to run `system_id` every frame it is held.
pub(crate) fn register_held(app: &mut App, key: KeyCode, system_id: SystemId) {
    push(app, key, ShortcutTiming::Held, system_id);
}

fn push(app: &mut App, key: KeyCode, timing: ShortcutTiming, system_id: SystemId) {
    app.world_mut()
        .resource_mut::<ShortcutRegistrations>()
        .0
        .push(ShortcutRegistration {
            key,
            timing,
            system_id,
        });
}

/// Records a Fairy Dust bare-key binding. Repeated reservations by `O` are
/// idempotent; another owner reserving `key` is rejected immediately, and
/// [`assert_no_reserved_collisions`] rejects example shortcuts at startup.
pub(crate) fn reserve_key<O: 'static>(app: &mut App, key: KeyCode, label: &'static str) {
    app.init_resource::<ReservedKeys>();
    let owner = TypeId::of::<O>();
    let owner_name = std::any::type_name::<O>();
    let mut reserved = app.world_mut().resource_mut::<ReservedKeys>();
    if let Some(existing) = reserved.0.iter().find(|reserved| reserved.key == key) {
        assert!(
            existing.owner == owner,
            "fairy_dust reserved key {:?} for `{}` ({}) collides with `{}` ({}); use only one capability for a bare key",
            key,
            label,
            owner_name,
            existing.label,
            existing.owner_name,
        );
        return;
    }
    reserved.0.push(ReservedKey {
        key,
        owner,
        owner_name,
        label,
    });
}

/// Runs each registered shortcut whose key fires this frame, skipping all of
/// them while any modifier is held so bare keys never shadow Fairy Dust chords.
fn run_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    registrations: Res<ShortcutRegistrations>,
    reserved: Res<ReservedKeys>,
    command_registry: Option<Res<CommandRegistry>>,
    keymap_bindings: Option<Res<KeymapBindings>>,
    mut collision_check_lifecycle: ResMut<ShortcutCollisionCheckLifecycle>,
    ime_input_blocker: Option<Res<ImeInputBlocker>>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
) {
    recheck_collisions_when_authored_bindings_load(
        &registrations,
        &reserved,
        command_registry.as_deref(),
        keymap_bindings.as_deref(),
        &mut collision_check_lifecycle,
    );
    if !no_text_entry_in_progress(ime_input_blocker, windows) {
        return;
    }
    if keys.any_pressed(MODIFIER_KEYS) {
        return;
    }
    for registration in &registrations.0 {
        let fired = match registration.timing {
            ShortcutTiming::Press => keys.just_pressed(registration.key),
            ShortcutTiming::Held => keys.pressed(registration.key),
        };
        if fired {
            commands.run_system(registration.system_id);
        }
    }
}

/// Fails the run at startup if an example shortcut reuses a key Fairy Dust
/// already binds bare, turning a silent double-fire into a clear error.
///
/// Two registries can claim a bare key. [`ReservedKeys`] holds the ones a
/// capability wires straight to a [`KeyCode`], and the keymap holds the ones a
/// document binds to a command — including a user document that moved a Fairy
/// Dust chord onto a bare letter. This runs in `Startup`, after `finish` has
/// committed the first keymap generation, so [`KeymapBindings`] is the live
/// table rather than an empty one.
fn assert_no_reserved_collisions(
    registrations: Res<ShortcutRegistrations>,
    reserved: Res<ReservedKeys>,
    command_registry: Option<Res<CommandRegistry>>,
    keymap_bindings: Option<Res<KeymapBindings>>,
    mut collision_check_lifecycle: ResMut<ShortcutCollisionCheckLifecycle>,
) {
    let collision_check_availability = ShortcutCollisionCheckAvailability::from_resources(
        command_registry.as_deref(),
        keymap_bindings.as_deref(),
    );
    check_for_reserved_collisions(&registrations, &reserved, collision_check_availability);
    *collision_check_lifecycle = match collision_check_availability {
        ShortcutCollisionCheckAvailability::NoKeymapPlugin
        | ShortcutCollisionCheckAvailability::ReadyToCheck { .. } => {
            ShortcutCollisionCheckLifecycle::Complete
        },
        ShortcutCollisionCheckAvailability::AwaitingRegistryAssembly
        | ShortcutCollisionCheckAvailability::AuthoredBindingsUnavailable(_) => {
            ShortcutCollisionCheckLifecycle::WaitingForAuthoredBindings
        },
    };
}

/// Performs the one deferred collision check after Rubric publishes authored
/// bindings. The lifecycle becomes complete before this function returns, so a
/// stable loaded resource cannot run example shortcuts through this path twice.
fn recheck_collisions_when_authored_bindings_load(
    registrations: &ShortcutRegistrations,
    reserved: &ReservedKeys,
    command_registry: Option<&CommandRegistry>,
    keymap_bindings: Option<&KeymapBindings>,
    collision_check_lifecycle: &mut ShortcutCollisionCheckLifecycle,
) {
    if !matches!(
        &*collision_check_lifecycle,
        ShortcutCollisionCheckLifecycle::WaitingForAuthoredBindings
    ) {
        return;
    }

    let collision_check_availability =
        ShortcutCollisionCheckAvailability::from_resources(command_registry, keymap_bindings);
    if !matches!(
        collision_check_availability,
        ShortcutCollisionCheckAvailability::ReadyToCheck { .. }
    ) {
        return;
    }

    *collision_check_lifecycle = ShortcutCollisionCheckLifecycle::Complete;
    check_for_reserved_collisions(registrations, reserved, collision_check_availability);
}

/// Rejects an example shortcut when a built-in capability or a loaded authored
/// binding fires from that same bare key.
fn check_for_reserved_collisions(
    registrations: &ShortcutRegistrations,
    reserved: &ReservedKeys,
    collision_check_availability: ShortcutCollisionCheckAvailability<'_>,
) {
    let collisions: Vec<String> = registrations
        .0
        .iter()
        .filter_map(|registration| {
            let shortcut_key_claim = reserved
                .0
                .iter()
                .find(|reserved| reserved.key == registration.key)
                .map_or_else(
                    || collision_check_availability.bare_key_claim(registration.key),
                    |reserved| ShortcutKeyClaim::ClaimedBy(reserved.label.to_owned()),
                );

            match shortcut_key_claim {
                ShortcutKeyClaim::Unclaimed => None,
                ShortcutKeyClaim::ClaimedBy(claimant) => Some(format!(
                    "{:?} collides with the reserved `{claimant}` binding",
                    registration.key
                )),
            }
        })
        .collect();

    // `panic!` is denied workspace-wide; `assert!` is the allowed hard-fail.
    assert!(
        collisions.is_empty(),
        "fairy_dust example shortcut key {}; use the matching Fairy Dust capability or pick a \
         different key",
        collisions.join("; "),
    );
}

/// Whether authored keymap bindings can currently answer a bare-key collision.
///
/// An installed Rubric runtime always owns [`KeymapBindings`], including while
/// authored defaults are unavailable. The resource's absence therefore means
/// no [`KeymapPlugin`](hana_rubric::KeymapPlugin) was installed; registry and
/// authored availability describe the two later assembly boundaries.
#[derive(Clone, Copy)]
enum ShortcutCollisionCheckAvailability<'world> {
    /// No keymap plugin is installed, so no document can claim the key.
    NoKeymapPlugin,
    /// The keymap plugin is installed but command metadata is not ready yet.
    AwaitingRegistryAssembly,
    /// Authored keymap bindings cannot yet name a collision.
    AuthoredBindingsUnavailable(KeymapBindingUnavailability),
    /// Loaded command metadata and authored bindings can answer the question.
    ReadyToCheck {
        command_registry: &'world CommandRegistry,
        keymap_bindings:  &'world KeymapBindings,
    },
}

impl<'world> ShortcutCollisionCheckAvailability<'world> {
    const fn from_resources(
        command_registry: Option<&'world CommandRegistry>,
        keymap_bindings: Option<&'world KeymapBindings>,
    ) -> Self {
        let Some(keymap_bindings) = keymap_bindings else {
            return Self::NoKeymapPlugin;
        };
        let Some(command_registry) = command_registry else {
            return Self::AwaitingRegistryAssembly;
        };
        match keymap_bindings.authored() {
            AuthoredKeymapBindings::Unavailable(unavailability) => {
                Self::AuthoredBindingsUnavailable(unavailability)
            },
            AuthoredKeymapBindings::Loaded(_) => Self::ReadyToCheck {
                command_registry,
                keymap_bindings,
            },
        }
    }

    /// The command the live keymap runs from `key` alone.
    ///
    /// Only a one-keystroke, modifier-free binding can double-fire with an
    /// example shortcut: [`run_shortcuts`] stands down while any modifier is
    /// held.
    ///
    /// No-keymap, registry-assembly, and authored-unavailable outcomes leave
    /// the key unclaimed for now. Startup records the latter two in
    /// [`ShortcutCollisionCheckLifecycle`] and checks once more after a loaded
    /// authored table is published.
    fn bare_key_claim(&self, key: KeyCode) -> ShortcutKeyClaim {
        let (command_registry, keymap_bindings) = match self {
            Self::ReadyToCheck {
                command_registry,
                keymap_bindings,
            } => (command_registry, keymap_bindings),
            Self::NoKeymapPlugin => return ShortcutKeyClaim::Unclaimed,
            Self::AwaitingRegistryAssembly => {
                warn!(
                    "fairy_dust: example shortcut key {key:?} awaits command-registry assembly \
                     before its authored-keymap collision check"
                );
                return ShortcutKeyClaim::Unclaimed;
            },
            Self::AuthoredBindingsUnavailable(unavailability) => {
                warn!(
                    "fairy_dust: example shortcut key {key:?} awaits authored keymap bindings \
                     ({unavailability:?}) before its collision check"
                );
                return ShortcutKeyClaim::Unclaimed;
            },
        };

        command_registry
            .iter()
            .find(|command_info| {
                keystroke_on_bare_key(keymap_bindings.keystroke(command_info.id), key)
            })
            .map_or(ShortcutKeyClaim::Unclaimed, |command_info| {
                ShortcutKeyClaim::ClaimedBy(command_info.id.to_string())
            })
    }
}

/// Whether something other than the example shortcut already fires from a key
/// pressed alone.
enum ShortcutKeyClaim {
    /// A reserved capability key or a live keymap binding fires from it.
    ClaimedBy(String),
    /// Nothing else fires from it.
    Unclaimed,
}

fn keystroke_on_bare_key(command_keystroke: CommandKeystroke<'_>, key: KeyCode) -> bool {
    let CommandKeystroke::BoundTo(keystroke_sequence) = command_keystroke else {
        return false;
    };
    let [keystroke] = keystroke_sequence.as_slice() else {
        return false;
    };
    keystroke.modifiers() == Modifiers::none()
        && matches!(
            keystroke.primary_trigger(),
            PrimaryTrigger::OrdinaryKey(ordinary_key) if ordinary_key.key_code() == key
        )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::panic::AssertUnwindSafe;

    use bevy::app::Update;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::NextState;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Startup;
    use bevy::prelude::State;
    use bevy::prelude::States;
    use bevy::state::app::AppExtStates;
    use bevy::state::state::StateTransition;
    use bevy::window::PrimaryWindow;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::HeadlessDiegeticUiPlugin;
    use hana_diegetic::ImeAppOwnedFieldSpec;
    use hana_diegetic::ImeEditableFieldSpec;
    use hana_diegetic::ImeOpenSession;
    use hana_diegetic::ImeTarget;
    use hana_rubric::CommandRegistry;
    use hana_rubric::KeymapBindingUnavailability;
    use hana_rubric::KeymapBindings;

    use super::ReservedKeys;
    use super::ShortcutCollisionCheckAvailability;
    use super::ShortcutCollisionCheckLifecycle;
    use super::ShortcutRegistrations;
    use super::assert_no_reserved_collisions;
    use super::install;
    use super::register_press;
    use super::reserve_key;
    use crate::AssetRootPending;
    use crate::CommandPaletteKeymap;
    use crate::NoOrbitCam;
    use crate::SprinkleBuilder;

    include!("../examples/support/keymap_contexts_shortcuts.rs");

    struct FirstCapability;
    struct SecondCapability;

    #[derive(Default, Resource)]
    struct ShortcutRuns(usize);

    /// The application state controlled by the canonical `1` and `2` shortcuts.
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, States)]
    enum ContextShortcutApplicationState {
        #[default]
        MainMenu,
        Running,
    }

    /// The independent interaction state controlled by the canonical `3` and `4` shortcuts.
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, States)]
    enum ContextShortcutInteractionState {
        #[default]
        Resting,
        DimensionLock,
    }

    fn enter_main_menu(mut next_state: ResMut<NextState<ContextShortcutApplicationState>>) {
        next_state.set(ContextShortcutApplicationState::MainMenu);
    }

    fn enter_running(mut next_state: ResMut<NextState<ContextShortcutApplicationState>>) {
        next_state.set(ContextShortcutApplicationState::Running);
    }

    fn enter_resting(mut next_state: ResMut<NextState<ContextShortcutInteractionState>>) {
        next_state.set(ContextShortcutInteractionState::Resting);
    }

    fn enter_dimension_lock(mut next_state: ResMut<NextState<ContextShortcutInteractionState>>) {
        next_state.set(ContextShortcutInteractionState::DimensionLock);
    }

    /// Builds the exact permanent-control `SprinkleBuilder::with_shortcut` mapping that the
    /// canonical example installs, with Fairy Dust's test-only headless baseline.
    fn permanent_context_control_builder() -> SprinkleBuilder<NoOrbitCam> {
        let builder = SprinkleBuilder::<NoOrbitCam, AssetRootPending>::new(App::new());
        let mut builder = with_context_state_shortcuts!(
            builder;
            enter_main_menu,
            enter_running,
            enter_resting,
            enter_dimension_lock
        );
        builder
            .app_mut()
            .insert_state(ContextShortcutApplicationState::MainMenu)
            .insert_state(ContextShortcutInteractionState::Resting);
        builder
    }

    /// Runs the production shortcut runner, then commits the state systems it requested, without
    /// starting Winit or the full application loop.
    fn advance_permanent_context_controls(builder: &mut SprinkleBuilder<NoOrbitCam>) {
        let world = builder.app_mut().world_mut();
        world.run_schedule(Update);
        world.run_schedule(StateTransition);
    }

    fn press_permanent_context_control(builder: &mut SprinkleBuilder<NoOrbitCam>, key: KeyCode) {
        builder
            .app_mut()
            .world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        advance_permanent_context_controls(builder);
        let mut keys = builder
            .app_mut()
            .world_mut()
            .resource_mut::<ButtonInput<KeyCode>>();
        keys.clear_just_pressed(key);
        keys.release(key);
        keys.clear_just_released(key);
    }

    /// Builds the same registered command table and committed authored binding resource an
    /// application receives after Rubric finishes a valid default document.
    fn loaded_keymap_resources(defaults: &'static str) -> (CommandRegistry, KeymapBindings) {
        let mut source = App::new();
        crate::keymap::configure(&mut source, CommandPaletteKeymap::new(defaults));
        crate::keymap::install(&mut source);
        source.finish();

        let command_registry = source
            .world_mut()
            .remove_resource::<CommandRegistry>()
            .expect("a finished keymap owns its command registry");
        let keymap_bindings = source
            .world_mut()
            .remove_resource::<KeymapBindings>()
            .expect("a finished keymap owns its bindings");
        (command_registry, keymap_bindings)
    }

    /// Builds the pre-load shortcut state that has command metadata but cannot yet inspect an
    /// authored binding for collision.
    fn awaiting_authored_bindings_shortcut_app(command_registry: CommandRegistry) -> App {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ShortcutRuns>();
        install(&mut app);
        app.insert_resource(command_registry);
        app.insert_resource(KeymapBindings::from(
            KeymapBindingUnavailability::AwaitingInitialLoad,
        ));
        let system_id = app.register_system(|mut runs: ResMut<ShortcutRuns>| runs.0 += 1);
        register_press(&mut app, KeyCode::KeyQ, system_id);
        app
    }

    /// A key bound as an example shortcut is text while the palette's query
    /// field holds the window's IME lease, so the shortcut must not run.
    #[test]
    fn a_bound_key_does_not_fire_while_an_editor_holds_the_lease() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        app.init_resource::<ShortcutRuns>();
        let window = app.world_mut().spawn(PrimaryWindow).id();
        let system_id = app.register_system(|mut runs: ResMut<ShortcutRuns>| runs.0 += 1);
        install(&mut app);
        register_press(&mut app, KeyCode::KeyH, system_id);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);

        app.update();
        let ran_before_the_lease = app.world().resource::<ShortcutRuns>().0;

        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::AppOwned {
                owner:    window,
                field_id: "query".into(),
            },
            window,
            initial_text: String::new(),
            field_spec: ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("query")),
            anchor: None,
        });
        app.update();

        assert_eq!(ran_before_the_lease, 1);
        assert_eq!(app.world().resource::<ShortcutRuns>().0, 1);
    }

    #[test]
    fn same_capability_key_reservation_is_idempotent() {
        let mut app = App::new();

        reserve_key::<FirstCapability>(&mut app, KeyCode::KeyP, "first");
        reserve_key::<FirstCapability>(&mut app, KeyCode::KeyP, "first");

        assert_eq!(app.world().resource::<ReservedKeys>().0.len(), 1);
    }

    #[test]
    fn different_capability_key_reservation_is_rejected() {
        let mut app = App::new();
        reserve_key::<FirstCapability>(&mut app, KeyCode::KeyP, "first");

        let collision = std::panic::catch_unwind(AssertUnwindSafe(|| {
            reserve_key::<SecondCapability>(&mut app, KeyCode::KeyP, "second");
        }));

        assert!(collision.is_err());
    }

    #[test]
    fn permanent_context_controls_use_the_builder_shortcut_mapping_and_return_from_opposites() {
        let mut builder = permanent_context_control_builder();

        press_permanent_context_control(&mut builder, KeyCode::Digit2);
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::Running
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::Resting
        );

        press_permanent_context_control(&mut builder, KeyCode::Digit4);
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::Running
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::DimensionLock
        );

        press_permanent_context_control(&mut builder, KeyCode::Digit1);
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::MainMenu
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::DimensionLock
        );

        press_permanent_context_control(&mut builder, KeyCode::Digit3);
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::MainMenu
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::Resting
        );
    }

    #[test]
    fn permanent_context_controls_apply_independent_state_changes_in_one_frame() {
        let mut builder = permanent_context_control_builder();
        {
            let mut keys = builder
                .app_mut()
                .world_mut()
                .resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::Digit2);
            keys.press(KeyCode::Digit4);
        }

        advance_permanent_context_controls(&mut builder);

        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::Running
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::DimensionLock
        );
    }

    #[test]
    fn permanent_context_controls_keep_working_when_authored_routing_is_unavailable() {
        let (command_registry, _) = loaded_keymap_resources(r#"{ "bindings": [] }"#);
        let mut builder = permanent_context_control_builder();
        builder.app_mut().insert_resource(command_registry);
        builder.app_mut().insert_resource(KeymapBindings::from(
            KeymapBindingUnavailability::InvalidDefault,
        ));
        let collision_check_availability = {
            let app = builder.app_mut();
            ShortcutCollisionCheckAvailability::from_resources(
                app.world().get_resource::<CommandRegistry>(),
                app.world().get_resource::<KeymapBindings>(),
            )
        };
        assert!(matches!(
            collision_check_availability,
            ShortcutCollisionCheckAvailability::AuthoredBindingsUnavailable(
                KeymapBindingUnavailability::InvalidDefault
            )
        ));

        press_permanent_context_control(&mut builder, KeyCode::Digit2);
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::Running
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::Resting
        );

        press_permanent_context_control(&mut builder, KeyCode::Digit4);
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::Running
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::DimensionLock
        );

        press_permanent_context_control(&mut builder, KeyCode::Digit1);
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::MainMenu
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::DimensionLock
        );

        press_permanent_context_control(&mut builder, KeyCode::Digit3);

        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutApplicationState>>()
                .get(),
            &ContextShortcutApplicationState::MainMenu
        );
        assert_eq!(
            builder
                .app_mut()
                .world()
                .resource::<State<ContextShortcutInteractionState>>()
                .get(),
            &ContextShortcutInteractionState::Resting
        );
    }

    /// A user keymap can bind a Fairy Dust command to a bare letter, which the
    /// hardcoded reservation list knows nothing about. The example shortcut on
    /// that letter would then double-fire, so the startup check reads the live
    /// keymap as well.
    #[test]
    fn an_example_shortcut_on_a_bare_key_the_keymap_binds_is_rejected_at_startup() {
        let mut app = App::new();
        app.init_resource::<ShortcutRegistrations>();
        app.init_resource::<ReservedKeys>();
        crate::keymap::configure(
            &mut app,
            CommandPaletteKeymap::new(
                r#"{ "bindings": [{ "bindings": { "h": "fairy_dust::show_help" } }] }"#,
            ),
        );
        crate::keymap::install(&mut app);
        app.finish();
        let system_id = app.world_mut().register_system(|| {});
        register_press(&mut app, KeyCode::KeyH, system_id);

        let collision = std::panic::catch_unwind(AssertUnwindSafe(|| {
            app.add_systems(Startup, assert_no_reserved_collisions);
            app.update();
        }));

        assert!(collision.is_err());
    }

    /// A bare key reserved by a capability is refused to an example shortcut,
    /// which would otherwise double-fire against the reserving capability.
    #[test]
    fn an_example_shortcut_on_a_reserved_key_is_rejected_at_startup() {
        let mut app = App::new();
        app.init_resource::<ShortcutRegistrations>();
        reserve_key::<FirstCapability>(&mut app, KeyCode::KeyP, "first");
        let system_id = app.world_mut().register_system(|| {});
        register_press(&mut app, KeyCode::KeyP, system_id);

        let collision = std::panic::catch_unwind(AssertUnwindSafe(|| {
            app.add_systems(Startup, assert_no_reserved_collisions);
            app.update();
        }));

        assert!(collision.is_err());
    }

    #[test]
    fn non_colliding_shortcut_rechecks_once_after_authored_bindings_load() {
        let (command_registry, keymap_bindings) = loaded_keymap_resources(
            r#"{ "bindings": [{ "bindings": { "h": "fairy_dust::show_help" } }] }"#,
        );
        let mut app = awaiting_authored_bindings_shortcut_app(command_registry);
        assert!(matches!(
            ShortcutCollisionCheckAvailability::from_resources(
                app.world().get_resource::<CommandRegistry>(),
                app.world().get_resource::<KeymapBindings>(),
            ),
            ShortcutCollisionCheckAvailability::AuthoredBindingsUnavailable(
                KeymapBindingUnavailability::AwaitingInitialLoad
            )
        ));

        app.update();
        assert!(matches!(
            app.world().resource::<ShortcutCollisionCheckLifecycle>(),
            ShortcutCollisionCheckLifecycle::WaitingForAuthoredBindings
        ));

        app.insert_resource(keymap_bindings);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.update();
        assert!(matches!(
            app.world().resource::<ShortcutCollisionCheckLifecycle>(),
            ShortcutCollisionCheckLifecycle::Complete
        ));
        assert_eq!(app.world().resource::<ShortcutRuns>().0, 1);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::KeyQ);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyQ);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_released(KeyCode::KeyQ);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.update();

        assert_eq!(app.world().resource::<ShortcutRuns>().0, 2);
    }

    #[test]
    fn delayed_authored_collision_fails_before_the_shortcut_runs() {
        let (command_registry, keymap_bindings) = loaded_keymap_resources(
            r#"{ "bindings": [{ "bindings": { "q": "fairy_dust::show_help" } }] }"#,
        );
        let mut app = awaiting_authored_bindings_shortcut_app(command_registry);
        app.update();

        app.insert_resource(keymap_bindings);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        let collision = std::panic::catch_unwind(AssertUnwindSafe(|| app.update()));

        assert!(collision.is_err());
        assert_eq!(app.world().resource::<ShortcutRuns>().0, 0);
    }
}
