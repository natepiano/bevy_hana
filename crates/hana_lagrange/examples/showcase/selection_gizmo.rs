use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Color;
use bevy::prelude::Component;
use bevy::prelude::GizmoConfigGroup;
use bevy::prelude::GizmoConfigStore;
use bevy::prelude::GizmoPrimitive3d;
use bevy::prelude::Gizmos;
use bevy::prelude::Isometry3d;
use bevy::prelude::Reflect;
use bevy::prelude::Torus;
use bevy::prelude::Transform;
use bevy::prelude::Without;

use super::*;

#[derive(Component)]
pub(crate) struct Selected;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct SelectionGizmo;

pub(crate) fn init_selection_gizmo(mut config_store: ResMut<GizmoConfigStore>) {
    let (config, _) = config_store.config_mut::<SelectionGizmo>();
    config.depth_bias = GIZMO_DEPTH_BIAS;
    config.line.width = GIZMO_LINE_WIDTH;
    config.render_layers = RenderLayers::layer(DEFAULT_SCENE_LAYER);
}

fn draw_shape_gizmo(
    gizmos: &mut Gizmos<SelectionGizmo>,
    transform: &Transform,
    shape: &scene::MeshShape,
    color: Color,
) {
    match shape {
        scene::MeshShape::Cuboid(size) => {
            gizmos.cube(
                Transform::from_translation(transform.translation)
                    .with_rotation(transform.rotation)
                    .with_scale(*size * GIZMO_SCALE),
                color,
            );
        },
        scene::MeshShape::Sphere(radius) => {
            gizmos.sphere(
                Isometry3d::new(transform.translation, transform.rotation),
                radius * GIZMO_SCALE,
                color,
            );
        },
        scene::MeshShape::Torus {
            minor_radius,
            major_radius,
        } => {
            gizmos.primitive_3d(
                &Torus::new(*minor_radius * GIZMO_SCALE, *major_radius * GIZMO_SCALE),
                Isometry3d::new(transform.translation, transform.rotation),
                color,
            );
        },
    }
}

pub(crate) fn draw_selection_gizmo(
    mut gizmos: Gizmos<SelectionGizmo>,
    query: Query<(&Transform, &scene::MeshShape), With<Selected>>,
) {
    let color = Color::from(DEEP_SKY_BLUE);
    for (transform, shape) in &query {
        draw_shape_gizmo(&mut gizmos, transform, shape, color);
    }
}

pub(crate) fn draw_hover_gizmo(
    mut gizmos: Gizmos<SelectionGizmo>,
    hovered: Res<pointer::HoveredEntity>,
    query: Query<(&Transform, &scene::MeshShape), Without<Selected>>,
) {
    let Some(entity) = hovered.0 else {
        return;
    };
    let Ok((transform, shape)) = query.get(entity) else {
        return;
    };
    draw_shape_gizmo(&mut gizmos, transform, shape, Color::from(ORANGE));
}
