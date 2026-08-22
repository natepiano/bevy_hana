//! Typography overlay — renders font-level metric lines and per-glyph
//! bounding boxes on any [`TextContent`](crate::TextContent) entity.
//!
//! Uses [`ComputedWorldText`](crate::render::ComputedWorldText) data
//! populated by the renderer to ensure exact alignment with the rendered
//! slug glyphs — no independent layout computation.
//!
//! Guide lines, dimension arrows, and dashed callouts are authored as
//! element-owned [`PanelDraw::lines`](crate::layout::PanelDraw::lines) on
//! transparent world panels, so they render through the shared analytic
//! path renderer with [`HairlineFade::Full`](crate::render::HairlineFade)
//! pinned (debug guides never fade with distance). Labels are spawned as
//! [`TextContent`](crate::TextContent) children.

mod constants;
mod glyph;
mod labels;
mod lifecycle;
mod metric_lines;
mod pipeline;
mod scaling;

use bevy::color::palettes::css::WHITE;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::Transform;
use constants::DEFAULT_OVERLAY_EXTEND;
use constants::DEFAULT_OVERLAY_LABEL_SIZE;
pub(super) use lifecycle::on_overlay_added;
pub(super) use lifecycle::on_overlay_removed;
pub(super) use pipeline::build_typography_overlay;

use super::constants::DEFAULT_LINE_WIDTH;
use crate::DiegeticText;
use crate::layout::GlyphShadowMode;
use crate::layout::ShadowCasting;
use crate::layout::TextStyle;

/// Spawns a world-space overlay label as a one-element [`DiegeticText`] panel
/// child of `container`.
///
/// The label's [`TextStyle`] anchor becomes the panel anchor, so the text sits at
/// `transform`. [`DiegeticText`] labels are one-element panels; a bare
/// [`TextContent`](crate::TextContent) does not render on its own.
pub(super) fn spawn_overlay_label(
    commands: &mut Commands,
    container: Entity,
    text: impl Into<String>,
    style: TextStyle,
    transform: Transform,
) {
    let anchor = style.anchor();
    commands.entity(container).with_child(
        DiegeticText::world(text)
            .style(style)
            .anchor(anchor)
            .transform(transform)
            .build(),
    );
}

/// Whether per-glyph bounding box annotations are visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum GlyphMetricVisibility {
    /// Glyph bounding boxes and origin dots are drawn.
    Shown,
    /// Glyph-level annotations are suppressed.
    Hidden,
}

/// Attach to a [`TextContent`](crate::TextContent) entity to render typography
/// metric annotations.
///
/// Built into the library as a debug tool — only available when the
/// `typography_overlay` feature is enabled.
///
/// Metric lines, dimension arrows, and dashed callouts render as
/// element-owned panel lines on transparent world panels (rebuilt only when
/// the text or overlay changes). Labels are spawned as
/// [`TextContent`](crate::TextContent) children.
///
/// # Example
///
/// ```ignore
/// commands.spawn((
///     TextContent::new("Typography"),
///     TextStyle::new(48.0),
///     TypographyOverlay::default(),
///     Transform::from_xyz(0.0, 2.0, 0.0),
/// ));
/// ```
#[derive(Component, Clone, Debug, Reflect)]
pub struct TypographyOverlay {
    /// Show font-level metric lines (ascent, descent, cap height, x-height,
    /// baseline, top, bottom).
    pub font_metrics:   GlyphMetricVisibility,
    /// Show per-glyph bounding boxes as panel-line outlines (from font bbox).
    pub glyph_metrics:  GlyphMetricVisibility,
    /// Show text labels on the metric lines.
    pub labels:         GlyphMetricVisibility,
    /// Color for overlay lines and labels (includes alpha).
    pub color:          Color,
    /// Guide line width in pixels.
    pub line_width:     f32,
    /// Font size for metric labels.
    pub label_size:     f32,
    /// How far annotation lines extend beyond text bounds (in layout units).
    pub extend:         f32,
    /// Whether overlay geometry and labels cast shadows.
    pub shadow_casting: ShadowCasting,
}

impl Default for TypographyOverlay {
    fn default() -> Self {
        Self {
            font_metrics:   GlyphMetricVisibility::Shown,
            glyph_metrics:  GlyphMetricVisibility::Shown,
            labels:         GlyphMetricVisibility::Shown,
            color:          Color::from(WHITE),
            line_width:     DEFAULT_LINE_WIDTH,
            label_size:     DEFAULT_OVERLAY_LABEL_SIZE,
            extend:         DEFAULT_OVERLAY_EXTEND,
            shadow_casting: ShadowCasting::Off,
        }
    }
}

impl TypographyOverlay {
    /// Sets whether overlay constituents cast shadows.
    #[must_use]
    pub const fn with_shadow(mut self, shadow_casting: ShadowCasting) -> Self {
        self.shadow_casting = shadow_casting;
        self
    }

    const fn label_shadow_mode(&self) -> GlyphShadowMode {
        match self.shadow_casting {
            ShadowCasting::Off => GlyphShadowMode::None,
            ShadowCasting::On => GlyphShadowMode::Cast,
        }
    }
}

/// Marker for the single container entity that holds all overlay children.
/// Spawned by [`on_overlay_added`] and despawned by [`on_overlay_removed`].
#[derive(Component)]
pub struct OverlayContainer;

/// Hidden mesh entity representing the full overlay extent for fit/home
/// operations.
#[derive(Component)]
pub struct OverlayBoundingBox;
