//! Screen-space comparison of exact and integral-advance monospace sizes.
//!
//! A fixed-pitch font can still look uneven when its glyph advance is a
//! fractional logical-pixel width: successive glyph origins land at different
//! pixel phases, so identical outlines receive different alpha coverage.
//! [`Font::nearest_integral_advance_size`] returns the nearest point size whose
//! fixed advance is a whole logical pixel. The right column applies that size;
//! the left column renders the exact request.
//! Using the resolved size improves the on-screen appearance of monospace text
//! by keeping glyph-edge coverage consistent across each run.
//!
//! The resolver only controls the distance between glyph origins. It does not
//! snap the first glyph's origin, and it returns a typed error for proportional
//! fonts or invalid requests.
//!
//! The comparison panel stays centered in screen space while the lit ground
//! plane and rotating cube move with the world camera behind it.

use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::error;
use fairy_dust::CubeSpinConfig;
use fairy_dust::DescriptionPanel;
use fairy_dust::OrbitCamPose;
use fairy_dust::TitleBar;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material_handle;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::Font;
use hana_diegetic::FontId;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::IntegralAdvanceSizeError;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_lagrange::OrbitCamPreset;

const EXAMPLE_TITLE: &str = "Integral Advance";
const SAMPLE_TEXT: &str = "HHHHHHHHH";
const REQUESTED_SIZES: [Pt; 10] = [
    Pt(8.0),
    Pt(9.0),
    Pt(10.0),
    Pt(11.0),
    Pt(12.0),
    Pt(14.0),
    Pt(16.0),
    Pt(20.0),
    Pt(24.0),
    Pt(32.0),
];

const CUBE_CLEARANCE: f32 = 0.1;
const CAMERA_FOCUS: Vec3 = Vec3::new(0.0, 0.6, 0.0);
const CAMERA_YAW: f32 = 1.222_275_4;
const CAMERA_PITCH: f32 = 0.541_051_57;
const CAMERA_RADIUS: f32 = 40.0;

const LABEL_COLUMN_WIDTH: Px = Px(150.0);
const SAMPLE_COLUMN_WIDTH: Px = Px(250.0);
const MINIMUM_ROW_HEIGHT: f32 = 32.0;
const ROW_HEIGHT_PER_POINT: f32 = 1.4;
const ROW_HEIGHT_PADDING: f32 = 10.0;
const COLUMN_GAP: Px = Px(12.0);
const SECTION_GAP: Px = Px(8.0);
const ROW_GAP: Px = Px(2.0);
const ROW_PADDING: Px = Px(4.0);
const DIVIDER_HEIGHT: Px = Px(1.0);

const TITLE_SIZE: Pt = Pt(12.5);
const EXPLANATION_SIZE: Pt = Pt(8.75);
const HEADER_SIZE: Pt = Pt(8.75);
const SIZE_LABEL_SIZE: Pt = Pt(8.75);

const PANEL_BACKGROUND: Color = Color::srgba(0.025, 0.03, 0.045, 0.84);
const TITLE_COLOR: Color = Color::srgb(0.9, 0.94, 1.0);
const BODY_COLOR: Color = Color::srgba(0.7, 0.76, 0.86, 0.9);
const HEADER_COLOR: Color = Color::srgb(0.48, 0.82, 1.0);
const SIZE_LABEL_COLOR: Color = Color::srgba(0.62, 0.68, 0.78, 0.86);
const SAMPLE_COLOR: Color = Color::srgb(0.92, 0.94, 1.0);
const ROW_BACKGROUND: Color = Color::srgba(0.08, 0.1, 0.15, 0.42);
const ROW_BORDER_COLOR: Color = Color::srgba(0.26, 0.34, 0.48, 0.34);
const DIVIDER_COLOR: Color = Color::srgba(0.3, 0.68, 1.0, 0.42);

const DESCRIPTION_LINES: &[&str] = &[
    "Identical monospace runs at the same requested sizes.",
    "Left keeps the request; right uses the nearest whole-pixel advance.",
    "The pixel differences are readily apparent if you take a screenshot and blow it up.",
];

fn main() {
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
        .cube_spin(CubeSpinConfig::new())
        .with_orbit_cam_preset_pose(
            OrbitCamPose {
                focus:  CAMERA_FOCUS,
                yaw:    CAMERA_YAW,
                pitch:  CAMERA_PITCH,
                radius: CAMERA_RADIUS,
            },
            OrbitCamPreset::blender_like(),
        )
        .with_stable_transparency()
        .with_title_bar(TitleBar::new().with_title(EXAMPLE_TITLE))
        .with_description_panel(
            DescriptionPanel::new("What to compare")
                .with_anchor(Anchor::BottomLeft)
                .lines(DESCRIPTION_LINES.iter().copied()),
        )
        .with_camera_control_panel()
        .add_systems(Startup, spawn_comparison_panel)
        .run();
}

fn spawn_comparison_panel(
    mut commands: Commands,
    fonts: Res<FontRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(font) = fonts.font(FontId::MONOSPACE) else {
        error!("integral_advance: embedded monospace font is unavailable");
        return;
    };
    let tree = match comparison_tree(font) {
        Ok(tree) => tree,
        Err(error) => {
            error!("integral_advance: cannot resolve comparison sizes: {error}");
            return;
        },
    };

    let unlit = screen_panel_material_handle(&mut materials);
    let panel = DiegeticPanel::screen()
        .size(Sizing::FIT, Sizing::FIT)
        .anchor(Anchor::Center)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(tree)
        .build();

    match panel {
        Ok(panel) => {
            commands.spawn((panel, Transform::default()));
        },
        Err(error) => {
            error!("integral_advance: failed to build comparison panel: {error}");
        },
    }
}

/// Builds the table while propagating the resolver's typed error unchanged.
///
/// The call inside the loop is the demonstrated API: keep `requested` for the
/// exact column and use the returned [`Pt`] for the integral-advance column.
fn comparison_tree(font: &Font) -> Result<LayoutTree, IntegralAdvanceSizeError> {
    let mut comparisons = Vec::with_capacity(REQUESTED_SIZES.len());
    for requested in REQUESTED_SIZES {
        let integral = font.nearest_integral_advance_size(requested)?;
        comparisons.push(SizeComparison {
            requested,
            integral,
        });
    }

    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::FIT,
        Sizing::FIT,
        PANEL_BACKGROUND,
        |builder| comparison_contents(builder, &comparisons),
    );
    Ok(builder.build())
}

#[derive(Clone, Copy)]
struct SizeComparison {
    requested: Pt,
    integral:  Pt,
}

fn comparison_contents(builder: &mut LayoutBuilder, comparisons: &[SizeComparison]) {
    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(SECTION_GAP),
        |builder| {
            builder.text(("MONOSPACE GLYPH PHASE", text_style(TITLE_SIZE, TITLE_COLOR)));
            builder.text((
                "Same glyph run; only the point-size policy changes",
                text_style(EXPLANATION_SIZE, BODY_COLOR),
            ));
            divider(builder);
            header_row(builder);
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(ROW_GAP),
                |builder| {
                    for comparison in comparisons {
                        comparison_row(builder, *comparison);
                    }
                },
            );
        },
    );
}

fn header_row(builder: &mut LayoutBuilder) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(COLUMN_GAP)
            .align_y(AlignY::Center),
        |builder| {
            header_cell(builder, "REQUEST → RESOLVED", LABEL_COLUMN_WIDTH);
            header_cell(builder, "OFF: EXACT REQUEST", SAMPLE_COLUMN_WIDTH);
            header_cell(builder, "ON: NEAREST INTEGRAL", SAMPLE_COLUMN_WIDTH);
        },
    );
}

fn header_cell(builder: &mut LayoutBuilder, text: &'static str, width: Px) {
    builder.with(
        El::row()
            .width(Sizing::fixed(width))
            .height(Sizing::FIT)
            .align_y(AlignY::Center),
        |builder| {
            builder.text((text, text_style(HEADER_SIZE, HEADER_COLOR)));
        },
    );
}

fn comparison_row(builder: &mut LayoutBuilder, comparison: SizeComparison) {
    let largest_size = comparison.requested.0.max(comparison.integral.0);
    let row_height = Px(largest_size
        .mul_add(ROW_HEIGHT_PER_POINT, ROW_HEIGHT_PADDING)
        .max(MINIMUM_ROW_HEIGHT));
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::fixed(row_height))
            .padding(Padding::all(ROW_PADDING))
            .gap(COLUMN_GAP)
            .align_y(AlignY::Center)
            .background(ROW_BACKGROUND)
            .border(Border::all(1.0, ROW_BORDER_COLOR)),
        |builder| {
            size_label(builder, comparison);
            sample_cell(builder, comparison.requested);
            sample_cell(builder, comparison.integral);
        },
    );
}

fn size_label(builder: &mut LayoutBuilder, comparison: SizeComparison) {
    builder.with(
        El::row()
            .width(Sizing::fixed(LABEL_COLUMN_WIDTH))
            .height(Sizing::GROW)
            .align_y(AlignY::Center),
        |builder| {
            builder.text((
                format!(
                    "{:.2} → {:.2} pt",
                    comparison.requested.0, comparison.integral.0
                ),
                text_style(SIZE_LABEL_SIZE, SIZE_LABEL_COLOR),
            ));
        },
    );
}

fn sample_cell(builder: &mut LayoutBuilder, size: Pt) {
    builder.with(
        El::row()
            .width(Sizing::fixed(SAMPLE_COLUMN_WIDTH))
            .height(Sizing::GROW)
            .align_y(AlignY::Center),
        |builder| {
            builder.text((SAMPLE_TEXT, text_style(size, SAMPLE_COLOR)));
        },
    );
}

fn divider(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(DIVIDER_HEIGHT))
            .background(DIVIDER_COLOR),
        |_| {},
    );
}

fn text_style(size: Pt, color: Color) -> TextStyle {
    TextStyle::new(size)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}
