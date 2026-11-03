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

/// The last command-owned scene effect shown in the status panel.
#[derive(Clone, Copy, Debug, Default, Resource)]
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
#[derive(Clone, Debug, Default, Resource)]
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
        .control(palette_control())
}

/// The palette recovery chord remains application-owned and available outside authored routing.
fn palette_control() -> String {
    format!("{} Palette", command_palette_recovery_keystroke().label())
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
