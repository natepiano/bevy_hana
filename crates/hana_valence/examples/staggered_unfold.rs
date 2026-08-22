//! Five hinged panels form a staged accordion from a visible fixed mount.

use std::time::Duration;

use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::color::Srgba;
use bevy::color::palettes::css::CORAL;
use bevy::color::palettes::css::GOLD;
use bevy::color::palettes::css::MEDIUM_PURPLE;
use bevy::color::palettes::css::SEA_GREEN;
use bevy::color::palettes::css::SILVER;
use bevy::color::palettes::css::SKY_BLUE;
use bevy::color::palettes::css::TURQUOISE;
use bevy::math::Dir3;
use bevy::math::curve::EaseFunction;
use bevy::prelude::Assets;
use bevy::prelude::Bundle;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Cylinder;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::LinearRgba;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Msaa;
use bevy::prelude::On;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Result;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::default;
use bevy_kana::ToF32;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DescriptionPanel;
use fairy_dust::TitleBar;
use hana_diegetic::DiegeticText;
use hana_diegetic::Sidedness;
use hana_lagrange::OrbitCamPreset;
use hana_valence::AnchorPose;
use hana_valence::AnchorSite;
use hana_valence::AnchoredTo;
use hana_valence::Angle;
use hana_valence::Displacement;
use hana_valence::Edge;
use hana_valence::FoldEventTiming;
use hana_valence::FoldMemberBegin;
use hana_valence::FoldSequenceBuilder;
use hana_valence::FoldSequencePlayback;
use hana_valence::FoldTiming;
use hana_valence::Hinge;
use hana_valence::ResolvedAnchorGeometry;
use hana_valence::SequencePosition;

#[path = "../fixtures.rs"]
#[allow(
    dead_code,
    reason = "shared geometry fixtures; this example uses a subset"
)]
mod fixtures;

use fixtures::QUAD_LEFT_EDGE;

// app
const EXAMPLE_TITLE: &str = "Staggered Unfold";

// folding
const FOLD_STAGE: Duration = Duration::from_millis(800);
const FULL_FOLD_ANGLE: f32 = core::f32::consts::PI;
const HALF_FOLD_ANGLE: f32 = core::f32::consts::FRAC_PI_2;
const PANEL_REST_ANGLE: f32 = 0.0;
// Panel 1 stops parallel to the fixed mount. The remaining signs alternate for
// the accordion motion; ±PI close to the same panel plane, forming a stack.
const PANEL_FOLD_ANGLES: [f32; PANEL_COUNT] = [
    HALF_FOLD_ANGLE,
    -FULL_FOLD_ANGLE,
    FULL_FOLD_ANGLE,
    -FULL_FOLD_ANGLE,
    FULL_FOLD_ANGLE,
];
const PANEL_ATTACHMENT_OFFSETS: [Vec3; PANEL_COUNT] = [
    Vec3::new(0.0, 0.0, (PANEL_THICKNESS - MOUNT_DEPTH) / 2.0),
    Vec3::ZERO,
    Vec3::ZERO,
    Vec3::ZERO,
    Vec3::ZERO,
];

// member events
const FLASH_COLOR: Srgba = Srgba::rgb(1.0, 0.85, 0.4);

// camera home
const HOME_MARGIN: f32 = 0.2;
const HOME_PITCH: f32 = 0.36;
const HOME_YAW: f32 = 0.37;

// description panel
const DESCRIPTION_LINES: [&str; 10] = [
    "Gold fixed root owns the chain transform and no fold stage.",
    "Panels 1–5 each own one consecutive authored fold stage.",
    "Segmented knuckles mark each Hinge's invariant pivot axis.",
    "Half-turn stages stack panel faces after panel 1 clears the root.",
    "Space / Shift+Space steps one stage forward / backward.",
    "At a terminal, P selects the other endpoint.",
    "Idle in the interior: P follows the latest step direction.",
    "During a step: P continues that direction to the terminal.",
    "During Play: P reverses immediately.",
    "Each panel flashes when its own fold stage begins.",
];
const DESCRIPTION_TITLE: &str = "Authored Accordion";

// labels
const FIRST_PANEL_NUMBER: usize = 1;
const FIXED_ROOT_LABEL: &str = "FIXED ROOT";
const FIXED_ROOT_LABEL_OFFSET: Vec3 =
    Vec3::new(0.0, MOUNT_HEIGHT / 2.0 + 0.25, MOUNT_DEPTH / 2.0 + 0.03);
const FIXED_ROOT_LABEL_SIZE: f32 = 0.18;
const LABEL_COLOR: Color = Color::BLACK;
const LABEL_SIZE: f32 = 0.32;
const LABEL_Z_OFFSET: f32 = PANEL_THICKNESS / 2.0 + 0.006;

// mount
const BASE_DEPTH: f32 = 0.8;
const BASE_HEIGHT: f32 = 0.14;
const BASE_WIDTH: f32 = 0.9;
const MOUNT_DEPTH: f32 = 0.42;
const MOUNT_HEIGHT: f32 = 1.8;
const MOUNT_POSITION: Vec3 = Vec3::new(
    -PANEL_SPAN / 2.0 - MOUNT_WIDTH / 2.0,
    MOUNT_HEIGHT / 2.0,
    0.0,
);
const METAL_ROUGHNESS: f32 = 0.24;
const MOUNT_WIDTH: f32 = 0.28;

// panels
const HINGE_HEIGHT: f32 = PANEL_HEIGHT + 0.12;
const HINGE_KNUCKLE_COUNT: usize = 3;
const HINGE_KNUCKLE_GAP: f32 = 0.08;
const HINGE_RADIUS: f32 = 0.065;
const PANEL_COLORS: [Srgba; PANEL_COUNT] = [CORAL, SKY_BLUE, SEA_GREEN, MEDIUM_PURPLE, TURQUOISE];
const PANEL_COUNT: usize = 5;
const PANEL_HEIGHT: f32 = 1.2;
const PANEL_ROUGHNESS: f32 = 0.42;
const PANEL_SOURCE_ANCHOR: AnchorSite = AnchorSite::EdgeMidpoint(3);
const PANEL_SPAN: f32 = 7.25;
const PANEL_TARGET_ANCHOR: AnchorSite = AnchorSite::EdgeMidpoint(1);
const PANEL_THICKNESS: f32 = 0.08;
const PANEL_WIDTH: f32 = 1.45;

// scene
const GROUND_SIZE: f32 = 11.0;

#[derive(Clone, Copy)]
enum Face {
    Front,
    Back,
}

struct PanelAssets {
    knuckle_material: Handle<StandardMaterial>,
    knuckle_mesh:     Handle<Mesh>,
    panel_mesh:       Handle<Mesh>,
}

#[derive(Clone, Copy)]
struct PanelJoint {
    attachment_offset: Vec3,
    edge:              Edge,
    folded_angle:      f32,
    pivot_offset:      Vec3,
    source_anchor:     AnchorSite,
    target_anchor:     AnchorSite,
}

impl PanelJoint {
    fn new(folded_angle: f32, attachment_offset: Vec3) -> Self {
        Self {
            attachment_offset,
            edge: QUAD_LEFT_EDGE,
            folded_angle,
            pivot_offset: Vec3::Z * (-folded_angle.signum() * PANEL_THICKNESS / 2.0),
            source_anchor: PANEL_SOURCE_ANCHOR,
            target_anchor: PANEL_TARGET_ANCHOR,
        }
    }

    // `PANEL_REST_ANGLE` lays the panel flat against its parent and
    // `Self::folded_angle` swings it about `Self::edge`. `Self::pivot_offset`
    // becomes the `Hinge` pivot, moving the turn center half a `PANEL_THICKNESS`
    // off the attachment edge and onto the knuckle axis `knuckle_line` draws.
    fn hinge(self) -> Result<Hinge> {
        Ok(Hinge::try_new(
            self.edge,
            Angle::from_radians(PANEL_REST_ANGLE)?,
            Angle::from_radians(self.folded_angle)?,
            Displacement::from(self.pivot_offset),
        )?)
    }

    fn knuckle_line(self, geometry: &ResolvedAnchorGeometry) -> Option<KnuckleLine> {
        let direction = self.edge.axis(geometry).ok()?;
        let source_frame = geometry.frame(self.source_anchor).ok()?;
        Some(KnuckleLine {
            center: source_frame.position().into_inner()
                + source_frame.orientation().into_inner() * self.pivot_offset,
            direction,
        })
    }
}

#[derive(Clone, Copy)]
struct KnuckleLine {
    center:    Vec3,
    direction: Dir3,
}

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    let app = fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .size(GROUND_SIZE)
        .with_orbit_cam_preset_bundle(
            |_| {},
            OrbitCamPreset::blender_like(),
            (Msaa::Off, TemporalAntiAliasing::default()),
        )
        .with_stable_transparency()
        .with_camera_home()
        .pitch(HOME_PITCH)
        .yaw(HOME_YAW)
        .margin(HOME_MARGIN)
        .with_camera_control_panel()
        .with_fold_controls()
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft),
        )
        .with_description_panel(description_panel());
    app.add_observer(flash_panel_that_began_folding)
        .add_systems(Startup, setup)
        .add_systems(Update, fade_panel_flash)
        .run();
}

/// One panel's flash, started by the fold boundary event that named it.
///
/// `elapsed` starts at the travel the sequence already passed when the event
/// arrived, so a seek that lands mid-stage flashes from where the fold actually
/// is instead of restarting the effect. `duration` is the extent the sequence
/// resolved for this panel's own segment, so a panel authored to move for
/// longer also flashes for longer.
#[derive(Component)]
struct PanelFlash {
    elapsed:  Duration,
    duration: Duration,
}

/// Starts one flash per panel whose own fold stage began travelling.
///
/// This is the documented member-observer pattern: the event carries the exact
/// place of the panel's own boundary and the timing resolved for it, the
/// retained sequence already holds the updated position, and nothing here keeps
/// a second progress value of its own.
fn flash_panel_that_began_folding(
    began: On<FoldMemberBegin>,
    mut commands: Commands,
    playbacks: Query<&FoldSequencePlayback>,
) {
    let Ok(playback) = playbacks.get(began.arrangement) else {
        return;
    };
    let duration = began.member_timing.duration;
    commands.entity(began.member).insert(PanelFlash {
        elapsed: catch_up(began.timing, playback.position()).min(duration),
        duration,
    });
}

/// Returns how far past a crossed boundary the sequence already stands.
fn catch_up(timing: FoldEventTiming, position: SequencePosition) -> Duration {
    let travelled = f64::from(position.normalized()) - f64::from(timing.position().normalized());
    Duration::try_from_secs_f64(travelled.abs() * timing.total().as_seconds_f64())
        .unwrap_or_default()
}

/// Fades every running flash out and removes the finished ones.
fn fade_panel_flash(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    time: Res<Time>,
    mut flashing: Query<(Entity, &mut PanelFlash, &MeshMaterial3d<StandardMaterial>)>,
) {
    for (panel, mut flash, material) in &mut flashing {
        flash.elapsed = flash.elapsed.saturating_add(time.delta());
        let Some(mut material) = materials.get_mut(&material.0) else {
            continue;
        };
        if flash.elapsed >= flash.duration {
            material.emissive = LinearRgba::BLACK;
            commands.entity(panel).remove::<PanelFlash>();
            continue;
        }
        let remaining = 1.0 - flash.elapsed.div_duration_f32(flash.duration);
        material.emissive = LinearRgba::from(Color::from(FLASH_COLOR)) * remaining;
    }
}

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_TITLE)
        .with_fit_width()
        .lines(DESCRIPTION_LINES)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) -> Result {
    let panel_assets = PanelAssets {
        knuckle_material: materials.add(metal_material(SILVER)),
        knuckle_mesh:     meshes.add(Cylinder::new(HINGE_RADIUS, hinge_knuckle_height())),
        panel_mesh:       meshes.add(Cuboid::new(PANEL_WIDTH, PANEL_HEIGHT, PANEL_THICKNESS)),
    };
    let panel_materials = PANEL_COLORS.map(|color| materials.add(panel_material(color)));

    let mut parent = spawn_fixed_mount(&mut commands, &mut meshes, &mut materials);
    let mut panels = Vec::with_capacity(PANEL_FOLD_ANGLES.len());
    for (stage, ((folded_angle, attachment_offset), material)) in PANEL_FOLD_ANGLES
        .into_iter()
        .zip(PANEL_ATTACHMENT_OFFSETS)
        .zip(panel_materials)
        .enumerate()
    {
        parent = spawn_hinged_panel(
            &mut commands,
            &panel_assets,
            material,
            parent,
            PanelJoint::new(folded_angle, attachment_offset),
            stage + FIRST_PANEL_NUMBER,
        )?;
        panels.push(parent);
    }
    commands.spawn(
        FoldSequenceBuilder::new(FoldTiming::new(FOLD_STAGE, EaseFunction::SmootherStep))
            .stages(panels)
            .build(),
    );
    Ok(())
}

// Every `AnchoredTo` chain ends at an entity whose `Transform` is authored
// instead of resolved. The gold mount is that reference: it exposes anchor
// geometry, but carries neither `AnchoredTo` nor `Hinge`, and no fold stage
// tracks it.
fn spawn_fixed_mount(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Entity {
    let material = materials.add(metal_material(GOLD));
    let mount = commands
        .spawn((
            CameraHomeTarget,
            Mesh3d(meshes.add(Cuboid::new(MOUNT_WIDTH, MOUNT_HEIGHT, MOUNT_DEPTH))),
            MeshMaterial3d(material.clone()),
            fixtures::quad_geometry(MOUNT_WIDTH, MOUNT_HEIGHT),
            Transform::from_translation(MOUNT_POSITION),
            GlobalTransform::from_translation(MOUNT_POSITION),
        ))
        .id();
    commands.entity(mount).with_children(|root| {
        root.spawn((
            Mesh3d(meshes.add(Cuboid::new(BASE_WIDTH, BASE_HEIGHT, BASE_DEPTH))),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, BASE_HEIGHT / 2.0 - MOUNT_POSITION.y, 0.0),
        ));
        root.spawn(fixed_root_label());
    });
    mount
}

fn spawn_hinged_panel(
    commands: &mut Commands,
    panel_assets: &PanelAssets,
    material: Handle<StandardMaterial>,
    parent: Entity,
    joint: PanelJoint,
    number: usize,
) -> Result<Entity> {
    let geometry = fixtures::quad_geometry(PANEL_WIDTH, PANEL_HEIGHT);
    let knuckle_line = joint.knuckle_line(&geometry);
    let entity = commands
        .spawn((
            CameraHomeTarget,
            Mesh3d(panel_assets.panel_mesh.clone()),
            MeshMaterial3d(material),
            geometry,
            Transform::default(),
            GlobalTransform::default(),
            AnchoredTo::new(parent, joint.source_anchor, joint.target_anchor)
                .with_offset(Displacement::from(joint.attachment_offset)),
            AnchorPose::default(),
            joint.hinge()?,
        ))
        .id();
    spawn_panel_details(commands, panel_assets, entity, number, knuckle_line);
    Ok(entity)
}

fn spawn_panel_details(
    commands: &mut Commands,
    panel_assets: &PanelAssets,
    panel: Entity,
    number: usize,
    knuckle_line: Option<KnuckleLine>,
) {
    commands.entity(panel).with_children(|visual| {
        if let Some(knuckle_line) = knuckle_line {
            let rotation = Quat::from_rotation_arc(Vec3::Y, *knuckle_line.direction);
            let stride = hinge_knuckle_height() + HINGE_KNUCKLE_GAP;
            let center_index = (HINGE_KNUCKLE_COUNT - 1).to_f32() / 2.0;
            for index in 0..HINGE_KNUCKLE_COUNT {
                let offset = (index.to_f32() - center_index) * stride;
                visual.spawn((
                    Mesh3d(panel_assets.knuckle_mesh.clone()),
                    MeshMaterial3d(panel_assets.knuckle_material.clone()),
                    Transform::from_translation(
                        knuckle_line.center + *knuckle_line.direction * offset,
                    )
                    .with_rotation(rotation),
                ));
            }
        }
        visual.spawn(panel_label(number, Face::Front));
        visual.spawn(panel_label(number, Face::Back));
    });
}

fn hinge_knuckle_height() -> f32 {
    let gaps = (HINGE_KNUCKLE_COUNT - 1).to_f32();
    (HINGE_HEIGHT - gaps * HINGE_KNUCKLE_GAP) / HINGE_KNUCKLE_COUNT.to_f32()
}

fn panel_label(number: usize, face: Face) -> impl Bundle {
    let (offset, facing) = match face {
        Face::Front => (LABEL_Z_OFFSET, Quat::IDENTITY),
        Face::Back => (
            -LABEL_Z_OFFSET,
            Quat::from_rotation_y(core::f32::consts::PI),
        ),
    };
    DiegeticText::world(number.to_string())
        .size(LABEL_SIZE)
        .color(LABEL_COLOR)
        .sidedness(Sidedness::FrontOnly)
        .transform(Transform::from_xyz(0.0, 0.0, offset).with_rotation(facing))
        .build()
}

fn fixed_root_label() -> impl Bundle {
    DiegeticText::world(FIXED_ROOT_LABEL)
        .size(FIXED_ROOT_LABEL_SIZE)
        .color(Color::from(GOLD))
        .sidedness(Sidedness::FrontOnly)
        .transform(Transform::from_translation(FIXED_ROOT_LABEL_OFFSET))
        .build()
}

fn panel_material(color: Srgba) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::from(color),
        perceptual_roughness: PANEL_ROUGHNESS,
        ..default()
    }
}

fn metal_material(color: Srgba) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::from(color),
        metallic: 1.0,
        perceptual_roughness: METAL_ROUGHNESS,
        ..default()
    }
}
