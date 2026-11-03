//! @generated `bevy_example_template`
//! Font size newtypes — `Pt`, `Mm`, `In`, and bare `f32`.
//!
//! Left: a [`DiegeticPanel`] with four rows, each using a different size
//! newtype. Center: standalone [`WorldText`] entities with matching sizes.
//! Right: commentary panel explaining how the unit system works.
//!
//! All "Hello" samples render at the same height (~6.35mm), showing that
//! the unit system converts consistently across both rendering paths.

use std::time::Duration;

use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::Entity;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::PerspectiveProjection;
use bevy::prelude::Plane3d;
use bevy::prelude::PostStartup;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Vec4;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use hana_clerestory::WindowManagerPlugin;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::CascadeEntityCommandsExt;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::Font;
use hana_diegetic::FontId;
use hana_diegetic::FontRegistry;
use hana_diegetic::In;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextInkMeasurement;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Unit;
use hana_kana::ToF32;
use hana_lagrange::Focus;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitAngles;
use hana_lagrange::OrbitCam;
use hana_lagrange::Radius;
use hana_lagrange::ScalarLimit;
use hana_lagrange::ZoomToFit;

// ── Layout ───────────────────────────────────────────────────────────
const PANEL_PAD: f32 = 3.0; // mm
const HELLO_ROW_PITCH_RATIO: f32 = 1.25;
const INNER_GUTTER: f32 = 3.0; // mm between a label and its Hello
const COLUMN_GAP: f32 = 0.012; // meters between columns
const HEADER_GAP: f32 = 0.003; // meters between header and content
const HEADER_FONT_PT: f32 = 9.0;
const MARGIN: f32 = 0.004; // meters — backdrop overshoot
const MILLIMETERS_PER_METER: f32 = 0.001;

// ── Commentary panel ─────────────────────────────────────────────────
const NOTE_PAD: f32 = 3.5; // mm
const NOTE_FONT: f32 = 8.0; // pt
const LABEL_FONT: f32 = 8.0; // pt
const NOTE_PARAGRAPHS: [&str; 3] = [
    "18 pt, 6.35 mm and 0.25 in are the same height.",
    "A size without a unit uses points in panels.\nWorld text uses meters unless a unit is chosen.",
    "Both columns show Hello at the same height.",
];

// ── Sizes — all target ~6.35mm (18pt) ────────────────────────────────
const SIZE_PT: f32 = 18.0; // 18pt = 6.35mm
const SIZE_MM: f32 = 6.35; // 6.35mm
const SIZE_IN: f32 = 0.25; // 1/4 inch = 6.35mm
const SIZE_BARE_PANEL: f32 = SIZE_PT; // panel font_unit = Points
const SIZE_BARE_WORLD: f32 = 0.00635; // 6.35mm in meters

// ── Visual ───────────────────────────────────────────────────────────
const SECONDARY_TEXT_COLOR: Color = Color::srgb(180.0 / 255.0, 180.0 / 255.0, 180.0 / 255.0);
const SAMPLE_COLOR: Color = Color::WHITE;
const HEADER_COLOR: Color = Color::srgba(0.65, 0.75, 0.95, 1.0);
const PANEL_BACKGROUND: Color = Color::srgb(46.0 / 255.0, 52.0 / 255.0, 64.0 / 255.0);
const BORDER_COLOR: Color = Color::srgba(129.0 / 255.0, 161.0 / 255.0, 193.0 / 255.0, 0.85);
const BORDER_WIDTH: f32 = 0.25; // mm

// ── Row labels ───────────────────────────────────────────────────────
const WORLD_LABELS: &[&str] = &["18 pt", "6.35 mm", "0.25 in", "0.00635 m"];
const PANEL_LABELS: &[&str] = &["18 pt", "6.35 mm", "0.25 in", "18 (default)"];
const SAMPLE_TEXT: &str = "Hello";

// ── Zoom ─────────────────────────────────────────────────────────────
const ZOOM_MARGIN: f32 = 0.06;
const ZOOM_DURATION_MS: u64 = 600;
const PANEL_RADIUS_LOGICAL_PIXELS: f32 = 12.0;

#[derive(Resource)]
struct Backdrop(Entity);

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            LagrangePlugin,
            BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
            WindowManagerPlugin,
            MeshPickingPlugin,
            DiegeticUiPlugin,
        ))
        .insert_resource(Lighting::Unlit)
        .add_systems(Startup, setup)
        .add_systems(PostStartup, fit_camera_on_start)
        .run();
}

struct SizesLayout {
    panel_width:        f32,
    panel_height:       f32,
    panel_top_pad:      f32,
    note_height:        f32,
    note_width:         f32,
    world_column_width: f32,
    label_column:       f32,
    row_pitch:          f32,
    row_gap:            f32,
}

struct DemoColumns {
    panel_left:  f32,
    world_left:  f32,
    note_left:   f32,
    total_width: f32,
}

#[derive(Clone, Copy)]
struct BackdropExtentMeters {
    width:  f32,
    height: f32,
}

#[derive(Clone, Copy)]
enum FontUnitChoice {
    Contextual,
    Explicit(Unit),
}

#[derive(Clone, Copy)]
struct TextEdgeMetrics {
    top_trim:    Pt,
    bottom_trim: Pt,
    baseline:    f32,
}

#[derive(Clone, Copy)]
struct SizesTextEdges {
    label:  TextEdgeMetrics,
    sample: TextEdgeMetrics,
    note:   TextEdgeMetrics,
}

fn measure_text_edges(font: &Font, size: f32) -> TextEdgeMetrics {
    let metrics = font.metrics(size);
    TextEdgeMetrics {
        top_trim:    Pt(metrics.line_gap.mul_add(0.5, metrics.ascent) - metrics.cap_height),
        bottom_trim: Pt(metrics.line_gap.mul_add(0.5, metrics.descent)),
        baseline:    metrics.line_gap.mul_add(0.5, metrics.ascent),
    }
}

impl SizesTextEdges {
    fn measure(font: &Font) -> Self {
        Self {
            label:  measure_text_edges(font, LABEL_FONT),
            sample: measure_text_edges(font, SIZE_PT),
            note:   measure_text_edges(font, NOTE_FONT),
        }
    }
}

fn demo_columns(layout: &SizesLayout) -> DemoColumns {
    let panel_width = layout.panel_width * MILLIMETERS_PER_METER;
    let world_width = layout.world_column_width * MILLIMETERS_PER_METER;
    let note_width = layout.note_width * MILLIMETERS_PER_METER;
    let total_width = panel_width + COLUMN_GAP + world_width + COLUMN_GAP + note_width;
    let panel_left = -total_width / 2.0;
    let world_left = panel_left + panel_width + COLUMN_GAP;
    let note_left = world_left + world_width + COLUMN_GAP;
    DemoColumns {
        panel_left,
        world_left,
        note_left,
        total_width,
    }
}

const fn backdrop_extent(total_width: f32, total_height: f32) -> BackdropExtentMeters {
    BackdropExtentMeters {
        width:  2.0_f32.mul_add(MARGIN, total_width),
        height: 2.0_f32.mul_add(MARGIN, total_height),
    }
}

fn home_panel_radius_mm(backdrop_extent: BackdropExtentMeters, window_size: Vec2) -> f32 {
    let world_meters_per_logical_pixel = (backdrop_extent.width / window_size.x)
        .max(backdrop_extent.height / window_size.y)
        / (1.0 - ZOOM_MARGIN);
    PANEL_RADIUS_LOGICAL_PIXELS * world_meters_per_logical_pixel / MILLIMETERS_PER_METER
}

impl SizesLayout {
    fn measure(
        text_measurer: &DiegeticTextMeasurer,
        fonts: &FontRegistry,
        edges: SizesTextEdges,
    ) -> Self {
        let points_per_mm = Unit::Millimeters.to_points();
        let sample_measure = TextStyle::new(Pt(SIZE_PT)).as_measure();
        let sample = (text_measurer.measure_fn)(SAMPLE_TEXT, &sample_measure);
        let label_measure = TextStyle::new(Pt(LABEL_FONT)).as_measure();
        let sample_height =
            sample.line_height - edges.sample.top_trim.0 - edges.sample.bottom_trim.0;
        let row_pitch = SIZE_PT * HELLO_ROW_PITCH_RATIO / points_per_mm;
        let row_gap = row_pitch - sample_height / points_per_mm;
        let leftmost_label_ink = PANEL_LABELS
            .iter()
            .filter_map(|label| match fonts.measure_ink(label, &label_measure) {
                Ok(TextInkMeasurement::Visible(ink)) => Some(ink.left),
                Ok(TextInkMeasurement::NoVisibleInk) | Err(_) => None,
            })
            .fold(f32::INFINITY, f32::min);
        let sample_ink = match fonts.measure_ink(SAMPLE_TEXT, &sample_measure) {
            Ok(TextInkMeasurement::Visible(ink)) => ink.right,
            Ok(TextInkMeasurement::NoVisibleInk) | Err(_) => sample.width,
        };
        let panel_top_pad = PANEL_PAD + leftmost_label_ink / points_per_mm;
        let label_width = PANEL_LABELS
            .iter()
            .chain(WORLD_LABELS)
            .map(|label| (text_measurer.measure_fn)(label, &label_measure).width)
            .fold(0.0, f32::max)
            / points_per_mm;
        let label_column = label_width + INNER_GUTTER;
        let panel_width = 2.0_f32.mul_add(
            PANEL_PAD + BORDER_WIDTH,
            label_column + (leftmost_label_ink + sample_ink) / points_per_mm,
        );
        let panel_tree = build_demo_panel(
            panel_width,
            label_column,
            panel_top_pad,
            row_gap,
            0.0,
            edges,
        );
        let panel_height = measure_panel_height(&panel_tree, panel_width, text_measurer);
        let note_width = commentary_width(fonts);
        let note_tree = build_commentary(note_width, 0.0, edges);
        let note_height = measure_panel_height(&note_tree, note_width, text_measurer);
        Self {
            panel_width,
            panel_height,
            panel_top_pad,
            note_height,
            note_width,
            world_column_width: label_column + sample.width / points_per_mm,
            label_column,
            row_pitch,
            row_gap,
        }
    }
}

fn measure_panel_height(
    tree: &hana_diegetic::LayoutTree,
    width_mm: f32,
    text_measurer: &DiegeticTextMeasurer,
) -> f32 {
    let points_per_mm = Unit::Millimeters.to_points();
    tree.scaled(points_per_mm, 1.0).measured_height_at_width(
        width_mm * points_per_mm,
        1.0,
        text_measurer.measure_fn.clone(),
    ) / points_per_mm
}

fn commentary_width(fonts: &FontRegistry) -> f32 {
    let points_per_mm = Unit::Millimeters.to_points();
    let measure = TextStyle::new(Pt(NOTE_FONT)).as_measure();
    let (leftmost_ink, rightmost_ink) = NOTE_PARAGRAPHS
        .iter()
        .flat_map(|paragraph| paragraph.lines())
        .filter_map(|line| match fonts.measure_ink(line, &measure) {
            Ok(TextInkMeasurement::Visible(ink)) => Some(ink),
            Ok(TextInkMeasurement::NoVisibleInk) | Err(_) => None,
        })
        .fold((f32::INFINITY, 0.0_f32), |(left, right), ink| {
            (left.min(ink.left), right.max(ink.right))
        });
    2.0_f32.mul_add(
        NOTE_PAD + BORDER_WIDTH,
        (leftmost_ink + rightmost_ink) / points_per_mm,
    )
}

fn first_world_row_offset(panel_top_pad: f32, sample_top_trim: Pt) -> f32 {
    let offset = (panel_top_pad + BORDER_WIDTH).mul_add(
        MILLIMETERS_PER_METER,
        HEADER_FONT_PT.mul_add(Unit::Points.meters_per_unit(), HEADER_GAP),
    );
    sample_top_trim
        .0
        .mul_add(Unit::Points.meters_per_unit(), -offset)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    text_measurer: Res<DiegeticTextMeasurer>,
    fonts: Res<FontRegistry>,
    windows: Query<&Window>,
) {
    let font = match fonts.font(FontId::MONOSPACE) {
        Ok(font) => font,
        Err(error) => {
            error!("sizes example: missing monospace font for text layout: {error}");
            return;
        },
    };
    let edges = SizesTextEdges::measure(font);
    let layout = SizesLayout::measure(&text_measurer, &fonts, edges);
    let Ok(window) = windows.single() else {
        error!("sizes example: missing window for home panel radius");
        return;
    };
    let panel_height = layout.panel_height * MILLIMETERS_PER_METER;
    let note_height = layout.note_height * MILLIMETERS_PER_METER;
    let row_step = layout.row_pitch * MILLIMETERS_PER_METER;
    let columns = demo_columns(&layout);

    // All three headers top-align at this Y. Content sits below.
    let header_style = TextStyle::new(HEADER_FONT_PT)
        .with_color(HEADER_COLOR)
        .with_anchor(Anchor::TopLeft);
    let max_content_height = panel_height.max(note_height);
    let lift = max_content_height * 0.10; // shift everything up 10%
    let header_y = max_content_height + HEADER_GAP + lift;
    let content_top = Pt(HEADER_FONT_PT)
        .0
        .mul_add(-Unit::Points.meters_per_unit(), header_y - HEADER_GAP);

    let total_height = header_y;
    let backdrop_extent = backdrop_extent(columns.total_width, total_height);
    let panel_radius =
        home_panel_radius_mm(backdrop_extent, Vec2::new(window.width(), window.height()));
    spawn_backdrop(
        &mut commands,
        &mut meshes,
        &mut materials,
        backdrop_extent,
        total_height,
    );
    spawn_headers(
        &mut commands,
        &header_style,
        columns.panel_left,
        columns.note_left,
        header_y,
    );
    spawn_panels(
        &mut commands,
        columns.panel_left,
        columns.note_left,
        content_top,
        panel_radius,
        &layout,
        edges,
    );
    spawn_world_text_column(
        &mut commands,
        header_style,
        columns.world_left,
        header_y,
        row_step,
        layout.label_column,
        layout.panel_top_pad,
        edges,
    );
    spawn_camera(&mut commands, total_height);
}

fn spawn_backdrop(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    backdrop_extent: BackdropExtentMeters,
    total_height: f32,
) {
    let backdrop = commands
        .spawn((
            Mesh3d(meshes.add(Plane3d::new(
                Vec3::Z,
                Vec2::new(backdrop_extent.width / 2.0, backdrop_extent.height / 2.0),
            ))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgba(0.12, 0.12, 0.14, 0.0),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(0.0, total_height / 2.0, -0.001),
        ))
        .id();
    commands.insert_resource(Backdrop(backdrop));
}

fn spawn_headers(
    commands: &mut Commands,
    header_style: &TextStyle,
    left_x: f32,
    note_x: f32,
    header_y: f32,
) {
    commands
        .spawn(
            DiegeticText::world("Panel text")
                .style(header_style.clone())
                .transform(Transform::from_xyz(left_x, header_y, 0.0))
                .build(),
        )
        .override_font_unit(Unit::Points);
    commands
        .spawn(
            DiegeticText::world("How font sizes work")
                .style(header_style.clone())
                .transform(Transform::from_xyz(note_x, header_y, 0.0))
                .build(),
        )
        .override_font_unit(Unit::Points);
}

fn spawn_panels(
    commands: &mut Commands,
    left_x: f32,
    note_x: f32,
    content_top: f32,
    panel_radius: f32,
    layout: &SizesLayout,
    edges: SizesTextEdges,
) {
    let demo_panel = DiegeticPanel::world()
        .size(Mm(layout.panel_width), Mm(layout.panel_height))
        .anchor(Anchor::TopLeft)
        .with_tree(build_demo_panel(
            layout.panel_width,
            layout.label_column,
            layout.panel_top_pad,
            layout.row_gap,
            panel_radius,
            edges,
        ))
        .build();
    let Ok(demo_panel) = demo_panel else {
        error!("failed to build demo panel dimensions");
        return;
    };
    commands.spawn((demo_panel, Transform::from_xyz(left_x, content_top, 0.0)));

    let commentary_panel = DiegeticPanel::world()
        .size(Mm(layout.note_width), Mm(layout.note_height))
        .anchor(Anchor::TopLeft)
        .with_tree(build_commentary(layout.note_width, panel_radius, edges))
        .build();
    let Ok(commentary_panel) = commentary_panel else {
        error!("failed to build commentary panel dimensions");
        return;
    };
    commands.spawn((
        commentary_panel,
        Transform::from_xyz(note_x, content_top, 0.0),
    ));
}

fn spawn_world_text_column(
    commands: &mut Commands,
    header_style: TextStyle,
    right_x: f32,
    header_y: f32,
    row_step: f32,
    label_column: f32,
    panel_top_pad: f32,
    edges: SizesTextEdges,
) {
    let wt_title = commands
        .spawn(
            DiegeticText::world("World text")
                .style(header_style)
                .transform(Transform::from_xyz(right_x, header_y, 0.0))
                .build(),
        )
        .override_font_unit(Unit::Points)
        .id();

    let label_style = TextStyle::new(LABEL_FONT)
        .with_color(SECONDARY_TEXT_COLOR)
        .with_anchor(Anchor::TopLeft);

    let world_styles: &[(TextStyle, FontUnitChoice)] = &[
        (
            TextStyle::new(SIZE_PT).with_color(SAMPLE_COLOR),
            FontUnitChoice::Explicit(Unit::Points),
        ),
        (
            TextStyle::new(SIZE_MM).with_color(SAMPLE_COLOR),
            FontUnitChoice::Explicit(Unit::Millimeters),
        ),
        (
            TextStyle::new(SIZE_IN).with_color(SAMPLE_COLOR),
            FontUnitChoice::Explicit(Unit::Inches),
        ),
        (
            TextStyle::new(SIZE_BARE_WORLD).with_color(SAMPLE_COLOR),
            FontUnitChoice::Contextual,
        ),
    ];

    let first_row_dy = first_world_row_offset(panel_top_pad, edges.sample.top_trim);
    let sample_dx = label_column * MILLIMETERS_PER_METER;
    let baseline_difference =
        (edges.sample.baseline - edges.label.baseline) * Unit::Points.meters_per_unit();

    for (i, (label, (style, unit))) in WORLD_LABELS.iter().zip(world_styles.iter()).enumerate() {
        let dy = row_step.mul_add(-i.to_f32(), first_row_dy);

        commands.entity(wt_title).with_children(|parent| {
            parent
                .spawn(
                    DiegeticText::world(*label)
                        .style(label_style.clone())
                        .transform(Transform::from_xyz(0.0, dy - baseline_difference, 0.0))
                        .build(),
                )
                .override_font_unit(Unit::Points);
        });
        commands.entity(wt_title).with_children(|parent| {
            let mut sample = parent.spawn(
                DiegeticText::world(SAMPLE_TEXT)
                    .style(style.clone().with_anchor(Anchor::TopLeft))
                    .transform(Transform::from_xyz(sample_dx, dy, 0.0))
                    .build(),
            );
            match unit {
                FontUnitChoice::Contextual => {},
                FontUnitChoice::Explicit(unit) => {
                    sample.override_font_unit(*unit);
                },
            }
        });
    }
}

fn spawn_camera(commands: &mut Commands, total_height: f32) {
    let mut camera = OrbitCam::from_pose(
        Focus(Vec3::new(0.0, total_height / 2.0, 0.0)),
        OrbitAngles {
            yaw:   0.0,
            pitch: 0.0,
        },
        Radius(0.25),
    );
    *camera.zoom.limit_mut() = ScalarLimit::Clamp {
        min: 0.000_000_1,
        max: f32::INFINITY,
    };
    commands.spawn((
        camera,
        Projection::Perspective(PerspectiveProjection {
            near: 0.001,
            near_clip_plane: Vec4::new(0.0, 0.0, -1.0, -0.001),
            ..default()
        }),
    ));
}

/// Fires a [`ZoomToFit`] on the backdrop so the camera frames everything.
fn fit_camera_on_start(
    backdrop: Res<Backdrop>,
    cameras: Query<Entity, With<OrbitCam>>,
    mut commands: Commands,
) {
    if let Ok(camera) = cameras.single() {
        commands.trigger(
            ZoomToFit::new(camera, backdrop.0)
                .margin(ZOOM_MARGIN)
                .duration(Duration::from_millis(ZOOM_DURATION_MS)),
        );
    }
}

// ── Panel builders ───────────────────────────────────────────────────

/// Demo panel: four rows of label + sample. Title is external.
fn build_demo_panel(
    panel_width: f32,
    label_column: f32,
    panel_top_pad: f32,
    row_gap: f32,
    corner_radius: f32,
    edges: SizesTextEdges,
) -> hana_diegetic::LayoutTree {
    let label_style = TextStyle::new(Pt(LABEL_FONT)).with_color(SECONDARY_TEXT_COLOR);
    let label_edges = edges.label;
    let sample_edges = edges.sample;

    let sample_styles = [
        TextStyle::new(Pt(SIZE_PT)).with_color(SAMPLE_COLOR),
        TextStyle::new(Mm(SIZE_MM)).with_color(SAMPLE_COLOR),
        TextStyle::new(In(SIZE_IN)).with_color(SAMPLE_COLOR),
        TextStyle::new(SIZE_BARE_PANEL).with_color(SAMPLE_COLOR),
    ];
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(panel_width))
            .height(Sizing::FIT)
            .padding(Padding::new(PANEL_PAD, PANEL_PAD, panel_top_pad, PANEL_PAD))
            .gap(row_gap)
            .background(PANEL_BACKGROUND)
            .border(Border::all(BORDER_WIDTH, BORDER_COLOR))
            .corner_radius(hana_diegetic::CornerRadius::all(Mm(corner_radius))),
    );
    for (label, sample_style) in PANEL_LABELS.iter().zip(sample_styles.iter()) {
        let b = &mut builder;
        b.with(
            El::row()
                .width(Sizing::grow_min(0.0))
                .height(Sizing::fit_min(0.0))
                .alignment(AlignX::Left, AlignY::Bottom),
            |b| {
                b.with(
                    El::new()
                        .width(Sizing::fixed(label_column))
                        .height(Sizing::fit_min(0.0)),
                    |b| {
                        b.text(
                            Text::new(*label, label_style.clone())
                                .trim_text_box(label_edges.top_trim, label_edges.bottom_trim),
                        );
                    },
                );
                b.with(
                    El::new()
                        .width(Sizing::grow_min(0.0))
                        .height(Sizing::fit_min(0.0)),
                    |b| {
                        b.text(
                            Text::new(SAMPLE_TEXT, sample_style.clone())
                                .trim_text_box(sample_edges.top_trim, sample_edges.bottom_trim),
                        );
                    },
                );
            },
        );
    }
    builder.build()
}

/// Commentary panel: word-wrapped explanation. Title is external.
fn build_commentary(
    note_width: f32,
    corner_radius: f32,
    edges: SizesTextEdges,
) -> hana_diegetic::LayoutTree {
    let note_style = TextStyle::new(Pt(NOTE_FONT)).with_color(SECONDARY_TEXT_COLOR);
    let note_edges = edges.note;

    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(note_width))
            .height(Sizing::FIT)
            .padding(Padding::all(NOTE_PAD))
            .gap(2.5)
            .background(PANEL_BACKGROUND)
            .border(Border::all(BORDER_WIDTH, BORDER_COLOR))
            .corner_radius(hana_diegetic::CornerRadius::all(Mm(corner_radius))),
    );
    for (index, paragraph) in NOTE_PARAGRAPHS.iter().enumerate() {
        let top_trim = if index == 0 {
            note_edges.top_trim
        } else {
            Pt(0.0)
        };
        let bottom_trim = if index + 1 == NOTE_PARAGRAPHS.len() {
            note_edges.bottom_trim
        } else {
            Pt(0.0)
        };
        builder.text(
            Text::new(*paragraph, note_style.clone())
                .wrap(TextWrap::Newlines)
                .trim_text_box(top_trim, bottom_trim),
        );
    }
    builder.build()
}
