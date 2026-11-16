//! Retained vertical panel-text fit: glyphs travel rigidly during motion and settle at rest.

use std::collections::HashMap;
use std::collections::HashSet;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Added;
use bevy::prelude::Camera;
use bevy::prelude::Camera3d;
use bevy::prelude::Changed;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Mat4;
use bevy::prelude::MessageWriter;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::RemovedComponents;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy::prelude::Vec4;
use bevy::prelude::With;
use bevy::window::RequestRedraw;
use hana_kana::ToUsize;

use super::PreparedPanelText;
use super::TextRunOf;
use crate::ReadableBackFrame;
use crate::render::PackedPathRecord;
use crate::render::PathQuadRecord;
use crate::render::TextFitMode;
use crate::text::GlyphCache;
use crate::text::RunStorageKey;

const FIT_START_PX_PER_EM: f32 = 5.0;
const FIT_END_PX_PER_EM: f32 = 9.0;
const SETTLE_FRAMES: u8 = 4;
const PROJECTION_EPSILON: f32 = 0.000_001;

#[derive(Clone, Copy, Debug, PartialEq)]
struct FitView {
    camera:          Entity,
    clip_from_world: Mat4,
    viewport_min:    Vec2,
    viewport_size:   Vec2,
    scale_factor:    f32,
}

#[derive(Clone, Debug, PartialEq)]
struct LayeredFitView {
    order:         isize,
    view:          FitView,
    render_layers: RenderLayers,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum FitViewState {
    #[default]
    Unavailable,
    Ready(FitView),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FitMotion {
    Resting,
    Moving,
    Settling { frames_left: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GlyphFitTarget {
    FigureCrossbar,
    CapBaseline,
}

#[derive(Debug)]
struct GlyphFitState {
    view:           FitViewState,
    last_transform: Mat4,
    offsets:        Vec<Vec2>,
    target:         Vec<Vec2>,
    motion:         FitMotion,
}

impl GlyphFitState {
    fn settle_one_frame(&mut self) -> bool {
        let FitMotion::Settling { frames_left } = self.motion else {
            return false;
        };
        for (offset, target) in self.offsets.iter_mut().zip(&self.target) {
            *offset += (*target - *offset) / f32::from(frames_left);
        }
        self.motion = if frames_left == 1 {
            FitMotion::Resting
        } else {
            FitMotion::Settling {
                frames_left: frames_left - 1,
            }
        };
        true
    }
}

/// A settled run stays in the map but is never visited until its prepared text,
/// view or transform changes. Only active settling runs request another frame.
#[derive(Default, Resource)]
pub(super) struct SmallTextFitStates {
    views:     Vec<LayeredFitView>,
    runs:      HashMap<Entity, GlyphFitState>,
    back_runs: HashMap<Entity, GlyphFitState>,
}

/// Active camera views with non-empty viewports, ordered from the first drawn
/// camera to the last. A text run selects the first entry whose
/// `RenderLayers` intersect its owner panel.
fn current_views(
    cameras: &Query<(Entity, &Camera, &GlobalTransform, Option<&RenderLayers>), With<Camera3d>>,
) -> Vec<LayeredFitView> {
    let mut views = cameras
        .iter()
        .filter_map(|(entity, camera, transform, render_layers)| {
            if !camera.is_active {
                return None;
            }
            let viewport = camera.physical_viewport_rect()?;
            let size = (viewport.max - viewport.min).as_vec2();
            (size.min_element() > 0.0).then_some(LayeredFitView {
                order:         camera.order,
                view:          FitView {
                    camera:          entity,
                    clip_from_world: camera.clip_from_view() * transform.to_matrix().inverse(),
                    viewport_min:    viewport.min.as_vec2(),
                    viewport_size:   size,
                    scale_factor:    camera.target_scaling_factor().unwrap_or(1.0),
                },
                render_layers: render_layers
                    .cloned()
                    .unwrap_or_else(|| RenderLayers::layer(0)),
            })
        })
        .collect::<Vec<_>>();
    views.sort_by_key(|layered| (layered.order, layered.view.camera.index()));
    views
}

fn view_rendering_panel_layers(
    views: &[LayeredFitView],
    panel_layers: &RenderLayers,
) -> FitViewState {
    views
        .iter()
        .find(|layered| layered.render_layers.intersects(panel_layers))
        .map_or(FitViewState::Unavailable, |layered| {
            FitViewState::Ready(layered.view)
        })
}

fn panel_render_layers(
    panel: Entity,
    render_layers: &Query<Option<&RenderLayers>>,
) -> RenderLayers {
    render_layers
        .get(panel)
        .ok()
        .flatten()
        .cloned()
        .unwrap_or_else(|| RenderLayers::layer(0))
}

fn fitted_offset(
    quad: &PathQuadRecord,
    path: &PackedPathRecord,
    transform: Mat4,
    view: FitView,
    fit_target: GlyphFitTarget,
) -> Vec2 {
    if path.text_em_size <= 0.0 {
        return Vec2::ZERO;
    }
    let bounds_size = Vec2::new(path.bounds_min_size.z, path.bounds_min_size.w);
    let design_span = quad.uv_size * bounds_size;
    if design_span.min_element() <= PROJECTION_EPSILON {
        return Vec2::ZERO;
    }
    let layout_units_per_design = quad.rect_size / design_span;
    let layout_em = path.text_em_size * layout_units_per_design.x.abs();
    let design_bottom =
        (1.0 - quad.uv_min.y - quad.uv_size.y).mul_add(bounds_size.y, path.bounds_min_size.y);
    let baseline_y = design_bottom.mul_add(-layout_units_per_design.y, quad.rect_min.y);
    let feature_x =
        ((0.5 - quad.uv_min.x) * bounds_size.x).mul_add(layout_units_per_design.x, quad.rect_min.x);
    let feature_y = match fit_target {
        GlyphFitTarget::FigureCrossbar => path
            .figure_crossbar_mid
            .mul_add(layout_units_per_design.y, baseline_y),
        GlyphFitTarget::CapBaseline => baseline_y,
    };
    let feature = Vec2::new(feature_x, feature_y);
    let clip = view.clip_from_world * transform * Vec4::new(feature.x, feature.y, 0.0, 1.0);
    if clip.w <= PROJECTION_EPSILON {
        return Vec2::ZERO;
    }
    let dx = view.clip_from_world * transform * Vec4::new(1.0, 0.0, 0.0, 0.0);
    let dy = view.clip_from_world * transform * Vec4::new(0.0, 1.0, 0.0, 0.0);
    let px_per_unit_x = 0.5 * (dx.x - clip.x * dx.w / clip.w) / clip.w * view.viewport_size.x;
    let px_per_unit_y = -0.5 * (dy.y - clip.y * dy.w / clip.w) / clip.w * view.viewport_size.y;
    if px_per_unit_x.abs().min(px_per_unit_y.abs()) <= PROJECTION_EPSILON {
        return Vec2::ZERO;
    }
    let row = (0.5 - 0.5 * clip.y / clip.w).mul_add(view.viewport_size.y, view.viewport_min.y);
    match fit_target {
        GlyphFitTarget::FigureCrossbar => {},
        GlyphFitTarget::CapBaseline => {
            // The baseline is the lower edge of the cap-to-baseline span. Settling
            // it beyond the next pixel-centre boundary puts the contour on a whole
            // physical row at every projected text size, without a size-specific
            // layout nudge.
            let whole_row = (row + 0.5).ceil();
            return Vec2::new(0.0, (whole_row - row) / px_per_unit_y);
        },
    }
    let pixels_per_em = layout_em * px_per_unit_x.abs().min(px_per_unit_y.abs());
    if !(FIT_START_PX_PER_EM..FIT_END_PX_PER_EM).contains(&pixels_per_em) {
        return Vec2::ZERO;
    }
    Vec2::new(0.0, (0.5 - row.rem_euclid(1.0)) / px_per_unit_y)
}

fn fitted_offsets(
    backend: &GlyphCache,
    quads: &[PathQuadRecord],
    transform: Mat4,
    view: FitView,
    fit_target: GlyphFitTarget,
) -> Vec<Vec2> {
    quads
        .iter()
        .map(|quad| {
            backend
                .packed_path_records()
                .get(quad.packed_path_index.to_usize())
                .map_or(Vec2::ZERO, |path| {
                    fitted_offset(quad, path, transform, view, fit_target)
                })
        })
        .collect()
}

fn fitted_offsets_for_view(
    backend: &GlyphCache,
    quads: &[PathQuadRecord],
    transform: Mat4,
    view: FitViewState,
    fit_target: GlyphFitTarget,
) -> Vec<Vec2> {
    match view {
        FitViewState::Unavailable => vec![Vec2::ZERO; quads.len()],
        FitViewState::Ready(view) => fitted_offsets(backend, quads, transform, view, fit_target),
    }
}

/// Holds local fit during camera and panel motion, then eases to the newly
/// projected position over four drawn frames. Idle frames touch no text run.
pub(super) fn settle_small_text_fit(
    cameras: Query<(Entity, &Camera, &GlobalTransform, Option<&RenderLayers>), With<Camera3d>>,
    labels: Query<(Entity, Ref<PreparedPanelText>, &TextRunOf)>,
    changed_labels: Query<
        Entity,
        (
            With<PreparedPanelText>,
            // A same-count reshape replaces fitted quads even at rest.
            Or<(Changed<PreparedPanelText>, Changed<GlobalTransform>)>,
        ),
    >,
    panel_layers: Query<Option<&RenderLayers>>,
    changed_panel_layers: Query<
        Entity,
        Or<(
            Added<RenderLayers>,
            Changed<RenderLayers>,
            Changed<ReadableBackFrame>,
        )>,
    >,
    frames: Query<Ref<GlobalTransform>>,
    back_panels: Query<(Entity, &ReadableBackFrame)>,
    mut removed_panel_layers: RemovedComponents<RenderLayers>,
    mut removed_labels: RemovedComponents<PreparedPanelText>,
    mut fit: ResMut<SmallTextFitStates>,
    mut backend: ResMut<GlyphCache>,
    mut redraw: Option<MessageWriter<RequestRedraw>>,
) {
    for entity in removed_labels.read() {
        fit.runs.remove(&entity);
        fit.back_runs.remove(&entity);
    }
    let current_views = current_views(&cameras);
    let views_changed = fit.views != current_views;
    fit.views = current_views;
    let mut active: HashSet<Entity> = changed_labels.iter().collect();
    if views_changed {
        active.extend(labels.iter().map(|(entity, ..)| entity));
    } else {
        active.extend(
            fit.runs
                .iter()
                .chain(&fit.back_runs)
                .filter_map(|(entity, state)| {
                    (state.motion != FitMotion::Resting).then_some(*entity)
                }),
        );
    }
    activate_moved_back_panels(&mut active, &labels, &back_panels, &frames);
    let mut panels_with_changed_layers: HashSet<Entity> = changed_panel_layers.iter().collect();
    panels_with_changed_layers.extend(removed_panel_layers.read());
    if !panels_with_changed_layers.is_empty() {
        active.extend(labels.iter().filter_map(|(entity, _, run_of)| {
            panels_with_changed_layers
                .contains(&run_of.panel())
                .then_some(entity)
        }));
    }
    if active.is_empty() {
        return;
    }
    for entity in active {
        let Ok((_, prepared, run_of)) = labels.get(entity) else {
            fit.runs.remove(&entity);
            fit.back_runs.remove(&entity);
            continue;
        };
        let panel_layers = panel_render_layers(run_of.panel(), &panel_layers);
        let view = view_rendering_panel_layers(&fit.views, &panel_layers);
        let front = RunStorageKey::from(entity);
        let SmallTextFitStates {
            runs, back_runs, ..
        } = &mut *fit;
        for (run, states) in [(front, runs), (front.back(), back_runs)] {
            update_run_fit(
                run,
                entity,
                &prepared,
                view,
                &mut backend,
                states,
                &mut redraw,
            );
        }
    }
}

/// Activates text only when its readable back's stock frame moves.
fn activate_moved_back_panels(
    active: &mut HashSet<Entity>,
    labels: &Query<(Entity, Ref<PreparedPanelText>, &TextRunOf)>,
    back_panels: &Query<(Entity, &ReadableBackFrame)>,
    frames: &Query<Ref<GlobalTransform>>,
) {
    if back_panels.is_empty() {
        return;
    }
    let moved_back_panels: HashSet<_> = back_panels
        .iter()
        .filter(|(_, frame)| frames.get(frame.0).is_ok_and(|pose| pose.is_changed()))
        .map(|(panel, _)| panel)
        .collect();
    if !moved_back_panels.is_empty() {
        active.extend(labels.iter().filter_map(|(entity, _, owner)| {
            moved_back_panels.contains(&owner.panel()).then_some(entity)
        }));
    }
}

fn update_run_fit(
    run: RunStorageKey,
    entity: Entity,
    prepared: &Ref<'_, PreparedPanelText>,
    view: FitViewState,
    backend: &mut GlyphCache,
    states: &mut HashMap<Entity, GlyphFitState>,
    redraw: &mut Option<MessageWriter<RequestRedraw>>,
) {
    let Some(transform) = backend.batch_store().run_transform(run) else {
        states.remove(&entity);
        return;
    };
    let Some(quads) = backend.batch_store().run_path_records(run) else {
        states.remove(&entity);
        return;
    };
    let fit_target = if prepared.style_gate.aligns_cap_baseline_to_pixels() {
        GlyphFitTarget::CapBaseline
    } else {
        GlyphFitTarget::FigureCrossbar
    };
    let text_fit_mode = match fit_target {
        GlyphFitTarget::FigureCrossbar => TextFitMode::SmallGlyph,
        GlyphFitTarget::CapBaseline => TextFitMode::CapBaseline,
    };
    let needs_new_state = states.get(&entity).is_none_or(|state| {
        state.offsets.len() != quads.len() || (prepared.is_changed() && !prepared.render_only)
    });
    if needs_new_state {
        let offsets = fitted_offsets_for_view(backend, quads, transform, view, fit_target);
        backend
            .batch_store_mut()
            .update_run_text_fit(run, &offsets, text_fit_mode);
        states.insert(
            entity,
            GlyphFitState {
                view,
                last_transform: transform,
                target: offsets.clone(),
                offsets,
                motion: FitMotion::Resting,
            },
        );
        return;
    }
    let Some(state) = states.get_mut(&entity) else {
        return;
    };
    if view != state.view || transform != state.last_transform {
        state.view = view;
        state.last_transform = transform;
        state.motion = FitMotion::Moving;
        return;
    }
    if state.motion == FitMotion::Moving {
        state.target = fitted_offsets_for_view(backend, quads, transform, view, fit_target);
        state.motion = FitMotion::Settling {
            frames_left: SETTLE_FRAMES,
        };
    }
    if state.settle_one_frame() {
        backend
            .batch_store_mut()
            .update_run_text_fit(run, &state.offsets, text_fit_mode);
        if state.motion != FitMotion::Resting
            && let Some(redraw) = redraw.as_mut()
        {
            redraw.write(RequestRedraw);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::asset::AssetPlugin;
    use bevy::camera::Viewport;
    use bevy::camera::visibility::RenderLayers;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::AssetApp;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::Mesh;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Transform;
    use bevy::prelude::TransformPlugin;
    use bevy::prelude::TransformSystems;
    use bevy::prelude::UVec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::World;
    use bevy::render::storage::ShaderBuffer;

    use super::*;
    use crate::Anchor;
    use crate::GlyphShadowMode;
    use crate::cascade;
    use crate::cascade::TextAlpha;
    use crate::cascade::TextMaterial;
    use crate::layout::Lighting;
    use crate::layout::Sidedness;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::render;
    use crate::render::AntiAlias;
    use crate::render::PathExtendedMaterial;
    use crate::render::PathRenderRecord;
    use crate::render::display_compensation::DisplayCompensationPlugin;
    use crate::render::material_table::BatchAssetWrites;
    use crate::render::material_table::MaterialTableAppendReady;
    use crate::render::material_table::MaterialTablePlugin;
    use crate::render::panel_text::PanelTextCascadeBridgePlugin;
    use crate::render::panel_text::batching;
    use crate::render::panel_text::batching::TextBatchInvalidation;
    use crate::render::panel_text::batching::TextBatchRouteCounts;
    use crate::render::panel_text::reify;
    use crate::render::panel_text::shaping;
    use crate::render::text_shaping::TextShapingContext;
    use crate::text::DiegeticTextMeasurer;
    use crate::text::FontRegistry;
    use crate::widgets::VisualOverrideIndex;

    fn fitting_app() -> Result<App, String> {
        let registry = FontRegistry::new().map_err(|error| format!("embedded font: {error}"))?;
        let measure_fn =
            crate::text::create_parley_measurer(registry.font_context(), registry.family_names());
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .add_plugins(DisplayCompensationPlugin)
            .add_plugins(MaterialTablePlugin)
            .add_plugins(TransformPlugin)
            .insert_resource(DiegeticTextMeasurer { measure_fn })
            .add_plugins(HeadlessLayoutPlugin)
            .add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .add_plugins(cascade::cascade_plugin::<TextMaterial>())
            .add_plugins(cascade::cascade_plugin::<Lighting>())
            .add_plugins(cascade::cascade_plugin::<Sidedness>())
            .add_plugins(cascade::cascade_plugin::<GlyphShadowMode>())
            .insert_resource(registry)
            .init_resource::<TextShapingContext>()
            .init_resource::<GlyphCache>()
            .init_resource::<TextBatchInvalidation>()
            .init_resource::<AntiAlias>()
            .init_resource::<VisualOverrideIndex>()
            .init_resource::<TextBatchRouteCounts>()
            .init_resource::<SmallTextFitStates>()
            .init_asset::<Mesh>()
            .init_asset::<ShaderBuffer>()
            .init_asset::<PathExtendedMaterial>()
            .add_plugins(PanelTextCascadeBridgePlugin)
            .add_systems(
                PostUpdate,
                (
                    reify::reify_text_entities,
                    shaping::shape_panel_text_children.after(reify::reify_text_entities),
                    batching::reset_text_batch_route_counts
                        .before(batching::update_panel_text_batches),
                    batching::update_panel_text_batches
                        .run_if(batching::panel_text_batch_inputs_changed)
                        .after(shaping::shape_panel_text_children)
                        .after(MaterialTableAppendReady)
                        .in_set(BatchAssetWrites)
                        .before(TransformSystems::Propagate),
                    batching::write_batch_run_transforms.after(TransformSystems::Propagate),
                    settle_small_text_fit
                        .after(batching::write_batch_run_transforms)
                        .before(batching::update_batch_bounds)
                        .before(batching::commit_batch_buffers),
                    batching::update_batch_bounds.after(batching::write_batch_run_transforms),
                    batching::commit_batch_buffers
                        .after(batching::update_panel_text_batches)
                        .after(batching::write_batch_run_transforms),
                ),
            );
        render::seed_default_material_cascades(&mut app);
        let mut camera = Camera {
            viewport: Some(Viewport {
                physical_size: UVec2::new(128, 96),
                ..Viewport::default()
            }),
            ..Camera::default()
        };
        camera.computed.clip_from_view =
            bevy::math::proj::orthographic(-0.064, 0.064, -0.048, 0.048, 0.1, 100.0);
        app.world_mut().spawn((
            Camera3d::default(),
            camera,
            Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
            RenderLayers::from_layers(&[0, 1]),
        ));
        Ok(app)
    }

    fn spawn_fit_panel(app: &mut App, layer: usize, text: &str) -> Result<Entity, String> {
        let mut builder = crate::LayoutBuilder::with_root(
            crate::El::new()
                .size(72.0, 48.0)
                .padding(crate::Padding::all(8.0)),
        );
        builder.text(
            crate::Text::new(
                text,
                crate::TextStyle::new(crate::Mm(8.0)).with_shadow_mode(GlyphShadowMode::None),
            )
            .id("fit-label"),
        );
        Ok(app
            .world_mut()
            .spawn((
                crate::DiegeticPanel::world()
                    .size(crate::Mm(72.0), crate::Mm(48.0))
                    .anchor(Anchor::Center)
                    .with_tree(builder.build())
                    .build()
                    .map_err(|error| format!("fit panel: {error}"))?,
                RenderLayers::layer(layer),
            ))
            .id())
    }

    #[test]
    fn equal_count_text_edit_matches_fresh_fit_on_first_frame() -> Result<(), String> {
        let mut app = fitting_app()?;
        let panels = [
            spawn_fit_panel(&mut app, 0, "AAAAA")?,
            spawn_fit_panel(&mut app, 1, "BBBBB")?,
        ];
        for _ in 0..6 {
            app.update();
        }
        let runs = panels
            .iter()
            .map(|panel| {
                app.world_mut()
                    .query::<(Entity, &TextRunOf)>()
                    .iter(app.world())
                    .find(|(_, owner)| owner.panel() == *panel)
                    .map(|(run, _)| run)
                    .ok_or("fit run missing")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fit_before = app.world().resource::<SmallTextFitStates>().runs[&runs[0]]
            .offsets
            .clone();
        assert!(fit_before.iter().any(|offset| *offset != Vec2::ZERO));
        let retained = panels[0];
        assert!(
            app.world_mut()
                .run_system_once(move |mut text: crate::PanelText| {
                    text.set_text(
                        retained,
                        &crate::PanelElementId::named("fit-label"),
                        "BBBBB",
                    )
                })
                .map_err(|error| format!("equal-count text setter: {error}"))?
        );
        app.update();

        let fit = app.world().resource::<SmallTextFitStates>();
        assert_eq!(fit.runs[&runs[0]].offsets, fit.runs[&runs[1]].offsets);
        let store = app.world().resource::<GlyphCache>().batch_store();
        let retained = store
            .run_path_records(RunStorageKey::from(runs[0]))
            .ok_or("changed run records missing")?;
        let fresh = store
            .run_path_records(RunStorageKey::from(runs[1]))
            .ok_or("fresh run records missing")?;
        assert_eq!(
            retained, fresh,
            "first-frame drawn records; retained state {:?}, original offsets {fit_before:?}",
            fit.runs[&runs[0]],
        );
        Ok(())
    }

    #[test]
    fn reflected_small_glyphs_retain_the_front_vertical_fit() -> Result<(), String> {
        let mut app = fitting_app()?;
        let panel = spawn_fit_panel(&mut app, 0, "27.4 ms")?;
        for _ in 0..6 {
            app.update();
        }
        app.world_mut()
            .run_system_once(move |mut text: crate::PanelText| {
                text.set_style(
                    panel,
                    &crate::PanelElementId::named("fit-label"),
                    crate::TextStyle::new(crate::Mm(8.0))
                        .with_shadow_mode(GlyphShadowMode::None)
                        .with_readable_back(),
                )
            })
            .map_err(|error| format!("back style: {error}"))?;
        for _ in 0..6 {
            app.update();
        }
        let run = app
            .world_mut()
            .query::<(Entity, &TextRunOf)>()
            .iter(app.world())
            .find(|(_, owner)| owner.panel() == panel)
            .map(|(entity, _)| entity)
            .ok_or("reflected run")?;
        let backend = app.world().resource::<GlyphCache>();
        let records = backend
            .batch_store()
            .run_path_records(RunStorageKey::from(run))
            .ok_or("reflected glyphs")?;
        let front = records;
        let back = backend
            .batch_store()
            .run_path_records(RunStorageKey::from(run).back())
            .ok_or("back glyph records")?;
        assert_ne!(front, []);
        assert_eq!(front.len(), back.len());
        assert!(
            front
                .iter()
                .any(|glyph| glyph.text_fit_offset != Vec2::ZERO)
        );
        for (front, back) in front.iter().zip(back) {
            assert!(front.rect_size.x > 0.0 && back.rect_size.x < 0.0);
            assert_eq!(front.packed_path_index, back.packed_path_index);
            assert_eq!(front.uv_min, back.uv_min);
            assert_eq!(front.uv_size, back.uv_size);
            assert_eq!(front.text_fit_offset, back.text_fit_offset);
        }
        Ok(())
    }

    #[test]
    fn translated_and_rotated_back_words_keep_the_shared_panel_depth() -> Result<(), String> {
        let mut app = fitting_app()?;
        let panel = spawn_fit_panel(&mut app, 0, "Log")?;
        app.world_mut().entity_mut(panel).insert(
            Transform::from_xyz(1.7, 0.5, -0.9).with_rotation(bevy::prelude::Quat::from_euler(
                bevy::prelude::EulerRot::XYZ,
                0.14,
                0.38,
                0.05,
            )),
        );
        for _ in 0..6 {
            app.update();
        }
        app.world_mut()
            .run_system_once(move |mut text: crate::PanelText| {
                text.set_style(
                    panel,
                    &crate::PanelElementId::named("fit-label"),
                    crate::TextStyle::new(crate::Mm(8.0)).with_readable_back(),
                );
            })
            .map_err(|error| format!("back style: {error}"))?;
        for _ in 0..6 {
            app.update();
        }
        let backend = app.world().resource::<GlyphCache>();
        let mut records = 0;
        for (_, batch) in backend.batch_store().batches() {
            let faces = batch.run_records();
            assert_eq!(faces.len(), 2);
            assert_eq!(
                faces[0].panel_plane, faces[1].panel_plane,
                "coplanar text must retain the exact shared depth for OIT ordering"
            );
            records += faces.len();
        }
        assert_eq!(records, 2);
        Ok(())
    }

    /// The front and back records of the one readable-back run on `panel`.
    fn front_and_back_records(
        app: &mut App,
        panel: Entity,
    ) -> Result<(PathRenderRecord, PathRenderRecord), String> {
        let run = app
            .world_mut()
            .query::<(Entity, &TextRunOf)>()
            .iter(app.world())
            .find(|(_, owner)| owner.panel() == panel)
            .map(|(entity, _)| entity)
            .ok_or("readable-back run")?;
        let front_matrix = app
            .world()
            .get::<GlobalTransform>(run)
            .ok_or("run transform")?
            .to_matrix();
        let records: Vec<_> = app
            .world()
            .resource::<GlyphCache>()
            .batch_store()
            .batches()
            .flat_map(|(_, batch)| batch.run_records().to_vec())
            .collect();
        let [first, second] = records[..] else {
            return Err(format!(
                "expected a front and a back record: {}",
                records.len()
            ));
        };
        if first.transform == front_matrix {
            Ok((first, second))
        } else if second.transform == front_matrix {
            Ok((second, first))
        } else {
            Err("no record has the front run transform".to_owned())
        }
    }

    #[test]
    fn readable_back_frame_mirrors_the_back_and_follows_a_frame_only_move() -> Result<(), String> {
        let mut app = fitting_app()?;
        let panel = spawn_fit_panel(&mut app, 0, "Log")?;
        let frame = app
            .world_mut()
            .spawn(Transform::from_xyz(0.3, 0.0, -0.2))
            .id();
        app.world_mut()
            .entity_mut(panel)
            .insert(ReadableBackFrame(frame));
        for _ in 0..6 {
            app.update();
        }
        app.world_mut()
            .run_system_once(move |mut text: crate::PanelText| {
                text.set_style(
                    panel,
                    &crate::PanelElementId::named("fit-label"),
                    crate::TextStyle::new(crate::Mm(8.0)).with_readable_back(),
                );
            })
            .map_err(|error| format!("back style: {error}"))?;
        for _ in 0..6 {
            app.update();
        }
        let (front, back) = front_and_back_records(&mut app, panel)?;
        let frame_origin = Vec3::new(0.3, 0.0, -0.2);
        // The frame's vertical axis is the mirror: depth reflects through the
        // frame origin, and height does not change.
        let tolerance = 1.0e-5;
        assert!(
            (back.transform.w_axis.z - 2.0f32.mul_add(frame_origin.z, -front.transform.w_axis.z))
                .abs()
                < tolerance,
            "back depth mirrors through the frame: front={:?}, back={:?}",
            front.transform.w_axis,
            back.transform.w_axis
        );
        assert!((back.transform.w_axis.y - front.transform.w_axis.y).abs() < tolerance);
        let normal = front.panel_plane.truncate();
        assert!(
            (back.panel_plane.w - 2.0f32.mul_add(-normal.dot(frame_origin), -front.panel_plane.w))
                .abs()
                < tolerance,
            "back plane reflects through the frame: front={:?}, back={:?}",
            front.panel_plane,
            back.panel_plane
        );

        let step = Vec3::new(0.25, 0.0, 0.1);
        app.world_mut()
            .entity_mut(frame)
            .insert(Transform::from_translation(frame_origin + step));
        for _ in 0..6 {
            app.update();
        }
        let (moved_front, moved_back) = front_and_back_records(&mut app, panel)?;
        assert_eq!(moved_front.transform, front.transform, "the front stays");
        assert_eq!(
            moved_front.panel_plane, front.panel_plane,
            "the front plane stays"
        );
        let shift = moved_back.transform.w_axis.truncate() - back.transform.w_axis.truncate();
        assert!(
            (shift - 2.0 * step).length() < tolerance,
            "the back moves twice the frame step: shift={shift:?}, step={step:?}"
        );
        assert!(
            (moved_back.panel_plane.w - 2.0f32.mul_add(-normal.dot(step), back.panel_plane.w))
                .abs()
                < tolerance,
            "the back plane follows the frame: before={:?}, after={:?}",
            back.panel_plane,
            moved_back.panel_plane
        );
        Ok(())
    }

    #[test]
    fn screen_panel_text_fit_uses_the_camera_that_renders_its_layer() {
        let mut world = World::new();
        let viewport = Some(Viewport {
            physical_size: UVec2::new(800, 600),
            ..Viewport::default()
        });
        let panel_layers = RenderLayers::layer(31);
        let panel = world.spawn(panel_layers.clone()).id();
        let text_run = world.spawn(TextRunOf(panel)).id();
        let screen_camera = world
            .spawn((
                Camera3d::default(),
                Camera {
                    order: 110,
                    viewport: viewport.clone(),
                    ..Camera::default()
                },
                GlobalTransform::default(),
                panel_layers,
            ))
            .id();
        world.spawn((
            Camera3d::default(),
            Camera {
                order: 0,
                viewport,
                ..Camera::default()
            },
            GlobalTransform::default(),
            RenderLayers::layer(0),
        ));

        let view = world.run_system_once(
            move |cameras: Query<
                (Entity, &Camera, &GlobalTransform, Option<&RenderLayers>),
                With<Camera3d>,
            >,
                  text_runs: Query<&TextRunOf>,
                  panel_layers: Query<Option<&RenderLayers>>| {
                let Ok(run_of) = text_runs.get(text_run) else {
                    return FitViewState::Unavailable;
                };
                let panel_layers = panel_render_layers(run_of.panel(), &panel_layers);
                view_rendering_panel_layers(&current_views(&cameras), &panel_layers)
            },
        );
        assert!(
            matches!(view, Ok(FitViewState::Ready(view)) if view.camera == screen_camera),
            "screen-panel text should use the layer-matching camera, got {view:?}"
        );
    }

    #[test]
    fn glyph_fit_reaches_target_in_four_monotone_frames_then_rests() {
        let mut state = GlyphFitState {
            view:           FitViewState::Ready(FitView {
                camera:          Entity::PLACEHOLDER,
                clip_from_world: Mat4::IDENTITY,
                viewport_min:    Vec2::ZERO,
                viewport_size:   Vec2::ONE,
                scale_factor:    1.0,
            }),
            last_transform: Mat4::IDENTITY,
            offsets:        vec![Vec2::ZERO],
            target:         vec![Vec2::new(0.4, -0.3)],
            motion:         FitMotion::Settling { frames_left: 4 },
        };
        for frame in 1_u16..=4 {
            assert!(state.settle_one_frame());
            let progress = f32::from(frame) / 4.0;
            assert!((state.offsets[0] - state.target[0] * progress).length() < 0.000_01);
        }
        assert_eq!(state.motion, FitMotion::Resting);
        assert!(!state.settle_one_frame());
    }
}
