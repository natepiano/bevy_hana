use std::collections::HashSet;

use bevy::asset::Assets;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
use bevy::mesh::Mesh3d;
use bevy::pbr::MeshMaterial3d;
use bevy::pbr::StandardMaterial;
use bevy::picking::mesh_picking::ray_cast::MeshRayCast;
use bevy::picking::mesh_picking::ray_cast::MeshRayCastSettings;
use bevy::picking::mesh_picking::ray_cast::RayCastVisibility;
use bevy::prelude::Alpha;
use bevy::prelude::AlphaMode;
use bevy::prelude::Camera;
use bevy::prelude::Children;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Dir3;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Has;
use bevy::prelude::Query;
use bevy::prelude::Ray3d;
use bevy::prelude::Res;
#[cfg(test)]
use bevy::prelude::ResMut;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::transform::helper::TransformHelper;
use hana_valence::ResolvedAnchorGeometry;

use super::placement_state::TooltipPlacementState;
use super::placement_state::WorldTooltipOcclusion;
use super::world_target;
use super::world_target::TooltipCameraView;
use super::world_target::WorldTooltipTarget;
use crate::cascade::Resolved;
use crate::layout::Anchor;
use crate::layout::Sidedness;
use crate::panel::DiegeticPanel;
use crate::panel::PanelOwned;
use crate::panel::PanelPlane;
use crate::render::PanelInteractionMesh;
use crate::widgets::tooltip::MaterializedTooltip;

pub(in crate::widgets::tooltip) fn refresh_occluder_globals(
    transforms: TransformHelper,
    mut meshes: Query<(Entity, &mut GlobalTransform), With<Mesh3d>>,
    #[cfg(test)] mut run_count: Option<ResMut<OccluderRefreshRunCount>>,
) {
    #[cfg(test)]
    if let Some(run_count) = run_count.as_mut() {
        run_count.0 += 1;
    }
    for (entity, mut global) in &mut meshes {
        if let Ok(current) = transforms.compute_global_transform(entity) {
            global.set_if_neq(current);
        }
    }
}

pub(in crate::widgets::tooltip) fn refresh_occluder_globals_after_resolve(
    transforms: TransformHelper,
    meshes: Query<(Entity, &mut GlobalTransform), With<Mesh3d>>,
    #[cfg(test)] run_count: Option<ResMut<OccluderRefreshRunCount>>,
) {
    refresh_occluder_globals(
        transforms,
        meshes,
        #[cfg(test)]
        run_count,
    );
}

#[cfg(test)]
#[derive(Default, bevy::prelude::Resource)]
pub(in crate::widgets::tooltip) struct OccluderRefreshRunCount(usize);

#[derive(SystemParam)]
pub(in crate::widgets::tooltip) struct WorldTooltipRevealOcclusion<'w, 's> {
    occluders:       SceneOccluders<'w, 's>,
    transforms:      TransformHelper<'w, 's>,
    cameras:         Query<'w, 's, Option<&'static RenderLayers>, With<Camera>>,
    panels:          Query<'w, 's, &'static DiegeticPanel>,
    anchor_geometry: Query<'w, 's, &'static ResolvedAnchorGeometry>,
    placements:      Query<'w, 's, &'static TooltipPlacementState>,
}

impl WorldTooltipRevealOcclusion<'_, '_> {
    pub(in crate::widgets::tooltip) fn clear(
        &mut self,
        controller: Entity,
        materialized: &MaterializedTooltip,
        panel: &DiegeticPanel,
        camera: Entity,
    ) -> bool {
        if self.occluders.ray_cast.is_none() {
            return true;
        }
        let Ok(camera_layers) = self.cameras.get(camera) else {
            return false;
        };
        let camera_layers = camera_layers
            .cloned()
            .unwrap_or_else(|| RenderLayers::layer(0));
        let Ok(camera_global) = self.transforms.compute_global_transform(camera) else {
            return false;
        };
        let Ok(tooltip_global) = self.transforms.compute_global_transform(controller) else {
            return false;
        };
        let Ok(plane) = PanelPlane::from_panel(panel, &tooltip_global) else {
            return false;
        };
        let origin = camera_global.translation();
        let Some(control_points) = world_target::world_control_sample_points(
            materialized.target,
            &self.transforms,
            &self.panels,
            &self.anchor_geometry,
        ) else {
            return false;
        };
        let mut scene_occlusion =
            self.occluders
                .scene_occlusion(controller, materialized.target, camera_layers);
        if control_fully_occluded(control_points, origin, &mut scene_occlusion) {
            return false;
        }
        if self
            .placements
            .get(controller)
            .is_ok_and(|placement| placement.world_occlusion == WorldTooltipOcclusion::CardOverlay)
        {
            return true;
        }
        ![
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomRight,
            Anchor::BottomLeft,
            Anchor::Center,
        ]
        .into_iter()
        .any(|anchor| scene_occlusion.blocks(origin, plane.point(anchor)))
    }
}

/// A scene mesh's layers, material, interaction role, and panel owner.
type SceneMesh = (
    Option<&'static RenderLayers>,
    Option<&'static MeshMaterial3d<StandardMaterial>>,
    Has<PanelInteractionMesh>,
    Option<&'static PanelOwned>,
);

type OccludingPanel = (
    &'static DiegeticPanel,
    &'static GlobalTransform,
    Option<&'static Resolved<Sidedness>>,
);

/// The scene meshes that can hide a world tooltip or its target from a camera.
#[derive(SystemParam)]
pub(in crate::widgets::tooltip) struct SceneOccluders<'w, 's> {
    ray_cast:       Option<MeshRayCast<'w, 's>>,
    meshes:         Query<'w, 's, SceneMesh, With<Mesh3d>>,
    materials:      Option<Res<'w, Assets<StandardMaterial>>>,
    children:       Query<'w, 's, &'static Children>,
    tooltip_meshes: Query<'w, 's, (Entity, &'static PanelOwned), With<Mesh3d>>,
    panels:         Query<'w, 's, OccludingPanel>,
}

impl<'w, 's> SceneOccluders<'w, 's> {
    /// Occlusion as seen by `camera_layers` for the tooltip `controller` owns,
    /// anchored on `target`.
    ///
    /// The tooltip and target meshes do not hide the tooltip.
    /// Other panel meshes hide it only when the ray reaches a drawn side.
    pub(super) fn scene_occlusion(
        &mut self,
        controller: Entity,
        target: Entity,
        camera_layers: RenderLayers,
    ) -> SceneOcclusion<'_, 'w, 's> {
        let Some(ray_cast) = self.ray_cast.as_mut() else {
            return SceneOcclusion::NoMeshAssets;
        };
        let own_meshes = self
            .tooltip_meshes
            .iter()
            .filter_map(|(mesh, owner)| (owner.owner() == controller).then_some(mesh))
            .chain(std::iter::once(target))
            .chain(self.children.iter_descendants(target))
            .collect();
        SceneOcclusion::Available {
            ray_cast,
            own_meshes,
            camera_layers,
            meshes: &self.meshes,
            materials: self.materials.as_deref(),
            panels: &self.panels,
        }
    }
}

pub(super) enum SceneOcclusion<'a, 'w, 's> {
    Available {
        ray_cast:      &'a mut MeshRayCast<'w, 's>,
        /// The tooltip's own meshes and its target's, which never hide it.
        own_meshes:    HashSet<Entity>,
        camera_layers: RenderLayers,
        meshes:        &'a Query<'w, 's, SceneMesh, With<Mesh3d>>,
        materials:     Option<&'a Assets<StandardMaterial>>,
        panels:        &'a Query<'w, 's, OccludingPanel>,
    },
    NoMeshAssets,
}

impl SceneOcclusion<'_, '_, '_> {
    pub(super) fn blocks(&mut self, origin: Vec3, point: Vec3) -> bool {
        let Self::Available {
            ray_cast,
            own_meshes,
            camera_layers,
            meshes,
            materials,
            panels,
        } = self
        else {
            return false;
        };
        let to_point = point - origin;
        let Ok(direction) = Dir3::new(to_point) else {
            return true;
        };
        let filter = |entity| {
            !own_meshes.contains(&entity)
                && meshes
                    .get(entity)
                    .is_ok_and(|(layers, material, interaction, owner)| {
                        layers
                            .cloned()
                            .unwrap_or_else(|| RenderLayers::layer(0))
                            .intersects(camera_layers)
                            && panel_draws_toward_ray(
                                owner.map_or(entity, |owner| owner.owner()),
                                direction,
                                panels,
                            )
                            && (interaction || !see_through(material, *materials))
                    })
        };
        let settings = MeshRayCastSettings::default()
            .with_visibility(RayCastVisibility::Visible)
            .with_filter(&filter);
        ray_cast
            .cast_ray(Ray3d::new(origin, direction), &settings)
            .iter()
            .any(|(_, hit)| hit.distance < to_point.length() - 0.001)
    }
}

fn panel_draws_toward_ray(entity: Entity, direction: Dir3, panels: &Query<OccludingPanel>) -> bool {
    let Ok((panel, pose, sidedness)) = panels.get(entity) else {
        return true;
    };
    let sidedness = sidedness.map_or(Sidedness::BothSides, |value| *value.value());
    if sidedness == Sidedness::BothSides {
        return true;
    }
    let Ok(plane) = PanelPlane::from_panel(panel, pose) else {
        return true;
    };
    let toward_front = direction.dot(plane.normal()) < 0.0;
    match sidedness {
        Sidedness::FrontOnly => toward_front,
        Sidedness::BackOnly => !toward_front,
        Sidedness::BothSides => true,
    }
}

/// Whether a mesh drawn with `material` lets the scene behind it show: a
/// blended tint below full alpha, such as a selection box, hides nothing.
///
/// A panel's interaction mesh is clear and blended too, but it stands for the
/// panel's drawn face, so the filter never asks this of one.
fn see_through(
    material: Option<&MeshMaterial3d<StandardMaterial>>,
    materials: Option<&Assets<StandardMaterial>>,
) -> bool {
    material
        .zip(materials)
        .and_then(|(material, materials)| materials.get(&material.0))
        .is_some_and(|material| {
            !matches!(material.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_))
                && material.base_color.alpha() < 1.0
        })
}

pub(super) fn world_target_fully_occluded(
    target: &WorldTooltipTarget,
    camera: &TooltipCameraView<'_>,
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> bool {
    let points = [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::BottomLeft,
        Anchor::Center,
    ];
    control_fully_occluded(
        points.map(|anchor| target.anchor(anchor).position),
        camera.global_transform.translation(),
        scene_occlusion,
    )
}

fn control_fully_occluded(
    points: [Vec3; 5],
    origin: Vec3,
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> bool {
    points
        .into_iter()
        .all(|point| scene_occlusion.blocks(origin, point))
}

#[cfg(test)]
mod tests {
    use bevy::camera::primitives::Aabb;
    use bevy::camera::visibility::InheritedVisibility;
    use bevy::camera::visibility::ViewVisibility;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::mesh_picking::ray_cast::RayCastBackfaces;
    use bevy::prelude::App;
    use bevy::prelude::Mesh;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Quat;
    use bevy::prelude::Rectangle;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::transform::TransformPlugin;

    use super::*;
    use crate::HeadlessLayoutPlugin;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;

    #[derive(Clone, Copy, Debug)]
    enum PanelMeshRole {
        Root,
        OwnedInteraction,
    }

    struct PanelOcclusionScene {
        app:    App,
        target: Entity,
    }

    impl PanelOcclusionScene {
        fn new(sidedness: Sidedness, rotation: Quat, role: PanelMeshRole) -> Result<Self, String> {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .init_resource::<Assets<Mesh>>();
            let panel = DiegeticPanel::world()
                .size(2.0, 2.0)
                .build()
                .map_err(|error| error.to_string())?;
            let pose = Transform::from_xyz(0.0, 0.0, 2.0).with_rotation(rotation);
            let root = app
                .world_mut()
                .spawn((
                    panel,
                    pose,
                    GlobalTransform::from(pose),
                    Resolved(sidedness),
                ))
                .id();
            let mesh = app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(Rectangle::new(2.0, 2.0));
            let mesh_entity = match role {
                PanelMeshRole::Root => root,
                PanelMeshRole::OwnedInteraction => app
                    .world_mut()
                    .spawn((
                        PanelOwned::from(root),
                        PanelInteractionMesh::new(Vec2::splat(2.0), Vec2::ZERO),
                        GlobalTransform::from(pose),
                    ))
                    .id(),
            };
            app.world_mut().entity_mut(mesh_entity).insert((
                Mesh3d(mesh),
                RayCastBackfaces,
                Aabb::from_min_max(Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)),
                InheritedVisibility::VISIBLE,
                ViewVisibility::VISIBLE,
            ));
            let target = app.world_mut().spawn_empty().id();
            Ok(Self { app, target })
        }

        fn blocks(&mut self) -> Result<bool, String> {
            let target = self.target;
            self.app
                .world_mut()
                .run_system_once(move |mut occluders: SceneOccluders| {
                    occluders
                        .scene_occlusion(Entity::PLACEHOLDER, target, RenderLayers::layer(0))
                        .blocks(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO)
                })
                .map_err(|error| error.to_string())
        }

        fn add_opaque_mesh(&mut self) {
            let mesh = self
                .app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(Rectangle::new(2.0, 2.0));
            self.app.world_mut().spawn((
                Mesh3d(mesh),
                Aabb::from_min_max(Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)),
                GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 1.0)),
                InheritedVisibility::VISIBLE,
                ViewVisibility::VISIBLE,
            ));
        }
    }

    #[test]
    fn an_opaque_mesh_behind_an_undrawn_panel_still_hides_a_tooltip() -> Result<(), String> {
        for role in [PanelMeshRole::Root, PanelMeshRole::OwnedInteraction] {
            for (sidedness, rotation) in [
                (
                    Sidedness::FrontOnly,
                    Quat::from_rotation_y(std::f32::consts::PI),
                ),
                (Sidedness::BackOnly, Quat::IDENTITY),
            ] {
                let mut scene = PanelOcclusionScene::new(sidedness, rotation, role)?;
                assert!(
                    !scene.blocks()?,
                    "an undrawn {sidedness:?} {role:?} must not hide help"
                );
                scene.add_opaque_mesh();
                assert!(
                    scene.blocks()?,
                    "an excluded {sidedness:?} {role:?} must not mask the opaque mesh behind it"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn a_panel_seen_from_its_drawn_side_still_hides_a_tooltip() -> Result<(), String> {
        for role in [PanelMeshRole::Root, PanelMeshRole::OwnedInteraction] {
            for (sidedness, rotation) in [
                (Sidedness::FrontOnly, Quat::IDENTITY),
                (
                    Sidedness::BackOnly,
                    Quat::from_rotation_y(std::f32::consts::PI),
                ),
                (Sidedness::BothSides, Quat::IDENTITY),
                (
                    Sidedness::BothSides,
                    Quat::from_rotation_y(std::f32::consts::PI),
                ),
            ] {
                let mut scene = PanelOcclusionScene::new(sidedness, rotation, role)?;
                assert!(
                    scene.blocks()?,
                    "a drawn {sidedness:?} {role:?} must hide help"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn idle_frames_skip_occluder_transform_refresh() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app.init_resource::<OccluderRefreshRunCount>();
        app.world_mut().spawn((
            Mesh3d::default(),
            Transform::default(),
            GlobalTransform::default(),
        ));
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(app.world().resource::<OccluderRefreshRunCount>().0, 0);
    }
}
