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
use bevy::prelude::With;
use bevy::prelude::error;
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
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DrawZIndex;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::In;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::PanelBuildError;
use hana_diegetic::PaperSize;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_lagrange::OrbitCamPreset;

const HOME_FOCUS: Vec3 = Vec3::ZERO;
const HOME_MARGIN: f32 = 0.12;
const HOME_PITCH: f32 = 0.08;
const HOME_RADIUS: f32 = 0.50;
const HOME_YAW: f32 = 0.0;
const PANEL_BACKGROUND_ALPHA: f32 = 0.88;
const PANEL_TRANSLATION: Vec3 = Vec3::new(0.0, 0.015, 0.0);

const PAGE_BORDER_COLOR: Color = Color::srgba(0.15, 0.7, 0.9, 0.5);
const PAGE_BORDER_IN: f32 = 0.014;
const PAGE_PADDING_IN: f32 = 0.24;
const PAGE_RADIUS_IN: f32 = 0.08;

const SWEEP_BORDER_COLOR: Color = Color::srgba(1.0, 0.93, 0.84, 0.62);
const SWEEP_COLOR: Color = Color::srgba(1.0, 0.34, 0.12, 0.72);
const SWEEP_INNER_COLOR: Color = Color::srgba(0.04, 0.025, 0.018, 0.46);
const SWEEP_INNER_RADIUS_IN: f32 = 0.055;
const SWEEP_RADIUS_IN: f32 = 0.08;
const SWEEP_SPEED_IN_PER_SEC: f32 = 1.20;
const SWEEP_WIDTH_IN: f32 = 1.10;

const STORY_TEXT: &str = "Alice was beginning to get very tired of sitting by her sister on the bank, and of having nothing to do: once or twice she had peeped into the book her sister was reading, but it had no pictures or conversations in it, and what is the use of a book, thought Alice, without pictures or conversations?";
const TEXT_COLOR: Color = Color::srgb(0.94, 0.98, 1.0);
const TEXT_SIZE_PT: f32 = 22.0;
const TEXT_Z: DrawZIndex = DrawZIndex(10);

const BEHIND_SEGMENT: &str = "z-behind";
const FRONT_SEGMENT: &str = "z-front";
const DESCRIPTION_HEADING: &str = "Panel Draw Order";
const DESCRIPTION_LINES: [&str; 4] = [
    "B Behind and F Front change the sweep DrawZIndex only.",
    "The text layer and sweep layer are siblings inside El::overlay().",
    "DrawZIndex is scoped to one diegetic panel tree.",
    "The name differs from Bevy UI's ZIndex on purpose.",
];

#[derive(Component)]
struct DrawOrderPanel;

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
    fn advance(&mut self, delta_secs: f32) {
        self.x = (self.direction * SWEEP_SPEED_IN_PER_SEC).mul_add(delta_secs, self.x);
        let track = sweep_track_in();
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
        .with_title("Draw Order")
        .with_background_color(DEFAULT_PANEL_BACKGROUND.with_alpha(0.88))
        .control(TitleBarControl::segmented(
            "DrawZIndex",
            [
                TitleBarSegment::new(BEHIND_SEGMENT, "B Behind"),
                TitleBarSegment::new(FRONT_SEGMENT, "F Front"),
            ],
        ))
}

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_HEADING)
        .with_body_size(LABEL_SIZE.0)
        .lines(DESCRIPTION_LINES)
}

fn setup(
    mut commands: Commands,
    state: Res<DemoState>,
    sweep: Res<SweepPosition>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(panel) = build_panel(*state, sweep.x, &mut materials) else {
        error!("panel_draw_order: failed to build demo panel");
        return;
    };
    commands.spawn((
        Name::new("Panel draw order demo"),
        CameraHomeTarget,
        DrawOrderPanel,
        panel,
        Transform::from_translation(PANEL_TRANSLATION),
    ));
}

fn update_panel(
    mut commands: Commands,
    state: Res<DemoState>,
    time: Res<Time>,
    mut sweep: ResMut<SweepPosition>,
    panel: Single<Entity, With<DrawOrderPanel>>,
) {
    sweep.advance(time.delta_secs());
    if let Err(error) = commands.set_tree(*panel, draw_order_tree(*state, sweep.x)) {
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
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    let material = materials.add(screen_panel_material());
    let (page_width, page_height) = page_size();
    DiegeticPanel::world()
        .size(page_width, page_height)
        .anchor(Anchor::Center)
        .material(material.clone())
        .text_material(material)
        .with_tree(draw_order_tree(state, sweep_x))
        .build()
}

fn draw_order_tree(state: DemoState, sweep_x: f32) -> LayoutTree {
    let (page_width, page_height) = page_size();
    let mut builder = LayoutBuilder::with_root(
        El::overlay()
            .size(page_width, page_height)
            .background(DEFAULT_PANEL_BACKGROUND.with_alpha(PANEL_BACKGROUND_ALPHA))
            .border(page_border())
            .corner_radius(CornerRadius::all(In(PAGE_RADIUS_IN))),
    );
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(In(PAGE_PADDING_IN))),
        |builder| {
            builder.text(
                Text::new(STORY_TEXT, story_style())
                    .layout(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::FIT)
                            .z_index(TEXT_Z),
                    )
                    .wrap(TextWrap::Words),
            );
        },
    );
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(In(PAGE_PADDING_IN))),
        |builder| {
            sweep_layer(builder, state, sweep_x);
        },
    );
    builder.build()
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

fn page_size() -> (In, In) {
    (
        In(PaperSize::Photo5x7.height_as::<In>()),
        In(PaperSize::Photo5x7.width_as::<In>()),
    )
}

fn sweep_track_in() -> f32 {
    let (page_width, _) = page_size();
    let inset = PAGE_BORDER_IN + PAGE_PADDING_IN;
    let content_width = inset.mul_add(-2.0, page_width.0);
    (content_width - SWEEP_WIDTH_IN).max(0.0)
}
