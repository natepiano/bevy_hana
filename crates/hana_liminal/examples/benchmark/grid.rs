use bevy::color::palettes::css::YELLOW;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Handle;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy_kana::ToF32;
use bevy_kana::ToU32;
use hana_liminal::Outline;
use hana_liminal::OutlineMethod;
use hana_liminal::OverlapMode;
use rand::RngExt;
use rand::rng;

use crate::benchmark_state::OutlinePresence;
use crate::constants::DEFAULT_OUTLINE_INTENSITY;
use crate::constants::DEPTH_SPACING_MULTIPLIER;
use crate::constants::GRID_3D_COLUMNS;
use crate::constants::GRID_3D_ROWS;
use crate::constants::GRID_CENTER_DIVISOR;
use crate::constants::GRID_CENTER_OFFSET;
use crate::constants::GRID_TO_3D_THRESHOLD;
use crate::viewport::ViewportInfo;

#[derive(Component)]
pub(super) struct BenchmarkEntity;

pub(super) struct GridSpawnSpec<'a> {
    pub(super) count:            u32,
    pub(super) width:            f32,
    pub(super) cube_fill:        f32,
    pub(super) viewport:         &'a ViewportInfo,
    pub(super) outline_presence: OutlinePresence,
    pub(super) outline_method:   OutlineMethod,
}

pub(super) fn spawn_grid(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    grid_spawn_spec: GridSpawnSpec<'_>,
) {
    let mesh_handle = meshes.add(Cuboid::default());
    let material_handle = materials.add(Color::from(YELLOW));

    if grid_spawn_spec.count > GRID_TO_3D_THRESHOLD {
        spawn_3d_grid(commands, &mesh_handle, &material_handle, grid_spawn_spec);
        return;
    }

    let GridSpawnSpec {
        count,
        width,
        cube_fill,
        viewport,
        outline_presence,
        outline_method,
    } = grid_spawn_spec;
    let cols = count.to_f32().sqrt().ceil().to_u32();
    let rows = count.div_ceil(cols);
    let horizontal_spacing = viewport.width / cols.to_f32();
    let vertical_spacing = viewport.height / rows.to_f32();
    let cube_scale = vertical_spacing * cube_fill;

    let mut spawned = 0u32;
    for row in 0..rows {
        for col in 0..cols {
            if spawned >= count {
                break;
            }
            let col_offset =
                col.to_f32() - (cols.to_f32() - GRID_CENTER_OFFSET) / GRID_CENTER_DIVISOR;
            let row_offset =
                row.to_f32() - (rows.to_f32() - GRID_CENTER_OFFSET) / GRID_CENTER_DIVISOR;
            let position = viewport.center
                + col_offset * horizontal_spacing * viewport.right
                + row_offset * vertical_spacing * viewport.up;
            let mut entity = commands.spawn((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(material_handle.clone()),
                Transform::from_translation(position).with_scale(Vec3::splat(cube_scale)),
                BenchmarkEntity,
            ));
            if outline_presence == OutlinePresence::Enabled {
                entity.insert(build_outline(width, outline_method));
            }
            spawned += 1;
        }
    }
}

fn random_outline_color() -> Color {
    let mut rng = rng();
    Color::srgb(rng.random(), rng.random(), rng.random())
}

fn build_outline(width: f32, outline_method: OutlineMethod) -> Outline {
    match outline_method {
        OutlineMethod::JumpFlood => Outline::jump_flood(width)
            .with_intensity(DEFAULT_OUTLINE_INTENSITY)
            .with_color(random_outline_color())
            .build(),
        OutlineMethod::WorldHull => Outline::world_hull(width)
            .with_intensity(DEFAULT_OUTLINE_INTENSITY)
            .with_color(random_outline_color())
            .with_overlap(OverlapMode::PerMesh)
            .build(),
        OutlineMethod::ScreenHull => Outline::screen_hull(width)
            .with_intensity(DEFAULT_OUTLINE_INTENSITY)
            .with_color(random_outline_color())
            .with_overlap(OverlapMode::PerMesh)
            .build(),
    }
}

fn spawn_3d_grid(
    commands: &mut Commands,
    mesh_handle: &Handle<Mesh>,
    material_handle: &Handle<StandardMaterial>,
    grid_spawn_spec: GridSpawnSpec<'_>,
) {
    let GridSpawnSpec {
        count,
        width,
        cube_fill,
        viewport,
        outline_presence,
        outline_method,
    } = grid_spawn_spec;
    let cols = GRID_3D_COLUMNS;
    let rows = GRID_3D_ROWS;
    let face_size = cols * rows;
    let layers = count.div_ceil(face_size);
    let horizontal_spacing = viewport.width / cols.to_f32();
    let vertical_spacing = viewport.height / rows.to_f32();
    let cube_scale = vertical_spacing * cube_fill;

    let mut spawned = 0u32;
    for depth in 0..layers {
        for row in 0..rows {
            for col in 0..cols {
                if spawned >= count {
                    break;
                }
                let col_offset =
                    col.to_f32() - (cols.to_f32() - GRID_CENTER_OFFSET) / GRID_CENTER_DIVISOR;
                let row_offset =
                    row.to_f32() - (rows.to_f32() - GRID_CENTER_OFFSET) / GRID_CENTER_DIVISOR;
                let depth_offset = depth.to_f32();
                let position = viewport.center
                    + col_offset * horizontal_spacing * viewport.right
                    + row_offset * vertical_spacing * viewport.up
                    + depth_offset * vertical_spacing * DEPTH_SPACING_MULTIPLIER * viewport.forward;
                let mut entity = commands.spawn((
                    Mesh3d(mesh_handle.clone()),
                    MeshMaterial3d(material_handle.clone()),
                    Transform::from_translation(position).with_scale(Vec3::splat(cube_scale)),
                    BenchmarkEntity,
                ));
                if outline_presence == OutlinePresence::Enabled {
                    entity.insert(build_outline(width, outline_method));
                }
                spawned += 1;
            }
        }
    }
}
