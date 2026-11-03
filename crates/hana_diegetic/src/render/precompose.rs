use bevy::camera::Camera3d;
use bevy::camera::ClearColorConfig;
use bevy::camera::ImageRenderTarget;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::camera::Viewport;
use bevy::color::Color;
use bevy::ecs::entity::Entity;
use bevy::image::Image;
use bevy::image::ImageSampler;
use bevy::light::cluster::ClusterConfig;
use bevy::math::UVec2;
use bevy::math::Vec3;
use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Handle;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::OrthographicProjection;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Transform;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::render::render_resource::TextureFormat;
use hana_kana::ToF32;
use hana_kana::ToU32;

use super::PrivateViewLayer;
use super::PrivateViewLayerAllocator;
use super::PrivateViewLayers;
use crate::cascade::Cascade;
use crate::cascade::CascadeFrom;
use crate::cascade::FontUnit;
use crate::cascade::HdrTextCoverageBias;
use crate::cascade::PanelDefaults;
use crate::cascade::Resolved;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::layout::Anchor;
use crate::layout::BoundingBox;
use crate::layout::Dimension;
use crate::layout::LayoutTree;
use crate::layout::Lighting;
use crate::layout::Pt;
use crate::layout::RenderCommand;
use crate::layout::RenderCommandKind;
use crate::layout::Unit;
use crate::panel;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::PanelLayout;
use crate::panel::PanelPrecomposeCache;
use crate::panel::PrecomposeCacheEntry;
use crate::panel::PrecomposeHelper;

const PRECOMPOSE_CAMERA_ORDER: isize = -100;
const PRECOMPOSE_CAMERA_Z: f32 = 1000.0;
const PRECOMPOSE_CAMERA_FAR: f32 = 2000.0;
const PRECOMPOSE_SUPERSAMPLE: u32 = 8;
const MIN_PRECOMPOSE_PIXELS: u32 = 1;
const PRECOMPOSE_TEXT_COVERAGE_BIAS: HdrTextCoverageBias = HdrTextCoverageBias::NO_BIAS;

/// Holds one precomposed element's helper panel and camera outside the source
/// panel hierarchy, so the camera outlives an `Update` despawn of that panel.
#[derive(bevy::prelude::Component)]
#[require(Transform, Visibility)]
pub(super) struct PrecomposeRig {
    panel: Entity,
    layer: PrivateViewLayer,
}

/// Despawns each rig unless its source has `DiegeticPanel` and a
/// `PanelPrecomposeCache` entry that owns the rig.
pub(super) fn despawn_orphaned_precompose_rigs(
    rigs: Query<(Entity, &PrecomposeRig)>,
    panels: Query<&PanelPrecomposeCache, With<DiegeticPanel>>,
    mut commands: Commands,
) {
    for (entity, rig) in &rigs {
        if !panels
            .get(rig.panel)
            .is_ok_and(|cache| cache.contains_rig(entity))
        {
            commands.entity(entity).despawn();
        }
    }
}

/// Returns a rig's private render layer when the rig is removed.
pub(super) fn release_precompose_rig_layer(
    trigger: On<Remove<PrecomposeRig>>,
    rigs: Query<&PrecomposeRig>,
    mut layers: ResMut<PrivateViewLayers>,
) {
    if let Ok(rig) = rigs.get(trigger.entity) {
        layers.release(rig.layer);
    }
}

/// Synchronizes hidden LDR helper panels for every precomposed element boundary.
pub(super) fn ensure_panel_precompose_caches(
    mut panels: Query<
        (
            Entity,
            &DiegeticPanel,
            &ComputedDiegeticPanel,
            &mut PanelPrecomposeCache,
            Option<&Resolved<FontUnit>>,
        ),
        Changed<ComputedDiegeticPanel>,
    >,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    defaults: Res<PanelDefaults>,
    mut private_view_layer_allocator: PrivateViewLayerAllocator,
) {
    for (panel_entity, panel, computed, mut cache, font_unit) in &mut panels {
        let PanelLayout::Solved(result) = computed.layout() else {
            clear_cache(&mut commands, &mut cache);
            continue;
        };
        let precompose_commands = collect_precompose_commands(result.commands.as_slice());
        remove_stale_entries(&mut commands, &mut cache, &precompose_commands);
        if precompose_commands.is_empty() {
            continue;
        }

        let font_unit = font_unit.map_or(defaults.panel_font_unit, |resolved| resolved.0.0);
        let scaled_tree = panel
            .tree()
            .scaled(panel.layout_unit().to_points(), font_unit.to_points());
        for (element_idx, bounds) in precompose_commands {
            let Some(subtree) = precompose_subtree(&scaled_tree, element_idx, bounds) else {
                continue;
            };
            let pixel_size = precompose_pixel_size(bounds);
            let Some(helper_panel) = helper_panel_for_subtree(subtree, bounds) else {
                continue;
            };
            sync_entry(
                SyncEntry {
                    commands: &mut commands,
                    images: &mut images,
                    cache: &mut cache,
                    panel_entity,
                    element_idx,
                    pixel_size,
                    bounds,
                    helper_panel,
                },
                &mut private_view_layer_allocator,
            );
        }
    }
}

/// Removes old render targets after cameras have had a frame to stop targeting
/// them.
pub(super) fn cleanup_retired_precompose_images(
    mut caches: Query<&mut PanelPrecomposeCache>,
    mut images: ResMut<Assets<Image>>,
) {
    for mut cache in &mut caches {
        for handle in cache.drain_ready_retired_images() {
            images.remove(handle.id());
        }
    }
}

/// Reactivates helper cameras after their image targets have had a frame to
/// propagate through Bevy's camera target bookkeeping.
pub(super) fn activate_pending_precompose_cameras(
    mut caches: Query<&mut PanelPrecomposeCache>,
    mut cameras: Query<&mut Camera>,
) {
    for mut cache in &mut caches {
        for camera_entity in cache.drain_ready_camera_activations() {
            if let Ok(mut camera) = cameras.get_mut(camera_entity) {
                camera.is_active = true;
            }
        }
    }
}

fn collect_precompose_commands(commands: &[RenderCommand]) -> Vec<(usize, BoundingBox)> {
    commands
        .iter()
        .filter_map(|command| {
            matches!(command.kind, RenderCommandKind::PrecomposeLdr)
                .then_some((command.element_idx, command.bounds))
        })
        .collect()
}

fn precompose_subtree(
    scaled_tree: &LayoutTree,
    element_idx: usize,
    bounds: BoundingBox,
) -> Option<LayoutTree> {
    scaled_tree.precompose_subtree(
        element_idx,
        Dimension {
            value: bounds.width,
            unit:  None,
        },
        Dimension {
            value: bounds.height,
            unit:  None,
        },
    )
}

fn precompose_pixel_size(bounds: BoundingBox) -> UVec2 {
    UVec2::new(
        precompose_axis_pixels(bounds.width),
        precompose_axis_pixels(bounds.height),
    )
}

fn precompose_axis_pixels(points: f32) -> u32 {
    let pixels = if points.is_finite() && points > 0.0 {
        (points * PRECOMPOSE_SUPERSAMPLE.to_f32()).ceil()
    } else {
        MIN_PRECOMPOSE_PIXELS.to_f32()
    };
    pixels
        .clamp(MIN_PRECOMPOSE_PIXELS.to_f32(), u32::MAX.to_f32())
        .to_u32()
}

fn helper_panel_for_subtree(subtree: LayoutTree, bounds: BoundingBox) -> Option<DiegeticPanel> {
    DiegeticPanel::world()
        .size(Pt(bounds.width), Pt(bounds.height))
        .world_height(bounds.height.max(MIN_PRECOMPOSE_PIXELS.to_f32()))
        .anchor(Anchor::Center)
        .font_unit(Unit::Points)
        .hdr_text_coverage_bias(PRECOMPOSE_TEXT_COVERAGE_BIAS.0)
        .with_tree(subtree)
        .build()
        .ok()
}

struct SyncEntry<'a, 'w, 's> {
    commands:     &'a mut Commands<'w, 's>,
    images:       &'a mut Assets<Image>,
    cache:        &'a mut PanelPrecomposeCache,
    panel_entity: Entity,
    element_idx:  usize,
    pixel_size:   UVec2,
    bounds:       BoundingBox,
    helper_panel: DiegeticPanel,
}

fn sync_entry(
    input: SyncEntry<'_, '_, '_>,
    private_view_layer_allocator: &mut PrivateViewLayerAllocator,
) {
    let SyncEntry {
        commands,
        images,
        cache,
        panel_entity,
        element_idx,
        pixel_size,
        bounds,
        helper_panel,
    } = input;

    if let Some(update) = update_existing_cache_entry(cache, images, element_idx, pixel_size) {
        sync_existing_entry(ExistingEntrySync {
            commands,
            cache,
            update,
            panel_entity,
            helper_panel,
            bounds,
        });
        return;
    }

    spawn_precompose_entry(
        SpawnEntry {
            commands,
            images,
            cache,
            panel_entity,
            element_idx,
            pixel_size,
            bounds,
            helper_panel,
        },
        private_view_layer_allocator,
    );
}

struct ExistingEntryUpdate {
    helper_panel:  Entity,
    camera:        Entity,
    image:         Handle<Image>,
    pixel_size:    UVec2,
    image_changed: bool,
    retired_image: Option<Handle<Image>>,
    layer:         PrivateViewLayer,
}

fn update_existing_cache_entry(
    cache: &mut PanelPrecomposeCache,
    images: &mut Assets<Image>,
    element_idx: usize,
    pixel_size: UVec2,
) -> Option<ExistingEntryUpdate> {
    let entry = cache.entries_mut().get_mut(&element_idx)?;
    let image_changed = entry.pixel_size != pixel_size;
    entry.pixel_size = pixel_size;
    let retired_image = image_changed
        .then(|| std::mem::replace(&mut entry.image, images.add(precompose_image(pixel_size))));
    Some(ExistingEntryUpdate {
        helper_panel: entry.helper_panel,
        camera: entry.camera,
        image: entry.image.clone(),
        pixel_size,
        image_changed,
        retired_image,
        layer: entry.layer,
    })
}

struct ExistingEntrySync<'a, 'w, 's> {
    commands:     &'a mut Commands<'w, 's>,
    cache:        &'a mut PanelPrecomposeCache,
    update:       ExistingEntryUpdate,
    panel_entity: Entity,
    helper_panel: DiegeticPanel,
    bounds:       BoundingBox,
}

fn sync_existing_entry(input: ExistingEntrySync<'_, '_, '_>) {
    let ExistingEntrySync {
        commands,
        cache,
        update,
        panel_entity,
        helper_panel,
        bounds,
    } = input;
    if let Some(retired_image) = update.retired_image {
        cache.retire_image(retired_image);
    }
    commands.run_system_cached_with(
        panel::apply_precompose_helper_panel,
        (update.helper_panel, helper_panel),
    );
    commands.entity(update.helper_panel).insert((
        PrecomposeHelper,
        Transform::IDENTITY,
        CascadeFrom::new(panel_entity),
        Cascade::Override(Lighting::Unlit),
        Cascade::Override(PRECOMPOSE_TEXT_COVERAGE_BIAS),
        update.layer.render_layers(),
    ));
    if update.image_changed {
        cache.defer_camera_activation(update.camera);
    }
    commands.entity(update.camera).insert((
        PrecomposeHelper,
        precompose_camera(
            update.pixel_size,
            bounds,
            &update.image,
            !update.image_changed,
        ),
        ClusterConfig::Single,
        precompose_projection(bounds),
        Transform::from_translation(Vec3::Z * PRECOMPOSE_CAMERA_Z).looking_at(Vec3::ZERO, Vec3::Y),
        update.layer.render_layers(),
    ));
}

struct SpawnEntry<'a, 'w, 's> {
    commands:     &'a mut Commands<'w, 's>,
    images:       &'a mut Assets<Image>,
    cache:        &'a mut PanelPrecomposeCache,
    panel_entity: Entity,
    element_idx:  usize,
    pixel_size:   UVec2,
    bounds:       BoundingBox,
    helper_panel: DiegeticPanel,
}

fn spawn_precompose_entry(
    input: SpawnEntry<'_, '_, '_>,
    private_view_layer_allocator: &mut PrivateViewLayerAllocator,
) {
    let SpawnEntry {
        commands,
        images,
        cache,
        panel_entity,
        element_idx,
        pixel_size,
        bounds,
        helper_panel,
    } = input;
    let Ok(layer) = private_view_layer_allocator.allocate() else {
        bevy::prelude::error!(
            target: DIAGNOSTIC_LOG_TARGET, "No private render layer is available for precompose element {element_idx}"
        );
        return;
    };
    let image = images.add(precompose_image(pixel_size));
    let mut helper = Entity::PLACEHOLDER;
    let mut camera = Entity::PLACEHOLDER;
    let rig = commands
        .spawn((
            Name::new(format!("precompose rig {element_idx}")),
            PrecomposeHelper,
            PrecomposeRig {
                panel: panel_entity,
                layer,
            },
            Transform::IDENTITY,
        ))
        .id();
    commands.entity(rig).with_children(|children| {
        helper = children
            .spawn((
                Name::new(format!("precompose helper panel {element_idx}")),
                PrecomposeHelper,
                helper_panel,
                Transform::IDENTITY,
                CascadeFrom::new(panel_entity),
                Cascade::Override(Lighting::Unlit),
                Cascade::Override(PRECOMPOSE_TEXT_COVERAGE_BIAS),
                layer.render_layers(),
            ))
            .id();
        camera = children
            .spawn((
                Name::new(format!("precompose ldr camera {element_idx}")),
                PrecomposeHelper,
                precompose_camera(pixel_size, bounds, &image, false),
                ClusterConfig::Single,
                precompose_projection(bounds),
                Transform::from_translation(Vec3::Z * PRECOMPOSE_CAMERA_Z)
                    .looking_at(Vec3::ZERO, Vec3::Y),
                layer.render_layers(),
            ))
            .id();
    });
    cache.defer_camera_activation(camera);
    cache.entries_mut().insert(
        element_idx,
        PrecomposeCacheEntry {
            image,
            rig,
            layer,
            helper_panel: helper,
            camera,
            pixel_size,
        },
    );
}

fn precompose_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(
        size.x.max(MIN_PRECOMPOSE_PIXELS),
        size.y.max(MIN_PRECOMPOSE_PIXELS),
        TextureFormat::Bgra8UnormSrgb,
        None,
    );
    image.sampler = ImageSampler::linear();
    image
}

fn precompose_camera(
    pixel_size: UVec2,
    _bounds: BoundingBox,
    image: &Handle<Image>,
    is_active: bool,
) -> (Camera3d, Camera, RenderTarget) {
    (
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            is_active,
            order: PRECOMPOSE_CAMERA_ORDER,
            viewport: Some(Viewport {
                physical_position: UVec2::ZERO,
                physical_size: pixel_size.max(UVec2::ONE),
                ..default()
            }),
            ..default()
        },
        RenderTarget::Image(ImageRenderTarget::from(image.clone())),
    )
}

fn precompose_projection(bounds: BoundingBox) -> Projection {
    Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::FixedVertical {
            viewport_height: bounds.height.max(MIN_PRECOMPOSE_PIXELS.to_f32()),
        },
        far: PRECOMPOSE_CAMERA_FAR,
        ..OrthographicProjection::default_3d()
    })
}

fn remove_stale_entries(
    commands: &mut Commands,
    cache: &mut PanelPrecomposeCache,
    live: &[(usize, BoundingBox)],
) {
    let stale: Vec<_> = cache
        .entries_mut()
        .keys()
        .copied()
        .filter(|element_idx| !live.iter().any(|(live_idx, _)| live_idx == element_idx))
        .collect();
    for element_idx in stale {
        if let Some(entry) = cache.entries_mut().remove(&element_idx) {
            commands.entity(entry.rig).despawn();
            cache.retire_image(entry.image);
        }
    }
}

fn clear_cache(commands: &mut Commands, cache: &mut PanelPrecomposeCache) {
    let entries: Vec<_> = cache
        .entries_mut()
        .drain()
        .map(|(_, entry)| entry)
        .collect();
    for entry in entries {
        commands.entity(entry.rig).despawn();
        cache.retire_image(entry.image);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected missing ECS state"
)]
mod tests {
    use std::sync::Arc;

    use bevy::camera::visibility::RenderLayers;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::ChildOf;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::World;

    use super::*;
    use crate::cascade;
    use crate::layout::El;
    use crate::layout::LayoutEngine;
    use crate::layout::MeasureTextFn;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::TextStyle;

    const PANEL_WIDTH: f32 = 100.0;
    const PANEL_HEIGHT: f32 = 50.0;
    const PRECOMPOSE_WIDTH: f32 = 80.0;
    const PRECOMPOSE_HEIGHT: f32 = 30.0;
    const PRECOMPOSE_ELEMENT_INDEX: usize = 1;

    fn measure_text() -> MeasureTextFn {
        Arc::new(|text: &str, measure: &TextMeasure| TextDimensions {
            width:       text.chars().count().to_f32() * measure.size,
            height:      measure.size,
            line_height: measure.size,
        })
    }

    fn precompose_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>();
        app.init_resource::<PanelDefaults>();
        app.init_resource::<PrivateViewLayers>();
        app.add_observer(release_precompose_rig_layer);
        app.add_systems(
            PostUpdate,
            (
                despawn_orphaned_precompose_rigs,
                ensure_panel_precompose_caches.after(despawn_orphaned_precompose_rigs),
            ),
        );
        app
    }

    fn spawn_precomposing_panel(app: &mut App, panel: DiegeticPanel) -> Entity {
        let result =
            LayoutEngine::new(measure_text()).compute(panel.tree(), PANEL_WIDTH, PANEL_HEIGHT, 1.0);
        let panel_entity = app.world_mut().spawn(panel).id();
        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel_entity)
            .expect("required computed panel")
            .set_result(result);
        panel_entity
    }

    #[test]
    fn cache_sync_spawns_helper_panel_camera_and_image() {
        let mut app = precompose_test_app();
        app.add_plugins(cascade::cascade_plugin::<Lighting>());
        app.add_plugins(cascade::cascade_plugin::<HdrTextCoverageBias>());

        let panel = DiegeticPanel::world()
            .size(Pt(PANEL_WIDTH), Pt(PANEL_HEIGHT))
            .font_unit(Unit::Points)
            .hdr_text_coverage_bias(2.0)
            .layout(|builder| {
                builder.with(
                    El::column()
                        .size(PRECOMPOSE_WIDTH, PRECOMPOSE_HEIGHT)
                        .precompose_ldr(),
                    |builder| {
                        builder.text(("child", TextStyle::new(10.0)));
                    },
                );
            })
            .build()
            .expect("valid panel");
        let result =
            LayoutEngine::new(measure_text()).compute(panel.tree(), PANEL_WIDTH, PANEL_HEIGHT, 1.0);
        let panel_entity = app.world_mut().spawn(panel).id();
        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel_entity)
            .expect("required computed panel")
            .set_result(result);

        app.update();

        let cache = app
            .world()
            .get::<PanelPrecomposeCache>(panel_entity)
            .expect("required precompose cache");
        let entry = cache
            .entry(PRECOMPOSE_ELEMENT_INDEX)
            .expect("precompose cache entry");
        assert_eq!(
            entry.pixel_size,
            UVec2::new(
                PRECOMPOSE_WIDTH.to_u32() * PRECOMPOSE_SUPERSAMPLE,
                PRECOMPOSE_HEIGHT.to_u32() * PRECOMPOSE_SUPERSAMPLE,
            )
        );
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&entry.image)
                .is_some()
        );
        assert!(
            app.world()
                .get::<DiegeticPanel>(entry.helper_panel)
                .is_some()
        );
        assert_eq!(
            app.world()
                .get::<CascadeFrom>(entry.helper_panel)
                .expect("helper panel should inherit from its source panel")
                .target(),
            panel_entity
        );
        assert!(app.world().get::<Camera>(entry.camera).is_some());
        assert_eq!(
            app.world()
                .get::<ChildOf>(entry.helper_panel)
                .map(ChildOf::parent),
            Some(entry.rig)
        );
        assert!(app.world().get::<ChildOf>(entry.rig).is_none());
        let lighting = app
            .world()
            .get::<Resolved<Lighting>>(entry.helper_panel)
            .expect("helper panel lighting is resolved");
        assert!(matches!(lighting.0, Lighting::Unlit));
        let coverage_override = app
            .world()
            .get::<Cascade<HdrTextCoverageBias>>(entry.helper_panel)
            .expect("helper panel coverage bias is overridden");
        assert_eq!(
            coverage_override,
            &Cascade::Override(PRECOMPOSE_TEXT_COVERAGE_BIAS)
        );
        let coverage_resolved = app
            .world()
            .get::<Resolved<HdrTextCoverageBias>>(entry.helper_panel)
            .expect("helper panel coverage bias is resolved");
        assert_eq!(coverage_resolved.0, PRECOMPOSE_TEXT_COVERAGE_BIAS);
    }

    #[test]
    fn cache_sync_preserves_image_when_precompose_bounds_are_unchanged() {
        let mut app = precompose_test_app();

        let panel = DiegeticPanel::world()
            .size(Pt(PANEL_WIDTH), Pt(PANEL_HEIGHT))
            .font_unit(Unit::Points)
            .layout(|builder| {
                builder.with(
                    El::column()
                        .size(PRECOMPOSE_WIDTH, PRECOMPOSE_HEIGHT)
                        .precompose_ldr(),
                    |builder| {
                        builder.text(("child", TextStyle::new(10.0)));
                    },
                );
            })
            .build()
            .expect("valid panel");
        let result =
            LayoutEngine::new(measure_text()).compute(panel.tree(), PANEL_WIDTH, PANEL_HEIGHT, 1.0);
        let panel_entity = app.world_mut().spawn(panel).id();
        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel_entity)
            .expect("required computed panel")
            .set_result(result.clone());

        app.update();

        let first_image = app
            .world()
            .get::<PanelPrecomposeCache>(panel_entity)
            .expect("required precompose cache")
            .entry(PRECOMPOSE_ELEMENT_INDEX)
            .expect("precompose cache entry")
            .image
            .clone();

        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel_entity)
            .expect("required computed panel")
            .set_result(result);

        app.update();

        let second_image = app
            .world()
            .get::<PanelPrecomposeCache>(panel_entity)
            .expect("required precompose cache")
            .entry(PRECOMPOSE_ELEMENT_INDEX)
            .expect("precompose cache entry")
            .image
            .clone();
        assert_eq!(first_image, second_image);
    }

    #[test]
    fn cache_sync_refreshes_helper_tree_when_text_changes_with_same_bounds() {
        let mut app = precompose_test_app();

        let panel = precompose_panel("Blend");
        let result =
            LayoutEngine::new(measure_text()).compute(panel.tree(), PANEL_WIDTH, PANEL_HEIGHT, 1.0);
        let panel_entity = app.world_mut().spawn(panel).id();
        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel_entity)
            .expect("required computed panel")
            .set_result(result);

        app.update();

        let (helper_panel, first_image) = {
            let cache = app
                .world()
                .get::<PanelPrecomposeCache>(panel_entity)
                .expect("required precompose cache");
            let entry = cache
                .entry(PRECOMPOSE_ELEMENT_INDEX)
                .expect("precompose cache entry");
            let helper = app
                .world()
                .get::<DiegeticPanel>(entry.helper_panel)
                .expect("helper panel should exist");
            assert_eq!(helper.tree().element_text(1), Some("Blend"));
            (entry.helper_panel, entry.image.clone())
        };

        let updated_panel = precompose_panel("Add");
        let result = LayoutEngine::new(measure_text()).compute(
            updated_panel.tree(),
            PANEL_WIDTH,
            PANEL_HEIGHT,
            1.0,
        );
        app.world_mut()
            .get_mut::<DiegeticPanel>(panel_entity)
            .expect("source panel should exist")
            .replace_tree_full_rebuild(updated_panel.tree().clone());
        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel_entity)
            .expect("required computed panel")
            .set_result(result);

        app.update();

        let cache = app
            .world()
            .get::<PanelPrecomposeCache>(panel_entity)
            .expect("required precompose cache");
        let entry = cache
            .entry(PRECOMPOSE_ELEMENT_INDEX)
            .expect("precompose cache entry");
        let helper = app
            .world()
            .get::<DiegeticPanel>(helper_panel)
            .expect("helper panel should exist");
        assert_eq!(entry.image, first_image);
        assert_eq!(helper.tree().element_text(1), Some("Add"));
    }

    fn precompose_panel(text: &str) -> DiegeticPanel {
        DiegeticPanel::world()
            .size(Pt(PANEL_WIDTH), Pt(PANEL_HEIGHT))
            .font_unit(Unit::Points)
            .layout(|builder| {
                builder.with(
                    El::column()
                        .size(PRECOMPOSE_WIDTH, PRECOMPOSE_HEIGHT)
                        .precompose_ldr(),
                    |builder| {
                        builder.text((text, TextStyle::new(10.0)));
                    },
                );
            })
            .build()
            .expect("valid panel")
    }

    #[derive(bevy::prelude::Component)]
    struct CameraInsertMarker;

    #[test]
    fn despawning_a_precomposing_panel_keeps_its_rig_until_post_update() {
        let mut app = precompose_test_app();
        app.add_plugins(cascade::cascade_plugin::<Lighting>());
        app.add_plugins(cascade::cascade_plugin::<HdrTextCoverageBias>());
        let panel = spawn_precomposing_panel(&mut app, precompose_panel("Blend"));
        app.update();
        let entry = app
            .world()
            .get::<PanelPrecomposeCache>(panel)
            .expect("precompose cache")
            .entry(PRECOMPOSE_ELEMENT_INDEX)
            .expect("precompose entry");
        let (rig, helper, camera, layer) =
            (entry.rig, entry.helper_panel, entry.camera, entry.layer);

        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.entity(panel).despawn();
            })
            .expect("panel despawn system should run");
        assert!(app.world().get_entity(panel).is_err());
        for entity in [rig, helper, camera] {
            assert!(app.world().get_entity(entity).is_ok());
        }
        app.world_mut()
            .entity_mut(camera)
            .insert(CameraInsertMarker);
        app.update();
        for entity in [rig, helper, camera] {
            assert!(app.world().get_entity(entity).is_err());
        }
        let recycled = app
            .world_mut()
            .run_system_once(
                |mut private_view_layer_allocator: PrivateViewLayerAllocator| {
                    private_view_layer_allocator
                        .allocate()
                        .expect("free private layer")
                },
            )
            .expect("layer allocation system should run");
        assert_eq!(recycled, layer);
    }

    #[test]
    fn a_panel_role_removed_and_reinserted_does_not_strand_its_old_rig() {
        let mut app = precompose_test_app();
        let panel = spawn_precomposing_panel(&mut app, precompose_panel("Before"));
        app.update();
        let entry = app
            .world()
            .get::<PanelPrecomposeCache>(panel)
            .expect("precompose cache")
            .entry(PRECOMPOSE_ELEMENT_INDEX)
            .expect("precompose entry");
        let old_entities = [entry.rig, entry.helper_panel, entry.camera];

        let replacement = precompose_panel("After");
        let result = LayoutEngine::new(measure_text()).compute(
            replacement.tree(),
            PANEL_WIDTH,
            PANEL_HEIGHT,
            1.0,
        );
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                let replacement = replacement.clone();
                let result = result.clone();
                commands.queue(move |world: &mut World| {
                    world
                        .entity_mut(panel)
                        .remove::<(DiegeticPanel, PanelPrecomposeCache)>();
                    world.entity_mut(panel).insert(replacement);
                    world
                        .get_mut::<ComputedDiegeticPanel>(panel)
                        .expect("reinserted panel has a computed panel")
                        .set_result(result);
                });
            })
            .expect("panel replacement system should run");

        app.update();

        for entity in old_entities {
            assert!(app.world().get_entity(entity).is_err());
        }
        let mut cache = app
            .world_mut()
            .get_mut::<PanelPrecomposeCache>(panel)
            .expect("replacement precompose cache");
        assert_eq!(cache.entries_mut().len(), 1);
        let replacement_rig = cache
            .entry(PRECOMPOSE_ELEMENT_INDEX)
            .expect("replacement precompose entry")
            .rig;
        assert_ne!(replacement_rig, old_entities[0]);
        assert!(app.world().get_entity(replacement_rig).is_ok());
    }

    #[test]
    fn two_precomposing_panels_render_on_disjoint_layers() {
        let mut app = precompose_test_app();
        let first = spawn_precomposing_panel(&mut app, precompose_panel("First"));
        let second = spawn_precomposing_panel(&mut app, precompose_panel("Second"));
        app.update();
        let entries: Vec<_> = [first, second]
            .into_iter()
            .map(|panel| {
                app.world()
                    .get::<PanelPrecomposeCache>(panel)
                    .expect("precompose cache")
                    .entry(PRECOMPOSE_ELEMENT_INDEX)
                    .expect("precompose entry")
            })
            .collect();
        let first_camera = app
            .world()
            .get::<RenderLayers>(entries[0].camera)
            .expect("first camera layers");
        let second_camera = app
            .world()
            .get::<RenderLayers>(entries[1].camera)
            .expect("second camera layers");
        let first_helper = app
            .world()
            .get::<RenderLayers>(entries[0].helper_panel)
            .expect("first helper layers");
        let second_helper = app
            .world()
            .get::<RenderLayers>(entries[1].helper_panel)
            .expect("second helper layers");
        assert_ne!(first_camera, second_camera);
        assert!(!first_camera.intersects(second_helper));
        assert!(!second_camera.intersects(first_helper));
    }

    #[test]
    fn a_stale_precompose_element_despawns_its_whole_rig() {
        let mut app = precompose_test_app();
        let panel = spawn_precomposing_panel(&mut app, precompose_panel("Blend"));
        app.update();
        let entry = app
            .world()
            .get::<PanelPrecomposeCache>(panel)
            .expect("precompose cache")
            .entry(PRECOMPOSE_ELEMENT_INDEX)
            .expect("precompose entry");
        let rig_entities = [entry.rig, entry.helper_panel, entry.camera];
        let updated_panel = DiegeticPanel::world()
            .size(Pt(PANEL_WIDTH), Pt(PANEL_HEIGHT))
            .font_unit(Unit::Points)
            .layout(|builder| {
                builder.text(("direct", TextStyle::new(10.0)));
            })
            .build()
            .expect("valid direct panel");
        let result = LayoutEngine::new(measure_text()).compute(
            updated_panel.tree(),
            PANEL_WIDTH,
            PANEL_HEIGHT,
            1.0,
        );
        app.world_mut()
            .get_mut::<DiegeticPanel>(panel)
            .expect("source panel")
            .replace_tree_full_rebuild(updated_panel.tree().clone());
        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel)
            .expect("computed panel")
            .set_result(result);
        app.update();

        for entity in rig_entities {
            assert!(app.world().get_entity(entity).is_err());
        }
        assert!(
            app.world()
                .get::<PanelPrecomposeCache>(panel)
                .expect("cache")
                .entry(PRECOMPOSE_ELEMENT_INDEX)
                .is_none()
        );
    }
}
