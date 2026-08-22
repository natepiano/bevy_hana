//! Fluent text primitive: [`DiegeticText`].
//!
//! [`DiegeticText::world`] / [`DiegeticText::screen`] start a
//! [`DiegeticTextBuilder`] that produces a one-element [`DiegeticPanel`] holding
//! a single text child, so wrapping, lighting, and the unified [`TextStyle`] come
//! from the panel-text pipeline rather than a separate standalone render path.
//! This mirrors [`DiegeticPanel::world`] / [`DiegeticPanel::screen`]; the string
//! is the constructor argument because, unlike a panel (a container sized later),
//! a text label is a filled value whose string is its one required input.
//!
//! The chain ends in [`DiegeticTextBuilder::spawn`], which builds the panel plus
//! its text child and returns the **panel** entity. That entity carries the
//! [`DiegeticText`] marker, so a single label is addressable via
//! `With<DiegeticText>`; the string itself is stored once, on the spawned run's
//! [`TextContent`](crate::TextContent).
//!
//! ```ignore
//! // World text, fit to content, anchored at its center:
//! let panel = DiegeticText::world("Hello")
//!     .size(0.16)
//!     .color(Color::WHITE)
//!     .anchor(Anchor::Center)
//!     .transform(Transform::from_xyz(0.0, 2.0, 0.0))
//!     .spawn(&mut commands);
//!
//! // Screen text, wrapped to 320 px, positioned in pixels:
//! DiegeticText::screen("Paused")
//!     .size(48.0)
//!     .width(320.0)
//!     .screen_position(40.0, 40.0)
//!     .spawn(&mut commands);
//! ```

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::Bundle;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Plugin;
use bevy::prelude::Reflect;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::error;

use crate::layout::Anchor;
use crate::layout::Dimension;
use crate::layout::El;
use crate::layout::FontFeatures;
use crate::layout::FontSlant;
use crate::layout::FontWeight;
use crate::layout::GlyphRenderMode;
use crate::layout::GlyphShadowMode;
use crate::layout::LayoutBuilder;
use crate::layout::LayoutTree;
use crate::layout::Lighting;
use crate::layout::Px;
use crate::layout::Sidedness;
use crate::layout::Sizing;
use crate::layout::Text;
use crate::layout::TextAlign;
use crate::layout::TextStyle;
use crate::layout::TextWrap;
use crate::layout::Unit;
use crate::panel::DiegeticPanel;
use crate::panel::Fit;

/// Marker on the panel-root entity a [`DiegeticTextBuilder`] spawns.
///
/// A one-element text panel — the runtime form of [`DiegeticText::world`] /
/// [`DiegeticText::screen`] — carries this so a single label is queryable via
/// `With<DiegeticText>`. The string is stored on the spawned run's
/// [`TextContent`](crate::TextContent), not here, and the coordinate space on
/// the panel's `CoordinateSpace`; the marker holds no state of its own, so
/// there is nothing on it to drift from the panel.
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
pub struct DiegeticText;

impl DiegeticText {
    /// Starts a world-space text builder with the given string and default
    /// style. The string is the one required input; size/anchor/wrap default.
    #[must_use]
    pub fn world(text: impl Into<String>) -> DiegeticTextBuilder {
        DiegeticTextBuilder::new(TextSpace::World, text)
    }

    /// Starts a screen-space (overlay) text builder with the given string and
    /// default style.
    #[must_use]
    pub fn screen(text: impl Into<String>) -> DiegeticTextBuilder {
        DiegeticTextBuilder::new(TextSpace::Screen, text)
    }
}

/// Which coordinate space a [`DiegeticTextBuilder`] resolves to at build time.
///
/// Chosen by the `DiegeticText::world` / `screen` constructor and never mutated
/// after; the space-specific setters take effect only for the matching space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextSpace {
    World,
    Screen,
}

/// Generates the shared typography setters as inherent methods, so callers reach
/// `.size()` / `.bold()` / `.color()` directly without importing a trait.
macro_rules! text_style_setters {
    ($ty:ty) => {
        impl $ty {
            /// Sets the font size. Accepts [`Pt`](crate::Pt), [`Mm`](crate::Mm),
            /// [`In`](crate::In), [`Px`], or a bare `f32` (resolved from the
            /// contextual unit: world meters for world text, pixels for screen).
            #[must_use]
            pub fn size(mut self, size: impl Into<Dimension>) -> Self {
                self.style.set_dimension(size);
                self
            }

            /// Sets the font by identifier.
            #[must_use]
            pub const fn font(mut self, font_id: u16) -> Self {
                self.style.set_font_id(font_id);
                self
            }

            /// Sets the font weight.
            #[must_use]
            pub const fn weight(mut self, weight: FontWeight) -> Self {
                self.style.set_weight(weight);
                self
            }

            /// Shorthand for [`Self::weight`] with [`FontWeight::BOLD`].
            #[must_use]
            pub const fn bold(mut self) -> Self {
                self.style.set_weight(FontWeight::BOLD);
                self
            }

            /// Sets the font slant.
            #[must_use]
            pub const fn slant(mut self, slant: FontSlant) -> Self {
                self.style.set_slant(slant);
                self
            }

            /// Shorthand for [`Self::slant`] with [`FontSlant::Italic`].
            #[must_use]
            pub const fn italic(mut self) -> Self {
                self.style.set_slant(FontSlant::Italic);
                self
            }

            /// Sets the line height in the font's unit. `0.0` = derive from size.
            #[must_use]
            pub const fn line_height(mut self, line_height: f32) -> Self {
                self.style.set_line_height(line_height);
                self
            }

            /// Sets extra spacing between characters.
            #[must_use]
            pub const fn letter_spacing(mut self, spacing: f32) -> Self {
                self.style.set_letter_spacing(spacing);
                self
            }

            /// Sets extra spacing between words.
            #[must_use]
            pub const fn word_spacing(mut self, spacing: f32) -> Self {
                self.style.set_word_spacing(spacing);
                self
            }

            /// Sets the text color.
            #[must_use]
            pub const fn color(mut self, color: Color) -> Self {
                self.style.set_color(color);
                self
            }

            /// Sets horizontal alignment of glyphs within the measured run.
            #[must_use]
            pub const fn align(mut self, align: TextAlign) -> Self {
                self.style.set_align(align);
                self
            }

            /// Sets the glyph render mode.
            #[must_use]
            pub const fn render_mode(mut self, mode: GlyphRenderMode) -> Self {
                self.style.set_render_mode(mode);
                self
            }

            /// Sets the glyph shadow mode.
            #[must_use]
            pub const fn shadow_mode(mut self, mode: GlyphShadowMode) -> Self {
                self.style.set_shadow_mode(mode);
                self
            }

            /// Overrides glyph sidedness for this text (else inherits the
            /// context default via the `Sidedness` cascade attribute).
            #[must_use]
            pub const fn sidedness(mut self, sidedness: Sidedness) -> Self {
                self.style.set_sidedness(sidedness);
                self
            }

            /// Overrides glyph lighting for this text (else inherits the context
            /// default via the `Lighting` cascade attribute).
            #[must_use]
            pub const fn lighting(mut self, lighting: Lighting) -> Self {
                self.style.set_lighting(lighting);
                self
            }

            /// Shorthand for [`Self::lighting`] with [`Lighting::Unlit`].
            #[must_use]
            pub const fn unlit(mut self) -> Self {
                self.style.set_lighting(Lighting::Unlit);
                self
            }

            /// Sets OpenType font feature overrides.
            #[must_use]
            pub const fn font_features(mut self, features: FontFeatures) -> Self {
                self.style.set_font_features(features);
                self
            }

            /// Sets the per-text [`AlphaMode`] override.
            #[must_use]
            pub const fn alpha_mode(mut self, alpha_mode: AlphaMode) -> Self {
                self.style.set_alpha_mode(alpha_mode);
                self
            }

            /// Replaces the whole [`TextStyle`] at once. Later chained setters
            /// still apply on top of it.
            #[must_use]
            pub fn style(mut self, style: TextStyle) -> Self {
                self.style = style;
                self
            }

            /// Wraps the text to this fixed width (in the font's unit). Absent →
            /// fit-to-content with no wrapping. Explicit `\n` always breaks
            /// regardless of wrap mode.
            #[must_use]
            pub const fn width(mut self, width: f32) -> Self {
                self.wrap_width = Some(width);
                self
            }
        }
    };
}

/// Fluent builder for a one-element text label, world- or screen-space.
///
/// Spawns a one-element [`DiegeticPanel`] sized to fit (or to [`Self::width`])
/// and returns the panel entity from [`Self::spawn`]. Start it with
/// [`DiegeticText::world`] or [`DiegeticText::screen`]. The world-only setters
/// ([`Self::world_width`] / [`Self::world_height`] / [`Self::transform`]) and the
/// screen-only setters ([`Self::screen_position`] / [`Self::camera_order`] /
/// [`Self::render_layers`]) take effect only for the matching space; the other
/// space ignores them.
pub struct DiegeticTextBuilder {
    space:         TextSpace,
    text:          String,
    style:         TextStyle,
    wrap_width:    Option<f32>,
    world_width:   Option<f32>,
    world_height:  Option<f32>,
    anchor:        Option<Anchor>,
    transform:     Transform,
    position:      Option<Vec2>,
    render_layers: Option<RenderLayers>,
    camera_order:  Option<isize>,
}

text_style_setters!(DiegeticTextBuilder);

impl DiegeticTextBuilder {
    fn new(space: TextSpace, text: impl Into<String>) -> Self {
        // World text sits *at* its point, so it centers on the transform by
        // default; screen text pins to the window edge via its panel anchor, so
        // it starts unanchored (the screen builder seeds `TopLeft`).
        let anchor = match space {
            TextSpace::World => Some(Anchor::Center),
            TextSpace::Screen => None,
        };
        Self {
            space,
            text: text.into(),
            style: TextStyle::default(),
            wrap_width: None,
            world_width: None,
            world_height: None,
            anchor,
            transform: Transform::IDENTITY,
            position: None,
            render_layers: None,
            camera_order: None,
        }
    }

    /// Sets the anchor point.
    ///
    /// For world text (default [`Anchor::Center`]) this places the label box at
    /// its transform — [`Anchor::TopLeft`] hangs the box from its top-left corner
    /// instead. For screen text it places the label within the window;
    /// [`Anchor::Center`] centers it on screen.
    #[must_use]
    pub const fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = Some(anchor);
        self
    }

    /// (World only) Scales the label so its whole world width matches `meters`;
    /// height follows the aspect ratio. Combines with [`Self::world_height`] for
    /// non-uniform scaling.
    #[must_use]
    pub const fn world_width(mut self, meters: f32) -> Self {
        self.world_width = Some(meters);
        self
    }

    /// (World only) Scales the label so its whole world height matches `meters`;
    /// width follows the aspect ratio.
    #[must_use]
    pub const fn world_height(mut self, meters: f32) -> Self {
        self.world_height = Some(meters);
        self
    }

    /// (World only) Places the label at this world transform.
    #[must_use]
    pub const fn transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    /// (Screen only) Places the label at an explicit pixel position (top-left
    /// origin, y-down).
    #[must_use]
    pub const fn screen_position(mut self, x: f32, y: f32) -> Self {
        self.position = Some(Vec2::new(x, y));
        self
    }

    /// (Screen only) Sets the overlay camera render order (default `100`).
    #[must_use]
    pub const fn camera_order(mut self, order: isize) -> Self {
        self.camera_order = Some(order);
        self
    }

    /// (Screen only) Sets the render layers for camera isolation (default layer
    /// 31).
    #[must_use]
    pub fn render_layers(mut self, layers: RenderLayers) -> Self {
        self.render_layers = Some(layers);
        self
    }

    /// Builds the one-element panel as a [`Bundle`], for composing with marker
    /// components or spawning under `with_children`. [`Self::spawn`] is the
    /// terminal one-liner over this.
    ///
    /// A degenerate size (unreachable while the height is always `Fit`) logs and
    /// falls back to a default panel rather than panicking.
    #[must_use]
    pub fn build(self) -> impl Bundle {
        let tree = build_one_element_tree(&self.text, &self.style, self.wrap_width);
        let panel = match self.space {
            TextSpace::World => self.build_world_panel(tree),
            TextSpace::Screen => self.build_screen_panel(tree),
        };
        let transform = match self.space {
            TextSpace::World => self.transform,
            TextSpace::Screen => Transform::IDENTITY,
        };
        // The authoritative style is stored in the tree's `El.config` (built
        // above) and reification derives the run from it; the root carries no
        // `TextStyle`, so there is no stale copy a query could mutate to no
        // effect.
        (panel, DiegeticText, transform)
    }

    fn build_world_panel(&self, tree: LayoutTree) -> DiegeticPanel {
        let sized = match self.wrap_width {
            Some(width) => DiegeticPanel::world().size(width, Fit),
            None => DiegeticPanel::world().size(Fit, Fit),
        }
        .font_unit(Unit::Meters);
        let scaled_width = match self.world_width {
            Some(meters) => sized.world_width(meters),
            None => sized,
        };
        let scaled = match self.world_height {
            Some(meters) => scaled_width.world_height(meters),
            None => scaled_width,
        };
        let anchored = match self.anchor {
            Some(anchor) => scaled.anchor(anchor),
            None => scaled,
        };
        anchored.with_tree(tree).build().unwrap_or_else(|error| {
            error!("DiegeticText::world: {error}; falling back to a default panel");
            DiegeticPanel::default()
        })
    }

    fn build_screen_panel(&self, tree: LayoutTree) -> DiegeticPanel {
        let sized = match self.wrap_width {
            Some(width) => DiegeticPanel::screen().size(Px(width), Fit),
            None => DiegeticPanel::screen().size(Fit, Fit),
        };
        let anchored = match self.anchor {
            Some(anchor) => sized.anchor(anchor),
            None => sized,
        };
        let positioned = match self.position {
            Some(pos) => anchored.screen_position(pos.x, pos.y),
            None => anchored,
        };
        let ordered = match self.camera_order {
            Some(order) => positioned.camera_order(order),
            None => positioned,
        };
        let layered = match self.render_layers.clone() {
            Some(layers) => ordered.render_layers(layers),
            None => ordered,
        };
        layered.with_tree(tree).build().unwrap_or_else(|error| {
            error!("DiegeticText::screen: {error}; falling back to a default panel");
            DiegeticPanel::default()
        })
    }

    /// Spawns the one-element panel, returning the panel entity (the handle
    /// addressable via `With<DiegeticText>`).
    pub fn spawn(self, commands: &mut Commands) -> Entity { commands.spawn(self.build()).id() }
}

/// Builds the single-element layout tree for a [`DiegeticText`] panel.
fn build_one_element_tree(text: &str, style: &TextStyle, wrap_width: Option<f32>) -> LayoutTree {
    // The root must carry the sizing the panel resolves to: `Fit` width
    // (shrink-wrap to the text) when there is no wrap width, or a fixed wrap
    // width; height is always `Fit`. This root sizing must match what
    // `DiegeticPanel::build` produces — a `Fixed(0, 0)` root (the old
    // `LayoutBuilder::new(0.0, 0.0)`) overwrites the panel's `Fit` root and
    // collapses the measured width to zero.
    let width = wrap_width.map_or(Sizing::FIT, Sizing::fixed);
    let mut builder = LayoutBuilder::with_root(El::new().width(width).height(Sizing::FIT));
    let text = Text::new(text, style.clone());
    let text = if wrap_width.is_some() {
        text.wrap(TextWrap::Words)
    } else {
        text
    };
    builder.text(text);
    builder.build()
}

pub(crate) struct DiegeticTextPlugin;

impl Plugin for DiegeticTextPlugin {
    fn build(&self, _app: &mut App) {}
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests use expect for clearer failure messages"
)]
mod tests {
    use std::sync::Arc;

    use bevy_kana::ToF32;

    use super::*;
    use crate::layout::LayoutEngine;
    use crate::layout::MeasureTextFn;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;

    fn monospace_measure() -> MeasureTextFn {
        Arc::new(|text: &str, m: &TextMeasure| TextDimensions {
            width:       text.chars().count().to_f32() * m.size * 0.6,
            height:      m.size,
            line_height: m.size,
        })
    }

    #[test]
    fn one_element_tree_resolves_nonzero_width() {
        // Regression: `build_one_element_tree` must produce a `Fit` (shrink-wrap)
        // root, not a `Fixed(0, 0)` root, or the measured width collapses to zero
        // (no glyphs render).
        let tree = build_one_element_tree("Hello", &TextStyle::new(16.0), None);
        let engine = LayoutEngine::new(monospace_measure());
        let result = engine.compute(&tree, 0.0, 0.0, 1.0);
        let bounds = result.content_bounds().expect("content bounds");
        assert!(
            bounds.width > 0.0,
            "one-element tree width collapsed to {}",
            bounds.width
        );
    }
}
