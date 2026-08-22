//! Canonical runtime example for `hana_diegetic` widgets.
//!
//! The example keeps the complete widget interaction path runnable in one
//! Fairy Dust app.
//!
//! Current controls:
//!   D - Disable or re-enable the secondary button and level slider
//!   H - Return to the camera home pose
//!   Tab / Shift+Tab - Move keyboard focus to the next/previous widget
//!   Enter or Space / Escape - Activate/cancel the focused widget
//!   P - Move keyboard focus to the previous widget through an app-defined
//!     shortcut
//!   T - Switch the primary-button tooltip between two show delays
//!   Left / Right Arrow - Step the focused slider; holding repeats after a
//!     short pause
//!   Pointer drag on the level slider - Grab, drag, and release move the value
//!     through the same `SliderChangeRequested` proposals the app applies.

use std::time::Duration;

use bevy::anti_alias::smaa::Smaa;
use bevy::picking::hover::PickingInteraction;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Event;
use bevy::prelude::Handle;
use bevy::prelude::Has;
use bevy::prelude::In;
use bevy::prelude::KeyCode;
use bevy::prelude::MessageWriter;
use bevy::prelude::Msaa;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::PostStartup;
use bevy::prelude::Query;
use bevy::prelude::Ref;
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
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::Fire;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use bevy_enhanced_input::prelude::Pulse;
use bevy_enhanced_input::prelude::bindings;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DescriptionPanel;
use fairy_dust::Face;
use fairy_dust::FairyDustCube;
use fairy_dust::TitleBar;
use fairy_dust::cube_face_panel_material;
use fairy_dust::cube_face_transform;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Appearance;
use hana_diegetic::BackgroundColor;
use hana_diegetic::Border;
use hana_diegetic::BorderColor;
use hana_diegetic::Button;
use hana_diegetic::ButtonCanceled;
use hana_diegetic::ButtonClicked;
use hana_diegetic::ButtonPressed;
use hana_diegetic::ButtonReleased;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticText;
use hana_diegetic::EditorStateColors;
use hana_diegetic::El;
use hana_diegetic::FacePicking;
use hana_diegetic::Fit;
use hana_diegetic::FitMax;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::ImeBuiltInFieldKind;
use hana_diegetic::ImeBuiltInFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::MeshAnchorCommandsExt;
use hana_diegetic::MeshFace;
use hana_diegetic::Padding;
use hana_diegetic::PanelAnchorOffset;
use hana_diegetic::PanelAttachment;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelEntityReader;
use hana_diegetic::PanelPicking;
use hana_diegetic::PanelText;
use hana_diegetic::PanelWidget;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::PanelWidgetWriter;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::RequestPanelFocus;
use hana_diegetic::RequestSliderAdjustment;
use hana_diegetic::Row;
use hana_diegetic::ShadowCasting;
use hana_diegetic::Sizing;
use hana_diegetic::Slider;
use hana_diegetic::SliderAdjustment;
use hana_diegetic::SliderCancelCause;
use hana_diegetic::SliderCanceled;
use hana_diegetic::SliderChangeRequested;
use hana_diegetic::SliderDirection;
use hana_diegetic::SliderGrabbed;
use hana_diegetic::SliderReleased;
use hana_diegetic::SliderResetBehavior;
use hana_diegetic::SliderState;
use hana_diegetic::Text;
use hana_diegetic::TextAlign;
use hana_diegetic::TextColor;
use hana_diegetic::TextStyle;
use hana_diegetic::Tooltip;
use hana_diegetic::TooltipCommandsExt;
use hana_diegetic::TooltipFor;
use hana_diegetic::TooltipHidden;
use hana_diegetic::TooltipPlacementPolicy;
use hana_diegetic::TooltipShown;
use hana_diegetic::WidgetDisabled;
use hana_diegetic::WidgetDisabledAppearance;
use hana_diegetic::WidgetElement;
use hana_diegetic::WidgetFocusChanged;
use hana_diegetic::WidgetFocused;
use hana_diegetic::WidgetFocusedAppearance;
use hana_diegetic::WidgetHoveredAppearance;
use hana_diegetic::WidgetInput;
use hana_diegetic::WidgetInputPlugin;
use hana_diegetic::WidgetInteractivity;
use hana_diegetic::WidgetOf;
use hana_diegetic::WidgetPressedAppearance;
use hana_lagrange::OrbitCamPreset;
use hana_rubric::Keybindings;
use hana_rubric::action;
use hana_rubric::bind_action_system;
use hana_rubric::event;

// widget lab
const BUTTON_BORDER: Color = Color::srgba(0.30, 0.62, 1.0, 0.90);
const BUTTON_BORDER_DISABLED: Color = Color::srgba(0.34, 0.36, 0.40, 0.60);
const BUTTON_BORDER_FOCUSED: Color = Color::srgba(1.0, 0.86, 0.30, 0.94);
const BUTTON_FILL: Color = Color::srgba(0.03, 0.10, 0.24, 0.92);
const BUTTON_FILL_DISABLED: Color = Color::srgba(0.10, 0.11, 0.13, 0.78);
const BUTTON_FILL_HOVERED: Color = Color::srgba(0.10, 0.26, 0.52, 0.95);
const BUTTON_FILL_PRESSED: Color = Color::srgba(0.55, 0.30, 0.08, 0.98);
const BUTTON_HEIGHT: Px = Px(42.0);
const CONTROL_BORDER_WIDTH: Px = Px(1.0);
const CONTROL_BORDER_WIDTH_FOCUSED: Px = Px(3.0);
const CONTROL_GAP: Px = Px(8.0);
const CONTROL_PADDING: Px = Px(8.0);
const CONTROL_RADIUS: Px = Px(7.0);
const CONTROL_TEXT: Color = Color::srgb(0.92, 0.96, 1.0);
const CONTROL_WIDTH: Px = Px(280.0);
const CUBE_CLEARANCE: f32 = 0.1;
const CASCADE_FACE_LIGHT_ILLUMINANCE: f32 = 5_000.0;
const CASCADE_FACE_LIGHT_POS: Vec3 = Vec3::new(5.0, 2.5, 1.5);
const CASCADE_PANEL_TITLE: &str = "Cascade Lab";
const CUBE_EDGE_INSET: f32 = 0.06;
const CUBE_HALF_USABLE_WIDTH: f32 = (fairy_dust::EXAMPLE_CUBE_SIZE - CUBE_EDGE_INSET * 2.0) * 0.5;
const DESCRIPTION_LINES: [&str; 6] = [
    "D disables the secondary button and level slider.",
    "Tab and Shift+Tab move keyboard focus to the next or previous widget.",
    "Tabbing to the text field selects it for editing; Enter completes the edit.",
    "The primary button's on_click callback counts clicks in the status readout.",
    "Left and Right Arrow step the focused slider; double-click its thumb to reset to 50%.",
    "T switches the primary-button tooltip between 500 ms and 1.5 s show delays; the tooltip displays its current delay.",
];
const PANEL_BACKGROUND: Color = Color::srgba(0.02, 0.03, 0.07, 0.92);
const PANEL_BORDER_WIDTH: Px = Px(2.0);
const PANEL_FACE_OFFSET: f32 = 0.03;
const PANEL_MAX_HEIGHT: Px = Px(300.0);
const PANEL_MAX_WIDTH: Px = Px(340.0);
const PANEL_PADDING: Px = Px(12.0);
const PANEL_RADIUS: Px = Px(10.0);
const PANEL_TITLE: &str = "Widget Lab";
const PANEL_TITLE_GAP: f32 = 0.05;
const PANEL_TITLE_WORLD_HEIGHT: f32 = 0.085;
const BUTTON_STATUS_ID: &str = "button-status";
const BUTTON_STATUS_IDLE: &str = "none";
const BUTTON_STATUS_MEASURE: &str = "Canceled secondary-button (pointer/cause)";
const CALLBACK_STATUS_ID: &str = "callback-status";
const CALLBACK_STATUS_IDLE: &str = "none";
const CALLBACK_STATUS_MEASURE: &str = "999 clicks on primary-button";
const FOCUS_STATUS_ID: &str = "focus-status";
const FOCUS_STATUS_MEASURE: &str = "secondary-button";
const FOCUS_STATUS_NONE: &str = "none";
const FOCUS_STATUS_UNAVAILABLE: &str = "unavailable";
const PRIMARY_BUTTON_ID: &str = "primary-button";
const POINTER_STATUS_ID: &str = "pointer-status";
const POINTER_STATUS_IDLE: &str = "none";
const POINTER_STATUS_MEASURE: &str = "Pressed secondary-button";
const PRIMARY_TOOLTIP_DEFAULT_BODY: &str = "Show delay: 500 ms";
const PRIMARY_TOOLTIP_DEFAULT_DELAY: Duration = Duration::from_millis(500);
const PRIMARY_TOOLTIP_SLOW_BODY: &str = "Show delay: 1.5 seconds";
const PRIMARY_TOOLTIP_SLOW_DELAY: Duration = Duration::from_millis(1_500);
const SECONDARY_BUTTON_ID: &str = "secondary-button";
const TOGGLE_CONTROL: &str = "D Toggle Disabled";
const SCREEN_ATTACHMENT_ATTACHED_LABEL: &str = "Attached to this button";
const SCREEN_ATTACHMENT_COLOR: Color = Color::srgb(1.0, 0.78, 0.32);
const SCREEN_ATTACHMENT_DETACHED_LABEL: &str = "Detached: screen anchored";
const SCREEN_ATTACHMENT_MAX_HEIGHT: Px = Px(64.0);
const SCREEN_ATTACHMENT_MAX_WIDTH: Px = Px(260.0);
const SCREEN_ATTACHMENT_STATUS_ID: &str = "screen-attachment-status";
const SCREEN_CONTROL_WIDTH: Px = Px(218.0);
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
const SCREEN_TARGET_LABEL: &str = "Toggle attachment";
const STATE_STATUS_ID: &str = "state-status";
const STATE_STATUS_IDLE: &str = "pri=normal sec=normal lvl=normal";
const STATE_STATUS_MEASURE: &str = "pri=pressed,off sec=pressed,off lvl=pressed,off";
const TEXT_FIELD_ID: &str = "editable-text";
const TEXT_FIELD_INITIAL: &str = "Editable text";
const TEXT_FIELD_TEXT: Color = Color::BLACK;
const TOOLTIP_STATUS_ID: &str = "tooltip-status";
const TOOLTIP_STATUS_IDLE: &str = "none";
const TOOLTIP_STATUS_MEASURE: &str = "hidden secondary-button";
const SLIDER_ADJUST_STEPS: f32 = 1.0;
const SLIDER_DISABLED_COLOR: Color = Color::srgba(0.32, 0.34, 0.37, 0.90);
const SLIDER_ID: &str = "level-slider";
const SLIDER_INITIAL_VALUE: f32 = 0.5;
const SLIDER_LABEL_GAP: Px = Px(5.0);
const SLIDER_LABEL_ID: &str = "slider-label";
const SLIDER_LABEL_IDLE: &str = "50%";
const SLIDER_LABEL_TEXT: Color = Color::BLACK;
const SLIDER_RANGE_END: f32 = 1.0;
const SLIDER_RANGE_START: f32 = 0.0;
const SLIDER_REPEAT_INITIAL_DELAY_SECONDS: f32 = 0.4;
const SLIDER_REPEAT_INTERVAL_SECONDS: f32 = 0.15;
const SLIDER_STATUS_ID: &str = "slider-status";
const SLIDER_STATUS_IDLE: &str = "0.50 (50%)";
const SLIDER_STATUS_MEASURE: &str = "0.00 (100%)";
const SLIDER_STEP: f32 = 0.05;
const SLIDER_THUMB_BORDER: Color = Color::srgba(0.62, 1.0, 1.0, 1.0);
const SLIDER_THUMB_DIAMETER: Px = Px(16.0);
const SLIDER_THUMB_FILL: Color = Color::srgba(0.05, 0.90, 1.0, 1.0);
const SLIDER_THUMB_FOCUSED_BORDER: Color = BUTTON_BORDER_FOCUSED;
const SLIDER_THUMB_ID: &str = "slider-thumb";
const SLIDER_THUMB_RADIUS: Px = Px(8.0);
const SLIDER_TRACK_FILL: Color = Color::srgba(0.00, 0.62, 0.78, 0.98);
const SLIDER_TRACK_HEIGHT: Px = Px(5.0);
const SLIDER_TRACK_HOVERED: Color = Color::srgba(0.08, 0.74, 0.90, 0.98);
const SLIDER_TRACK_INSET: Px = SLIDER_THUMB_RADIUS;
const SLIDER_TRACK_RADIUS: Px = Px(2.5);
const STATUS_BACKGROUND: Color = Color::srgba(0.01, 0.06, 0.08, 0.88);
const STATUS_ANCHOR_OFFSET: Px = Px(24.0);
const STATUS_BORDER: Color = Color::srgba(0.20, 0.80, 0.68, 0.86);
const STATUS_BORDER_WIDTH: Px = Px(1.0);
const STATUS_COLUMN_GAP: Px = Px(10.0);
const STATUS_COLOR: Color = Color::srgb(0.38, 0.94, 0.78);
const STATUS_LABEL_COLOR: Color = Color::srgba(0.68, 0.75, 0.88, 0.92);
const STATUS_LABEL_WIDTH: Px = Px(88.0);
const STATUS_LINE_GAP: Px = Px(4.0);
const STATUS_PADDING: Px = Px(6.0);
const STATUS_RADIUS: Px = PANEL_RADIUS;
const STATUS_TEXT_SIZE: Pt = Pt(14.0);
const TOOLTIP_BORDER_WIDTH: Px = Px(5.0);
const TOOLTIP_GAP: Px = Px(20.0);
const TOOLTIP_OFFSET: Px = Px(16.0);
const TOOLTIP_PADDING: Px = Px(30.0);
const TOOLTIP_RADIUS: Px = Px(35.0);
const TOOLTIP_TEXT_SIZE: Pt = Pt(fairy_dust::LABEL_SIZE.0 * 5.0);
/// Width shared by both cube-face readouts. Sizing them by width rather than
/// height keeps their text at one scale: the two trees differ in row count but
/// share their widest row, so an equal world width gives an equal pixel scale.
const WORLD_READOUT_WORLD_WIDTH: f32 = 0.76;

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

/// App-owned record of whether the level slider is currently in a pointer
/// drag. The crate-private drag marker that drives the slider's pressed
/// appearance is not observable, so the app tracks the drag lifecycle through
/// the public `SliderGrabbed`/`SliderReleased`/`SliderCanceled` observers and
/// reports pressed from this record — a captured drag stays pressed even when
/// the pointer aggregate changes.
#[derive(Clone, Copy, Default, Resource)]
enum LevelSliderDrag {
    #[default]
    Idle,
    Grabbed,
}

impl LevelSliderDrag {
    const fn is_grabbed(self) -> bool { matches!(self, Self::Grabbed) }
}

/// Where the `State:` row reads a widget's pressed flag. Buttons read the
/// pointer aggregate; the level slider reads [`LevelSliderDrag`].
#[derive(Clone, Copy)]
enum PressedSource {
    PointerAggregate,
    DragRecord,
}

#[derive(Clone, Copy, Default)]
enum InteractionChanges {
    #[default]
    None,
    Observed,
}

impl InteractionChanges {
    const fn observe(&mut self) { *self = Self::Observed; }

    const fn were_observed(self) -> bool { matches!(self, Self::Observed) }
}

#[derive(Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
enum InteractionPriority {
    #[default]
    None,
    Hovered,
    Pressed,
}

impl From<PickingInteraction> for InteractionPriority {
    fn from(interaction: PickingInteraction) -> Self {
        match interaction {
            PickingInteraction::None => Self::None,
            PickingInteraction::Hovered => Self::Hovered,
            PickingInteraction::Pressed => Self::Pressed,
        }
    }
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

    fn tooltip(self) -> Tooltip {
        let (body, delay) = match self {
            Self::Default => (PRIMARY_TOOLTIP_DEFAULT_BODY, PRIMARY_TOOLTIP_DEFAULT_DELAY),
            Self::Slow => (PRIMARY_TOOLTIP_SLOW_BODY, PRIMARY_TOOLTIP_SLOW_DELAY),
        };
        authored_tooltip("Primary button", body).show_after(delay)
    }

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
struct WidgetLabFocusInitialized;

#[derive(Component)]
struct WidgetInteractionReadout;

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

action!(
    /// App-owned held action that steps the level slider down.
    AppSliderDecrease
);

action!(
    /// App-owned held action that steps the level slider up.
    AppSliderIncrease
);

event!(
    /// App-owned event that invokes the core widget-focus request system.
    AppFocusPreviousEvent
);

struct AppOwnedWidgetInputPlugin;

impl Plugin for AppOwnedWidgetInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_input_context::<AppWidgetInputContext>()
            .add_systems(Startup, spawn_app_widget_input)
            .add_observer(decrease_slider_from_held)
            .add_observer(increase_slider_from_held);
        bind_action_system!(
            app,
            AppFocusPrevious,
            AppFocusPreviousEvent,
            focus_previous_widget
        );
    }
}

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    fairy_dust::sprinkle_example()
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
        .margin(0.375)
        .with_title_bar(
            TitleBar::new()
                .with_title("Widgets")
                .with_anchor(Anchor::TopLeft)
                .control(TOGGLE_CONTROL),
        )
        .wire_chip_to_state::<ToggleMode, _>(TOGGLE_CONTROL, |mode| mode.control_activation())
        .with_description_panel(
            DescriptionPanel::new(PANEL_TITLE)
                .with_anchor(Anchor::BottomLeft)
                .lines(DESCRIPTION_LINES),
        )
        .with_camera_control_panel()
        .init_resource::<ToggleMode>()
        .init_resource::<LevelSliderDrag>()
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
                .border_width(CONTROL_BORDER_WIDTH_FOCUSED),
        ))
        .insert_resource(WidgetDisabledAppearance::new(
            Appearance::new()
                .background(ROOT_FILL_DISABLED)
                .border_color(ROOT_BORDER_DISABLED),
        ))
        .add_plugins((WidgetInputPlugin, AppOwnedWidgetInputPlugin))
        .add_observer(report_button_pressed)
        .add_observer(report_button_released)
        .add_observer(report_button_clicked)
        .add_observer(toggle_screen_widget_attachment)
        .add_observer(report_button_canceled)
        .add_observer(report_widget_focus_changed)
        .add_observer(apply_slider_change)
        .add_observer(report_slider_grabbed)
        .add_observer(report_slider_released)
        .add_observer(report_slider_canceled)
        .add_observer(report_tooltip_shown)
        .add_observer(report_tooltip_hidden)
        .add_systems(Startup, spawn_cascade_face_light)
        .add_systems(PostStartup, spawn_widget_lab)
        .add_systems(
            Update,
            (
                initialize_screen_widget_attachment,
                initialize_widget_lab_focus,
                report_interaction_changes,
                report_presentation_states,
                reset_widget_lab_focus_on_window_blur,
            ),
        )
        .with_shortcut(KeyCode::KeyD, toggle_disabled_widgets)
        .with_shortcut(KeyCode::KeyT, replace_primary_tooltip)
        .run();
}

fn report_button_pressed(
    event: On<ButtonPressed>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    retain_button_status(
        &readouts,
        &mut panel_text,
        format!("Pressed {} ({:?})", event.id, event.pointer_id),
    );
    info!("widgets: {} pressed by {:?}", event.id, event.pointer_id);
}

fn report_button_released(
    event: On<ButtonReleased>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    retain_button_status(
        &readouts,
        &mut panel_text,
        format!("Released {} ({:?})", event.id, event.pointer_id),
    );
    info!("widgets: {} released by {:?}", event.id, event.pointer_id);
}

fn report_button_clicked(
    event: On<ButtonClicked>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    let status = event.pointer_id.as_ref().map_or_else(
        || format!("Clicked {} (semantic)", event.id),
        |pointer_id| format!("Clicked {} ({pointer_id:?})", event.id),
    );
    retain_button_status(&readouts, &mut panel_text, status);
    info!("widgets: {} clicked by {:?}", event.id, event.pointer_id);
}

fn report_button_canceled(
    event: On<ButtonCanceled>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    retain_button_status(
        &readouts,
        &mut panel_text,
        format!(
            "Canceled {} ({:?}, {:?})",
            event.id, event.pointer_id, event.cause
        ),
    );
    info!(
        "widgets: {} canceled for {:?} by {:?}",
        event.id, event.cause, event.pointer_id
    );
}

/// Typed `.on_click` callback for the primary button.
///
/// Installed through `Button::new().on_click(...)` at authoring time; reify
/// registers it once as a tracked system and the plugin's single
/// global `ButtonClicked` observer dispatches it with each completed click.
fn count_primary_click(
    click: In<ButtonClicked>,
    mut clicks: ResMut<PrimaryClicks>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    clicks.0 += 1;
    info!(
        "widgets: on_click callback ran for {} ({:?}), {} total",
        click.id, click.pointer_id, clicks.0
    );
    let Ok(readout) = readouts.single() else {
        return;
    };
    let status = format!("{} clicks on {}", clicks.0, click.id);
    if !panel_text.set_text(readout, &PanelElementId::named(CALLBACK_STATUS_ID), status) {
        warn!("widgets: callback status has not been reified");
    }
}

fn retain_button_status(
    readouts: &Query<Entity, With<WidgetInteractionReadout>>,
    panel_text: &mut PanelText,
    status: String,
) {
    let Ok(readout) = readouts.single() else {
        return;
    };
    if !panel_text.set_text(readout, &PanelElementId::named(BUTTON_STATUS_ID), status) {
        warn!("widgets: button status has not been reified");
    }
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
    let slider_repeat = || {
        Pulse::new(SLIDER_REPEAT_INTERVAL_SECONDS)
            .with_initial_delay(SLIDER_REPEAT_INITIAL_DELAY_SECONDS)
    };
    keybindings.spawn_binding::<AppSliderDecrease, _>(
        spawner,
        (slider_repeat(), bindings![KeyCode::ArrowLeft]),
    );
    keybindings.spawn_binding::<AppSliderIncrease, _>(
        spawner,
        (slider_repeat(), bindings![KeyCode::ArrowRight]),
    );
}

/// Sends one step-down request immediately, then follows [`Pulse`]'s keyboard
/// repeat timing while Left Arrow remains held and the slider has focus.
fn decrease_slider_from_held(
    _: On<Fire<AppSliderDecrease>>,
    panels: Query<Entity, With<WidgetLabPanel>>,
    focused: Query<(), With<WidgetFocused>>,
    reader: PanelWidgetReader,
    mut commands: Commands,
) {
    request_slider_steps(
        &panels,
        &focused,
        &reader,
        -SLIDER_ADJUST_STEPS,
        &mut commands,
    );
}

/// Sends one step-up request immediately, then follows [`Pulse`]'s keyboard
/// repeat timing while Right Arrow remains held and the slider has focus.
fn increase_slider_from_held(
    _: On<Fire<AppSliderIncrease>>,
    panels: Query<Entity, With<WidgetLabPanel>>,
    focused: Query<(), With<WidgetFocused>>,
    reader: PanelWidgetReader,
    mut commands: Commands,
) {
    request_slider_steps(
        &panels,
        &focused,
        &reader,
        SLIDER_ADJUST_STEPS,
        &mut commands,
    );
}

fn request_slider_steps(
    panels: &Query<Entity, With<WidgetLabPanel>>,
    focused: &Query<(), With<WidgetFocused>>,
    reader: &PanelWidgetReader,
    steps: f32,
    commands: &mut Commands,
) {
    let Ok(panel) = panels.single() else {
        return;
    };
    let Some(widget) = reader.entity(panel, &PanelElementId::named(SLIDER_ID)) else {
        warn!("widgets: level slider has not been reified");
        return;
    };
    if focused.get(widget).is_err() {
        return;
    }
    commands.trigger(RequestSliderAdjustment {
        entity:     widget,
        adjustment: SliderAdjustment::RelativeSteps(steps),
    });
}

/// Applies each slider proposal explicitly — the app stays authoritative over
/// the applied value — then mirrors the accepted value into the slider label
/// and the diagnostic readout.
fn apply_slider_change(
    change: On<SliderChangeRequested>,
    mut sliders: Query<&mut SliderState>,
    panels: Query<Entity, With<WidgetLabPanel>>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    mut panel_text: PanelText,
) {
    let Ok(mut state) = sliders.get_mut(change.event_target()) else {
        return;
    };
    match state.bypass_change_detection().set_value(change.value) {
        Ok(true) => state.set_changed(),
        Ok(false) => return,
        Err(error) => {
            warn!("widgets: rejected slider proposal: {error}");
            return;
        },
    }
    let range = state.range();
    let span = range.end() - range.start();
    let percent = (state.value() - range.start()) / span * 100.0;
    if let Ok(panel) = panels.single()
        && !panel_text.set_text(
            panel,
            &PanelElementId::named(SLIDER_LABEL_ID),
            format!("{percent:.0}%"),
        )
    {
        warn!("widgets: slider label has not been reified");
    }
    let Ok(readout) = readouts.single() else {
        return;
    };
    if !panel_text.set_text(
        readout,
        &PanelElementId::named(SLIDER_STATUS_ID),
        format!("{:.2} ({percent:.0}%)", state.value()),
    ) {
        warn!("widgets: slider status has not been reified");
    }
}

/// Records the grab and logs the pointer grab that begins a drag; the drag's
/// proposals move the value through [`apply_slider_change`].
fn report_slider_grabbed(event: On<SliderGrabbed>, mut drag: ResMut<LevelSliderDrag>) {
    if event.id.as_str() == Some(SLIDER_ID) {
        *drag = LevelSliderDrag::Grabbed;
    }
    info!("widgets: {} grabbed by {:?}", event.id, event.pointer_id);
}

/// Clears the grab record and logs the valid pointer release that completes a
/// drag.
fn report_slider_released(event: On<SliderReleased>, mut drag: ResMut<LevelSliderDrag>) {
    if event.id.as_str() == Some(SLIDER_ID) {
        *drag = LevelSliderDrag::Idle;
    }
    info!("widgets: {} released by {:?}", event.id, event.pointer_id);
}

/// Clears the grab record and logs a drag that ended without a valid release —
/// projection loss, disable, an explicit cancel, or teardown.
fn report_slider_canceled(event: On<SliderCanceled>, mut drag: ResMut<LevelSliderDrag>) {
    if event.id.as_str() == Some(SLIDER_ID) {
        *drag = LevelSliderDrag::Idle;
    }
    let cause = match event.cause {
        SliderCancelCause::PointerCanceled => "pointer canceled",
        SliderCancelCause::PointerRemoved => "pointer removed",
        SliderCancelCause::CaptureLost => "capture lost",
        SliderCancelCause::Disabled => "disabled",
        SliderCancelCause::ProjectionUnavailable => "projection unavailable",
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
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    targets: Query<&TooltipFor>,
    widgets: Query<&PanelWidget>,
    mut panel_text: PanelText,
) {
    report_tooltip_visibility(
        event.entity,
        "shown",
        &readouts,
        &targets,
        &widgets,
        &mut panel_text,
    );
}

fn report_tooltip_hidden(
    event: On<TooltipHidden>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    targets: Query<&TooltipFor>,
    widgets: Query<&PanelWidget>,
    mut panel_text: PanelText,
) {
    report_tooltip_visibility(
        event.entity,
        "hidden",
        &readouts,
        &targets,
        &widgets,
        &mut panel_text,
    );
}

fn report_tooltip_visibility(
    tooltip: Entity,
    visibility: &str,
    readouts: &Query<Entity, With<WidgetInteractionReadout>>,
    targets: &Query<&TooltipFor>,
    widgets: &Query<&PanelWidget>,
    panel_text: &mut PanelText,
) {
    let target = targets.get(tooltip).map_or_else(
        |_| "unavailable".to_owned(),
        |target| {
            widgets.get(target.target()).map_or_else(
                |_| format!("{:?}", target.target()),
                |widget| widget.id().to_string(),
            )
        },
    );
    info!("widgets: tooltip {tooltip:?} {visibility} for {target}");
    let Ok(readout) = readouts.single() else {
        return;
    };
    if !panel_text.set_text(
        readout,
        &PanelElementId::named(TOOLTIP_STATUS_ID),
        format!("{visibility} {target}"),
    ) {
        warn!("widgets: tooltip status has not been reified");
    }
}

fn report_widget_focus_changed(
    change: On<WidgetFocusChanged>,
    readouts: Query<Entity, With<WidgetInteractionReadout>>,
    widgets: Query<&PanelWidget>,
    mut panel_text: PanelText,
) {
    info!(
        "widgets: window {:?} focus changed from {:?} to {:?} ({:?})",
        change.window, change.previous, change.current, change.cause
    );

    let focus_status = match change.current {
        Some(entity) => widgets.get(entity).map_or_else(
            |_| FOCUS_STATUS_UNAVAILABLE.to_owned(),
            |widget| widget.id().to_string(),
        ),
        None => FOCUS_STATUS_NONE.to_owned(),
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
    reader: PanelWidgetReader,
    panel_entities: PanelEntityReader,
) {
    let id = PanelElementId::named(SCREEN_TARGET_ID);
    let Some(owner) = panel_entities.screen(*panel) else {
        return;
    };
    let Some(source) = panel_entities.screen(*card) else {
        return;
    };
    let Some(widget) = reader.typed_entity(owner, &id) else {
        return;
    };
    commands.attach_to_widget(source, widget, screen_widget_attachment());
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
    reader: PanelWidgetReader,
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
    let Some(widget) = reader.typed_entity(owner, &PanelElementId::named(SCREEN_TARGET_ID)) else {
        return;
    };
    let next = state.toggled();
    if let Err(error) = commands.set_tree(card, screen_attachment_status_tree(next)) {
        warn!("widgets: failed to update screen attachment status: {error}");
        return;
    }
    match next {
        ScreenWidgetAttachmentState::Attached => {
            commands.attach_to_widget(source, widget, screen_widget_attachment());
        },
        ScreenWidgetAttachmentState::Detached => commands.detach(source),
    }
    *state = next;
}

fn screen_widget_attachment() -> PanelAttachment {
    PanelAttachment::new(Anchor::TopRight, Anchor::BottomRight)
        .with_offset(PanelAnchorOffset::new(Px(0.0), STATUS_ANCHOR_OFFSET))
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
    windows: Query<&Window, (With<PrimaryWindow>, Changed<Window>)>,
    panels: Query<Entity, (With<WidgetLabPanel>, With<WidgetLabFocusInitialized>)>,
    mut commands: Commands,
) {
    if windows.iter().any(|window| !window.focused) {
        for panel in panels.iter() {
            commands.entity(panel).remove::<WidgetLabFocusInitialized>();
        }
    }
}

fn spawn_widget_lab(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cube: Single<Entity, With<FairyDustCube>>,
    primary_tooltip_timing: Res<PrimaryTooltipTiming>,
    slider_tooltip: Res<SliderTooltipBlueprint>,
) {
    let cube_target = commands.mesh_anchor_target(*cube, MeshFace::PositiveZ);
    commands.spawn_tooltip(cube_target, cube_tooltip());
    let slider = slider_declaration();
    let material = materials.add(cube_face_panel_material());
    let panel = DiegeticPanel::world()
        .size(
            FitMax(PANEL_MAX_WIDTH.into()),
            FitMax(PANEL_MAX_HEIGHT.into()),
        )
        .world_width(CUBE_HALF_USABLE_WIDTH)
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
                .border_width(CONTROL_BORDER_WIDTH_FOCUSED),
        )
        .widget_disabled_appearance(
            Appearance::new()
                .background(BUTTON_FILL_DISABLED)
                .border_color(BUTTON_BORDER_DISABLED),
        )
        .with_tree(widget_tree(
            slider,
            primary_tooltip_timing.tooltip(),
            slider_tooltip.0.clone(),
        ))
        .build();
    let readout = DiegeticPanel::world()
        .size(Fit, Fit)
        .world_width(WORLD_READOUT_WORLD_WIDTH)
        .anchor(Anchor::BottomLeft)
        .shadow_casting(ShadowCasting::Off)
        .material(material.clone())
        .text_material(material.clone())
        .with_tree(interaction_status_tree())
        .build();
    match panel {
        Ok(panel) => {
            commands.entity(*cube).with_children(|cube| {
                cube.spawn((
                    Name::new("Widget lab title"),
                    DiegeticText::world(PANEL_TITLE)
                        .size(fairy_dust::TITLE_SIZE)
                        .color(fairy_dust::TITLE_COLOR)
                        .shadow_mode(GlyphShadowMode::None)
                        .anchor(Anchor::BottomLeft)
                        .world_height(PANEL_TITLE_WORLD_HEIGHT)
                        .transform(widget_title_transform())
                        .build(),
                ));
                cube.spawn((
                    Name::new("Widget lab panel"),
                    WidgetLabPanel,
                    panel,
                    widget_panel_transform(),
                ));
                match readout {
                    Ok(readout) => {
                        cube.spawn((
                            Name::new("Widget interaction readout"),
                            WidgetInteractionReadout,
                            readout,
                            interaction_status_transform(),
                        ));
                    },
                    Err(error) => error!("widgets: failed to build interaction readout: {error}"),
                }
            });
        },
        Err(error) => error!("widgets: failed to build widget panel: {error}"),
    }

    spawn_cascade_widget_lab(
        &mut commands,
        *cube,
        material.clone(),
        primary_tooltip_timing.tooltip(),
        slider_tooltip.0.clone(),
    );
    spawn_screen_widget_lab(&mut commands, material);
}

/// Lights the cube's right face, which the studio rig's key light leaves in
/// shadow. Shadow casting is off so this light only raises the face's exposure.
fn spawn_cascade_face_light(mut commands: Commands) {
    commands.spawn((
        Name::new("Cascade face light"),
        DirectionalLight {
            illuminance: CASCADE_FACE_LIGHT_ILLUMINANCE,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_translation(CASCADE_FACE_LIGHT_POS)
            .looking_at(fairy_dust::example_cube_on_ground(CUBE_CLEARANCE), Vec3::Y),
    ));
}

/// Spawns the right face's widget lab. It declares the same tree as the front
/// face and authors no state appearance at any level, so its hover, press,
/// focus, and disabled looks all resolve from the four attribute resources
/// inserted in `main`. The front face overrides those same resources on its
/// panel, which is what makes the two faces differ on screen.
fn spawn_cascade_widget_lab(
    commands: &mut Commands,
    cube: Entity,
    material: Handle<StandardMaterial>,
    primary_tooltip: Tooltip,
    slider_tooltip: Tooltip,
) {
    let panel = DiegeticPanel::world()
        .size(
            FitMax(PANEL_MAX_WIDTH.into()),
            FitMax(PANEL_MAX_HEIGHT.into()),
        )
        .world_width(CUBE_HALF_USABLE_WIDTH)
        .anchor(Anchor::TopLeft)
        .picking(PanelPicking {
            front: FacePicking::Interactive,
            back:  FacePicking::PanelOnly,
        })
        .shadow_casting(ShadowCasting::Off)
        .material(material.clone())
        .text_material(material.clone())
        .with_tree(widget_tree(
            slider_declaration(),
            primary_tooltip,
            slider_tooltip,
        ))
        .build();
    let readout = DiegeticPanel::world()
        .size(Fit, Fit)
        .world_width(WORLD_READOUT_WORLD_WIDTH)
        .anchor(Anchor::BottomLeft)
        .shadow_casting(ShadowCasting::Off)
        .material(material.clone())
        .text_material(material)
        .with_tree(cascade_status_tree())
        .build();
    match panel {
        Ok(panel) => {
            commands.entity(cube).with_children(|cube| {
                cube.spawn((
                    Name::new("Cascade lab title"),
                    DiegeticText::world(CASCADE_PANEL_TITLE)
                        .size(fairy_dust::TITLE_SIZE)
                        .color(fairy_dust::TITLE_COLOR)
                        .shadow_mode(GlyphShadowMode::None)
                        .anchor(Anchor::BottomLeft)
                        .world_height(PANEL_TITLE_WORLD_HEIGHT)
                        .transform(cascade_title_transform())
                        .build(),
                ));
                cube.spawn((
                    Name::new("Cascade lab panel"),
                    CascadeWidgetLabPanel,
                    panel,
                    cascade_panel_transform(),
                ));
                match readout {
                    Ok(readout) => {
                        cube.spawn((
                            Name::new("Cascade interaction readout"),
                            CascadeInteractionReadout,
                            readout,
                            cascade_status_transform(),
                        ));
                    },
                    Err(error) => {
                        error!("widgets: failed to build cascade readout: {error}");
                    },
                }
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
                .border_width(CONTROL_BORDER_WIDTH_FOCUSED),
        )
        .widget_disabled_appearance(
            Appearance::new()
                .background(BUTTON_FILL_DISABLED)
                .border_color(BUTTON_BORDER_DISABLED),
        )
        .with_tree(screen_widget_tree())
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

fn screen_widget_tree() -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(PANEL_PADDING))
            .gap(CONTROL_GAP)
            .background(PANEL_BACKGROUND)
            .border(Border::all(PANEL_BORDER_WIDTH, SCREEN_ATTACHMENT_COLOR))
            .corner_radius(CornerRadius::all(PANEL_RADIUS)),
    );
    builder.text((
        "Screen widget attachment",
        TextStyle::new(fairy_dust::LABEL_SIZE).with_color(SCREEN_ATTACHMENT_COLOR),
    ));
    add_button(
        &mut builder,
        SCREEN_TARGET_ID,
        SCREEN_TARGET_LABEL,
        SCREEN_CONTROL_WIDTH,
        None,
        |element| element,
    );
    builder.build()
}

fn screen_attachment_status_tree(state: ScreenWidgetAttachmentState) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(STATUS_PADDING))
            .alignment(AlignX::Center, AlignY::Center)
            .background(STATUS_BACKGROUND)
            .border(Border::all(STATUS_BORDER_WIDTH, SCREEN_ATTACHMENT_COLOR))
            .corner_radius(CornerRadius::all(STATUS_RADIUS)),
    );
    builder.text(
        Text::new(
            state.label(),
            TextStyle::new(fairy_dust::LABEL_SIZE).with_color(SCREEN_ATTACHMENT_COLOR),
        )
        .id(SCREEN_ATTACHMENT_STATUS_ID),
    );
    builder.build()
}

/// Builds the slider away from the layout chain so the tree below can show the
/// pre-built [`El::widget`] path next to the inline `El::button` one.
fn slider_declaration() -> Slider {
    Slider::new(SLIDER_RANGE_START..=SLIDER_RANGE_END)
        .value(SLIDER_INITIAL_VALUE)
        .step(SLIDER_STEP)
        .direction(SliderDirection::LeftToRight)
        .reset_behavior(SliderResetBehavior::DoubleClick)
}

fn widget_tree(slider: Slider, primary_tooltip: Tooltip, slider_tooltip: Tooltip) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(CONTROL_GAP),
    );
    add_editable_text(&mut builder);
    add_button(
        &mut builder,
        PRIMARY_BUTTON_ID,
        "Primary button",
        CONTROL_WIDTH,
        Some(primary_tooltip),
        |element| element.on_click(count_primary_click),
    );
    add_button(
        &mut builder,
        SECONDARY_BUTTON_ID,
        "Secondary button",
        CONTROL_WIDTH,
        Some(authored_tooltip(
            "Secondary button",
            "Press D to toggle this control and the slider",
        )),
        |element| element,
    );
    add_slider(&mut builder, slider, slider_tooltip);
    builder.build()
}

fn add_slider(builder: &mut LayoutBuilder, slider: Slider, slider_tooltip: Tooltip) {
    builder.with(
        El::column()
            .width(Sizing::fixed(CONTROL_WIDTH))
            .height(Sizing::FIT)
            .gap(SLIDER_LABEL_GAP)
            .alignment(AlignX::Center, AlignY::Center)
            .widget(SLIDER_ID, slider)
            .disabled(TextColor(SLIDER_DISABLED_COLOR))
            .tooltip(slider_tooltip),
        |builder| {
            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    builder.text(
                        Text::new(
                            SLIDER_LABEL_IDLE,
                            TextStyle::new(fairy_dust::LABEL_SIZE)
                                .with_color(SLIDER_LABEL_TEXT)
                                .with_align(TextAlign::Center),
                        )
                        .id(SLIDER_LABEL_ID)
                        .layout(El::new().width(Sizing::FIT).height(Sizing::FIT)),
                    );
                },
            );

            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(SLIDER_THUMB_DIAMETER)),
                |builder| {
                    builder.with(
                        El::overlay()
                            .width(Sizing::GROW)
                            .height(Sizing::fixed(SLIDER_THUMB_DIAMETER))
                            .padding(Padding::xy(SLIDER_TRACK_INSET, Px(0.0)))
                            .alignment(AlignX::Center, AlignY::Center),
                        |builder| {
                            builder.with(
                                builder
                                    .child(El::new())
                                    .width(Sizing::GROW)
                                    .height(Sizing::fixed(SLIDER_TRACK_HEIGHT))
                                    .background(SLIDER_TRACK_FILL)
                                    .hovered(BackgroundColor(SLIDER_TRACK_HOVERED))
                                    .disabled(BackgroundColor(SLIDER_DISABLED_COLOR))
                                    .corner_radius(CornerRadius::all(SLIDER_TRACK_RADIUS)),
                                |_| {},
                            );
                        },
                    );
                    builder.with(
                        El::overlay()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .alignment(AlignX::Left, AlignY::Center),
                        |builder| {
                            builder.with(
                                builder
                                    .child(El::new())
                                    .size(SLIDER_THUMB_DIAMETER, SLIDER_THUMB_DIAMETER)
                                    .background(SLIDER_THUMB_FILL)
                                    .border(Border::all(CONTROL_BORDER_WIDTH, SLIDER_THUMB_BORDER))
                                    .corner_radius(CornerRadius::all(SLIDER_THUMB_RADIUS))
                                    .id(SLIDER_THUMB_ID)
                                    .slider_thumb()
                                    .hovered(BackgroundColor(SLIDER_THUMB_FILL))
                                    .focused(BorderColor(SLIDER_THUMB_FOCUSED_BORDER))
                                    .disabled(
                                        Appearance::new()
                                            .background(SLIDER_DISABLED_COLOR)
                                            .border_color(SLIDER_THUMB_BORDER),
                                    ),
                                |_| {},
                            );
                        },
                    );
                },
            );
        },
    );
}

fn add_editable_text(builder: &mut LayoutBuilder) {
    let field = ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
    builder.with(
        El::new()
            .size(CONTROL_WIDTH, BUTTON_HEIGHT)
            .padding(Padding::all(CONTROL_PADDING))
            .alignment(AlignX::Center, AlignY::Center)
            .editable_field(TEXT_FIELD_ID, field)
            .editor_text(EditorStateColors::new().focused(CONTROL_TEXT))
            .editor_selection(EditorStateColors::new().focused(BUTTON_FILL_HOVERED))
            .editor_caret(EditorStateColors::new().focused(BUTTON_BORDER_FOCUSED))
            .editor_validation(EditorStateColors::new().focused(BUTTON_BORDER_FOCUSED))
            .tooltip(authored_tooltip(
                "Editable text",
                "Tab selects all; Enter or double-click edits",
            )),
        |builder| {
            builder.text((
                TEXT_FIELD_INITIAL,
                TextStyle::new(fairy_dust::LABEL_SIZE).with_color(TEXT_FIELD_TEXT),
            ));
        },
    );
}

/// The right face's readout. It carries only the `State:` row, which is the
/// row that names which state each widget is in — the states whose look the
/// right face resolves entirely from the attribute resources.
fn cascade_status_tree() -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(STATUS_PADDING))
            .gap(STATUS_LINE_GAP)
            .alignment(AlignX::Left, AlignY::Center)
            .background(STATUS_BACKGROUND)
            .border(Border::all(STATUS_BORDER_WIDTH, STATUS_BORDER))
            .corner_radius(CornerRadius::all(STATUS_RADIUS)),
    );
    interaction_status_row(
        &mut builder,
        "State",
        STATE_STATUS_ID,
        STATE_STATUS_IDLE,
        STATE_STATUS_MEASURE,
    );
    builder.build()
}

fn interaction_status_tree() -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(STATUS_PADDING))
            .gap(STATUS_LINE_GAP)
            .alignment(AlignX::Left, AlignY::Center)
            .background(STATUS_BACKGROUND)
            .border(Border::all(STATUS_BORDER_WIDTH, STATUS_BORDER))
            .corner_radius(CornerRadius::all(STATUS_RADIUS)),
    );
    interaction_status_row(
        &mut builder,
        "Pointer",
        POINTER_STATUS_ID,
        POINTER_STATUS_IDLE,
        POINTER_STATUS_MEASURE,
    );
    interaction_status_row(
        &mut builder,
        "Focus",
        FOCUS_STATUS_ID,
        FOCUS_STATUS_NONE,
        FOCUS_STATUS_MEASURE,
    );
    interaction_status_row(
        &mut builder,
        "Button",
        BUTTON_STATUS_ID,
        BUTTON_STATUS_IDLE,
        BUTTON_STATUS_MEASURE,
    );
    interaction_status_row(
        &mut builder,
        "Callback",
        CALLBACK_STATUS_ID,
        CALLBACK_STATUS_IDLE,
        CALLBACK_STATUS_MEASURE,
    );
    interaction_status_row(
        &mut builder,
        "State",
        STATE_STATUS_ID,
        STATE_STATUS_IDLE,
        STATE_STATUS_MEASURE,
    );
    interaction_status_row(
        &mut builder,
        "Slider",
        SLIDER_STATUS_ID,
        SLIDER_STATUS_IDLE,
        SLIDER_STATUS_MEASURE,
    );
    interaction_status_row(
        &mut builder,
        "Tooltip",
        TOOLTIP_STATUS_ID,
        TOOLTIP_STATUS_IDLE,
        TOOLTIP_STATUS_MEASURE,
    );
    builder.build()
}

fn interaction_status_row(
    builder: &mut LayoutBuilder,
    label: &'static str,
    id: &'static str,
    value: &'static str,
    measure_as: &'static str,
) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(STATUS_COLUMN_GAP)
            .alignment(AlignX::Left, AlignY::Center),
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
        .with_shadow_mode(GlyphShadowMode::None)
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

fn cascade_title_transform() -> Transform {
    let half_extent = fairy_dust::EXAMPLE_CUBE_SIZE * 0.5;
    cube_right_transform(Vec2::new(
        -half_extent + CUBE_EDGE_INSET,
        half_extent + PANEL_TITLE_GAP,
    ))
}

fn cascade_panel_transform() -> Transform {
    let half_extent = fairy_dust::EXAMPLE_CUBE_SIZE * 0.5;
    cube_right_transform(Vec2::new(
        -half_extent + CUBE_EDGE_INSET,
        half_extent - CUBE_EDGE_INSET,
    ))
}

fn cascade_status_transform() -> Transform {
    let half_extent = fairy_dust::EXAMPLE_CUBE_SIZE * 0.5;
    cube_right_transform(Vec2::new(
        -half_extent + CUBE_EDGE_INSET,
        -half_extent + CUBE_EDGE_INSET,
    ))
}

fn widget_title_transform() -> Transform {
    let half_extent = fairy_dust::EXAMPLE_CUBE_SIZE * 0.5;
    cube_front_transform(Vec2::new(
        -half_extent + CUBE_EDGE_INSET,
        half_extent + PANEL_TITLE_GAP,
    ))
}

fn widget_panel_transform() -> Transform {
    let half_extent = fairy_dust::EXAMPLE_CUBE_SIZE * 0.5;
    cube_front_transform(Vec2::new(
        -half_extent + CUBE_EDGE_INSET,
        half_extent - CUBE_EDGE_INSET,
    ))
}

fn interaction_status_transform() -> Transform {
    let half_extent = fairy_dust::EXAMPLE_CUBE_SIZE * 0.5;
    cube_front_transform(Vec2::new(
        -half_extent + CUBE_EDGE_INSET,
        -half_extent + CUBE_EDGE_INSET,
    ))
}

/// A button element in the widget lab's row layout.
type ButtonElement = El<Row, WidgetElement<Button>>;

fn add_button(
    builder: &mut LayoutBuilder,
    id: &'static str,
    label: &'static str,
    width: Px,
    tooltip: Option<Tooltip>,
    configure: impl FnOnce(ButtonElement) -> ButtonElement,
) {
    let element = configure(
        El::new()
            .size(width, BUTTON_HEIGHT)
            .padding(Padding::all(CONTROL_PADDING))
            .alignment(AlignX::Center, AlignY::Center)
            .background(BUTTON_FILL)
            .border(Border::all(CONTROL_BORDER_WIDTH, BUTTON_BORDER))
            .corner_radius(CornerRadius::all(CONTROL_RADIUS))
            .button(id),
    );
    let element = match tooltip {
        Some(tooltip) => element.tooltip(tooltip),
        None => element,
    };
    builder.with(element, |builder| {
        builder.text((
            label,
            TextStyle::new(fairy_dust::LABEL_SIZE).with_color(CONTROL_TEXT),
        ));
    });
}

fn slider_tooltip() -> Tooltip {
    authored_tooltip(
        "Level slider",
        "Drag or use Left/Right Arrow; double-click the thumb to reset to 50%",
    )
}

fn cube_tooltip() -> Tooltip {
    authored_tooltip("Widget cube", "Standalone PositiveZ mesh-face target")
        .placement_policy(TooltipPlacementPolicy::Fixed)
}

fn authored_tooltip(title: &'static str, body: &'static str) -> Tooltip {
    let mut tooltip = Tooltip::new(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(TOOLTIP_PADDING))
            .gap(TOOLTIP_GAP)
            .shadow_casting(ShadowCasting::Off)
            .background(STATUS_BACKGROUND)
            .border(Border::all(TOOLTIP_BORDER_WIDTH, STATUS_BORDER))
            .corner_radius(CornerRadius::all(TOOLTIP_RADIUS)),
    )
    .source_anchor(Anchor::CenterLeft)
    .target_anchor(Anchor::CenterRight)
    .offset(PanelAnchorOffset::new(TOOLTIP_OFFSET, Px(0.0)));
    tooltip.text((
        title,
        TextStyle::new(TOOLTIP_TEXT_SIZE).with_color(CONTROL_TEXT),
    ));
    tooltip.text((
        body,
        TextStyle::new(TOOLTIP_TEXT_SIZE).with_color(STATUS_COLOR),
    ));
    tooltip
}

fn report_interaction_changes(
    panel: Single<Entity, With<WidgetLabPanel>>,
    screen_panel: Single<Entity, With<ScreenWidgetLabPanel>>,
    readout: Single<Entity, With<WidgetInteractionReadout>>,
    widgets: Query<(&PanelWidget, &WidgetOf, Ref<PickingInteraction>)>,
    mut panel_text: PanelText,
) {
    let mut world_interaction_changes = InteractionChanges::None;
    let mut active_priority = InteractionPriority::None;
    let mut active_status = None;

    for (widget, widget_of, interaction) in &widgets {
        let owner = widget_of.panel();
        if owner != *panel && owner != *screen_panel {
            continue;
        }
        let interaction_changed = interaction.is_changed();
        if interaction_changed {
            info!(
                "widgets: {} interaction changed to {:?}",
                widget.id(),
                *interaction
            );
        }
        if owner != *panel {
            continue;
        }
        if interaction_changed {
            world_interaction_changes.observe();
        }

        let priority = InteractionPriority::from(*interaction);
        if priority > active_priority {
            active_priority = priority;
            active_status = Some(format!("{:?} {}", *interaction, widget.id()));
        }
    }

    if world_interaction_changes.were_observed()
        && !panel_text.set_text(
            *readout,
            &PanelElementId::named(POINTER_STATUS_ID),
            active_status.as_deref().unwrap_or(POINTER_STATUS_IDLE),
        )
    {
        warn!("widgets: interaction status has not been reified");
    }
}

/// Mirrors each world button's and the level slider's presentation inputs into
/// the `State:` diagnostic row. Hover and availability read `PickingInteraction`
/// and `WidgetDisabled`; the buttons' pressed flag reads the pointer aggregate
/// while the slider's reads [`LevelSliderDrag`], so a captured drag stays
/// pressed even when the aggregate changes. The separate `Focus:` row reports
/// the retained keyboard target, so slider focus stays visible there while
/// hover, press, and disabled show here.
fn report_presentation_states(
    panel: Single<Entity, With<WidgetLabPanel>>,
    readout: Single<Entity, With<WidgetInteractionReadout>>,
    cascade_panel: Single<Entity, With<CascadeWidgetLabPanel>>,
    cascade_readout: Single<Entity, With<CascadeInteractionReadout>>,
    widgets: Query<(
        &PanelWidget,
        &WidgetOf,
        Option<&PickingInteraction>,
        Has<WidgetDisabled>,
    )>,
    drag: Res<LevelSliderDrag>,
    mut panel_text: PanelText,
) {
    let flags = |panel: Entity, id: &str, pressed_source: PressedSource| {
        widgets
            .iter()
            .find(|(widget, widget_of, ..)| {
                widget_of.panel() == panel && *widget.id() == PanelElementId::named(id)
            })
            .map_or_else(
                || "?".to_owned(),
                |(_, _, interaction, disabled)| {
                    let pressed = match pressed_source {
                        PressedSource::PointerAggregate => {
                            matches!(interaction, Some(PickingInteraction::Pressed))
                        },
                        PressedSource::DragRecord => drag.is_grabbed(),
                    };
                    let mut parts = Vec::new();
                    if pressed {
                        parts.push("pressed");
                    } else if matches!(
                        interaction,
                        Some(PickingInteraction::Hovered | PickingInteraction::Pressed)
                    ) {
                        // A slider whose drag record is idle takes the hover layer
                        // from a generic pressed aggregate, matching production
                        // `present_slider_state`. A button's own pressed aggregate
                        // is already reported above, so it never reaches here.
                        parts.push("hover");
                    }
                    if disabled {
                        parts.push("off");
                    }
                    if parts.is_empty() {
                        "normal".to_owned()
                    } else {
                        parts.join(",")
                    }
                },
            )
    };
    let status = |panel: Entity| {
        format!(
            "pri={} sec={} lvl={}",
            flags(panel, PRIMARY_BUTTON_ID, PressedSource::PointerAggregate),
            flags(panel, SECONDARY_BUTTON_ID, PressedSource::PointerAggregate),
            flags(panel, SLIDER_ID, PressedSource::DragRecord)
        )
    };
    // `PanelText` skips the layout revision bump for an unchanged string, so
    // writing every frame stays free of relayout work.
    panel_text.set_text(
        *readout,
        &PanelElementId::named(STATE_STATUS_ID),
        status(*panel),
    );
    panel_text.set_text(
        *cascade_readout,
        &PanelElementId::named(STATE_STATUS_ID),
        status(*cascade_panel),
    );
}

/// Applies `interactivity` to one panel's secondary button and level slider,
/// reporting whether both were reified and updated.
fn set_toggled_interactivity(
    panel: Entity,
    reader: &PanelWidgetReader,
    writer: &mut PanelWidgetWriter,
    interactivity: WidgetInteractivity,
) -> bool {
    let secondary = reader.entity(panel, &PanelElementId::named(SECONDARY_BUTTON_ID));
    let slider = reader.entity(panel, &PanelElementId::named(SLIDER_ID));
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
    cascade_panel: Single<Entity, With<CascadeWidgetLabPanel>>,
    reader: PanelWidgetReader,
    mut writer: PanelWidgetWriter,
    mut mode: ResMut<ToggleMode>,
) {
    let next = mode.toggled();
    let front_toggled =
        set_toggled_interactivity(*panel, &reader, &mut writer, next.interactivity());
    let cascade_toggled =
        set_toggled_interactivity(*cascade_panel, &reader, &mut writer, next.interactivity());
    if front_toggled && cascade_toggled {
        *mode = next;
        info!(
            "widgets: both faces' secondary buttons and level sliders are now {:?}",
            next.interactivity()
        );
    } else {
        warn!("widgets: failed to update secondary button or level slider interactivity");
    }
}

fn replace_primary_tooltip(
    panel: Single<Entity, With<WidgetLabPanel>>,
    mut primary_tooltip_timing: ResMut<PrimaryTooltipTiming>,
    slider_tooltip: Res<SliderTooltipBlueprint>,
    mut commands: Commands,
) {
    let next_timing = primary_tooltip_timing.toggled();
    let slider = slider_declaration();
    match commands.set_tree(
        *panel,
        widget_tree(slider, next_timing.tooltip(), slider_tooltip.0.clone()),
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
