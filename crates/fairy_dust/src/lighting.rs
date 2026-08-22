//! Capability: simple studio lighting for example scenes.

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::light::CascadeShadowConfig;
use bevy::light::CascadeShadowConfigBuilder;
use bevy::light::DirectionalLightShadowMap;
use bevy::prelude::App;
use bevy::prelude::ClearColor;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DirectionalLight;
use bevy::prelude::GlobalTransform;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mesh3d;
use bevy::prelude::PointLight;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::SystemSet;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::default;

use crate::constants::AABB_CORNER_SIGNS;
use crate::constants::CASCADE_CAMERA_HEADROOM;
use crate::constants::CASCADE_COUNT;
use crate::constants::CASCADE_FIRST_BOUND_HEADROOM;
use crate::constants::CASCADE_FIRST_BOUND_RATIO;
use crate::constants::CASCADE_FIRST_FAR_BOUND;
use crate::constants::CASCADE_FIT_RADIUS_MULTIPLE;
use crate::constants::CASCADE_MAX_DISTANCE;
use crate::constants::CASCADE_MIN_DISTANCE;
use crate::constants::CASCADE_REFIT_EPSILON;
use crate::constants::CLEAR_COLOR;
use crate::constants::FILL_LIGHT_ILLUMINANCE;
use crate::constants::FILL_LIGHT_POS;
use crate::constants::KEY_LIGHT_ILLUMINANCE;
use crate::constants::KEY_LIGHT_POS;
use crate::constants::KEY_SHADOW_DEPTH_BIAS;
use crate::constants::KEY_SHADOW_NORMAL_BIAS;
use crate::constants::POINT_LIGHT_COLOR;
use crate::constants::POINT_LIGHT_INTENSITY;
use crate::constants::POINT_LIGHT_POS;
use crate::constants::POINT_LIGHT_RANGE;
use crate::constants::SHADOW_MAP_SIZE;
use crate::constants::TARGET;
use crate::orbit_cam::FairyDustOrbitCam;

#[derive(Component)]
struct FairyDustStudioLight;

/// Marks the key directional light whose shadow cascade [`fit_cascade_to_scene`]
/// keeps fitted to the scene and the active camera. The marker is permanent: the
/// fit re-runs so the cascade re-adjusts when the projection toggles between
/// perspective and orthographic.
#[derive(Component)]
struct FairyDustAutoCascade;

/// Startup set that spawns Fairy Dust's studio lighting rig.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct FairyDustStudioLightingSet;

/// Configuration consumed by the studio-lighting startup system. Defaults
/// match the original hard-coded rig; builder methods on
/// [`crate::builder::StudioLightingBuilder`] override individual fields.
#[derive(Resource, Clone, Copy)]
pub(crate) struct StudioLightingConfig {
    pub(crate) key_light_pos:         Vec3,
    pub(crate) aim_at:                Vec3,
    pub(crate) key_light_illuminance: f32,
}

impl Default for StudioLightingConfig {
    fn default() -> Self {
        Self {
            key_light_pos:         KEY_LIGHT_POS,
            aim_at:                TARGET,
            key_light_illuminance: KEY_LIGHT_ILLUMINANCE,
        }
    }
}

pub(crate) fn install(app: &mut App, config: StudioLightingConfig) {
    app.insert_resource(ClearColor(CLEAR_COLOR))
        .insert_resource(DirectionalLightShadowMap {
            size: SHADOW_MAP_SIZE,
        })
        .insert_resource(config)
        .add_systems(
            Startup,
            spawn_studio_lights.in_set(FairyDustStudioLightingSet),
        )
        .add_systems(Update, fit_cascade_to_scene);
}

fn spawn_studio_lights(mut commands: Commands, config: Res<StudioLightingConfig>) {
    commands.spawn((
        FairyDustStudioLight,
        FairyDustAutoCascade,
        DirectionalLight {
            illuminance: config.key_light_illuminance,
            shadow_maps_enabled: true,
            shadow_depth_bias: KEY_SHADOW_DEPTH_BIAS,
            shadow_normal_bias: KEY_SHADOW_NORMAL_BIAS,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: CASCADE_COUNT,
            minimum_distance: CASCADE_MIN_DISTANCE,
            maximum_distance: CASCADE_MAX_DISTANCE,
            first_cascade_far_bound: CASCADE_FIRST_FAR_BOUND,
            ..default()
        }
        .build(),
        Transform::from_translation(config.key_light_pos).looking_at(config.aim_at, Vec3::Y),
    ));

    commands.spawn((
        FairyDustStudioLight,
        DirectionalLight {
            illuminance: FILL_LIGHT_ILLUMINANCE,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_translation(FILL_LIGHT_POS).looking_at(config.aim_at, Vec3::Y),
    ));

    commands.spawn((
        FairyDustStudioLight,
        PointLight {
            color: POINT_LIGHT_COLOR,
            intensity: POINT_LIGHT_INTENSITY,
            range: POINT_LIGHT_RANGE,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_translation(POINT_LIGHT_POS),
    ));
}

/// Fits the key light's shadow cascade to the scene and the active camera,
/// re-running each frame so the cascade re-adjusts when the projection toggles.
/// The fit is the larger of two terms:
///
/// - **Geometry** — the bounding-sphere radius of only the meshes the key light actually shadows
///   (those sharing its [`RenderLayers`], so screen-space UI panels on other layers are excluded)
///   times [`CASCADE_FIT_RADIUS_MULTIPLE`]. This covers a perspective camera, which frames the
///   scene proportionally.
/// - **Camera** — the [`FairyDustOrbitCam`], when orthographic, parks at a fixed `(near + far) / 2`
///   distance (see `hana_lagrange`'s `update_orbit_transform`) that a small scene's geometry term
///   can't reach, so the far cascade must extend to it. Perspective projections leave this term at
///   zero.
///
/// Taking the larger of the two only ever holds or raises the cascade, so a
/// scene already covered by its geometry term (e.g. a large ground plane) is
/// left untouched — shadows can never shrink away. Keeping `maximum_distance` to
/// what is needed packs the shadow map over less area, so shadows stay sharp.
/// [`Aabb`]s are computed in `PostUpdate`, so this can't run at startup.
fn fit_cascade_to_scene(
    mut light: Query<(&mut CascadeShadowConfig, Option<&RenderLayers>), With<FairyDustAutoCascade>>,
    camera: Query<&Projection, With<FairyDustOrbitCam>>,
    meshes: Query<(&Aabb, &GlobalTransform, Option<&RenderLayers>), With<Mesh3d>>,
) {
    let Ok((mut cascade, light_layers)) = light.single_mut() else {
        return;
    };
    let light_layers = light_layers.cloned().unwrap_or_default();
    let Some(radius) = scene_bounding_radius(&meshes, &light_layers) else {
        return;
    };

    let geometry_distance = radius * CASCADE_FIT_RADIUS_MULTIPLE;
    let camera_distance = match camera.single() {
        Ok(Projection::Orthographic(ortho)) => {
            (f32::midpoint(ortho.near, ortho.far) + radius) * CASCADE_CAMERA_HEADROOM
        },
        _ => 0.0,
    };
    let maximum_distance =
        adjusted_cascade_maximum_distance(geometry_distance.max(camera_distance));

    // Re-running every frame keeps the cascade re-adjusting across projection
    // toggles; rewrite the config only when the fit actually moves so a steady
    // camera costs nothing.
    if cascade
        .bounds
        .last()
        .is_some_and(|current| (current - maximum_distance).abs() < CASCADE_REFIT_EPSILON)
    {
        return;
    }

    *cascade = CascadeShadowConfigBuilder {
        num_cascades: CASCADE_COUNT,
        minimum_distance: CASCADE_MIN_DISTANCE,
        maximum_distance,
        first_cascade_far_bound: first_cascade_far_bound(maximum_distance),
        ..default()
    }
    .build();
}

const fn adjusted_cascade_maximum_distance(maximum_distance: f32) -> f32 {
    let minimum_distance =
        (CASCADE_MIN_DISTANCE + CASCADE_FIRST_BOUND_HEADROOM) / CASCADE_FIRST_BOUND_RATIO;
    if maximum_distance < minimum_distance {
        minimum_distance
    } else {
        maximum_distance
    }
}

const fn first_cascade_far_bound(maximum_distance: f32) -> f32 {
    maximum_distance * CASCADE_FIRST_BOUND_RATIO
}

/// World-space bounding-sphere radius of the meshes whose [`RenderLayers`]
/// intersect `light_layers`, or `None` when none has an [`Aabb`] yet. Each
/// mesh's local [`Aabb`] is walked into world space through its
/// [`GlobalTransform`]; the radius is the half-diagonal of the union box.
fn scene_bounding_radius(
    meshes: &Query<(&Aabb, &GlobalTransform, Option<&RenderLayers>), With<Mesh3d>>,
    light_layers: &RenderLayers,
) -> Option<f32> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    for (aabb, global_transform, mesh_layers) in meshes {
        let mesh_layers = mesh_layers.cloned().unwrap_or_default();
        if !light_layers.intersects(&mesh_layers) {
            continue;
        }
        let local_center = Vec3::from(aabb.center);
        let half = Vec3::from(aabb.half_extents);
        for sign in AABB_CORNER_SIGNS {
            let world = global_transform.transform_point(local_center + sign * half);
            min = min.min(world);
            max = max.max(world);
            found = true;
        }
    }
    if !found {
        return None;
    }
    Some(((max - min) * 0.5).length())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_scene_fit_keeps_first_cascade_bound_above_minimum() {
        let maximum_distance = adjusted_cascade_maximum_distance(CASCADE_MIN_DISTANCE);

        assert!(maximum_distance > CASCADE_MIN_DISTANCE);
        assert!(first_cascade_far_bound(maximum_distance) > CASCADE_MIN_DISTANCE);
    }

    #[test]
    fn large_scene_fit_keeps_measured_distance() {
        let maximum_distance = adjusted_cascade_maximum_distance(CASCADE_MAX_DISTANCE);
        let difference = (maximum_distance - CASCADE_MAX_DISTANCE).abs();

        assert!(difference <= f32::EPSILON);
    }
}
