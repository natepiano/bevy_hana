//! Panel performance statistics and diagnostics publishing.

use core::fmt;
use core::fmt::Display;
use core::fmt::Formatter;
use core::ops::AddAssign;

use bevy::diagnostic::Diagnostic;
use bevy::diagnostic::Diagnostics;
use bevy::diagnostic::RegisterDiagnostic;
use bevy::prelude::App;
use bevy::prelude::Last;
use bevy::prelude::Plugin;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use hana_kana::ToF64;

use super::constants::DIAG_LAYOUT_COMPUTE_MS;
use super::constants::DIAG_LAYOUT_COMPUTE_PANELS;
use super::constants::DIAG_MATERIAL_TABLE_CAPACITY;
use super::constants::DIAG_MATERIAL_TABLE_ROWS;
use super::constants::DIAG_MATERIAL_TABLE_UPLOAD_BYTES;
use super::constants::DIAG_PANEL_REIFY_MS;
use super::constants::DIAG_PANEL_SDF_BATCHES;
use super::constants::DIAG_PANEL_SDF_RECORDS;
use super::constants::DIAG_PANEL_SDF_UPLOADS;
use super::constants::DIAG_PANEL_SHAPE_BATCHES;
use super::constants::DIAG_PANEL_SHAPE_RECORDS;
use super::constants::DIAG_PANEL_SHAPE_UPLOADS;
use super::constants::DIAG_PANEL_TEXT_MESH_BUILD_MS;
use super::constants::DIAG_PANEL_TEXT_PARLEY_MS;
use super::constants::DIAG_PANEL_TEXT_SHAPE_MS;
use super::constants::DIAG_PANEL_TEXT_SHAPED_PANELS;
use super::constants::DIAG_PANEL_TEXT_TOTAL_MS;
use super::constants::DIAG_TEXT_BATCH_GLYPHS;
use super::constants::DIAG_TEXT_BATCH_INSTANCE_UPLOADS;
use super::constants::DIAG_TEXT_BATCH_RUN_TABLE_UPLOADS;
use super::constants::DIAG_TEXT_BATCH_RUNS;
use super::constants::DIAG_TEXT_BATCHES;
use crate::DrawZIndex;

/// Lightweight timing data for diegetic UI systems.
///
/// These values are updated by the built-in layout and text extraction systems
/// so examples and applications can inspect where time is being spent during
/// content-heavy updates.
///
/// **Note:** This API is provisional. Field names and structure are coupled
/// to the current internal system architecture and may change as the
/// library matures. Consider using Bevy's `DiagnosticsStore` for
/// production profiling.
#[derive(Resource, Clone, Debug, Default, Reflect)]
#[reflect(Resource)]
pub struct DiegeticPerfStats {
    /// Stage 1 — `compute_panel_layouts` wall time in milliseconds.
    /// Layout math: element positions and sizes for every dirty panel.
    pub compute_ms:      f32,
    /// Stage 1 — panels processed by the most recent layout run.
    pub compute_panels:  FrameWork,
    /// Between stages 1 and 2 — `reify_text_entities` wall time in
    /// milliseconds: re-deriving text child entities from each changed panel's
    /// render commands. `route_image_batch_records` does not add to this timing.
    pub reify_ms:        f32,
    /// Stages 2 & 3 — panel-text shaping + record-build timings and counts.
    pub panel_text:      PanelTextPerfStats,
    /// Glyph-batch counters, written by `commit_batch_buffers`.
    pub batch:           BatchPerfStats,
    /// Panel-line analytic path batch counters, written by
    /// `commit_panel_shape_batch_buffers`.
    pub line_batch:      PanelShapeBatchPerfStats,
    /// SDF panel surface batch counters, written by `commit_sdf_batch_buffers`.
    pub panel_geometry:  PanelGeometryPerfStats,
    /// Frame material-table counters, written after the shared table buffer is current.
    pub material_table:  MaterialTablePerfStats,
    /// Per-batch decomposition of the live glyph batches, one entry per text
    /// draw. Rebuilt each frame by `commit_batch_buffers`.
    pub text_breakdown:  Vec<BatchSummary>,
    /// Per-batch decomposition of the live panel-line (analytic path) batches,
    /// rebuilt each frame by `commit_panel_line_batch_buffers`.
    pub shape_breakdown: Vec<BatchSummary>,
    /// Per-batch decomposition of the live SDF surface batches, rebuilt each
    /// frame by `commit_sdf_batch_buffers`.
    pub sdf_breakdown:   Vec<BatchSummary>,
    /// Per-batch decomposition of the live image batches, rebuilt each frame by
    /// `commit_image_batch_buffers`. Every image batch is `Blend`, unlit, and
    /// textured; batches split by texture, render layer, shadow, and z-index.
    pub image_breakdown: Vec<BatchSummary>,
}

/// A quantity that exists right now, replaced whenever its producer measures it.
///
/// A live count answers "how many are there", so adding one frame's to the next
/// multiplies it instead of tracking it. `LiveCount` has no [`AddAssign`], which
/// is what stops that from compiling. Build one with `LiveCount::from`.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Reflect)]
pub struct LiveCount(usize);

impl Display for LiveCount {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
}

impl LiveCount {
    /// The measured quantity.
    #[must_use]
    pub const fn get(self) -> usize { self.0 }
}

impl From<usize> for LiveCount {
    fn from(count: usize) -> Self { Self(count) }
}

/// Work performed during one frame, rebuilt from zero each frame by the system
/// that performs it.
///
/// A `FrameWork` describes only the frame that produced it; the next frame's
/// producer replaces it wholesale. Anything that must stay readable after the
/// frame it happened on is a [`LifetimeTotal`] instead.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Reflect)]
pub struct FrameWork(usize);

impl Display for FrameWork {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
}

impl FrameWork {
    /// Work counted for the frame that produced this value.
    #[must_use]
    pub const fn get(self) -> usize { self.0 }
}

impl From<usize> for FrameWork {
    fn from(work: usize) -> Self { Self(work) }
}

impl AddAssign<usize> for FrameWork {
    fn add_assign(&mut self, work: usize) { self.0 += work; }
}

/// A running total since startup that only ever grows.
///
/// [`AddAssign`] is the only way to write one — there is no `From<usize>` and no
/// setter — so no per-frame producer can reset it. That is what keeps a transient
/// event readable after the frame it happened on: a surface dropped once, hundreds
/// of frames ago, is still counted in the total.
///
/// Addition saturates. A total that runs for the life of the process must not
/// panic on overflow in a debug build.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Reflect)]
pub struct LifetimeTotal(usize);

impl Display for LifetimeTotal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
}

impl LifetimeTotal {
    /// Everything counted since startup.
    #[must_use]
    pub const fn get(self) -> usize { self.0 }
}

impl AddAssign<usize> for LifetimeTotal {
    fn add_assign(&mut self, amount: usize) { self.0 = self.0.saturating_add(amount); }
}

/// One live batch's identity, for the per-family batch decomposition in
/// [`DiegeticPerfStats`].
///
/// Carries the batch-key discriminants a viewer needs to see why a family split
/// into more than one draw: render layer, lit/unlit shader path, alpha mode, and
/// whether a base-color texture is bound. Each entry is one draw; `record_count`
/// is how many records routed into it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Reflect)]
pub struct BatchSummary {
    /// Authored `DrawZIndex` value for the batch.
    pub z_index:       DrawZIndex,
    /// Active render-layer indices copied from the batch key.
    pub render_layers: Vec<u32>,
    /// Batch casts a shadow, a batch-key discriminant that splits otherwise
    /// identical draws.
    pub casts_shadow:  bool,
    /// Unlit shader path, from the batch's pipeline compatibility.
    pub unlit:         bool,
    /// Short alpha-mode label, from the batch's pipeline compatibility.
    pub alpha_mode:    String,
    /// A base-color texture is bound, splitting this batch from untextured ones.
    pub textured:      bool,
    /// Records routed into this batch.
    pub record_count:  LiveCount,
}

/// Per-frame glyph-batch counters, written by `commit_batch_buffers`.
///
/// The two upload counters are split to match the store's per-buffer dirty
/// flags: a transform-only frame uploads only run tables, a same-count text
/// edit only instance buffers, an unchanged frame nothing.
#[derive(Clone, Debug, Default, Reflect)]
pub struct BatchPerfStats {
    /// Live batch count (one render entity + one draw per pass each).
    pub batches:           LiveCount,
    /// Text runs routed across all batches.
    pub runs:              LiveCount,
    /// Glyph instance records across all batches.
    pub glyph_records:     LiveCount,
    /// Glyph-instance buffer uploads this frame.
    pub instance_uploads:  FrameWork,
    /// Run-table buffer uploads this frame.
    pub run_table_uploads: FrameWork,
}

/// Per-frame panel-line analytic path batch counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub struct PanelShapeBatchPerfStats {
    /// Live vector-mark batch count.
    pub batches: LiveCount,
    /// Analytic path instance records routed across all batches.
    pub records: LiveCount,
    /// Analytic path instance/run buffer uploads this frame.
    pub uploads: FrameWork,
}

/// Per-frame panel geometry counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub struct PanelGeometryPerfStats {
    /// Live SDF batch render entities for panel backgrounds, borders, and
    /// divider rectangles.
    pub sdf_batches:       LiveCount,
    /// Live SDF records routed across all SDF batches.
    pub sdf_records:       LiveCount,
    /// SDF record-buffer uploads this frame.
    pub sdf_uploads:       FrameWork,
    /// `StoredResolvedSdfSurface`s held in `ResolvedSdfSurfaceRegistry` when
    /// `route_sdf_batch_records` last ran.
    ///
    /// This is the input `sdf_records` is produced from. Zero here means
    /// `build_panel_geometry` resolved no surfaces; non-zero here with zero
    /// `sdf_records` means every surface was dropped, and `dropped_surfaces`
    /// names the cause.
    pub resolved_surfaces: LiveCount,
    /// Resolved surfaces that routed no SDF record, by cause, totalled since
    /// startup.
    ///
    /// These are [`LifetimeTotal`]s. A drop is usually transient —
    /// one frame empties a panel and the next frame is clean — so a per-frame
    /// count can be non-zero at the moment of failure and zero by the time
    /// anyone reads it.
    pub dropped_surfaces:  DroppedSdfSurfaces,
}

/// Why `route_sdf_batch_records` routed no SDF record for a resolved surface.
///
/// Every `StoredResolvedSdfSurface` either becomes an `SdfRecordKey` in
/// `SdfBatchStore` or increments exactly one of these counters, so an empty
/// store reports the reason it is empty rather than only the fact.
///
/// `Counter` is the kind of number held. A producer tallies one frame into a
/// `DroppedSdfSurfaces<FrameWork>`, and `Self::accumulate` is the only route
/// from there to the [`LifetimeTotal`] copy on [`PanelGeometryPerfStats`].
/// Assigning a frame's tally over the running totals does not typecheck, so a drop
/// stays readable after the frame it happened on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub struct DroppedSdfSurfaces<Counter = LifetimeTotal> {
    /// `build_panel_geometry` reached a panel whose `ComputedDiegeticPanel`
    /// holds no `PanelLayout::Solved`, so it evicted the panel's surfaces
    /// without resolving new ones. This empties a panel that is still spawned
    /// and still visible.
    pub layout_unsolved:  Counter,
    /// The owning panel entity no longer matches the `DiegeticPanel` query, so
    /// `ResolvedSdfSurfaceRegistry::remove_panel` evicts its surfaces.
    pub panel_missing:    Counter,
    /// Neither `ResolvedSdfSurface::fill_material` nor `border_material` is
    /// authored, so the surface paints nothing.
    pub unauthored:       Counter,
    /// The surface's base `StandardMaterial` is still loading, so
    /// `material::material_asset_for_frame` withheld it for a later frame.
    pub material_pending: Counter,
    /// `FrameMaterialTableBuilder` refused a row because the frame material
    /// table is at capacity.
    pub slot_limit:       Counter,
}

impl DroppedSdfSurfaces<LifetimeTotal> {
    /// Adds one frame's tally to the running totals.
    ///
    /// The only writer of the totals, and the only thing a frame tally can do to
    /// them.
    pub(crate) fn accumulate(&mut self, frame: &DroppedSdfSurfaces<FrameWork>) {
        self.layout_unsolved += frame.layout_unsolved.get();
        self.panel_missing += frame.panel_missing.get();
        self.unauthored += frame.unauthored.get();
        self.material_pending += frame.material_pending.get();
        self.slot_limit += frame.slot_limit.get();
    }
}

/// Shared material-table counters.
///
/// Every field but [`Self::allocations`] holds current-frame state;
/// [`Self::allocations`] is a [`LifetimeTotal`] and accumulates across frames.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub struct MaterialTablePerfStats {
    /// Current-frame `MaterialSlotValues` rows appended by render producers.
    pub rows:         LiveCount,
    /// Bytes represented by the current-frame live material rows.
    pub upload_bytes: LiveCount,
    /// Row capacity of the shared material-table storage buffer.
    pub capacity:     LiveCount,
    /// Microseconds spent cloning the row Vec into the frozen frame table.
    pub freeze_us:    u64,
    /// Microseconds spent padding the rows to capacity and writing them into
    /// the storage buffer. Paid every frame regardless of whether any material
    /// changed; a durable slot table would skip this on a no-change frame.
    pub upload_us:    u64,
    /// Buffer reallocations from capacity grow and shrink events, since startup.
    pub allocations:  LifetimeTotal,
}

/// Panel-text per-frame timings. Covers stages 2 and 3 of the panel pipeline:
///
/// 1. `compute_panel_layouts` → [`DiegeticPerfStats::compute_ms`], then `reify_text_entities` →
///    [`DiegeticPerfStats::reify_ms`]
/// 2. `shape_panel_text_children` → [`Self::shape_ms`] (strings → positioned glyphs)
/// 3. `update_panel_text_batches` → [`Self::mesh_build_ms`] (glyphs → batch records)
///
/// Render-pass time is not measured here — Bevy's own diagnostics report it
/// (`FrameTimeDiagnosticsPlugin`, `RenderDiagnosticsPlugin`) and it is outside
/// this crate's control.
///
/// All text is covered: standalone `DiegeticText` labels are one-element
/// panels, so every `TextContent` run flows through this pipeline.
#[derive(Clone, Debug, Default, Reflect)]
pub struct PanelTextPerfStats {
    /// End-to-end panel-text wall time this frame, in milliseconds.
    /// Equals [`Self::shape_ms`] + [`Self::mesh_build_ms`].
    ///
    /// Written twice per frame: first by `shape_panel_text_children` using
    /// the *previous* frame's `mesh_build_ms`, then overwritten by
    /// `update_panel_text_batches` using the current frame's values. The
    /// final value is only correct because the record build is scheduled
    /// `.after(shape_panel_text_children)`; reordering those systems would
    /// leave `total_ms` stale by one frame.
    pub total_ms:      f32,
    /// Stage 2 — wall time of `shape_panel_text_children` this frame.
    /// Covers turning strings into positioned glyphs for every panel-text
    /// entity that changed or is waiting on glyph loading.
    pub shape_ms:      f32,
    /// Inside [`Self::shape_ms`] — time spent in parley text shaping,
    /// summed across entities. If this dominates, the cost is content-side
    /// (many strings, complex scripts, heavy font features).
    pub parley_ms:     f32,
    /// Stage 3 — wall time of `update_panel_text_batches` this frame.
    /// Covers building glyph records, routing runs through the batch store,
    /// and reconciling batch entities and GPU assets.
    pub mesh_build_ms: f32,
    /// Number of panels whose text shaping ran this frame.
    pub shaped_panels: FrameWork,
}

#[derive(Resource, Default)]
struct DiegeticDiagnosticsRegistered;

pub(super) struct DiagnosticsPlugin;

impl Plugin for DiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        if app
            .world()
            .contains_resource::<DiegeticDiagnosticsRegistered>()
        {
            return;
        }

        app.insert_resource(DiegeticDiagnosticsRegistered);
        // Auto-registration cannot see generic types, so the instantiation held
        // by `PanelGeometryPerfStats` is registered by hand.
        app.register_type::<DroppedSdfSurfaces>();
        for diagnostic in [
            Diagnostic::new(DIAG_LAYOUT_COMPUTE_MS).with_suffix(" ms"),
            Diagnostic::new(DIAG_LAYOUT_COMPUTE_PANELS),
            Diagnostic::new(DIAG_PANEL_REIFY_MS).with_suffix(" ms"),
            Diagnostic::new(DIAG_MATERIAL_TABLE_ROWS),
            Diagnostic::new(DIAG_MATERIAL_TABLE_UPLOAD_BYTES).with_suffix(" bytes"),
            Diagnostic::new(DIAG_MATERIAL_TABLE_CAPACITY),
            Diagnostic::new(DIAG_PANEL_SDF_BATCHES),
            Diagnostic::new(DIAG_PANEL_SDF_RECORDS),
            Diagnostic::new(DIAG_PANEL_SDF_UPLOADS),
            Diagnostic::new(DIAG_PANEL_SHAPE_BATCHES),
            Diagnostic::new(DIAG_PANEL_SHAPE_RECORDS),
            Diagnostic::new(DIAG_PANEL_SHAPE_UPLOADS),
            Diagnostic::new(DIAG_PANEL_TEXT_TOTAL_MS).with_suffix(" ms"),
            Diagnostic::new(DIAG_PANEL_TEXT_SHAPE_MS).with_suffix(" ms"),
            Diagnostic::new(DIAG_PANEL_TEXT_PARLEY_MS).with_suffix(" ms"),
            Diagnostic::new(DIAG_PANEL_TEXT_MESH_BUILD_MS).with_suffix(" ms"),
            Diagnostic::new(DIAG_PANEL_TEXT_SHAPED_PANELS),
            Diagnostic::new(DIAG_TEXT_BATCHES),
            Diagnostic::new(DIAG_TEXT_BATCH_RUNS),
            Diagnostic::new(DIAG_TEXT_BATCH_GLYPHS),
            Diagnostic::new(DIAG_TEXT_BATCH_INSTANCE_UPLOADS),
            Diagnostic::new(DIAG_TEXT_BATCH_RUN_TABLE_UPLOADS),
        ] {
            app.register_diagnostic(diagnostic);
        }

        app.add_systems(Last, publish_perf_diagnostics);
    }
}

fn publish_perf_diagnostics(perf: Res<DiegeticPerfStats>, mut diagnostics: Diagnostics) {
    diagnostics.add_measurement(&DIAG_LAYOUT_COMPUTE_MS, || f64::from(perf.compute_ms));
    diagnostics.add_measurement(&DIAG_LAYOUT_COMPUTE_PANELS, || {
        perf.compute_panels.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_PANEL_REIFY_MS, || f64::from(perf.reify_ms));
    diagnostics.add_measurement(&DIAG_MATERIAL_TABLE_ROWS, || {
        perf.material_table.rows.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_MATERIAL_TABLE_UPLOAD_BYTES, || {
        perf.material_table.upload_bytes.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_MATERIAL_TABLE_CAPACITY, || {
        perf.material_table.capacity.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_PANEL_SDF_BATCHES, || {
        perf.panel_geometry.sdf_batches.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_PANEL_SDF_RECORDS, || {
        perf.panel_geometry.sdf_records.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_PANEL_SDF_UPLOADS, || {
        perf.panel_geometry.sdf_uploads.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_PANEL_SHAPE_BATCHES, || {
        perf.line_batch.batches.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_PANEL_SHAPE_RECORDS, || {
        perf.line_batch.records.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_PANEL_SHAPE_UPLOADS, || {
        perf.line_batch.uploads.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_PANEL_TEXT_TOTAL_MS, || {
        f64::from(perf.panel_text.total_ms)
    });
    diagnostics.add_measurement(&DIAG_PANEL_TEXT_SHAPE_MS, || {
        f64::from(perf.panel_text.shape_ms)
    });
    diagnostics.add_measurement(&DIAG_PANEL_TEXT_PARLEY_MS, || {
        f64::from(perf.panel_text.parley_ms)
    });
    diagnostics.add_measurement(&DIAG_PANEL_TEXT_MESH_BUILD_MS, || {
        f64::from(perf.panel_text.mesh_build_ms)
    });
    diagnostics.add_measurement(&DIAG_PANEL_TEXT_SHAPED_PANELS, || {
        perf.panel_text.shaped_panels.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_TEXT_BATCHES, || perf.batch.batches.get().to_f64());
    diagnostics.add_measurement(&DIAG_TEXT_BATCH_RUNS, || perf.batch.runs.get().to_f64());
    diagnostics.add_measurement(&DIAG_TEXT_BATCH_GLYPHS, || {
        perf.batch.glyph_records.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_TEXT_BATCH_INSTANCE_UPLOADS, || {
        perf.batch.instance_uploads.get().to_f64()
    });
    diagnostics.add_measurement(&DIAG_TEXT_BATCH_RUN_TABLE_UPLOADS, || {
        perf.batch.run_table_uploads.get().to_f64()
    });
}
