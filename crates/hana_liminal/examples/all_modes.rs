//! Every `OutlineMethod` — `WorldHull`, `ScreenHull`, and `JumpFlood` — applied
//! to the same three meshes one method at a time, so switching between them
//! changes the algorithm and nothing else in the scene. The `OverlapMode` on
//! those same `Outline` components switches the same way.
//!
//! A torus, a cube carrying two child spheres, and a glTF spaceship scene stand
//! left to right. The cube is what makes `OverlapMode` visible: `Merged` draws
//! no outline where two outlined surfaces overlap, whichever entities they
//! belong to; `Grouped` merges a parent and its children into one outline that
//! stays distinct from other groups; `PerMesh` gives every `Mesh3d` its own
//! boundary, children included.
//!
//! Both selections live in one `OutlineSelection` resource. The shortcuts write
//! it, `apply_outline_selection` reads it onto the three outlines,
//! `refresh_ground_readout` names it on a placard lying in the ground plane, and
//! the title bar highlights the segment naming each live value.
//!
//! Controls:
//!   W / S / J - Outline with `WorldHull` / `ScreenHull` / `JumpFlood`
//!   M / G / P - Overlap as `Merged` / `Grouped` / `PerMesh`
//!   Click a mesh - `ZoomToFit` that entity
//!   H - Return to the camera home pose

use std::f32::consts::FRAC_PI_2;
use std::time::Duration;

use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::AlphaMode;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Click;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Entity;
use bevy::prelude::EulerRot;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::Pointer;
use bevy::prelude::PointerButton;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Rectangle;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Sphere;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Torus;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::WorldAssetRoot;
use bevy::prelude::default;
use bevy::prelude::info;
use bevy::prelude::resource_changed;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use hana_diegetic::DiegeticText;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::ZoomToFit;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;
use hana_liminal::OutlineMethod;
use hana_liminal::OverlapMode;

// assets
const SCENE_ASSET_PATH: &str = "spaceship.glb#Scene0";

// camera home
/// Elevation of the home pose. Reproduces the hand-placed eye the example used
/// before the conversion — `(0, 12, 18)` aimed at the origin, so `atan2(12, 18)`.
/// Looking this far down puts the ground-flush readout at a readable angle; a
/// shallower angle brings the horizon into frame and flattens it.
const HOME_PITCH: f32 = 0.588;
/// Wider than the Fairy Dust default so the spaceship clears the camera control
/// panel down the right edge.
const HOME_MARGIN: f32 = 0.4;

// entity names
const CUBE_NAME: &str = "Cube";
const READOUT_PLACARD_NAME: &str = "Ground Readout";
const READOUT_TEXT_NAME: &str = "Ground Readout Text";
const SPACESHIP_NAME: &str = "Spaceship";
const SPHERE_NEGATIVE_X_NAME: &str = "Sphere -X";
const SPHERE_POSITIVE_X_NAME: &str = "Sphere +X";
const TORUS_NAME: &str = "Torus";

// ground
/// The three objects and the readout reach about four and a half units out from
/// the origin, so the plane is sized to sit under all of it with room to spare
/// rather than taking the Fairy Dust default.
const GROUND_SIZE: f32 = 12.0;

// ground readout
/// Lifts the readout quad clear of the ground plane so the two coplanar surfaces
/// do not z-fight.
const READOUT_GROUND_CLEARANCE: f32 = 0.01;
/// Places the readout between the objects and the camera, on empty ground.
const READOUT_FORWARD_Z: f32 = 2.2;
const OUTLINE_READOUT_PREFIX: &str = "Outline: ";
const OVERLAP_READOUT_PREFIX: &str = "Overlap Mode: ";
/// A translucent dark fill is what separates the glyphs from the ground plane
/// under them.
const READOUT_PANEL_COLOR: Color = Color::srgba(0.04, 0.05, 0.07, 0.72);
/// Two lines of [`READOUT_TEXT_SIZE`] glyphs plus room above and below them.
const READOUT_PANEL_HEIGHT: f32 = 1.0;
/// Sized for `Overlap Mode: ScreenHull`, the longest line either row can hold,
/// at [`READOUT_TEXT_SIZE`].
const READOUT_PANEL_WIDTH: f32 = 4.4;
/// Each line sits half a line off the placard's center. The quad's local `+Y`
/// points away from the camera once it is laid flat, so the larger offset is the
/// row that reads first.
const READOUT_TOP_LINE_Y: f32 = 0.19;
const READOUT_BOTTOM_LINE_Y: f32 = -0.19;
const READOUT_TEXT_COLOR: Color = Color::srgb(0.88, 0.95, 1.0);
/// Nudge off the quad's face so the glyphs do not z-fight with their backing.
const READOUT_TEXT_DEPTH: f32 = 0.005;
/// Cap height in world meters. The home pose looks down at about 34 degrees, so
/// a line lying in the ground plane is seen at roughly half its height.
const READOUT_TEXT_SIZE: f32 = 0.26;

// logging
const MESH_CLICK_LOG_PREFIX: &str = "Mesh clicked: ";

// meshes
const CUBE_BASE_COLOR: Color = Color::srgb(0.8, 0.7, 0.6);
const SPHERE_BASE_COLOR: Color = Color::srgb(0.65, 0.55, 0.75);
/// Far enough out that both spheres break the cube's silhouette at
/// [`CUBE_ROTATION`], where a shorter offset leaves the one on `-X` buried
/// inside it and `OverlapMode` with only one place to show itself.
const SPHERE_CHILD_OFFSET_X: f32 = 0.65;
const SPHERE_RADIUS: f32 = 0.35;
const SPHERE_UV_LATITUDES: u32 = 16;
const SPHERE_UV_LONGITUDES: u32 = 32;
const TORUS_BASE_COLOR: Color = Color::srgb(0.2, 0.7, 0.3);
const TORUS_INNER_RADIUS: f32 = 0.25;
const TORUS_MAJOR_RESOLUTION: usize = 64;
const TORUS_MINOR_RESOLUTION: usize = 64;
const TORUS_OUTER_RADIUS: f32 = 0.75;

// objects
const CUBE_X: f32 = 0.0;
const CUBE_Y: f32 = 1.0;
/// Yaw, pitch, and roll in `EulerRot::YXZ` order. Yaw alone keeps the two child
/// spheres on a horizontal line, where the surface they share with the cube is
/// what `Merged` and `Grouped` differ over.
const CUBE_ROTATION: [f32; 3] = [0.4, 0.0, 0.0];
/// Half the gap between neighbours, and the x of the outer two objects. Wide
/// enough that no two objects overlap, so `Merged` resolves only the cube
/// against its own spheres.
const OBJECT_SPACING: f32 = 3.2;
const SPACESHIP_SCALE: f32 = 0.3;
const SPACESHIP_X: f32 = OBJECT_SPACING;
const SPACESHIP_Y: f32 = 1.5;
/// Turns the hull to a three-quarter view, which gives the outline both a long
/// smooth edge and the wing corners to trace.
const SPACESHIP_ROTATION: [f32; 3] = [-0.7, -0.9, 0.15];
const TORUS_X: f32 = -OBJECT_SPACING;
const TORUS_Y: f32 = 1.0;
/// Tilts the ring off axis so the hole through it stays open to the camera —
/// that inner edge is the part `JumpFlood` traces and the two hull methods
/// cannot.
const TORUS_ROTATION: [f32; 3] = [0.7, 0.4, 0.0];

// outline
const OUTLINE_COLOR: Color = Color::srgb(0.0, 0.8, 1.0);
const OUTLINE_INTENSITY: f32 = 1.5;
/// Pixels, for the two methods that measure width on screen.
const OUTLINE_WIDTH: f32 = 4.0;
/// World units, for the one method that measures width in the scene.
const WORLD_HULL_OUTLINE_WIDTH: f32 = 0.03;

// title bar
const CLICK_CONTROL: &str = "Click Zoom";
const GROUPED_LABEL: &str = "G Grouped";
const GROUPED_SEGMENT: &str = "overlap-grouped";
const JUMP_FLOOD_LABEL: &str = "J JumpFlood";
const JUMP_FLOOD_SEGMENT: &str = "method-jump-flood";
const MERGED_LABEL: &str = "M Merged";
const MERGED_SEGMENT: &str = "overlap-merged";
const METHOD_HINT: &str = "Method";
const OVERLAP_HINT: &str = "Overlap";
const PER_MESH_LABEL: &str = "P PerMesh";
const PER_MESH_SEGMENT: &str = "overlap-per-mesh";
const SCREEN_HULL_LABEL: &str = "S ScreenHull";
const SCREEN_HULL_SEGMENT: &str = "method-screen-hull";
const TITLE: &str = "All Modes";
const WORLD_HULL_LABEL: &str = "W WorldHull";
const WORLD_HULL_SEGMENT: &str = "method-world-hull";

// zoom
const ZOOM_DURATION_MS: u64 = 1000;
const ZOOM_MARGIN_MESH: f32 = 0.15;

/// The method and overlap mode every outlined object currently carries. Written
/// by the six shortcut systems, read by [`apply_outline_selection`], and
/// mirrored into the title bar through `wire_chip_to_state`.
#[derive(Resource, Default)]
struct OutlineSelection {
    outline_method: OutlineMethod,
    overlap_mode:   OverlapMode,
}

impl OutlineSelection {
    /// Builds the `Outline` this selection describes. `WorldHull` measures width
    /// in world units while `JumpFlood` and `ScreenHull` measure it in pixels,
    /// so the width constant changes with the method.
    const fn outline(&self) -> Outline {
        match self.outline_method {
            OutlineMethod::WorldHull => Outline::world_hull(WORLD_HULL_OUTLINE_WIDTH)
                .with_color(OUTLINE_COLOR)
                .with_intensity(OUTLINE_INTENSITY)
                .with_overlap(self.overlap_mode)
                .build(),
            OutlineMethod::ScreenHull => Outline::screen_hull(OUTLINE_WIDTH)
                .with_color(OUTLINE_COLOR)
                .with_intensity(OUTLINE_INTENSITY)
                .with_overlap(self.overlap_mode)
                .build(),
            OutlineMethod::JumpFlood => Outline::jump_flood(OUTLINE_WIDTH)
                .with_color(OUTLINE_COLOR)
                .with_intensity(OUTLINE_INTENSITY)
                .with_overlap(self.overlap_mode)
                .build(),
        }
    }
}

/// Marks the three entities whose `Outline` the shortcuts write. Descendants
/// that `hana_liminal` propagated an outline to are left out: its
/// `sync_propagated_outlines` copies each source change down to them.
#[derive(Component)]
struct OutlineTarget;

/// The ground-flush quad the readout lines are parented to.
#[derive(Component)]
struct ReadoutPlacard;

/// The readout's current text, despawned and respawned on every selection
/// change.
#[derive(Component)]
struct ReadoutText;

struct MeshAndMaterial {
    mesh:     Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

fn main() {
    // The spaceship scene lives in this crate's own `assets/`, so the root is
    // pinned here rather than left to whichever directory cargo is invoked from.
    let mut example = fairy_dust::sprinkle_example()
        .with_asset_root(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"));

    // `hana_liminal` is the demonstrated crate, so its plugin is registered directly
    // rather than through a Fairy Dust capability. `MeshPickingPlugin` backs the
    // click-to-zoom observers below.
    example
        .app_mut()
        .add_plugins((MeshPickingPlugin, LiminalPlugin));

    example
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .size(GROUND_SIZE)
        // `OutlineCamera` marks this camera as the one the outline passes render for.
        .with_orbit_cam_preset_bundle(|_| {}, OrbitCamPreset::blender_like(), (OutlineCamera,))
        .with_stable_transparency()
        // Lights the scene with sky rather than a flat clear color. The home pose
        // looks down far enough to put the horizon off the top edge, so the sky
        // itself only comes into view once the camera is orbited up.
        .with_atmosphere()
        // Reconstructs sub-pixel edges the outline passes cannot produce themselves.
        .with_experimental_smaa()
        .with_camera_home()
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(title_bar())
        // One chip per value of each enum: the segment naming the live value
        // highlights, the other two grey back.
        .wire_chip_to_state::<OutlineSelection, _>(WORLD_HULL_SEGMENT, |selection| {
            method_activation(selection, OutlineMethod::WorldHull)
        })
        .wire_chip_to_state::<OutlineSelection, _>(SCREEN_HULL_SEGMENT, |selection| {
            method_activation(selection, OutlineMethod::ScreenHull)
        })
        .wire_chip_to_state::<OutlineSelection, _>(JUMP_FLOOD_SEGMENT, |selection| {
            method_activation(selection, OutlineMethod::JumpFlood)
        })
        .wire_chip_to_state::<OutlineSelection, _>(MERGED_SEGMENT, |selection| {
            overlap_activation(selection, OverlapMode::Merged)
        })
        .wire_chip_to_state::<OutlineSelection, _>(GROUPED_SEGMENT, |selection| {
            overlap_activation(selection, OverlapMode::Grouped)
        })
        .wire_chip_to_state::<OutlineSelection, _>(PER_MESH_SEGMENT, |selection| {
            overlap_activation(selection, OverlapMode::PerMesh)
        })
        .with_camera_control_panel()
        .init_resource::<OutlineSelection>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (apply_outline_selection, refresh_ground_readout)
                .run_if(resource_changed::<OutlineSelection>),
        )
        .with_shortcut(KeyCode::KeyW, select_world_hull)
        .with_shortcut(KeyCode::KeyS, select_screen_hull)
        .with_shortcut(KeyCode::KeyJ, select_jump_flood)
        .with_shortcut(KeyCode::KeyM, select_merged)
        .with_shortcut(KeyCode::KeyG, select_grouped)
        .with_shortcut(KeyCode::KeyP, select_per_mesh)
        .run();
}

fn title_bar() -> TitleBar {
    TitleBar::new()
        .with_title(TITLE)
        .control(TitleBarControl::segmented(
            METHOD_HINT,
            [
                TitleBarSegment::new(WORLD_HULL_SEGMENT, WORLD_HULL_LABEL),
                TitleBarSegment::new(SCREEN_HULL_SEGMENT, SCREEN_HULL_LABEL),
                TitleBarSegment::new(JUMP_FLOOD_SEGMENT, JUMP_FLOOD_LABEL),
            ],
        ))
        .control(TitleBarControl::segmented(
            OVERLAP_HINT,
            [
                TitleBarSegment::new(MERGED_SEGMENT, MERGED_LABEL),
                TitleBarSegment::new(GROUPED_SEGMENT, GROUPED_LABEL),
                TitleBarSegment::new(PER_MESH_SEGMENT, PER_MESH_LABEL),
            ],
        ))
        .control(CLICK_CONTROL)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    selection: Res<OutlineSelection>,
) {
    let cube = MeshAndMaterial {
        mesh:     meshes.add(Cuboid::default()),
        material: materials.add(StandardMaterial {
            base_color: CUBE_BASE_COLOR,
            ..default()
        }),
    };
    let sphere = MeshAndMaterial {
        mesh:     meshes.add(
            Sphere::new(SPHERE_RADIUS)
                .mesh()
                .uv(SPHERE_UV_LONGITUDES, SPHERE_UV_LATITUDES),
        ),
        material: materials.add(StandardMaterial {
            base_color: SPHERE_BASE_COLOR,
            ..default()
        }),
    };
    let torus = MeshAndMaterial {
        mesh:     meshes.add(
            Torus::new(TORUS_INNER_RADIUS, TORUS_OUTER_RADIUS)
                .mesh()
                .minor_resolution(TORUS_MINOR_RESOLUTION)
                .major_resolution(TORUS_MAJOR_RESOLUTION),
        ),
        material: materials.add(StandardMaterial {
            base_color: TORUS_BASE_COLOR,
            ..default()
        }),
    };
    let outline = selection.outline();
    spawn_torus(&mut commands, &torus, &outline);
    spawn_cube(&mut commands, &cube, &sphere, &outline);
    spawn_spaceship(&mut commands, &asset_server, &outline);
    spawn_readout_placard(&mut commands, &mut meshes, &mut materials);
}

fn spawn_torus(commands: &mut Commands, torus: &MeshAndMaterial, outline: &Outline) {
    commands
        .spawn((
            Name::new(TORUS_NAME),
            Mesh3d(torus.mesh.clone()),
            MeshMaterial3d(torus.material.clone()),
            Transform {
                translation: Vec3::new(TORUS_X, TORUS_Y, 0.0),
                rotation: rotation(TORUS_ROTATION),
                ..default()
            },
            outline.clone(),
            OutlineTarget,
            // Every object contributes its AABB to the framed home region.
            CameraHomeTarget,
        ))
        .observe(on_mesh_clicked);
}

/// The two spheres are children of the cube, which is what gives `Grouped`
/// something to merge and `PerMesh` something to separate.
fn spawn_cube(
    commands: &mut Commands,
    cube: &MeshAndMaterial,
    sphere: &MeshAndMaterial,
    outline: &Outline,
) {
    commands
        .spawn((
            Name::new(CUBE_NAME),
            Mesh3d(cube.mesh.clone()),
            MeshMaterial3d(cube.material.clone()),
            Transform {
                translation: Vec3::new(CUBE_X, CUBE_Y, 0.0),
                rotation: rotation(CUBE_ROTATION),
                ..default()
            },
            outline.clone(),
            OutlineTarget,
            CameraHomeTarget,
        ))
        .observe(on_mesh_clicked)
        .with_children(|parent| {
            parent.spawn((
                Name::new(SPHERE_POSITIVE_X_NAME),
                Mesh3d(sphere.mesh.clone()),
                MeshMaterial3d(sphere.material.clone()),
                Transform::from_xyz(SPHERE_CHILD_OFFSET_X, 0.0, 0.0),
            ));
            parent.spawn((
                Name::new(SPHERE_NEGATIVE_X_NAME),
                Mesh3d(sphere.mesh.clone()),
                MeshMaterial3d(sphere.material.clone()),
                Transform::from_xyz(-SPHERE_CHILD_OFFSET_X, 0.0, 0.0),
            ));
        });
}

/// The glTF scene arrives as descendants of this entity, so its `Outline`
/// reaches the hull through `hana_liminal`'s propagation rather than sitting on
/// a `Mesh3d` of its own.
fn spawn_spaceship(commands: &mut Commands, asset_server: &AssetServer, outline: &Outline) {
    commands
        .spawn((
            Name::new(SPACESHIP_NAME),
            WorldAssetRoot(asset_server.load(SCENE_ASSET_PATH)),
            Transform {
                translation: Vec3::new(SPACESHIP_X, SPACESHIP_Y, 0.0),
                rotation:    rotation(SPACESHIP_ROTATION),
                scale:       Vec3::splat(SPACESHIP_SCALE),
            },
            outline.clone(),
            OutlineTarget,
            CameraHomeTarget,
        ))
        .observe(on_mesh_clicked);
}

/// Spawns the readout's backing quad, lying in the ground plane in front of the
/// three objects. `Quat::from_rotation_x(-FRAC_PI_2)` turns the quad's face from
/// `+Z` to `+Y` and its text baseline from `+Y` to `-Z`, so the lines run away
/// from a camera sitting on `+Z`.
///
/// The text itself is spawned by [`refresh_ground_readout`], which replaces it
/// on every selection change.
fn spawn_readout_placard(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    commands.spawn((
        Name::new(READOUT_PLACARD_NAME),
        ReadoutPlacard,
        Mesh3d(meshes.add(Rectangle::new(READOUT_PANEL_WIDTH, READOUT_PANEL_HEIGHT))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: READOUT_PANEL_COLOR,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        })),
        Transform {
            translation: Vec3::new(CUBE_X, READOUT_GROUND_CLEARANCE, READOUT_FORWARD_Z),
            rotation: Quat::from_rotation_x(-FRAC_PI_2),
            ..default()
        },
        // Keeps the readout inside the framed home region with the objects.
        CameraHomeTarget,
    ));
}

/// Replaces the readout text with one naming the live selection. `DiegeticText`
/// holds its string inside the panel's layout tree with no component to write
/// through, so the change is a despawn and a fresh spawn under the same placard.
///
/// Each line is its own panel rather than one string carrying a `\n`: a single
/// panel sizes itself to fit and then breaks anything longer than that fit, which
/// split `Overlap Mode: Merged` across two rows. One line per panel leaves nothing
/// for the fit to break.
fn refresh_ground_readout(
    mut commands: Commands,
    selection: Res<OutlineSelection>,
    stale: Query<Entity, With<ReadoutText>>,
    placards: Query<Entity, With<ReadoutPlacard>>,
) {
    for entity in &stale {
        commands.entity(entity).despawn();
    }
    let rows = [
        (
            READOUT_TOP_LINE_Y,
            format!(
                "{OUTLINE_READOUT_PREFIX}{name}",
                name = outline_method_name(selection.outline_method)
            ),
        ),
        (
            READOUT_BOTTOM_LINE_Y,
            format!(
                "{OVERLAP_READOUT_PREFIX}{name}",
                name = overlap_mode_name(selection.overlap_mode)
            ),
        ),
    ];
    for placard in &placards {
        commands.entity(placard).with_children(|parent| {
            for (line_y, line) in &rows {
                parent.spawn((
                    Name::new(READOUT_TEXT_NAME),
                    ReadoutText,
                    DiegeticText::world(line)
                        .size(READOUT_TEXT_SIZE)
                        .color(READOUT_TEXT_COLOR)
                        .transform(Transform::from_xyz(0.0, *line_y, READOUT_TEXT_DEPTH))
                        .build(),
                ));
            }
        });
    }
}

const fn outline_method_name(outline_method: OutlineMethod) -> &'static str {
    match outline_method {
        OutlineMethod::WorldHull => "WorldHull",
        OutlineMethod::ScreenHull => "ScreenHull",
        OutlineMethod::JumpFlood => "JumpFlood",
    }
}

const fn overlap_mode_name(overlap_mode: OverlapMode) -> &'static str {
    match overlap_mode {
        OverlapMode::Merged => "Merged",
        OverlapMode::Grouped => "Grouped",
        OverlapMode::PerMesh => "PerMesh",
    }
}

/// Replaces the `Outline` on each marked object with the one the live selection
/// describes. Runs only on a `resource_changed::<OutlineSelection>` tick.
fn apply_outline_selection(
    selection: Res<OutlineSelection>,
    mut outlines: Query<&mut Outline, With<OutlineTarget>>,
) {
    let outline = selection.outline();
    for mut target in &mut outlines {
        *target = outline.clone();
    }
}

fn select_world_hull(mut selection: ResMut<OutlineSelection>) {
    selection.outline_method = OutlineMethod::WorldHull;
}

fn select_screen_hull(mut selection: ResMut<OutlineSelection>) {
    selection.outline_method = OutlineMethod::ScreenHull;
}

fn select_jump_flood(mut selection: ResMut<OutlineSelection>) {
    selection.outline_method = OutlineMethod::JumpFlood;
}

fn select_merged(mut selection: ResMut<OutlineSelection>) {
    selection.overlap_mode = OverlapMode::Merged;
}

fn select_grouped(mut selection: ResMut<OutlineSelection>) {
    selection.overlap_mode = OverlapMode::Grouped;
}

fn select_per_mesh(mut selection: ResMut<OutlineSelection>) {
    selection.overlap_mode = OverlapMode::PerMesh;
}

fn on_mesh_clicked(click: On<Pointer<Click>>, mut commands: Commands) {
    if click.button != PointerButton::Primary {
        return;
    }
    info!("{MESH_CLICK_LOG_PREFIX}{entity:?}", entity = click.entity);
    let camera = click.hit.camera;
    commands.trigger(
        ZoomToFit::new(camera, click.entity)
            .margin(ZOOM_MARGIN_MESH)
            .duration(Duration::from_millis(ZOOM_DURATION_MS)),
    );
}

/// Highlights the title-bar segment whose [`OutlineMethod`] the selection holds.
fn method_activation(
    selection: &OutlineSelection,
    outline_method: OutlineMethod,
) -> ControlActivation {
    if selection.outline_method == outline_method {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}

/// Highlights the title-bar segment whose [`OverlapMode`] the selection holds.
fn overlap_activation(
    selection: &OutlineSelection,
    overlap_mode: OverlapMode,
) -> ControlActivation {
    if selection.overlap_mode == overlap_mode {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}

fn rotation([yaw, pitch, roll]: [f32; 3]) -> Quat {
    Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll)
}
