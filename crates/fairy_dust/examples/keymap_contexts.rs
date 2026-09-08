//! One Fairy Dust application with two simultaneous, application-owned keymap states.
//!
//! The keymap document is authored against `application` and `interaction` independently.
//! Rubric observes the complete pair of states, while this application owns every transition.

#[path = "keymap_contexts/status_panel.rs"]
mod status_panel;

use bevy::prelude::App;
use bevy::prelude::AppExtStates;
use bevy::prelude::ClearColor;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::KeyCode;
use bevy::prelude::NextState;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::States;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use fairy_dust::CommandPaletteKeymap;
use fairy_dust::Face;
use fairy_dust::OrbitCam;
use fairy_dust::OrbitCamPose;
use fairy_dust::SprinkleBuilder;
use fairy_dust::TitleBar;
use fairy_dust::command_palette_recovery_command_id;
use fairy_dust::command_palette_recovery_keystroke;
use fairy_dust::sprinkle_example;
use hana_rubric::CommandId;
use hana_rubric::KeymapPlugin;
use hana_rubric::Keystroke;
use strum::AsRefStr;
use strum::EnumIter;
use strum::EnumMessage;

#[cfg(test)]
use self::status_panel::context_status_sections;
use self::status_panel::refresh_context_status_panel;
use self::status_panel::spawn_context_status_panel;

include!("support/keymap_contexts_shortcuts.rs");

const APPLICATION_NAME: &str = "fairy_dust_keymap_contexts";
const CAMERA_FOCUS: Vec3 = Vec3::new(0.0, 0.5, 0.0);
const COMBINED_ROUTE_AFTER_OVERRIDE_COLOR: Color = Color::srgb(0.4, 0.28, 0.04);
const COMBINED_ROUTE_BEFORE_OVERRIDE_COLOR: Color = Color::srgb(0.26, 0.1, 0.16);
const COMBINED_ROUTE_COLOR: Color = Color::srgb(0.32, 0.06, 0.12);
const CONTEXT_CUBE_COLOR: Color = Color::srgb(0.2, 0.25, 0.32);
const CONTEXT_CUBE_LABEL_COLOR: Color = Color::srgb(0.7, 0.88, 1.0);
const CONTEXT_CUBE_LABEL_SIZE: f32 = 0.12;
const CONTEXT_CUBE_SIZE: f32 = 1.0;
const DIMENSION_LOCK_ROUTE_COLOR: Color = Color::srgb(0.3, 0.18, 0.05);
const EXAMPLE_PROTECTED_KEYSTROKES: [&str; 2] = ["ctrl-shift-q", "alt-shift-x"];
const GLOBAL_ROUTE_COLOR: Color = Color::srgb(0.08, 0.16, 0.25);
const GROUND_COLOR: Color = Color::srgb(0.09, 0.11, 0.14);
const GROUND_SIZE: f32 = 6.0;
const INITIAL_EFFECT_COLOR: Color = Color::srgb(0.08, 0.08, 0.1);
const KEYMAP_DEFAULTS: &str = include_str!("keymap_contexts.keymap.jsonc");
const MAIN_MENU_ROUTE_COLOR: Color = Color::srgb(0.2, 0.12, 0.28);
const RESTING_ROUTE_COLOR: Color = Color::srgb(0.08, 0.25, 0.28);
const RUNNING_ROUTE_COLOR: Color = Color::srgb(0.1, 0.28, 0.16);
const TOMBSTONED_ROUTE_COLOR: Color = Color::srgb(0.2, 0.03, 0.03);

const CONTEXT_CAMERA_POSE: OrbitCamPose = OrbitCamPose {
    focus:  CAMERA_FOCUS,
    yaw:    -0.6,
    pitch:  0.35,
    radius: 6.0,
};

mod commands {
    use bevy::prelude::Event;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy_enhanced_input::prelude::InputAction;
    use hana_rubric::ReflectKeymapCommand;
    use hana_rubric::command;

    command! {
        action:      EnterMainMenuAction,
        event:       EnterMainMenu,
        id:          "example::enter_main_menu",
        title:       "Enter Main Menu",
        description: "Set the application state to its main menu without changing interaction state.",
    }

    command! {
        action:      EnterRunningAction,
        event:       EnterRunning,
        id:          "example::enter_running",
        title:       "Enter Running",
        description: "Set the application state to running without changing interaction state.",
    }

    command! {
        action:      EnterRestingAction,
        event:       EnterResting,
        id:          "example::enter_resting",
        title:       "Enter Resting",
        description: "Set the interaction state to resting without changing application state.",
    }

    command! {
        action:      EnterDimensionLockAction,
        event:       EnterDimensionLock,
        id:          "example::enter_dimension_lock",
        title:       "Enter Dimension Lock",
        description: "Set the interaction state to dimension lock without changing application state.",
    }

    command! {
        action:      ShowGlobalRouteAction,
        event:       ShowGlobalRoute,
        id:          "example::show_global_route",
        title:       "Show Global Route",
        description: "Color the scene for the binding that applies to every state snapshot.",
    }

    command! {
        action:      ShowMainMenuRouteAction,
        event:       ShowMainMenuRoute,
        id:          "example::show_main_menu_route",
        title:       "Show Main Menu Route",
        description: "Color the scene for the main-menu application binding.",
    }

    command! {
        action:      ShowRunningRouteAction,
        event:       ShowRunningRoute,
        id:          "example::show_running_route",
        title:       "Show Running Route",
        description: "Color the scene for the running application binding.",
    }

    command! {
        action:      ShowRestingRouteAction,
        event:       ShowRestingRoute,
        id:          "example::show_resting_route",
        title:       "Show Resting Route",
        description: "Color the scene for the resting interaction binding.",
    }

    command! {
        action:      ShowDimensionLockRouteAction,
        event:       ShowDimensionLockRoute,
        id:          "example::show_dimension_lock_route",
        title:       "Show Dimension Lock Route",
        description: "Color the scene for the dimension-lock interaction binding.",
    }

    command! {
        action:      ShowCombinedRouteAction,
        event:       ShowCombinedRoute,
        id:          "example::show_combined_route",
        title:       "Show Combined Route",
        description: "Color the scene for the running dimension-lock conjunction binding.",
    }

    command! {
        action:      ShowCombinedRouteBeforeOverrideAction,
        event:       ShowCombinedRouteBeforeOverride,
        id:          "example::show_combined_route_before_override",
        title:       "Show Combined Route Before Override",
        description: "Names the earlier combined binding that the later document block overrides.",
    }

    command! {
        action:      ShowCombinedRouteAfterOverrideAction,
        event:       ShowCombinedRouteAfterOverride,
        id:          "example::show_combined_route_after_override",
        title:       "Show Combined Route After Override",
        description: "Color the scene for the later combined binding that wins document precedence.",
    }

    command! {
        action:      ShowTombstonedRouteAction,
        event:       ShowTombstonedRoute,
        id:          "example::show_tombstoned_route",
        title:       "Show Tombstoned Route",
        description: "Names the global binding removed for the running dimension-lock conjunction.",
    }
}

/// The application-owned state that describes whether this example is at its menu or running.
#[derive(
    AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
)]
#[strum(serialize_all = "snake_case")]
enum ExampleApplicationState {
    #[default]
    #[strum(message = "While the example is at its main menu")]
    MainMenu,
    #[strum(message = "While the example application is running")]
    Running,
}

/// The application-owned state that describes the active editing interaction.
#[derive(
    AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
)]
#[strum(serialize_all = "snake_case")]
enum ExampleInteractionState {
    #[default]
    #[strum(message = "While no editing interaction is active")]
    Resting,
    #[strum(message = "While dimension lock is the active editing interaction")]
    DimensionLock,
}

/// The last command-owned scene effect, retained so tests can assert visible routing behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Resource)]
enum ContextSceneEffect {
    #[default]
    Initial,
    GlobalRoute,
    MainMenuRoute,
    RunningRoute,
    RestingRoute,
    DimensionLockRoute,
    CombinedRoute,
    CombinedRouteBeforeOverride,
    CombinedRouteAfterOverride,
    TombstonedRoute,
}

/// The semantic command history this example renders beside its visible scene effect.
#[derive(Clone, Debug, Default, Eq, PartialEq, Resource)]
enum ExampleInvocationSummary {
    /// No example command has changed state or scene output yet.
    #[default]
    NoInvocations,
    /// At least one example command ran through authored routing, the palette, or a permanent
    /// control.
    Invoked {
        /// The command whose observer most recently ran.
        last_command: CommandId,
        /// The number of example command observers that have run.
        count:        usize,
    },
}

impl ExampleInvocationSummary {
    fn record(&mut self, command_id: CommandId) {
        match self {
            Self::NoInvocations => {
                *self = Self::Invoked {
                    last_command: command_id,
                    count:        1,
                };
            },
            Self::Invoked {
                last_command,
                count,
            } => {
                *last_command = command_id;
                *count += 1;
            },
        }
    }
}

fn main() {
    let mut builder = sprinkle_example().with_default_asset_root();
    install_example_contexts(builder.app_mut(), contextual_keymap_plugin());

    with_context_state_controls(builder)
        .add_systems(Startup, spawn_context_status_panel)
        .add_systems(Update, refresh_context_status_panel)
        .with_title_bar(title_bar())
        .with_studio_lighting()
        .aim_at(CAMERA_FOCUS)
        .with_ground_plane()
        .size(GROUND_SIZE)
        .color(GROUND_COLOR)
        .with_cube()
        .size(CONTEXT_CUBE_SIZE)
        .color(CONTEXT_CUBE_COLOR)
        .transform(Transform::from_xyz(0.0, CONTEXT_CUBE_SIZE * 0.5, 0.0))
        .face_text(
            Face::Front,
            "CONTEXTS",
            CONTEXT_CUBE_LABEL_SIZE,
            CONTEXT_CUBE_LABEL_COLOR,
        )
        .with_orbit_cam_configured(configure_camera)
        .with_camera_home()
        .with_camera_control_panel()
        .with_stable_transparency()
        .with_command_palette_keymap(command_palette_keymap())
        .with_save_window_position()
        .with_brp_extras()
        .run();
}

/// Installs the four state controls outside authored routing through Fairy Dust's shortcut path.
fn with_context_state_controls<S>(builder: SprinkleBuilder<S>) -> SprinkleBuilder<S> {
    with_context_state_shortcuts!(
        builder;
        trigger_enter_main_menu,
        trigger_enter_running,
        trigger_enter_resting,
        trigger_enter_dimension_lock
    )
}

fn title_bar() -> TitleBar {
    TitleBar::new()
        .with_title("Keymap contexts")
        .controls([palette_control()])
}

/// The palette recovery chord remains application-owned and available outside authored routing.
const fn palette_control() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+P Palette"
    } else {
        "Ctrl+P Palette"
    }
}

fn configure_camera(orbit_cam: &mut OrbitCam) { CONTEXT_CAMERA_POSE.apply_to(orbit_cam); }

fn contextual_keymap_plugin() -> KeymapPlugin {
    keymap_plugin()
        .with_app_name(APPLICATION_NAME)
        .with_state_dimension::<ExampleApplicationState>("application")
        .with_state_dimension::<ExampleInteractionState>("interaction")
}

fn keymap_plugin() -> KeymapPlugin {
    let [first_keystroke, second_keystroke] = example_protected_keystrokes();
    KeymapPlugin::new()
        .with_defaults(KEYMAP_DEFAULTS)
        .with_protected_command_binding(
            command_palette_recovery_command_id(),
            command_palette_recovery_keystroke(),
        )
        .with_protected_keystroke(first_keystroke)
        .with_protected_keystroke(second_keystroke)
}

fn command_palette_keymap() -> CommandPaletteKeymap {
    let [first_keystroke, second_keystroke] = example_protected_keystrokes();
    CommandPaletteKeymap::new(KEYMAP_DEFAULTS)
        .for_application(APPLICATION_NAME)
        .with_protected_keystroke(first_keystroke)
        .with_protected_keystroke(second_keystroke)
}

fn example_protected_keystrokes() -> [Keystroke; 2] {
    EXAMPLE_PROTECTED_KEYSTROKES.map(parse_protected_keystroke)
}

fn parse_protected_keystroke(source: &str) -> Keystroke {
    match source.parse() {
        Ok(keystroke) => keystroke,
        Err(error) => {
            bevy::log::error!(
                "fairy_dust: invalid example protected keystroke `{source}`: {error}"
            );
            std::process::abort();
        },
    }
}

fn install_example_contexts(app: &mut App, keymap_plugin: KeymapPlugin) {
    app.init_state::<ExampleApplicationState>()
        .init_state::<ExampleInteractionState>()
        .init_resource::<ContextSceneEffect>()
        .init_resource::<ExampleInvocationSummary>()
        .add_plugins(keymap_plugin)
        .add_observer(enter_main_menu)
        .add_observer(enter_running)
        .add_observer(enter_resting)
        .add_observer(enter_dimension_lock)
        .add_observer(show_global_route)
        .add_observer(show_main_menu_route)
        .add_observer(show_running_route)
        .add_observer(show_resting_route)
        .add_observer(show_dimension_lock_route)
        .add_observer(show_combined_route)
        .add_observer(show_combined_route_before_override)
        .add_observer(show_combined_route_after_override)
        .add_observer(show_tombstoned_route);
}

/// Triggers the application-state command from the permanent `1` control.
fn trigger_enter_main_menu(mut commands: Commands) { commands.trigger(commands::EnterMainMenu); }

/// Triggers the application-state command from the permanent `2` control.
fn trigger_enter_running(mut commands: Commands) { commands.trigger(commands::EnterRunning); }

/// Triggers the interaction-state command from the permanent `3` control.
fn trigger_enter_resting(mut commands: Commands) { commands.trigger(commands::EnterResting); }

/// Triggers the interaction-state command from the permanent `4` control.
fn trigger_enter_dimension_lock(mut commands: Commands) {
    commands.trigger(commands::EnterDimensionLock);
}

fn enter_main_menu(
    _: On<commands::EnterMainMenu>,
    mut next_state: ResMut<NextState<ExampleApplicationState>>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    next_state.set(ExampleApplicationState::MainMenu);
    invocation_summary.record(CommandId::declared::<commands::EnterMainMenu>());
}

fn enter_running(
    _: On<commands::EnterRunning>,
    mut next_state: ResMut<NextState<ExampleApplicationState>>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    next_state.set(ExampleApplicationState::Running);
    invocation_summary.record(CommandId::declared::<commands::EnterRunning>());
}

fn enter_resting(
    _: On<commands::EnterResting>,
    mut next_state: ResMut<NextState<ExampleInteractionState>>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    next_state.set(ExampleInteractionState::Resting);
    invocation_summary.record(CommandId::declared::<commands::EnterResting>());
}

fn enter_dimension_lock(
    _: On<commands::EnterDimensionLock>,
    mut next_state: ResMut<NextState<ExampleInteractionState>>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    next_state.set(ExampleInteractionState::DimensionLock);
    invocation_summary.record(CommandId::declared::<commands::EnterDimensionLock>());
}

fn show_global_route(
    _: On<commands::ShowGlobalRoute>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::GlobalRoute,
    );
    invocation_summary.record(CommandId::declared::<commands::ShowGlobalRoute>());
}

fn show_main_menu_route(
    _: On<commands::ShowMainMenuRoute>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::MainMenuRoute,
    );
    invocation_summary.record(CommandId::declared::<commands::ShowMainMenuRoute>());
}

fn show_running_route(
    _: On<commands::ShowRunningRoute>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::RunningRoute,
    );
    invocation_summary.record(CommandId::declared::<commands::ShowRunningRoute>());
}

fn show_resting_route(
    _: On<commands::ShowRestingRoute>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::RestingRoute,
    );
    invocation_summary.record(CommandId::declared::<commands::ShowRestingRoute>());
}

fn show_dimension_lock_route(
    _: On<commands::ShowDimensionLockRoute>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::DimensionLockRoute,
    );
    invocation_summary.record(CommandId::declared::<commands::ShowDimensionLockRoute>());
}

fn show_combined_route(
    _: On<commands::ShowCombinedRoute>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::CombinedRoute,
    );
    invocation_summary.record(CommandId::declared::<commands::ShowCombinedRoute>());
}

fn show_combined_route_after_override(
    _: On<commands::ShowCombinedRouteAfterOverride>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::CombinedRouteAfterOverride,
    );
    invocation_summary.record(CommandId::declared::<
        commands::ShowCombinedRouteAfterOverride,
    >());
}

fn show_combined_route_before_override(
    _: On<commands::ShowCombinedRouteBeforeOverride>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::CombinedRouteBeforeOverride,
    );
    invocation_summary.record(CommandId::declared::<
        commands::ShowCombinedRouteBeforeOverride,
    >());
}

fn show_tombstoned_route(
    _: On<commands::ShowTombstonedRoute>,
    mut clear_color: ResMut<ClearColor>,
    mut scene_effect: ResMut<ContextSceneEffect>,
    mut invocation_summary: ResMut<ExampleInvocationSummary>,
) {
    apply_scene_effect(
        &mut clear_color,
        &mut scene_effect,
        ContextSceneEffect::TombstonedRoute,
    );
    invocation_summary.record(CommandId::declared::<commands::ShowTombstonedRoute>());
}

const fn apply_scene_effect(
    clear_color: &mut ClearColor,
    scene_effect: &mut ContextSceneEffect,
    next_effect: ContextSceneEffect,
) {
    *clear_color = ClearColor(match next_effect {
        ContextSceneEffect::Initial => INITIAL_EFFECT_COLOR,
        ContextSceneEffect::GlobalRoute => GLOBAL_ROUTE_COLOR,
        ContextSceneEffect::MainMenuRoute => MAIN_MENU_ROUTE_COLOR,
        ContextSceneEffect::RunningRoute => RUNNING_ROUTE_COLOR,
        ContextSceneEffect::RestingRoute => RESTING_ROUTE_COLOR,
        ContextSceneEffect::DimensionLockRoute => DIMENSION_LOCK_ROUTE_COLOR,
        ContextSceneEffect::CombinedRoute => COMBINED_ROUTE_COLOR,
        ContextSceneEffect::CombinedRouteBeforeOverride => COMBINED_ROUTE_BEFORE_OVERRIDE_COLOR,
        ContextSceneEffect::CombinedRouteAfterOverride => COMBINED_ROUTE_AFTER_OVERRIDE_COLOR,
        ContextSceneEffect::TombstonedRoute => TOMBSTONED_ROUTE_COLOR,
    });
    *scene_effect = next_effect;
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use bevy::input::keyboard::KeyCode;
    use bevy::prelude::AppTypeRegistry;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::ClearColor;
    use bevy::prelude::State;
    use bevy::state::app::StatesPlugin;
    use hana_rubric::AuthoredKeymapBindings;
    use hana_rubric::CommandId;
    use hana_rubric::CommandInvocationOutcome;
    use hana_rubric::CommandRegistry;
    use hana_rubric::EffectiveKeymapStatus;
    use hana_rubric::KeymapBindingUnavailability;
    use hana_rubric::KeymapBindings;
    use hana_rubric::KeymapPlugin;
    use hana_rubric::PaletteBinding;
    use hana_rubric::PaletteSelectionOutcome;
    use hana_rubric::query_command_palette;

    use super::CommandPaletteKeymap;
    use super::ContextSceneEffect;
    use super::ExampleApplicationState;
    use super::ExampleInteractionState;
    use super::ExampleInvocationSummary;
    use super::command_palette_keymap;
    use super::command_palette_recovery_command_id;
    use super::command_palette_recovery_keystroke;
    use super::install_example_contexts;
    use super::keymap_plugin;

    mod fairy_dust_commands {
        use bevy::prelude::Event;
        use bevy::prelude::Reflect;
        use bevy::prelude::ReflectEvent;
        use bevy_enhanced_input::prelude::InputAction;
        use hana_rubric::ReflectKeymapCommand;
        use hana_rubric::command;

        command! {
            action:      CycleCameraPresetAction,
            event:       CycleCameraPreset,
            id:          "fairy_dust::cycle_camera_preset",
            title:       "Cycle Camera Preset",
            description: "Headless registration for the Fairy Dust baseline command.",
        }

        command! {
            action:      RestartAction,
            event:       Restart,
            id:          "fairy_dust::restart",
            title:       "Restart Example",
            description: "Headless registration for the Fairy Dust baseline command.",
        }

        command! {
            action:      ShowHelpAction,
            event:       ShowHelp,
            id:          "fairy_dust::show_help",
            title:       "Show Help",
            description: "Headless registration for the Fairy Dust baseline command.",
        }

        command! {
            action:      ToggleFreeCamLookPitchAction,
            event:       ToggleFreeCamLookPitch,
            id:          "fairy_dust::toggle_free_cam_look_pitch",
            title:       "Toggle Free Camera Look Pitch",
            description: "Headless registration for the Fairy Dust baseline command.",
        }

        command! {
            action:      ToggleHomeAabbGizmoAction,
            event:       ToggleHomeAabbGizmo,
            id:          "fairy_dust::toggle_home_aabb_gizmo",
            title:       "Toggle Home Bounds Gizmo",
            description: "Headless registration for the Fairy Dust baseline command.",
        }

        command! {
            action:      ToggleScreenSpacePanelsAction,
            event:       ToggleScreenSpacePanels,
            id:          "fairy_dust::toggle_screen_space_panels",
            title:       "Toggle Screen-Space Panels",
            description: "Headless registration for the Fairy Dust baseline command.",
        }

        command! {
            action:      OpenPaletteAction,
            event:       OpenPalette,
            id:          "palette::open",
            title:       "Open Command Palette",
            description: "Headless registration for the protected recovery command.",
            capability:  Unremappable,
        }
    }

    enum ExpectedPaletteBinding {
        ApplicationRecovery,
        BoundTo(&'static str),
        KeymapUnavailable(KeymapBindingUnavailability),
        Unbound,
    }

    fn headless_app() -> App {
        let mut app = assembled_headless_app();
        app.finish();
        settle_initial_keymap_load(&mut app);
        app
    }

    // The initial keymap load runs off the main thread, so a single update does not
    // guarantee routing has bindings. The palette reports the authored default before
    // that point, which makes an unsettled app look ready.
    fn settle_initial_keymap_load(app: &mut App) {
        const MAX_UPDATES: usize = 256;

        let settled = (0..MAX_UPDATES).any(|_| {
            app.update();
            matches!(
                app.world().resource::<KeymapBindings>().authored(),
                AuthoredKeymapBindings::Loaded(_)
            )
        });

        assert!(
            settled,
            "keymap did not finish its initial load within {MAX_UPDATES} updates"
        );
    }

    // The routing tests assemble their app in-process, and they name no application on
    // purpose. Naming one points the plugin at the real user configuration directory and
    // starts a disk worker there, which rewrites the shipped files and delivers its first
    // read at a moment no test can predict. That delivery commits a fresh keymap generation,
    // and routing resets and inhibits whatever key is held at that instant, so a press the
    // test just dispatched is swallowed and the scene never changes. Nothing these tests
    // assert comes from disk: the bindings under test are the embedded defaults.
    fn assembled_headless_app() -> App { assembled_headless_app_with(headless_keymap_plugin()) }

    fn headless_keymap_plugin() -> KeymapPlugin {
        keymap_plugin()
            .with_state_dimension::<ExampleApplicationState>("application")
            .with_state_dimension::<ExampleInteractionState>("interaction")
    }

    fn invalid_default_headless_app() -> App {
        let mut app = assembled_headless_app_with(invalid_contextual_keymap_plugin());
        app.finish();
        app.update();
        app
    }

    fn assembled_headless_app_with(keymap_plugin: KeymapPlugin) -> App {
        let mut app = App::new();
        app.insert_resource(AppTypeRegistry::default());
        register_commands(&app);
        app.add_plugins(StatesPlugin)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ClearColor>();
        install_example_contexts(&mut app, keymap_plugin);
        app
    }

    fn invalid_contextual_keymap_plugin() -> KeymapPlugin {
        let [first_keystroke, second_keystroke] = super::example_protected_keystrokes();
        KeymapPlugin::new()
            .with_defaults("{ invalid default")
            .with_protected_command_binding(
                command_palette_recovery_command_id(),
                command_palette_recovery_keystroke(),
            )
            .with_protected_keystroke(first_keystroke)
            .with_protected_keystroke(second_keystroke)
            .with_state_dimension::<ExampleApplicationState>("application")
            .with_state_dimension::<ExampleInteractionState>("interaction")
    }

    fn register_commands(app: &App) {
        let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
        let mut type_registry = app_type_registry.write();
        type_registry.register::<super::commands::EnterMainMenu>();
        type_registry.register::<super::commands::EnterRunning>();
        type_registry.register::<super::commands::EnterResting>();
        type_registry.register::<super::commands::EnterDimensionLock>();
        type_registry.register::<super::commands::ShowGlobalRoute>();
        type_registry.register::<super::commands::ShowMainMenuRoute>();
        type_registry.register::<super::commands::ShowRunningRoute>();
        type_registry.register::<super::commands::ShowRestingRoute>();
        type_registry.register::<super::commands::ShowDimensionLockRoute>();
        type_registry.register::<super::commands::ShowCombinedRoute>();
        type_registry.register::<super::commands::ShowCombinedRouteBeforeOverride>();
        type_registry.register::<super::commands::ShowCombinedRouteAfterOverride>();
        type_registry.register::<super::commands::ShowTombstonedRoute>();
        type_registry.register::<fairy_dust_commands::CycleCameraPreset>();
        type_registry.register::<fairy_dust_commands::Restart>();
        type_registry.register::<fairy_dust_commands::ShowHelp>();
        type_registry.register::<fairy_dust_commands::ToggleFreeCamLookPitch>();
        type_registry.register::<fairy_dust_commands::ToggleHomeAabbGizmo>();
        type_registry.register::<fairy_dust_commands::ToggleScreenSpacePanels>();
        type_registry.register::<fairy_dust_commands::OpenPalette>();
    }

    fn command_id(command_id: &str) -> Result<CommandId, String> {
        command_id
            .try_into()
            .map_err(|error| format!("invalid command id `{command_id}`: {error}"))
    }

    fn assert_palette_binding(
        app: &App,
        command_id_text: &str,
        expected: ExpectedPaletteBinding,
    ) -> Result<(), String> {
        let command_id = command_id(command_id_text)?;
        let world = app.world();
        let query_result = query_command_palette(
            world.resource::<CommandRegistry>(),
            world.resource(),
            world.resource(),
            world.resource(),
            command_id_text,
        );
        let matching_rows = query_result
            .rows()
            .iter()
            .filter(|row| row.command().id() == &command_id)
            .collect::<Vec<_>>();
        if matching_rows.len() != 1 {
            return Err(format!(
                "palette query `{command_id_text}` returned {} exact rows",
                matching_rows.len()
            ));
        }

        let actual_binding = matching_rows[0].binding();
        let matches_expected = match expected {
            ExpectedPaletteBinding::ApplicationRecovery => {
                let recovery_keystroke = command_palette_recovery_keystroke();
                matches!(
                    actual_binding,
                    PaletteBinding::ApplicationRecovery(keystroke) if keystroke == &recovery_keystroke
                )
            },
            ExpectedPaletteBinding::BoundTo(source) => {
                let expected_sequence = source
                    .parse()
                    .map_err(|error| format!("invalid expected sequence `{source}`: {error}"))?;
                matches!(
                    actual_binding,
                    PaletteBinding::BoundTo(sequence) if sequence == &expected_sequence
                )
            },
            ExpectedPaletteBinding::KeymapUnavailable(unavailability) => matches!(
                actual_binding,
                PaletteBinding::KeymapUnavailable(actual) if actual == unavailability
            ),
            ExpectedPaletteBinding::Unbound => matches!(actual_binding, PaletteBinding::Unbound),
        };
        if matches_expected {
            Ok(())
        } else {
            Err(format!(
                "palette binding for `{command_id_text}` was {actual_binding:?}"
            ))
        }
    }

    fn assert_palette_selects(app: &App, command_id_text: &str) -> Result<(), String> {
        let command_id = command_id(command_id_text)?;
        let world = app.world();
        let query_result = query_command_palette(
            world.resource::<CommandRegistry>(),
            world.resource(),
            world.resource(),
            world.resource(),
            command_id_text,
        );
        if matches!(
            query_result.selection(),
            PaletteSelectionOutcome::Selected(command) if command.id() == &command_id
        ) {
            Ok(())
        } else {
            Err(format!(
                "palette did not select `{command_id_text}`: {:?}",
                query_result.selection()
            ))
        }
    }

    fn invoke_command(app: &mut App, command_id_text: &str) -> Result<(), String> {
        let command_id = command_id(command_id_text)?;
        let command_registry = app
            .world_mut()
            .remove_resource::<CommandRegistry>()
            .ok_or_else(|| String::from("command registry was not assembled"))?;
        let outcome = command_registry.invoke(&command_id, app.world_mut());
        app.world_mut().insert_resource(command_registry);
        if matches!(outcome, CommandInvocationOutcome::Invoked) {
            Ok(())
        } else {
            Err(format!("invoking `{command_id_text}` produced {outcome:?}"))
        }
    }

    fn advance_state_transition(app: &mut App) {
        app.update();
        app.update();
    }

    fn dispatch_key(app: &mut App, key: KeyCode) {
        press_key(app, key);
        release_key(app, key);
    }

    fn press_key(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(key);
    }

    fn release_key(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(key);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_released(key);
    }

    fn assert_scene_effect(app: &App, expected: ContextSceneEffect) {
        assert_eq!(app.world().resource::<ContextSceneEffect>(), &expected);
    }

    fn invocation_summary(app: &App) -> ExampleInvocationSummary {
        app.world().resource::<ExampleInvocationSummary>().clone()
    }

    fn rendered_status(app: &App) -> String {
        let world = app.world();
        super::context_status_sections(
            world.resource(),
            world.resource(),
            world.resource(),
            world.resource(),
            *world.resource::<ContextSceneEffect>(),
            world.resource(),
        )
        .into_iter()
        .flat_map(|section| {
            std::iter::once(section.title).chain(section.rows.into_iter().flat_map(|row| {
                std::iter::once(format!("{}: {}", row.label, row.value)).chain(row.details)
            }))
        })
        .collect::<Vec<_>>()
        .join("\n")
    }

    #[test]
    fn explicit_and_deferred_keymap_configuration_start_together() {
        let [first_keystroke, second_keystroke] = super::example_protected_keystrokes();
        assert_eq!(
            command_palette_keymap(),
            CommandPaletteKeymap::new(super::KEYMAP_DEFAULTS)
                .for_application(super::APPLICATION_NAME)
                .with_protected_keystroke(first_keystroke)
                .with_protected_keystroke(second_keystroke),
        );

        // This one names the application on both halves, because agreeing about the
        // application name is what it checks. It dispatches no keys, so the disk worker the
        // name starts cannot disturb it.
        let mut app = assembled_headless_app_with(super::contextual_keymap_plugin());
        app.add_plugins(keymap_plugin().with_app_name(super::APPLICATION_NAME));
        app.finish();
        app.update();
    }

    #[test]
    fn main_menu_and_resting_route_global_application_and_interaction_bindings()
    -> Result<(), String> {
        let mut app = headless_app();
        assert_eq!(
            app.world()
                .resource::<State<ExampleApplicationState>>()
                .get(),
            &ExampleApplicationState::MainMenu
        );
        assert_eq!(
            app.world()
                .resource::<State<ExampleInteractionState>>()
                .get(),
            &ExampleInteractionState::Resting
        );
        assert_palette_selects(&app, "example::enter_running")?;
        assert_palette_selects(&app, "example::enter_dimension_lock")?;
        assert_palette_binding(
            &app,
            "example::show_global_route",
            ExpectedPaletteBinding::BoundTo("g"),
        )?;
        assert_palette_binding(
            &app,
            "example::show_main_menu_route",
            ExpectedPaletteBinding::BoundTo("a"),
        )?;
        assert_palette_binding(
            &app,
            "example::show_running_route",
            ExpectedPaletteBinding::Unbound,
        )?;
        assert_palette_binding(
            &app,
            "example::show_resting_route",
            ExpectedPaletteBinding::BoundTo("i"),
        )?;
        assert_palette_binding(
            &app,
            "example::show_dimension_lock_route",
            ExpectedPaletteBinding::Unbound,
        )?;

        dispatch_key(&mut app, KeyCode::KeyG);
        assert_scene_effect(&app, ContextSceneEffect::GlobalRoute);
        dispatch_key(&mut app, KeyCode::KeyA);
        assert_scene_effect(&app, ContextSceneEffect::MainMenuRoute);
        dispatch_key(&mut app, KeyCode::KeyI);
        assert_scene_effect(&app, ContextSceneEffect::RestingRoute);
        Ok(())
    }

    #[test]
    fn running_changes_only_the_application_dimension_and_its_effective_binding()
    -> Result<(), String> {
        let mut app = headless_app();
        invoke_command(&mut app, "example::enter_running")?;
        advance_state_transition(&mut app);

        assert_eq!(
            app.world()
                .resource::<State<ExampleApplicationState>>()
                .get(),
            &ExampleApplicationState::Running
        );
        assert_eq!(
            app.world()
                .resource::<State<ExampleInteractionState>>()
                .get(),
            &ExampleInteractionState::Resting
        );
        assert_palette_binding(
            &app,
            "example::show_main_menu_route",
            ExpectedPaletteBinding::Unbound,
        )?;
        assert_palette_binding(
            &app,
            "example::show_running_route",
            ExpectedPaletteBinding::BoundTo("a"),
        )?;
        assert_palette_binding(
            &app,
            "example::show_resting_route",
            ExpectedPaletteBinding::BoundTo("i"),
        )?;

        dispatch_key(&mut app, KeyCode::KeyA);
        assert_scene_effect(&app, ContextSceneEffect::RunningRoute);
        Ok(())
    }

    #[test]
    fn dimension_lock_changes_only_the_interaction_dimension_and_its_effective_binding()
    -> Result<(), String> {
        let mut app = headless_app();
        invoke_command(&mut app, "example::enter_dimension_lock")?;
        advance_state_transition(&mut app);

        assert_eq!(
            app.world()
                .resource::<State<ExampleApplicationState>>()
                .get(),
            &ExampleApplicationState::MainMenu
        );
        assert_eq!(
            app.world()
                .resource::<State<ExampleInteractionState>>()
                .get(),
            &ExampleInteractionState::DimensionLock
        );
        assert_palette_binding(
            &app,
            "example::show_resting_route",
            ExpectedPaletteBinding::Unbound,
        )?;
        assert_palette_binding(
            &app,
            "example::show_dimension_lock_route",
            ExpectedPaletteBinding::BoundTo("i"),
        )?;

        dispatch_key(&mut app, KeyCode::KeyI);
        assert_scene_effect(&app, ContextSceneEffect::DimensionLockRoute);
        Ok(())
    }

    #[test]
    fn combined_context_uses_later_precedence_and_tombstones_the_global_binding()
    -> Result<(), String> {
        let mut app = headless_app();

        assert_palette_binding(
            &app,
            "example::show_tombstoned_route",
            ExpectedPaletteBinding::BoundTo("t"),
        )?;
        dispatch_key(&mut app, KeyCode::KeyT);
        assert_scene_effect(&app, ContextSceneEffect::TombstonedRoute);

        invoke_command(&mut app, "example::enter_running")?;
        advance_state_transition(&mut app);
        invoke_command(&mut app, "example::enter_dimension_lock")?;
        advance_state_transition(&mut app);

        assert_palette_binding(
            &app,
            "example::show_combined_route",
            ExpectedPaletteBinding::BoundTo("c"),
        )?;
        assert_palette_binding(
            &app,
            "example::show_combined_route_before_override",
            ExpectedPaletteBinding::Unbound,
        )?;
        assert_palette_binding(
            &app,
            "example::show_combined_route_after_override",
            ExpectedPaletteBinding::BoundTo("o"),
        )?;
        assert_palette_binding(
            &app,
            "example::show_tombstoned_route",
            ExpectedPaletteBinding::Unbound,
        )?;
        dispatch_key(&mut app, KeyCode::KeyT);
        assert_scene_effect(&app, ContextSceneEffect::TombstonedRoute);

        dispatch_key(&mut app, KeyCode::KeyC);
        assert_scene_effect(&app, ContextSceneEffect::CombinedRoute);
        dispatch_key(&mut app, KeyCode::KeyO);
        assert_scene_effect(&app, ContextSceneEffect::CombinedRouteAfterOverride);
        Ok(())
    }

    #[test]
    fn status_renders_the_control_legend_scene_effect_and_exactly_once_summary()
    -> Result<(), String> {
        let mut app = headless_app();
        invoke_command(&mut app, "example::show_global_route")?;

        let status = rendered_status(&app);
        assert!(status.contains("Permanent controls"));
        assert!(status.contains("1: main menu"));
        assert!(status.contains("2: running"));
        assert!(status.contains("3: resting"));
        assert!(status.contains("4: dimension lock"));
        assert!(status.contains(super::palette_control()));
        assert!(status.contains("scene: global route"));
        assert!(status.contains("1 invocation(s); last example::show_global_route"));
        assert_eq!(
            invocation_summary(&app),
            ExampleInvocationSummary::Invoked {
                last_command: command_id("example::show_global_route")?,
                count:        1,
            }
        );
        Ok(())
    }

    #[test]
    fn status_and_palette_share_the_canonical_binding_and_terminal_recovery() -> Result<(), String>
    {
        let app = headless_app();
        assert_palette_binding(
            &app,
            "example::show_global_route",
            ExpectedPaletteBinding::BoundTo("g"),
        )?;
        assert!(rendered_status(&app).contains("global route: g"));

        let terminal_app = invalid_default_headless_app();
        assert!(matches!(
            terminal_app.world().resource::<EffectiveKeymapStatus>(),
            EffectiveKeymapStatus::RejectedInitialDocument
        ));
        assert_palette_binding(
            &terminal_app,
            "example::show_global_route",
            ExpectedPaletteBinding::KeymapUnavailable(KeymapBindingUnavailability::InvalidDefault),
        )?;
        assert_palette_binding(
            &terminal_app,
            "palette::open",
            ExpectedPaletteBinding::ApplicationRecovery,
        )?;
        let status = rendered_status(&terminal_app);
        assert!(status.contains("global route: embedded defaults invalid"));
        assert!(status.contains(&format!("protected {}", super::palette_control())));
        Ok(())
    }

    #[test]
    fn held_key_across_a_state_transition_waits_for_release_and_fresh_press() -> Result<(), String>
    {
        let mut app = headless_app();
        press_key(&mut app, KeyCode::KeyA);
        assert_scene_effect(&app, ContextSceneEffect::MainMenuRoute);

        invoke_command(&mut app, "example::enter_running")?;
        advance_state_transition(&mut app);
        app.update();
        assert_scene_effect(&app, ContextSceneEffect::MainMenuRoute);
        assert_eq!(
            invocation_summary(&app),
            ExampleInvocationSummary::Invoked {
                last_command: command_id("example::enter_running")?,
                count:        2,
            }
        );

        release_key(&mut app, KeyCode::KeyA);
        press_key(&mut app, KeyCode::KeyA);
        assert_scene_effect(&app, ContextSceneEffect::RunningRoute);
        assert_eq!(
            invocation_summary(&app),
            ExampleInvocationSummary::Invoked {
                last_command: command_id("example::show_running_route")?,
                count:        3,
            }
        );
        release_key(&mut app, KeyCode::KeyA);
        Ok(())
    }

    #[test]
    fn protected_recovery_chord_has_the_plugin_configured_palette_binding() -> Result<(), String> {
        let app = headless_app();
        assert_eq!(
            command_palette_recovery_command_id(),
            command_id("palette::open")?
        );
        assert_palette_binding(
            &app,
            "palette::open",
            ExpectedPaletteBinding::ApplicationRecovery,
        )
    }
}
