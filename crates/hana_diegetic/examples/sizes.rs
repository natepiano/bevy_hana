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
use bevy::prelude::DirectionalLight;
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
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use bevy_kana::ToF32;
use hana_clerestory::WindowManagerPlugin;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::CascadeEntityCommandsExt;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::In;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_lagrange::Focus;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitAngles;
use hana_lagrange::OrbitCam;
use hana_lagrange::Radius;
use hana_lagrange::ScalarLimit;
use hana_lagrange::ZoomToFit;

// ── Layout ───────────────────────────────────────────────────────────
const PANEL_WIDTH: f32 = 60.0; // mm
const PANEL_HEIGHT: f32 = 42.0; // mm — content only (no title row)
const PANEL_PAD: f32 = 3.0; // mm
const ROW_SPACING: f32 = 1.5; // mm between rows
const LABEL_COL: f32 = 22.0; // mm — fixed label column
const COLUMN_GAP: f32 = 0.008; // meters between columns
const HEADER_GAP: f32 = 0.003; // meters between header and content
const MARGIN: f32 = 0.004; // meters — backdrop overshoot
const MILLIMETERS_PER_METER: f32 = 0.001;

// ── Commentary panel ─────────────────────────────────────────────────
const NOTE_WIDTH: f32 = 80.0; // mm
const NOTE_HEIGHT: f32 = 50.0; // mm — content only (no title row)
const NOTE_PAD: f32 = 3.5; // mm
const NOTE_FONT: f32 = 8.0; // pt

// ── Sizes — all target ~6.35mm (18pt) ────────────────────────────────
const SIZE_PT: f32 = 18.0; // 18pt = 6.35mm
const SIZE_MM: f32 = 6.35; // 6.35mm
const SIZE_IN: f32 = 0.25; // 1/4 inch = 6.35mm
const SIZE_BARE_PANEL: f32 = 18.0; // 18pt (panel font_unit = Points)
const SIZE_BARE_WORLD: f32 = 0.00635; // 6.35mm in meters

// ── Visual ───────────────────────────────────────────────────────────
const LABEL_COLOR: Color = Color::srgba(0.55, 0.55, 0.55, 0.9);
const SAMPLE_COLOR: Color = Color::WHITE;
const HEADER_COLOR: Color = Color::srgba(0.65, 0.75, 0.95, 1.0);
const BORDER_COLOR: Color = Color::srgba(0.35, 0.35, 0.45, 0.5);
const BORDER_WIDTH: f32 = 0.25; // mm

// ── Row labels ───────────────────────────────────────────────────────
const WORLD_LABELS: &[&str] = &["Pt(18)", "Mm(6.35)", "In(0.25)", "0.00635"];

// ── Zoom ─────────────────────────────────────────────────────────────
const ZOOM_MARGIN: f32 = 0.06;
const ZOOM_DURATION_MS: u64 = 600;

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
        .add_systems(Startup, setup)
        .add_systems(PostStartup, fit_camera_on_start)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let panel_width = PANEL_WIDTH * MILLIMETERS_PER_METER;
    let panel_height = PANEL_HEIGHT * MILLIMETERS_PER_METER;
    let note_width = NOTE_WIDTH * MILLIMETERS_PER_METER;
    let note_height = NOTE_HEIGHT * MILLIMETERS_PER_METER;

    // Three columns: demo panel | WorldText | commentary.
    // WorldText column is narrower than the panel — size to actual content.
    let world_text_column_width = (LABEL_COL + 28.0) * MILLIMETERS_PER_METER; // label col + "Hello" width
    let total_width = panel_width + COLUMN_GAP + world_text_column_width + COLUMN_GAP + note_width;
    let left_x = -total_width / 2.0;
    let right_x = left_x + panel_width + COLUMN_GAP;
    let note_x = right_x + world_text_column_width + COLUMN_GAP;

    // All three headers top-align at this Y. Content sits below.
    let header_style = TextStyle::new(9.0)
        .with_color(HEADER_COLOR)
        .with_anchor(Anchor::TopLeft);
    let max_content_height = panel_height.max(note_height);
    let lift = max_content_height * 0.10; // shift everything up 10%
    let header_y = max_content_height + HEADER_GAP + lift;
    let content_top = Pt(9.0)
        .0
        .mul_add(-Unit::Points.meters_per_unit(), header_y - HEADER_GAP);

    let total_height = header_y;
    spawn_backdrop(
        &mut commands,
        &mut meshes,
        &mut materials,
        total_width,
        total_height,
    );
    spawn_headers(&mut commands, &header_style, left_x, note_x, header_y);
    spawn_panels(&mut commands, left_x, note_x, content_top);
    spawn_world_text_column(&mut commands, header_style, right_x, header_y);
    spawn_lighting_and_camera(&mut commands, total_height);
}

fn spawn_backdrop(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    total_width: f32,
    total_height: f32,
) {
    let backdrop = commands
        .spawn((
            Mesh3d(meshes.add(Plane3d::new(
                Vec3::Z,
                Vec2::new(total_width / 2.0 + MARGIN, total_height / 2.0 + MARGIN),
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
            DiegeticText::world("DiegeticPanel")
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

fn spawn_panels(commands: &mut Commands, left_x: f32, note_x: f32, content_top: f32) {
    let demo_panel = DiegeticPanel::world()
        .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
        .anchor(Anchor::TopLeft)
        .with_tree(build_demo_panel())
        .build();
    let Ok(demo_panel) = demo_panel else {
        error!("failed to build demo panel dimensions");
        return;
    };
    commands.spawn((demo_panel, Transform::from_xyz(left_x, content_top, 0.0)));

    let commentary_panel = DiegeticPanel::world()
        .size(Mm(NOTE_WIDTH), Mm(NOTE_HEIGHT))
        .anchor(Anchor::TopLeft)
        .with_tree(build_commentary())
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
) {
    let wt_title = commands
        .spawn(
            DiegeticText::world("WorldText")
                .style(header_style)
                .transform(Transform::from_xyz(right_x, header_y, 0.0))
                .build(),
        )
        .override_font_unit(Unit::Points)
        .id();

    let label_style = TextStyle::new(8.0)
        .with_color(LABEL_COLOR)
        .with_anchor(Anchor::TopLeft);

    let world_styles: &[(TextStyle, Option<Unit>)] = &[
        (
            TextStyle::new(SIZE_PT).with_color(SAMPLE_COLOR),
            Some(Unit::Points),
        ),
        (
            TextStyle::new(SIZE_MM).with_color(SAMPLE_COLOR),
            Some(Unit::Millimeters),
        ),
        (
            TextStyle::new(SIZE_IN).with_color(SAMPLE_COLOR),
            Some(Unit::Inches),
        ),
        (
            TextStyle::new(SIZE_BARE_WORLD).with_color(SAMPLE_COLOR),
            None,
        ),
    ];

    let first_row_dy = -(PANEL_PAD + BORDER_WIDTH).mul_add(MILLIMETERS_PER_METER, HEADER_GAP);
    let sample_dx = LABEL_COL * MILLIMETERS_PER_METER;
    let row_step = (SIZE_MM + ROW_SPACING + 2.0) * MILLIMETERS_PER_METER;

    for (i, (label, (style, unit))) in WORLD_LABELS.iter().zip(world_styles.iter()).enumerate() {
        let dy = first_row_dy - row_step * i.to_f32();

        commands.entity(wt_title).with_children(|parent| {
            parent
                .spawn(
                    DiegeticText::world(*label)
                        .style(label_style.clone())
                        .transform(Transform::from_xyz(0.0, dy, 0.0))
                        .build(),
                )
                .override_font_unit(Unit::Points);
        });
        commands.entity(wt_title).with_children(|parent| {
            let mut sample = parent.spawn(
                DiegeticText::world("Hello")
                    .style(style.clone().with_anchor(Anchor::TopLeft))
                    .transform(Transform::from_xyz(sample_dx, dy, 0.0))
                    .build(),
            );
            if let Some(unit) = *unit {
                sample.override_font_unit(unit);
            }
        });
    }
}

fn spawn_lighting_and_camera(commands: &mut Commands, total_height: f32) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.5, 1.5, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(-0.5, 1.5, -1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

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
fn build_demo_panel() -> hana_diegetic::LayoutTree {
    let label_style = TextStyle::new(Pt(8.0)).with_color(LABEL_COLOR);

    let sample_styles: &[(&str, TextStyle)] = &[
        (
            "Pt(18)",
            TextStyle::new(Pt(SIZE_PT)).with_color(SAMPLE_COLOR),
        ),
        (
            "Mm(6.35)",
            TextStyle::new(Mm(SIZE_MM)).with_color(SAMPLE_COLOR),
        ),
        (
            "In(0.25)",
            TextStyle::new(In(SIZE_IN)).with_color(SAMPLE_COLOR),
        ),
        (
            "18.0",
            TextStyle::new(SIZE_BARE_PANEL).with_color(SAMPLE_COLOR),
        ),
    ];

    let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
    builder.with(
        El::column()
            .padding(Padding::all(PANEL_PAD))
            .gap(ROW_SPACING)
            .border(Border::all(BORDER_WIDTH, BORDER_COLOR))
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0)),
        |b| {
            for (label, sample_style) in sample_styles {
                b.with(
                    El::row()
                        .width(Sizing::grow_min(0.0))
                        .height(Sizing::fit_min(0.0)),
                    |b| {
                        b.with(
                            El::new()
                                .width(Sizing::fixed(LABEL_COL))
                                .height(Sizing::fit_min(0.0)),
                            |b| {
                                b.text((*label, label_style.clone()));
                            },
                        );
                        b.with(
                            El::new()
                                .width(Sizing::grow_min(0.0))
                                .height(Sizing::fit_min(0.0)),
                            |b| {
                                b.text(("Hello", sample_style.clone()));
                            },
                        );
                    },
                );
            }
        },
    );
    builder.build()
}

/// Commentary panel: word-wrapped explanation. Title is external.
fn build_commentary() -> hana_diegetic::LayoutTree {
    let note_color = Color::srgba(0.72, 0.72, 0.72, 0.95);
    let note_style = TextStyle::new(Pt(NOTE_FONT)).with_color(note_color);

    let mut builder = LayoutBuilder::new(NOTE_WIDTH, NOTE_HEIGHT);
    builder.with(
        El::column()
            .padding(Padding::all(NOTE_PAD))
            .gap(2.5)
            .border(Border::all(BORDER_WIDTH, BORDER_COLOR))
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0)),
        |b| {
            b.text((
                "Pt (Points), Mm (Millimeters), and In (Inches) are \
                 newtypes that carry the unit. Pt(18), Mm(6.35), and \
                 In(0.25) all describe the same physical size — the \
                 constructor knows how to convert.",
                note_style.clone(),
            ));
            b.text((
                "Bare 18.0 uses the contextual default: panel text \
                 inherits font_unit from the panel (Points by default). \
                 WorldText resolves font_unit from the FontUnit resource \
                 (Meters by default).",
                note_style.clone(),
            ));
            b.text((
                "Both the DiegeticPanel and the individual WorldText \
                 instances render \"Hello\" at the same height, \
                 showing that Pt, Mm, In, and bare f32 all convert \
                 consistently.",
                note_style,
            ));
        },
    );
    builder.build()
}
