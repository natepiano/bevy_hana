//! Text configuration types used by layout and standalone world text.

use std::hash::Hash;

use bevy::asset::Handle;
use bevy::color::Color;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;

use super::Anchor;
use super::Dimension;
use super::FontFeatureFlags;
use super::FontFeatures;
use super::Unit;
use super::constants::DEFAULT_FONT_SIZE;
use crate::cascade::Cascade;

/// Controls how the layout engine breaks text across lines.
///
/// The engine splits text according to this mode and measures individual
/// runs via the [`MeasureTextFn`](crate::layout::MeasureTextFn) callback
/// to determine break points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum TextWrap {
    /// Never wrap. The full text is measured as a single run and may
    /// overflow the element's bounds.
    None,
    /// Break at word boundaries when text exceeds the element's width.
    ///
    /// Words are split on ASCII whitespace. The engine measures each word
    /// individually, accumulates widths on a line, and breaks when the
    /// next word would exceed the available width.
    #[default]
    Words,
    /// Break only at explicit `\n` characters.
    ///
    /// Each line between newlines is measured as a single run. The element's
    /// width is the widest line; height is the sum of all line heights.
    Newlines,
}

// ── Font property types ──────────────────────────────────────────────────────

/// Font weight (boldness) as a numeric value on the 1–1000 scale.
///
/// Standard weights: 100 (Thin) through 900 (Black). `400` is normal, `700` is bold.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct FontWeight(pub f32);

impl FontWeight {
    /// Normal weight (400).
    pub const NORMAL: Self = Self(400.0);
    /// Bold weight (700).
    pub const BOLD: Self = Self(700.0);
    /// Light weight (300).
    pub const LIGHT: Self = Self(300.0);
}

impl Default for FontWeight {
    fn default() -> Self { Self::NORMAL }
}

/// Font slant (posture).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum FontSlant {
    /// Upright (roman) style.
    #[default]
    Normal,
    /// Italic style (true italic glyphs).
    Italic,
    /// Oblique style (slanted roman glyphs).
    Oblique,
}

/// Horizontal text alignment within bounds.
///
/// Positions each measured or wrapped line within the text element's bounds.
/// For standalone [`TextContent`](crate::TextContent), this is stored as part of
/// the text style and participates in shaping/render cache keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum TextAlign {
    /// Align to the left edge.
    #[default]
    Left,
    /// Center horizontally.
    Center,
    /// Align to the right edge.
    Right,
}

/// How the visible glyph renders.
///
/// Controls the text shader's coverage computation. Both modes use
/// `AlphaMode::Blend` for smooth anti-aliased edges. Discriminants are
/// `#[repr(u32)]` and explicit because they map directly to shader
/// constants in `analytic_path.wgsl`; the compile-time assertions below keep
/// them in sync.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[repr(u32)]
pub enum GlyphRenderMode {
    /// Normal text rendering — smooth alpha-blended edges.
    #[default]
    Text     = 1,
    /// Glyph quad filled everywhere except the letter outline (inverted alpha).
    PunchOut = 2,
}

impl GlyphRenderMode {
    #[must_use]
    const fn discriminant(self) -> u32 {
        match self {
            Self::Text => 1,
            Self::PunchOut => 2,
        }
    }
}

impl From<GlyphRenderMode> for u32 {
    fn from(render_mode: GlyphRenderMode) -> Self { render_mode.discriminant() }
}

/// Whether glyphs cast a shadow.
///
/// The visible glyph mesh casts its own coverage-silhouette shadow
/// directly. For a shadow with
/// no visible fill (ghost text), spawn a `Cast` glyph and set its fill
/// color alpha to `0`: the color pass paints nothing while the shadow
/// pass still writes the full letter silhouette.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[reflect(Resource)]
pub enum GlyphShadowMode {
    /// Glyph casts no shadow.
    None,
    /// Glyph casts its coverage-silhouette shadow.
    #[default]
    Cast,
}

/// Whether rendered diegetic content casts 3D shadows.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[reflect(Resource)]
pub enum ShadowCasting {
    /// Do not cast 3D shadows.
    Off,
    /// Cast 3D shadows.
    #[default]
    On,
}

/// Which face(s) of a glyph mesh render, by back/front-face culling.
///
/// World text defaults to both faces; screen text defaults to front-only.
/// The cascade carries the contextual default (`Sidedness` is a cascade
/// attribute); a per-label value on [`TextStyle`] overrides it.
///
/// On a transparent panel the front and back faces are both viewable, so
/// `FrontOnly` and `BackOnly` author labels visible from one side only.
/// Back-only glyphs read mirror-reversed when viewed from behind; a future
/// reverse-text feature flips them to read correctly.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[reflect(Resource)]
pub enum Sidedness {
    /// Render both faces with no culling (default).
    #[default]
    BothSides,
    /// Render only the front face (cull the back). Visible from the front.
    FrontOnly,
    /// Render only the back face (cull the front). Visible from behind.
    BackOnly,
}

/// Whether glyph materials respond to scene lighting.
///
/// World text defaults to lit; screen text defaults to unlit. The cascade
/// carries the contextual default (`Lighting` is a cascade attribute); a
/// per-label value on [`TextStyle`] overrides it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[reflect(Resource)]
pub enum Lighting {
    /// Use normal PBR lighting.
    #[default]
    Lit,
    /// Bypass PBR lighting and render with the authored material color.
    Unlit,
}

/// Panel-scoped authored draw-order z-index.
///
/// `DrawZIndex(0)` is the default level. Positive values move forward, and
/// negative values move back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Reflect)]
pub struct DrawZIndex(pub i8);

impl From<i8> for DrawZIndex {
    fn from(value: i8) -> Self { Self(value) }
}

impl From<DrawZIndex> for i8 {
    fn from(value: DrawZIndex) -> Self { value.0 }
}

impl From<DrawZIndex> for i32 {
    fn from(value: DrawZIndex) -> Self { Self::from(value.0) }
}

// ── TextStyle ────────────────────────────────────────────────────────────────

/// Unified text styling for both layout (panel) text and standalone world text.
///
/// One struct carries every typography and render field. Layout and
/// standalone contexts share the data; context-specific defaults (lighting,
/// sidedness, font unit) are supplied by the cascade and per-context builders
/// rather than by separate types.
///
/// All fields are private and authored through builder methods.
///
/// ```ignore
/// // Panel text style:
/// TextStyle::new(14.0)
///     .with_font(FontId::MONOSPACE.0)
///     .bold()
///     .with_align(TextAlign::Center)
///
/// // Standalone world text:
/// TextStyle::new(24.0).with_font(FontId::MONOSPACE.0).bold().with_color(Color::RED)
/// ```
#[derive(Component, Clone, Debug, Reflect)]
pub struct TextStyle {
    font_id:                u16,
    size:                   f32,
    weight:                 FontWeight,
    slant:                  FontSlant,
    line_height:            f32,
    letter_spacing:         f32,
    word_spacing:           f32,
    color:                  Color,
    align:                  TextAlign,
    anchor:                 Anchor,
    render_mode:            GlyphRenderMode,
    /// Per-label glyph-shadow authoring.
    shadow_mode:            Cascade<GlyphShadowMode>,
    /// Per-label shadow-casting authoring.
    shadow_casting:         Cascade<ShadowCasting>,
    /// Per-label sidedness authoring.
    sidedness:              Cascade<Sidedness>,
    /// Per-label lighting authoring.
    lighting:               Cascade<Lighting>,
    /// Per-label source material handle for text runs.
    ///
    /// This is an authored cascade/source handle. It is not a Bevy render
    /// material asset and it does not affect text measurement or layout cache
    /// keys. Create the material once through `Assets<StandardMaterial>` and
    /// pass the handle here; do not create a fresh asset while rebuilding panel
    /// data each frame.
    material:               Cascade<Handle<StandardMaterial>>,
    font_features:          FontFeatures,
    /// What unit `size` is expressed in.
    unit:                   Cascade<Unit>,
    /// Per-label alpha-mode authoring.
    alpha_mode:             Cascade<AlphaMode>,
    /// Per-label HDR text coverage-bias authoring.
    hdr_text_coverage_bias: Cascade<f32>,
}

impl PartialEq for TextStyle {
    fn eq(&self, other: &Self) -> bool {
        self.font_id == other.font_id
            && self.size == other.size
            && self.weight == other.weight
            && self.slant == other.slant
            && self.line_height == other.line_height
            && self.letter_spacing == other.letter_spacing
            && self.word_spacing == other.word_spacing
            && self.color == other.color
            && self.align == other.align
            && self.anchor == other.anchor
            && self.render_mode == other.render_mode
            && self.shadow_mode == other.shadow_mode
            && self.shadow_casting == other.shadow_casting
            && self.sidedness == other.sidedness
            && self.lighting == other.lighting
            && self.material == other.material
            && self.font_features == other.font_features
            && self.unit == other.unit
            && self.alpha_mode == other.alpha_mode
            && self.hdr_text_coverage_bias == other.hdr_text_coverage_bias
    }
}

impl Default for TextStyle {
    fn default() -> Self { Self::new(DEFAULT_FONT_SIZE) }
}

impl TextStyle {
    /// Creates a new text style with the given font size.
    ///
    /// Accepts [`Pt`](crate::Pt), [`Mm`](crate::Mm), [`In`](crate::In),
    /// [`Px`](crate::Px), or bare `f32`. Newtypes carry their unit; a bare
    /// `f32` records no unit and resolves from the contextual `FontUnit`
    /// cascade attribute (panel font unit, world units, or pixels).
    ///
    /// Defaults to centered anchor, normal weight, white color.
    #[must_use]
    pub fn new(size: impl Into<Dimension>) -> Self {
        let font_size = size.into();
        Self {
            font_id:                0,
            size:                   font_size.value,
            weight:                 FontWeight::NORMAL,
            slant:                  FontSlant::Normal,
            line_height:            0.0,
            letter_spacing:         0.0,
            word_spacing:           0.0,
            color:                  Color::WHITE,
            align:                  TextAlign::Left,
            anchor:                 Anchor::Center,
            render_mode:            GlyphRenderMode::Text,
            shadow_mode:            Cascade::Inherit,
            shadow_casting:         Cascade::Inherit,
            sidedness:              Cascade::Inherit,
            lighting:               Cascade::Inherit,
            material:               Cascade::Inherit,
            font_features:          FontFeatures::NONE,
            unit:                   font_size.unit.map_or(Cascade::Inherit, Cascade::Override),
            alpha_mode:             Cascade::Inherit,
            hdr_text_coverage_bias: Cascade::Inherit,
        }
    }

    // ── Getters ───────────────────────────────────────────────────────────

    /// Returns the font identifier.
    #[must_use]
    pub const fn font_id(&self) -> u16 { self.font_id }

    /// Returns the font size in layout units.
    #[must_use]
    pub const fn size(&self) -> f32 { self.size }

    /// Returns the font weight.
    #[must_use]
    pub const fn weight(&self) -> FontWeight { self.weight }

    /// Returns the font slant.
    #[must_use]
    pub const fn slant(&self) -> FontSlant { self.slant }

    /// Returns the line height in layout units (0.0 = use `size`).
    #[must_use]
    pub const fn line_height_raw(&self) -> f32 { self.line_height }

    /// Returns the letter spacing in layout units.
    #[must_use]
    pub const fn letter_spacing(&self) -> f32 { self.letter_spacing }

    /// Returns the word spacing in layout units.
    #[must_use]
    pub const fn word_spacing(&self) -> f32 { self.word_spacing }

    /// Returns the text color.
    #[must_use]
    pub const fn color(&self) -> Color { self.color }

    /// Returns the glyph render mode.
    #[must_use]
    pub const fn render_mode(&self) -> GlyphRenderMode { self.render_mode }

    /// Returns the glyph shadow mode authoring.
    #[must_use]
    pub(crate) const fn shadow_mode(&self) -> Cascade<GlyphShadowMode> { self.shadow_mode }

    /// Returns the shadow-casting authoring.
    #[must_use]
    pub(crate) const fn shadow_casting(&self) -> Cascade<ShadowCasting> { self.shadow_casting }

    /// Returns the per-label sidedness authoring.
    #[must_use]
    pub(crate) const fn sidedness(&self) -> Cascade<Sidedness> { self.sidedness }

    /// Returns the per-label lighting authoring.
    #[must_use]
    pub(crate) const fn lighting(&self) -> Cascade<Lighting> { self.lighting }

    /// Returns the text-run source material authoring.
    #[must_use]
    pub(crate) const fn material(&self) -> Cascade<&Handle<StandardMaterial>> {
        self.material.as_ref()
    }

    /// Returns the font feature overrides.
    #[must_use]
    pub const fn font_features(&self) -> FontFeatures { self.font_features }

    /// Returns the per-label unit authoring.
    #[must_use]
    pub(super) const fn unit(&self) -> Cascade<Unit> { self.unit }

    /// Returns the text alignment.
    #[must_use]
    pub const fn text_align(&self) -> TextAlign { self.align }

    /// Returns the anchor point.
    #[must_use]
    pub const fn anchor(&self) -> Anchor { self.anchor }

    /// Returns the per-label alpha-mode authoring.
    #[must_use]
    pub(crate) const fn alpha_mode(&self) -> Cascade<AlphaMode> { self.alpha_mode }

    /// Returns the per-label HDR text coverage-bias authoring.
    #[must_use]
    pub(crate) const fn hdr_text_coverage_bias(&self) -> Cascade<f32> {
        self.hdr_text_coverage_bias
    }

    // ── Chained (with_*) setters ──────────────────────────────────────────

    /// Sets the font identifier.
    #[must_use]
    pub const fn with_font(mut self, font_id: u16) -> Self {
        self.font_id = font_id;
        self
    }

    /// Sets the font weight.
    #[must_use]
    pub const fn with_weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    /// Shorthand for `with_weight(FontWeight::BOLD)`.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.weight = FontWeight::BOLD;
        self
    }

    /// Sets the font slant.
    #[must_use]
    pub const fn with_slant(mut self, slant: FontSlant) -> Self {
        self.slant = slant;
        self
    }

    /// Shorthand for `with_slant(FontSlant::Italic)`.
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.slant = FontSlant::Italic;
        self
    }

    /// Sets the line height in layout units. `0.0` = use `size`.
    #[must_use]
    pub const fn with_line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }

    /// Sets extra spacing between characters in layout units.
    #[must_use]
    pub const fn with_letter_spacing(mut self, spacing: f32) -> Self {
        self.letter_spacing = spacing;
        self
    }

    /// Sets extra spacing between words in layout units.
    #[must_use]
    pub const fn with_word_spacing(mut self, spacing: f32) -> Self {
        self.word_spacing = spacing;
        self
    }

    /// Sets the text color.
    #[must_use]
    pub const fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Sets the glyph render mode.
    #[must_use]
    pub const fn with_render_mode(mut self, mode: GlyphRenderMode) -> Self {
        self.render_mode = mode;
        self
    }

    /// Sets the glyph shadow mode.
    #[must_use]
    pub const fn with_shadow_mode(mut self, mode: GlyphShadowMode) -> Self {
        self.shadow_mode = Cascade::Override(mode);
        self
    }

    /// Removes the per-label glyph shadow mode override.
    #[must_use]
    pub const fn inherit_shadow_mode(mut self) -> Self {
        self.shadow_mode = Cascade::Inherit;
        self
    }

    /// Sets whether this label casts 3D shadows.
    #[must_use]
    pub const fn with_shadow_casting(mut self, shadow_casting: ShadowCasting) -> Self {
        self.shadow_casting = Cascade::Override(shadow_casting);
        self
    }

    /// Removes the per-label shadow-casting override.
    #[must_use]
    pub const fn inherit_shadow_casting(mut self) -> Self {
        self.shadow_casting = Cascade::Inherit;
        self
    }

    /// Sets horizontal text alignment within bounds.
    #[must_use]
    pub const fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Sets the anchor point within the text block's bounding box.
    #[must_use]
    pub const fn with_anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Sets a per-label sidedness override (overrides the panel/context default).
    #[must_use]
    pub const fn with_sidedness(mut self, sidedness: Sidedness) -> Self {
        self.sidedness = Cascade::Override(sidedness);
        self
    }

    /// Sets a per-label lighting override (overrides the panel/context default).
    #[must_use]
    pub const fn with_lighting(mut self, lighting: Lighting) -> Self {
        self.lighting = Cascade::Override(lighting);
        self
    }

    /// Sets the glyph material to render unlit, bypassing PBR lighting.
    #[must_use]
    pub const fn with_unlit(mut self) -> Self {
        self.lighting = Cascade::Override(Lighting::Unlit);
        self
    }

    /// Sets the source material handle for this text run.
    ///
    /// The handle overrides the `TextMaterial` cascade source for rendering
    /// only. Measurement and shaping cache keys ignore it; scalar color edits
    /// still flow through the material-table row, with `with_color` overriding
    /// that row's `base_color`.
    #[must_use]
    pub fn with_material(mut self, material: Handle<StandardMaterial>) -> Self {
        self.material = Cascade::Override(material);
        self
    }

    /// Sets font feature overrides.
    #[must_use]
    pub const fn with_font_features(mut self, features: FontFeatures) -> Self {
        self.font_features = features;
        self
    }

    /// Disables contextual alternates (`calt`).
    #[must_use]
    pub const fn without_contextual_alternates(mut self) -> Self {
        self.font_features = self.font_features.without(FontFeatureFlags::CALT);
        self
    }

    /// Disables standard ligatures (`liga`).
    #[must_use]
    pub const fn without_ligatures(mut self) -> Self {
        self.font_features = self.font_features.without(FontFeatureFlags::LIGA);
        self
    }

    /// Sets the per-label text [`AlphaMode`] override.
    ///
    /// The panel-text reifier captures this value before converting via
    /// [`for_shaping`](Self::for_shaping) and inserts `Cascade<TextAlpha>`
    /// on the label.
    #[must_use]
    pub const fn with_alpha_mode(mut self, alpha_mode: AlphaMode) -> Self {
        self.alpha_mode = Cascade::Override(alpha_mode);
        self
    }

    /// Sets the per-label HDR text coverage-bias override.
    ///
    /// `0.0` keeps the automatic screen-size adjustment unchanged. Positive
    /// values make fractional glyph edges more opaque beyond that adjustment,
    /// which can compensate for dark text that looks too thin under HDR,
    /// especially on light backgrounds. Tune this per scene, panel, or label;
    /// it can make light text on dark backgrounds look heavier.
    #[must_use]
    pub const fn with_hdr_text_coverage_bias(mut self, bias: f32) -> Self {
        self.hdr_text_coverage_bias = Cascade::Override(bias);
        self
    }

    /// Removes the per-label HDR text coverage-bias override.
    pub(super) const fn clear_hdr_text_coverage_bias(&mut self) {
        self.hdr_text_coverage_bias = Cascade::Inherit;
    }

    // ── In-place (set_*) setters ──────────────────────────────────────────

    /// Sets the font identifier.
    pub const fn set_font_id(&mut self, font_id: u16) { self.font_id = font_id; }

    /// Sets the font size.
    pub const fn set_size(&mut self, size: f32) { self.size = size; }

    /// Sets the font weight.
    pub const fn set_weight(&mut self, weight: FontWeight) { self.weight = weight; }

    /// Sets the font slant.
    pub const fn set_slant(&mut self, slant: FontSlant) { self.slant = slant; }

    /// Sets the line height in layout units. `0.0` = use `size`.
    pub const fn set_line_height(&mut self, line_height: f32) { self.line_height = line_height; }

    /// Sets extra spacing between characters in layout units.
    pub const fn set_letter_spacing(&mut self, spacing: f32) { self.letter_spacing = spacing; }

    /// Sets extra spacing between words in layout units.
    pub const fn set_word_spacing(&mut self, spacing: f32) { self.word_spacing = spacing; }

    /// Sets the text color.
    pub const fn set_color(&mut self, color: Color) { self.color = color; }

    /// Sets horizontal text alignment within bounds.
    pub const fn set_align(&mut self, align: TextAlign) { self.align = align; }

    /// Sets the anchor point within the text block's bounding box.
    pub const fn set_anchor(&mut self, anchor: Anchor) { self.anchor = anchor; }

    /// Sets the glyph render mode.
    pub const fn set_render_mode(&mut self, mode: GlyphRenderMode) { self.render_mode = mode; }

    /// Sets the glyph shadow mode.
    pub const fn set_shadow_mode(&mut self, mode: GlyphShadowMode) {
        self.shadow_mode = Cascade::Override(mode);
    }

    /// Removes the per-label glyph shadow mode override.
    pub const fn set_shadow_mode_inherit(&mut self) { self.shadow_mode = Cascade::Inherit; }

    /// Sets whether this label casts 3D shadows.
    pub const fn set_shadow_casting(&mut self, shadow_casting: ShadowCasting) {
        self.shadow_casting = Cascade::Override(shadow_casting);
    }

    /// Removes the per-label shadow-casting override.
    pub const fn set_shadow_casting_inherit(&mut self) { self.shadow_casting = Cascade::Inherit; }

    /// Sets a per-label sidedness override (overrides the panel/context default).
    pub const fn set_sidedness(&mut self, sidedness: Sidedness) {
        self.sidedness = Cascade::Override(sidedness);
    }

    /// Sets a per-label lighting override (overrides the panel/context default).
    pub const fn set_lighting(&mut self, lighting: Lighting) {
        self.lighting = Cascade::Override(lighting);
    }

    /// Sets the source material handle for this text run.
    pub fn set_material(&mut self, material: Handle<StandardMaterial>) {
        self.material = Cascade::Override(material);
    }

    /// Sets font feature overrides.
    pub const fn set_font_features(&mut self, features: FontFeatures) {
        self.font_features = features;
    }

    /// Sets the font size and unit from a [`Dimension`].
    ///
    /// A bare `f32` records no unit (`None`) and resolves from the contextual
    /// `FontUnit` cascade attribute; an explicit `Px`/`Pt`/`Mm`/`In` records
    /// its unit and always wins. Used by the `WorldText` / `ScreenText`
    /// builders, whose `.size(..)` takes any `Into<Dimension>`.
    pub fn set_dimension(&mut self, size: impl Into<Dimension>) {
        let dimension = size.into();
        self.size = dimension.value;
        self.unit = dimension.unit.map_or(Cascade::Inherit, Cascade::Override);
    }

    /// Sets the per-label [`AlphaMode`] override.
    pub const fn set_alpha_mode(&mut self, alpha_mode: AlphaMode) {
        self.alpha_mode = Cascade::Override(alpha_mode);
    }

    /// Sets the per-label HDR text coverage-bias override.
    pub const fn set_hdr_text_coverage_bias(&mut self, bias: f32) {
        self.hdr_text_coverage_bias = Cascade::Override(bias);
    }

    // ── Conversions and derived views ─────────────────────────────────────

    /// Returns a copy with font-related dimensions multiplied by `factor`.
    ///
    /// Used by the layout engine to convert font sizes from font units to
    /// layout units in render commands. Non-dimensional fields (color, font
    /// features, etc.) are preserved unchanged.
    #[must_use]
    pub fn scaled(&self, factor: f32) -> Self {
        let mut copy = self.clone();
        copy.size *= factor;
        copy.line_height *= factor;
        copy.letter_spacing *= factor;
        copy.word_spacing *= factor;
        copy
    }

    /// Returns a copy whose numeric font dimensions are expressed in `unit`.
    ///
    /// Used when a panel conversion resolves authored text sizes into a new
    /// source unit before the normal panel layout pass runs again.
    #[must_use]
    pub(super) fn scaled_as_unit(&self, factor: f32, unit: Unit) -> Self {
        let mut copy = self.scaled(factor);
        copy.unit = Cascade::Override(unit);
        copy
    }

    /// Returns a copy prepared for text shaping at the given anchor.
    ///
    /// Forces [`TextAlign::Left`] and clears the unit / alpha-mode authoring
    /// fields (those route through the cascade). The two
    /// contexts differ only in anchor: world text uses [`Anchor::TopLeft`] (the
    /// command origin), layout-engine text uses [`Anchor::Center`].
    /// Crate-internal helper.
    #[must_use]
    pub fn for_shaping(&self, anchor: Anchor) -> Self {
        Self {
            align: TextAlign::Left,
            anchor,
            unit: Cascade::Inherit,
            alpha_mode: Cascade::Inherit,
            hdr_text_coverage_bias: Cascade::Inherit,
            ..self.clone()
        }
    }

    /// Extracts measurement-relevant fields as a [`TextMeasure`].
    ///
    /// Used by [`MeasureTextFn`](crate::layout::MeasureTextFn) — no generic
    /// parameter, no infection into the layout engine.
    #[must_use]
    pub const fn as_measure(&self) -> TextMeasure {
        TextMeasure {
            font_id:        self.font_id,
            size:           self.size,
            weight:         self.weight,
            slant:          self.slant,
            line_height:    self.line_height,
            letter_spacing: self.letter_spacing,
            word_spacing:   self.word_spacing,
            font_features:  self.font_features,
        }
    }

    // ── Equality / hashing helpers ────────────────────────────────────────

    /// Hashes all layout-affecting fields into `hasher`, excluding color.
    ///
    /// Uses exhaustive destructuring so that adding a new field to
    /// [`TextStyle`] without updating this method is a compiler error.
    pub fn hash_layout(&self, hasher: &mut impl std::hash::Hasher) {
        // Destructure exhaustively — compiler error if a field is added.
        let Self {
            font_id,
            size,
            weight,
            slant,
            line_height,
            letter_spacing,
            word_spacing,
            align,
            anchor,
            font_features,
            // Render-only — explicitly skipped.
            color: _,
            render_mode: _,
            shadow_mode: _,
            shadow_casting: _,
            sidedness: _,
            lighting: _,
            material: _,
            // Measurement context — not a layout-cache key.
            unit: _,
            // Render-only — affects compositing, not measurement.
            alpha_mode: _,
            // Render-only — coverage transfer, not measurement.
            hdr_text_coverage_bias: _,
        } = self;

        font_id.hash(hasher);
        size.to_bits().hash(hasher);
        weight.0.to_bits().hash(hasher);
        (*slant as u8).hash(hasher);
        line_height.to_bits().hash(hasher);
        letter_spacing.to_bits().hash(hasher);
        word_spacing.to_bits().hash(hasher);
        (*align as u8).hash(hasher);
        (*anchor as u8).hash(hasher);
        font_features.hash(hasher);
    }

    /// Returns whether layout-affecting text fields match, ignoring fields
    /// that only affect rendering.
    pub(super) fn layout_eq_excluding_visuals(&self, other: &Self) -> bool {
        let Self {
            font_id,
            size,
            weight,
            slant,
            line_height,
            letter_spacing,
            word_spacing,
            align,
            anchor,
            font_features,
            unit,
            // Render-only.
            color: _,
            render_mode: _,
            shadow_mode: _,
            shadow_casting: _,
            sidedness: _,
            lighting: _,
            material: _,
            alpha_mode: _,
            hdr_text_coverage_bias: _,
        } = self;

        *font_id == other.font_id
            && size.to_bits() == other.size.to_bits()
            && weight.0.to_bits() == other.weight.0.to_bits()
            && *slant == other.slant
            && line_height.to_bits() == other.line_height.to_bits()
            && letter_spacing.to_bits() == other.letter_spacing.to_bits()
            && word_spacing.to_bits() == other.word_spacing.to_bits()
            && *align == other.align
            && *anchor == other.anchor
            && *font_features == other.font_features
            && *unit == other.unit
    }

    /// Bit-equality over the fields panel-text glyph geometry depends on, used
    /// to gate per-run glyph and path-quad rebuilds.
    ///
    /// Compares the measurement fields (`font_id`, `size`, `weight`, `slant`,
    /// `line_height`, letter/word spacing, `align`, `anchor`, `font_features`)
    /// via `to_bits`.
    ///
    /// Excludes render/material fields (`color`, `render_mode`, `shadow_mode`,
    /// `shadow_casting`, `sidedness`, `lighting`, `material`, `alpha_mode`,
    /// `hdr_text_coverage_bias`) because `PreparedPanelText`, cascade
    /// overrides, `PathRenderRecord`, and the frame material table own those
    /// updates without changing glyph geometry.
    /// Excludes `unit` (measurement context, not a mesh input) and
    /// `alpha_mode` (gated separately through `Cascade<TextAlpha>`).
    pub(crate) fn gating_eq(&self, other: &Self) -> bool {
        let Self {
            font_id,
            size,
            weight,
            slant,
            line_height,
            letter_spacing,
            word_spacing,
            align,
            anchor,
            font_features,
            color: _,
            render_mode: _,
            shadow_mode: _,
            shadow_casting: _,
            sidedness: _,
            lighting: _,
            material: _,
            unit: _,
            alpha_mode: _,
            hdr_text_coverage_bias: _,
        } = self;

        *font_id == other.font_id
            && size.to_bits() == other.size.to_bits()
            && weight.0.to_bits() == other.weight.0.to_bits()
            && *slant == other.slant
            && line_height.to_bits() == other.line_height.to_bits()
            && letter_spacing.to_bits() == other.letter_spacing.to_bits()
            && word_spacing.to_bits() == other.word_spacing.to_bits()
            && *align == other.align
            && *anchor == other.anchor
            && *font_features == other.font_features
    }
}

// ── TextMeasure ──────────────────────────────────────────────────────────────

/// The subset of text properties needed for measurement.
///
/// Extracted from [`TextStyle`] via [`as_measure()`](TextStyle::as_measure).
/// This is what [`MeasureTextFn`](crate::layout::MeasureTextFn) receives — no
/// generic parameter, no infection into the layout engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMeasure {
    /// Font identifier.
    pub font_id:        u16,
    /// Font size in layout units.
    pub size:           f32,
    /// Font weight.
    pub weight:         FontWeight,
    /// Font slant.
    pub slant:          FontSlant,
    /// Line height (0.0 = use `size`).
    pub line_height:    f32,
    /// Letter spacing in layout units.
    pub letter_spacing: f32,
    /// Word spacing in layout units.
    pub word_spacing:   f32,
    /// OpenType feature overrides.
    pub font_features:  FontFeatures,
}

impl TextMeasure {
    /// Returns a copy with font-related dimensions multiplied by `factor`.
    ///
    /// Used by the layout engine to convert font sizes from font units to
    /// layout units when the two differ (e.g. points → millimeters).
    #[must_use]
    pub fn scaled(mut self, factor: f32) -> Self {
        self.size *= factor;
        self.line_height *= factor;
        self.letter_spacing *= factor;
        self.word_spacing *= factor;
        self
    }
}

/// Measured dimensions of a text string.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextDimensions {
    /// Width in layout units.
    pub width:       f32,
    /// Height in layout units.
    pub height:      f32,
    /// Per-line height from parley (includes font's natural line gap
    /// when no explicit override is set).
    pub line_height: f32,
}

// ── Shader discriminant assertions ──────────────────────────────────────────
//
// These compile-time assertions ensure that `GlyphRenderMode` discriminants
// stay in sync with the `render_mode` constants in `analytic_path.wgsl` (and the
// matching `RenderMode` variants). If you add or reorder variants, update
// the shader constants to match and adjust these assertions.

const _: () = assert!(GlyphRenderMode::Text.discriminant() == 1);
const _: () = assert!(GlyphRenderMode::PunchOut.discriminant() == 2);

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests use expect for clearer failure messages"
)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;

    use bevy::asset::Assets;
    use bevy::pbr::StandardMaterial;
    use bevy::prelude::default;

    use super::*;
    use crate::layout::BoundingBox;

    #[test]
    fn for_shaping_preserves_size() {
        let style = TextStyle::new(24.0);
        let prepared = style.for_shaping(Anchor::TopLeft);
        assert!(
            (prepared.size() - 24.0).abs() < f32::EPSILON,
            "size should be preserved"
        );
    }

    #[test]
    fn for_shaping_applies_given_anchor() {
        // World text anchors at the command origin (TopLeft); layout-engine
        // text anchors at Center. The anchor is the one per-context difference.
        let style = TextStyle::new(12.0);
        assert_eq!(style.for_shaping(Anchor::TopLeft).anchor(), Anchor::TopLeft);
        assert_eq!(style.for_shaping(Anchor::Center).anchor(), Anchor::Center);
    }

    #[test]
    fn with_anchor_overrides_default() {
        let standalone = TextStyle::new(12.0)
            .for_shaping(Anchor::TopLeft)
            .with_anchor(Anchor::TopLeft);
        assert_eq!(standalone.anchor(), Anchor::TopLeft);
    }

    // ── TextStyle::gating_eq ───────────────────────────────────────

    #[test]
    fn gating_eq_true_for_identical_style() {
        let style = TextStyle::new(24.0).with_color(Color::WHITE);
        assert!(style.gating_eq(&style.clone()));
    }

    #[test]
    fn gating_eq_detects_size_change() {
        let base = TextStyle::new(24.0);
        let bigger = TextStyle::new(48.0);
        assert!(!base.gating_eq(&bigger));
    }

    #[test]
    fn gating_eq_ignores_color_change() {
        let base = TextStyle::new(24.0).with_color(Color::WHITE);
        let recolored = base.clone().with_color(Color::BLACK);
        assert!(base.layout_eq_excluding_visuals(&recolored));
        assert!(
            base.gating_eq(&recolored),
            "text color updates material-table rows without rebuilding glyph geometry"
        );
    }

    #[test]
    fn material_handle_is_render_only_for_layout_and_gating_keys() {
        let mut materials = Assets::<StandardMaterial>::default();
        let red = materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.0, 0.0),
            ..default()
        });
        let blue = materials.add(StandardMaterial {
            base_color: Color::srgb(0.0, 0.0, 1.0),
            ..default()
        });

        let red_style = TextStyle::new(24.0).with_material(red);
        let blue_style = TextStyle::new(24.0).with_material(blue);

        assert_ne!(
            red_style, blue_style,
            "full TextStyle equality should include exact source handles"
        );
        assert!(
            red_style.layout_eq_excluding_visuals(&blue_style),
            "text material handles do not affect measurement layout"
        );
        assert!(
            red_style.gating_eq(&blue_style),
            "text material handles update frame rows without rebuilding glyph geometry"
        );

        let mut red_hash = DefaultHasher::new();
        red_style.hash_layout(&mut red_hash);
        let mut blue_hash = DefaultHasher::new();
        blue_style.hash_layout(&mut blue_hash);
        assert_eq!(red_hash.finish(), blue_hash.finish());
    }

    #[test]
    fn for_shaping_drops_render_cascade_authoring() {
        let prepared = TextStyle::new(crate::Pt(24.0))
            .with_alpha_mode(AlphaMode::Add)
            .with_hdr_text_coverage_bias(2.0)
            .for_shaping(Anchor::TopLeft);

        assert_eq!(
            prepared.unit,
            Cascade::Inherit,
            "unit authoring routes through FontUnit, not the per-run view"
        );
        assert_eq!(
            prepared.alpha_mode,
            Cascade::Inherit,
            "alpha authoring routes through TextAlpha, not the per-run view"
        );
        assert_eq!(
            prepared.hdr_text_coverage_bias,
            Cascade::Inherit,
            "HDR coverage authoring routes through HdrTextCoverageBias, not the per-run view"
        );
    }

    #[test]
    fn gating_eq_distinguishes_signed_zero() {
        // to_bits, not ==: +0.0 and -0.0 are distinct bit patterns, matching
        // the layout layer's own comparison.
        let positive = TextStyle::new(24.0).with_line_height(0.0);
        let negative = TextStyle::new(24.0).with_line_height(-0.0);
        assert!(!positive.gating_eq(&negative));
    }

    #[test]
    fn anchor_offset_top_left_is_zero() {
        let (x, y) = Anchor::TopLeft.offset(100.0, 50.0);
        assert!((x).abs() < f32::EPSILON);
        assert!((y).abs() < f32::EPSILON);
    }

    #[test]
    fn anchor_offset_center_is_half() {
        let (x, y) = Anchor::Center.offset(100.0, 50.0);
        assert!((x - 50.0).abs() < f32::EPSILON);
        assert!((y - 25.0).abs() < f32::EPSILON);
    }

    #[test]
    fn anchor_offset_bottom_right_is_full() {
        let (x, y) = Anchor::BottomRight.offset(100.0, 50.0);
        assert!((x - 100.0).abs() < f32::EPSILON);
        assert!((y - 50.0).abs() < f32::EPSILON);
    }

    // ── BoundingBox::intersect ─────────────────────────────────────

    fn bbox(x: f32, y: f32, width: f32, height: f32) -> BoundingBox {
        BoundingBox {
            x,
            y,
            width,
            height,
        }
    }

    fn approx_eq(a: &BoundingBox, b: &BoundingBox) -> bool {
        (a.x - b.x).abs() < f32::EPSILON
            && (a.y - b.y).abs() < f32::EPSILON
            && (a.width - b.width).abs() < f32::EPSILON
            && (a.height - b.height).abs() < f32::EPSILON
    }

    #[test]
    fn intersect_overlapping_boxes() {
        let a = bbox(0.0, 0.0, 10.0, 10.0);
        let b = bbox(5.0, 5.0, 10.0, 10.0);
        let result = a.intersect(&b).expect("should overlap");
        assert!(approx_eq(&result, &bbox(5.0, 5.0, 5.0, 5.0)));
    }

    #[test]
    fn intersect_contained_box() {
        let outer = bbox(0.0, 0.0, 100.0, 100.0);
        let inner = bbox(10.0, 20.0, 30.0, 40.0);
        let result = outer.intersect(&inner).expect("should overlap");
        assert!(approx_eq(&result, &inner));
    }

    #[test]
    fn intersect_disjoint_boxes() {
        let a = bbox(0.0, 0.0, 10.0, 10.0);
        let b = bbox(20.0, 20.0, 10.0, 10.0);
        assert!(a.intersect(&b).is_none());
    }

    #[test]
    fn intersect_touching_edges() {
        let a = bbox(0.0, 0.0, 10.0, 10.0);
        let b = bbox(10.0, 0.0, 10.0, 10.0);
        assert!(a.intersect(&b).is_none());
    }

    #[test]
    fn intersect_zero_size_box() {
        let a = bbox(5.0, 5.0, 0.0, 0.0);
        let b = bbox(0.0, 0.0, 10.0, 10.0);
        assert!(a.intersect(&b).is_none());
    }

    #[test]
    fn intersect_identical_boxes() {
        let a = bbox(10.0, 20.0, 30.0, 40.0);
        let result = a.intersect(&a).expect("should overlap");
        assert!(approx_eq(&result, &a));
    }
}
