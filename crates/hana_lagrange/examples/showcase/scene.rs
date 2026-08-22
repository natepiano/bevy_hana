use bevy::prelude::Alpha;
use bevy::prelude::AlphaMode;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Plane3d;
use bevy::prelude::Quat;
use bevy::prelude::Sphere;
use bevy::prelude::Torus;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::default;

use super::*;

#[derive(Component)]
pub(crate) enum MeshShape {
    Cuboid(Vec3),
    Sphere(f32),
    Torus {
        minor_radius: f32,
        major_radius: f32,
    },
}

pub(crate) fn spawn_scene_objects(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Entity {
    // Ground plane (clickable from above — deselects and zooms to scene bounds)
    let ground = commands
        .spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(GROUND_SIZE, GROUND_SIZE))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: GROUND_COLOR.with_alpha(GROUND_ALPHA),
                alpha_mode: AlphaMode::Blend,
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
        ))
        .observe(pointer::on_ground_clicked)
        .id();

    // Underside plane (clickable from below — deselects and animates back to scene)
    commands
        .spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(GROUND_SIZE, GROUND_SIZE))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: UNDERSIDE_PLANE_COLOR,
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            })),
            Transform::from_rotation(Quat::from_rotation_x(UNDERSIDE_PLANE_ROTATION_X)),
        ))
        .observe(pointer::on_below_clicked);

    // Cuboid
    commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::new(
                MESH_CUBOID_SIZE.x,
                MESH_CUBOID_SIZE.y,
                MESH_CUBOID_SIZE.z,
            ))),
            MeshMaterial3d(materials.add(MESH_CUBOID_COLOR)),
            Transform::from_translation(MESH_CUBOID_TRANSLATION),
            MeshShape::Cuboid(MESH_CUBOID_SIZE),
            CameraHomeTarget,
        ))
        .observe(pointer::on_mesh_clicked)
        .observe(pointer::on_mesh_dragged)
        .observe(pointer::on_mesh_hover)
        .observe(pointer::on_mesh_unhover);

    // Sphere
    commands
        .spawn((
            Mesh3d(
                meshes.add(
                    Sphere::new(MESH_SPHERE_RADIUS)
                        .mesh()
                        .uv(MESH_SPHERE_LONGITUDES, MESH_SPHERE_LATITUDES),
                ),
            ),
            MeshMaterial3d(materials.add(MESH_SPHERE_COLOR)),
            Transform::from_translation(MESH_SPHERE_TRANSLATION),
            MeshShape::Sphere(MESH_SPHERE_RADIUS),
            CameraHomeTarget,
        ))
        .observe(pointer::on_mesh_clicked)
        .observe(pointer::on_mesh_dragged)
        .observe(pointer::on_mesh_hover)
        .observe(pointer::on_mesh_unhover);

    // Torus
    commands
        .spawn((
            Mesh3d(
                meshes.add(
                    Torus::new(MESH_TORUS_MINOR_RADIUS, MESH_TORUS_MAJOR_RADIUS)
                        .mesh()
                        .minor_resolution(MESH_TORUS_MINOR_RESOLUTION)
                        .major_resolution(MESH_TORUS_MAJOR_RESOLUTION),
                ),
            ),
            MeshMaterial3d(materials.add(MESH_TORUS_COLOR)),
            Transform::from_translation(MESH_TORUS_TRANSLATION),
            MeshShape::Torus {
                minor_radius: MESH_TORUS_MINOR_RADIUS,
                major_radius: MESH_TORUS_MAJOR_RADIUS,
            },
            CameraHomeTarget,
        ))
        .observe(pointer::on_mesh_clicked)
        .observe(pointer::on_mesh_dragged)
        .observe(pointer::on_mesh_hover)
        .observe(pointer::on_mesh_unhover);

    ground
}
