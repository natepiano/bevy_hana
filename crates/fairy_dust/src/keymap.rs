//! Baseline capability: the one `hana_rubric` [`KeymapPlugin`] every Fairy Dust
//! app installs, and the document it is configured from.
//!
//! Fairy Dust declares its own capabilities with `command!`, so the command
//! registry — not a `bevy_enhanced_input` binding — is what turns `Ctrl+Shift+R`
//! into a restart. Every example therefore needs the keymap runtime, whether or
//! not it opens the command palette, which is why
//! [`SprinkleBuilder::run`](crate::SprinkleBuilder::run) installs the plugin
//! unconditionally.
//!
//! The install is deferred to `run` rather than done in the baseline plugin set
//! because a plugin's configuration is fixed at `add_plugins` time, while
//! [`SprinkleBuilder::with_command_palette_keymap`](crate::SprinkleBuilder::with_command_palette_keymap)
//! can name a different document at any point in the builder chain. Recording
//! the choice first and installing once at the end is what lets the two agree.

use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::App;
use bevy::prelude::Resource;
use hana_rubric::CommandId;
use hana_rubric::KeymapConfigurationDirectory;
use hana_rubric::KeymapPlugin;
use hana_rubric::Keystroke;

use crate::command_palette;

/// Fairy Dust's shipped default keymap, binding its authored capabilities.
///
/// Command+P on macOS and Control+P elsewhere open the palette through the
/// protected direct-recovery path rather than this authored document.
pub(crate) const FAIRY_DUST_DEFAULT_KEYMAP: &str = include_str!("../assets/keymap.default.jsonc");

/// The physical chord Fairy Dust reserves to recover the command palette when
/// a keymap cannot route any authored binding.
///
/// It is Command+P on macOS and Control+P elsewhere. Applications that install
/// a contextual [`KeymapPlugin`] before Fairy Dust must pass both
/// [`command_palette_recovery_command_id`] and this value to
/// [`KeymapPlugin::with_protected_command_binding`] so their configuration
/// agrees with the direct recovery route.
#[must_use]
pub fn command_palette_recovery_keystroke() -> Keystroke {
    match "secondary-p".parse() {
        Ok(keystroke) => keystroke,
        Err(error) => {
            bevy::log::error!(
                "fairy_dust: the built-in recovery keystroke could not be parsed: {error}"
            );
            std::process::abort();
        },
    }
}

/// Returns the command id the application-owned recovery chord invokes.
///
/// Applications that install the state-dimension `KeymapPlugin` themselves use this together
/// with [`command_palette_recovery_keystroke`] so their base configuration exactly matches Fairy
/// Dust's deferred install.
#[must_use]
pub fn command_palette_recovery_command_id() -> CommandId { command_palette::recovery_command_id() }

/// Reports whether this frame pressed Fairy Dust's recovery chord.
///
/// This deliberately reads physical input rather than going through Rubric's
/// keymap routing: the chord must still open the repair palette when no
/// validated bindings exist.
pub(crate) fn recovery_keystroke_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    keys.just_pressed(KeyCode::KeyP)
        && recovery_has_secondary_modifier(keys)
        && !recovery_has_extra_modifier(keys)
}

/// Reports whether either physical platform-secondary modifier is down.
fn recovery_has_secondary_modifier(keys: &ButtonInput<KeyCode>) -> bool {
    let secondary_modifiers = if cfg!(target_os = "macos") {
        [KeyCode::SuperLeft, KeyCode::SuperRight]
    } else {
        [KeyCode::ControlLeft, KeyCode::ControlRight]
    };
    secondary_modifiers
        .into_iter()
        .any(|modifier| keys.pressed(modifier))
}

/// Reports modifiers that turn Command/Control+P into a different authored
/// chord, such as secondary-shift-p.
fn recovery_has_extra_modifier(keys: &ButtonInput<KeyCode>) -> bool {
    let non_secondary_modifiers = if cfg!(target_os = "macos") {
        [KeyCode::ControlLeft, KeyCode::ControlRight]
    } else {
        [KeyCode::SuperLeft, KeyCode::SuperRight]
    };
    [
        KeyCode::ShiftLeft,
        KeyCode::ShiftRight,
        KeyCode::AltLeft,
        KeyCode::AltRight,
    ]
    .into_iter()
    .chain(non_secondary_modifiers)
    .any(|modifier| keys.pressed(modifier))
}

/// The keymap document Fairy Dust's `KeymapPlugin` is installed with, and
/// whether that keymap reads and writes a configuration directory.
///
/// Configuring an application name is what lets a user keep their own
/// `keymap.jsonc` next to the published defaults. Without one, no disk worker
/// starts, nothing is written, and the palette reports the missing
/// configuration directory as a failure row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPaletteKeymap {
    defaults:                &'static str,
    configuration_directory: KeymapConfigurationDirectory,
    protected_keystrokes:    Vec<Keystroke>,
}

impl CommandPaletteKeymap {
    /// Builds a keymap from a JSONC document, normally supplied with
    /// `include_str!`.
    ///
    /// The document replaces Fairy Dust's shipped defaults outright, so it
    /// binds every `fairy_dust::` command as well as the application's own
    /// command ids. Fairy Dust reserves Command+P on macOS and Control+P
    /// elsewhere for the direct recovery path, so an authored document cannot
    /// bind that exact chord. A document naming a command the registry does not
    /// declare is rejected whole, which leaves its authored bindings dead but
    /// keeps recovery available.
    #[must_use]
    pub const fn new(defaults: &'static str) -> Self {
        Self {
            defaults,
            configuration_directory: KeymapConfigurationDirectory::Unconfigured,
            protected_keystrokes: Vec::new(),
        }
    }

    /// Reads and writes the user's own keymap under `app_name` in the platform
    /// configuration directory.
    ///
    /// Without this the palette reports the unavailable configuration directory
    /// as a failure row, because a keymap the user cannot edit is a real
    /// limitation rather than a normal state.
    #[must_use]
    pub fn for_application(mut self, app_name: &str) -> Self {
        self.configuration_directory =
            KeymapConfigurationDirectory::ForApplication(app_name.to_owned());
        self
    }

    /// Reserves `keystroke` from user-authored bindings, the way
    /// [`KeymapPlugin::with_protected_keystroke`] does.
    ///
    /// An application that installs its own contextual `KeymapPlugin` must
    /// pass [`command_palette_recovery_command_id`] and
    /// [`command_palette_recovery_keystroke`] to
    /// [`KeymapPlugin::with_protected_command_binding`], because `hana_rubric`
    /// refuses two plugin configurations that disagree on defaults,
    /// application name, or protected associations. Repeating Fairy Dust's
    /// built-in recovery chord here is harmless and does not duplicate it in
    /// the configuration.
    #[must_use]
    pub fn with_protected_keystroke(mut self, keystroke: Keystroke) -> Self {
        if !self.protected_keystrokes.contains(&keystroke) {
            self.protected_keystrokes.push(keystroke);
        }
        self
    }

    fn keymap_plugin(&self) -> KeymapPlugin {
        let recovery_keystroke = command_palette_recovery_keystroke();
        let keymap_plugin = KeymapPlugin::new()
            .with_defaults(self.defaults)
            .with_protected_command_binding(
                command_palette_recovery_command_id(),
                recovery_keystroke,
            );
        let mut keymap_plugin = match &self.configuration_directory {
            KeymapConfigurationDirectory::ForApplication(app_name) => {
                keymap_plugin.with_app_name(app_name)
            },
            KeymapConfigurationDirectory::Unconfigured => keymap_plugin,
        };
        for keystroke in &self.protected_keystrokes {
            if *keystroke != recovery_keystroke {
                keymap_plugin = keymap_plugin.with_protected_keystroke(*keystroke);
            }
        }
        keymap_plugin
    }
}

impl Default for CommandPaletteKeymap {
    fn default() -> Self { Self::new(FAIRY_DUST_DEFAULT_KEYMAP) }
}

/// The keymap the application chose, recorded before the plugin is built so a
/// second request that disagrees is refused rather than silently ignored.
#[derive(Resource)]
struct ChosenKeymap(CommandPaletteKeymap);

/// Records `keymap` as the document Fairy Dust's `KeymapPlugin` will be built
/// from.
///
/// A second request carrying a different keymap is refused: silently keeping the
/// first document would leave an application running bindings it never asked
/// for.
pub(crate) fn configure(app: &mut App, keymap: CommandPaletteKeymap) {
    if let Some(chosen_keymap) = app.world().get_resource::<ChosenKeymap>() {
        assert!(
            chosen_keymap.0 == keymap,
            "fairy_dust: the keymap is already configured with a different document. Call \
             `with_command_palette` or `with_command_palette_keymap` once."
        );
        return;
    }
    app.insert_resource(ChosenKeymap(keymap));
}

/// Whether a builder call has named the keymap Fairy Dust's `KeymapPlugin` is
/// built from.
#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum KeymapChoice {
    /// A builder call recorded this document.
    Named(CommandPaletteKeymap),
    /// No builder call named one, so `run` installs Fairy Dust's shipped
    /// defaults.
    Unnamed,
}

/// The keymap recorded so far.
#[cfg(test)]
pub(crate) fn chosen(app: &App) -> KeymapChoice {
    app.world()
        .get_resource::<ChosenKeymap>()
        .map_or(KeymapChoice::Unnamed, |chosen_keymap| {
            KeymapChoice::Named(chosen_keymap.0.clone())
        })
}

/// Adds the one `KeymapPlugin` this app gets, built from the recorded keymap or
/// Fairy Dust's shipped defaults.
///
/// Fairy Dust always submits the selected base configuration. Rubric permits
/// repeated base-plugin installation and rejects any configuration mismatch,
/// so an application-owned state-dimension plugin must include the same
/// defaults and protected recovery association.
pub(crate) fn install(app: &mut App) {
    if app.world().contains_resource::<FairyDustKeymapInstalled>() {
        return;
    }
    let keymap = app
        .world()
        .get_resource::<ChosenKeymap>()
        .map_or_else(CommandPaletteKeymap::default, |chosen| chosen.0.clone());
    app.add_plugins(keymap.keymap_plugin());
    app.insert_resource(FairyDustKeymapInstalled);
}

/// Marks the base keymap plugin that Fairy Dust itself installed.
#[derive(Resource)]
struct FairyDustKeymapInstalled;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::str::FromStr;

    use bevy::input::ButtonInput;
    use bevy::input::keyboard::KeyCode;
    use bevy::prelude::App;
    use bevy::prelude::States;
    use hana_rubric::CommandId;
    use hana_rubric::CommandKeystroke;
    use hana_rubric::CommandLookup;
    use hana_rubric::CommandRegistry;
    use hana_rubric::KeymapBindingUnavailability;
    use hana_rubric::KeymapBindings;
    use hana_rubric::KeymapPlugin;
    use strum::AsRefStr;
    use strum::EnumIter;
    use strum::EnumMessage;

    use super::CommandPaletteKeymap;
    use super::FAIRY_DUST_DEFAULT_KEYMAP;
    use super::command_palette_recovery_keystroke;
    use super::configure;
    use super::install;
    use super::recovery_keystroke_pressed;

    const OTHER_KEYMAP: &str = r#"{ "bindings": [] }"#;
    const CONST_KEYMAP: CommandPaletteKeymap = CommandPaletteKeymap::new(OTHER_KEYMAP);
    const STATE_DIMENSION_APPLICATION_NAME: &str = "fairy-dust-keymap-agreement";

    /// Stands in for an application that registers one typed keymap state dimension.
    #[derive(AsRefStr, Clone, Copy, Debug, EnumIter, EnumMessage, Eq, Hash, PartialEq, States)]
    #[strum(serialize_all = "snake_case")]
    enum RecoveryContext {
        #[strum(message = "While the recovery test context is active")]
        Active,
    }

    /// Every capability Fairy Dust declares, and the palette that lists them.
    const FAIRY_DUST_AUTHORED_COMMAND_IDS: [&str; 6] = [
        "fairy_dust::restart",
        "fairy_dust::toggle_screen_space_panels",
        "fairy_dust::toggle_home_aabb_gizmo",
        "fairy_dust::show_help",
        "fairy_dust::toggle_free_cam_look_pitch",
        "fairy_dust::cycle_camera_preset",
    ];

    fn command_id(text: &str) -> CommandId {
        CommandId::from_str(text).expect("fairy dust command ids are valid")
    }

    #[test]
    fn an_unconfigured_app_installs_the_shipped_defaults() {
        let mut app = App::new();

        install(&mut app);

        assert!(app.is_plugin_added::<KeymapPlugin>());
        assert_eq!(
            CommandPaletteKeymap::default(),
            CommandPaletteKeymap::new(FAIRY_DUST_DEFAULT_KEYMAP)
        );
    }

    #[test]
    fn command_palette_keymap_can_still_be_declared_in_constants() {
        assert_eq!(CONST_KEYMAP, CommandPaletteKeymap::new(OTHER_KEYMAP));
    }

    #[test]
    fn repeating_the_same_choice_is_accepted() {
        let mut app = App::new();

        configure(&mut app, CommandPaletteKeymap::new(OTHER_KEYMAP));
        configure(&mut app, CommandPaletteKeymap::new(OTHER_KEYMAP));
        install(&mut app);

        assert!(app.is_plugin_added::<KeymapPlugin>());
    }

    /// An application-owned state-dimension plugin supplies the complete base
    /// configuration before Fairy Dust's deferred install repeats it.
    #[test]
    fn a_state_dimension_plugin_agrees_on_every_base_keymap_field() -> Result<(), String> {
        let additional_protected_keystroke = "ctrl-shift-p"
            .parse()
            .map_err(|error| format!("additional protected keystroke parses: {error}"))?;
        let mut app = App::new();
        app.add_plugins(
            KeymapPlugin::new()
                .with_defaults(FAIRY_DUST_DEFAULT_KEYMAP)
                .with_app_name(STATE_DIMENSION_APPLICATION_NAME)
                .with_protected_command_binding(
                    super::command_palette_recovery_command_id(),
                    command_palette_recovery_keystroke(),
                )
                .with_protected_keystroke(additional_protected_keystroke)
                .with_state_dimension::<RecoveryContext>("recovery"),
        );

        configure(
            &mut app,
            CommandPaletteKeymap::default()
                .for_application(STATE_DIMENSION_APPLICATION_NAME)
                .with_protected_keystroke(additional_protected_keystroke),
        );
        install(&mut app);

        assert!(app.is_plugin_added::<KeymapPlugin>());
        Ok(())
    }

    /// A state-dimension plugin that omits Fairy Dust's automatic recovery association
    /// disagrees with every `CommandPaletteKeymap` configuration.
    #[test]
    #[should_panic(expected = "already installed with different defaults")]
    fn a_state_dimension_plugin_missing_the_reserved_chord_is_refused() {
        let mut app = App::new();
        app.add_plugins(
            KeymapPlugin::new()
                .with_defaults(FAIRY_DUST_DEFAULT_KEYMAP)
                .with_state_dimension::<RecoveryContext>("recovery"),
        );

        configure(&mut app, CommandPaletteKeymap::default());
        install(&mut app);
    }

    #[test]
    fn a_second_fairy_dust_install_leaves_its_first_plugin_in_place() {
        let mut app = App::new();

        install(&mut app);
        install(&mut app);

        assert!(app.is_plugin_added::<KeymapPlugin>());
    }

    #[test]
    fn every_palette_keymap_rejects_an_authored_recovery_binding() {
        let mut app = App::new();
        configure(
            &mut app,
            CommandPaletteKeymap::new(
                r#"{ "bindings": [{ "bindings": { "secondary-p": "palette::open" } }] }"#,
            ),
        );
        install(&mut app);
        app.finish();

        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            hana_rubric::AuthoredKeymapBindings::Unavailable(
                KeymapBindingUnavailability::InvalidDefault
            )
        ));
    }

    #[test]
    fn recovery_input_accepts_each_platform_secondary_side_and_their_combination() {
        let secondary_modifiers = if cfg!(target_os = "macos") {
            [KeyCode::SuperLeft, KeyCode::SuperRight]
        } else {
            [KeyCode::ControlLeft, KeyCode::ControlRight]
        };

        for modifier in secondary_modifiers {
            let mut keys = ButtonInput::<KeyCode>::default();
            keys.press(modifier);
            keys.press(KeyCode::KeyP);

            assert!(recovery_keystroke_pressed(&keys));
        }

        let mut keys = ButtonInput::<KeyCode>::default();
        for modifier in secondary_modifiers {
            keys.press(modifier);
        }
        keys.press(KeyCode::KeyP);

        assert!(recovery_keystroke_pressed(&keys));
    }

    #[test]
    fn recovery_input_rejects_every_extra_modifier() {
        let secondary_modifier = if cfg!(target_os = "macos") {
            KeyCode::SuperLeft
        } else {
            KeyCode::ControlLeft
        };
        let non_secondary_modifier = if cfg!(target_os = "macos") {
            KeyCode::ControlLeft
        } else {
            KeyCode::SuperLeft
        };

        for extra_modifier in [
            KeyCode::ShiftLeft,
            KeyCode::AltRight,
            non_secondary_modifier,
        ] {
            let mut keys = ButtonInput::<KeyCode>::default();
            keys.press(secondary_modifier);
            keys.press(extra_modifier);
            keys.press(KeyCode::KeyP);

            assert!(
                !recovery_keystroke_pressed(&keys),
                "{extra_modifier:?} must turn the chord into a non-recovery binding"
            );
        }
    }

    #[test]
    #[should_panic(expected = "already installed with different defaults")]
    fn a_preinstalled_mismatched_keymap_plugin_is_rejected_before_runtime() {
        let mut app = App::new();
        app.add_plugins(KeymapPlugin::new().with_defaults(FAIRY_DUST_DEFAULT_KEYMAP));

        install(&mut app);
    }

    /// The whole point of the baseline install: an application that never asks
    /// for the command palette still reaches every Fairy Dust capability by its
    /// keystroke. A missing binding here is a dead hotkey in every example.
    #[test]
    fn the_baseline_install_binds_every_fairy_dust_command() {
        let mut app = App::new();
        install(&mut app);
        app.finish();

        let keymap_bindings = app.world().resource::<KeymapBindings>();
        for command_id in FAIRY_DUST_AUTHORED_COMMAND_IDS.map(command_id) {
            assert!(
                matches!(
                    keymap_bindings.keystroke(&command_id),
                    CommandKeystroke::BoundTo(_)
                ),
                "the shipped defaults leave `{command_id}` unbound"
            );
        }
    }

    /// The registry is what the palette lists, so a command missing here — or
    /// carrying an unauthored title — is one a user cannot find by name.
    #[test]
    fn every_fairy_dust_command_is_listed_with_an_authored_title() {
        let mut app = App::new();
        install(&mut app);
        app.finish();

        let command_registry = app.world().resource::<CommandRegistry>();
        for command_id in FAIRY_DUST_AUTHORED_COMMAND_IDS.map(command_id) {
            let CommandLookup::Found(command_info) = command_registry.lookup(&command_id) else {
                panic!("`{command_id}` is bound by the shipped defaults but never declared");
            };
            assert!(
                !command_info.title.is_empty(),
                "`{command_id}` carries no title"
            );
            assert!(
                command_info.capability.is_palette_invocable(),
                "`{command_id}` is declared but the palette will not list it"
            );
        }
    }
}
