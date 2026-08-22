//! Batching validation scene for SDF surfaces, text, and analytic shapes.
//!
//! The scene displays authored record counts, material-table rows, and expected
//! batch counts alongside live renderer counters. `R` toggles HDR and `[` / `]`
//! tune `HdrTextCoverageBias`, the cascading text coverage compensation used
//! when analytic text looks too thin under HDR, especially dark text on light
//! backgrounds.

use std::fmt::Write as _;

use bevy::camera::Hdr;
use bevy::camera::primitives::Aabb;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::diagnostic::Diagnostic;
use bevy::diagnostic::DiagnosticsStore;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomCompositeMode;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::Add;
use bevy::prelude::AlphaMode;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Camera;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Image;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::LinearRgba;
use bevy::prelude::Local;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::PointLight;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy_kana::ToF32;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::FairyDustOrbitCam;
use fairy_dust::OrbitCamPose;
use fairy_dust::SprinkleBuilder;
use fairy_dust::StatsPanelRow;
use fairy_dust::StatsPanelSection;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use fairy_dust::diegetic_stats_sections_panel_with_integral_advance;
use fairy_dust::diegetic_stats_sections_tree_with_integral_advance;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::BatchSummary;
use hana_diegetic::Border;
use hana_diegetic::CalloutCap;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::HdrTextCoverageBias;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::LineStyle;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelAnchorOffset;
use hana_diegetic::PanelAttachment;
use hana_diegetic::PanelCircle;
use hana_diegetic::PanelCoord;
use hana_diegetic::PanelDraw;
use hana_diegetic::PanelEntityReader;
use hana_diegetic::PanelLine;
use hana_diegetic::PanelPoint;
use hana_diegetic::Px;
use hana_diegetic::ShadowCasting;
use hana_diegetic::Sidedness;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::default_panel_material;
use hana_lagrange::OrbitCamPreset;

const PANEL_W: f32 = 170.0;
const PANEL_H: f32 = 132.0;
const PANEL_GAP_X: f32 = 0.012;
const PANEL_GAP_Y: f32 = 0.012;
const MM_TO_WORLD: f32 = 0.001;
const PANEL_WORLD_W: f32 = PANEL_W * MM_TO_WORLD;
const PANEL_WORLD_H: f32 = PANEL_H * MM_TO_WORLD;
const PANEL_STEP_X: f32 = PANEL_WORLD_W + PANEL_GAP_X;
const PANEL_STEP_Y: f32 = PANEL_WORLD_H + PANEL_GAP_Y;
const PANEL_GRID_CENTER_Y: f32 = 0.17;
const PANEL_HOME_PAD: f32 = 0.012;
const GROUND_SIZE: f32 = 1.45;
const HOME_FOCUS: Vec3 = Vec3::new(0.0, PANEL_GRID_CENTER_Y, 0.0);
const HOME_RADIUS: f32 = 0.50;
const HOME_PITCH: f32 = 0.0;
const HOME_MARGIN: f32 = 0.33;
// Point light placed in front of and up-left of the metallic center card
// (world center ~(-0.091, 0.242, 0), front face +Z) to land a specular glint.
const GLINT_LIGHT_POS: Vec3 = Vec3::new(-0.13, 0.30, 0.16);
const GLINT_LIGHT_LUMENS: f32 = 2000.0;

// Authored content each panel draws, counted by family. Each panel renders its
// own copy in the upper-right corner, so per-panel readouts stay consistent.

const SDF_ANIMATION_GREEN_OFFSET: f32 = 2.1;
const SDF_ANIMATION_RED_OFFSET: f32 = 4.2;
const SDF_ANIMATION_SPEED: f32 = 0.9;
const DIAGNOSTIC_UPDATE_INTERVAL: f32 = 1.0;
const HDR_CONTROL: &str = "R HDR";
const BLOOM_CONTROL: &str = "M Bloom";
const TONEMAPPING_CONTROL: &str = "T Tonemapping";
const TEXT_COVERAGE_CONTROL: &str = "Text Coverage";
const TEXT_COVERAGE_LEFT_SEGMENT: &str = "text-coverage-left";
const TEXT_COVERAGE_RIGHT_SEGMENT: &str = "text-coverage-right";
const TEXT_COVERAGE_TARGET_VALUE_SEGMENT: &str = "text-coverage-target-value";
const TEXT_COVERAGE_ACTIVE_VALUE_SEGMENT: &str = "text-coverage-active-value";

// One render family's live batch decomposition, paired for the breakdown table.
struct FamilyBreakdown<'a> {
    label:        &'static str,
    color:        Color,
    batch_count:  usize,
    record_total: usize,
    batches:      &'a [BatchSummary],
}

// The four families in the order the diagnostic renders: text, shape, sdf,
// image. Image has no dedicated counter struct; its batch and record totals are
// derived from the per-batch breakdown `commit_image_batch_buffers` publishes.
fn family_breakdowns(perf: &DiegeticPerfStats) -> [FamilyBreakdown<'_>; 4] {
    [
        FamilyBreakdown {
            label:        "text",
            color:        ACCENT_GREEN,
            batch_count:  perf.batch.batches.get(),
            record_total: perf.batch.glyph_records.get(),
            batches:      &perf.text_breakdown,
        },
        FamilyBreakdown {
            label:        "shape",
            color:        ACCENT_YELLOW,
            batch_count:  perf.line_batch.batches.get(),
            record_total: perf.line_batch.records.get(),
            batches:      &perf.shape_breakdown,
        },
        FamilyBreakdown {
            label:        "sdf",
            color:        ACCENT_BLUE,
            batch_count:  perf.panel_geometry.sdf_batches.get(),
            record_total: perf.panel_geometry.sdf_records.get(),
            batches:      &perf.sdf_breakdown,
        },
        FamilyBreakdown {
            label:        "image",
            color:        ACCENT_RED,
            batch_count:  perf.image_breakdown.len(),
            record_total: perf
                .image_breakdown
                .iter()
                .map(|batch| batch.record_count.get())
                .sum(),
            batches:      &perf.image_breakdown,
        },
    ]
}

// Derived batching invariants, checked per family against the live breakdown.
// No authored target: a green latch means the renderer's own decomposition is
// self-consistent, which holds across any panel set.
//
//   1. one breakdown row per counted draw,
//   2. every record routed into exactly one batch (per-batch counts sum to the family total),
//   3. no empty batch lingering.
fn batch_invariant_failures(perf: &DiegeticPerfStats) -> Vec<String> {
    let mut failures = Vec::new();
    for family in family_breakdowns(perf) {
        if family.batches.len() != family.batch_count {
            failures.push(format!(
                "{}: {} draws, {} rows",
                family.label,
                family.batch_count,
                family.batches.len()
            ));
        }
        let routed: usize = family
            .batches
            .iter()
            .map(|batch| batch.record_count.get())
            .sum();
        if routed != family.record_total {
            failures.push(format!(
                "{}: {routed}/{} records routed",
                family.label, family.record_total
            ));
        }
        if family
            .batches
            .iter()
            .any(|batch| batch.record_count.get() == 0)
        {
            failures.push(format!("{}: empty batch", family.label));
        }
    }
    failures
}

// Compact label for why a batch is its own draw: render layer, then the
// discriminants that vary across the scene's batches — texture binding and the
// unlit screen-layer path. Alpha mode is appended when it leaves Blend.
fn batch_reason(batch: &BatchSummary) -> String {
    let layer = batch
        .render_layers
        .first()
        .map_or_else(|| "L?".to_owned(), |index| format!("L{index}"));
    let mut tags = Vec::new();
    tags.push(if batch.textured {
        "textured".to_owned()
    } else {
        "untextured".to_owned()
    });
    if batch.unlit {
        tags.push("unlit".to_owned());
    }
    if batch.casts_shadow {
        tags.push("shadow".to_owned());
    }
    if batch.alpha_mode != "Blend" {
        tags.push(batch.alpha_mode.to_lowercase());
    }
    format!("{layer} {}", tags.join(" "))
}

// Per-family column colors: text green, shape yellow, sdf blue, image red.
const LEDGER_FAMILY_COLORS: [Color; 4] = [ACCENT_GREEN, ACCENT_YELLOW, ACCENT_BLUE, ACCENT_RED];
const LEDGER_TITLE_FONT_SIZE: f32 = 11.25;
const LEDGER_FONT_SIZE: f32 = 10.0;
const LEDGER_NUM_WIDTH: f32 = 40.0;
const LEDGER_ROW_GAP: f32 = 2.0;
const LEDGER_CELL_GAP: f32 = 4.0;
const LEDGER_BREAKDOWN_LABEL_MEASURE: &str = "L0 untextured shadow alphatocoverage";
const LEDGER_BREAKDOWN_MIN_SPACER_MEASURE: &str = "  ";
const LEDGER_BREAKDOWN_VALUE_MEASURE: &str = "00";
const LEDGER_SEPARATOR_COLOR: Color = Color::srgba(0.1, 0.4, 0.6, 0.3);
const CARD_RADIUS: Mm = Mm(4.0);
const IMAGE_CARD_CLIPPED_BORDER_FILL: Color = Color::srgba(0.08, 0.11, 0.12, 1.0);
const IMAGE_CARD_CLIPPED_BORDER_RADIUS: Mm = Mm(1.0);
const IMAGE_CARD_CLIPPED_BORDER_WIDTH: Mm = Mm(0.35);
const PANEL_PAD: Mm = Mm(4.0);
const ROW_GAP: f32 = 4.0;
const TITLE_FONT_SIZE: f32 = 18.75;
const SUBTITLE_FONT_SIZE: f32 = 11.25;
const BODY_FONT_SIZE: f32 = 15.75;
const SMALL_FONT_SIZE: f32 = 12.75;
const MATERIAL_GROUP_TITLE_FONT_SIZE: f32 = 16.0;
const MATERIAL_CASE_FONT_SIZE: f32 = 15.0;
const MATERIAL_CASE_CAPTION_FONT_SIZE: f32 = 11.75;
const SHAPE_CARD_CAPTION_FONT_SIZE: f32 = 10.5;
const MATERIAL_GROUP_GAP: f32 = 2.0;
const MATERIAL_CASE_GAP: f32 = 0.5;
const MATERIAL_CASE_PAD_X: f32 = 2.0;
const MATERIAL_CASE_PAD_Y: f32 = 1.0;
const MATERIAL_VALUE_GAP: f32 = 2.0;
const SWATCH_FONT_SIZE: f32 = 24.0;
const MATERIAL_CASE_BG: Color = Color::srgba(0.10, 0.11, 0.13, 0.72);
const CARD_BG: Color = Color::srgba(0.055, 0.065, 0.075, 0.94);
const CARD_BG_ALT: Color = Color::srgba(0.075, 0.055, 0.075, 0.94);
const CARD_BORDER: Color = Color::srgba(0.34, 0.56, 0.72, 0.75);
const CARD_BORDER_WARM: Color = Color::srgba(0.84, 0.56, 0.26, 0.78);
const ACCENT_BLUE: Color = Color::srgb(0.24, 0.62, 0.95);
const ACCENT_GREEN: Color = Color::srgb(0.32, 0.88, 0.54);
const ACCENT_YELLOW: Color = Color::srgb(0.95, 0.78, 0.24);
const ACCENT_RED: Color = Color::srgb(0.95, 0.34, 0.30);
// Over-bright warm readout. Base color is a material-table value, so boosting it
// past 1.0 keeps the run in the Shared group's single batch (unlike `Unlit`,
// which the plan classifies as a pipeline splitter).
const EMISSIVE_WARM: Color = Color::linear_rgb(3.6, 2.3, 0.2);
// Sharp cool-white glint. `metallic` is a material-table value, so this row stays
// in the Shared group's single batch. Kept at full range (<=1.0) so it reads as a
// crisp highlight without crossing the bloom threshold the warm readout owns.
const GLINT: Color = Color::linear_rgb(0.95, 0.97, 1.0);
const TEXT_MAIN: Color = Color::srgb(0.90, 0.92, 0.96);
const TEXT_MUTED: Color = Color::srgba(0.64, 0.70, 0.78, 0.9);
const SDF_EMISSIVE_TEXT_INK: Color = Color::srgb(0.06, 0.16, 0.95);
const SDF_EMISSIVE_TEXT_GLOW: Color = Color::linear_rgb(0.0, 0.4, 4.6);
const BATCH_BLOOM_INTENSITY: f32 = 0.25;
const BATCH_BLOOM_THRESHOLD: f32 = 4.0;
const BATCH_BLOOM_THRESHOLD_SOFTNESS: f32 = 0.0;
const TEXT_EMISSIVE_GAIN: f32 = 1.8;
// Match the A4 text sample's black-on-white treatment so the live alpha case is
// judged against the same contrast baseline as `units.rs`.
const ALPHA_CELL_BG: Color = Color::WHITE;
const ALPHA_CELL_INK: Color = Color::BLACK;
const ALPHA_CELL_CAPTION: Color = Color::BLACK;
// Previous HDR-path shader compensation for this cell:
// const ALPHA_CELL_HDR_TEXT_COVERAGE_BIAS: f32 = 2.0;

// The alpha modes the center-left selector cycles through, in letter-key order
// (A..G). The selected mode is applied to the SDF panel's fills and
// borders and to the Text panel's live alpha case, so both render in the same
// mode and the per-mode shadow behavior is observable on the ground plane.
const ALPHA_MODES: [(&str, AlphaMode); 7] = [
    ("Opaque", AlphaMode::Opaque),
    ("Blend", AlphaMode::Blend),
    ("Mask 0.5", AlphaMode::Mask(0.5)),
    ("Premultiplied", AlphaMode::Premultiplied),
    ("Add", AlphaMode::Add),
    ("Multiply", AlphaMode::Multiply),
    ("AlphaToCoverage", AlphaMode::AlphaToCoverage),
];
// Blend is the panel default, so the selector starts there and nothing changes
// until a letter key is pressed.
const ALPHA_DEFAULT_INDEX: usize = 1;
const ALPHA_KEYS: [KeyCode; 7] = [
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
];
const TONEMAPPING_KEYS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];
const ALPHA_ROW_GAP: f32 = 2.0;
const ALPHA_ROW_WIDTH: f32 = 120.0;

const TONEMAPPING_MODES: [(&str, Tonemapping); 9] = [
    ("None", Tonemapping::None),
    ("Reinhard", Tonemapping::Reinhard),
    ("ReinhardLum", Tonemapping::ReinhardLuminance),
    ("ACES", Tonemapping::AcesFitted),
    ("AgX", Tonemapping::AgX),
    ("BoringDisplay", Tonemapping::SomewhatBoringDisplayTransform),
    ("TonyMcMapface", Tonemapping::TonyMcMapface),
    ("BlenderFilmic", Tonemapping::BlenderFilmic),
    ("KhronosPbr", Tonemapping::KhronosPbrNeutral),
];
const TONEMAPPING_DEFAULT_INDEX: usize = 8;
const TONEMAPPING_ROW_GAP: f32 = 2.0;
const TONEMAPPING_ROW_WIDTH: f32 = 150.0;
const TEXT_COVERAGE_BIAS_STEP: f32 = 0.1;
const TEXT_COVERAGE_BIAS_RATE: f32 = 0.8;
const TEXT_COVERAGE_BIAS_MIN: f32 = -4.0;
const TEXT_COVERAGE_BIAS_MAX: f32 = 4.0;

/// The alpha mode the SDF panel fills/borders and the Text panel alpha case
/// currently render in, chosen from [`ALPHA_MODES`] by the center-left selector.
#[derive(Resource)]
struct AlphaModeSelection {
    index: usize,
}

impl Default for AlphaModeSelection {
    fn default() -> Self {
        Self {
            index: ALPHA_DEFAULT_INDEX,
        }
    }
}

impl AlphaModeSelection {
    const fn mode(&self) -> AlphaMode { ALPHA_MODES[self.index].1 }
}

/// The tonemapper applied to every batch-validation camera, cycled by `T`.
#[derive(Resource)]
struct TonemappingSelection {
    index: usize,
}

impl Default for TonemappingSelection {
    fn default() -> Self {
        Self {
            index: TONEMAPPING_DEFAULT_INDEX,
        }
    }
}

impl TonemappingSelection {
    const fn mode(&self) -> Tonemapping { TONEMAPPING_MODES[self.index].1 }

    const fn cycle(&mut self) { self.index = (self.index + 1) % TONEMAPPING_MODES.len(); }
}

/// HDR-only coverage target, adjusted by `[` and `]`.
#[derive(Resource, Clone, Copy, Default)]
struct HdrTextCoverageSelection {
    selected: f32,
}

impl HdrTextCoverageSelection {
    const fn active_for(self, features: RenderFeatures) -> f32 {
        match features.hdr {
            RenderFeature::On => self.selected,
            RenderFeature::Off => 0.0,
        }
    }
}

#[derive(Component)]
struct BatchValidationPanel;

/// Marker for the SDF material-value animation panel.
#[derive(Component)]
struct BatchValidationSdfPanel {
    /// Texture handle reused by the animated SDF material cases.
    image: Handle<Image>,
}

/// Registered SDF source material handles reused by the animated validation panel.
#[derive(Clone, Resource)]
struct SdfSurfaceMaterialHandles {
    /// Source material for the panel-default card.
    panel_default: Handle<StandardMaterial>,
    /// Source material for the metallic animated card.
    metallic:      Handle<StandardMaterial>,
    /// Source material for the emissive animated card.
    emissive:      Handle<StandardMaterial>,
    /// Cool emissive text material for the warm emissive card.
    emissive_text: Handle<StandardMaterial>,
    /// Source material for the texture-backed card.
    image:         Handle<StandardMaterial>,
}

impl SdfSurfaceMaterialHandles {
    /// Registers the SDF card materials once and returns their handles.
    fn new(
        materials: &mut Assets<StandardMaterial>,
        image: Handle<Image>,
        alpha: AlphaMode,
    ) -> Self {
        Self {
            panel_default: materials.add(panel_default_card_material(alpha)),
            metallic:      materials.add(with_alpha(metallic_glint_material(0.0), alpha)),
            emissive:      materials.add(with_alpha(emissive_fill_material(0.0), alpha)),
            emissive_text: materials.add(sdf_emissive_text_material()),
            image:         materials.add(with_alpha(image_fill_material(image), alpha)),
        }
    }

    /// Updates the existing material assets for the current animation frame.
    fn refresh(
        &self,
        materials: &mut Assets<StandardMaterial>,
        image: Handle<Image>,
        phase: f32,
        alpha: AlphaMode,
    ) {
        replace_material_asset(
            materials,
            &self.panel_default,
            panel_default_card_material(alpha),
        );
        replace_material_asset(
            materials,
            &self.metallic,
            with_alpha(metallic_glint_material(phase), alpha),
        );
        replace_material_asset(
            materials,
            &self.emissive,
            with_alpha(emissive_fill_material(phase), alpha),
        );
        replace_material_asset(
            materials,
            &self.image,
            with_alpha(image_fill_material(image), alpha),
        );
    }
}

/// Registered text source material handles reused by the text validation panel.
#[derive(Clone, Resource)]
struct TextPanelMaterialHandles {
    /// Panel-level default text material.
    panel_default: Handle<StandardMaterial>,
    /// Source material for the emissive scalar/vector row.
    emissive:      Handle<StandardMaterial>,
    /// Source material for the metallic scalar/vector row.
    metallic:      Handle<StandardMaterial>,
    /// Source material carrying a base-color texture for the texture split row.
    texture:       Handle<StandardMaterial>,
}

impl TextPanelMaterialHandles {
    /// Registers the text validation source materials once and returns handles.
    fn new(materials: &mut Assets<StandardMaterial>, image: Handle<Image>) -> Self {
        Self {
            panel_default: materials.add(text_panel_default_material()),
            emissive:      materials.add(text_emissive_material()),
            metallic:      materials.add(text_metallic_material()),
            texture:       materials.add(text_texture_material(image)),
        }
    }
}

/// Registered panel-shape source material handles used by the shape validation panel.
#[derive(Clone)]
struct ShapePanelMaterialHandles {
    /// Panel-level shape material default.
    panel_default: Handle<StandardMaterial>,
    /// Shape-local emissive material used through `PanelLine::material`.
    local_line:    Handle<StandardMaterial>,
    /// Scalar material used through `LineStyle::material`.
    style_line:    Handle<StandardMaterial>,
    /// Scalar material used through `PanelCircle::material`.
    circle:        Handle<StandardMaterial>,
    /// Source material whose alpha mode splits the shape batch.
    alpha_split:   Handle<StandardMaterial>,
    /// Source material whose texture resource splits the shape batch.
    texture:       Handle<StandardMaterial>,
}

impl ShapePanelMaterialHandles {
    /// Registers the panel-shape validation source materials once.
    fn new(materials: &mut Assets<StandardMaterial>, image: Handle<Image>) -> Self {
        Self {
            panel_default: materials.add(shape_panel_default_material()),
            local_line:    materials.add(shape_local_line_material()),
            style_line:    materials.add(shape_style_line_material()),
            circle:        materials.add(shape_circle_material()),
            alpha_split:   materials.add(shape_alpha_split_material()),
            texture:       materials.add(shape_texture_material(image)),
        }
    }
}

/// Marker for the Text material panel, rebuilt when the alpha selection changes
/// so its live alpha case re-renders in the chosen mode.
#[derive(Component)]
struct BatchValidationTextPanel;

/// Marker for the center-left alpha-mode selector panel.
#[derive(Component)]
struct AlphaSelectorPanel;

#[derive(Component)]
struct TonemappingSelectorPanel;

#[derive(Component)]
struct BatchValidationStatsPanel;

#[derive(Component)]
struct BatchValidationLedgerPanel;

#[derive(Resource, Default)]
struct LastDisplayedDiagnostics {
    key: String,
}

// Outcome of checking the renderer's live batch decomposition against the
// derived batching invariants. `Stabilizing` holds until the observed batch
// totals stay unchanged for `VALIDATION_STABLE_FRAMES` consecutive frames; the
// latch re-arms whenever the alpha selection changes.
#[derive(Resource, Default)]
struct ValidationStatus {
    state:         ValidationState,
    last_observed: Option<[usize; 4]>,
    stable_frames: u32,
    last_alpha:    usize,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RenderFeature {
    #[default]
    On,
    Off,
}

impl RenderFeature {
    const fn activation(self) -> ControlActivation {
        match self {
            Self::On => ControlActivation::Active,
            Self::Off => ControlActivation::Inactive,
        }
    }
}

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq)]
struct RenderFeatures {
    hdr:   RenderFeature,
    bloom: RenderFeature,
}

impl RenderFeatures {
    const fn toggle_hdr(&mut self) {
        match self.hdr {
            RenderFeature::On => {
                self.hdr = RenderFeature::Off;
                self.bloom = RenderFeature::Off;
            },
            RenderFeature::Off => {
                self.hdr = RenderFeature::On;
            },
        }
    }

    const fn toggle_bloom(&mut self) {
        match self.bloom {
            RenderFeature::On => {
                self.bloom = RenderFeature::Off;
            },
            RenderFeature::Off => {
                self.hdr = RenderFeature::On;
                self.bloom = RenderFeature::On;
            },
        }
    }
}

#[derive(Default, PartialEq, Eq)]
enum ValidationState {
    #[default]
    Stabilizing,
    Match,
    Mismatch {
        failures: Vec<String>,
    },
}

const VALIDATION_STABLE_FRAMES: u32 = 30;

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    let sprinkle = fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_perf_mode()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .size(GROUND_SIZE)
        .with_orbit_cam_preset_pose(
            OrbitCamPose {
                focus:  HOME_FOCUS,
                yaw:    0.0,
                pitch:  HOME_PITCH,
                radius: HOME_RADIUS,
            },
            OrbitCamPreset::blender_like(),
        )
        .with_stable_transparency()
        .add_systems(Update, tune_batch_validation_bloom)
        .with_environment_map()
        .with_camera_home()
        .yaw(0.0)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(batch_validation_title_bar(0.0, 0.0))
        .wire_chip_to_state::<RenderFeatures, _>(HDR_CONTROL, |features| features.hdr.activation())
        .wire_chip_to_state::<RenderFeatures, _>(BLOOM_CONTROL, |features| {
            features.bloom.activation()
        })
        .wire_chip_to_state::<ButtonInput<KeyCode>, _>(TEXT_COVERAGE_LEFT_SEGMENT, |keyboard| {
            activation_for(keyboard.pressed(KeyCode::BracketLeft))
        })
        .wire_chip_to_state::<ButtonInput<KeyCode>, _>(TEXT_COVERAGE_RIGHT_SEGMENT, |keyboard| {
            activation_for(keyboard.pressed(KeyCode::BracketRight))
        })
        .wire_chip_to_state::<HdrTextCoverageSelection, _>(
            TEXT_COVERAGE_TARGET_VALUE_SEGMENT,
            |_selection| ControlActivation::Active,
        )
        .wire_chip_to_state::<HdrTextCoverageBias, _>(
            TEXT_COVERAGE_ACTIVE_VALUE_SEGMENT,
            |_active_bias| ControlActivation::Active,
        )
        .with_camera_control_panel();
    let sprinkle = wire_batch_validation_shortcuts(sprinkle);
    sprinkle
        .init_resource::<LastDisplayedDiagnostics>()
        .init_resource::<AlphaModeSelection>()
        .init_resource::<TonemappingSelection>()
        .init_resource::<HdrTextCoverageSelection>()
        .init_resource::<ValidationStatus>()
        .init_resource::<RenderFeatures>()
        .add_systems(
            Startup,
            (
                spawn_validation_panels,
                spawn_stats_panel,
                spawn_expected_batches_panel,
                spawn_alpha_selector_panel,
                spawn_tonemapping_selector_panel,
            ),
        )
        .add_observer(anchor_alpha_selector_when_added)
        .add_observer(anchor_alpha_selector_when_stats_added)
        .add_observer(anchor_tonemapping_selector_when_added)
        .add_observer(anchor_tonemapping_selector_when_alpha_added)
        .add_observer(apply_render_features_to_added_camera)
        .add_observer(apply_tonemapping_to_added_camera)
        .add_observer(apply_render_features_to_added_orbit_camera)
        .add_systems(
            Update,
            validate_batch_counts.before(update_diagnostic_panels),
        )
        .add_systems(Update, update_diagnostic_panels)
        .add_systems(Update, animate_sdf_surface_panel)
        .add_systems(Update, apply_alpha_selection)
        .add_systems(Update, apply_tonemapping_selection)
        .add_systems(
            Update,
            (
                adjust_text_coverage_bias,
                apply_text_coverage_bias_default,
                update_text_coverage_title_bar,
            )
                .chain(),
        )
        .add_systems(PostUpdate, sync_render_feature_components)
        .run();
}

fn wire_batch_validation_shortcuts<S>(sprinkle: SprinkleBuilder<S>) -> SprinkleBuilder<S> {
    sprinkle
        .with_shortcut(ALPHA_KEYS[0], select_alpha::<0>)
        .with_shortcut(ALPHA_KEYS[1], select_alpha::<1>)
        .with_shortcut(ALPHA_KEYS[2], select_alpha::<2>)
        .with_shortcut(ALPHA_KEYS[3], select_alpha::<3>)
        .with_shortcut(ALPHA_KEYS[4], select_alpha::<4>)
        .with_shortcut(ALPHA_KEYS[5], select_alpha::<5>)
        .with_shortcut(ALPHA_KEYS[6], select_alpha::<6>)
        .with_shortcut(TONEMAPPING_KEYS[0], select_tonemapping::<0>)
        .with_shortcut(TONEMAPPING_KEYS[1], select_tonemapping::<1>)
        .with_shortcut(TONEMAPPING_KEYS[2], select_tonemapping::<2>)
        .with_shortcut(TONEMAPPING_KEYS[3], select_tonemapping::<3>)
        .with_shortcut(TONEMAPPING_KEYS[4], select_tonemapping::<4>)
        .with_shortcut(TONEMAPPING_KEYS[5], select_tonemapping::<5>)
        .with_shortcut(TONEMAPPING_KEYS[6], select_tonemapping::<6>)
        .with_shortcut(TONEMAPPING_KEYS[7], select_tonemapping::<7>)
        .with_shortcut(TONEMAPPING_KEYS[8], select_tonemapping::<8>)
        .with_shortcut(KeyCode::KeyR, toggle_hdr)
        .with_shortcut(KeyCode::KeyM, toggle_bloom)
        .with_shortcut(KeyCode::KeyT, cycle_tonemapping)
}

fn spawn_validation_panels(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    selection: Res<AlphaModeSelection>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let sdf_fill_image = asset_server.load("textures/array_texture.png");
    let alpha = selection.mode();
    let sdf_materials =
        SdfSurfaceMaterialHandles::new(&mut materials, sdf_fill_image.clone(), alpha);
    let text_materials = TextPanelMaterialHandles::new(&mut materials, sdf_fill_image.clone());
    let shape_materials = ShapePanelMaterialHandles::new(&mut materials, sdf_fill_image.clone());
    commands.insert_resource(sdf_materials.clone());
    commands.insert_resource(text_materials.clone());
    let panels = [
        (
            "sdf-surfaces",
            build_sdf_surface_panel(&sdf_materials),
            None,
            None,
        ),
        (
            "text-materials",
            build_text_panel(&text_materials, alpha),
            Some(text_materials.panel_default.clone()),
            None,
        ),
        (
            "analytic-shapes",
            build_shape_panel(&shape_materials),
            None,
            Some(shape_materials.panel_default.clone()),
        ),
        (
            "images",
            build_image_panel(sdf_fill_image.clone()),
            None,
            None,
        ),
    ];
    for (index, (name, tree, text_material, shape_material)) in panels.into_iter().enumerate() {
        let (x, y) = panel_grid_position(index);
        let panel = validation_panel(tree, index, text_material, shape_material, &mut materials);
        match panel {
            Ok(panel) => {
                let mut entity = commands.spawn((
                    Name::new(format!("batch validation {name}")),
                    BatchValidationPanel,
                    CameraHomeTarget,
                    panel,
                    Transform::from_xyz(x, y, 0.0),
                ));
                if index == 0 {
                    entity.insert(BatchValidationSdfPanel {
                        image: sdf_fill_image.clone(),
                    });
                }
                if index == 1 {
                    entity.insert(BatchValidationTextPanel);
                }
            },
            Err(error) => error!("batch_validation: failed to build {name}: {error}"),
        }
    }
    commands.spawn((
        CameraHomeTarget,
        Aabb::from_min_max(
            Vec3::new(
                -PANEL_STEP_X.mul_add(0.5, PANEL_WORLD_W.mul_add(0.5, PANEL_HOME_PAD)),
                PANEL_WORLD_H.mul_add(
                    -0.5,
                    PANEL_STEP_Y.mul_add(-0.5, PANEL_GRID_CENTER_Y) - PANEL_HOME_PAD,
                ),
                -PANEL_HOME_PAD,
            ),
            Vec3::new(
                PANEL_STEP_X.mul_add(0.5, PANEL_WORLD_W.mul_add(0.5, PANEL_HOME_PAD)),
                PANEL_WORLD_H.mul_add(
                    0.5,
                    PANEL_STEP_Y.mul_add(0.5, PANEL_GRID_CENTER_Y) + PANEL_HOME_PAD,
                ),
                PANEL_HOME_PAD,
            ),
        ),
        Transform::default(),
    ));
    // Small point light in front of the metallic center card, offset up-left so
    // its specular reflection localizes into a glint on that card. Studio lights
    // are broad and only produce a uniform sheen on a flat metal facing the
    // camera; a point light's direction varies across the surface, so the
    // highlight lands as a hot spot.
    commands.spawn((
        Name::new("glint key light"),
        PointLight {
            intensity: GLINT_LIGHT_LUMENS,
            range: 0.3,
            radius: 0.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_translation(GLINT_LIGHT_POS),
    ));
}

fn animate_sdf_surface_panel(
    time: Res<Time>,
    selection: Res<AlphaModeSelection>,
    sdf_materials: Res<SdfSurfaceMaterialHandles>,
    panels: Query<&BatchValidationSdfPanel>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let phase = time.elapsed_secs() * SDF_ANIMATION_SPEED;
    let alpha = selection.mode();
    for panel in &panels {
        sdf_materials.refresh(&mut materials, panel.image.clone(), phase, alpha);
    }
}

fn tune_batch_validation_bloom(mut blooms: Query<&mut Bloom>) {
    for mut bloom in &mut blooms {
        configure_batch_validation_bloom(&mut bloom);
    }
}

const fn batch_validation_bloom() -> Bloom {
    Bloom {
        intensity: BATCH_BLOOM_INTENSITY,
        prefilter: BloomPrefilter {
            threshold:          BATCH_BLOOM_THRESHOLD,
            threshold_softness: BATCH_BLOOM_THRESHOLD_SOFTNESS,
        },
        composite_mode: BloomCompositeMode::Additive,
        ..Bloom::OLD_SCHOOL
    }
}

const fn configure_batch_validation_bloom(bloom: &mut Bloom) { *bloom = batch_validation_bloom(); }

fn batch_validation_title_bar(target_bias: f32, active_bias: f32) -> TitleBar {
    TitleBar::new()
        .with_title("Batch Validation")
        .with_anchor(Anchor::TopLeft)
        .active_control(HDR_CONTROL)
        .active_control(BLOOM_CONTROL)
        .active_control(TONEMAPPING_CONTROL)
        .control(text_coverage_title_control(target_bias, active_bias))
}

fn text_coverage_title_control(target_bias: f32, active_bias: f32) -> TitleBarControl {
    TitleBarControl::segmented(
        TEXT_COVERAGE_CONTROL,
        [
            TitleBarSegment::new(TEXT_COVERAGE_LEFT_SEGMENT, "["),
            TitleBarSegment::new(TEXT_COVERAGE_RIGHT_SEGMENT, "]"),
            TitleBarSegment::new("text-coverage-target-label", "target"),
            TitleBarSegment::new(
                TEXT_COVERAGE_TARGET_VALUE_SEGMENT,
                format!("{target_bias:+.2}"),
            ),
            TitleBarSegment::new("text-coverage-active-label", "active"),
            TitleBarSegment::new(
                TEXT_COVERAGE_ACTIVE_VALUE_SEGMENT,
                format!("{active_bias:+.2}"),
            ),
        ],
    )
}

const fn activation_for(active: bool) -> ControlActivation {
    if active {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}

fn toggle_hdr(mut features: ResMut<RenderFeatures>) { features.toggle_hdr(); }

fn toggle_bloom(mut features: ResMut<RenderFeatures>) { features.toggle_bloom(); }

fn cycle_tonemapping(mut selection: ResMut<TonemappingSelection>) { selection.cycle(); }

fn select_tonemapping<const INDEX: usize>(mut selection: ResMut<TonemappingSelection>) {
    selection.index = INDEX;
}

fn select_alpha<const INDEX: usize>(mut selection: ResMut<AlphaModeSelection>) {
    selection.index = INDEX;
}

fn adjust_text_coverage_bias(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<HdrTextCoverageSelection>,
) {
    let left = keyboard.pressed(KeyCode::BracketLeft);
    let right = keyboard.pressed(KeyCode::BracketRight);
    let direction = match (left, right) {
        (true, false) => -1.0,
        (false, true) => 1.0,
        (true, true) | (false, false) => return,
    };
    let key = if direction < 0.0 {
        KeyCode::BracketLeft
    } else {
        KeyCode::BracketRight
    };
    let amount = if keyboard.just_pressed(key) {
        TEXT_COVERAGE_BIAS_STEP
    } else {
        TEXT_COVERAGE_BIAS_RATE * time.delta_secs()
    };
    let next = (selection.selected + direction * amount)
        .clamp(TEXT_COVERAGE_BIAS_MIN, TEXT_COVERAGE_BIAS_MAX);
    if (selection.selected - next).abs() > f32::EPSILON {
        selection.selected = next;
    }
}

fn apply_text_coverage_bias_default(
    selection: Res<HdrTextCoverageSelection>,
    features: Res<RenderFeatures>,
    mut cascade_default: ResMut<HdrTextCoverageBias>,
) {
    let bias = selection.active_for(*features);
    if (cascade_default.0 - bias).abs() > f32::EPSILON {
        cascade_default.0 = bias;
    }
}

fn apply_render_features_to_added_camera(
    trigger: On<Add, Camera>,
    features: Res<RenderFeatures>,
    cameras: Query<Option<&Hdr>, With<Camera>>,
    mut commands: Commands,
) {
    let Ok(hdr) = cameras.get(trigger.entity) else {
        return;
    };
    set_hdr_component(trigger.entity, hdr, features.hdr, &mut commands);
}

fn apply_tonemapping_to_added_camera(
    trigger: On<Add, Camera>,
    selection: Res<TonemappingSelection>,
    cameras: Query<Option<&Tonemapping>, With<Camera>>,
    mut commands: Commands,
) {
    let Ok(tonemapping) = cameras.get(trigger.entity) else {
        return;
    };
    set_tonemapping_component(trigger.entity, tonemapping, selection.mode(), &mut commands);
}

fn apply_render_features_to_added_orbit_camera(
    trigger: On<Add, FairyDustOrbitCam>,
    features: Res<RenderFeatures>,
    cameras: Query<(Option<&Hdr>, Option<&Bloom>), With<FairyDustOrbitCam>>,
    mut commands: Commands,
) {
    let Ok((hdr, bloom)) = cameras.get(trigger.entity) else {
        return;
    };
    set_bloom_component(trigger.entity, bloom, features.bloom, &mut commands);
    set_hdr_component(trigger.entity, hdr, features.hdr, &mut commands);
}

fn sync_render_feature_components(
    features: Res<RenderFeatures>,
    cameras: Query<(Entity, Option<&Hdr>), With<Camera>>,
    bloom_cameras: Query<(Entity, Option<&Bloom>), With<FairyDustOrbitCam>>,
    mut commands: Commands,
) {
    if !features.is_changed() {
        return;
    }
    for (entity, bloom) in &bloom_cameras {
        set_bloom_component(entity, bloom, features.bloom, &mut commands);
    }
    for (entity, hdr) in &cameras {
        set_hdr_component(entity, hdr, features.hdr, &mut commands);
    }
}

fn apply_tonemapping_selection(
    selection: Res<TonemappingSelection>,
    selectors: Query<Entity, With<TonemappingSelectorPanel>>,
    cameras: Query<(Entity, Option<&Tonemapping>), With<Camera>>,
    mut commands: Commands,
) {
    if !selection.is_changed() {
        return;
    }
    for entity in &selectors {
        if let Err(error) = commands.set_tree(entity, tonemapping_selector_tree(selection.index)) {
            error!("failed to replace tonemapping selector tree: {error}");
        }
    }
    for (entity, tonemapping) in &cameras {
        set_tonemapping_component(entity, tonemapping, selection.mode(), &mut commands);
    }
}

fn update_text_coverage_title_bar(
    selection: Res<HdrTextCoverageSelection>,
    cascade_default: Res<HdrTextCoverageBias>,
    mut title_bars: Query<&mut TitleBar>,
) {
    if !selection.is_changed() && !cascade_default.is_changed() {
        return;
    }
    let next_title_bar = batch_validation_title_bar(selection.selected, cascade_default.0);
    for mut title_bar in &mut title_bars {
        *title_bar = next_title_bar.clone();
    }
}

fn set_hdr_component(
    camera: Entity,
    hdr: Option<&Hdr>,
    feature: RenderFeature,
    commands: &mut Commands,
) {
    match (feature, hdr) {
        (RenderFeature::On, None) => {
            commands.entity(camera).insert(Hdr);
        },
        (RenderFeature::Off, Some(_)) => {
            commands.entity(camera).remove::<Hdr>();
        },
        (RenderFeature::On, Some(_)) | (RenderFeature::Off, None) => {},
    }
}

fn set_tonemapping_component(
    camera: Entity,
    current: Option<&Tonemapping>,
    selected: Tonemapping,
    commands: &mut Commands,
) {
    if current != Some(&selected) {
        commands.entity(camera).insert(selected);
    }
}

fn set_bloom_component(
    camera: Entity,
    bloom: Option<&Bloom>,
    feature: RenderFeature,
    commands: &mut Commands,
) {
    match (feature, bloom) {
        (RenderFeature::On, None) => {
            commands.entity(camera).insert(batch_validation_bloom());
        },
        (RenderFeature::Off, Some(_)) => {
            commands.entity(camera).remove::<Bloom>();
        },
        (RenderFeature::On, Some(_)) | (RenderFeature::Off, None) => {},
    }
}

// Repaints the center-left selector (highlighting the chosen mode) and rebuilds
// the Text panel's live alpha case whenever a number key changes the selection.
// The SDF panel needs no rebuild here: `animate_sdf_surface_panel` edits the
// existing material assets, and the fill producer reads those assets each frame.
fn apply_alpha_selection(
    selection: Res<AlphaModeSelection>,
    text_materials: Res<TextPanelMaterialHandles>,
    selectors: Query<Entity, With<AlphaSelectorPanel>>,
    text_panels: Query<Entity, With<BatchValidationTextPanel>>,
    mut commands: Commands,
) {
    if !selection.is_changed() {
        return;
    }
    for entity in &selectors {
        if let Err(error) = commands.set_tree(entity, alpha_selector_tree(selection.index)) {
            error!("failed to replace alpha selector tree: {error}");
        }
    }
    for entity in &text_panels {
        if let Err(error) =
            commands.set_tree(entity, build_text_panel(&text_materials, selection.mode()))
        {
            error!("failed to replace text panel tree: {error}");
        }
    }
}

fn panel_grid_position(index: usize) -> (f32, f32) {
    let column = (index % 2).to_f32();
    let row = (index / 2).to_f32();
    let x = (column - 0.5) * PANEL_STEP_X;
    let y = (0.5 - row).mul_add(PANEL_STEP_Y, PANEL_GRID_CENTER_Y);
    (x, y)
}

fn validation_panel(
    tree: LayoutTree,
    index: usize,
    text_material: Option<Handle<StandardMaterial>>,
    shape_material: Option<Handle<StandardMaterial>>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, hana_diegetic::PanelBuildError> {
    let mut material = default_panel_material();
    material.base_color = if index.is_multiple_of(2) {
        CARD_BG
    } else {
        CARD_BG_ALT
    };
    let builder = DiegeticPanel::world()
        .size(Mm(PANEL_W), Mm(PANEL_H))
        .anchor(Anchor::Center)
        .shadow_casting(ShadowCasting::On);
    let builder = match text_material {
        Some(material) => builder.text_material(material),
        None => builder,
    };
    let builder = match shape_material {
        Some(material) => builder.shape_material(material),
        None => builder,
    };
    if index == 3 {
        builder.with_tree(tree).build()
    } else {
        let material = materials.add(material);
        builder.material(material).with_tree(tree).build()
    }
}

fn spawn_stats_panel(
    mut commands: Commands,
    fonts: Res<FontRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let sections = runtime_stats_sections(None, 0.0, 0.0);
    match diegetic_stats_sections_panel_with_integral_advance(&sections, &fonts, &mut materials) {
        Ok(panel) => {
            commands.spawn((BatchValidationStatsPanel, panel, Transform::default()));
        },
        Err(error) => error!("batch_validation: failed to build stats panel: {error}"),
    }
}

fn spawn_expected_batches_panel(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    match build_expected_batches_panel(None, &ValidationState::Stabilizing, &mut materials) {
        Ok(panel) => {
            commands.spawn((BatchValidationLedgerPanel, panel, Transform::default()));
        },
        Err(error) => error!("batch_validation: failed to build expected-batches panel: {error}"),
    }
}

fn spawn_alpha_selector_panel(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    match build_alpha_selector_panel(ALPHA_DEFAULT_INDEX, &mut materials) {
        Ok(panel) => {
            commands.spawn((AlphaSelectorPanel, panel, Transform::default()));
        },
        Err(error) => error!("batch_validation: failed to build alpha selector panel: {error}"),
    }
}

fn spawn_tonemapping_selector_panel(
    mut commands: Commands,
    selection: Res<TonemappingSelection>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    match build_tonemapping_selector_panel(selection.index, &mut materials) {
        Ok(panel) => {
            commands.spawn((TonemappingSelectorPanel, panel, Transform::default()));
        },
        Err(error) => {
            error!("batch_validation: failed to build tonemapping selector panel: {error}");
        },
    }
}

fn build_alpha_selector_panel(
    index: usize,
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, hana_diegetic::PanelBuildError> {
    let unlit = materials.add(screen_panel_material());
    DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::TopLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(alpha_selector_tree(index))
        .build()
}

fn build_tonemapping_selector_panel(
    index: usize,
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, hana_diegetic::PanelBuildError> {
    let unlit = materials.add(screen_panel_material());
    DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::TopLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(tonemapping_selector_tree(index))
        .build()
}

// Pins the alpha selector's top-right corner just under the stats panel's
// bottom-right, so the diagnostics controls stay grouped on the right edge. Two
// observers cover either spawn order.
fn alpha_selector_stats_attachment() -> PanelAttachment {
    PanelAttachment::new(Anchor::TopRight, Anchor::BottomRight)
        .with_offset(PanelAnchorOffset::new(Px(0.0), Px(4.0)))
}

fn anchor_alpha_selector_when_added(
    trigger: On<Add, AlphaSelectorPanel>,
    stats_panels: Query<Entity, With<BatchValidationStatsPanel>>,
    panel_entities: PanelEntityReader,
    mut attachments: Commands,
) {
    let Ok(stats_panel) = stats_panels.single() else {
        return;
    };
    let (Some(source), Some(target)) = (
        panel_entities.screen(trigger.entity),
        panel_entities.screen(stats_panel),
    ) else {
        return;
    };
    attachments.attach_to_panel(source, target, alpha_selector_stats_attachment());
}

fn anchor_alpha_selector_when_stats_added(
    trigger: On<Add, BatchValidationStatsPanel>,
    selectors: Query<Entity, With<AlphaSelectorPanel>>,
    panel_entities: PanelEntityReader,
    mut attachments: Commands,
) {
    let Some(target) = panel_entities.screen(trigger.entity) else {
        return;
    };
    for selector in &selectors {
        let Some(source) = panel_entities.screen(selector) else {
            continue;
        };
        attachments.attach_to_panel(source, target, alpha_selector_stats_attachment());
    }
}

// Pins the tonemapping selector's top-right corner just under the alpha
// selector's bottom-right, so both diagnostic controls move as one stack.
fn tonemapping_selector_alpha_attachment() -> PanelAttachment {
    PanelAttachment::new(Anchor::TopRight, Anchor::BottomRight)
        .with_offset(PanelAnchorOffset::new(Px(0.0), Px(4.0)))
}

fn anchor_tonemapping_selector_when_added(
    trigger: On<Add, TonemappingSelectorPanel>,
    alpha_selectors: Query<Entity, With<AlphaSelectorPanel>>,
    panel_entities: PanelEntityReader,
    mut attachments: Commands,
) {
    let Ok(alpha_selector) = alpha_selectors.single() else {
        return;
    };
    let (Some(source), Some(target)) = (
        panel_entities.screen(trigger.entity),
        panel_entities.screen(alpha_selector),
    ) else {
        return;
    };
    attachments.attach_to_panel(source, target, tonemapping_selector_alpha_attachment());
}

fn anchor_tonemapping_selector_when_alpha_added(
    trigger: On<Add, AlphaSelectorPanel>,
    tonemapping_selectors: Query<Entity, With<TonemappingSelectorPanel>>,
    panel_entities: PanelEntityReader,
    mut attachments: Commands,
) {
    let Some(target) = panel_entities.screen(trigger.entity) else {
        return;
    };
    for selector in &tonemapping_selectors {
        let Some(source) = panel_entities.screen(selector) else {
            continue;
        };
        attachments.attach_to_panel(source, target, tonemapping_selector_alpha_attachment());
    }
}

// Center-left key legend: a title plus one lettered row per alpha mode. The
// selected row is tinted and sits on a highlight bar so the current choice is
// obvious; pressing the matching letter key (A-G) selects that mode for the SDF
// panel's fills/borders and the Text panel's live alpha case.
fn alpha_selector_tree(selected: usize) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::FIT,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(ALPHA_ROW_GAP),
                |builder| {
                    builder.text(("alpha mode  (A-G)", ledger_title_style()));
                    for (slot, (label, _)) in ALPHA_MODES.iter().enumerate() {
                        let key =
                            ((b'A' + u8::try_from(slot).unwrap_or(u8::MAX)) as char).to_string();
                        selector_row(builder, &key, label, slot == selected, ALPHA_ROW_WIDTH);
                    }
                },
            );
        },
    );
    builder.build()
}

// Tonemapping is a camera component, so this selector makes HDR text comparisons
// possible without restarting the example. Press a number key (1-9) to select a
// row directly, or `T` to cycle the active row.
fn tonemapping_selector_tree(selected: usize) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::FIT,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(TONEMAPPING_ROW_GAP),
                |builder| {
                    builder.text(("tonemapping  (1-9)", ledger_title_style()));
                    for (slot, (label, _)) in TONEMAPPING_MODES.iter().enumerate() {
                        let key = (slot + 1).to_string();
                        selector_row(
                            builder,
                            &key,
                            label,
                            slot == selected,
                            TONEMAPPING_ROW_WIDTH,
                        );
                    }
                },
            );
        },
    );
    builder.build()
}

fn selector_row(builder: &mut LayoutBuilder, key: &str, label: &str, selected: bool, width: f32) {
    let color = if selected { ACCENT_YELLOW } else { TEXT_MUTED };
    let row = El::row()
        .width(Sizing::fixed(width))
        .height(Sizing::FIT)
        .gap(LEDGER_CELL_GAP)
        .padding(Padding::new(3.0, 3.0, 1.0, 1.0))
        .corner_radius(CornerRadius::all(Mm(0.8)))
        .alignment(AlignX::Left, AlignY::Center);
    builder.with(row, |builder| {
        builder.text((key, ledger_cell_style(color)));
        builder.text((label, ledger_cell_style(color)));
    });
}

// Latches the live batch totals: once `[text, shape, sdf]` batch counts hold
// steady for `VALIDATION_STABLE_FRAMES` frames, compares them against the
// predicted totals for the active alpha mode and records match/mismatch. SDF
// material animation changes table values, not batch counts, so it never
// disturbs the latch. Re-arms when the alpha selection changes.
fn validate_batch_counts(
    perf: Res<DiegeticPerfStats>,
    selection: Res<AlphaModeSelection>,
    mut status: ResMut<ValidationStatus>,
) {
    let observed = [
        perf.batch.batches.get(),
        perf.line_batch.batches.get(),
        perf.panel_geometry.sdf_batches.get(),
        perf.image_breakdown.len(),
    ];
    if selection.index != status.last_alpha {
        status.last_alpha = selection.index;
        status.state = ValidationState::Stabilizing;
        status.last_observed = None;
        status.stable_frames = 0;
    }
    // A zero in any family means the renderer has not populated that count yet;
    // hold in `Stabilizing` rather than latch onto a partial frame.
    if observed.contains(&0) {
        status.state = ValidationState::Stabilizing;
        status.last_observed = None;
        status.stable_frames = 0;
        return;
    }
    if status.last_observed == Some(observed) {
        status.stable_frames += 1;
    } else {
        status.last_observed = Some(observed);
        status.stable_frames = 0;
        status.state = ValidationState::Stabilizing;
    }
    if status.stable_frames < VALIDATION_STABLE_FRAMES {
        return;
    }
    let failures = batch_invariant_failures(&perf);
    status.state = if failures.is_empty() {
        ValidationState::Match
    } else {
        ValidationState::Mismatch { failures }
    };
}

fn update_diagnostic_panels(
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    diegetic_perf: Res<DiegeticPerfStats>,
    selection: Res<AlphaModeSelection>,
    validation: Res<ValidationStatus>,
    stats_panels: Query<Entity, With<BatchValidationStatsPanel>>,
    ledger_panels: Query<Entity, With<BatchValidationLedgerPanel>>,
    mut last: ResMut<LastDisplayedDiagnostics>,
    mut commands: Commands,
    mut timer: Local<Option<Timer>>,
    fonts: Res<FontRegistry>,
) {
    let timer = timer.get_or_insert_with(|| {
        Timer::from_seconds(DIAGNOSTIC_UPDATE_INTERVAL, TimerMode::Repeating)
    });
    timer.tick(time.delta());
    if !timer.just_finished() {
        return;
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(Diagnostic::smoothed);
    let stats_sections =
        runtime_stats_sections(Some(&diegetic_perf), fps.unwrap_or(0.0), time.delta_secs());
    let key = format!(
        "{}|{}",
        runtime_stats_key(&stats_sections),
        ledger_key(&diegetic_perf, &validation.state, selection.index)
    );
    if key == last.key {
        return;
    }
    last.key = key;
    for panel in &stats_panels {
        if let Err(error) = commands.set_tree(
            panel,
            diegetic_stats_sections_tree_with_integral_advance(&stats_sections, &fonts),
        ) {
            error!("failed to replace stats panel tree: {error}");
        }
    }
    for panel in &ledger_panels {
        if let Err(error) = commands.set_tree(
            panel,
            expected_batches_tree(Some(&diegetic_perf), &validation.state),
        ) {
            error!("failed to replace batch ledger panel tree: {error}");
        }
    }
}

fn ledger_key(perf: &DiegeticPerfStats, state: &ValidationState, alpha_index: usize) -> String {
    let mut key = format!("alpha={alpha_index}|val={}|", validation_key(state));
    for family in family_breakdowns(perf) {
        key.push_str(family.label);
        key.push('=');
        key.push_str(&family.batch_count.to_string());
        key.push('/');
        key.push_str(&family.record_total.to_string());
        key.push('|');
        for batch in family.batches {
            key.push_str(&batch_reason(batch));
            key.push(':');
            key.push_str(&batch.record_count.to_string());
            key.push('|');
        }
    }
    let table = perf.material_table;
    let _ = write!(
        key,
        "mat={}/{}/{}/{}/{}/{}",
        table.rows,
        table.capacity,
        table.upload_bytes,
        table.freeze_us,
        table.upload_us,
        table.allocations
    );
    key
}

// Short discriminant for the dedup key so the ledger rebuilds when the
// validation outcome changes even after the live counts stop moving.
fn validation_key(state: &ValidationState) -> String {
    match state {
        ValidationState::Stabilizing => "stabilizing".to_owned(),
        ValidationState::Match => "match".to_owned(),
        ValidationState::Mismatch { failures } => format!("mismatch:{failures:?}"),
    }
}

fn runtime_stats_sections(
    _perf: Option<&DiegeticPerfStats>,
    fps: f64,
    frame_secs: f32,
) -> Vec<StatsPanelSection> {
    let frame_ms = frame_secs * 1000.0;
    vec![StatsPanelSection::untitled([
        StatsPanelRow::new(
            "profile",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        ),
        StatsPanelRow::new("fps", format!("{fps:.0} / {frame_ms:.2} ms")),
    ])]
}

fn runtime_stats_key(sections: &[StatsPanelSection]) -> String {
    let mut key = String::new();
    for section in sections {
        key.push_str(&section.title);
        key.push('|');
        for row in &section.rows {
            key.push_str(&row.label);
            key.push('=');
            key.push_str(&row.value);
            key.push('|');
            for detail in &row.details {
                key.push_str(detail);
                key.push('|');
            }
        }
    }
    key
}

fn build_expected_batches_panel(
    perf: Option<&DiegeticPerfStats>,
    status: &ValidationState,
    materials: &mut Assets<StandardMaterial>,
) -> Result<DiegeticPanel, hana_diegetic::PanelBuildError> {
    let unlit = materials.add(screen_panel_material());
    DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(expected_batches_tree(perf, status))
        .build()
}

// The bottom-left batch diagnostic, entirely live: a family-totals table
// (draws / records / records-per-draw for text, shape, sdf), then a per-batch
// breakdown listing why each family split, and a validation line latched by
// `validate_batch_counts` against the renderer's own decomposition.
fn expected_batches_tree(perf: Option<&DiegeticPerfStats>, status: &ValidationState) -> LayoutTree {
    let perf = perf.cloned().unwrap_or_default();
    let families = family_breakdowns(&perf);
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::FIT,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(LEDGER_ROW_GAP),
                |builder| {
                    ledger_batch_section(builder, &perf, &families);

                    // Per-frame cost of rebuilding and re-uploading the whole
                    // material table. freeze/upload us are paid every frame even
                    // when no material changed — the work a durable slot table
                    // would skip on a no-change frame.
                    ledger_separator(builder);
                    ledger_material_section(builder, &perf);

                    ledger_separator(builder);
                    // Every state stays a single short line so the FIT-height
                    // panel keeps a constant height as the latch flips — a longer
                    // message would wrap and bump the bottom-anchored panel.
                    let (status_text, status_color) = match status {
                        ValidationState::Stabilizing => ("stabilizing…".to_owned(), TEXT_MUTED),
                        ValidationState::Match => ("records routed: ok".to_owned(), ACCENT_GREEN),
                        ValidationState::Mismatch { failures } => {
                            (format!("mismatch: {} fault(s)", failures.len()), ACCENT_RED)
                        },
                    };
                    builder.text((status_text, ledger_cell_style(status_color)));
                },
            );
        },
    );
    builder.build()
}

fn ledger_batch_section(
    builder: &mut LayoutBuilder,
    perf: &DiegeticPerfStats,
    families: &[FamilyBreakdown<'_>; 4],
) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(LEDGER_ROW_GAP),
        |builder| {
            ledger_width_anchor(builder);
            builder.text(("batch validation", ledger_title_style()));
            ledger_row(
                builder,
                "",
                TEXT_MUTED,
                [
                    "text".to_owned(),
                    "shape".to_owned(),
                    "sdf".to_owned(),
                    "image".to_owned(),
                ],
            );
            ledger_row(
                builder,
                "draws",
                TEXT_MAIN,
                families
                    .each_ref()
                    .map(|family| family.batch_count.to_string()),
            );
            ledger_row(
                builder,
                "records",
                TEXT_MAIN,
                families
                    .each_ref()
                    .map(|family| family.record_total.to_string()),
            );
            ledger_row(
                builder,
                "records/draw",
                TEXT_MAIN,
                families.each_ref().map(records_per_draw),
            );
            ledger_row(
                builder,
                "uploads",
                TEXT_MAIN,
                [
                    (perf.batch.instance_uploads.get() + perf.batch.run_table_uploads.get())
                        .to_string(),
                    perf.line_batch.uploads.to_string(),
                    perf.panel_geometry.sdf_uploads.to_string(),
                    // Image batches expose no per-frame upload counter.
                    "-".to_owned(),
                ],
            );
            ledger_separator(builder);
            ledger_record_explainer(builder, perf);
            ledger_separator(builder);
            ledger_upload_explainer(builder, perf);

            for family in families {
                ledger_separator(builder);
                // Header row: each row below is one draw, labeled by its
                // split reason; the right column is that draw's record
                // count, so the column sums to the family record total.
                ledger_breakdown_header(builder, &format!("{} draws", family.label), family.color);
                for batch in family.batches {
                    ledger_kv_row(
                        builder,
                        &batch_reason(batch),
                        family.color,
                        batch.record_count.to_string(),
                    );
                }
            }
        },
    );
}

fn ledger_material_section(builder: &mut LayoutBuilder, perf: &DiegeticPerfStats) {
    let table = perf.material_table;
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(LEDGER_ROW_GAP),
        |builder| {
            builder.text(("material table", ledger_cell_style(TEXT_MAIN)));
            ledger_plain_kv_row(builder, "rows", TEXT_MUTED, table.rows.to_string());
            ledger_plain_kv_row(builder, "capacity", TEXT_MUTED, table.capacity.to_string());
            ledger_plain_kv_row(builder, "bytes", TEXT_MUTED, table.upload_bytes.to_string());
            ledger_plain_kv_row(
                builder,
                "freeze time us",
                TEXT_MUTED,
                table.freeze_us.to_string(),
            );
            ledger_plain_kv_row(
                builder,
                "upload time us",
                TEXT_MUTED,
                table.upload_us.to_string(),
            );
            ledger_plain_kv_row(
                builder,
                "reallocs",
                TEXT_MUTED,
                table.allocations.to_string(),
            );
        },
    );
}

fn ledger_record_explainer(builder: &mut LayoutBuilder, perf: &DiegeticPerfStats) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(0.5),
        |builder| {
            builder.text(("record detail", ledger_cell_style(TEXT_MUTED)));
            ledger_detail_kv_row(
                builder,
                "glyph records",
                ACCENT_GREEN,
                perf.batch.glyph_records.get(),
            );
            ledger_detail_kv_row(
                builder,
                "text runs (not records)",
                TEXT_MUTED,
                perf.batch.runs.get(),
            );
            ledger_detail_kv_row(
                builder,
                "shape records",
                ACCENT_YELLOW,
                perf.line_batch.records.get(),
            );
            ledger_detail_kv_row(
                builder,
                "sdf records",
                ACCENT_BLUE,
                perf.panel_geometry.sdf_records.get(),
            );
            ledger_detail_kv_row(
                builder,
                "image records",
                ACCENT_RED,
                perf.image_breakdown
                    .iter()
                    .map(|batch| batch.record_count.get())
                    .sum(),
            );
        },
    );
}

fn ledger_upload_explainer(builder: &mut LayoutBuilder, perf: &DiegeticPerfStats) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(0.5),
        |builder| {
            ledger_full_note(builder, "upload detail - record buffers");
            ledger_full_note(builder, "changed materials or instructions");
            ledger_detail_kv_row(
                builder,
                "text record buffers",
                ACCENT_GREEN,
                perf.batch.instance_uploads.get() + perf.batch.run_table_uploads.get(),
            );
            ledger_detail_kv_row(
                builder,
                "shape record buffers",
                ACCENT_YELLOW,
                perf.line_batch.uploads.get(),
            );
            ledger_detail_kv_row(
                builder,
                "sdf record buffers",
                ACCENT_BLUE,
                perf.panel_geometry.sdf_uploads.get(),
            );
        },
    );
}

// One table row: a fixed-width left label cell plus three right-aligned numeric
// cells colored by family (text/shape/sdf).
// A GROW spacer between the left label and the right-aligned number columns:
// the label hugs its content on the left, the spacer eats the slack, and the
// fixed-width number cells share a right edge across every row.
fn ledger_spacer(builder: &mut LayoutBuilder) {
    builder.with(
        El::new().width(Sizing::GROW).height(Sizing::FIT),
        |_builder| {},
    );
}

fn ledger_width_anchor(builder: &mut LayoutBuilder) {
    builder.with(
        El::row().width(Sizing::FIT).height(Sizing::fixed(0.0)),
        |builder| {
            ledger_hidden_measure_cell(builder, LEDGER_BREAKDOWN_LABEL_MEASURE);
            ledger_hidden_measure_cell(builder, LEDGER_BREAKDOWN_MIN_SPACER_MEASURE);
            ledger_hidden_measure_cell(builder, LEDGER_BREAKDOWN_VALUE_MEASURE);
        },
    );
}

fn ledger_hidden_measure_cell(builder: &mut LayoutBuilder, measure: &str) {
    builder.text(
        Text::new("", ledger_cell_style(TEXT_MUTED))
            .layout(El::new().height(Sizing::fixed(0.0)))
            .measure_as(measure),
    );
}

fn ledger_min_spacer(builder: &mut LayoutBuilder) {
    ledger_hidden_measure_cell(builder, LEDGER_BREAKDOWN_MIN_SPACER_MEASURE);
    ledger_spacer(builder);
}

fn ledger_num_cell(builder: &mut LayoutBuilder, value: String, color: Color) {
    builder.with(
        El::new()
            .width(Sizing::fixed(LEDGER_NUM_WIDTH))
            .height(Sizing::FIT)
            .alignment(AlignX::Right, AlignY::Center),
        |builder| {
            builder.text((value, ledger_cell_style(color)));
        },
    );
}

// The three-column family row: a left label, a GROW spacer, then text / shape /
// sdf numbers right-aligned at the panel's padding edge.
fn ledger_row(builder: &mut LayoutBuilder, label: &str, label_color: Color, cells: [String; 4]) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(LEDGER_CELL_GAP)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text((label, ledger_cell_style(label_color)));
                },
            );
            ledger_spacer(builder);
            for (cell, color) in cells.into_iter().zip(LEDGER_FAMILY_COLORS) {
                ledger_num_cell(builder, cell, color);
            }
        },
    );
}

// Breakdown section header fills the width established by the data rows: the
// left label is natural, then a spacer pushes `records` to the same right edge
// as the counts. `records` must not be part of the measured data-row width.
fn ledger_breakdown_header(builder: &mut LayoutBuilder, label: &str, color: Color) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text((label, ledger_cell_style(color)));
                },
            );
            ledger_min_spacer(builder);
            builder.with(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Right, AlignY::Center),
                |builder| {
                    builder.text(("records", ledger_cell_style(color)));
                },
            );
        },
    );
}

// One breakdown row: a natural reason label plus a record count pushed to the
// anchored ledger edge.
fn ledger_kv_row(builder: &mut LayoutBuilder, label: &str, color: Color, value: String) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text((label, ledger_cell_style(color)));
                },
            );
            ledger_min_spacer(builder);
            builder.text((value, ledger_cell_style(color)));
        },
    );
}

fn ledger_plain_kv_row(builder: &mut LayoutBuilder, label: &str, color: Color, value: String) {
    ledger_single_value_row(builder, label, color, value);
}

fn ledger_detail_kv_row(builder: &mut LayoutBuilder, label: &str, color: Color, value: usize) {
    ledger_single_value_row(builder, label, color, value.to_string());
}

fn ledger_single_value_row(builder: &mut LayoutBuilder, label: &str, color: Color, value: String) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text((label, ledger_cell_style(color)));
                },
            );
            ledger_min_spacer(builder);
            builder.text((value, ledger_cell_style(color)));
        },
    );
}

fn ledger_full_note(builder: &mut LayoutBuilder, text: &str) {
    builder.with(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text((text, ledger_cell_style(TEXT_MUTED)));
        },
    );
}

// Records-per-draw compression ratio for one family, the headline number for
// "batching is working". Dashed when no draws have landed yet.
fn records_per_draw(family: &FamilyBreakdown) -> String {
    if family.batch_count == 0 {
        "—".to_owned()
    } else {
        format!(
            "{:.1}",
            family.record_total.to_f32() / family.batch_count.to_f32()
        )
    }
}

fn ledger_separator(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(1.0))
            .background(LEDGER_SEPARATOR_COLOR),
        |_builder| {},
    );
}

fn ledger_title_style() -> TextStyle {
    TextStyle::new(LEDGER_TITLE_FONT_SIZE)
        .bold()
        .with_color(TEXT_MAIN)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn ledger_cell_style(color: Color) -> TextStyle {
    TextStyle::new(LEDGER_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

// Four SDF fills laid out 2x2. The panel-default / metallic / emissive cards
// differ only in StandardMaterial table values, so post-material-table they
// share one SDF batch. The image card carries a
// `base_color_texture`, a batch-compatibility splitter, so it forms its own
// batch — the SDF column's predicted 2 batches in the expected-batches ledger.
fn build_sdf_surface_panel(materials: &SdfSurfaceMaterialHandles) -> LayoutTree {
    let mut builder = panel_root();
    panel_header(
        &mut builder,
        "SDF fills + borders",
        "Cards with plain StandardMaterial f32 values share one untextured record; \
         the textured card splits into its own record.",
    );
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(ROW_GAP),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .gap(ROW_GAP),
                |builder| {
                    sdf_panel_default_card(
                        builder,
                        "panel default",
                        "builder material",
                        ACCENT_BLUE,
                        materials.panel_default.clone(),
                    );
                    sdf_fill_card(
                        builder,
                        "El material",
                        "base color animates",
                        ACCENT_GREEN,
                        materials.metallic.clone(),
                    );
                },
            );
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .gap(ROW_GAP),
                |builder| {
                    sdf_emissive_fill_card(
                        builder,
                        "El material",
                        "emissive value",
                        ACCENT_YELLOW,
                        materials.emissive.clone(),
                        materials.emissive_text.clone(),
                    );
                    sdf_fill_card(
                        builder,
                        "image",
                        "texture splits",
                        ACCENT_RED,
                        materials.image.clone(),
                    );
                },
            );
        },
    );
    builder.build()
}

// Stamps the selected alpha mode onto a card material so the SDF fill and its
// border (which inherits the fill material) both render in that mode.
const fn with_alpha(mut material: StandardMaterial, alpha: AlphaMode) -> StandardMaterial {
    material.alpha_mode = alpha;
    material
}

fn text_panel_default_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = TEXT_MAIN;
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn text_emissive_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::srgb(0.08, 0.06, 0.02);
    material.emissive = EMISSIVE_WARM.to_linear() * TEXT_EMISSIVE_GAIN;
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn text_metallic_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = GLINT;
    material.metallic = 1.0;
    material.perceptual_roughness = 0.26;
    material.reflectance = 0.8;
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn text_texture_material(image: Handle<Image>) -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::WHITE;
    material.base_color_texture = Some(image);
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn shape_panel_default_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::srgb(0.08, 0.10, 0.13);
    material.metallic = 0.0;
    material.perceptual_roughness = 0.42;
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn shape_local_line_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::srgb(0.05, 0.06, 0.02);
    material.emissive = EMISSIVE_WARM.to_linear();
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn shape_style_line_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = GLINT;
    material.metallic = 1.0;
    material.perceptual_roughness = 0.24;
    material.reflectance = 0.8;
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn shape_circle_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::srgb(0.12, 0.08, 0.18);
    material.perceptual_roughness = 0.72;
    material.reflectance = 0.28;
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn shape_alpha_split_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = ACCENT_RED;
    material.alpha_mode = AlphaMode::Add;
    material
}

fn shape_texture_material(image: Handle<Image>) -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::WHITE;
    material.base_color_texture = Some(image);
    material.alpha_mode = AlphaMode::Blend;
    material
}

fn build_text_panel(materials: &TextPanelMaterialHandles, alpha: AlphaMode) -> LayoutTree {
    let mut builder = panel_root();
    panel_header(
        &mut builder,
        "Text",
        "Text runs with matching values share one record; a different style key or \
         a texture splits them into separate records.",
    );
    builder.with(
        // GROW height: claim the panel's leftover vertical space below the
        // header so the two group columns (also GROW height) stretch to a
        // matching height instead of each hugging its own content.
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(ROW_GAP),
        |builder| {
            shared_text_material_group(builder, materials);
            divergent_group(builder, materials, alpha);
        },
    );
    builder.build()
}

fn build_shape_panel(materials: &ShapePanelMaterialHandles) -> LayoutTree {
    let mut builder = panel_root();
    panel_header(
        &mut builder,
        "Analytic shapes",
        "Shapes that share a material batch into one shape record; a textured or \
         differing material splits into its own record.",
    );
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(ROW_GAP),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .gap(ROW_GAP),
                |builder| {
                    shape_group_card(
                        builder,
                        materials,
                        0,
                        "panel default",
                        "shape_material",
                        ACCENT_BLUE,
                    );
                    shape_group_card(
                        builder,
                        materials,
                        1,
                        "line local",
                        "PanelLine::material",
                        ACCENT_GREEN,
                    );
                    shape_group_card(
                        builder,
                        materials,
                        2,
                        "line style",
                        "LineStyle::material",
                        ACCENT_YELLOW,
                    );
                },
            );
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .gap(ROW_GAP),
                |builder| {
                    shape_group_card(
                        builder,
                        materials,
                        3,
                        "circle local",
                        "PanelCircle::material",
                        ACCENT_BLUE,
                    );
                    shape_group_card(builder, materials, 4, "alpha split", "Add mode", ACCENT_RED);
                    shape_group_card(
                        builder,
                        materials,
                        5,
                        "texture split",
                        "texture resource splits",
                        ACCENT_GREEN,
                    );
                },
            );
        },
    );
    builder.build()
}

// Draws real images through the batch path (`LayoutBuilder::image`). Every card
// samples the same texture, so post-key they route into ONE image batch — the
// tint varies per record but stays in-batch. This is the runtime confirmation
// that batched images render before the legacy entity path is retired.
fn build_image_panel(image: Handle<Image>) -> LayoutTree {
    let mut builder = panel_root();
    panel_header(
        &mut builder,
        "Image batches",
        "Images sampling one texture share a single batch; the per-record tint \
         varies without splitting the draw.",
    );
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(ROW_GAP),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .gap(ROW_GAP),
                |builder| {
                    image_card(
                        builder,
                        image.clone(),
                        Color::WHITE,
                        "plain",
                        "full texture",
                        ImageCardFrame::ClippedBorder,
                    );
                    image_card(
                        builder,
                        image.clone(),
                        ACCENT_GREEN,
                        "green tint",
                        "same batch",
                        ImageCardFrame::FilledClippedBorder,
                    );
                },
            );
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .gap(ROW_GAP),
                |builder| {
                    image_card(
                        builder,
                        image.clone(),
                        ACCENT_BLUE,
                        "blue tint",
                        "same batch",
                        ImageCardFrame::Direct,
                    );
                    image_card(
                        builder,
                        image,
                        ACCENT_RED,
                        "red tint",
                        "one image batch",
                        ImageCardFrame::Direct,
                    );
                },
            );
        },
    );
    builder.build()
}

#[derive(Clone, Copy)]
enum ImageCardFrame {
    Direct,
    ClippedBorder,
    FilledClippedBorder,
}

// One image card: a bordered frame filled by an image leaf, captioned below.
fn image_card(
    builder: &mut LayoutBuilder,
    image: Handle<Image>,
    tint: Color,
    label: &str,
    caption: &str,
    frame: ImageCardFrame,
) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(1.0)
            .padding(Padding::all(2.0))
            .border(Border::all(Mm(0.3), ACCENT_RED))
            .corner_radius(CornerRadius::all(Mm(1.4)))
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            match frame {
                ImageCardFrame::Direct => {
                    builder.image(
                        El::new().width(Sizing::GROW).height(Sizing::GROW),
                        image,
                        tint,
                    );
                },
                ImageCardFrame::ClippedBorder => {
                    builder.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .clip()
                            .border(Border::all(IMAGE_CARD_CLIPPED_BORDER_WIDTH, ACCENT_GREEN))
                            .corner_radius(CornerRadius::all(IMAGE_CARD_CLIPPED_BORDER_RADIUS)),
                        |builder| {
                            builder.image(
                                El::new().width(Sizing::GROW).height(Sizing::GROW),
                                image,
                                tint,
                            );
                        },
                    );
                },
                ImageCardFrame::FilledClippedBorder => {
                    builder.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .clip()
                            .background(IMAGE_CARD_CLIPPED_BORDER_FILL)
                            .border(Border::all(IMAGE_CARD_CLIPPED_BORDER_WIDTH, ACCENT_GREEN))
                            .corner_radius(CornerRadius::all(IMAGE_CARD_CLIPPED_BORDER_RADIUS)),
                        |builder| {
                            builder.image(
                                El::new().width(Sizing::GROW).height(Sizing::GROW),
                                image,
                                tint,
                            );
                        },
                    );
                },
            }
            builder.text((label, swatch_style(ACCENT_RED)));
            builder.text((caption, small_style(TEXT_MUTED)));
        },
    );
}

fn panel_root() -> LayoutBuilder {
    LayoutBuilder::with_root(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(ROW_GAP)
            .padding(Padding::all(PANEL_PAD))
            .border(Border::all(Mm(0.45), CARD_BORDER))
            .corner_radius(CornerRadius::all(CARD_RADIUS)),
    )
}

fn panel_header(builder: &mut LayoutBuilder, title: &str, note: &str) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(1.0)
            .padding(Padding::new(0.0, 0.0, 0.0, 2.0))
            .alignment(AlignX::Left, AlignY::Top),
        |builder| {
            builder.text((title, title_style()));
            // Full-width note beneath the title: GROW bounds the run to the panel
            // content width so it wraps per word across as many lines as it needs.
            builder.with(
                El::new().width(Sizing::GROW).height(Sizing::FIT),
                |builder| {
                    builder.text(Text::new(note, subtitle_style(TEXT_MUTED)).wrap(TextWrap::Words));
                },
            );
        },
    );
}

fn shared_text_material_group(builder: &mut LayoutBuilder, materials: &TextPanelMaterialHandles) {
    builder.with(
        // GROW height so both group columns fill the row and share a height; the
        // FIT cases inside keep their content from inflating the panel.
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(MATERIAL_GROUP_GAP)
            .padding(Padding::all(Mm(1.5)))
            .background(Color::srgba(0.02, 0.03, 0.04, 0.30))
            .border(Border::all(Mm(0.3), ACCENT_GREEN))
            .corner_radius(CornerRadius::all(Mm(1.3))),
        |builder| {
            material_group_header(builder, "Shared group", "varies table values");
            material_group_spacer(builder);
            material_case_block_with_style(
                builder,
                "style color",
                "cool blue",
                material_value_style(ACCENT_BLUE),
            );
            material_group_spacer(builder);
            material_case_block_with_style(
                builder,
                "local material",
                "emissive value",
                material_value_style(EMISSIVE_WARM).with_material(materials.emissive.clone()),
            );
            material_group_spacer(builder);
            material_case_block_with_style(
                builder,
                "local material",
                "metallic glint",
                material_value_style(GLINT).with_material(materials.metallic.clone()),
            );
        },
    );
}

fn material_group_header(builder: &mut LayoutBuilder, title: &str, subtitle: &str) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(MATERIAL_CASE_GAP),
        |builder| {
            builder.text((title, material_group_title_style()));
            builder.text((subtitle, material_caption_style(TEXT_MUTED)));
        },
    );
}

fn material_group_spacer(builder: &mut LayoutBuilder) {
    builder.with(
        El::new().width(Sizing::GROW).height(Sizing::GROW),
        |_builder| {},
    );
}

// Each case stacks a muted caption above its value. The value owns the full
// group width and wraps, so the long divergent-group strings cannot clip at the
// card edge the way a fixed-label-width row did.
fn material_case_block_with_style(
    builder: &mut LayoutBuilder,
    label: &str,
    value: &str,
    style: TextStyle,
) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(MATERIAL_CASE_GAP)
            .padding(Padding::xy(MATERIAL_CASE_PAD_X, MATERIAL_CASE_PAD_Y))
            .corner_radius(CornerRadius::all(Mm(1.5)))
            .background(MATERIAL_CASE_BG)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text((label, material_caption_style(TEXT_MUTED)));
            builder.text((value, style));
        },
    );
}

// The Divergent group authors text runs that actually carry the batch splitters,
// so each one forms its own text batch (observable in `batch.batches` / the
// ledger's `actual` text count). The texture row splits by resource; the
// cull-mode rows split by sidedness; the live alpha row splits only when the
// selector leaves the panel-default Blend mode.
fn divergent_group(
    builder: &mut LayoutBuilder,
    materials: &TextPanelMaterialHandles,
    alpha: AlphaMode,
) {
    builder.with(
        // GROW height to match the Shared group; see `shared_text_material_group`.
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(MATERIAL_GROUP_GAP)
            .padding(Padding::all(Mm(1.5)))
            .background(Color::srgba(0.02, 0.03, 0.04, 0.30))
            .border(Border::all(Mm(0.3), ACCENT_RED))
            .corner_radius(CornerRadius::all(Mm(1.3))),
        |builder| {
            material_group_header(builder, "Divergent group", "splits compatibility");
            material_group_spacer(builder);
            divergent_alpha_case(builder, alpha);
            material_group_spacer(builder);
            divergent_texture_case(builder, materials);
            material_group_spacer(builder);
            divergent_cull_case(builder);
        },
    );
}

// Live alpha-mode case: one run rendered in the mode the center-left selector
// currently holds, so the text material's alpha tracks the same selection that
// drives the SDF panel. Sits on a light cell so `Multiply` reads as a tint
// rather than vanishing into the dark panel.
fn divergent_alpha_case(builder: &mut LayoutBuilder, alpha: AlphaMode) {
    let caption_style = material_caption_style(ALPHA_CELL_CAPTION);
    let value_style = material_value_style(ALPHA_CELL_INK).with_alpha_mode(alpha);
    // Previous HDR-path shader compensation, replaced by LDR precompose:
    // let caption_style =
    //     caption_style.with_hdr_text_coverage_bias(ALPHA_CELL_HDR_TEXT_COVERAGE_BIAS);
    // let value_style = value_style.with_hdr_text_coverage_bias(ALPHA_CELL_HDR_TEXT_COVERAGE_BIAS);

    divergent_text_precomposed_case_shell(
        builder,
        "alpha mode (precomposed text)",
        ALPHA_CELL_BG,
        caption_style,
        |builder| {
            builder.text(Text::new(alpha_mode_label(alpha), value_style).precompose_ldr());
        },
    );
}

// Short display name for an alpha mode, matching the selector's row labels.
fn alpha_mode_label(alpha: AlphaMode) -> &'static str {
    ALPHA_MODES
        .iter()
        .find(|(_, mode)| *mode == alpha)
        .map_or("custom", |(label, _)| *label)
}

// Texture-backed text samples the material image across the run-local box UV,
// and the texture resource forms its own compatibility batch.
fn divergent_texture_case(builder: &mut LayoutBuilder, materials: &TextPanelMaterialHandles) {
    divergent_case_shell(
        builder,
        "texture",
        MATERIAL_CASE_BG,
        material_caption_style(TEXT_MUTED),
        |builder| {
            builder.text((
                "image glyphs",
                material_value_style(TEXT_MAIN).with_material(materials.texture.clone()),
            ));
        },
    );
}

// Cull-mode split: three runs, one per `Sidedness`. `FrontOnly` and `BackOnly`
// each form their own text batch; `BothSides` matches the panel default and
// joins the shared batch. `BackOnly` culls front faces, so it is invisible from
// the front camera and only legible when the orbit camera swings behind.
fn divergent_cull_case(builder: &mut LayoutBuilder) {
    divergent_case_shell(
        builder,
        "cull mode",
        MATERIAL_CASE_BG,
        material_caption_style(TEXT_MUTED),
        |builder| {
            builder.text((
                "FrontOnly",
                material_value_style(ACCENT_RED).with_sidedness(Sidedness::FrontOnly),
            ));
            builder.text((
                "BackOnly",
                material_value_style(ACCENT_RED).with_sidedness(Sidedness::BackOnly),
            ));
            builder.text((
                "BothSides",
                material_value_style(ACCENT_RED).with_sidedness(Sidedness::BothSides),
            ));
        },
    );
}

// A divergent case: a captioned cell whose value runs wrap in a row, so each
// run keeps its own `TextStyle` (and thus its own batch key) instead of being
// merged into one descriptive string.
fn divergent_case_shell(
    builder: &mut LayoutBuilder,
    caption: &str,
    cell_bg: Color,
    caption_style: TextStyle,
    values: impl FnOnce(&mut LayoutBuilder),
) {
    divergent_case_shell_impl(builder, caption, cell_bg, caption_style, false, values);
}

fn divergent_text_precomposed_case_shell(
    builder: &mut LayoutBuilder,
    caption: &str,
    cell_bg: Color,
    caption_style: TextStyle,
    values: impl FnOnce(&mut LayoutBuilder),
) {
    divergent_case_shell_impl(builder, caption, cell_bg, caption_style, true, values);
}

fn divergent_case_shell_impl(
    builder: &mut LayoutBuilder,
    caption: &str,
    cell_bg: Color,
    caption_style: TextStyle,
    precompose_text_ldr: bool,
    values: impl FnOnce(&mut LayoutBuilder),
) {
    let shell = El::column()
        .width(Sizing::GROW)
        .height(Sizing::FIT)
        .gap(MATERIAL_CASE_GAP)
        .padding(Padding::xy(MATERIAL_CASE_PAD_X, MATERIAL_CASE_PAD_Y))
        .corner_radius(CornerRadius::all(Mm(1.5)))
        .background(cell_bg)
        .alignment(AlignX::Left, AlignY::Center);

    builder.with(shell, |builder| {
        let caption_text = Text::new(caption, caption_style);
        let caption_text = if precompose_text_ldr {
            caption_text.precompose_ldr()
        } else {
            caption_text
        };
        builder.text(caption_text);
        // FIT width, not GROW: a GROW-width row seeds to ~0 width in the
        // bottom-up fit pass, forcing its text runs to wrap per word and
        // measure tall, which balloons the case height. FIT measures each
        // run at its intrinsic single-line width and packs them left.
        builder.with(
            El::row()
                .width(Sizing::FIT)
                .height(Sizing::FIT)
                .gap(MATERIAL_VALUE_GAP),
            values,
        );
    });
}

// One SDF fill: an El with a material background and a border (rendered in a
// single quad), captioned with its material name and role. The label/caption
// column carries no background, so it adds no SDF surface — only the outer El.
fn sdf_fill_card(
    builder: &mut LayoutBuilder,
    label: &str,
    caption: &str,
    accent: Color,
    material: Handle<StandardMaterial>,
) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(2.0))
            .material(material)
            .border(Border::all(Mm(0.3), accent))
            .corner_radius(CornerRadius::all(Mm(1.4)))
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(1.0)
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    builder.text((label, swatch_style(accent)));
                    builder.text((caption, small_style(TEXT_MUTED)));
                },
            );
        },
    );
}

fn sdf_emissive_fill_card(
    builder: &mut LayoutBuilder,
    label: &str,
    caption: &str,
    accent: Color,
    material: Handle<StandardMaterial>,
    text_material: Handle<StandardMaterial>,
) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(2.0))
            .material(material)
            .border(Border::all(Mm(0.3), accent))
            .corner_radius(CornerRadius::all(Mm(1.4)))
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            let label_style = swatch_style(SDF_EMISSIVE_TEXT_INK)
                .with_material(text_material.clone())
                .with_hdr_text_coverage_bias(2.0);
            let caption_style = small_style(SDF_EMISSIVE_TEXT_INK)
                .with_material(text_material)
                .with_hdr_text_coverage_bias(2.0);
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(1.0)
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    builder.text((label, label_style));
                    builder.text((caption, caption_style));
                },
            );
        },
    );
}

fn sdf_panel_default_card(
    builder: &mut LayoutBuilder,
    label: &str,
    caption: &str,
    accent: Color,
    material: Handle<StandardMaterial>,
) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(2.0))
            .material(material)
            .border(Border::all(Mm(0.3), accent))
            .corner_radius(CornerRadius::all(Mm(1.4)))
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(1.0)
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    builder.text((
                        label,
                        swatch_style(Color::BLACK).with_hdr_text_coverage_bias(2.0),
                    ));
                    builder.text((
                        caption,
                        small_style(Color::BLACK).with_hdr_text_coverage_bias(2.0),
                    ));
                },
            );
        },
    );
}

fn panel_default_card_material(alpha: AlphaMode) -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = ACCENT_BLUE;
    material.alpha_mode = alpha;
    material
}

fn replace_material_asset(
    materials: &mut Assets<StandardMaterial>,
    handle: &Handle<StandardMaterial>,
    material: StandardMaterial,
) {
    if let Some(mut existing) = materials.get_mut(handle) {
        *existing = material;
    }
}

// Matte dielectric SDF fill. Differs from the metallic and emissive cards only
// in StandardMaterial table values, so the three share a batch.
fn animated_unit(phase: f32, offset: f32) -> f32 { (phase + offset).sin().mul_add(0.5, 0.5) }

// Brushed-metal SDF fill whose base color also sweeps each frame, so the fill
// visibly cycles while staying batch-compatible with the panel-default and
// emissive cards: base color, metallic, roughness, and reflectance are all
// material-table values, never batch splitters. Keeps `default_panel_material`'s
// double-sided / no-cull setup so the diegetic panel renders from both faces.
fn metallic_glint_material(phase: f32) -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::srgb(
        animated_unit(phase, SDF_ANIMATION_RED_OFFSET).mul_add(0.25, 0.18),
        animated_unit(phase, SDF_ANIMATION_GREEN_OFFSET).mul_add(0.30, 0.28),
        animated_unit(phase, 0.0).mul_add(0.30, 0.50),
    );
    material.metallic = 1.0;
    material.perceptual_roughness =
        animated_unit(phase, SDF_ANIMATION_GREEN_OFFSET).mul_add(0.35, 0.35);
    material.reflectance = animated_unit(phase, SDF_ANIMATION_RED_OFFSET).mul_add(0.2, 0.7);
    material
}

// Emissive SDF fill: a warm self-lit readout color. `emissive` is a table value,
// so this stays batch-compatible with the colored and metallic cards.
fn emissive_fill_material(phase: f32) -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::srgb(0.06, 0.05, 0.03);
    // Drive the emissive into HDR at the peak so the bloom pass blooms:
    // 1.0x at the trough, 5.0x at the crest of the animation cycle.
    let intensity = animated_unit(phase, SDF_ANIMATION_GREEN_OFFSET).mul_add(4.0, 1.0);
    material.emissive = EMISSIVE_WARM.to_linear() * intensity;
    material
}

fn sdf_emissive_text_material() -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = SDF_EMISSIVE_TEXT_INK;
    material.emissive = SDF_EMISSIVE_TEXT_GLOW.to_linear();
    material.alpha_mode = AlphaMode::Blend;
    material
}

// Image SDF fill: a white base color modulated by `base_color_texture`, sampled
// over the quad UV. The texture handle is a batch-compatibility splitter, so this
// card forms its own SDF batch separate from the three table-value cards.
fn image_fill_material(image: Handle<Image>) -> StandardMaterial {
    let mut material = default_panel_material();
    material.base_color = Color::WHITE;
    material.base_color_texture = Some(image.clone());
    material.emissive_texture = Some(image);
    material.emissive = LinearRgba::WHITE;
    material
}

// Each card draws only the shape its label names. The shapes live in a
// full-card-width strip element between the title and the material caption, so
// they resolve against that element's local space: every line spans `start(3)`
// to `end(6)`, vertically centered, the same length regardless of card width.
fn shape_strip(
    materials: &ShapePanelMaterialHandles,
    index: usize,
    color: Color,
) -> Vec<hana_diegetic::PanelShape> {
    // A fresh line spanning the strip width, built per row because `PanelLine`
    // and `PanelPoint` move on each builder call. The end inset varies: a
    // centered end cap (circle/diamond) extends half its size past its end
    // point, but the arrowhead tip lands on its end point, so the arrow's line
    // ends further right to reach the same rightmost point.
    let span = |end_inset: f32| {
        PanelLine::new(
            PanelPoint::new(PanelCoord::start(Mm(3.0)), PanelCoord::percent(0.5)),
            PanelPoint::new(PanelCoord::end(Mm(end_inset)), PanelCoord::percent(0.5)),
        )
        .color(color)
    };
    match index {
        // Panel-level shape material default: no local source handle.
        0 => vec![
            span(3.6)
                .width(Mm(0.5))
                .end_cap(CalloutCap::arrow().solid().length(4.0).width(3.2))
                .into(),
        ],
        // Shape-local material through `PanelLine::material`.
        1 => vec![
            span(6.0)
                .width(Mm(0.45))
                .material(materials.local_line.clone())
                .end_cap(CalloutCap::circle().radius(2.4))
                .into(),
        ],
        // Shape-local material through `LineStyle::material`.
        2 => vec![
            PanelLine::new(
                PanelPoint::new(PanelCoord::start(Mm(3.0)), PanelCoord::percent(0.5)),
                PanelPoint::new(PanelCoord::end(Mm(6.0)), PanelCoord::percent(0.5)),
            )
            .style(
                LineStyle::default()
                    .width(Mm(0.4))
                    .color(color)
                    .material(materials.style_line.clone())
                    .start_cap(CalloutCap::circle().radius(1.8))
                    .end_cap(CalloutCap::diamond().width(3.0).height(3.0)),
            )
            .into(),
        ],
        // Shape-local material through `PanelCircle::material`.
        3 => vec![
            PanelCircle::new(
                PanelPoint::new(PanelCoord::percent(0.5), PanelCoord::percent(0.5)),
                Mm(4.0),
            )
            .color(color)
            .material(materials.circle.clone())
            .into(),
        ],
        // Material alpha mode is a pipeline splitter.
        4 => vec![
            span(5.0)
                .width(Mm(0.62))
                .material(materials.alpha_split.clone())
                .into(),
        ],
        // Base-color texture samples across the shape-local 0..1 box UV.
        _ => vec![
            span(5.0)
                .width(Mm(1.0))
                .material(materials.texture.clone())
                .end_cap(CalloutCap::square().width(3.0).height(3.0))
                .into(),
        ],
    }
}

fn shape_group_card(
    builder: &mut LayoutBuilder,
    materials: &ShapePanelMaterialHandles,
    index: usize,
    label: &str,
    detail: &str,
    color: Color,
) {
    // Title on top, the drawn line spanning the full card width in the middle,
    // and the material caption beneath in a smaller font. Stacking gives the
    // line strip the whole card width so its start/end insets and end caps have
    // room — a narrow side strip collapses the shaft and leaves only the caps.
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(1.0)
            .padding(Padding::new(4.0, 4.0, 3.0, 3.0))
            .background(Color::srgba(0.02, 0.03, 0.04, 0.42))
            .border(Border::all(Mm(0.25), CARD_BORDER_WARM))
            .corner_radius(CornerRadius::all(Mm(1.2)))
            .alignment(AlignX::Left, AlignY::Top),
        |builder| {
            builder.text((label, body_style(color)));
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .draw(PanelDraw::shapes(shape_strip(materials, index, color))),
                |_builder| {},
            );
            builder.text((detail, shape_card_caption_style(TEXT_MUTED)));
        },
    );
}

fn shape_card_caption_style(color: Color) -> TextStyle {
    TextStyle::new(SHAPE_CARD_CAPTION_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn material_group_title_style() -> TextStyle {
    TextStyle::new(MATERIAL_GROUP_TITLE_FONT_SIZE)
        .bold()
        .with_color(TEXT_MAIN)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn material_caption_style(color: Color) -> TextStyle {
    TextStyle::new(MATERIAL_CASE_CAPTION_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn material_value_style(color: Color) -> TextStyle {
    TextStyle::new(MATERIAL_CASE_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn title_style() -> TextStyle {
    TextStyle::new(TITLE_FONT_SIZE)
        .bold()
        .with_color(TEXT_MAIN)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn subtitle_style(color: Color) -> TextStyle {
    TextStyle::new(SUBTITLE_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn swatch_style(color: Color) -> TextStyle {
    TextStyle::new(SWATCH_FONT_SIZE)
        .bold()
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn body_style(color: Color) -> TextStyle {
    TextStyle::new(BODY_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn small_style(color: Color) -> TextStyle {
    TextStyle::new(SMALL_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}
