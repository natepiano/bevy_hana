//! Panel-domain constants: builder defaults and diagnostic paths.

use bevy::color::Color;
use bevy::diagnostic::DiagnosticPath;

// builder defaults
/// Default camera render order for screen-space overlay panels. Chosen high
/// enough that user-spawned 3D viewport cameras (e.g. a minimap at order 1)
/// don't collide with the screen-space camera on the primary window.
pub(super) const DEFAULT_SCREEN_SPACE_CAMERA_ORDER: isize = 100;
/// Default render layer for screen-space overlay panels.
pub(super) const DEFAULT_SCREEN_SPACE_RENDER_LAYER: usize = 31;
/// Minimum implicit world height assigned to fixed screen panels so that
/// 1 layout pixel maps to at least 1 world unit under the ortho camera.
pub(super) const MIN_PANEL_WORLD_HEIGHT: f32 = 1.0;
/// Hysteresis tolerance for `Fit`-axis world panel resize: skip writing back
/// a clamped width or height if it differs from the current value by less
/// than this.
pub(super) const PANEL_RESIZE_EPSILON: f32 = 0.001;

// gizmos
/// Color used for text-bound debug gizmos.
pub(super) const DEBUG_TEXT_GIZMO_COLOR: Color = Color::srgba(0.9, 0.9, 0.2, 0.2);
/// Line width for text-bound debug gizmos, in pixels.
pub(super) const DEBUG_TEXT_GIZMO_LINE_WIDTH: f32 = 1.0;
/// Per-corner segment count for rounded gizmo line joints — controls
/// smoothness of corner curvature on panel border outlines.
pub(super) const GIZMO_LINE_JOINT_SEGMENTS: u32 = 8;

// diagnostic paths
pub(super) const DIAG_LAYOUT_COMPUTE_MS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/layout/compute_ms");
pub(super) const DIAG_LAYOUT_COMPUTE_PANELS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/layout/compute_panels");
pub(super) const DIAG_PANEL_REIFY_MS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel/reify_ms");
pub(super) const DIAG_MATERIAL_TABLE_CAPACITY: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/material_table/capacity");
pub(super) const DIAG_MATERIAL_TABLE_ROWS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/material_table/rows");
pub(super) const DIAG_MATERIAL_TABLE_UPLOAD_BYTES: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/material_table/upload_bytes");
pub(super) const DIAG_PANEL_SDF_BATCHES: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel/sdf_batches");
pub(super) const DIAG_PANEL_SDF_RECORDS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel/sdf_records");
pub(super) const DIAG_PANEL_SDF_UPLOADS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel/sdf_uploads");
pub(super) const DIAG_PANEL_SHAPE_BATCHES: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel_shape/batches");
pub(super) const DIAG_PANEL_SHAPE_RECORDS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel_shape/path_records");
pub(super) const DIAG_PANEL_SHAPE_UPLOADS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel_shape/uploads");
pub(super) const DIAG_PANEL_TEXT_MESH_BUILD_MS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel_text/mesh_build_ms");
pub(super) const DIAG_PANEL_TEXT_PARLEY_MS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel_text/parley_ms");
pub(super) const DIAG_PANEL_TEXT_SHAPED_PANELS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel_text/shaped_panels");
pub(super) const DIAG_PANEL_TEXT_SHAPE_MS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel_text/shape_ms");
pub(super) const DIAG_PANEL_TEXT_TOTAL_MS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/panel_text/total_ms");
pub(super) const DIAG_TEXT_BATCHES: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/text_batch/batches");
pub(super) const DIAG_TEXT_BATCH_RUNS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/text_batch/runs");
pub(super) const DIAG_TEXT_BATCH_GLYPHS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/text_batch/glyph_records");
pub(super) const DIAG_TEXT_BATCH_INSTANCE_UPLOADS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/text_batch/instance_uploads");
pub(super) const DIAG_TEXT_BATCH_RUN_TABLE_UPLOADS: DiagnosticPath =
    DiagnosticPath::const_new("hana_diegetic/text_batch/run_table_uploads");
