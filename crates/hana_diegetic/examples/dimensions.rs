//! @generated `bevy_example_template`
//! Layout dimensions — `Dimension` newtypes in spatial properties.
//!
//! Demonstrates that every spatial property in the layout system —
//! padding, border width, child gap, element sizing, and font size —
//! accepts `Pt`, `Mm`, `In`, or bare `f32` via the `Dimension` type.
//!
//! The left panel shows code snippets of each property using different
//! newtypes. The right panel explains how `Dimension` works.

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
const DEMO_WIDTH: f32 = 90.0; // mm
const DEMO_HEIGHT: f32 = 60.0; // mm
const COLUMN_GAP: f32 = 0.008; // meters
const HEADER_GAP: f32 = 0.003; // meters
const MARGIN: f32 = 0.004; // meters
const MILLIMETERS_TO_METERS: f32 = 0.001;

// ── Commentary ───────────────────────────────────────────────────────
const NOTE_WIDTH: f32 = 85.0; // mm
const NOTE_HEIGHT: f32 = 60.0; // mm
const NOTE_PAD: f32 = 3.5; // mm
const NOTE_FONT: f32 = 8.0; // pt

// ── Visual ───────────────────────────────────────────────────────────
const LABEL_COLOR: Color = Color::srgba(0.55, 0.55, 0.55, 0.9);
const CODE_COLOR: Color = Color::srgba(0.85, 0.75, 0.55, 1.0);
const SAMPLE_COLOR: Color = Color::WHITE;
const HEADER_COLOR: Color = Color::srgba(0.65, 0.75, 0.95, 1.0);
const BOX_COLOR: Color = Color::srgba(0.22, 0.25, 0.32, 0.6);
const CONTENT_BG: Color = Color::srgba(0.18, 0.20, 0.25, 0.4);
const PAD_LABEL_COLOR: Color = Color::srgba(0.5, 0.65, 0.5, 0.8);

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
    let demo_width = DEMO_WIDTH * MILLIMETERS_TO_METERS;
    let demo_height = DEMO_HEIGHT * MILLIMETERS_TO_METERS;
    let note_width = NOTE_WIDTH * MILLIMETERS_TO_METERS;
    let note_height = NOTE_HEIGHT * MILLIMETERS_TO_METERS;

    let total_width = demo_width + COLUMN_GAP + note_width;
    let left_x = -total_width / 2.0;
    let note_x = left_x + demo_width + COLUMN_GAP;
    let max_height = demo_height.max(note_height);
    let header_y = max_height + HEADER_GAP;
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
    spawn_headers(&mut commands, left_x, note_x, header_y);
    spawn_panels(&mut commands, left_x, note_x, content_top);
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

fn spawn_headers(commands: &mut Commands, left_x: f32, note_x: f32, header_y: f32) {
    commands
        .spawn(
            DiegeticText::world("Dimension in layout properties")
                .size(9.0)
                .color(HEADER_COLOR)
                .anchor(Anchor::TopLeft)
                .transform(Transform::from_xyz(left_x, header_y, 0.0))
                .build(),
        )
        .override_font_unit(Unit::Points);
    commands
        .spawn(
            DiegeticText::world("How it works")
                .size(9.0)
                .color(HEADER_COLOR)
                .anchor(Anchor::TopLeft)
                .transform(Transform::from_xyz(note_x, header_y, 0.0))
                .build(),
        )
        .override_font_unit(Unit::Points);
}

fn spawn_panels(commands: &mut Commands, left_x: f32, note_x: f32, content_top: f32) {
    let demo_panel = DiegeticPanel::world()
        .size(Mm(DEMO_WIDTH), Mm(DEMO_HEIGHT))
        .anchor(Anchor::TopLeft)
        .with_tree(build_demo())
        .build();
    let Ok(demo_panel) = demo_panel else {
        error!("failed to build demo panel dimensions");
        return;
    };

    commands.spawn((demo_panel, Transform::from_xyz(left_x, content_top, 0.0)));

    let note_panel = DiegeticPanel::world()
        .size(Mm(NOTE_WIDTH), Mm(NOTE_HEIGHT))
        .anchor(Anchor::TopLeft)
        .with_tree(build_commentary())
        .build();
    let Ok(note_panel) = note_panel else {
        error!("failed to build note panel dimensions");
        return;
    };

    commands.spawn((note_panel, Transform::from_xyz(note_x, content_top, 0.0)));
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

/// Demo panel showing every spatial property using `Dimension` newtypes.
/// The panel `layout_unit` is `Millimeters`, so bare f32 values are in mm.
///
/// Structure: outer border -> padding (visible gap) -> content background.
/// Each padding side is labeled with its unit. Inner content shows
/// `gap` and fixed sizing.
fn build_demo() -> hana_diegetic::LayoutTree {
    let code = TextStyle::new(Pt(7.0)).with_color(CODE_COLOR);
    let label = TextStyle::new(Pt(6.0)).with_color(LABEL_COLOR);
    let pad_label = TextStyle::new(Pt(5.5)).with_color(PAD_LABEL_COLOR);
    let text = TextStyle::new(Pt(10.0)).with_color(SAMPLE_COLOR);

    let mut builder = LayoutBuilder::new(DEMO_WIDTH, DEMO_HEIGHT);

    // Outer container: border + padding. The padding is the visible gap
    // between the border and the content background.
    builder.with(
        El::column()
            .padding(Padding::new(
                Mm(5.0),   // left: Mm
                Mm(5.0),   // right: Mm
                Pt(14.17), // top: Pt (≈ 5mm)
                In(0.197), // bottom: In (≈ 5mm)
            ))
            .border(Border::all(Mm(1.0), CODE_COLOR))
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0)),
        |b| {
            // Padding labels — sit in the padding space.
            b.text(("top: Pt(14.17)", pad_label.clone()));

            // Content area with visible background.
            b.with(
                El::column()
                    .padding(Padding::all(2.0))
                    .gap(Mm(2.0))
                    .background(CONTENT_BG)
                    .border(Border::all(Mm(0.8), CODE_COLOR))
                    .width(Sizing::grow_min(0.0))
                    .height(Sizing::grow_min(0.0)),
                |b| {
                    // Padding description.
                    b.text((
                        "Padding: Mm(5) left/right, Pt(14) top, In(0.2) bottom",
                        code.clone(),
                    ));
                    b.text(("visible gap between border and this box", label.clone()));

                    // Border description.
                    b.text(("Border::all(Mm(1.0), color)", code.clone()));

                    // Gap description.
                    b.text(("gap(Mm(2.0)) — space between rows", code.clone()));

                    // Fixed-size boxes.
                    b.text(("Sizing::fixed(Mm(8.0)):", code.clone()));
                    b.with(
                        El::row()
                            .gap(Mm(2.0))
                            .width(Sizing::grow_min(0.0))
                            .height(Sizing::fit_min(0.0)),
                        |b| {
                            for label_text in &["A", "B", "C", "D"] {
                                b.with(
                                    El::new()
                                        .width(Sizing::fixed(Mm(8.0)))
                                        .height(Sizing::fixed(Mm(8.0)))
                                        .background(BOX_COLOR),
                                    |b| {
                                        b.text((*label_text, text.clone()));
                                    },
                                );
                            }
                        },
                    );
                },
            );

            // Bottom padding label.
            b.text(("bottom: In(0.197)", pad_label));
        },
    );
    builder.build()
}

/// Commentary panel.
fn build_commentary() -> hana_diegetic::LayoutTree {
    let note_color = Color::srgba(0.72, 0.72, 0.72, 0.95);
    let note = TextStyle::new(Pt(NOTE_FONT)).with_color(note_color);

    let mut builder = LayoutBuilder::new(NOTE_WIDTH, NOTE_HEIGHT);
    builder.with(
        El::column()
            .padding(Padding::all(NOTE_PAD))
            .gap(2.5)
            .border(Border::all(Mm(1.0), CODE_COLOR))
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0)),
        |b| {
            b.text((
                "The Dimension type — used by Pt (Points), \
                 Mm (Millimeters), In (Inches), and bare f32 — \
                 is accepted by every spatial property in the \
                 layout system.",
                note.clone(),
            ));
            b.text((
                "Padding, Border width, gap, Sizing, \
                 and font size all take impl Into<Dimension>. \
                 This means you can mix units freely within \
                 a single element.",
                note.clone(),
            ));
            b.text((
                "Bare f32 uses the panel's layout_unit \
                 (Millimeters in this example). Newtypes \
                 carry their unit explicitly.",
                note.clone(),
            ));
            b.text((
                "The demo panel on the left uses Mm for \
                 padding sides, Pt for top padding, In for \
                 bottom padding, Mm for borders and gaps, \
                 and Mm for fixed box sizing — all in one \
                 layout tree.",
                note,
            ));
        },
    );
    builder.build()
}
