//! [`compute_panel_layouts`] — recomputes layout for changed panels, layout
//! trees, and resolved font units, plus [`resolve_world_panel_fit`] — shrinks
//! `Fit`-axis world panels to their content bounds after layout runs.

use std::sync::Arc;
use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::prelude::Commands;
use bevy::prelude::DetectChanges;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::Mut;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::World;

use super::constants::PANEL_RESIZE_EPSILON;
use super::coordinate_space::CoordinateSpace;
use super::diegetic_panel::ComputedDiegeticPanel;
use super::diegetic_panel::DiegeticPanel;
use super::diegetic_panel::DiegeticPanelChangeClassification;
use super::diegetic_panel::ScaledLayoutTreeCache;
use super::events;
use super::events::LastPanelDimensions;
use super::events::PanelChangeKind;
use super::field;
use super::perf::DiegeticPerfStats;
use super::scroll;
use super::scroll_view::PanelScrollViews;
use crate::FontRegistry;
use crate::cascade::FontUnit;
use crate::cascade::PanelDefaults;
use crate::cascade::Resolved;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::constants::MILLISECONDS_PER_SECOND;
use crate::layout::BoundingBox;
use crate::layout::CapHeightMeasurer;
use crate::layout::LayoutEngine;
use crate::layout::LayoutResult;
use crate::layout::LayoutTree;
use crate::layout::LayoutTreeChange;
use crate::layout::MeasureTextFn;
use crate::layout::RichMeasurement;
use crate::layout::ScrollSolveInput;
use crate::layout::ShapedTextCache;
use crate::layout::Sizing;
use crate::layout::TextMeasure;
use crate::text;
use crate::text::DiegeticTextMeasurer;
use crate::text::RichTextMeasurer;

/// Keeps the cap measurer's shaping buffers and style cache across frames.
#[derive(SystemParam)]
pub(super) struct PanelTextMeasurers<'w, 's> {
    plain: Res<'w, DiegeticTextMeasurer>,
    rich:  Option<Res<'w, RichTextMeasurer>>,
    fonts: Option<Res<'w, FontRegistry>>,
    caps:  Local<'s, CapHeightMeasurer>,
}

/// Marks whether IME input queued a tree edit after the regular layout pass.
#[derive(Resource, Default)]
pub(crate) struct LateEditorLayoutNeeded(bool);

pub(super) fn mark_late_editor_layout(
    mut needed: ResMut<LateEditorLayoutNeeded>,
    panels: Query<(
        &DiegeticPanelChangeClassification,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
    )>,
) {
    needed.0 = panels.iter().any(|(change, panel, computed)| {
        change.pending().is_some()
            && (panel.tree().has_caret_follow_field()
                || computed.layout().solved().is_some_and(|solved| {
                    computed.field_records().iter().any(|field| {
                        solved
                            .scrolls
                            .iter()
                            .any(|scroll| scroll.element_index == field.element_index)
                    })
                }))
    });
}

pub(crate) fn late_editor_layout_needed(needed: Res<LateEditorLayoutNeeded>) -> bool { needed.0 }

/// Applies tree replacements queued by observers during the input flush.
pub(super) fn flush_editor_tree_replacements(world: &mut World) { world.flush(); }

/// Recomputes layout after a panel, its layout tree, or its resolved font unit
/// changes.
///
/// Uses the [`ShapedTextCache`] for measurement: if a text string has already
/// been shaped (by a previous layout or render pass), its dimensions are
/// returned from the cache without calling parley. On cache miss, falls back
/// to the parley-backed [`DiegeticTextMeasurer`].
pub(super) fn compute_panel_layouts(
    mut panels: Query<(
        Entity,
        Ref<DiegeticPanel>,
        &mut DiegeticPanelChangeClassification,
        &mut ScaledLayoutTreeCache,
        Option<&mut PanelScrollViews>,
    )>,
    mut computed_panels: Query<&mut ComputedDiegeticPanel>,
    mut commands: Commands,
    panel_font_units: Query<Ref<Resolved<FontUnit>>>,
    mut measurers: PanelTextMeasurers,
    cache: Res<ShapedTextCache>,
    mut perf: ResMut<DiegeticPerfStats>,
    defaults: Res<PanelDefaults>,
    mut trace_init: Local<bool>,
    mut trace_remaining: Local<u32>,
) {
    let start = Instant::now();
    let mut panel_count = 0_usize;
    if !*trace_init {
        *trace_init = true;
        *trace_remaining = 30;
    }

    if let Some(fonts) = measurers.fonts.as_ref()
        && (fonts.is_changed() || matches!(*measurers.caps, CapHeightMeasurer::LineBoxFallback))
    {
        *measurers.caps = CapHeightMeasurer::FontMetrics(text::create_cap_height_measurer(
            fonts.font_context(),
            fonts.family_names(),
        ));
    }
    let cached_measure = build_cached_measure(&cache, &measurers.plain);
    let rich_measure = PanelRichMeasurement {
        rich: measurers
            .rich
            .as_deref()
            .map_or(RichMeasurement::PlainTextFallback, |measurer| {
                RichMeasurement::ShapedParagraph(Arc::clone(&measurer.measure_fn))
            }),
        caps: measurers.caps.clone(),
    };

    for (entity, panel_ref, mut tree_change, mut scaled_tree_cache, mut views) in &mut panels {
        let views_changed = views.as_ref().is_some_and(DetectChanges::is_changed);
        let panel_changed = panel_ref.is_changed();
        let pending_some = tree_change.pending().is_some();
        let resolved_font_unit = panel_font_units.get(entity).ok();
        let font_unit_changed = resolved_font_unit
            .as_ref()
            .is_some_and(DetectChanges::is_changed);
        if !panel_changed && !pending_some && !font_unit_changed && !views_changed {
            continue;
        }
        trace_panel_change(&mut trace_remaining, entity, panel_changed, pending_some);
        let Ok(mut computed) = computed_panels.get_mut(entity) else {
            continue;
        };

        let font_unit =
            resolved_font_unit.map_or(defaults.panel_font_unit, |resolved| resolved.0.0);
        let refreshed = refresh_panel_layout(
            entity,
            &panel_ref,
            &mut tree_change,
            &mut scaled_tree_cache,
            &mut computed,
            views.as_deref(),
            font_unit.to_points(),
            font_unit_changed,
            views_changed,
            &cached_measure,
            &rich_measure,
            &mut commands,
        );
        if refreshed {
            if let Some(views) = &mut views {
                // Reconciliation describes the result just committed, so it
                // must not schedule another identical solve next frame.
                views
                    .bypass_change_detection()
                    .reconcile(&computed, panel_ref.tree());
            }
            panel_count += 1;
        }
    }

    record_layout_perf(&mut perf, panel_count, start);
}

fn refresh_panel_layout(
    entity: Entity,
    panel: &DiegeticPanel,
    tree_change: &mut DiegeticPanelChangeClassification,
    scaled_tree_cache: &mut Mut<'_, ScaledLayoutTreeCache>,
    computed: &mut Mut<'_, ComputedDiegeticPanel>,
    views: Option<&PanelScrollViews>,
    font_to_points: f32,
    font_unit_changed: bool,
    views_changed: bool,
    cached_measure: &MeasureTextFn,
    rich_measure: &PanelRichMeasurement,
    commands: &mut Commands,
) -> bool {
    let layout_to_points = panel.layout_unit().to_points();
    let viewport_width = panel.width() * layout_to_points;
    let viewport_height = panel.height() * layout_to_points;

    let (pending_change, tree_visual_geometry_stable) =
        tree_change.take_with_tree_visual_geometry_stable();
    // An identical replacement skips the solve only while the last solve
    // still matches the panel. A screen panel with dynamic axes solves on
    // its spawn frame before those axes resolve against the window, so an
    // identical tree sent on the next frame must not retain that solve.
    if !views_changed
        && can_skip_identical_layout(
            pending_change,
            computed,
            font_unit_changed,
            viewport_width,
            viewport_height,
        )
    {
        return false;
    }
    let had_result = computed.is_solved();

    let scaled_tree =
        scaled_tree_cache.get_or_update(panel.tree_source(), layout_to_points, font_to_points);

    // Geometry-stable skip: regenerate render commands from cached positions
    // without re-running the layout solve. Safe only when
    // `Resolved<FontUnit>` is unchanged and a text-only edit (`VisualOnly`)
    // leaves every leaf's box unchanged — verified by `can_reuse_geometry`,
    // which re-measures the leaves and rejects anything the reuse would
    // render wrong.
    let can_reuse_geometry = !views_changed
        && !font_unit_changed
        && can_reuse_visual_geometry(
            tree_visual_geometry_stable,
            computed,
            scaled_tree,
            cached_measure,
            viewport_width,
            viewport_height,
        );
    if matches!(pending_change, Some(LayoutTreeChange::VisualOnly))
        && can_reuse_geometry
        && computed.regenerate_commands(scaled_tree, cached_measure, layout_to_points)
    {
        events::trigger_panel_changed(commands, entity, PanelChangeKind::VisualOnly);
        return true;
    }

    let result = solve_panel_with_caret_history(
        scaled_tree,
        computed,
        cached_measure,
        rich_measure,
        viewport_width,
        viewport_height,
        views,
        layout_to_points,
    );

    commit_layout_result(computed, panel, scaled_tree, result, entity);
    let layout_change = (font_unit_changed || views_changed)
        .then_some(LayoutTreeChange::LayoutAffecting)
        .or(pending_change);
    events::trigger_panel_changed(
        commands,
        entity,
        PanelChangeKind::from_layout_change(layout_change, had_result),
    );
    true
}

fn trace_panel_change(
    trace_remaining: &mut u32,
    entity: Entity,
    panel_changed: bool,
    pending_some: bool,
) {
    if *trace_remaining > 0 {
        *trace_remaining -= 1;
        bevy::log::debug!(
            target: "compute_panel_layouts_trace",
            "entity={entity:?} panel_changed={panel_changed} pending={pending_some} budget_left={}",
            *trace_remaining,
        );
    }
}

fn can_skip_identical_layout(
    pending_change: Option<LayoutTreeChange>,
    computed: &ComputedDiegeticPanel,
    font_unit_changed: bool,
    viewport_width: f32,
    viewport_height: f32,
) -> bool {
    matches!(pending_change, Some(LayoutTreeChange::Identical))
        && !font_unit_changed
        && computed
            .layout()
            .solved()
            .is_some_and(|result| result.solved_for_viewport(viewport_width, viewport_height))
}

fn can_reuse_visual_geometry(
    tree_visual_geometry_stable: bool,
    computed: &ComputedDiegeticPanel,
    scaled_tree: &LayoutTree,
    cached_measure: &MeasureTextFn,
    viewport_width: f32,
    viewport_height: f32,
) -> bool {
    tree_visual_geometry_stable
        || computed.layout().solved().is_some_and(|result| {
            result.can_reuse_geometry(
                scaled_tree,
                cached_measure,
                viewport_width,
                viewport_height,
                1.0,
            )
        })
}

struct PanelRichMeasurement {
    rich: RichMeasurement,
    caps: CapHeightMeasurer,
}

/// Solves a panel with the applied field offsets from its previous layout.
fn solve_panel_with_caret_history(
    scaled_tree: &LayoutTree,
    computed: &ComputedDiegeticPanel,
    cached_measure: &MeasureTextFn,
    rich_measure: &PanelRichMeasurement,
    viewport_width: f32,
    viewport_height: f32,
    views: Option<&PanelScrollViews>,
    layout_to_points: f32,
) -> LayoutResult {
    let mut previous_offsets = previous_caret_offsets(computed);
    if let Some(views) = views {
        views.solve_input(layout_to_points, scaled_tree, &mut previous_offsets);
    }
    let engine = match &rich_measure.rich {
        _ if !scaled_tree.contains_rich_text() => LayoutEngine::new(Arc::clone(cached_measure)),
        RichMeasurement::PlainTextFallback => LayoutEngine::new(Arc::clone(cached_measure)),
        RichMeasurement::ShapedParagraph(rich) => {
            LayoutEngine::with_rich_measure(Arc::clone(cached_measure), Arc::clone(rich))
        },
    }
    .with_cap_height_measure(rich_measure.caps.clone());
    if previous_offsets.is_empty() {
        engine.compute(scaled_tree, viewport_width, viewport_height, 1.0)
    } else {
        engine.compute_with_caret_offsets(
            scaled_tree,
            viewport_width,
            viewport_height,
            1.0,
            &previous_offsets,
        )
    }
}

/// Leaves the timing row at zero when no panel needed a solve.
fn record_layout_perf(perf: &mut DiegeticPerfStats, panel_count: usize, start: Instant) {
    perf.compute_ms = if panel_count == 0 {
        0.0
    } else {
        start.elapsed().as_secs_f32() * MILLISECONDS_PER_SECOND
    };
    perf.compute_panels = panel_count.into();
}

/// Resolves field ids against the previous solve's applied point offsets.
fn previous_caret_offsets(computed: &ComputedDiegeticPanel) -> ScrollSolveInput {
    let mut offsets = ScrollSolveInput::default();
    if let Some(previous) = computed.layout().solved() {
        for record in computed.field_records() {
            if let Some(scroll) = previous
                .scrolls
                .iter()
                .find(|scroll| scroll.element_index == record.element_index)
            {
                offsets.insert(record.field_id.clone(), scroll.offset_points.x);
            }
        }
    }
    offsets
}

/// Builds the cache-backed text measurer for one layout pass.
///
/// Clones the shared [`ShapedTextCache`] handle (a refcount bump, not a map
/// copy) into the returned `'static` closure. Each lookup hits the shared cache;
/// on a miss it falls back to the parley-backed [`DiegeticTextMeasurer`] and
/// inserts the result back into the shared cache, so measurements computed
/// during layout persist for the renderer instead of being discarded each frame.
fn build_cached_measure(cache: &ShapedTextCache, measurer: &DiegeticTextMeasurer) -> MeasureTextFn {
    let cache_handle = cache.clone();
    let parley_fn = Arc::clone(&measurer.measure_fn);

    Arc::new(move |text: &str, measure: &TextMeasure| {
        if let Some(dims) = cache_handle.get_measurement(text, measure) {
            return dims;
        }
        let dims = parley_fn(text, measure);
        cache_handle.insert_measurement(text, measure, dims);
        dims
    })
}

/// Finalizes a completed layout pass onto the panel.
///
/// Records panel-surface bounds (converted to world units) and editable-field
/// records, warning on duplicate field ids.
fn commit_layout_result(
    computed: &mut ComputedDiegeticPanel,
    panel_ref: &DiegeticPanel,
    scaled_tree: &LayoutTree,
    result: LayoutResult,
    entity: Entity,
) {
    if let Some(bounds) = panel_surface_bounds(&result) {
        let s = panel_ref.points_to_world();
        computed.set_content_size(bounds.width * s, bounds.height * s);
    }

    let (field_records, field_id_conflicts) =
        field::collect_panel_field_records(scaled_tree, &result);
    if !field_id_conflicts.is_empty() {
        bevy::log::warn!(
            target: DIAGNOSTIC_LOG_TARGET,
            "panel {entity:?} has duplicate editable field ids: {field_id_conflicts:?}"
        );
    }
    let widget_records = scaled_tree.computed_widget_records(&result);
    let tooltip_records = scaled_tree.computed_tooltip_records();
    let scroll_records = scroll::collect_panel_scroll_records(
        scaled_tree,
        &result,
        panel_ref.layout_unit().to_points(),
    );
    computed.set_result_with_fields(
        result,
        field_records,
        field_id_conflicts,
        widget_records,
        tooltip_records,
        scroll_records,
    );
}

/// Returns the solved root surface, including root padding and border.
fn panel_surface_bounds(result: &LayoutResult) -> Option<BoundingBox> {
    result.computed.first().map(|computed| computed.bounds)
}

/// Resolves `Fit`-axis world panels to their panel-surface bounds.
///
/// Runs after [`compute_panel_layouts`] writes the layout result. For each
/// world panel whose width or height is `Sizing::Fit { min, max }`, reads
/// the computed root bounds (in layout points) and shrinks the panel's
/// physical width / height to match, clamped to `[min, max]`.
///
/// Screen panels resolve their own dynamic sizing earlier in the pipeline
/// via `position_screen_space_panels` + `resolve_screen_axis`, so this
/// system only touches world panels.
pub(super) fn resolve_world_panel_fit(
    mut panels: Query<(
        Entity,
        &mut DiegeticPanel,
        &ComputedDiegeticPanel,
        &mut LastPanelDimensions,
    )>,
    mut commands: Commands,
) {
    for (entity, mut panel, computed, mut last_dimensions) in &mut panels {
        let (w_sizing, h_sizing) = match panel.coordinate_space() {
            CoordinateSpace::World { width, height } => (*width, *height),
            CoordinateSpace::Screen { .. } => continue,
        };

        if let Some(bounds) = computed.layout().solved().and_then(panel_surface_bounds) {
            let layout_to_points = panel.layout_unit().to_points();
            if layout_to_points > 0.0 {
                let horizontal_content = bounds.width / layout_to_points;
                let vertical_content = bounds.height / layout_to_points;

                if let Sizing::Fit { min, max } = w_sizing {
                    let clamped = horizontal_content.clamp(min.value, max.value);
                    if (panel.width() - clamped).abs() > PANEL_RESIZE_EPSILON {
                        panel.set_width(clamped);
                    }
                }
                if let Sizing::Fit { min, max } = h_sizing {
                    let clamped = vertical_content.clamp(min.value, max.value);
                    if (panel.height() - clamped).abs() > PANEL_RESIZE_EPSILON {
                        panel.set_height(clamped);
                    }
                }
            }
        }

        events::trigger_panel_dimensions_changed(
            &mut commands,
            entity,
            &panel,
            computed,
            &mut last_dimensions,
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact expected layout values"
)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::sync::Arc;

    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::Entity;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::window::Ime;
    use bevy::window::PrimaryWindow;
    use bevy::window::Window;
    use bevy::window::WindowClosed;
    use bevy::window::WindowFocused;
    use bevy::window::WindowRef;
    use hana_kana::ToF32;

    use super::panel_surface_bounds;
    use crate::Anchor;
    use crate::Border;
    use crate::BoundingBox;
    use crate::CascadeEntityCommandsExt as _;
    use crate::El;
    use crate::Fit;
    use crate::FitMax;
    use crate::HeadlessDiegeticUiPlugin;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::ImeOpenSession;
    use crate::ImeTarget;
    use crate::Mm;
    use crate::Padding;
    use crate::PanelElementId;
    use crate::Percent;
    use crate::Px;
    use crate::Sizing;
    use crate::TextStyle;
    use crate::cascade::FontUnit;
    use crate::cascade::Resolved;
    use crate::constants::MONOSPACE_WIDTH_RATIO;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutTree;
    use crate::layout::RenderCommandKind;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::Unit;
    use crate::panel::ComputedDiegeticPanel;
    use crate::panel::DiegeticPanel;
    use crate::panel::DiegeticPanelCommands;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::panel::PanelChangeKind;
    use crate::panel::PanelChanged;
    use crate::panel::PanelDimensionsChanged;
    use crate::panel::diegetic_panel::ScaledLayoutTreeCache;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;

    const FONT_RELAYOUT_PANEL_HEIGHT_MM: f32 = 30.0;
    const FONT_RELAYOUT_PANEL_WIDTH_MM: f32 = 50.0;
    const FONT_RELAYOUT_TEXT_SIZE: f32 = 6.0;

    fn monospace_measurer() -> DiegeticTextMeasurer {
        DiegeticTextMeasurer {
            measure_fn: Arc::new(|text: &str, measure: &TextMeasure| {
                let char_width = measure.size * MONOSPACE_WIDTH_RATIO;
                let mut max_line_width: f32 = 0.0;
                let mut line_count = 0_u32;
                for line in text.lines() {
                    line_count += 1;
                    let width = line.chars().count().to_f32() * char_width;
                    max_line_width = max_line_width.max(width);
                }
                if line_count == 0 {
                    line_count = 1;
                }
                TextDimensions {
                    width:       max_line_width,
                    height:      measure.size * line_count.to_f32(),
                    line_height: measure.size,
                }
            }),
        }
    }

    fn make_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(monospace_measurer());
        app.add_plugins(HeadlessLayoutPlugin);
        app
    }

    fn assert_inline_tree_solved(app: &App, panel: Entity) {
        let world = app.world();
        let tree_len = world
            .get::<DiegeticPanel>(panel)
            .expect("panel exists")
            .tree()
            .len();
        let computed = world
            .get::<ComputedDiegeticPanel>(panel)
            .expect("computed panel exists");
        let solved_len = computed
            .layout()
            .solved()
            .expect("panel solved")
            .computed
            .len();
        assert_eq!(solved_len, tree_len, "tree and solve differ on this update");
        let scroll = computed
            .scroll_records()
            .iter()
            .find(|record| record.id == PanelElementId::named("field"))
            .expect("named field scroll is solved on this update");
        assert!(scroll.element_index < solved_len);
    }

    #[test]
    fn inline_ime_tree_has_solved_geometry_on_each_update() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .add_message::<Ime>()
            .add_message::<WindowClosed>()
            .add_message::<WindowFocused>()
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let window = app.world_mut().spawn(Window::default()).id();
        let mut builder = LayoutBuilder::new(140.0, 40.0);
        builder.with(
            El::new().size(72.0, 18.0).editable_field(
                "field",
                ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text)),
                TextStyle::new(12.0),
            ),
            |builder| {
                builder.text(("", TextStyle::new(12.0)));
            },
        );
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::screen()
                    .size(Px(140.0), Px(40.0))
                    .window(WindowRef::Entity(window))
                    .font_unit(Unit::Pixels)
                    .with_tree(builder.build())
                    .build()
                    .expect("field panel builds"),
            )
            .id();
        app.update();
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::ScreenPanelField {
                panel,
                field_id: PanelElementId::named("field"),
            },
            window,
            initial_text: String::new(),
            field_spec: ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Text,
            )),
            anchor: None,
        });
        app.update();
        assert_inline_tree_solved(&app, panel);
        for value in "macOS field scrolling across a very long single line".chars() {
            app.world_mut().write_message(Ime::Commit {
                window,
                value: value.to_string(),
            });
            app.update();
            assert_inline_tree_solved(&app, panel);
        }
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Home,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
        app.update();
        assert_inline_tree_solved(&app, panel);
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected {expected}, got {actual}",
        );
    }

    fn colored_text_tree(text: &str, color: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((text, TextStyle::new(10.0).with_color(color)));
        builder.build()
    }

    fn first_text_color(computed: &ComputedDiegeticPanel) -> Color {
        let result = computed
            .layout()
            .solved()
            .expect("layout result should exist");
        result
            .commands
            .iter()
            .find_map(|command| {
                if let RenderCommandKind::Text { config, .. } = &command.kind {
                    Some(config.color())
                } else {
                    None
                }
            })
            .expect("panel should produce a text command")
    }

    fn first_text_bounds(computed: &ComputedDiegeticPanel) -> BoundingBox {
        let result = computed
            .layout()
            .solved()
            .expect("layout result should exist");
        result
            .commands
            .iter()
            .find_map(|command| {
                matches!(command.kind, RenderCommandKind::Text { .. }).then_some(command.bounds)
            })
            .expect("panel should produce a text command")
    }

    #[derive(Resource, Default)]
    struct DimensionEventLog(Vec<PanelDimensionsChanged>);

    #[derive(Resource, Default)]
    struct PanelChangeEventLog(Vec<PanelChanged>);

    fn record_dimension_event(
        event: On<PanelDimensionsChanged>,
        mut log: ResMut<DimensionEventLog>,
    ) {
        log.0.push(*event.event());
    }

    fn record_panel_changed_event(event: On<PanelChanged>, mut log: ResMut<PanelChangeEventLog>) {
        log.0.push(*event.event());
    }

    #[test]
    fn world_panel_dimensions_changed_fires_once_for_first_measurement() {
        let mut app = make_app();
        app.init_resource::<DimensionEventLog>();
        app.add_observer(record_dimension_event);
        let entity = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Fit, Fit)
                    .with_tree(colored_text_tree("Hello", Color::WHITE))
                    .build()
                    .expect("panel should build"),
            )
            .id();

        app.update();

        let log = app.world().resource::<DimensionEventLog>();
        assert_eq!(log.0.len(), 1);
        let event = log.0[0];
        assert_eq!(event.entity, entity);
        assert!(event.previous.is_none());
        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel should exist");
        assert_close(event.dimensions.resolved_size.x, panel.width());
        assert_close(event.dimensions.resolved_size.y, panel.height());
        assert_close(
            event.dimensions.resolved_size.x,
            event.dimensions.content_size.x,
        );
        assert_close(
            event.dimensions.resolved_size.y,
            event.dimensions.content_size.y,
        );

        app.update();

        let log = app.world().resource::<DimensionEventLog>();
        assert_eq!(log.0.len(), 1);
    }

    #[test]
    fn queued_visual_only_tree_change_regenerates_commands_without_moving_content() {
        let mut app = make_app();
        let entity = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(colored_text_tree("Hello", Color::WHITE))
                    .build()
                    .expect("panel should build"),
            )
            .id();

        app.update();

        let before = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .expect("computed panel should exist")
            .content_bounds()
            .expect("content bounds should exist");
        assert_eq!(
            first_text_color(
                app.world()
                    .get::<ComputedDiegeticPanel>(entity)
                    .expect("computed panel should exist")
            ),
            Color::WHITE
        );

        assert!(
            app.world_mut()
                .commands()
                .set_tree(entity, colored_text_tree("Hello", Color::BLACK))
                .is_ok()
        );
        app.update();

        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .expect("computed panel should exist");
        let after = computed
            .content_bounds()
            .expect("content bounds should still exist");
        assert_eq!(before, after);
        assert_eq!(first_text_color(computed), Color::BLACK);

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel should exist");
        assert_eq!(u64::from(panel.tree_revision()), 1);
    }

    #[test]
    fn visual_only_tree_change_fires_panel_changed_but_not_dimensions_changed() {
        let mut app = make_app();
        app.init_resource::<PanelChangeEventLog>();
        app.init_resource::<DimensionEventLog>();
        app.add_observer(record_panel_changed_event);
        app.add_observer(record_dimension_event);

        let entity = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(colored_text_tree("Hello", Color::WHITE))
                    .build()
                    .expect("panel should build"),
            )
            .id();
        app.update();
        app.world_mut()
            .resource_mut::<PanelChangeEventLog>()
            .0
            .clear();
        app.world_mut()
            .resource_mut::<DimensionEventLog>()
            .0
            .clear();

        assert!(
            app.world_mut()
                .commands()
                .set_tree(entity, colored_text_tree("Hello", Color::BLACK))
                .is_ok()
        );
        app.update();

        let panel_change_events = &app.world().resource::<PanelChangeEventLog>().0;
        assert_eq!(panel_change_events.len(), 1);
        assert_eq!(panel_change_events[0].entity, entity);
        assert_eq!(panel_change_events[0].kind, PanelChangeKind::VisualOnly);
        assert!(
            app.world().resource::<DimensionEventLog>().0.is_empty(),
            "color-only edits refresh computed output without changing dimensions",
        );
    }

    #[test]
    fn visual_only_tree_change_with_font_unit_override_recomputes_text_geometry() {
        let mut app = make_app();
        app.init_resource::<PanelChangeEventLog>();
        app.add_observer(record_panel_changed_event);

        let entity = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(
                        Mm(FONT_RELAYOUT_PANEL_WIDTH_MM),
                        Mm(FONT_RELAYOUT_PANEL_HEIGHT_MM),
                    )
                    .font_unit(Unit::Points)
                    .with_tree(colored_text_tree("Hello", Color::WHITE))
                    .build()
                    .expect("font relayout panel should build"),
            )
            .id();
        app.update();

        let initial_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(entity)
                .expect("panel should complete its initial layout"),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<FontUnit>>(entity)
                .expect("panel should resolve its initial font unit")
                .0
                .0,
            Unit::Points
        );
        assert_eq!(
            first_text_color(
                app.world()
                    .get::<ComputedDiegeticPanel>(entity)
                    .expect("computed panel should exist")
            ),
            Color::WHITE
        );
        app.world_mut()
            .resource_mut::<PanelChangeEventLog>()
            .0
            .clear();

        {
            let mut commands = app.world_mut().commands();
            assert!(
                commands
                    .set_tree(entity, colored_text_tree("Hello", Color::BLACK))
                    .is_ok()
            );
            commands
                .entity(entity)
                .override_font_unit(Unit::Millimeters);
        }
        app.update();

        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .expect("panel should recompute after the combined update");
        let updated_bounds = first_text_bounds(computed);
        assert_eq!(first_text_color(computed), Color::BLACK);
        assert_eq!(
            app.world()
                .get::<Resolved<FontUnit>>(entity)
                .expect("panel should resolve its runtime font unit")
                .0
                .0,
            Unit::Millimeters
        );
        assert!(
            updated_bounds.height > initial_bounds.height,
            "millimeter text should be taller than point text after the combined update"
        );

        let panel_change_events = &app.world().resource::<PanelChangeEventLog>().0;
        assert_eq!(panel_change_events.len(), 1);
        assert_eq!(panel_change_events[0].entity, entity);
        assert_eq!(
            panel_change_events[0].kind,
            PanelChangeKind::LayoutAffecting
        );
    }

    #[test]
    fn repeated_queued_tree_changes_compose_to_layout_affecting() {
        let mut app = make_app();
        let entity = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(colored_text_tree("Hi", Color::WHITE))
                    .build()
                    .expect("panel should build"),
            )
            .id();

        app.update();
        let before = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .expect("computed panel should exist")
            .content_bounds()
            .expect("content bounds should exist");

        {
            let mut commands = app.world_mut().commands();
            assert!(
                commands
                    .set_tree(entity, colored_text_tree("Hi", Color::BLACK))
                    .is_ok()
            );
            assert!(
                commands
                    .set_tree(entity, colored_text_tree("Hello", Color::BLACK))
                    .is_ok()
            );
        }
        app.update();

        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .expect("computed panel should exist");
        let after = computed
            .content_bounds()
            .expect("content bounds should still exist");
        assert!(
            after.width > before.width,
            "layout-affecting text change should recompute content width"
        );
        assert_eq!(first_text_color(computed), Color::BLACK);

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel should exist");
        assert_eq!(u64::from(panel.tree_revision()), 2);
    }

    #[test]
    fn world_fit_panel_shrinks_to_content_bounds_in_meters() {
        let mut app = make_app();

        let panel = DiegeticPanel::world()
            .size(Fit, Fit)
            .layout(|b| {
                b.text(("Hello", TextStyle::new(Mm(6.0))));
            })
            .build()
            .expect("Fit world panel should build even at zero initial size");

        let entity = app.world_mut().spawn(panel).id();

        app.update();
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel component must exist");

        assert!(
            panel.width() > 0.0,
            "panel.width() = {}, expected > 0 after layout",
            panel.width()
        );
        assert!(
            panel.height() > 0.0,
            "panel.height() = {}, expected > 0 after layout",
            panel.height()
        );

        let horizontal_meters = 5.0 * 6.0 * MONOSPACE_WIDTH_RATIO * 0.001;
        let vertical_meters = 6.0 * 0.001;
        assert!(
            (panel.width() - horizontal_meters).abs() < 0.001,
            "panel.width() = {}, expected ~{horizontal_meters}",
            panel.width()
        );
        assert!(
            (panel.height() - vertical_meters).abs() < 0.001,
            "panel.height() = {}, expected ~{vertical_meters}",
            panel.height()
        );
    }

    #[test]
    fn world_fit_panel_with_bordered_root_keeps_root_chrome_in_viewport() {
        const PANEL_WIDTH_MM: f32 = 100.0;
        const BORDER_MM: f32 = 2.0;
        const PADDING_MM: f32 = 3.0;
        const FONT_SIZE_MM: f32 = 6.0;
        const EXPECTED_PANEL_HEIGHT_MM: f32 = FONT_SIZE_MM + PADDING_MM * 2.0 + BORDER_MM * 2.0;

        let mut app = make_app();
        let mut builder = LayoutBuilder::with_root(
            El::column()
                .width(Sizing::fixed(Mm(PANEL_WIDTH_MM)))
                .height(Sizing::FIT)
                .background(Color::BLACK)
                .border(Border::all(Mm(BORDER_MM), Color::WHITE)),
        );
        builder.with(El::column().width(Sizing::GROW).height(Sizing::FIT), |b| {
            b.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .padding(Padding::all(Mm(PADDING_MM))),
                |b| {
                    b.text(("Hi", TextStyle::new(Mm(FONT_SIZE_MM))));
                },
            );
        });

        let entity = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(PANEL_WIDTH_MM), Fit)
                    .with_tree(builder.build())
                    .build()
                    .expect("Fit world panel with bordered root should build"),
            )
            .id();

        app.update();
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel component must exist");
        assert_close(panel.height(), EXPECTED_PANEL_HEIGHT_MM);

        let result = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .expect("computed panel should exist")
            .layout()
            .solved()
            .expect("layout result should exist");
        let surface_bounds =
            panel_surface_bounds(result).expect("root surface bounds should exist");
        let inner_bounds = result
            .content_bounds()
            .expect("inner content bounds should exist");
        assert!(
            surface_bounds.height > inner_bounds.height,
            "surface height should include root chrome; surface={}, inner={}",
            surface_bounds.height,
            inner_bounds.height,
        );
    }

    #[test]
    fn world_fit_panel_with_explicit_unit_shrinks_in_that_unit() {
        let mut app = make_app();

        let panel = DiegeticPanel::world()
            .size(Fit, FitMax(Mm(1000.0).into()))
            .layout(|b| {
                b.text(("Hello", TextStyle::new(Mm(6.0))));
            })
            .build()
            .expect("Fit/FitMax world panel should build");

        let entity = app.world_mut().spawn(panel).id();
        app.update();
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel component must exist");

        let horizontal_mm = 5.0 * 6.0 * MONOSPACE_WIDTH_RATIO;
        let vertical_mm = 6.0;
        assert!(
            (panel.width() - horizontal_mm).abs() < 0.5,
            "panel.width() = {}, expected ~{horizontal_mm} mm",
            panel.width()
        );
        assert!(
            (panel.height() - vertical_mm).abs() < 0.5,
            "panel.height() = {}, expected ~{vertical_mm} mm",
            panel.height()
        );
    }

    #[test]
    fn world_fitmax_panel_caps_at_max() {
        let mut app = make_app();

        let panel = DiegeticPanel::world()
            .size(FitMax(Mm(20.0).into()), Fit)
            .layout(|b| {
                b.text(("HelloHello", TextStyle::new(Mm(6.0))));
            })
            .build()
            .expect("FitMax world panel should build");

        let entity = app.world_mut().spawn(panel).id();
        app.update();
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel component must exist");

        assert!(
            (panel.width() - 20.0).abs() < 0.5,
            "panel.width() = {}, expected ~20.0 (capped)",
            panel.width()
        );
    }

    #[test]
    fn screen_fit_panel_shrinks_to_content_bounds() {
        let mut app = make_app();
        app.world_mut().spawn((
            Window {
                resolution: (1600_u32, 900_u32).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        app.add_plugins(ScreenSpacePlugin);

        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .layout(|b| {
                b.with(
                    crate::El::new()
                        .width(crate::Sizing::GROW)
                        .height(crate::Sizing::GROW)
                        .padding(crate::Padding::all(8.0)),
                    |b| {
                        b.text(("Hello", TextStyle::new(16.0)));
                    },
                );
            })
            .build()
            .expect("Fit screen panel should build");

        let entity = app.world_mut().spawn(panel).id();

        for _ in 0..5 {
            app.update();
        }

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel component must exist");

        assert!(
            panel.width() > 0.0 && panel.width() < 200.0,
            "panel.width() = {}, expected tight shrink-wrap value",
            panel.width()
        );
        assert!(
            panel.height() > 0.0 && panel.height() < 100.0,
            "panel.height() = {}, expected tight shrink-wrap value",
            panel.height()
        );

        let (ax, ay) = panel.anchor_offsets();
        assert!(
            ax.abs() < 0.01 && ay.abs() < 0.01,
            "TopLeft anchor_offsets = ({ax}, {ay}), expected ~(0, 0)",
        );
    }

    /// A `Fit` screen panel runs its first solve before `resolve_screen_axis`
    /// writes `panel.width()` back, so the viewport arrives zero-area. The root
    /// clip is seeded from the solved root box (content-sized for `Fit`), not the
    /// zero viewport, so an owner-bounds draw line must survive that first solve
    /// rather than clipping against an empty rectangle.
    #[test]
    fn screen_fit_panel_emits_owner_clipped_line_on_first_solve() {
        let mut app = make_app();
        app.world_mut().spawn((
            Window {
                resolution: (1600_u32, 900_u32).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        app.add_plugins(ScreenSpacePlugin);

        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .layout(|b| {
                b.with(
                    crate::El::new()
                        .width(crate::Sizing::fixed(Px(40.0)))
                        .height(crate::Sizing::fixed(Px(12.0)))
                        .draw(crate::PanelDraw::lines([crate::PanelLine::new(
                            crate::PanelPoint::new(
                                crate::PanelCoord::start(0.0),
                                crate::PanelCoord::percent(0.5),
                            ),
                            crate::PanelPoint::new(
                                crate::PanelCoord::end(0.0),
                                crate::PanelCoord::percent(0.5),
                            ),
                        )
                        .width(Px(1.0))
                        .color(Color::WHITE)])),
                    |_| {},
                );
            })
            .build()
            .expect("Fit screen panel with a connector line should build");

        let entity = app.world_mut().spawn(panel).id();

        // One update runs the first solve, where panel.width() is still 0.
        app.update();

        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .expect("computed panel must exist");
        let shape_commands = computed
            .layout()
            .solved()
            .expect("layout result must exist")
            .commands
            .iter()
            .filter(|command| matches!(command.kind, RenderCommandKind::PanelShapes { .. }))
            .count();
        assert_eq!(
            shape_commands, 1,
            "owner-bounds line must be emitted on the first (zero-viewport) solve",
        );
    }

    #[test]
    fn screen_anchor_offsets_equal_panel_size_for_all_sizing_modes() {
        let mut app = make_app();
        app.world_mut().spawn((
            Window {
                resolution: (1600_u32, 900_u32).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        app.add_plugins(ScreenSpacePlugin);

        let fixed = DiegeticPanel::screen()
            .size(Px(600.0), Px(44.0))
            .anchor(Anchor::BottomRight)
            .layout(|b| {
                b.text(("fixed", TextStyle::new(16.0)));
            })
            .build()
            .expect("fixed screen panel");
        let percent = DiegeticPanel::screen()
            .size(Percent(0.25), Px(200.0))
            .anchor(Anchor::TopRight)
            .layout(|b| {
                b.text(("percent", TextStyle::new(16.0)));
            })
            .build()
            .expect("percent screen panel");
        let fit = DiegeticPanel::screen()
            .size(Fit, Fit)
            .anchor(Anchor::BottomRight)
            .layout(|b| {
                b.text(("fit", TextStyle::new(16.0)));
            })
            .build()
            .expect("fit screen panel");

        let e_fixed = app.world_mut().spawn(fixed).id();
        let e_percent = app.world_mut().spawn(percent).id();
        let e_fit = app.world_mut().spawn(fit).id();

        for _ in 0..5 {
            app.update();
        }

        for (entity, label, anchor) in [
            (e_fixed, "fixed", Anchor::BottomRight),
            (e_percent, "percent", Anchor::TopRight),
            (e_fit, "fit", Anchor::BottomRight),
        ] {
            let panel = app
                .world()
                .get::<DiegeticPanel>(entity)
                .expect("panel component");
            let (ax, ay) = panel.anchor_offsets();
            let (fx, fy) = anchor.offset_fraction();
            let expected_x = panel.width() * fx;
            let expected_y = panel.height() * fy;
            assert!(
                (ax - expected_x).abs() < 0.01,
                "{label}: anchor_offset.x = {ax}, expected {expected_x} \
                 (panel.width={}, fx={fx})",
                panel.width()
            );
            assert!(
                (ay - expected_y).abs() < 0.01,
                "{label}: anchor_offset.y = {ay}, expected {expected_y} \
                 (panel.height={}, fy={fy})",
                panel.height()
            );
        }
    }

    #[test]
    fn screen_window_resize_reuses_scaled_layout_tree_cache() {
        let mut app = make_app();
        let window_entity = app
            .world_mut()
            .spawn((
                Window {
                    resolution: (1600_u32, 900_u32).into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        app.add_plugins(ScreenSpacePlugin);

        let panel = DiegeticPanel::screen()
            .size(Percent(0.25), Percent(0.20))
            .layout(|b| {
                b.text(("Resize", TextStyle::new(16.0)));
            })
            .build()
            .expect("percent screen panel should build");

        let panel_entity = app.world_mut().spawn(panel).id();

        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(panel_entity)
            .expect("panel component must exist");
        assert_eq!(panel.width(), 400.0);
        assert_eq!(panel.height(), 180.0);

        let cache = app
            .world()
            .get::<ScaledLayoutTreeCache>(panel_entity)
            .expect("cache component must exist");
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.hits(), 0);

        {
            let mut window = app
                .world_mut()
                .get_mut::<Window>(window_entity)
                .expect("primary window component must exist");
            window.resolution.set(2000.0, 1000.0);
        }

        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(panel_entity)
            .expect("panel component must exist");
        assert_eq!(panel.width(), 500.0);
        assert_eq!(panel.height(), 200.0);

        let cache = app
            .world()
            .get::<ScaledLayoutTreeCache>(panel_entity)
            .expect("cache component must exist");
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.hits(), 1);

        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel_entity)
            .expect("computed panel component must exist");
        assert!(computed.is_solved());
    }

    #[test]
    fn world_fixed_panel_keeps_its_declared_size() {
        let mut app = make_app();

        let panel = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|b| {
                b.text(("Hi", TextStyle::new(Mm(6.0))));
            })
            .build()
            .expect("fixed-size world panel should build");

        let entity = app.world_mut().spawn(panel).id();
        app.update();
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel component must exist");

        assert_eq!(panel.width(), 50.0);
        assert_eq!(panel.height(), 30.0);
    }

    #[test]
    fn chained_font_unit_override_wins_on_initial_layout() {
        let mut app = make_app();
        let (builder_unit, chained_override) = {
            let mut commands = app.world_mut().commands();
            let builder_unit = commands
                .spawn(
                    DiegeticPanel::world()
                        .size(Mm(50.0), Mm(30.0))
                        .font_unit(Unit::Millimeters)
                        .layout(|builder| {
                            builder.text(("Hi", TextStyle::new(6.0)));
                        })
                        .build()
                        .expect("builder-unit panel should build"),
                )
                .id();
            let chained_override = commands
                .spawn(
                    DiegeticPanel::world()
                        .size(Mm(50.0), Mm(30.0))
                        .font_unit(Unit::Millimeters)
                        .layout(|builder| {
                            builder.text(("Hi", TextStyle::new(6.0)));
                        })
                        .build()
                        .expect("chained-override panel should build"),
                )
                .override_font_unit(Unit::Points)
                .id();
            (builder_unit, chained_override)
        };

        app.update();

        let builder_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(builder_unit)
                .expect("builder-unit panel should be computed"),
        );
        let override_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(chained_override)
                .expect("chained-override panel should be computed"),
        );
        assert!(
            builder_bounds.height > override_bounds.height,
            "millimeter builder text should be taller than point override text on the first layout"
        );
        assert_eq!(
            app.world()
                .get::<Resolved<FontUnit>>(chained_override)
                .expect("chained-override panel should resolve its font unit")
                .0
                .0,
            Unit::Points
        );
    }

    #[test]
    fn runtime_font_unit_override_recomputes_text_geometry() {
        let mut app = make_app();
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(
                        Mm(FONT_RELAYOUT_PANEL_WIDTH_MM),
                        Mm(FONT_RELAYOUT_PANEL_HEIGHT_MM),
                    )
                    .font_unit(Unit::Points)
                    .layout(|builder| {
                        builder.text(("Hi", TextStyle::new(FONT_RELAYOUT_TEXT_SIZE)));
                    })
                    .build()
                    .expect("font relayout panel should build"),
            )
            .id();

        app.update();

        let initial_unit = app
            .world()
            .get::<Resolved<FontUnit>>(panel)
            .expect("panel should resolve its initial font unit")
            .0
            .0;
        let initial_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("panel should complete its initial layout"),
        );
        assert_eq!(initial_unit, Unit::Points);

        app.world_mut()
            .commands()
            .entity(panel)
            .override_font_unit(Unit::Millimeters);
        app.update();

        let updated_unit = app
            .world()
            .get::<Resolved<FontUnit>>(panel)
            .expect("panel should resolve its runtime font unit")
            .0
            .0;
        let updated_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("panel should recompute after its font unit changes"),
        );
        assert_eq!(updated_unit, Unit::Millimeters);
        assert!(
            updated_bounds.height > initial_bounds.height,
            "millimeter text should be taller than point text after a live font-unit override"
        );
    }

    #[test]
    fn global_font_unit_change_recomputes_inherited_text_geometry_in_same_update() {
        let mut app = make_app();
        *app.world_mut().resource_mut::<FontUnit>() = FontUnit(Unit::Points);
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(
                        Mm(FONT_RELAYOUT_PANEL_WIDTH_MM),
                        Mm(FONT_RELAYOUT_PANEL_HEIGHT_MM),
                    )
                    .layout(|builder| {
                        builder.text(("Hi", TextStyle::new(FONT_RELAYOUT_TEXT_SIZE)));
                    })
                    .build()
                    .expect("font relayout panel should build"),
            )
            .id();
        app.update();

        app.world_mut().commands().entity(panel).inherit_font_unit();
        app.update();

        let initial_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("panel should complete its inherited-unit layout"),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<FontUnit>>(panel)
                .expect("panel should resolve the initial global font unit")
                .0
                .0,
            Unit::Points
        );

        *app.world_mut().resource_mut::<FontUnit>() = FontUnit(Unit::Millimeters);
        app.update();

        let updated_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("panel should recompute during the global-unit update"),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<FontUnit>>(panel)
                .expect("panel should resolve the updated global font unit")
                .0
                .0,
            Unit::Millimeters
        );
        assert!(
            updated_bounds.height > initial_bounds.height,
            "layout should consume the propagated global font unit in the same update"
        );
    }

    #[test]
    fn explicit_panel_font_unit_affects_initial_layout() {
        let mut app = make_app();
        let default_unit = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(50.0), Mm(30.0))
                    .layout(|builder| {
                        builder.text(("Hi", TextStyle::new(6.0)));
                    })
                    .build()
                    .expect("default-unit panel should build"),
            )
            .id();
        let explicit_unit = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(50.0), Mm(30.0))
                    .font_unit(Unit::Millimeters)
                    .layout(|builder| {
                        builder.text(("Hi", TextStyle::new(6.0)));
                    })
                    .build()
                    .expect("explicit-unit panel should build"),
            )
            .id();

        app.update();

        let default_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(default_unit)
                .expect("default-unit panel should be computed"),
        );
        let explicit_bounds = first_text_bounds(
            app.world()
                .get::<ComputedDiegeticPanel>(explicit_unit)
                .expect("explicit-unit panel should be computed"),
        );
        assert!(
            explicit_bounds.height > default_bounds.height,
            "millimeter-authored text should be taller than point-default text on the first layout"
        );
    }

    #[test]
    fn headless_panel_resolves_seeded_font_unit_to_points() {
        let mut app = make_app();

        let panel = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|b| {
                b.text(("Hi", TextStyle::new(Mm(6.0))));
            })
            .build()
            .expect("headless panel should build");
        let entity = app.world_mut().spawn(panel).id();
        app.update();

        // The panel carries a seeded `Resolved<FontUnit>` from the
        // construction-time `panel_font_unit` (`Points`); `compute_panel_layouts`
        // reads it directly. The `.expect` proves the component is present, so
        // the `defaults.panel_font_unit` fallback in `compute_panel_layouts` is
        // unreached for a panel.
        let resolved = app
            .world()
            .get::<Resolved<FontUnit>>(entity)
            .expect("panel should carry seeded Resolved<FontUnit>");
        assert_eq!(resolved.0.0, Unit::Points);
    }
}
