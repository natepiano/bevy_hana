//! Bidirectional screen/world panel conversion example.
//!
//! Controls:
//!   1 - Animate the world-authored panel into screen space.
//!   2 - Animate the world-authored panel back to its original world position.
//!   3 - Convert the world-authored panel from screen to world in place.
//!   4 - Jump the world-authored panel to screen space.
//!   5 - Jump the world-authored panel back to its original world position.
//!   A - Animate the screen-authored panel into world space.
//!   B - Animate the screen-authored panel back to screen space.
//!   C - Jump the screen-authored panel into world space.
//!   D - Jump the screen-authored panel back to screen space.
//!   H - Return the camera home.

use bevy::ecs::system::SystemParam;
use bevy::light::NotShadowReceiver;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Name;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::error;
use bevy::prelude::info;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use bevy::window::Window;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::TitleBar;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material;
use hana_diegetic::Anchor;
use hana_diegetic::CoordinateSpace;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::Dimension;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelBuildError;
use hana_diegetic::PanelEntityReader;
use hana_diegetic::PanelProjectionParam;
use hana_diegetic::PanelScreenConversion;
use hana_diegetic::PanelScreenTarget;
use hana_diegetic::PanelSystems;
use hana_diegetic::PanelWorldTarget;
use hana_diegetic::Px;
use hana_diegetic::SavedPanelWorldState;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_diegetic::default_panel_material;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamPreset;

const HOME_YAW: f32 = 0.4;
const HOME_PITCH: f32 = 0.35;
const HOME_MARGIN: f32 = 0.55;
const WORLD_PANEL_WIDTH_MM: f32 = 180.0;
const WORLD_PANEL_HEIGHT_MM: f32 = 78.0;
const WORLD_PANEL_HEIGHT: f32 = 0.7;
const SCREEN_PANEL_WIDTH_PX: f32 = 270.0;
const PANEL_TEXT_SIZE_MM: f32 = 7.0;
const SCREEN_TEXT_SIZE_PX: f32 = 18.0;
const PANEL_PADDING_MM: f32 = 8.0;
const PANEL_GAP_MM: f32 = 2.0;
const SCREEN_GAP_PX: f32 = 7.0;
const PANEL_RADIUS_MM: f32 = 4.0;
const WORLD_SCREEN_HEIGHT_FRACTION: f32 = 0.26;
const SCREEN_WORLD_HEIGHT: f32 = 0.36;
const TRANSITION_SECONDS: f32 = 0.82;
const TEXT_COLOR: Color = Color::srgb(0.94, 0.95, 1.0);
const DISABLED_TEXT_COLOR: Color = Color::srgba(0.52, 0.55, 0.64, 0.62);
const ACTION_HIGHLIGHT: Color = Color::srgba(1.0, 0.82, 0.18, 0.34);
const PANEL_FILL_BACKGROUND: Color = Color::srgba(0.27, 0.33, 0.43, 0.94);
const WORLD_PANEL_COLOR: Color = Color::srgb(0.16, 0.13, 0.22);
const SPACE_WORLD_HIGHLIGHT: Color = Color::srgba(0.25, 0.78, 0.46, 0.34);
const SPACE_SCREEN_HIGHLIGHT: Color = Color::srgba(0.26, 0.66, 1.0, 0.36);

#[derive(Component)]
struct WorldAuthoredPanel;

#[derive(Component)]
struct ScreenAuthoredPanel;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PanelSpace {
    World,
    Screen,
}

impl PanelSpace {
    const fn from_panel(panel: &DiegeticPanel) -> Self {
        if panel.coordinate_space().is_screen() {
            Self::Screen
        } else {
            Self::World
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::World => "WORLD",
            Self::Screen => "SCREEN",
        }
    }

    const fn highlight(self) -> Color {
        match self {
            Self::World => SPACE_WORLD_HIGHLIGHT,
            Self::Screen => SPACE_SCREEN_HIGHLIGHT,
        }
    }
}

#[derive(Default)]
struct DisplayedPanelSpaces {
    world:              Option<PanelSpace>,
    screen:             Option<PanelSpace>,
    world_enabled:      Vec<OperationKey>,
    screen_enabled:     Vec<OperationKey>,
    world_highlighted:  Vec<OperationKey>,
    screen_highlighted: Vec<OperationKey>,
}

#[derive(Clone, Copy)]
struct PanelTreeMetrics {
    unit:             Unit,
    title_size:       f32,
    status_size:      f32,
    detail_size:      f32,
    padding:          f32,
    gap:              f32,
    radius:           f32,
    status_padding_x: f32,
    status_padding_y: f32,
}

impl PanelTreeMetrics {
    fn world(unit: Unit, panel_height: f32) -> Self {
        let scale = (panel_height / WORLD_PANEL_HEIGHT_MM).max(f32::EPSILON);
        Self {
            unit,
            title_size: PANEL_TEXT_SIZE_MM * scale,
            status_size: PANEL_TEXT_SIZE_MM * 0.72 * scale,
            detail_size: PANEL_TEXT_SIZE_MM * 0.76 * scale,
            padding: PANEL_PADDING_MM * scale,
            gap: PANEL_GAP_MM * scale,
            radius: PANEL_RADIUS_MM * scale,
            status_padding_x: PANEL_PADDING_MM * 0.42 * scale,
            status_padding_y: PANEL_PADDING_MM * 0.16 * scale,
        }
    }

    const fn screen() -> Self {
        Self {
            unit:             Unit::Pixels,
            title_size:       SCREEN_TEXT_SIZE_PX,
            status_size:      SCREEN_TEXT_SIZE_PX * 0.72,
            detail_size:      SCREEN_TEXT_SIZE_PX * 0.82,
            padding:          0.0,
            gap:              SCREEN_GAP_PX,
            radius:           4.0,
            status_padding_x: 7.0,
            status_padding_y: 3.0,
        }
    }

    const fn dim(self, value: f32) -> Dimension {
        Dimension {
            value,
            unit: Some(self.unit),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum OperationKey {
    AnimateWorldToScreen,
    AnimateWorldToWorld,
    AnimateScreenToWorld,
    AnimateScreenToScreen,
    ImmediateWorldToScreen,
    ImmediateWorldToWorld,
    ImmediateWorldToOriginal,
    ImmediateScreenToWorld,
    ImmediateScreenToScreen,
}

impl OperationKey {
    const fn label(self) -> &'static str {
        match self {
            Self::AnimateWorldToScreen => "1 - Animate to screen",
            Self::AnimateWorldToWorld => "2 - Animate to original position",
            Self::ImmediateWorldToWorld => "3 - Convert from screen to world in place",
            Self::ImmediateWorldToScreen => "4 - Jump to screen",
            Self::ImmediateWorldToOriginal => "5 - Jump to original position",
            Self::AnimateScreenToWorld => "A - Animate to world",
            Self::AnimateScreenToScreen => "B - Animate to screen",
            Self::ImmediateScreenToWorld => "C - Jump to world",
            Self::ImmediateScreenToScreen => "D - Jump to screen",
        }
    }

    const fn is_world_panel(self) -> bool {
        matches!(
            self,
            Self::AnimateWorldToScreen
                | Self::AnimateWorldToWorld
                | Self::ImmediateWorldToScreen
                | Self::ImmediateWorldToWorld
                | Self::ImmediateWorldToOriginal
        )
    }
}

fn operation_enabled(enabled: &[OperationKey], operation: OperationKey) -> bool {
    enabled.contains(&operation)
}

const fn enabled_text_color(enabled: bool) -> Color {
    if enabled {
        TEXT_COLOR
    } else {
        DISABLED_TEXT_COLOR
    }
}

fn world_available_operations(
    panel: &DiegeticPanel,
    has_saved_state: bool,
    transitioning: bool,
) -> Vec<OperationKey> {
    if transitioning {
        return Vec::new();
    }
    if panel.coordinate_space().is_screen() {
        if has_saved_state {
            vec![
                OperationKey::AnimateWorldToWorld,
                OperationKey::ImmediateWorldToWorld,
                OperationKey::ImmediateWorldToOriginal,
            ]
        } else {
            Vec::new()
        }
    } else {
        vec![
            OperationKey::AnimateWorldToScreen,
            OperationKey::ImmediateWorldToScreen,
        ]
    }
}

fn screen_available_operations(panel: &DiegeticPanel, transitioning: bool) -> Vec<OperationKey> {
    if transitioning {
        return Vec::new();
    }
    if panel.coordinate_space().is_screen() {
        vec![
            OperationKey::AnimateScreenToWorld,
            OperationKey::ImmediateScreenToWorld,
        ]
    } else {
        vec![
            OperationKey::AnimateScreenToScreen,
            OperationKey::ImmediateScreenToScreen,
        ]
    }
}

struct FlashHighlight {
    key:   OperationKey,
    timer: Timer,
}

#[derive(Default, Resource)]
struct OperationHighlights {
    flash: Option<FlashHighlight>,
}

impl OperationHighlights {
    fn flash(&mut self, key: OperationKey) {
        self.flash = Some(FlashHighlight {
            key,
            timer: Timer::from_seconds(1.0, TimerMode::Once),
        });
    }
}

#[derive(Clone, Copy, Component)]
struct WorldPanelHome {
    transform: Transform,
}

#[derive(Clone, Component)]
struct ScreenPanelHome {
    conversion: PanelScreenConversion,
}

#[derive(Debug, Default, Reflect, Resource)]
#[reflect(Resource)]
struct PanelConversionRequest {
    action: u8,
}

impl PanelConversionRequest {
    const NONE: u8 = 0;
    const WORLD_TO_SCREEN: u8 = 1;
    const WORLD_TO_WORLD: u8 = 2;
    const WORLD_TO_WORLD_IMMEDIATE: u8 = 3;
    const WORLD_TO_SCREEN_IMMEDIATE: u8 = 4;
    const WORLD_TO_ORIGINAL_IMMEDIATE: u8 = 5;
    const SCREEN_TO_WORLD: u8 = 11;
    const SCREEN_TO_SCREEN: u8 = 12;
    const SCREEN_TO_WORLD_IMMEDIATE: u8 = 13;
    const SCREEN_TO_SCREEN_IMMEDIATE: u8 = 14;

    const fn request(&mut self, action: u8) { self.action = action; }

    const fn take(&mut self) -> u8 {
        let action = self.action;
        self.action = Self::NONE;
        action
    }
}

#[derive(Resource)]
struct DemoPanels {
    world:  Entity,
    screen: Entity,
}

#[derive(Component)]
struct PanelTransition {
    operation: OperationKey,
    timer:     Timer,
    from:      Transform,
    to:        Transform,
    finish:    FinishAction,
}

impl PanelTransition {
    fn new(operation: OperationKey, from: Transform, to: Transform, finish: FinishAction) -> Self {
        Self {
            operation,
            timer: Timer::from_seconds(TRANSITION_SECONDS, TimerMode::Once),
            from,
            to,
            finish,
        }
    }
}

#[derive(Clone)]
enum FinishAction {
    ApplyScreen {
        camera:     Entity,
        conversion: PanelScreenConversion,
    },
    None,
}

fn main() {
    let app = fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .insert(NotShadowReceiver)
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(TitleBar::new().with_title("Screen/World Panel Conversion"))
        .with_camera_control_panel();
    app.init_resource::<PanelConversionRequest>()
        .init_resource::<OperationHighlights>()
        .add_systems(Startup, spawn_panels)
        .add_systems(
            Update,
            (execute_panel_conversion_requests, animate_panel_transitions)
                .chain()
                .before(PanelSystems::ApplyConversions),
        )
        .add_systems(
            Update,
            update_panel_status_trees
                .after(PanelSystems::ApplyConversions)
                .before(PanelSystems::ComputeLayout),
        )
        .with_shortcut(KeyCode::Digit1, request_world_panel_to_screen)
        .with_shortcut(KeyCode::Digit2, request_world_panel_to_world)
        .with_shortcut(KeyCode::Digit3, request_world_panel_to_world_immediate)
        .with_shortcut(KeyCode::Digit4, request_world_panel_to_screen_immediate)
        .with_shortcut(KeyCode::Digit5, request_world_panel_to_original_immediate)
        .with_shortcut(KeyCode::KeyA, request_screen_panel_to_world)
        .with_shortcut(KeyCode::KeyB, request_screen_panel_to_screen)
        .with_shortcut(KeyCode::KeyC, request_screen_panel_to_world_immediate)
        .with_shortcut(KeyCode::KeyD, request_screen_panel_to_screen_immediate)
        .run();
}

fn spawn_panels(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let world_panel = match build_world_panel(&mut materials) {
        Ok(panel) => panel,
        Err(error) => {
            error!("screen_world_panel_conversion: world panel build failed: {error}");
            return;
        },
    };
    let screen_panel = match build_screen_panel(&mut materials) {
        Ok(panel) => panel,
        Err(error) => {
            error!("screen_world_panel_conversion: screen panel build failed: {error}");
            return;
        },
    };
    let world_transform = saved_world_transform();
    let world = commands
        .spawn((
            Name::new("World-authored conversion panel"),
            WorldAuthoredPanel,
            WorldPanelHome {
                transform: world_transform,
            },
            CameraHomeTarget,
            world_panel,
            world_transform,
            Visibility::default(),
        ))
        .id();
    let screen = commands
        .spawn((
            Name::new("Screen-authored conversion panel"),
            ScreenAuthoredPanel,
            screen_panel,
            Transform::default(),
            Visibility::default(),
        ))
        .id();
    commands.insert_resource(DemoPanels { world, screen });
}

fn request_world_panel_to_screen(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::WORLD_TO_SCREEN);
}

fn request_world_panel_to_world(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::WORLD_TO_WORLD);
}

fn request_screen_panel_to_world(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::SCREEN_TO_WORLD);
}

fn request_screen_panel_to_screen(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::SCREEN_TO_SCREEN);
}

fn request_world_panel_to_screen_immediate(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::WORLD_TO_SCREEN_IMMEDIATE);
}

fn request_world_panel_to_world_immediate(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::WORLD_TO_WORLD_IMMEDIATE);
}

fn request_screen_panel_to_world_immediate(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::SCREEN_TO_WORLD_IMMEDIATE);
}

fn request_world_panel_to_original_immediate(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::WORLD_TO_ORIGINAL_IMMEDIATE);
}

fn request_screen_panel_to_screen_immediate(mut request: ResMut<PanelConversionRequest>) {
    request.request(PanelConversionRequest::SCREEN_TO_SCREEN_IMMEDIATE);
}

#[derive(SystemParam)]
struct ConversionParams<'w, 's> {
    panel_entities: PanelEntityReader<'w, 's>,
    projections:    PanelProjectionParam<'w, 's>,
}

fn execute_panel_conversion_requests(
    panels: Res<DemoPanels>,
    mut request: ResMut<PanelConversionRequest>,
    cameras: Query<(Entity, &GlobalTransform), With<OrbitCam>>,
    world_panels: Query<
        (&DiegeticPanel, &Transform),
        (With<WorldAuthoredPanel>, Without<PanelTransition>),
    >,
    screen_panels: Query<
        (&DiegeticPanel, Option<&ScreenPanelHome>),
        (With<ScreenAuthoredPanel>, Without<PanelTransition>),
    >,
    screen_panels_with_home: Query<
        (&DiegeticPanel, &Transform, &ScreenPanelHome),
        (With<ScreenAuthoredPanel>, Without<PanelTransition>),
    >,
    saved_states: Query<&SavedPanelWorldState>,
    homes: Query<&WorldPanelHome>,
    transitions: Query<(), With<PanelTransition>>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    windows: Query<&Window>,
    conversions: ConversionParams,
    mut highlights: ResMut<OperationHighlights>,
    mut commands: Commands,
) {
    match request.take() {
        PanelConversionRequest::WORLD_TO_SCREEN => animate_world_panel_to_screen(
            &panels,
            &cameras,
            &world_panels,
            &primary_window,
            &windows,
            &conversions.panel_entities,
            &conversions.projections,
            &mut commands,
        ),
        PanelConversionRequest::WORLD_TO_WORLD => animate_world_panel_to_world(
            &panels,
            &world_panels,
            &saved_states,
            &homes,
            &transitions,
            &conversions.panel_entities,
            &conversions.projections,
            &mut commands,
        ),
        PanelConversionRequest::SCREEN_TO_WORLD => animate_screen_panel_to_world(
            &panels,
            &cameras,
            &screen_panels,
            &conversions.panel_entities,
            &conversions.projections,
            &mut commands,
        ),
        PanelConversionRequest::SCREEN_TO_SCREEN => animate_screen_panel_to_screen(
            &panels,
            &cameras,
            &screen_panels_with_home,
            &conversions.projections,
            &mut commands,
        ),
        PanelConversionRequest::WORLD_TO_SCREEN_IMMEDIATE => jump_world_panel_to_screen(
            &panels,
            &cameras,
            &world_panels,
            &primary_window,
            &windows,
            &conversions.panel_entities,
            &conversions.projections,
            &mut highlights,
            &mut commands,
        ),
        PanelConversionRequest::WORLD_TO_WORLD_IMMEDIATE => jump_world_panel_to_world(
            &panels,
            &world_panels,
            &transitions,
            &conversions.panel_entities,
            &conversions.projections,
            &mut highlights,
            &mut commands,
        ),
        PanelConversionRequest::WORLD_TO_ORIGINAL_IMMEDIATE => jump_world_panel_to_original(
            &panels,
            &world_panels,
            &saved_states,
            &homes,
            &transitions,
            &conversions.panel_entities,
            &conversions.projections,
            &mut highlights,
            &mut commands,
        ),
        PanelConversionRequest::SCREEN_TO_WORLD_IMMEDIATE => jump_screen_panel_to_world(
            &panels,
            &cameras,
            &screen_panels,
            &conversions.panel_entities,
            &conversions.projections,
            &mut highlights,
            &mut commands,
        ),
        PanelConversionRequest::SCREEN_TO_SCREEN_IMMEDIATE => jump_screen_panel_to_screen(
            &panels,
            &cameras,
            &screen_panels_with_home,
            &conversions.panel_entities,
            &conversions.projections,
            &mut highlights,
            &mut commands,
        ),
        PanelConversionRequest::NONE => {},
        action => {
            warn!("screen_world_panel_conversion: unknown BRP transition request {action}");
        },
    }
}

fn animate_world_panel_to_screen(
    panels: &DemoPanels,
    cameras: &Query<(Entity, &GlobalTransform), With<OrbitCam>>,
    panel_query: &Query<
        (&DiegeticPanel, &Transform),
        (With<WorldAuthoredPanel>, Without<PanelTransition>),
    >,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    windows: &Query<&Window>,
    panel_entities: &PanelEntityReader,
    projections: &PanelProjectionParam,
    commands: &mut Commands,
) {
    let Ok((panel, transform)) = panel_query.get(panels.world) else {
        return;
    };
    if panel.coordinate_space().is_screen() {
        return;
    }
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let Some(screen_target) = world_panel_screen_target(primary_window, windows) else {
        warn!("screen_world_panel_conversion: world-to-screen target window is unavailable");
        return;
    };
    let screen = match projections.project_to_screen_target(panels.world, camera, screen_target) {
        Ok(conversion) => conversion,
        Err(error) => {
            warn!("screen_world_panel_conversion: world-to-screen projection failed: {error}");
            return;
        },
    };
    let world_target = PanelWorldTarget::default()
        .transform(screen_facing_transform(
            transform.translation,
            camera_transform,
        ))
        .anchor(Anchor::Center);
    let Ok(landing) =
        projections.project_screen_to_world(panels.world, camera, screen.clone(), world_target)
    else {
        warn!("screen_world_panel_conversion: world-to-screen landing projection failed");
        return;
    };
    let target =
        transform_for_world_height(panel, landing.transform, landing.size.y, transform.scale.z);
    info!(
        "screen_world_panel_conversion: 1 world panel -> screen start={:?} target={:?} scale={:?}",
        transform.translation, target.translation, target.scale
    );
    let Some(source) = panel_entities.world(panels.world) else {
        return;
    };
    if let Err(error) = commands.begin_to_screen(source, camera, screen.clone()) {
        warn!("screen_world_panel_conversion: world-to-screen preparation failed: {error}");
        return;
    }
    commands.entity(panels.world).insert(PanelTransition::new(
        OperationKey::AnimateWorldToScreen,
        *transform,
        target,
        FinishAction::ApplyScreen {
            camera,
            conversion: screen,
        },
    ));
}

fn animate_world_panel_to_world(
    panels: &DemoPanels,
    panel_query: &Query<
        (&DiegeticPanel, &Transform),
        (With<WorldAuthoredPanel>, Without<PanelTransition>),
    >,
    saved_states: &Query<&SavedPanelWorldState>,
    homes: &Query<&WorldPanelHome>,
    transitions: &Query<(), With<PanelTransition>>,
    panel_entities: &PanelEntityReader,
    projections: &PanelProjectionParam,
    commands: &mut Commands,
) {
    if transitions.contains(panels.world) {
        return;
    }
    let Ok((panel, _)) = panel_query.get(panels.world) else {
        return;
    };
    if !panel.coordinate_space().is_screen() {
        return;
    }
    let Ok(saved) = saved_states.get(panels.world) else {
        warn!("screen_world_panel_conversion: world-authored panel has no saved world state");
        return;
    };
    let home = homes.get(panels.world).ok();
    let home_transform = home.map_or(saved.transform, |home| home.transform);
    let start = match projections.project_to_saved_world(panels.world) {
        Ok(projection) => projection,
        Err(error) => {
            warn!(
                "screen_world_panel_conversion: saved screen-to-world projection failed: {error}"
            );
            return;
        },
    };
    let target = home_transform;
    info!(
        "screen_world_panel_conversion: 2 world panel -> saved start={:?} target={:?} scale={:?}",
        start.transform.translation, target.translation, target.scale
    );
    let Some(source) = panel_entities.screen(panels.world) else {
        return;
    };
    if let Err(error) = commands.apply_to_world(source, start.clone()) {
        warn!("screen_world_panel_conversion: saved world handoff failed: {error}");
        return;
    }
    commands.entity(panels.world).insert(PanelTransition::new(
        OperationKey::AnimateWorldToWorld,
        start.transform,
        target,
        FinishAction::None,
    ));
}

fn animate_screen_panel_to_world(
    panels: &DemoPanels,
    cameras: &Query<(Entity, &GlobalTransform), With<OrbitCam>>,
    panel_query: &Query<
        (&DiegeticPanel, Option<&ScreenPanelHome>),
        (With<ScreenAuthoredPanel>, Without<PanelTransition>),
    >,
    panel_entities: &PanelEntityReader,
    projections: &PanelProjectionParam,
    commands: &mut Commands,
) {
    let Ok((panel, existing_home)) = panel_query.get(panels.screen) else {
        return;
    };
    if !panel.coordinate_space().is_screen() {
        return;
    }
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    if existing_home.is_none() {
        let projection = match projections.project_to_screen(panels.screen, camera) {
            Ok(projection) => projection,
            Err(error) => {
                warn!(
                    "screen_world_panel_conversion: screen-authored home projection failed: \
                     {error}"
                );
                return;
            },
        };
        commands.entity(panels.screen).insert(ScreenPanelHome {
            conversion: screen_home_conversion(panel, projection),
        });
    }
    let final_transform = screen_panel_world_transform(camera_transform);
    let start = match projections.project_to_world(
        panels.screen,
        camera,
        screen_panel_world_start_target(final_transform, camera_transform),
    ) {
        Ok(projection) => projection,
        Err(error) => {
            warn!(
                "screen_world_panel_conversion: screen-authored world projection failed: {error}"
            );
            return;
        },
    };
    let target = screen_panel_world_destination(final_transform, start.size);
    info!(
        "screen_world_panel_conversion: A screen panel -> world start={:?} target={:?} scale={:?}",
        start.transform.translation, target.translation, target.scale
    );
    let Some(source) = panel_entities.screen(panels.screen) else {
        return;
    };
    if let Err(error) = commands.apply_to_world(source, start.clone()) {
        warn!("screen_world_panel_conversion: screen-to-world handoff failed: {error}");
        return;
    }
    commands.entity(panels.screen).insert(PanelTransition::new(
        OperationKey::AnimateScreenToWorld,
        start.transform,
        target,
        FinishAction::None,
    ));
}

fn animate_screen_panel_to_screen(
    panels: &DemoPanels,
    cameras: &Query<(Entity, &GlobalTransform), With<OrbitCam>>,
    panel_query: &Query<
        (&DiegeticPanel, &Transform, &ScreenPanelHome),
        (With<ScreenAuthoredPanel>, Without<PanelTransition>),
    >,
    projections: &PanelProjectionParam,
    commands: &mut Commands,
) {
    let Ok((panel, transform, home)) = panel_query.get(panels.screen) else {
        return;
    };
    if panel.coordinate_space().is_screen() {
        return;
    }
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let screen = match projections.project_to_screen_target(
        panels.screen,
        camera,
        screen_target_from_home(panel, home),
    ) {
        Ok(conversion) => conversion,
        Err(error) => {
            warn!("screen_world_panel_conversion: screen return target failed: {error}");
            return;
        },
    };
    let world_target = PanelWorldTarget::default()
        .transform(screen_facing_transform(
            transform.translation,
            camera_transform,
        ))
        .anchor(Anchor::BottomLeft);
    let landing = match projections.project_screen_to_world(
        panels.screen,
        camera,
        screen.clone(),
        world_target,
    ) {
        Ok(projection) => projection,
        Err(error) => {
            warn!("screen_world_panel_conversion: screen return landing failed: {error}");
            return;
        },
    };
    let target =
        transform_for_world_size(panel, landing.transform, landing.size, transform.scale.z);
    info!(
        "screen_world_panel_conversion: B screen panel -> screen start={:?} target={:?} scale={:?}",
        transform.translation, target.translation, target.scale
    );
    commands.entity(panels.screen).insert(PanelTransition::new(
        OperationKey::AnimateScreenToScreen,
        *transform,
        target,
        FinishAction::ApplyScreen {
            camera,
            conversion: screen,
        },
    ));
}

fn jump_world_panel_to_screen(
    panels: &DemoPanels,
    cameras: &Query<(Entity, &GlobalTransform), With<OrbitCam>>,
    panel_query: &Query<
        (&DiegeticPanel, &Transform),
        (With<WorldAuthoredPanel>, Without<PanelTransition>),
    >,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    windows: &Query<&Window>,
    panel_entities: &PanelEntityReader,
    projections: &PanelProjectionParam,
    highlights: &mut OperationHighlights,
    commands: &mut Commands,
) {
    let Ok((panel, _)) = panel_query.get(panels.world) else {
        return;
    };
    if panel.coordinate_space().is_screen() {
        return;
    }
    let Ok((camera, _)) = cameras.single() else {
        return;
    };
    let Some(screen_target) = world_panel_screen_target(primary_window, windows) else {
        warn!(
            "screen_world_panel_conversion: immediate world-to-screen target window is unavailable"
        );
        return;
    };
    let Some(source) = panel_entities.world(panels.world) else {
        return;
    };
    match projections.project_to_screen_target(panels.world, camera, screen_target) {
        Ok(conversion) => {
            if let Err(error) = commands.finish_to_screen(source, camera, conversion) {
                warn!(
                    "screen_world_panel_conversion: immediate world-to-screen conversion is \
                     invalid: {error}"
                );
                return;
            }
            info!("screen_world_panel_conversion: 4 immediate world panel -> screen");
            highlights.flash(OperationKey::ImmediateWorldToScreen);
        },
        Err(error) => {
            warn!("screen_world_panel_conversion: immediate world-to-screen failed: {error}");
        },
    }
}

fn jump_world_panel_to_world(
    panels: &DemoPanels,
    panel_query: &Query<
        (&DiegeticPanel, &Transform),
        (With<WorldAuthoredPanel>, Without<PanelTransition>),
    >,
    transitions: &Query<(), With<PanelTransition>>,
    panel_entities: &PanelEntityReader,
    projections: &PanelProjectionParam,
    highlights: &mut OperationHighlights,
    commands: &mut Commands,
) {
    if transitions.contains(panels.world) {
        return;
    }
    let Ok((panel, _)) = panel_query.get(panels.world) else {
        return;
    };
    if !panel.coordinate_space().is_screen() {
        return;
    }
    let Some(source) = panel_entities.screen(panels.world) else {
        return;
    };
    match projections.project_to_saved_world(panels.world) {
        Ok(projection) => {
            if let Err(error) = commands.apply_to_world(source, projection) {
                warn!(
                    "screen_world_panel_conversion: immediate saved-world conversion is invalid: \
                     {error}"
                );
                return;
            }
            info!("screen_world_panel_conversion: 3 immediate world panel -> handoff world");
            highlights.flash(OperationKey::ImmediateWorldToWorld);
        },
        Err(error) => {
            warn!(
                "screen_world_panel_conversion: immediate saved-world conversion failed: {error}"
            );
        },
    }
}

fn jump_world_panel_to_original(
    panels: &DemoPanels,
    panel_query: &Query<
        (&DiegeticPanel, &Transform),
        (With<WorldAuthoredPanel>, Without<PanelTransition>),
    >,
    saved_states: &Query<&SavedPanelWorldState>,
    homes: &Query<&WorldPanelHome>,
    transitions: &Query<(), With<PanelTransition>>,
    panel_entities: &PanelEntityReader,
    projections: &PanelProjectionParam,
    highlights: &mut OperationHighlights,
    commands: &mut Commands,
) {
    if transitions.contains(panels.world) {
        return;
    }
    let Ok((panel, _)) = panel_query.get(panels.world) else {
        return;
    };
    if !panel.coordinate_space().is_screen() {
        return;
    }
    let Ok(saved) = saved_states.get(panels.world) else {
        warn!("screen_world_panel_conversion: world-authored panel has no saved world state");
        return;
    };
    let home_transform = homes
        .get(panels.world)
        .map_or(saved.transform, |home| home.transform);
    let mut projection = match projections.project_to_saved_world(panels.world) {
        Ok(projection) => projection,
        Err(error) => {
            warn!(
                "screen_world_panel_conversion: immediate original-world conversion failed: {error}"
            );
            return;
        },
    };
    projection.transform = home_transform;
    projection.size = saved.world_size();
    let Some(source) = panel_entities.screen(panels.world) else {
        return;
    };
    if let Err(error) = commands.apply_to_world(source, projection) {
        warn!("screen_world_panel_conversion: original-world conversion failed: {error}");
        return;
    }
    info!("screen_world_panel_conversion: 5 immediate world panel -> original world");
    highlights.flash(OperationKey::ImmediateWorldToOriginal);
}

fn jump_screen_panel_to_world(
    panels: &DemoPanels,
    cameras: &Query<(Entity, &GlobalTransform), With<OrbitCam>>,
    panel_query: &Query<
        (&DiegeticPanel, Option<&ScreenPanelHome>),
        (With<ScreenAuthoredPanel>, Without<PanelTransition>),
    >,
    panel_entities: &PanelEntityReader,
    projections: &PanelProjectionParam,
    highlights: &mut OperationHighlights,
    commands: &mut Commands,
) {
    let Ok((panel, existing_home)) = panel_query.get(panels.screen) else {
        return;
    };
    if !panel.coordinate_space().is_screen() {
        return;
    }
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    ensure_screen_panel_home(
        panels.screen,
        panel,
        existing_home,
        projections,
        camera,
        commands,
    );
    let final_transform = screen_panel_world_transform(camera_transform);
    let mut projection = match projections.project_to_world(
        panels.screen,
        camera,
        screen_panel_world_start_target(final_transform, camera_transform),
    ) {
        Ok(projection) => projection,
        Err(error) => {
            warn!("screen_world_panel_conversion: immediate screen-to-world failed: {error}");
            return;
        },
    };
    projection.transform = screen_panel_world_destination(final_transform, projection.size);
    let Some(source) = panel_entities.screen(panels.screen) else {
        return;
    };
    if let Err(error) = commands.apply_to_world(source, projection) {
        warn!("screen_world_panel_conversion: immediate screen-to-world failed: {error}");
        return;
    }
    info!("screen_world_panel_conversion: C immediate screen panel -> world");
    highlights.flash(OperationKey::ImmediateScreenToWorld);
}

fn jump_screen_panel_to_screen(
    panels: &DemoPanels,
    cameras: &Query<(Entity, &GlobalTransform), With<OrbitCam>>,
    panel_query: &Query<
        (&DiegeticPanel, &Transform, &ScreenPanelHome),
        (With<ScreenAuthoredPanel>, Without<PanelTransition>),
    >,
    panel_entities: &PanelEntityReader,
    projections: &PanelProjectionParam,
    highlights: &mut OperationHighlights,
    commands: &mut Commands,
) {
    let Ok((panel, _, home)) = panel_query.get(panels.screen) else {
        return;
    };
    if panel.coordinate_space().is_screen() {
        return;
    }
    let Ok((camera, _)) = cameras.single() else {
        return;
    };
    let Some(source) = panel_entities.world(panels.screen) else {
        return;
    };
    match projections.project_to_screen_target(
        panels.screen,
        camera,
        screen_target_from_home(panel, home),
    ) {
        Ok(conversion) => {
            if let Err(error) = commands.finish_to_screen(source, camera, conversion) {
                warn!(
                    "screen_world_panel_conversion: immediate screen return conversion is \
                     invalid: {error}"
                );
                return;
            }
            info!("screen_world_panel_conversion: D immediate screen panel -> screen");
            highlights.flash(OperationKey::ImmediateScreenToScreen);
        },
        Err(error) => {
            warn!("screen_world_panel_conversion: immediate screen return failed: {error}");
        },
    }
}

fn ensure_screen_panel_home(
    panel_entity: Entity,
    panel: &DiegeticPanel,
    existing_home: Option<&ScreenPanelHome>,
    projections: &PanelProjectionParam,
    camera: Entity,
    commands: &mut Commands,
) {
    if existing_home.is_some() {
        return;
    }
    match projections.project_to_screen(panel_entity, camera) {
        Ok(projection) => {
            commands.entity(panel_entity).insert(ScreenPanelHome {
                conversion: screen_home_conversion(panel, projection),
            });
        },
        Err(error) => {
            warn!("screen_world_panel_conversion: screen-authored home projection failed: {error}");
        },
    }
}

fn screen_target_from_home(panel: &DiegeticPanel, home: &ScreenPanelHome) -> PanelScreenTarget {
    let conversion = &home.conversion;
    PanelScreenTarget::default()
        .size(Px(conversion.size.x), Px(conversion.size.y))
        .anchor(conversion.anchor.unwrap_or_else(|| panel.anchor()))
        .screen_position(conversion.anchor_position.x, conversion.anchor_position.y)
}

fn animate_panel_transitions(
    time: Res<Time>,
    panel_entities: PanelEntityReader,
    mut commands: Commands,
    mut transitions: Query<(Entity, &mut Transform, &mut PanelTransition)>,
) {
    for (entity, mut transform, mut transition) in &mut transitions {
        transition.timer.tick(time.delta());
        let t = ease(transition.timer.fraction());
        transform.translation = transition
            .from
            .translation
            .lerp(transition.to.translation, t);
        transform.rotation = transition.from.rotation.slerp(transition.to.rotation, t);
        transform.scale = transition.from.scale.lerp(transition.to.scale, t);
        if !transition.timer.just_finished() {
            continue;
        }
        match transition.finish.clone() {
            FinishAction::ApplyScreen { camera, conversion } => {
                info!("screen_world_panel_conversion: applying screen conversion to {entity:?}");
                if let Some(source) = panel_entities.world(entity) {
                    if let Err(error) = commands.finish_to_screen(source, camera, conversion) {
                        warn!("screen_world_panel_conversion: screen handoff failed: {error}");
                    }
                } else {
                    warn!("screen_world_panel_conversion: transition source is no longer world");
                }
            },
            FinishAction::None => {},
        }
        commands.entity(entity).remove::<PanelTransition>();
    }
}

fn update_panel_status_trees(
    time: Res<Time>,
    panels: Res<DemoPanels>,
    mut displayed: Local<DisplayedPanelSpaces>,
    mut highlights: ResMut<OperationHighlights>,
    world_panels: Query<&DiegeticPanel, With<WorldAuthoredPanel>>,
    screen_panels: Query<&DiegeticPanel, With<ScreenAuthoredPanel>>,
    saved_states: Query<&SavedPanelWorldState>,
    transitions: Query<(Entity, &PanelTransition)>,
    mut commands: Commands,
) {
    let clear_flash = if let Some(flash) = &mut highlights.flash {
        flash.timer.tick(time.delta());
        flash.timer.just_finished()
    } else {
        false
    };
    if clear_flash {
        highlights.flash = None;
    }

    let mut active: Vec<OperationKey> = transitions
        .iter()
        .map(|(_, transition)| transition.operation)
        .collect();
    if let Some(flash) = &highlights.flash {
        active.push(flash.key);
    }
    active.sort_unstable();
    active.dedup();

    if let Ok(panel) = world_panels.get(panels.world) {
        let space = PanelSpace::from_panel(panel);
        let enabled = world_available_operations(
            panel,
            saved_states.get(panels.world).is_ok(),
            transitions.get(panels.world).is_ok(),
        );
        let highlighted = active
            .iter()
            .copied()
            .filter(|operation| operation.is_world_panel())
            .collect::<Vec<_>>();
        if displayed.world != Some(space)
            || displayed.world_enabled != enabled
            || displayed.world_highlighted != highlighted
        {
            if let Err(error) = commands.set_tree(
                panels.world,
                world_panel_tree(space, world_panel_metrics(panel), &enabled, &highlighted),
            ) {
                error!("failed to replace world conversion panel tree: {error}");
            }
            displayed.world = Some(space);
            displayed.world_enabled = enabled;
            displayed.world_highlighted = highlighted;
        }
    }
    if let Ok(panel) = screen_panels.get(panels.screen) {
        let space = PanelSpace::from_panel(panel);
        let enabled = screen_available_operations(panel, transitions.get(panels.screen).is_ok());
        let highlighted = active
            .iter()
            .copied()
            .filter(|operation| !operation.is_world_panel())
            .collect::<Vec<_>>();
        if displayed.screen != Some(space)
            || displayed.screen_enabled != enabled
            || displayed.screen_highlighted != highlighted
        {
            if let Err(error) = commands.set_tree(
                panels.screen,
                screen_panel_tree(space, &enabled, &highlighted),
            ) {
                error!("failed to replace screen conversion panel tree: {error}");
            }
            displayed.screen = Some(space);
            displayed.screen_enabled = enabled;
            displayed.screen_highlighted = highlighted;
        }
    }
}

fn build_world_panel(
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    let material = materials.add(panel_surface(WORLD_PANEL_COLOR));
    DiegeticPanel::world()
        .size(Mm(WORLD_PANEL_WIDTH_MM), Mm(WORLD_PANEL_HEIGHT_MM))
        .font_unit(Unit::Millimeters)
        .world_height(WORLD_PANEL_HEIGHT)
        .anchor(Anchor::Center)
        .material(material.clone())
        .text_material(material)
        .with_tree(world_panel_tree(
            PanelSpace::World,
            PanelTreeMetrics::world(Unit::Millimeters, WORLD_PANEL_HEIGHT_MM),
            &[
                OperationKey::AnimateWorldToScreen,
                OperationKey::ImmediateWorldToScreen,
            ],
            &[],
        ))
        .build()
}

fn build_screen_panel(
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    let material = materials.add(screen_panel_material());
    DiegeticPanel::screen()
        .size(Fit, Fit)
        .font_unit(Unit::Pixels)
        .anchor(Anchor::TopRight)
        .material(material.clone())
        .text_material(material)
        .with_tree(screen_panel_tree(
            PanelSpace::Screen,
            &[
                OperationKey::AnimateScreenToWorld,
                OperationKey::ImmediateScreenToWorld,
            ],
            &[],
        ))
        .build()
}

fn screen_panel_tree(
    space: PanelSpace,
    enabled: &[OperationKey],
    highlighted: &[OperationKey],
) -> LayoutTree {
    let metrics = PanelTreeMetrics::screen();
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::fixed(Px(SCREEN_PANEL_WIDTH_PX)),
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(Px(SCREEN_GAP_PX)),
                |builder| {
                    builder.text((
                        "Screen Authored Panel",
                        TextStyle::new(metrics.title_size).with_color(TEXT_COLOR),
                    ));
                    status_badge(builder, space, metrics);
                    panel_option_row(
                        builder,
                        OperationKey::AnimateScreenToWorld,
                        metrics,
                        enabled,
                        highlighted,
                    );
                    panel_option_row(
                        builder,
                        OperationKey::AnimateScreenToScreen,
                        metrics,
                        enabled,
                        highlighted,
                    );
                    panel_option_row(
                        builder,
                        OperationKey::ImmediateScreenToWorld,
                        metrics,
                        enabled,
                        highlighted,
                    );
                    panel_option_row(
                        builder,
                        OperationKey::ImmediateScreenToScreen,
                        metrics,
                        enabled,
                        highlighted,
                    );
                },
            );
        },
    );
    builder.build()
}

fn world_panel_tree(
    space: PanelSpace,
    metrics: PanelTreeMetrics,
    enabled: &[OperationKey],
    highlighted: &[OperationKey],
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(metrics.dim(metrics.padding)))
            .gap(metrics.dim(metrics.gap))
            .corner_radius(CornerRadius::all(metrics.dim(metrics.radius)))
            .background(PANEL_FILL_BACKGROUND),
    );
    builder.text((
        "World Authored Panel",
        TextStyle::new(metrics.title_size).with_color(TEXT_COLOR),
    ));
    status_badge(&mut builder, space, metrics);
    panel_option_row(
        &mut builder,
        OperationKey::AnimateWorldToScreen,
        metrics,
        enabled,
        highlighted,
    );
    panel_option_row(
        &mut builder,
        OperationKey::AnimateWorldToWorld,
        metrics,
        enabled,
        highlighted,
    );
    panel_option_row(
        &mut builder,
        OperationKey::ImmediateWorldToWorld,
        metrics,
        enabled,
        highlighted,
    );
    panel_option_row(
        &mut builder,
        OperationKey::ImmediateWorldToScreen,
        metrics,
        enabled,
        highlighted,
    );
    panel_option_row(
        &mut builder,
        OperationKey::ImmediateWorldToOriginal,
        metrics,
        enabled,
        highlighted,
    );
    builder.build()
}

fn panel_option_row(
    builder: &mut LayoutBuilder,
    operation: OperationKey,
    metrics: PanelTreeMetrics,
    enabled: &[OperationKey],
    highlighted: &[OperationKey],
) {
    let is_highlighted = operation_enabled(highlighted, operation);
    let is_enabled = operation_enabled(enabled, operation);
    let mut row = El::new()
        .width(Sizing::FIT)
        .height(Sizing::FIT)
        .padding(Padding::xy(
            metrics.dim(metrics.status_padding_x),
            metrics.dim(metrics.status_padding_y),
        ));
    if is_highlighted {
        row = row.background(ACTION_HIGHLIGHT);
    }
    builder.with(row, |builder| {
        builder.text((
            operation.label(),
            TextStyle::new(metrics.detail_size).with_color(if is_highlighted {
                TEXT_COLOR
            } else {
                enabled_text_color(is_enabled)
            }),
        ));
    });
}

fn world_panel_metrics(panel: &DiegeticPanel) -> PanelTreeMetrics {
    PanelTreeMetrics::world(panel.layout_unit(), panel.height())
}

fn status_badge(builder: &mut LayoutBuilder, space: PanelSpace, metrics: PanelTreeMetrics) {
    builder.with(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::xy(
                metrics.dim(metrics.status_padding_x),
                metrics.dim(metrics.status_padding_y),
            ))
            .corner_radius(CornerRadius::all(metrics.dim(metrics.radius * 0.45)))
            .background(space.highlight()),
        |builder| {
            builder.text((
                space.label(),
                TextStyle::new(metrics.status_size).with_color(TEXT_COLOR),
            ));
        },
    );
}

fn world_panel_screen_target(
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    windows: &Query<&Window>,
) -> Option<PanelScreenTarget> {
    let window = windows.get(primary_window.single().ok()?).ok()?;
    let height = window.height() * WORLD_SCREEN_HEIGHT_FRACTION;
    let width = height * WORLD_PANEL_WIDTH_MM / WORLD_PANEL_HEIGHT_MM;
    Some(
        PanelScreenTarget::default()
            .size(Px(width), Px(height))
            .anchor(Anchor::Center)
            .screen(),
    )
}

fn screen_home_conversion(
    panel: &DiegeticPanel,
    projection: hana_diegetic::PanelScreenProjection,
) -> PanelScreenConversion {
    let conversion = PanelScreenConversion::from(projection).anchor(panel.anchor());
    let CoordinateSpace::Screen {
        width,
        height,
        camera_order,
        ref render_layers,
        window,
        ..
    } = *panel.coordinate_space()
    else {
        return conversion;
    };
    conversion
        .sizing(width, height)
        .camera_order(camera_order)
        .render_layers(render_layers.clone())
        .window(window)
}

fn saved_world_transform() -> Transform {
    Transform::from_translation(Vec3::new(-0.78, 1.28, 0.16))
        .with_rotation(Quat::from_rotation_y(-0.18) * Quat::from_rotation_x(0.10))
}

fn screen_panel_world_transform(camera_transform: &GlobalTransform) -> Transform {
    screen_facing_transform(Vec3::new(-1.62, 0.92, 1.18), camera_transform)
}

fn screen_panel_world_start_target(
    final_transform: Transform,
    camera_transform: &GlobalTransform,
) -> PanelWorldTarget {
    PanelWorldTarget::default()
        .transform(screen_facing_transform(
            final_transform.translation,
            camera_transform,
        ))
        .anchor(Anchor::BottomLeft)
}

fn screen_panel_world_destination(final_transform: Transform, source_size: Vec2) -> Transform {
    final_transform.with_scale(Vec3::splat(
        SCREEN_WORLD_HEIGHT / source_size.y.max(f32::EPSILON),
    ))
}

fn screen_facing_transform(translation: Vec3, camera_transform: &GlobalTransform) -> Transform {
    Transform::from_translation(translation)
        .with_rotation(camera_transform.compute_transform().rotation)
}

fn transform_for_world_height(
    panel: &DiegeticPanel,
    mut transform: Transform,
    height: f32,
    z_scale: f32,
) -> Transform {
    let scale = height / panel.world_height().max(f32::EPSILON);
    transform.scale = Vec3::new(scale, scale, z_scale);
    transform
}

fn transform_for_world_size(
    panel: &DiegeticPanel,
    mut transform: Transform,
    size: Vec2,
    z_scale: f32,
) -> Transform {
    transform.scale = Vec3::new(
        size.x / panel.world_width().max(f32::EPSILON),
        size.y / panel.world_height().max(f32::EPSILON),
        z_scale,
    );
    transform
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * 2.0f32.mul_add(-t, 3.0)
}

fn panel_surface(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        unlit: true,
        ..default_panel_material()
    }
}
