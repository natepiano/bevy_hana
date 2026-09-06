//! Directly authored triangle fold chain with selectable fold profiles.

use std::time::Duration;

use bevy::color::Srgba;
use bevy::color::palettes::css::CORAL;
use bevy::color::palettes::css::GOLD;
use bevy::color::palettes::css::MEDIUM_PURPLE;
use bevy::color::palettes::css::SEA_GREEN;
use bevy::color::palettes::css::SKY_BLUE;
use bevy::color::palettes::css::TURQUOISE;
use bevy::math::curve::easing::EaseFunction;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Bundle;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Result;
use bevy::prelude::SpawnRelated;
use bevy::prelude::SpawnWith;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Triangle2d;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::default;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DescriptionPanel;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use hana_diegetic::DiegeticText;
use hana_diegetic::Sidedness;
use hana_kana::ToF32;
use hana_lagrange::OrbitCamPreset;
use hana_rubric::Keybindings;
use hana_rubric::action;
use hana_rubric::bind_action_system;
use hana_rubric::event;
use hana_valence::AnchorPose;
use hana_valence::AnchorSystems;
use hana_valence::AnchoredTo;
use hana_valence::Angle;
use hana_valence::Displacement;
use hana_valence::Edge;
use hana_valence::FoldSequenceBuilder;
use hana_valence::FoldSequencePlayback;
use hana_valence::FoldTiming;
use hana_valence::Hinge;

#[path = "../fixtures.rs"]
#[allow(
    dead_code,
    reason = "shared geometry fixtures; this example uses a subset"
)]
mod fixtures;

// app
const EXAMPLE_TITLE: &str = "Triangles";

// title-bar chips
const MODE_KEY: &str = "T";
const ACCORDION_CONTROL: &str = "Accordion";
const WRAP_CONTROL: &str = "Wrap";

// animation
const ACCORDION_LEAN: f32 = core::f32::consts::PI;
const EVEN_PERIOD: usize = 2;
const MOUNTAIN_SIGN: f32 = 1.0;
const STEP_DURATION: Duration = Duration::from_millis(500);
const VALLEY_SIGN: f32 = -1.0;
// Face-to-face spacing between folded tiles. Must exceed two label offsets so the
// labels floating off facing tile surfaces keep their own clearance in the stack.
const TILE_GAP: f32 = 0.001;

// teaching panel
const DESCRIPTION_TITLE: &str = "Arrangement-derived folding";
const DESCRIPTION_LINES: [&str; 7] = [
    "Arrangement order becomes one zero-based fold stage per triangle.",
    "Space / Shift+Space steps one crease forward / backward.",
    "At a terminal, P selects the other endpoint.",
    "Idle in the interior: P follows the latest step direction.",
    "During a step: P continues that direction to the terminal.",
    "During Play: P reverses immediately.",
    "T selects Accordion or Wrap; mid-fold selections wait until fully unfolded.",
];

// labels
const FIRST_TILE_NUMBER: usize = 1;
const LABEL_COLOR: Color = Color::BLACK;
const LABEL_SIZE: f32 = 0.3;
// Nudge each face label off the triangle surface so it does not z-fight the tile.
const LABEL_Z_OFFSET: f32 = 0.0005;

// camera home
const HOME_MARGIN: f32 = 0.45;
const HOME_PITCH: f32 = 0.76;
const HOME_YAW: f32 = 0.63;

// triangle
const TILE_COLORS: [Srgba; TILE_COUNT] =
    [CORAL, GOLD, SKY_BLUE, SEA_GREEN, MEDIUM_PURPLE, TURQUOISE];
const TILE_COUNT: usize = 6;
const TILE_ROUGHNESS: f32 = 0.55;
// The strip cascades downward in -Y and the accordion fold swings the tail tile
// deeper still; lift the whole strip so its lowest vertex clears the ground
// plane across the full fold cycle (deepest measured reach ~2.9 with margin).
const ROOT_LIFT: f32 = 3.3;
const TRIANGLE_HEIGHT: f32 = 0.866_025_4;
const TRIANGLE_REST_FLIP: f32 = core::f32::consts::PI;
const TRIANGLE_SIDE: f32 = 1.0;
const TRIANGLE_TWO_THIRDS: f32 = 2.0 / 3.0;

struct TriangleAlgorithmInputPlugin;

impl Plugin for TriangleAlgorithmInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_input_context::<TriangleAlgorithmInput>()
            .add_systems(Startup, spawn_algorithm_input);
        bind_action_system!(app, ToggleAlgorithm, ToggleAlgorithmEvent, toggle_algorithm);
    }
}

#[derive(Component)]
struct TriangleAlgorithmInput;

action!(
    /// Selects the next triangle fold algorithm.
    ToggleAlgorithm
);
action!(
    /// Tracks Shift so the bare T binding is modifier-safe.
    AlgorithmShift
);
event!(
    /// Routes a fold-algorithm selection request into the example system.
    ToggleAlgorithmEvent
);

#[derive(Component)]
struct TriangleFoldMember {
    index: usize,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum FoldAlgorithm {
    #[default]
    Accordion,
    Wrap,
}

#[derive(Resource)]
struct AlgorithmSelection {
    selected_algorithm: FoldAlgorithm,
    active_algorithm:   FoldAlgorithm,
}

impl Default for AlgorithmSelection {
    fn default() -> Self {
        Self {
            selected_algorithm: FoldAlgorithm::Accordion,
            active_algorithm:   FoldAlgorithm::Accordion,
        }
    }
}

impl AlgorithmSelection {
    fn activation_for(&self, algorithm: FoldAlgorithm) -> ControlActivation {
        if self.selected_algorithm == algorithm {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    }
}

fn main() {
    let app = fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_stable_transparency()
        .with_fold_controls()
        .with_camera_home()
        .pitch(HOME_PITCH)
        .yaw(HOME_YAW)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(TitleBarControl::segmented(
                    MODE_KEY,
                    [
                        TitleBarSegment::new(ACCORDION_CONTROL, "Accordion"),
                        TitleBarSegment::new(WRAP_CONTROL, "Wrap"),
                    ],
                )),
        )
        .wire_chip_to_state::<AlgorithmSelection, _>(ACCORDION_CONTROL, |selection| {
            selection.activation_for(FoldAlgorithm::Accordion)
        })
        .wire_chip_to_state::<AlgorithmSelection, _>(WRAP_CONTROL, |selection| {
            selection.activation_for(FoldAlgorithm::Wrap)
        })
        .with_description_panel(description_panel())
        .with_camera_control_panel()
        .add_plugins(TriangleAlgorithmInputPlugin);
    app.init_resource::<AlgorithmSelection>()
        .add_systems(
            PostUpdate,
            activate_selected_algorithm
                .in_set(AnchorSystems::AnimatePose)
                .before(AnchorSystems::HingeToPose),
        )
        .add_systems(Startup, setup)
        .run();
}

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_TITLE)
        .with_fit_width()
        .lines(DESCRIPTION_LINES)
}

fn spawn_algorithm_input(mut commands: Commands) {
    commands.spawn((
        TriangleAlgorithmInput,
        Actions::<TriangleAlgorithmInput>::spawn(SpawnWith(spawn_algorithm_actions)),
    ));
}

fn spawn_algorithm_actions(spawner: &mut ActionSpawner<TriangleAlgorithmInput>) {
    let keybindings = Keybindings::new::<AlgorithmShift>(spawner, ActionSettings::default());
    keybindings.spawn_key::<ToggleAlgorithm>(spawner, KeyCode::KeyT);
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) -> Result {
    let triangle_mesh = meshes.add(triangle_mesh());
    let [
        root_material,
        gold_material,
        sky_material,
        green_material,
        purple_material,
        turquoise_material,
    ] = TILE_COLORS.map(|color| materials.add(tile_material(color)));
    let root = spawn_tile(&mut commands, Transform::from_xyz(0.0, ROOT_LIFT, 0.0));
    spawn_tile_visual(
        &mut commands,
        root,
        triangle_mesh.clone(),
        root_material,
        FIRST_TILE_NUMBER,
    );
    let mut parent = root;
    let mut tiles = Vec::new();
    for (offset, material) in [
        gold_material,
        sky_material,
        green_material,
        purple_material,
        turquoise_material,
    ]
    .into_iter()
    .enumerate()
    {
        let member_index = offset + 1;
        let number = FIRST_TILE_NUMBER + member_index;
        let tile = spawn_member_tile(
            &mut commands,
            parent,
            member_index,
            FoldAlgorithm::Accordion,
        )?;
        spawn_tile_visual(&mut commands, tile, triangle_mesh.clone(), material, number);
        tiles.push(tile);
        parent = tile;
    }
    commands.spawn(
        FoldSequenceBuilder::new(FoldTiming::new(STEP_DURATION, EaseFunction::SmoothStep))
            .stages(tiles)
            .build(),
    );
    Ok(())
}

fn toggle_algorithm(mut selection: ResMut<AlgorithmSelection>) {
    selection.selected_algorithm = match selection.selected_algorithm {
        FoldAlgorithm::Accordion => FoldAlgorithm::Wrap,
        FoldAlgorithm::Wrap => FoldAlgorithm::Accordion,
    };
}

fn activate_selected_algorithm(
    mut selection: ResMut<AlgorithmSelection>,
    sequences: Query<&FoldSequencePlayback>,
    mut members: Query<(&TriangleFoldMember, &mut Hinge)>,
) -> Result {
    if selection.selected_algorithm == selection.active_algorithm {
        return Ok(());
    }
    let Ok(playback) = sequences.single() else {
        return Ok(());
    };
    if playback.normalized_position() != 0.0 {
        return Ok(());
    }
    for (member, mut hinge) in &mut members {
        // Only the folded endpoint and the pivot offset change with the
        // algorithm, so the authored edge is read back off the current hinge.
        let edge = hinge.edge();
        *hinge = algorithm_hinge(edge, member.index, selection.selected_algorithm)?;
    }
    selection.active_algorithm = selection.selected_algorithm;
    Ok(())
}

// `TRIANGLE_REST_FLIP` holds the tile a half-turn from its parent and
// `ACCORDION_LEAN` adds a further half-turn into the fold. `FoldAlgorithm`
// changes only the sign `fold_sign` returns and the `pivot_scale` applied to the
// `Hinge` pivot, so `TriangleFoldMember::index` selects both.
fn algorithm_hinge(edge: Edge, index: usize, algorithm: FoldAlgorithm) -> Result<Hinge> {
    let sign = fold_sign(index, algorithm);
    let signed_lean = ACCORDION_LEAN * sign;
    let pivot_offset = Vec3::Z * (sign * pivot_scale(index, algorithm) * TILE_GAP / 2.0);
    Ok(Hinge::try_new(
        edge,
        Angle::from_radians(TRIANGLE_REST_FLIP)?,
        Angle::from_radians(TRIANGLE_REST_FLIP + signed_lean)?,
        Displacement::from(pivot_offset),
    )?)
}

const fn fold_sign(index: usize, algorithm: FoldAlgorithm) -> f32 {
    match algorithm {
        FoldAlgorithm::Accordion => MOUNTAIN_SIGN,
        FoldAlgorithm::Wrap if index.is_multiple_of(EVEN_PERIOD) => MOUNTAIN_SIGN,
        FoldAlgorithm::Wrap => VALLEY_SIGN,
    }
}

fn pivot_scale(index: usize, algorithm: FoldAlgorithm) -> f32 {
    match algorithm {
        FoldAlgorithm::Accordion => 1.0,
        FoldAlgorithm::Wrap => index.to_f32(),
    }
}

fn spawn_member_tile(
    commands: &mut Commands,
    parent: Entity,
    index: usize,
    algorithm: FoldAlgorithm,
) -> Result<Entity> {
    let entity = spawn_tile(commands, Transform::default());
    let edge = fixtures::triangle_edge(index);
    let Some(source_anchor) = fixtures::triangle_edge_anchor(edge) else {
        return Ok(entity);
    };
    let Some(target_anchor) = fixtures::triangle_edge_anchor(edge) else {
        return Ok(entity);
    };
    commands.entity(entity).insert((
        TriangleFoldMember { index },
        AnchoredTo::new(parent, source_anchor, target_anchor),
        AnchorPose::default(),
        algorithm_hinge(edge, index, algorithm)?,
    ));
    Ok(entity)
}

// The anchored tile entity carries only the fold geometry and pose the pipeline
// drives; `resolve_anchors` owns its `Transform`, so anything static has to sit
// on a child. The visible mesh and labels live on `spawn_tile_visual`'s child
// instead.
fn spawn_tile(commands: &mut Commands, transform: Transform) -> Entity {
    commands
        .spawn((
            fixtures::triangle_geometry(),
            transform,
            GlobalTransform::from(transform),
            Visibility::default(),
        ))
        .id()
}

// Spawn the visible triangle plus its front/back number labels. The mesh sits on
// the tile with no offset, so the strip lies flat and coplanar when unfolded.
// Even-numbered tiles rest flipped a half-turn (each member adds the PI rest
// delta), so their local +Z faces away from the viewer — place `{n}F` on whichever
// local face points at the camera at rest so all "F" read on one side.
fn spawn_tile_visual(
    commands: &mut Commands,
    tile: Entity,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    number: usize,
) {
    let front_on_plus_z = !number.is_multiple_of(EVEN_PERIOD);
    commands.entity(tile).with_children(|parent| {
        parent
            .spawn((Mesh3d(mesh), MeshMaterial3d(material), CameraHomeTarget))
            .with_children(|visual| {
                visual.spawn(face_label(format!("{number}F"), front_on_plus_z));
                visual.spawn(face_label(format!("{number}B"), !front_on_plus_z));
            });
    });
}

fn face_label(text: String, on_plus_z: bool) -> impl Bundle {
    let offset = if on_plus_z {
        LABEL_Z_OFFSET
    } else {
        -LABEL_Z_OFFSET
    };
    let facing = if on_plus_z {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_y(core::f32::consts::PI)
    };
    DiegeticText::world(text)
        .size(LABEL_SIZE)
        .color(LABEL_COLOR)
        .sidedness(Sidedness::FrontOnly)
        .transform(Transform::from_xyz(0.0, 0.0, offset).with_rotation(facing))
        .build()
}

fn triangle_mesh() -> Triangle2d {
    let half_side = TRIANGLE_SIDE / 2.0;
    Triangle2d::new(
        Vec2::new(0.0, TRIANGLE_HEIGHT * TRIANGLE_TWO_THIRDS),
        Vec2::new(half_side, -TRIANGLE_HEIGHT / 3.0),
        Vec2::new(-half_side, -TRIANGLE_HEIGHT / 3.0),
    )
}

fn tile_material(color: Srgba) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::from(color),
        cull_mode: None,
        double_sided: true,
        perceptual_roughness: TILE_ROUGHNESS,
        ..default()
    }
}
