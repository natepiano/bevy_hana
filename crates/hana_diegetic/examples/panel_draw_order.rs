//! `panel_draw_order` - one panel tree ordered by `DrawZIndex`.
//!
//! Press `B` or `F` to move the sweep behind or in front of the story text.
//! The controls change only the sweep element's `DrawZIndex`; the layout stays
//! an `El::overlay()` with both children sharing the panel content rectangle.

use bevy::prelude::Alpha;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::Name;
use bevy::prelude::PerspectiveProjection;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::window::PrimaryWindow;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::DescriptionPanel;
use fairy_dust::LABEL_SIZE;
use fairy_dust::OrbitCamPose;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use fairy_dust::screen_panel_material;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::CascadeEntityCommandsExt;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::DrawZIndex;
use hana_diegetic::El;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::In;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Lighting;
use hana_diegetic::Padding;
use hana_diegetic::PanelBuildError;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextAlign;
use hana_diegetic::TextInk;
use hana_diegetic::TextInkMeasurement;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Unit;
use hana_lagrange::OrbitCamHomePose;
use hana_lagrange::OrbitCamPreset;

const HOME_FOCUS: Vec3 = Vec3::ZERO;
const HOME_MARGIN: f32 = 0.12;
const HOME_PITCH: f32 = 0.08;
const HOME_RADIUS: f32 = 0.50;
const HOME_WINDOW_HEIGHT: f32 = 720.0;
const HOME_YAW: f32 = 0.0;
const PANEL_TRANSLATION: Vec3 = Vec3::new(0.0, 0.015, 0.0);

const PAGE_FILL_COLOR: Color = Color::srgb(46.0 / 255.0, 52.0 / 255.0, 64.0 / 255.0);
const PAGE_BORDER_COLOR: Color = Color::srgba(129.0 / 255.0, 161.0 / 255.0, 193.0 / 255.0, 0.85);
const PAGE_BORDER_IN: f32 = 0.014;
const PAGE_MARGIN_IN: f32 = 0.24;
const PAGE_RADIUS_LOGICAL_PIXELS: f32 = 12.0;

const SWEEP_BORDER_COLOR: Color = Color::srgba(1.0, 0.93, 0.84, 0.62);
const SWEEP_COLOR: Color = Color::srgba(0.78, 0.24, 0.08, 0.72);
const SWEEP_INNER_COLOR: Color = Color::srgba(0.03, 0.02, 0.014, 0.46);
const SWEEP_INNER_RADIUS_IN: f32 = 0.055;
const SWEEP_RADIUS_IN: f32 = 0.08;
const SWEEP_SPEED_IN_PER_SEC: f32 = 1.20;
const SWEEP_WIDTH_IN: f32 = 1.10;

const STORY_TEXT: &str = concat!(
    "Alice was beginning to get very tired of sitting by her\n",
    "sister on the bank, and of having nothing to do: once or\n",
    "twice she had peeped into the book her sister was reading,\n",
    "but it had no pictures or conversations in it, and what is\n",
    "the use of a book, thought Alice, without pictures or\n",
    "conversations?",
);
const TEXT_COLOR: Color = Color::srgb(0.94, 0.98, 1.0);
const TEXT_SIZE_PT: f32 = 22.0;
const TEXT_Z: DrawZIndex = DrawZIndex(10);

const BEHIND_SEGMENT: &str = "z-behind";
const FRONT_SEGMENT: &str = "z-front";
const DESCRIPTION_HEADING: &str = "Panel draw order";
const DESCRIPTION_LINES: [&str; 2] = [
    "Both layers stay in the same panel and keep their layout.",
    "Only their drawing order changes.",
];

#[derive(Component)]
struct DrawOrderPanel;

#[derive(Resource, Clone, Copy)]
struct StoryFrame {
    width:          In,
    height:         In,
    left_padding:   In,
    top_padding:    In,
    bottom_padding: In,
}

impl StoryFrame {
    fn measure(fonts: &FontRegistry, measurer: &DiegeticTextMeasurer) -> Self {
        let measure = story_style().as_measure();
        let dimensions = (measurer.measure_fn)(STORY_TEXT, &measure);
        let points_per_inch = Unit::Inches.to_points();
        let ink = match fonts.measure_ink(STORY_TEXT, &measure) {
            Ok(TextInkMeasurement::Visible(ink)) => ink,
            Ok(TextInkMeasurement::NoVisibleInk) | Err(_) => TextInk {
                left:   0.0,
                top:    0.0,
                right:  dimensions.width,
                bottom: dimensions.height,
            },
        };
        let left_padding = In(PAGE_MARGIN_IN - ink.left / points_per_inch);
        let top_padding = In(PAGE_MARGIN_IN - ink.top / points_per_inch);
        let bottom_padding =
            In(PAGE_MARGIN_IN - (dimensions.height - ink.bottom) / points_per_inch);
        Self {
            width: In(2.0_f32.mul_add(PAGE_BORDER_IN, left_padding.0)
                + ink.right / points_per_inch
                + PAGE_MARGIN_IN),
            height: In(2.0_f32.mul_add(PAGE_BORDER_IN, top_padding.0)
                + dimensions.height / points_per_inch
                + bottom_padding.0),
            left_padding,
            top_padding,
            bottom_padding,
        }
    }

    fn sweep_track(self) -> f32 {
        (2.0_f32.mul_add(-PAGE_BORDER_IN, self.width.0)
            - self.left_padding.0
            - PAGE_MARGIN_IN
            - SWEEP_WIDTH_IN)
            .max(0.0)
    }
}

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq)]
struct DemoState {
    layer: LayerMode,
}

#[derive(Resource, Clone, Copy, PartialEq)]
struct SweepPosition {
    direction: f32,
    x:         f32,
}

impl Default for SweepPosition {
    fn default() -> Self {
        Self {
            direction: 1.0,
            x:         0.0,
        }
    }
}

impl SweepPosition {
    fn advance(&mut self, delta_secs: f32, track: f32) {
        self.x = (self.direction * SWEEP_SPEED_IN_PER_SEC).mul_add(delta_secs, self.x);
        if self.x >= track {
            self.x = track;
            self.direction = -1.0;
        } else if self.x <= 0.0 {
            self.x = 0.0;
            self.direction = 1.0;
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum LayerMode {
    #[default]
    BehindText,
    InFrontOfText,
}

impl LayerMode {
    const fn z_index(self) -> DrawZIndex {
        match self {
            Self::BehindText => DrawZIndex(9),
            Self::InFrontOfText => DrawZIndex(11),
        }
    }
}

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_orbit_cam_preset_pose(
            OrbitCamPose {
                focus:  HOME_FOCUS,
                yaw:    HOME_YAW,
                pitch:  HOME_PITCH,
                radius: HOME_RADIUS,
            },
            OrbitCamPreset::blender_like(),
        )
        .with_stable_transparency()
        .with_camera_home()
        .margin(HOME_MARGIN)
        .with_title_bar(title_bar())
        .wire_chip_to_state::<DemoState, _>(BEHIND_SEGMENT, |state| {
            chip_activation(state.layer == LayerMode::BehindText)
        })
        .wire_chip_to_state::<DemoState, _>(FRONT_SEGMENT, |state| {
            chip_activation(state.layer == LayerMode::InFrontOfText)
        })
        .with_description_panel(description_panel())
        .with_camera_control_panel()
        .with_camera_control_panel_background_color(DEFAULT_PANEL_BACKGROUND.with_alpha(0.88))
        .init_resource::<DemoState>()
        .init_resource::<SweepPosition>()
        .add_systems(Startup, setup)
        .add_systems(Update, update_panel)
        .with_shortcut(KeyCode::KeyB, set_behind_text)
        .with_shortcut(KeyCode::KeyF, set_in_front_of_text)
        .run();
}

fn title_bar() -> TitleBar {
    TitleBar::new()
        .with_title(DESCRIPTION_HEADING)
        .with_background_color(DEFAULT_PANEL_BACKGROUND.with_alpha(0.88))
        .control(
            TitleBarControl::segmented(
                "Sweep:",
                [
                    TitleBarSegment::new(BEHIND_SEGMENT, "B behind"),
                    TitleBarSegment::new(FRONT_SEGMENT, "F in front"),
                ],
            )
            .with_segment_separator(","),
        )
}

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_HEADING)
        .with_body_size(LABEL_SIZE.0)
        .with_fit_width()
        .lines(DESCRIPTION_LINES)
}

fn setup(
    mut commands: Commands,
    state: Res<DemoState>,
    sweep: Res<SweepPosition>,
    fonts: Res<FontRegistry>,
    measurer: Res<DiegeticTextMeasurer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let frame = StoryFrame::measure(&fonts, &measurer);
    let initial_radius = page_radius_in(
        HOME_RADIUS,
        PerspectiveProjection::default().fov,
        HOME_WINDOW_HEIGHT,
    );
    let Ok(panel) = build_panel(*state, sweep.x, frame, initial_radius, &mut materials) else {
        error!("panel_draw_order: failed to build demo panel");
        return;
    };
    commands.insert_resource(frame);
    commands
        .spawn((
            Name::new("Panel draw order demo"),
            CameraHomeTarget,
            DrawOrderPanel,
            panel,
            Transform::from_translation(PANEL_TRANSLATION),
        ))
        .override_lighting(Lighting::Unlit);
}

fn update_panel(
    mut commands: Commands,
    state: Res<DemoState>,
    time: Res<Time>,
    mut sweep: ResMut<SweepPosition>,
    frame: Res<StoryFrame>,
    panel: Single<Entity, With<DrawOrderPanel>>,
    cameras: Query<(&OrbitCamHomePose, &Projection)>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let Ok((home_pose, Projection::Perspective(projection))) = cameras.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    if window.height() <= 0.0 {
        return;
    }
    // Home's stored pose already includes the page bounds, margin, window size,
    // and side-panel insets used by the camera fit. A resize recaptures it.
    let page_radius = page_radius_in(home_pose.zoom.0, projection.fov, window.height());
    sweep.advance(time.delta_secs(), frame.sweep_track());
    if let Err(error) = commands.set_tree(
        *panel,
        draw_order_tree(*state, sweep.x, *frame, page_radius),
    ) {
        error!("failed to replace draw-order panel tree: {error}");
    }
}

fn set_behind_text(mut state: ResMut<DemoState>) { state.layer = LayerMode::BehindText; }

fn set_in_front_of_text(mut state: ResMut<DemoState>) { state.layer = LayerMode::InFrontOfText; }

const fn chip_activation(active: bool) -> ControlActivation {
    if active {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}

fn build_panel(
    state: DemoState,
    sweep_x: f32,
    frame: StoryFrame,
    page_radius: In,
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    let material = materials.add(screen_panel_material());
    DiegeticPanel::world()
        .size(frame.width, frame.height)
        .anchor(Anchor::Center)
        .material(material.clone())
        .text_material(material)
        .with_tree(draw_order_tree(state, sweep_x, frame, page_radius))
        .build()
}

fn draw_order_tree(
    state: DemoState,
    sweep_x: f32,
    frame: StoryFrame,
    page_radius: In,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::overlay()
            .size(frame.width, frame.height)
            .background(PAGE_FILL_COLOR)
            .border(page_border())
            .corner_radius(CornerRadius::all(page_radius)),
    );
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::new(
                frame.left_padding,
                In(PAGE_MARGIN_IN),
                frame.top_padding,
                frame.bottom_padding,
            )),
        |builder| {
            builder.text(
                Text::new(STORY_TEXT, story_style())
                    .layout(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::FIT)
                            .z_index(TEXT_Z),
                    )
                    .wrap(TextWrap::None),
            );
        },
    );
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::new(
                frame.left_padding,
                In(PAGE_MARGIN_IN),
                frame.top_padding,
                frame.top_padding,
            )),
        |builder| {
            sweep_layer(builder, state, sweep_x);
        },
    );
    builder.build()
}

fn page_radius_in(home_distance: f32, vertical_fov: f32, logical_window_height: f32) -> In {
    let visible_world_height = 2.0 * home_distance * (vertical_fov * 0.5).tan();
    In(PAGE_RADIUS_LOGICAL_PIXELS * visible_world_height
        / (Unit::Inches.meters_per_unit() * logical_window_height))
}

fn sweep_layer(builder: &mut LayoutBuilder, state: DemoState, sweep_x: f32) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .align_y(AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(In(sweep_x)))
                    .height(Sizing::GROW),
                |_| {},
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(In(SWEEP_WIDTH_IN)))
                    .height(Sizing::GROW)
                    .padding(Padding::all(In(PAGE_BORDER_IN)))
                    .corner_radius(CornerRadius::all(In(SWEEP_RADIUS_IN)))
                    .background(SWEEP_COLOR)
                    .border(Border::all(In(PAGE_BORDER_IN), SWEEP_BORDER_COLOR))
                    .z_index(state.layer.z_index()),
                |builder| {
                    builder.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .corner_radius(CornerRadius::all(In(SWEEP_INNER_RADIUS_IN)))
                            .background(SWEEP_INNER_COLOR)
                            .z_index(state.layer.z_index()),
                        |_| {},
                    );
                },
            );
            builder.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        },
    );
}

fn story_style() -> TextStyle {
    TextStyle::new(Pt(TEXT_SIZE_PT))
        .with_color(TEXT_COLOR)
        .with_align(TextAlign::Left)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn page_border() -> Border { Border::all(In(PAGE_BORDER_IN), PAGE_BORDER_COLOR) }
