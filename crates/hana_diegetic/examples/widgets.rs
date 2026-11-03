//! Canonical runtime example for `hana_diegetic` widgets.
//!
//! The example keeps the complete widget interaction path runnable in one
//! Fairy Dust app.
//!
//! Controls:
//!   D - Turn Revert and Level on or off
//!   F / L / R - Frame the front / left / right cube face
//!   H - Return to the camera home pose
//!   Tab / Shift+Tab - Move between controls
//!   Enter or Space - Press the focused button
//!   Escape - Stop editing the field or cancel a button press
//!   P - Move back to the previous control
//!   T - Change the Apply tooltip delay
//!   Left / Down Arrow - Decrease the focused slider; holding repeats
//!   Right / Up Arrow - Increase the focused slider; holding repeats
//!   Drag a slider to change its value; double-click to reset it

use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Duration;
use std::time::Instant;

#[path = "widgets/startup_timing/mod.rs"]
mod startup_timing;

use bevy::anti_alias::smaa::Smaa;
use bevy::picking::hover::PickingInteraction;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Changed;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Event;
use bevy::prelude::Handle;
use bevy::prelude::Has;
use bevy::prelude::In;
use bevy::prelude::KeyCode;
use bevy::prelude::MessageReader;
use bevy::prelude::MessageWriter;
use bevy::prelude::Msaa;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::PostStartup;
use bevy::prelude::PreUpdate;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::SpawnRelated;
use bevy::prelude::SpawnWith;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy::prelude::info;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use bevy::window::WindowFocused;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DescriptionPanel;
use fairy_dust::Face;
use fairy_dust::FairyDustCube;
use fairy_dust::FrameHomeFace;
use fairy_dust::HomeFace;
use fairy_dust::TitleBar;
use fairy_dust::cube_face_panel_material;
use fairy_dust::cube_face_transform;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Appearance;
use hana_diegetic::BackgroundColor;
use hana_diegetic::Border;
use hana_diegetic::Button;
use hana_diegetic::ButtonCancelCause;
use hana_diegetic::ButtonCanceled;
use hana_diegetic::ButtonClicked;
use hana_diegetic::ButtonLatch;
use hana_diegetic::ButtonPressed;
use hana_diegetic::ButtonReleased;
use hana_diegetic::CascadeEntityCommandsExt;
use hana_diegetic::ChildDivider;
use hana_diegetic::ControlMetrics;
use hana_diegetic::ControlPalette;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::EditorStateColors;
use hana_diegetic::El;
use hana_diegetic::FacePicking;
use hana_diegetic::FitMax;
use hana_diegetic::FontId;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::ImeBuiltInFieldKind;
use hana_diegetic::ImeBuiltInFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeInputBlocker;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Lighting;
use hana_diegetic::MeshAnchorCommandsExt;
use hana_diegetic::MeshFace;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelAnchorOffset;
use hana_diegetic::PanelAttachment;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelEntityReader;
use hana_diegetic::PanelPicking;
use hana_diegetic::PanelText;
use hana_diegetic::PanelTextReader;
use hana_diegetic::PanelWidget;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::PanelWidgetWriter;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::RequestPanelFocus;
use hana_diegetic::Row;
use hana_diegetic::ShadowCasting;
use hana_diegetic::Sidedness;
use hana_diegetic::Sizing;
use hana_diegetic::Slider;
use hana_diegetic::SliderCancelCause;
use hana_diegetic::SliderCanceled;
use hana_diegetic::SliderChangeRequested;
use hana_diegetic::SliderDirection;
use hana_diegetic::SliderGrabbed;
use hana_diegetic::SliderReleased;
use hana_diegetic::SliderState;
use hana_diegetic::SwitchSide;
use hana_diegetic::SwitchState;
use hana_diegetic::SwitchToggleRequested;
use hana_diegetic::Text;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_diegetic::Tooltip;
use hana_diegetic::TooltipCommandsExt;
use hana_diegetic::TooltipFor;
use hana_diegetic::TooltipHidden;
use hana_diegetic::TooltipShown;
use hana_diegetic::WidgetDisabled;
use hana_diegetic::WidgetDisabledAppearance;
use hana_diegetic::WidgetElement;
use hana_diegetic::WidgetFocusChanged;
use hana_diegetic::WidgetFocusTarget;
use hana_diegetic::WidgetFocused;
use hana_diegetic::WidgetFocusedAppearance;
use hana_diegetic::WidgetHoveredAppearance;
use hana_diegetic::WidgetInput;
use hana_diegetic::WidgetInputMode;
use hana_diegetic::WidgetInputPlugin;
use hana_diegetic::WidgetInteractivity;
use hana_diegetic::WidgetOf;
use hana_diegetic::WidgetPressedAppearance;
use hana_lagrange::OrbitCamPreset;
use hana_rubric::Keybindings;
use hana_rubric::action;
use hana_rubric::bind_action_system;
use hana_rubric::event;
use startup_timing::StartupTimingPlugin;

// widget lab
const PANEL_BORDER_COLOR: Color = Color::srgba(0.52, 0.55, 0.59, 0.78);
const BUTTON_BORDER_DISABLED: Color = Color::srgba(0.34, 0.36, 0.40, 0.60);
const BUTTON_BORDER_FOCUSED: Color = Color::srgba(1.0, 0.86, 0.30, 0.94);
const BUTTON_FILL_DISABLED: Color = Color::srgba(0.10, 0.11, 0.13, 0.78);
const BUTTON_FILL_HOVERED: Color = Color::srgba(0.44, 0.44, 0.46, 0.98);
const BUTTON_FILL_PRESSED: Color = Color::srgba(0.27, 0.27, 0.29, 0.98);
// Both keep the white Apply label at 4.5:1 or better, as the accent behind text does.
const PRIMARY_BUTTON_HOVERED: Color = Color::srgb(0.19, 0.42, 0.95);
const PRIMARY_BUTTON_PRESSED: Color = Color::srgb(29.0 / 255.0, 78.0 / 255.0, 216.0 / 255.0);
const BUTTON_ROW_LABEL_MEASURE: &str = "Keep on top";
// Fairy Dust's 11 pt label is 14 2/3 logical px.
static CONTROL_METRICS: LazyLock<ControlMetrics<Px>> =
    LazyLock::new(|| ControlMetrics::from_label_size(Px(fairy_dust::LABEL_SIZE.0 * 4.0 / 3.0)));
const CONTROL_TEXT: Color = Color::srgb(0.92, 0.96, 1.0);
const ROW_DIVIDER_COLOR: Color = Color::srgba(0.55, 0.66, 0.76, 0.35);
const CUBE_CLEARANCE: f32 = 0.1;
const FACE_TEXT_Z_OFFSET: f32 = 0.001;
const CASCADE_FACE_LIGHT_ILLUMINANCE: f32 = 5_000.0;
const CASCADE_FACE_LIGHT_POS: Vec3 = Vec3::new(5.0, 2.5, 1.5);
const LEFT_FACE_LIGHT_POS: Vec3 = Vec3::new(-5.0, 2.5, 1.5);
const CASCADE_PANEL_TITLE: &str = "Custom look";
const FACE_PANEL_WIDTH: Px = Px(456.0);
const DESCRIPTION_LINES: [&str; 5] = [
    "Tab / Shift+Tab: move between controls",
    "Space or Enter: press the focused button",
    "Enter or double-click the field: edit it. Esc: stop",
    "Double-click a slider: reset it",
    "F / L / R: frame the front / left / right face",
];
const PANEL_BACKGROUND: Color = Color::srgba(0.02, 0.03, 0.07, 0.92);
const FLOATING_PANEL_BACKGROUND: Color = Color::srgb(0.02, 0.03, 0.07);
const PANEL_BORDER_WIDTH: Px = Px(2.0);
const SCREEN_PANEL_BORDER_WIDTH: Px = Px(5.0);
const FIELD_BORDER_WIDTH: Px = Px(1.0);
const PANEL_FACE_OFFSET: f32 = FACE_TEXT_Z_OFFSET;
const PANEL_RADIUS: Px = Px(10.0);
const EXAMPLE_TITLE: &str = "Widget Lab";
const HELP_PANEL_TITLE: &str = "Keys";
const FRONT_PANEL_TITLE: &str = "Fields & buttons";
const LEFT_PANEL_TITLE: &str = "Power & sliders";
const FACE_TITLE_COLOR: Color = Color::srgb(0.08, 0.11, 0.16);
const BUTTON_STATUS_ID: &str = "button-status";
const BUTTON_STATUS_IDLE: &str = "Not pressed";
const BUTTON_STATUS_MEASURE: &str = "Revert: press canceled";
const FOCUS_STATUS_ID: &str = "focus-status";
const FOCUS_STATUS_MEASURE: &str = "Living Room";
const FOCUS_STATUS_NONE: &str = "No control";
const PRIMARY_BUTTON_ID: &str = "primary-button";
const PRIMARY_TOOLTIP_DEFAULT_DELAY: Duration = Duration::from_millis(500);
const PRIMARY_TOOLTIP_SLOW_DELAY: Duration = Duration::from_millis(1_500);
const PRIMARY_TOOLTIP_TEXT: &str = "Reports each press in the readout.";
const SECONDARY_BUTTON_ID: &str = "secondary-button";
const TOGGLE_CONTROL: &str = "D Enable/disable";
const SCREEN_ATTACHMENT_ATTACHED_LABEL: &str = "Follows the Floating panel";
const SCREEN_ATTACHMENT_COLOR: Color = Color::srgb(1.0, 0.78, 0.32);
const SCREEN_ATTACHMENT_DETACHED_LABEL: &str = "Does not follow the Floating panel";
const SCREEN_ATTACHMENT_MAX_HEIGHT: Px = Px(64.0);
const SCREEN_ATTACHMENT_MAX_WIDTH: Px = Px(360.0);
const SCREEN_ATTACHMENT_STATUS_ID: &str = "screen-attachment-status";
// Attribute-resource defaults. Deliberately unlike the front panel's authored
// colors so a widget showing these is visibly resolving from the resource.
const ROOT_BORDER_DISABLED: Color = Color::srgba(0.30, 0.26, 0.34, 0.60);
const ROOT_BORDER_FOCUSED: Color = Color::srgba(0.42, 0.95, 0.68, 0.94);
const ROOT_FILL_DISABLED: Color = Color::srgba(0.12, 0.10, 0.14, 0.78);
const ROOT_FILL_HOVERED: Color = Color::srgba(0.36, 0.14, 0.44, 0.95);
const ROOT_FILL_PRESSED: Color = Color::srgba(0.62, 0.20, 0.44, 0.98);
const SCREEN_PANEL_MAX_HEIGHT: Px = Px(120.0);
const SCREEN_PANEL_MAX_WIDTH: Px = Px(250.0);
const SCREEN_TARGET_ID: &str = "screen-target-button";
const STATE_STATUS_ID: &str = "state-status";
const STATE_STATUS_IDLE: &str = "Apply: at rest\nRevert: at rest\nPin: at rest";
const STATE_STATUS_MEASURE: &str = "Apply: pressed\nRevert: pressed, unavailable\nPin: pressed";
const CASCADE_STATE_STATUS_IDLE: &str = "Apply: at rest\nRevert: at rest\nLevel: at rest";
const CASCADE_STATE_STATUS_MEASURE: &str =
    "Apply: pressed\nRevert: pressed, unavailable\nLevel: pressed, unavailable";
const TEXT_FIELD_ID: &str = "editable-text";
const TEXT_FIELD_INITIAL: &str = "Living Room";
const DISABLED_SWITCH_ID: &str = "unavailable-switch";
const DISABLED_SLIDER_ID: &str = "unavailable-slider";
const DISABLED_BUTTON_ID: &str = "unavailable-button";
const DISABLED_FIELD_ID: &str = "unavailable-field";
const NO_DEVICE_SWITCH_ID: &str = "no-device-switch";
const SLIDER_DISABLED_COLOR: Color = Color::srgba(0.26, 0.28, 0.31, 0.90);
const SLIDER_DISABLED_FILL: Color = Color::srgb(0.24, 0.42, 0.62);
const SLIDER_FOCUS_SIDE_CLEARANCE_MULTIPLIER: f32 = 2.0;
const SLIDER_ID: &str = "level-slider";
const SLIDER_INITIAL_VALUE: f32 = 0.5;
const SLIDER_LABEL_ID: &str = "slider-label";
const SLIDER_LABEL_IDLE: &str = "50%";
const SLIDER_LABEL_TEXT: Color = CONTROL_TEXT;
const SLIDER_RANGE_END: f32 = 1.0;
const SLIDER_RANGE_START: f32 = 0.0;
const STEPLESS_SLIDER_ID: &str = "free-slider";
const STEPLESS_SLIDER_LABEL_ID: &str = "free-slider-label";
const SWITCH_ID: &str = "power-switch";
const TOGGLE_BUTTON_ID: &str = "toggle-button";
const SLIDER_STEP: f32 = 0.05;
const STATUS_BACKGROUND: Color = Color::srgb(0.22, 0.20, 0.19);
const SCREEN_ATTACHMENT_BACKGROUND: Color = Color::srgb(0.01, 0.06, 0.08);
const STATUS_BORDER: Color = Color::srgba(0.20, 0.80, 0.68, 0.86);
const STATUS_BORDER_WIDTH: Px = Px(1.0);
const STATUS_COLOR: Color = Color::srgb(0.38, 0.94, 0.78);
const STATUS_LABEL_COLOR: Color = Color::srgba(0.68, 0.75, 0.88, 0.92);
const STATUS_LABEL_WIDTH: Px = Px(88.0);
const STATUS_RADIUS: Px = PANEL_RADIUS;
const STATUS_TEXT_SIZE: Pt = fairy_dust::LABEL_SIZE;
const CUBE_TOOLTIP_LABEL_SIZE: Mm = Mm(38.0);
const SLIDER_TOOLTIP_SOURCE_ANCHOR: Anchor = Anchor::BottomCenter;
const SLIDER_TOOLTIP_TARGET_ANCHOR: Anchor = Anchor::TopCenter;

#[derive(Clone, Copy, Default, Resource)]
enum ToggleMode {
    #[default]
    Enabled,
    Disabled,
}

impl ToggleMode {
    const fn toggled(self) -> Self {
        match self {
            Self::Enabled => Self::Disabled,
            Self::Disabled => Self::Enabled,
        }
    }

    const fn interactivity(self) -> WidgetInteractivity {
        match self {
            Self::Enabled => WidgetInteractivity::Enabled,
            Self::Disabled => WidgetInteractivity::Disabled,
        }
    }

    const fn control_activation(self) -> ControlActivation {
        match self {
            Self::Enabled => ControlActivation::Inactive,
            Self::Disabled => ControlActivation::Active,
        }
    }
}

/// Live presses observed from the public button lifecycle.
#[derive(Default, Resource)]
struct HeldButtonPresses(HashSet<Entity>);

/// Live pointer drags observed from the public slider lifecycle.
#[derive(Default, Resource)]
struct SliderDrags(HashSet<Entity>);

#[derive(Clone, Copy, Debug)]
enum WidgetLabFace {
    Front,
    Left,
    Cascade,
}

#[derive(Default, Resource)]
struct PrimaryClicks(usize);

#[derive(Resource)]
struct SliderTooltipBlueprint(Tooltip);

#[derive(Clone, Copy, Default, Resource)]
enum PrimaryTooltipTiming {
    #[default]
    Default,
    Slow,
}

impl PrimaryTooltipTiming {
    const fn toggled(self) -> Self {
        match self {
            Self::Default => Self::Slow,
            Self::Slow => Self::Default,
        }
    }

    fn tooltip(self) -> Tooltip { authored_tooltip(PRIMARY_TOOLTIP_TEXT).show_after(self.delay()) }

    const fn delay(self) -> Duration {
        match self {
            Self::Default => PRIMARY_TOOLTIP_DEFAULT_DELAY,
            Self::Slow => PRIMARY_TOOLTIP_SLOW_DELAY,
        }
    }
}

#[derive(Component)]
struct WidgetLabPanel;

#[derive(Component)]
struct LeftWidgetLabPanel;

#[derive(Component)]
struct WidgetLabFocusInitialized;

#[derive(Component)]
struct WidgetInteractionReadout;

/// Current front-face status strings, read from the live panel before a tree rebuild.
struct FaceStatusReadouts {
    focus:  String,
    button: String,
    state:  String,
}

impl Default for FaceStatusReadouts {
    fn default() -> Self {
        Self {
            focus:  FOCUS_STATUS_NONE.to_owned(),
            button: BUTTON_STATUS_IDLE.to_owned(),
            state:  STATE_STATUS_IDLE.to_owned(),
        }
    }
}

impl FaceStatusReadouts {
    fn from_panel(panel: Entity, text: &PanelTextReader) -> Self {
        let idle = Self::default();
        let current = |id| {
            text.text(panel, &PanelElementId::named(id))
                .map(str::to_owned)
        };
        Self {
            focus:  current(FOCUS_STATUS_ID).unwrap_or(idle.focus),
            button: current(BUTTON_STATUS_ID).unwrap_or(idle.button),
            state:  current(STATE_STATUS_ID).unwrap_or(idle.state),
        }
    }
}

/// The right-face panel whose widgets author no state appearance of their own,
/// so every state look they show resolves from the four attribute resources.
#[derive(Component)]
struct CascadeWidgetLabPanel;

#[derive(Component)]
struct CascadeInteractionReadout;

#[derive(Component)]
struct ScreenWidgetLabPanel;

#[derive(Component)]
struct ScreenWidgetAttachmentCard;

#[derive(Component)]
struct ScreenWidgetAttachmentInitialized;

#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
enum ScreenWidgetAttachmentState {
    Attached,
    Detached,
}

impl ScreenWidgetAttachmentState {
    const fn toggled(self) -> Self {
        match self {
            Self::Attached => Self::Detached,
            Self::Detached => Self::Attached,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Attached => SCREEN_ATTACHMENT_ATTACHED_LABEL,
            Self::Detached => SCREEN_ATTACHMENT_DETACHED_LABEL,
        }
    }

    /// The Floating panel button's label: the one action a click takes.
    const fn action_label(self) -> &'static str {
        match self {
            Self::Attached => "Detach",
            Self::Detached => "Attach",
        }
    }

    const fn action_help(self) -> &'static str {
        match self {
            Self::Attached => "Moves the panel below to the right edge.",
            Self::Detached => "Moves the panel back below this one.",
        }
    }
}

#[derive(Component)]
struct AppWidgetInputContext;

action!(
    /// App-owned action that requests previous-widget focus.
    AppFocusPrevious
);

action!(
    /// Modifier action used by the app-owned widget keybindings.
    AppWidgetShift
);

event!(
    /// App-owned event that invokes the core widget-focus request system.
    AppFocusPreviousEvent
);

struct AppOwnedWidgetInputPlugin;

impl Plugin for AppOwnedWidgetInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_input_context::<AppWidgetInputContext>()
            .add_systems(Startup, spawn_app_widget_input);
        bind_action_system!(
            app,
            AppFocusPrevious,
            AppFocusPreviousEvent,
            focus_previous_widget
        );
    }
}

fn slider_arrow_help() -> String {
    let controls = WidgetInputMode::Default.control_summary();
    format!(
        "Sliders: {} / {} increase",
        controls.step_up.join(" or ").replace("Arrow", ""),
        controls.step_right.join(" or ").replace("Arrow", ""),
    )
}

fn widget_lab_help_panel() -> DescriptionPanel {
    DescriptionPanel::new(HELP_PANEL_TITLE)
        .with_anchor(Anchor::BottomLeft)
        .with_fit_width()
        .lines(DESCRIPTION_LINES)
        .line(slider_arrow_help())
        .line("Hold an arrow key: keep changing the slider")
        .line("T: change the Apply tooltip delay")
        .line_when_title_control_hidden("H Home", "H Home")
        .line_when_title_control_hidden(TOGGLE_CONTROL, TOGGLE_CONTROL)
}

fn main() {
    let launched = Instant::now();
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    let app = fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .size(fairy_dust::EXAMPLE_CUBE_SIZE)
        .color(fairy_dust::EXAMPLE_CUBE_COLOR)
        .transform(Transform::from_translation(
            fairy_dust::example_cube_on_ground(CUBE_CLEARANCE),
        ))
        .insert(CameraHomeTarget)
        .with_orbit_cam_preset_bundle(
            |_| {},
            OrbitCamPreset::blender_like(),
            (Msaa::Off, Smaa::default()),
        )
        .with_stable_transparency()
        .with_camera_home()
        .yaw(0.0)
        .pitch(0.0)
        .margin(0.1)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(TOGGLE_CONTROL),
        )
        .wire_chip_to_state::<ToggleMode, _>(TOGGLE_CONTROL, |mode| mode.control_activation())
        .with_description_panel(widget_lab_help_panel())
        .with_camera_control_panel();
    let capabilities_built = Instant::now();
    let mut app = app
        .init_resource::<ToggleMode>()
        .init_resource::<HeldButtonPresses>()
        .init_resource::<SliderDrags>()
        .init_resource::<PrimaryClicks>()
        .init_resource::<PrimaryTooltipTiming>()
        .insert_resource(SliderTooltipBlueprint(slider_tooltip()))
        .insert_resource(WidgetHoveredAppearance::new(BackgroundColor(
            ROOT_FILL_HOVERED,
        )))
        .insert_resource(WidgetPressedAppearance::new(BackgroundColor(
            ROOT_FILL_PRESSED,
        )))
        .insert_resource(WidgetFocusedAppearance::new(
            Appearance::new()
                .border_color(ROOT_BORDER_FOCUSED)
                .border_width(CONTROL_METRICS.focus_border_width),
        ))
        .insert_resource(WidgetDisabledAppearance::new(
            Appearance::new()
                .background(ROOT_FILL_DISABLED)
                .border_color(ROOT_BORDER_DISABLED),
        ))
        .add_plugins((WidgetInputPlugin, AppOwnedWidgetInputPlugin))
        .add_plugins(StartupTimingPlugin { launched })
        .add_observer(report_button_pressed)
        .add_observer(report_button_released)
        .add_observer(report_button_clicked)
        .add_observer(toggle_screen_widget_attachment)
        .add_observer(report_button_canceled)
        .add_observer(accept_switch_toggle)
        .add_observer(report_widget_focus_changed)
        .add_observer(apply_slider_change)
        .add_observer(report_slider_grabbed)
        .add_observer(report_slider_released)
        .add_observer(report_slider_canceled)
        .add_observer(report_tooltip_shown)
        .add_observer(report_tooltip_hidden)
        .add_systems(Startup, spawn_cube_side_lights)
        .add_systems(PostStartup, spawn_widget_lab)
        .add_systems(PreUpdate, handoff_widget_lab_focus)
        .add_systems(
            Update,
            (
                initialize_screen_widget_attachment,
                initialize_widget_lab_focus,
                report_interaction_changes,
                report_presentation_states,
                reset_widget_lab_focus_on_window_blur,
                show_focused_action_label,
            ),
        )
        .with_shortcut(KeyCode::KeyD, toggle_disabled_widgets)
        .with_shortcut(KeyCode::KeyF, frame_front_face)
        .with_shortcut(KeyCode::KeyL, frame_left_face)
        .with_shortcut(KeyCode::KeyR, frame_right_face)
        .with_shortcut(KeyCode::KeyT, replace_primary_tooltip)
        .build();
    startup_timing::record_app_built(&app, capabilities_built);
    startup_timing::apply_probe_window_size(&mut app);
    app.run();
}

fn frame_front_face(mut commands: Commands) { commands.trigger(FrameHomeFace(HomeFace::Front)); }

fn frame_left_face(mut commands: Commands) { commands.trigger(FrameHomeFace(HomeFace::Left)); }

fn frame_right_face(mut commands: Commands) { commands.trigger(FrameHomeFace(HomeFace::Right)); }

fn report_button_pressed(
    event: On<ButtonPressed>,
    mut presses: ResMut<HeldButtonPresses>,
    front_panels: Query<Entity, With<WidgetLabPanel>>,
    owners: Query<&WidgetOf>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    presses.0.insert(event.entity);
    info!("widgets: {} pressed by {:?}", event.id, event.source);
    set_front_button_status(
        event.entity,
        &event.id,
        "pressed",
        &front_panels,
        &owners,
        &readouts,
        &mut panel_text,
    );
}

fn report_button_released(
    event: On<ButtonReleased>,
    mut presses: ResMut<HeldButtonPresses>,
    front_panels: Query<Entity, With<WidgetLabPanel>>,
    owners: Query<&WidgetOf>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    presses.0.remove(&event.entity);
    info!("widgets: {} released by {:?}", event.id, event.source);
    set_front_button_status(
        event.entity,
        &event.id,
        "released",
        &front_panels,
        &owners,
        &readouts,
        &mut panel_text,
    );
}

fn report_button_clicked(
    event: On<ButtonClicked>,
    front_panels: Query<Entity, With<WidgetLabPanel>>,
    owners: Query<&WidgetOf>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    info!("widgets: {} clicked by {:?}", event.id, event.source);
    set_front_button_status(
        event.entity,
        &event.id,
        "clicked",
        &front_panels,
        &owners,
        &readouts,
        &mut panel_text,
    );
}

fn report_button_canceled(
    event: On<ButtonCanceled>,
    mut presses: ResMut<HeldButtonPresses>,
    front_panels: Query<Entity, With<WidgetLabPanel>>,
    owners: Query<&WidgetOf>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    presses.0.remove(&event.entity);
    info!(
        "widgets: {} canceled for {:?} by {:?}",
        event.id, event.cause, event.source
    );
    let result = match event.cause {
        ButtonCancelCause::FocusLost => "focus moved",
        ButtonCancelCause::InputInterrupted => "interrupted",
        ButtonCancelCause::Disabled
        | ButtonCancelCause::WidgetRemoved
        | ButtonCancelCause::WidgetKindChanged => "unavailable",
        ButtonCancelCause::Explicit
        | ButtonCancelCause::PointerCanceled
        | ButtonCancelCause::PointerRemoved
        | ButtonCancelCause::CaptureLost => "canceled",
    };
    set_front_button_status(
        event.entity,
        &event.id,
        result,
        &front_panels,
        &owners,
        &readouts,
        &mut panel_text,
    );
}

fn set_front_button_status(
    entity: Entity,
    id: &PanelElementId,
    result: &str,
    front_panels: &Query<Entity, With<WidgetLabPanel>>,
    owners: &Query<&WidgetOf>,
    readouts: &Query<Entity, With<WidgetInteractionReadout>>,
    panel_text: &mut PanelText,
) {
    let (Ok(front_panel), Ok(owner), Ok(readout)) =
        (front_panels.single(), owners.get(entity), readouts.single())
    else {
        return;
    };
    if owner.panel() != front_panel {
        return;
    }
    let name = match id.as_str() {
        Some(PRIMARY_BUTTON_ID) => "Apply",
        Some(SECONDARY_BUTTON_ID) => "Revert",
        Some(TOGGLE_BUTTON_ID) => "Pin",
        _ => return,
    };
    panel_text.set_text(
        readout,
        &PanelElementId::named(BUTTON_STATUS_ID),
        format!("{name}: {result}"),
    );
}

fn control_label(id: &PanelElementId) -> &'static str {
    match id.as_str() {
        Some(PRIMARY_BUTTON_ID) => "Apply",
        Some(SECONDARY_BUTTON_ID) => "Revert",
        Some(TOGGLE_BUTTON_ID) => "Pin",
        Some(SWITCH_ID) => "Power",
        Some(SLIDER_ID) => "Level",
        Some(STEPLESS_SLIDER_ID) => "Smooth",
        Some(SCREEN_TARGET_ID) => "Attachment",
        Some(TEXT_FIELD_ID) => "Name",
        _ => "Control",
    }
}

fn accept_switch_toggle(request: On<SwitchToggleRequested>, mut switches: Query<&mut SwitchState>) {
    if let Ok(mut switch) = switches.get_mut(request.entity) {
        switch.set_side(request.requested);
    }
}

/// Typed `.on_click` callback for the primary button.
///
/// Installed through `Button::new().on_click(...)` at authoring time; reify
/// registers it once as a tracked system and the plugin's single
/// global `ButtonClicked` observer dispatches it with each completed click.
fn count_primary_click(click: In<ButtonClicked>, mut clicks: ResMut<PrimaryClicks>) {
    clicks.0 += 1;
    info!(
        "widgets: on_click callback ran for {} ({:?}), {} total",
        click.id, click.source, clicks.0
    );
}

fn spawn_app_widget_input(mut commands: Commands) {
    commands.spawn((
        AppWidgetInputContext,
        Actions::<AppWidgetInputContext>::spawn(SpawnWith(spawn_app_widget_actions)),
    ));
}

fn spawn_app_widget_actions(spawner: &mut ActionSpawner<AppWidgetInputContext>) {
    let keybindings = Keybindings::new::<AppWidgetShift>(spawner, ActionSettings::default());
    keybindings.spawn_key::<AppFocusPrevious>(spawner, KeyCode::KeyP);
}

/// Applies each slider proposal explicitly — the app stays authoritative over
/// the applied value — then mirrors the accepted value into the slider label
/// on the owning face.
fn apply_slider_change(
    change: On<SliderChangeRequested>,
    mut sliders: Query<&mut SliderState>,
    widgets: Query<(&PanelWidget, &WidgetOf)>,
    mut panel_text: PanelText,
) {
    let Ok(mut state) = sliders.get_mut(change.event_target()) else {
        return;
    };
    if state
        .bypass_change_detection()
        .set_proposed_value(change.value)
    {
        state.set_changed();
    } else {
        return;
    }
    let range = state.range();
    let span = range.end() - range.start();
    let percent = (state.value() - range.start()) / span * 100.0;
    let Ok((widget, owner)) = widgets.get(change.event_target()) else {
        return;
    };
    let is_stepless = *widget.id() == PanelElementId::named(STEPLESS_SLIDER_ID);
    let is_stepped = *widget.id() == PanelElementId::named(SLIDER_ID);
    if !is_stepped && !is_stepless {
        return;
    }
    let label_id = if is_stepless {
        STEPLESS_SLIDER_LABEL_ID
    } else {
        SLIDER_LABEL_ID
    };
    if !panel_text.set_text(
        owner.panel(),
        &PanelElementId::named(label_id),
        format!("{percent:.0}%"),
    ) {
        warn!("widgets: slider label has not been reified");
    }
}

/// Records the grab and logs the pointer grab that begins a drag; the drag's
/// proposals move the value through [`apply_slider_change`].
fn report_slider_grabbed(event: On<SliderGrabbed>, mut drags: ResMut<SliderDrags>) {
    drags.0.insert(event.entity);
    info!("widgets: {} grabbed by {:?}", event.id, event.pointer_id);
}

/// Clears the grab record and logs the valid pointer release that completes a
/// drag.
fn report_slider_released(event: On<SliderReleased>, mut drags: ResMut<SliderDrags>) {
    drags.0.remove(&event.entity);
    info!("widgets: {} released by {:?}", event.id, event.pointer_id);
}

/// Clears the grab record and logs a drag that ended without a valid release —
/// projection loss, disable, an explicit cancel, or teardown.
fn report_slider_canceled(event: On<SliderCanceled>, mut drags: ResMut<SliderDrags>) {
    drags.0.remove(&event.entity);
    let cause = match event.cause {
        SliderCancelCause::PointerCanceled => "pointer canceled",
        SliderCancelCause::PointerRemoved => "pointer removed",
        SliderCancelCause::CaptureLost => "capture lost",
        SliderCancelCause::Disabled => "disabled",
        SliderCancelCause::AuthoringLocked => "authoring locked",
        SliderCancelCause::ProjectionUnavailable => "couldn't track the pointer",
        SliderCancelCause::WidgetRemoved => "widget removed",
        SliderCancelCause::WidgetKindChanged => "kind changed",
        SliderCancelCause::Explicit => "explicit",
    };
    info!(
        "widgets: {} canceled by {:?} ({cause})",
        event.id, event.pointer_id
    );
}

fn report_tooltip_shown(
    event: On<TooltipShown>,
    targets: Query<&TooltipFor>,
    widgets: Query<&PanelWidget>,
) {
    report_tooltip_visibility(event.entity, "shown", &targets, &widgets);
}

fn report_tooltip_hidden(
    event: On<TooltipHidden>,
    targets: Query<&TooltipFor>,
    widgets: Query<&PanelWidget>,
) {
    report_tooltip_visibility(event.entity, "hidden", &targets, &widgets);
}

fn report_tooltip_visibility(
    tooltip: Entity,
    visibility: &str,
    targets: &Query<&TooltipFor>,
    widgets: &Query<&PanelWidget>,
) {
    let Ok(target) = targets.get(tooltip) else {
        return;
    };
    let label = widgets
        .get(target.target())
        .map_or("Widget cube", |widget| control_label(widget.id()));
    info!("widgets: tooltip {tooltip:?} {visibility} for {label}");
}

fn report_widget_focus_changed(
    change: On<WidgetFocusChanged>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    widgets: Query<&PanelWidget>,
    attachments: Query<&ScreenWidgetAttachmentState, With<ScreenWidgetAttachmentCard>>,
    mut panel_text: PanelText,
) {
    info!(
        "widgets: window {:?} focus changed from {:?} to {:?} ({:?})",
        change.window, change.previous, change.current, change.cause
    );

    let focus_status = match change.current {
        WidgetFocusTarget::Widget(entity) => {
            let Ok(widget) = widgets.get(entity) else {
                return;
            };
            match (widget.id().as_str(), attachments.single()) {
                (Some(SCREEN_TARGET_ID), Ok(state)) => state.action_label(),
                _ => control_label(widget.id()),
            }
        },
        WidgetFocusTarget::Unfocused => FOCUS_STATUS_NONE,
    };
    let Ok(readout) = readouts.single() else {
        return;
    };
    if !panel_text.set_text(
        readout,
        &PanelElementId::named(FOCUS_STATUS_ID),
        focus_status,
    ) {
        warn!("widgets: focus status has not been reified");
    }
}

fn focus_previous_widget(
    window: Single<Entity, With<PrimaryWindow>>,
    mut widget_input: MessageWriter<WidgetInput>,
) {
    widget_input.write(WidgetInput::FocusPrevious { window: *window });
}

fn initialize_screen_widget_attachment(
    mut commands: Commands,
    panel: Single<Entity, With<ScreenWidgetLabPanel>>,
    card: Single<
        Entity,
        (
            With<ScreenWidgetAttachmentCard>,
            Without<ScreenWidgetAttachmentInitialized>,
        ),
    >,
    panel_entities: PanelEntityReader,
) {
    let Some(owner) = panel_entities.screen(*panel) else {
        return;
    };
    let Some(source) = panel_entities.screen(*card) else {
        return;
    };
    commands.attach_to_panel(source, owner, screen_widget_attachment());
    commands.entity(*card).insert((
        ScreenWidgetAttachmentInitialized,
        ScreenWidgetAttachmentState::Attached,
    ));
}

fn toggle_screen_widget_attachment(
    event: On<ButtonClicked>,
    mut commands: Commands,
    panel: Single<Entity, With<ScreenWidgetLabPanel>>,
    mut cards: Query<(Entity, &mut ScreenWidgetAttachmentState), With<ScreenWidgetAttachmentCard>>,
    panel_entities: PanelEntityReader,
) {
    if event.id.as_str() != Some(SCREEN_TARGET_ID) {
        return;
    }
    let Ok((card, mut state)) = cards.single_mut() else {
        return;
    };
    let Some(owner) = panel_entities.screen(*panel) else {
        return;
    };
    let Some(source) = panel_entities.screen(card) else {
        return;
    };
    let next = state.toggled();
    if let Err(error) = commands.set_tree(*panel, screen_widget_tree(next)) {
        warn!("widgets: failed to update the Floating panel button: {error}");
        return;
    }
    if let Err(error) = commands.set_tree(card, screen_attachment_status_tree(next)) {
        warn!("widgets: failed to update screen attachment status: {error}");
        return;
    }
    match next {
        ScreenWidgetAttachmentState::Attached => {
            commands.attach_to_panel(source, owner, screen_widget_attachment());
        },
        ScreenWidgetAttachmentState::Detached => commands.detach(source),
    }
    *state = next;
}

/// Keeps the focus readout on the Floating panel button's label after a click relabels it.
fn show_focused_action_label(
    attachments: Query<
        &ScreenWidgetAttachmentState,
        (
            With<ScreenWidgetAttachmentCard>,
            Changed<ScreenWidgetAttachmentState>,
        ),
    >,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    let Ok(state) = attachments.single() else {
        return;
    };
    let Ok(readout) = readouts.single() else {
        return;
    };
    let focus_id = PanelElementId::named(FOCUS_STATUS_ID);
    if panel_text.text(readout, &focus_id) != Some(state.toggled().action_label()) {
        return;
    }
    if !panel_text.set_text(readout, &focus_id, state.action_label()) {
        warn!("widgets: focus status has not been reified");
    }
}

fn screen_widget_attachment() -> PanelAttachment {
    PanelAttachment::new(Anchor::TopRight, Anchor::BottomRight).with_offset(PanelAnchorOffset::new(
        Px(0.0),
        Px(fairy_dust::SCREEN_PANEL_GAP),
    ))
}

fn initialize_widget_lab_focus(
    mut commands: Commands,
    window: Single<(Entity, &Window), With<PrimaryWindow>>,
    panel: Single<Entity, (With<WidgetLabPanel>, Without<WidgetLabFocusInitialized>)>,
) {
    let (window_entity, window) = *window;
    if !window.focused {
        return;
    }
    commands.trigger(RequestPanelFocus {
        window: window_entity,
        panel:  *panel,
    });
    commands.entity(*panel).insert(WidgetLabFocusInitialized);
}

fn reset_widget_lab_focus_on_window_blur(
    mut focus_events: MessageReader<WindowFocused>,
    windows: Query<&Window, (With<PrimaryWindow>, Changed<Window>)>,
    panels: Query<Entity, (With<WidgetLabPanel>, With<WidgetLabFocusInitialized>)>,
    mut commands: Commands,
) {
    if focus_events.read().any(|event| !event.focused)
        || windows.iter().any(|window| !window.focused)
    {
        for panel in panels.iter() {
            commands.entity(panel).remove::<WidgetLabFocusInitialized>();
        }
    }
}

/// Tab crosses the panel boundary in the same order the faces are presented.
fn handoff_widget_lab_focus(
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<Entity, With<PrimaryWindow>>,
    blocker: Res<ImeInputBlocker>,
    front_panel: Single<Entity, With<WidgetLabPanel>>,
    left_panel: Single<Entity, With<LeftWidgetLabPanel>>,
    focused: Query<(&PanelWidget, &WidgetOf), With<WidgetFocused>>,
    mut commands: Commands,
) {
    if !keys.just_pressed(KeyCode::Tab) || blocker.blocks_window(*window) {
        return;
    }
    let backwards = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let destination = focused.iter().find_map(|(widget, owner)| {
        let id = widget.id();
        match (owner.panel(), backwards) {
            (panel, false)
                if panel == *front_panel && *id == PanelElementId::named(TOGGLE_BUTTON_ID) =>
            {
                Some(*left_panel)
            },
            (panel, false)
                if panel == *left_panel && *id == PanelElementId::named(NO_DEVICE_SWITCH_ID) =>
            {
                Some(*front_panel)
            },
            (panel, true)
                if panel == *front_panel && *id == PanelElementId::named(TEXT_FIELD_ID) =>
            {
                Some(*left_panel)
            },
            (panel, true) if panel == *left_panel && *id == PanelElementId::named(SWITCH_ID) => {
                Some(*front_panel)
            },
            _ => None,
        }
    });
    if let Some(panel) = destination {
        commands.trigger(RequestPanelFocus {
            window: *window,
            panel,
        });
    }
}

fn spawn_widget_lab(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cube: Single<Entity, With<FairyDustCube>>,
    primary_tooltip_timing: Res<PrimaryTooltipTiming>,
    slider_tooltip: Res<SliderTooltipBlueprint>,
    text_measurer: Res<DiegeticTextMeasurer>,
) {
    let cube_target = commands.mesh_anchor_target(*cube, MeshFace::PositiveZ);
    commands.spawn_tooltip(cube_target, cube_tooltip());
    let material = materials.add(cube_face_panel_material());
    let (face_size, face_layout) = fitted_face_tree(
        WidgetLabFace::Front,
        slider_declaration(),
        primary_tooltip_timing.tooltip(),
        slider_tooltip.0.clone(),
        &FaceStatusReadouts::default(),
        &text_measurer,
    );
    let panel = DiegeticPanel::world()
        .size(face_size, face_size)
        .world_width(fairy_dust::EXAMPLE_CUBE_SIZE)
        .anchor(Anchor::TopLeft)
        .picking(PanelPicking {
            front: FacePicking::Interactive,
            back:  FacePicking::PanelOnly,
        })
        .shadow_casting(ShadowCasting::Off)
        .material(material.clone())
        .text_material(material.clone())
        .widget_hovered_appearance(BackgroundColor(BUTTON_FILL_HOVERED))
        .widget_pressed_appearance(BackgroundColor(BUTTON_FILL_PRESSED))
        .widget_focused_appearance(
            Appearance::new()
                .border_color(BUTTON_BORDER_FOCUSED)
                .border_width(CONTROL_METRICS.focus_border_width),
        )
        .widget_disabled_appearance(
            Appearance::new()
                .background(BUTTON_FILL_DISABLED)
                .border_color(BUTTON_BORDER_DISABLED),
        )
        .with_tree(face_layout)
        .build();
    match panel {
        Ok(panel) => {
            commands.entity(*cube).with_children(|cube| {
                cube.spawn((
                    Name::new("Widget lab face panel"),
                    WidgetLabPanel,
                    WidgetInteractionReadout,
                    panel,
                    widget_panel_transform(),
                ))
                .override_lighting(Lighting::Unlit)
                .override_sidedness(Sidedness::FrontOnly);
            });
        },
        Err(error) => error!("widgets: failed to build widget panel: {error}"),
    }

    spawn_left_widget_lab(&mut commands, *cube, material.clone(), &text_measurer);
    spawn_cascade_widget_lab(
        &mut commands,
        *cube,
        material.clone(),
        primary_tooltip_timing.tooltip(),
        slider_tooltip.0.clone(),
        &text_measurer,
    );
    spawn_screen_widget_lab(&mut commands, material);
}

/// Fills both side faces without adding shadows to their controls.
fn spawn_cube_side_lights(mut commands: Commands) {
    for (name, position) in [
        ("Power face light", LEFT_FACE_LIGHT_POS),
        ("Cascade face light", CASCADE_FACE_LIGHT_POS),
    ] {
        commands.spawn((
            Name::new(name),
            DirectionalLight {
                illuminance: CASCADE_FACE_LIGHT_ILLUMINANCE,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(position)
                .looking_at(fairy_dust::example_cube_on_ground(CUBE_CLEARANCE), Vec3::Y),
        ));
    }
}

fn spawn_left_widget_lab(
    commands: &mut Commands,
    cube: Entity,
    material: Handle<StandardMaterial>,
    text_measurer: &DiegeticTextMeasurer,
) {
    let (face_size, face_layout) = fitted_face_tree(
        WidgetLabFace::Left,
        slider_declaration(),
        PrimaryTooltipTiming::Default.tooltip(),
        slider_tooltip(),
        &FaceStatusReadouts::default(),
        text_measurer,
    );
    let panel = DiegeticPanel::world()
        .size(face_size, face_size)
        .world_width(fairy_dust::EXAMPLE_CUBE_SIZE)
        .anchor(Anchor::TopLeft)
        .picking(PanelPicking {
            front: FacePicking::Interactive,
            back:  FacePicking::PanelOnly,
        })
        .shadow_casting(ShadowCasting::Off)
        .material(material.clone())
        .text_material(material)
        .widget_hovered_appearance(BackgroundColor(BUTTON_FILL_HOVERED))
        .widget_pressed_appearance(BackgroundColor(BUTTON_FILL_PRESSED))
        .widget_focused_appearance(
            Appearance::new()
                .border_color(BUTTON_BORDER_FOCUSED)
                .border_width(CONTROL_METRICS.focus_border_width),
        )
        .widget_disabled_appearance(
            Appearance::new()
                .background(BUTTON_FILL_DISABLED)
                .border_color(BUTTON_BORDER_DISABLED),
        )
        .with_tree(face_layout)
        .build();
    match panel {
        Ok(panel) => {
            commands.entity(cube).with_children(|cube| {
                cube.spawn((
                    Name::new("Power and sliders face panel"),
                    LeftWidgetLabPanel,
                    panel,
                    left_panel_transform(),
                ))
                .override_lighting(Lighting::Unlit)
                .override_sidedness(Sidedness::FrontOnly);
            });
        },
        Err(error) => error!("widgets: failed to build power and sliders panel: {error}"),
    }
}

/// Spawns the right face's widget lab. It authors no state appearance, so hover, press,
/// focus, and disabled looks all resolve from the four attribute resources
/// inserted in `main`. The front face overrides those same resources on its
/// panel, which is what makes the two faces differ on screen.
fn spawn_cascade_widget_lab(
    commands: &mut Commands,
    cube: Entity,
    material: Handle<StandardMaterial>,
    primary_tooltip: Tooltip,
    slider_tooltip: Tooltip,
    text_measurer: &DiegeticTextMeasurer,
) {
    let (face_size, face_layout) = fitted_face_tree(
        WidgetLabFace::Cascade,
        slider_declaration(),
        primary_tooltip,
        slider_tooltip,
        &FaceStatusReadouts::default(),
        text_measurer,
    );
    let panel = DiegeticPanel::world()
        .size(face_size, face_size)
        .world_width(fairy_dust::EXAMPLE_CUBE_SIZE)
        .anchor(Anchor::TopLeft)
        .picking(PanelPicking {
            front: FacePicking::Interactive,
            back:  FacePicking::PanelOnly,
        })
        .shadow_casting(ShadowCasting::Off)
        .material(material.clone())
        .text_material(material)
        .with_tree(face_layout)
        .build();
    match panel {
        Ok(panel) => {
            commands.entity(cube).with_children(|cube| {
                cube.spawn((
                    Name::new("Cascade lab face panel"),
                    CascadeWidgetLabPanel,
                    CascadeInteractionReadout,
                    panel,
                    cascade_panel_transform(),
                ))
                .override_lighting(Lighting::Unlit)
                .override_sidedness(Sidedness::FrontOnly);
            });
        },
        Err(error) => error!("widgets: failed to build cascade panel: {error}"),
    }
}

fn spawn_screen_widget_lab(commands: &mut Commands, material: Handle<StandardMaterial>) {
    let screen_panel = DiegeticPanel::screen()
        .size(
            FitMax(SCREEN_PANEL_MAX_WIDTH.into()),
            FitMax(SCREEN_PANEL_MAX_HEIGHT.into()),
        )
        .anchor(Anchor::TopRight)
        .shadow_casting(ShadowCasting::Off)
        .material(material.clone())
        .text_material(material.clone())
        .widget_hovered_appearance(BackgroundColor(BUTTON_FILL_HOVERED))
        .widget_pressed_appearance(BackgroundColor(BUTTON_FILL_PRESSED))
        .widget_focused_appearance(
            Appearance::new()
                .border_color(BUTTON_BORDER_FOCUSED)
                .border_width(CONTROL_METRICS.focus_border_width),
        )
        .widget_disabled_appearance(
            Appearance::new()
                .background(BUTTON_FILL_DISABLED)
                .border_color(BUTTON_BORDER_DISABLED),
        )
        .with_tree(screen_widget_tree(ScreenWidgetAttachmentState::Attached))
        .build();
    let attachment_card = DiegeticPanel::screen()
        .size(
            FitMax(SCREEN_ATTACHMENT_MAX_WIDTH.into()),
            FitMax(SCREEN_ATTACHMENT_MAX_HEIGHT.into()),
        )
        .anchor(Anchor::CenterRight)
        .shadow_casting(ShadowCasting::Off)
        .material(material.clone())
        .text_material(material)
        .with_tree(screen_attachment_status_tree(
            ScreenWidgetAttachmentState::Attached,
        ))
        .build();

    match (screen_panel, attachment_card) {
        (Ok(screen_panel), Ok(attachment_card)) => {
            commands.spawn((
                Name::new("Screen widget target panel"),
                ScreenWidgetLabPanel,
                screen_panel,
                PanelPicking::INTERACTIVE,
            ));
            commands.spawn((
                Name::new("Screen widget attachment card"),
                ScreenWidgetAttachmentCard,
                attachment_card,
            ));
        },
        (Err(error), _) => error!("widgets: failed to build screen widget panel: {error}"),
        (_, Err(error)) => error!("widgets: failed to build screen attachment card: {error}"),
    }
}

fn screen_widget_tree(state: ScreenWidgetAttachmentState) -> LayoutTree {
    let (top_trim, _) = control_text_edge_trim();
    let inner_padding = Px(CONTROL_METRICS.spacing.panel_padding.0 - SCREEN_PANEL_BORDER_WIDTH.0);
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::new(
                inner_padding,
                inner_padding,
                inner_padding,
                inner_padding,
            ))
            .background(FLOATING_PANEL_BACKGROUND)
            .border(Border::all(
                SCREEN_PANEL_BORDER_WIDTH,
                SCREEN_ATTACHMENT_COLOR,
            ))
            .corner_radius(CornerRadius::all(PANEL_RADIUS)),
    );
    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(CONTROL_METRICS.spacing.row_gap)
            .tooltip_boundary(),
        |builder| {
            builder.text(
                Text::new(
                    "Floating panel",
                    TextStyle::new(fairy_dust::LABEL_SIZE).with_color(SCREEN_ATTACHMENT_COLOR),
                )
                .trim_text_box(top_trim, Px(0.0)),
            );
            add_button(
                builder,
                SCREEN_TARGET_ID,
                state.action_label(),
                Button::new(),
                |element| element.tooltip(screen_attach_tooltip(state)),
            );
        },
    );
    builder.build()
}

fn screen_attachment_status_tree(state: ScreenWidgetAttachmentState) -> LayoutTree {
    let (top_trim, bottom_trim) = control_text_edge_trim();
    let inner_padding = Px(CONTROL_METRICS.spacing.panel_padding.0 - SCREEN_PANEL_BORDER_WIDTH.0);
    let mut builder = LayoutBuilder::with_root(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::new(
                inner_padding,
                inner_padding,
                inner_padding,
                inner_padding,
            ))
            .alignment(AlignX::Center, AlignY::Center)
            .background(SCREEN_ATTACHMENT_BACKGROUND)
            .border(Border::all(
                SCREEN_PANEL_BORDER_WIDTH,
                SCREEN_ATTACHMENT_COLOR,
            ))
            .corner_radius(CornerRadius::all(STATUS_RADIUS)),
    );
    builder.text(
        Text::new(
            state.label(),
            TextStyle::new(fairy_dust::LABEL_SIZE).with_color(SCREEN_ATTACHMENT_COLOR),
        )
        .id(SCREEN_ATTACHMENT_STATUS_ID)
        .trim_text_box(top_trim, bottom_trim),
    );
    builder.build()
}

/// Builds the slider away from the layout chain so the tree below can show the
/// pre-built [`El::widget`] path next to the inline `El::button` one.
fn slider_declaration() -> Slider {
    Slider::new(SLIDER_RANGE_START..=SLIDER_RANGE_END)
        .default_value(SLIDER_INITIAL_VALUE)
        .step(SLIDER_STEP)
        .direction(SliderDirection::LeftToRight)
}

fn fitted_face_tree(
    face: WidgetLabFace,
    slider: Slider,
    primary_tooltip: Tooltip,
    slider_tooltip: Tooltip,
    readouts: &FaceStatusReadouts,
    text_measurer: &DiegeticTextMeasurer,
) -> (Px, LayoutTree) {
    let mut side = FACE_PANEL_WIDTH;
    let widest_face_width = 2.0f32.mul_add(
        CONTROL_METRICS.spacing.card_padding.0 + CONTROL_METRICS.spacing.panel_padding.0,
        minimum_face_content_width(text_measurer),
    );
    for _ in 0..6 {
        let mut tallest = 0.0_f32;
        for candidate in [
            WidgetLabFace::Front,
            WidgetLabFace::Left,
            WidgetLabFace::Cascade,
        ] {
            let tree = face_tree_at_width(
                candidate,
                side,
                slider.clone(),
                primary_tooltip.clone(),
                slider_tooltip.clone(),
                readouts,
            );
            tallest = tallest.max(tree.measured_height_at_width(
                side.0,
                4.0 / 3.0,
                text_measurer.measure_fn.clone(),
            ));
        }
        let needed_side = tallest.max(widest_face_width);
        if (needed_side - side.0).abs() < 0.1 {
            break;
        }
        side = Px(needed_side);
    }
    (
        side,
        face_tree_at_width(
            face,
            side,
            slider,
            primary_tooltip,
            slider_tooltip,
            readouts,
        ),
    )
}

fn minimum_face_content_width(text_measurer: &DiegeticTextMeasurer) -> f32 {
    let label_style = TextStyle::new(fairy_dust::LABEL_SIZE).as_measure();
    let text_width = |text: &str| {
        (text_measurer.measure_fn)(text, &label_style).width
            / hana_diegetic::Unit::Pixels.to_points()
    };
    let m = &*CONTROL_METRICS;
    let gap = m.row.gap.0;
    let slider =
        2.0f32.mul_add(gap, m.row.label_width.0) + m.slider.width.0 + m.slider.value_width.0;
    let field = m.row.label_width.0 + gap + m.field.width.0;
    let switch = 2.0f32.mul_add(gap, m.row.label_width.0) + m.slider.value_width.0;
    let button = 2.0f32.mul_add(
        m.button.side_padding.0,
        text_width(BUTTON_ROW_LABEL_MEASURE) + gap + text_width("Revert"),
    );
    let no_device = text_width("No device (still switchable)") + gap + m.switch.track_width.0;
    [slider, field, switch, button, no_device]
        .into_iter()
        .fold(0.0, f32::max)
}

fn face_tree_at_width(
    face: WidgetLabFace,
    side: Px,
    slider: Slider,
    primary_tooltip: Tooltip,
    slider_tooltip: Tooltip,
    readouts: &FaceStatusReadouts,
) -> LayoutTree {
    debug_assert!(
        (CONTROL_METRICS.spacing.title_gap.0 - CONTROL_METRICS.spacing.card_gap.0).abs() < 0.001
    );
    let group_gap = CONTROL_METRICS.spacing.title_gap;
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(side))
            .height(Sizing::fixed(side))
            .padding(Padding::all(CONTROL_METRICS.spacing.panel_padding))
            .gap(group_gap)
            .alignment(AlignX::Left, AlignY::Top),
    );
    let title = match face {
        WidgetLabFace::Front => FRONT_PANEL_TITLE,
        WidgetLabFace::Left => LEFT_PANEL_TITLE,
        WidgetLabFace::Cascade => CASCADE_PANEL_TITLE,
    };
    let (top_trim, bottom_trim) = title_text_edge_trim();
    builder.text(
        Text::new(
            title,
            TextStyle::new(fairy_dust::TITLE_SIZE)
                .with_color(FACE_TITLE_COLOR)
                .with_shadow_mode(GlyphShadowMode::None),
        )
        .trim_text_box(top_trim, bottom_trim),
    );
    builder.with(
        El::column()
            .width(Sizing::fixed(face_card_width_at(side)))
            .height(Sizing::FIT)
            .padding(Padding::all(CONTROL_METRICS.spacing.card_padding))
            .gap(CONTROL_METRICS.spacing.row_gap)
            .child_divider(ChildDivider::new(Px(1.0), ROW_DIVIDER_COLOR))
            .background(PANEL_BACKGROUND)
            .border(Border::all(PANEL_BORDER_WIDTH, PANEL_BORDER_COLOR))
            .corner_radius(CornerRadius::all(PANEL_RADIUS))
            .tooltip_boundary(),
        |builder| add_widget_card_content(builder, face, slider, primary_tooltip, slider_tooltip),
    );
    if matches!(face, WidgetLabFace::Front | WidgetLabFace::Cascade) {
        add_status_card(&mut builder, face, side, readouts);
    }
    builder.build()
}

fn add_widget_card_content(
    builder: &mut LayoutBuilder,
    face: WidgetLabFace,
    slider: Slider,
    primary_tooltip: Tooltip,
    slider_tooltip: Tooltip,
) {
    if matches!(face, WidgetLabFace::Front | WidgetLabFace::Cascade) {
        add_editable_text(builder, face);
        add_button_row(
            builder,
            face,
            PRIMARY_BUTTON_ID,
            "Changes",
            "Apply",
            Button::new(),
            |element| {
                let element = element
                    .background(ControlPalette::system_dark().accent_behind_text)
                    .tooltip(primary_tooltip)
                    .on_click(count_primary_click);
                if matches!(face, WidgetLabFace::Front) {
                    element
                        .hovered(BackgroundColor(PRIMARY_BUTTON_HOVERED))
                        .pressed(BackgroundColor(PRIMARY_BUTTON_PRESSED))
                } else {
                    element
                }
            },
        );
        add_button_row(
            builder,
            face,
            SECONDARY_BUTTON_ID,
            "Last edit",
            "Revert",
            Button::new(),
            |element| element.tooltip(authored_tooltip("Reports each press until D disables it.")),
        );
    }
    if matches!(face, WidgetLabFace::Front) {
        add_button_row(
            builder,
            face,
            TOGGLE_BUTTON_ID,
            "Keep on top",
            "Pin",
            Button::new().toggle(),
            |element| element,
        );
    }
    if matches!(face, WidgetLabFace::Left) {
        add_switch(builder);
        add_slider(
            builder,
            SLIDER_ID,
            SLIDER_LABEL_ID,
            slider,
            slider_tooltip,
            WidgetLabFace::Left,
        );
        add_slider(
            builder,
            STEPLESS_SLIDER_ID,
            STEPLESS_SLIDER_LABEL_ID,
            Slider::new(SLIDER_RANGE_START..=SLIDER_RANGE_END)
                .default_value(SLIDER_INITIAL_VALUE)
                .direction(SliderDirection::LeftToRight),
            above_slider_tooltip("Sets the value with no steps."),
            WidgetLabFace::Left,
        );
        add_disabled_row(builder);
    } else if matches!(face, WidgetLabFace::Cascade) {
        add_slider(
            builder,
            SLIDER_ID,
            SLIDER_LABEL_ID,
            slider,
            slider_tooltip,
            WidgetLabFace::Cascade,
        );
    }
}

fn card_row_slot_height() -> Px {
    Px(CONTROL_METRICS
        .button
        .height
        .0
        .max(CONTROL_METRICS.field.height.0)
        .max(CONTROL_METRICS.slider.thumb_hit_height.0))
}

fn add_switch(builder: &mut LayoutBuilder) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::fixed(card_row_slot_height()))
            .gap(CONTROL_METRICS.row.gap)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(CONTROL_METRICS.row.label_width))
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text((
                        "Power",
                        TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
                    ));
                },
            );
            builder.slider(
                "power-level",
                Slider::new(0.0..=1.0).default_value(0.5),
                &CONTROL_METRICS,
                hana_diegetic::SliderReachableBand::Hidden,
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(CONTROL_METRICS.slider.value_width))
                    .height(Sizing::FIT)
                    .alignment(AlignX::Right, AlignY::Center),
                |builder| {
                    builder.with(
                        El::new()
                            .size(
                                CONTROL_METRICS.switch.track_width,
                                CONTROL_METRICS.switch.track_height,
                            )
                            .padding(Padding::all(CONTROL_METRICS.switch.track_padding))
                            .alignment(AlignX::Left, AlignY::Center)
                            .background(ControlPalette::system_dark().accent)
                            .border(Border::all(
                                CONTROL_METRICS.switch.border_width,
                                ControlPalette::system_dark().switch_border,
                            ))
                            .corner_radius(CornerRadius::all(CONTROL_METRICS.switch.track_radius()))
                            .switch(SWITCH_ID),
                        |builder| {
                            builder.switch_knob(&CONTROL_METRICS.switch);
                        },
                    );
                },
            );
        },
    );
}

fn add_disabled_row(builder: &mut LayoutBuilder) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .padding(Padding::new(
                Px(0.0),
                Px(0.0),
                CONTROL_METRICS.spacing.card_padding,
                Px(0.0),
            ))
            .gap(CONTROL_METRICS.spacing.row_gap),
        |builder| {
            let (top_trim, bottom_trim) = control_text_edge_trim();
            builder.text(
                Text::new(
                    "Unavailable",
                    TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
                )
                .trim_text_box(top_trim, bottom_trim),
            );
            add_disabled_labeled_row(builder, "Switch", |builder| {
                builder.with(El::new().width(Sizing::GROW), |_| {});
                add_disabled_switch(builder);
            });
            add_disabled_labeled_row(builder, "Slider", |builder| {
                add_disabled_slider(builder);
                builder.with(
                    El::new().width(Sizing::fixed(CONTROL_METRICS.slider.value_width)),
                    |_| {},
                );
            });
            add_disabled_labeled_row(builder, "Button", |builder| {
                builder.with(El::new().width(Sizing::GROW), |_| {});
                add_button(
                    builder,
                    DISABLED_BUTTON_ID,
                    "Apply",
                    Button::new(),
                    |element| element.disabled(Appearance::new()),
                );
            });
            add_disabled_labeled_row(builder, "Name", |builder| {
                builder.with(El::new().width(Sizing::GROW), |_| {});
                add_disabled_field(builder);
            });
            add_switchable_no_device_row(builder);
        },
    );
}

fn add_disabled_labeled_row(
    builder: &mut LayoutBuilder,
    label: &'static str,
    add_control: impl FnOnce(&mut LayoutBuilder),
) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::fixed(card_row_slot_height()))
            .gap(CONTROL_METRICS.row.gap)
            .alignment(AlignX::Left, AlignY::Center)
            .widget_interactivity(WidgetInteractivity::Disabled),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(CONTROL_METRICS.row.label_width))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((
                        label,
                        TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
                    ));
                },
            );
            add_control(builder);
        },
    );
}

fn add_switchable_no_device_row(builder: &mut LayoutBuilder) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(CONTROL_METRICS.row.gap)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text(Text::new(
                "No device (still switchable)",
                TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
            ));
            builder.with(El::new().width(Sizing::GROW), |_| {});
            builder.with(
                El::new()
                    .size(
                        CONTROL_METRICS.switch.track_width,
                        CONTROL_METRICS.switch.track_height,
                    )
                    .padding(Padding::all(CONTROL_METRICS.switch.track_padding))
                    .alignment(AlignX::Left, AlignY::Center)
                    .background(ControlPalette::system_dark().accent)
                    .border(Border::all(
                        CONTROL_METRICS.switch.border_width,
                        ControlPalette::system_dark().switch_border,
                    ))
                    .corner_radius(CornerRadius::all(CONTROL_METRICS.switch.track_radius()))
                    .switch(NO_DEVICE_SWITCH_ID),
                |builder| {
                    builder.switch_knob(&CONTROL_METRICS.switch);
                },
            );
        },
    );
}

fn add_disabled_switch(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .size(
                CONTROL_METRICS.switch.track_width,
                CONTROL_METRICS.switch.track_height,
            )
            .padding(Padding::all(CONTROL_METRICS.switch.track_padding))
            .alignment(AlignX::Left, AlignY::Center)
            .background(ControlPalette::system_dark().accent)
            .border(Border::all(
                CONTROL_METRICS.switch.border_width,
                ControlPalette::system_dark().switch_border,
            ))
            .corner_radius(CornerRadius::all(CONTROL_METRICS.switch.track_radius()))
            .switch(DISABLED_SWITCH_ID)
            .disabled(Appearance::new()),
        |builder| {
            builder.switch_knob(&CONTROL_METRICS.switch);
        },
    );
}

fn slider_surface_minimum_width() -> Px {
    Px(2.0f32.mul_add(slider_focus_clearance().0, CONTROL_METRICS.slider.width.0))
}

fn add_disabled_slider(builder: &mut LayoutBuilder) {
    builder.with(
        El::overlay()
            .width(Sizing::grow_min(slider_surface_minimum_width()))
            .height(Sizing::fixed(slider_focus_height()))
            .padding(Padding::xy(slider_focus_clearance(), Px(0.0)))
            .alignment(AlignX::Left, AlignY::Center)
            .border(Border::all(Px(0.0), ControlPalette::system_dark().track))
            .corner_radius(CornerRadius::all(Px(slider_focus_height().0 * 0.5)))
            .widget(
                DISABLED_SLIDER_ID,
                Slider::new(SLIDER_RANGE_START..=SLIDER_RANGE_END)
                    .default_value(SLIDER_INITIAL_VALUE)
                    .direction(SliderDirection::LeftToRight),
            )
            .disabled(
                Appearance::new()
                    .background(Color::NONE)
                    .border_width(Px(0.0)),
            ),
        |builder| {
            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(CONTROL_METRICS.slider.thumb_hit_height))
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    builder.with(
                        builder
                            .child(El::overlay())
                            .width(Sizing::GROW)
                            .height(Sizing::fixed(CONTROL_METRICS.slider.track_height))
                            .background(ControlPalette::system_dark().track)
                            .corner_radius(CornerRadius::all(
                                CONTROL_METRICS.slider.track_radius(),
                            )),
                        |builder| {
                            builder.with(
                                builder
                                    .child(El::new())
                                    .width(Sizing::GROW)
                                    .height(Sizing::GROW)
                                    .background(ControlPalette::system_dark().accent)
                                    .disabled(BackgroundColor(SLIDER_DISABLED_FILL))
                                    .corner_radius(CornerRadius::all(
                                        CONTROL_METRICS.slider.track_radius(),
                                    ))
                                    .slider_fill(),
                                |_| {},
                            );
                        },
                    );
                },
            );
            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.slider_thumb(&CONTROL_METRICS.slider);
                },
            );
        },
    );
}

fn add_disabled_field(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::fixed(CONTROL_METRICS.field.width))
            .height(Sizing::fixed(CONTROL_METRICS.field.height))
            .padding(Padding::xy(CONTROL_METRICS.button.side_padding, Px(0.0)))
            .alignment(AlignX::Left, AlignY::Center)
            .background(ControlPalette::system_dark().field_fill)
            .border(Border::all(
                FIELD_BORDER_WIDTH,
                ControlPalette::system_dark().field_border,
            ))
            .corner_radius(CornerRadius::all(CONTROL_METRICS.button.corner_radius))
            .editable_field(
                DISABLED_FIELD_ID,
                ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text)),
                TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
            )
            .disabled(Appearance::new()),
        |builder| {
            builder.text((
                TEXT_FIELD_INITIAL,
                TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
            ));
        },
    );
}

fn add_slider(
    builder: &mut LayoutBuilder,
    id: &'static str,
    label_id: &'static str,
    slider: Slider,
    slider_tooltip: Tooltip,
    _face: WidgetLabFace,
) {
    let row = El::row()
        .width(Sizing::GROW)
        .height(Sizing::fixed(card_row_slot_height()))
        .gap(CONTROL_METRICS.row.gap)
        .alignment(AlignX::Left, AlignY::Center);
    builder.with(row, |builder| {
        builder.with(
            El::new()
                .width(Sizing::fixed(CONTROL_METRICS.row.label_width))
                .height(Sizing::FIT)
                .alignment(AlignX::Left, AlignY::Center),
            |builder| add_slider_caption(builder, id),
        );
        add_slider_surface(builder, id, slider, slider_tooltip);
        add_slider_value_label(builder, label_id, id);
    });
}

fn add_slider_surface(
    builder: &mut LayoutBuilder,
    id: &'static str,
    slider: Slider,
    slider_tooltip: Tooltip,
) {
    builder.with(
        El::overlay()
            .width(Sizing::grow_min(slider_surface_minimum_width()))
            .height(Sizing::fixed(CONTROL_METRICS.slider.thumb_hit_height))
            // Keep the visible thumb in the row slot while its focus ring extends outside it.
            .padding(Padding::new(
                Px(0.0),
                Px(0.0),
                Px(-slider_focus_clearance().0),
                Px(0.0),
            ))
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(slider_focus_height()))
                    .padding(Padding::xy(slider_focus_clearance(), Px(0.0)))
                    .alignment(AlignX::Left, AlignY::Center)
                    .border(Border::all(Px(0.0), ControlPalette::system_dark().track))
                    .corner_radius(CornerRadius::all(Px(slider_focus_height().0 * 0.5)))
                    .widget(id, slider)
                    .disabled(
                        Appearance::new()
                            .background(Color::NONE)
                            .border_width(Px(0.0)),
                    )
                    .tooltip(slider_tooltip),
                |builder| {
                    builder.with(
                        El::overlay()
                            .width(Sizing::GROW)
                            .height(Sizing::fixed(CONTROL_METRICS.slider.thumb_hit_height))
                            .alignment(AlignX::Center, AlignY::Center),
                        |builder| {
                            builder.with(
                                builder
                                    .child(El::overlay())
                                    .width(Sizing::GROW)
                                    .height(Sizing::fixed(CONTROL_METRICS.slider.track_height))
                                    .background(ControlPalette::system_dark().track)
                                    .disabled(BackgroundColor(SLIDER_DISABLED_COLOR))
                                    .corner_radius(CornerRadius::all(
                                        CONTROL_METRICS.slider.track_radius(),
                                    )),
                                |builder| {
                                    builder.with(
                                        builder
                                            .child(El::new())
                                            .width(Sizing::GROW)
                                            .height(Sizing::GROW)
                                            .background(ControlPalette::system_dark().accent)
                                            .disabled(BackgroundColor(SLIDER_DISABLED_FILL))
                                            .corner_radius(CornerRadius::all(
                                                CONTROL_METRICS.slider.track_radius(),
                                            ))
                                            .slider_fill(),
                                        |_| {},
                                    );
                                },
                            );
                        },
                    );
                    builder.with(
                        El::overlay()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .alignment(AlignX::Left, AlignY::Center),
                        |builder| {
                            builder.slider_thumb(&CONTROL_METRICS.slider);
                        },
                    );
                },
            );
        },
    );
}

fn add_slider_caption(builder: &mut LayoutBuilder, id: &'static str) {
    let label = if id == SLIDER_ID { "Level" } else { "Smooth" };
    builder.text(
        Text::new(
            label,
            TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
        )
        .layout(El::new().dim_with_widget(id)),
    );
}

fn slider_focus_clearance() -> Px {
    Px(CONTROL_METRICS.focus_border_width.0 * SLIDER_FOCUS_SIDE_CLEARANCE_MULTIPLIER)
}

fn slider_focus_height() -> Px {
    Px(slider_focus_clearance()
        .0
        .mul_add(2.0, CONTROL_METRICS.slider.thumb_hit_height.0))
}

fn add_slider_value_label(
    builder: &mut LayoutBuilder,
    label_id: &'static str,
    slider_id: &'static str,
) {
    builder.with(
        El::new()
            .width(Sizing::fixed(CONTROL_METRICS.slider.value_width))
            .height(Sizing::FIT)
            .alignment(AlignX::Right, AlignY::Center),
        |builder| {
            builder.text(
                Text::new(
                    SLIDER_LABEL_IDLE,
                    TextStyle::new(fairy_dust::LABEL_SIZE)
                        .with_color(SLIDER_LABEL_TEXT)
                        .with_align(TextAlign::Right),
                )
                .id(label_id)
                .measure_as("100%")
                .layout(El::new().dim_with_widget(slider_id)),
            );
        },
    );
}

fn add_editable_text(builder: &mut LayoutBuilder, _face: WidgetLabFace) {
    let field = ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
    let row_height = card_row_slot_height();
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::fixed(row_height))
            .gap(CONTROL_METRICS.row.gap)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(CONTROL_METRICS.row.label_width))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((
                        "Name",
                        TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
                    ));
                },
            );
            builder.with(El::new().width(Sizing::GROW), |_| {});
            builder.with(
                El::new()
                    .size(CONTROL_METRICS.field.width, CONTROL_METRICS.field.height)
                    .padding(Padding::xy(CONTROL_METRICS.button.side_padding, Px(0.0)))
                    .alignment(AlignX::Left, AlignY::Center)
                    .background(ControlPalette::system_dark().field_fill)
                    .border(Border::all(
                        FIELD_BORDER_WIDTH,
                        ControlPalette::system_dark().field_border,
                    ))
                    .corner_radius(CornerRadius::all(CONTROL_METRICS.button.corner_radius))
                    .editable_field(
                        TEXT_FIELD_ID,
                        field,
                        TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
                    )
                    .editor_text(EditorStateColors::new().focused(CONTROL_TEXT))
                    .editor_selection(EditorStateColors::new().focused(BUTTON_FILL_HOVERED))
                    .editor_caret(EditorStateColors::new().focused(BUTTON_BORDER_FOCUSED))
                    .editor_validation(EditorStateColors::new().focused(BUTTON_BORDER_FOCUSED))
                    .tooltip(authored_tooltip("Holds a name you can edit.")),
                |builder| {
                    builder.text((
                        TEXT_FIELD_INITIAL,
                        TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
                    ));
                },
            );
        },
    );
}

fn add_status_card(
    builder: &mut LayoutBuilder,
    face: WidgetLabFace,
    side: Px,
    readouts: &FaceStatusReadouts,
) {
    // The front readout fills the rest of its face, so the face keeps even
    // margins when a taller face sets the shared side.
    let height = match face {
        WidgetLabFace::Front => Sizing::GROW,
        WidgetLabFace::Left | WidgetLabFace::Cascade => Sizing::FIT,
    };
    builder.with(
        El::column()
            .width(Sizing::fixed(face_card_width_at(side)))
            .height(height)
            .padding(text_card_padding())
            .gap(CONTROL_METRICS.spacing.row_gap)
            .alignment(AlignX::Left, AlignY::Center)
            .background(STATUS_BACKGROUND)
            .border(Border::all(STATUS_BORDER_WIDTH, STATUS_BORDER))
            .corner_radius(CornerRadius::all(STATUS_RADIUS)),
        |builder| {
            if matches!(face, WidgetLabFace::Front) {
                interaction_status_row(
                    builder,
                    "Focus",
                    FOCUS_STATUS_ID,
                    readouts.focus.clone(),
                    FOCUS_STATUS_MEASURE,
                );
                interaction_status_row(
                    builder,
                    "Button",
                    BUTTON_STATUS_ID,
                    readouts.button.clone(),
                    BUTTON_STATUS_MEASURE,
                );
            }
            interaction_status_row(
                builder,
                "State",
                STATE_STATUS_ID,
                if matches!(face, WidgetLabFace::Cascade) {
                    CASCADE_STATE_STATUS_IDLE.to_owned()
                } else {
                    readouts.state.clone()
                },
                if matches!(face, WidgetLabFace::Cascade) {
                    CASCADE_STATE_STATUS_MEASURE
                } else {
                    STATE_STATUS_MEASURE
                },
            );
        },
    );
}

fn interaction_status_row(
    builder: &mut LayoutBuilder,
    label: &'static str,
    id: &'static str,
    value: String,
    measure_as: &'static str,
) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(CONTROL_METRICS.spacing.row_gap)
            .alignment(AlignX::Left, AlignY::Top),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(STATUS_LABEL_WIDTH))
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text((label, status_label_style()));
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text(
                        Text::new(value, status_value_style())
                            .id(id)
                            .measure_as(measure_as),
                    );
                },
            );
        },
    );
}

fn status_label_style() -> TextStyle {
    TextStyle::new(STATUS_TEXT_SIZE)
        .with_color(STATUS_LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn status_value_style() -> TextStyle {
    TextStyle::new(STATUS_TEXT_SIZE)
        .with_color(STATUS_COLOR)
        .with_line_height(status_line_pitch())
        .with_shadow_mode(GlyphShadowMode::None)
}

fn status_line_pitch() -> f32 {
    static PITCH: LazyLock<f32> = LazyLock::new(|| {
        FontRegistry::new().map_or(STATUS_TEXT_SIZE.0, |fonts| {
            fonts
                .font(FontId::MONOSPACE)
                .map_or(STATUS_TEXT_SIZE.0, |font| {
                    font.metrics(STATUS_TEXT_SIZE.0).line_height.ceil()
                })
        })
    });
    *PITCH
}

fn cube_front_transform(local_position: Vec2) -> Transform {
    let mut transform = cube_face_transform(Face::Front, fairy_dust::EXAMPLE_CUBE_SIZE);
    transform.translation += transform.rotation * local_position.extend(PANEL_FACE_OFFSET);
    transform
}

fn cube_right_transform(local_position: Vec2) -> Transform {
    let mut transform = cube_face_transform(Face::Right, fairy_dust::EXAMPLE_CUBE_SIZE);
    transform.translation += transform.rotation * local_position.extend(PANEL_FACE_OFFSET);
    transform
}

fn cube_left_transform(local_position: Vec2) -> Transform {
    let mut transform = cube_face_transform(Face::Left, fairy_dust::EXAMPLE_CUBE_SIZE);
    transform.translation += transform.rotation * local_position.extend(PANEL_FACE_OFFSET);
    transform
}

fn face_panel_origin() -> Vec2 {
    Vec2::new(
        -fairy_dust::EXAMPLE_CUBE_SIZE * 0.5,
        fairy_dust::EXAMPLE_CUBE_SIZE * 0.5,
    )
}

fn cascade_panel_transform() -> Transform { cube_right_transform(face_panel_origin()) }

fn widget_panel_transform() -> Transform { cube_front_transform(face_panel_origin()) }

fn left_panel_transform() -> Transform { cube_left_transform(face_panel_origin()) }

fn face_card_width_at(side: Px) -> Px {
    Px((-2.0_f32).mul_add(CONTROL_METRICS.spacing.panel_padding.0, side.0))
}

/// A button element in the widget lab's row layout.
type ButtonElement = El<Row, WidgetElement<Button>>;

fn add_button(
    builder: &mut LayoutBuilder,
    id: &'static str,
    label: &'static str,
    button: Button,
    configure: impl FnOnce(ButtonElement) -> ButtonElement,
) {
    let element = configure(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::fixed(CONTROL_METRICS.button.height))
            .padding(Padding::xy(CONTROL_METRICS.button.side_padding, Px(0.0)))
            .alignment(AlignX::Center, AlignY::Center)
            .background(ControlPalette::system_dark().control_fill)
            .border(Border::all(
                Px(0.0),
                ControlPalette::system_dark().control_fill,
            ))
            .corner_radius(CornerRadius::all(CONTROL_METRICS.button.corner_radius))
            .widget(id, button),
    );
    builder.with(element, |builder| {
        builder.text((
            label,
            TextStyle::new(fairy_dust::LABEL_SIZE).with_color(if id == PRIMARY_BUTTON_ID {
                ControlPalette::system_dark().knob
            } else {
                CONTROL_TEXT
            }),
        ));
    });
}

fn add_button_row(
    builder: &mut LayoutBuilder,
    _face: WidgetLabFace,
    id: &'static str,
    row_label: &'static str,
    label: &'static str,
    button: Button,
    configure: impl FnOnce(ButtonElement) -> ButtonElement,
) {
    let row_height = Sizing::fixed(card_row_slot_height());
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(row_height)
            .gap(CONTROL_METRICS.row.gap)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text(
                        Text::new(
                            row_label,
                            TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
                        )
                        .measure_as(BUTTON_ROW_LABEL_MEASURE),
                    );
                },
            );
            builder.with(El::new().width(Sizing::GROW), |_| {});
            add_button(builder, id, label, button, configure);
        },
    );
}

fn title_text_edge_trim() -> (Px, Px) {
    static TRIM: LazyLock<(Px, Px)> = LazyLock::new(|| {
        FontRegistry::new().map_or((Px(0.0), Px(0.0)), |fonts| {
            fonts
                .font(FontId::MONOSPACE)
                .map_or((Px(0.0), Px(0.0)), |font| {
                    let metrics = font.metrics(fairy_dust::TITLE_SIZE.0 * 4.0 / 3.0);
                    (
                        Px(metrics.line_gap.mul_add(0.5, metrics.ascent) - metrics.cap_height),
                        Px(metrics.line_gap.mul_add(0.5, metrics.descent)),
                    )
                })
        })
    });
    *TRIM
}

fn control_text_edge_trim() -> (Px, Px) {
    static TRIM: LazyLock<(Px, Px)> = LazyLock::new(|| {
        FontRegistry::new().map_or((Px(0.0), Px(0.0)), |fonts| {
            fairy_dust::screen_panel_text_edge_trim(&fonts, fairy_dust::LABEL_SIZE)
        })
    });
    *TRIM
}

fn text_card_padding() -> Padding {
    let side = CONTROL_METRICS.spacing.card_padding;
    let (top_trim, bottom_trim) = control_text_edge_trim();
    Padding::new(
        side,
        side,
        Px(side.0 - top_trim.0),
        Px(side.0 - bottom_trim.0),
    )
}

fn slider_tooltip() -> Tooltip { above_slider_tooltip("Sets the value in 5% steps.") }

fn above_slider_tooltip(text: &'static str) -> Tooltip {
    authored_tooltip(text)
        .source_anchor(SLIDER_TOOLTIP_SOURCE_ANCHOR)
        .target_anchor(SLIDER_TOOLTIP_TARGET_ANCHOR)
}

fn cube_tooltip() -> Tooltip {
    authored_tooltip("Hover a control for help.").with_label_size(CUBE_TOOLTIP_LABEL_SIZE)
}

fn screen_attach_tooltip(state: ScreenWidgetAttachmentState) -> Tooltip {
    authored_tooltip(state.action_help())
        .source_anchor(Anchor::CenterRight)
        .target_anchor(Anchor::CenterLeft)
}

/// A one-sentence help tag beside its control, as macOS writes one.
fn authored_tooltip(text: &'static str) -> Tooltip {
    let mut tooltip = Tooltip::new(El::column().width(Sizing::FIT).height(Sizing::FIT))
        .with_label_size(fairy_dust::LABEL_SIZE)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    tooltip.text(text);
    tooltip
}

/// Logs each pointer interaction change on the front, left, and screen panels.
fn report_interaction_changes(
    panel: Single<Entity, With<WidgetLabPanel>>,
    left_panel: Single<Entity, With<LeftWidgetLabPanel>>,
    screen_panel: Single<Entity, With<ScreenWidgetLabPanel>>,
    widgets: Query<(&PanelWidget, &WidgetOf, &PickingInteraction), Changed<PickingInteraction>>,
) {
    for (widget, widget_of, interaction) in &widgets {
        if [*panel, *left_panel, *screen_panel].contains(&widget_of.panel()) {
            info!(
                "widgets: {} interaction changed to {:?}",
                widget.id(),
                interaction
            );
        }
    }
}

/// Mirrors button presses, slider drags, hover, and availability into State.
fn report_presentation_states(
    panel: Single<Entity, With<WidgetLabPanel>>,
    readout: Single<Entity, With<WidgetInteractionReadout>>,
    cascade_panel: Single<Entity, With<CascadeWidgetLabPanel>>,
    cascade_readout: Single<Entity, With<CascadeInteractionReadout>>,
    widgets: Query<(
        Entity,
        &PanelWidget,
        &WidgetOf,
        Option<&PickingInteraction>,
        Option<&ButtonLatch>,
        Option<&SwitchState>,
        Has<WidgetDisabled>,
    )>,
    presses: Res<HeldButtonPresses>,
    drags: Res<SliderDrags>,
    mut panel_text: PanelText,
) {
    let flags = |panel: Entity, id: &str| {
        widgets
            .iter()
            .find(|(_, widget, widget_of, ..)| {
                widget_of.panel() == panel && *widget.id() == PanelElementId::named(id)
            })
            .map(|(entity, _, _, interaction, latch, switch, disabled)| {
                let is_slider = id == SLIDER_ID || id == STEPLESS_SLIDER_ID;
                let is_pressed = if is_slider {
                    drags.0.contains(&entity)
                } else {
                    presses.0.contains(&entity) || matches!(latch, Some(ButtonLatch::Pressed))
                };
                let mut parts = Vec::new();
                if is_pressed {
                    parts.push("pressed");
                } else if matches!(
                    interaction,
                    Some(PickingInteraction::Hovered | PickingInteraction::Pressed)
                ) {
                    parts.push("hovered");
                }
                if let Some(switch) = switch {
                    parts.push(match switch.side {
                        SwitchSide::On => "on",
                        SwitchSide::Off => "off",
                    });
                }
                if disabled {
                    parts.push("unavailable");
                }
                if parts.is_empty() {
                    "at rest".to_owned()
                } else {
                    parts.join(", ")
                }
            })
    };
    let front_status = || {
        Some(format!(
            "Apply: {}\nRevert: {}\nPin: {}",
            flags(*panel, PRIMARY_BUTTON_ID)?,
            flags(*panel, SECONDARY_BUTTON_ID)?,
            flags(*panel, TOGGLE_BUTTON_ID)?,
        ))
    };
    let cascade_status = || {
        Some(format!(
            "Apply: {}\nRevert: {}\nLevel: {}",
            flags(*cascade_panel, PRIMARY_BUTTON_ID)?,
            flags(*cascade_panel, SECONDARY_BUTTON_ID)?,
            flags(*cascade_panel, SLIDER_ID)?,
        ))
    };
    // `PanelText` skips the layout revision bump for an unchanged string, so
    // writing every frame stays free of relayout work.
    if let Some(front_status) = front_status() {
        panel_text.set_text(
            *readout,
            &PanelElementId::named(STATE_STATUS_ID),
            front_status,
        );
    }
    if let Some(cascade_status) = cascade_status() {
        panel_text.set_text(
            *cascade_readout,
            &PanelElementId::named(STATE_STATUS_ID),
            cascade_status,
        );
    }
}

/// Applies `interactivity` to one panel's secondary button and level slider,
/// reporting whether both were reified and updated.
fn set_toggled_interactivity(
    secondary_panel: Entity,
    slider_panel: Entity,
    reader: &PanelWidgetReader,
    writer: &mut PanelWidgetWriter,
    interactivity: WidgetInteractivity,
) -> bool {
    let secondary = reader.entity(secondary_panel, &PanelElementId::named(SECONDARY_BUTTON_ID));
    let slider = reader.entity(slider_panel, &PanelElementId::named(SLIDER_ID));
    let (Some(secondary), Some(slider)) = (secondary, slider) else {
        warn!("widgets: secondary button or level slider has not been reified");
        return false;
    };
    let secondary_toggled = writer.override_interactivity(secondary, interactivity);
    let slider_toggled = writer.override_interactivity(slider, interactivity);
    secondary_toggled && slider_toggled
}

fn toggle_disabled_widgets(
    panel: Single<Entity, With<WidgetLabPanel>>,
    left_panel: Single<Entity, With<LeftWidgetLabPanel>>,
    cascade_panel: Single<Entity, With<CascadeWidgetLabPanel>>,
    reader: PanelWidgetReader,
    mut writer: PanelWidgetWriter,
    mut mode: ResMut<ToggleMode>,
) {
    let next = mode.toggled();
    let front_toggled = set_toggled_interactivity(
        *panel,
        *left_panel,
        &reader,
        &mut writer,
        next.interactivity(),
    );
    let cascade_toggled = set_toggled_interactivity(
        *cascade_panel,
        *cascade_panel,
        &reader,
        &mut writer,
        next.interactivity(),
    );
    if front_toggled && cascade_toggled {
        *mode = next;
        info!(
            "widgets: both secondary buttons and level sliders are now {:?}",
            next.interactivity()
        );
    } else {
        warn!("widgets: failed to update secondary button or level slider interactivity");
    }
}

fn replace_primary_tooltip(
    panel: Single<Entity, With<WidgetLabPanel>>,
    panels: Query<&DiegeticPanel>,
    mut primary_tooltip_timing: ResMut<PrimaryTooltipTiming>,
    slider_tooltip: Res<SliderTooltipBlueprint>,
    panel_text: PanelTextReader,
    mut commands: Commands,
) {
    let next_timing = primary_tooltip_timing.toggled();
    let slider = slider_declaration();
    let readouts = FaceStatusReadouts::from_panel(*panel, &panel_text);
    let side = panels
        .get(*panel)
        .map_or(FACE_PANEL_WIDTH, |panel| Px(panel.width()));
    match commands.set_tree(
        *panel,
        face_tree_at_width(
            WidgetLabFace::Front,
            side,
            slider,
            next_timing.tooltip(),
            slider_tooltip.0.clone(),
            &readouts,
        ),
    ) {
        Ok(()) => {
            *primary_tooltip_timing = next_timing;
            info!(
                "widgets: queued a primary-button tooltip with a {:.1} s show delay",
                next_timing.delay().as_secs_f32()
            );
        },
        Err(error) => warn!("widgets: failed to replace primary-button tooltip: {error}"),
    }
}
