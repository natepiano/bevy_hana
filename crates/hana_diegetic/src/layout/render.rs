//! Render commands produced by the layout engine.

use bevy::asset::Handle;
use bevy::color::Color;
use bevy::image::Image;

use super::Border;
use super::BoundingBox;
use super::DrawZIndex;
use super::ResolvedPanelShape;
use super::TextStyle;

/// A single render command produced by the layout pass.
///
/// The layout engine outputs a flat, ordered list of these commands.
/// Consumers iterate them to draw rectangles, text, borders, and clip regions.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderCommand {
    /// Computed bounding box in layout coordinates.
    pub bounds:      BoundingBox,
    /// What to render.
    pub kind:        RenderCommandKind,
    /// Index of the source element in the `LayoutTree`.
    pub element_idx: usize,
    /// Authored draw layer from the source element.
    pub z_index:     DrawZIndex,
}

/// Distinguishes the origin of a [`RenderCommandKind::Rectangle`] command.
///
/// Matching Clay, both element backgrounds and row/column child dividers are
/// emitted as `Rectangle` commands. This enum lets consumers apply the correct
/// color source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectangleSource {
    /// `Element` background fill.
    Background,
    /// Separator from a row or column [`ChildDivider`](super::ChildDivider).
    ChildDivider {
        /// Zero-based adjacent-child pair ordinal inside the parent element.
        ordinal: usize,
    },
}

/// Fixed coarse sort tier derived from a [`RenderCommandKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawSortTier {
    /// [`RenderCommandKind::Rectangle`], [`RenderCommandKind::Border`],
    /// [`RenderCommandKind::Image`], and [`RenderCommandKind::PrecomposeLdr`]
    /// commands.
    Surface,
    /// [`RenderCommandKind::PanelShapes`] commands.
    PanelShape,
    /// [`RenderCommandKind::Text`] commands.
    Text,
}

impl DrawSortTier {
    /// Returns the stable ordinal for `DrawSortTier` sort keys.
    #[must_use]
    pub(crate) const fn sort_order(self) -> u8 {
        match self {
            Self::Surface => 0,
            Self::PanelShape => 1,
            Self::Text => 2,
        }
    }
}

/// GPU batch family derived from a [`RenderCommandKind`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum DrawBatchFamily {
    /// Textured image batches for image and precompose commands.
    Image,
    /// SDF surface batches for rectangle and border commands.
    SdfSurface,
    /// Analytic path batches for panel-shape commands.
    PanelShape,
    /// Analytic path batches for panel-text commands.
    Text,
}

/// The specific visual to render.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderCommandKind {
    /// A filled rectangle.
    Rectangle {
        /// Fill color.
        color:  Color,
        /// Whether this rectangle is a background or child divider.
        source: RectangleSource,
    },
    /// A text string.
    Text {
        /// The text content.
        text:   String,
        /// Text configuration (font, size, etc.).
        config: TextStyle,
    },
    /// A border outline.
    Border {
        /// Border specification.
        border: Border,
    },
    /// An image (textured quad).
    Image {
        /// Handle to the image asset.
        handle: Handle<Image>,
        /// Tint color multiplied against the texture (white = no tint).
        tint:   Color,
    },
    /// An element subtree flattened through an LDR render target.
    PrecomposeLdr,
    /// Resolved panel-local line primitives.
    PanelShapes {
        /// Resolved lines to render as one command group.
        shapes: Vec<ResolvedPanelShape>,
    },
    /// Begin a clipping region. All subsequent commands until the matching
    /// [`ScissorEnd`](Self::ScissorEnd) are clipped to this bounding box.
    ScissorStart,
    /// End the most recent clipping region.
    ScissorEnd,
}

impl RenderCommandKind {
    /// Returns the fixed [`DrawSortTier`] for commands that draw pixels.
    #[must_use]
    pub(crate) const fn draw_sort_tier(&self) -> Option<DrawSortTier> {
        match self {
            Self::Rectangle { .. }
            | Self::Border { .. }
            | Self::Image { .. }
            | Self::PrecomposeLdr => Some(DrawSortTier::Surface),
            Self::PanelShapes { .. } => Some(DrawSortTier::PanelShape),
            Self::Text { .. } => Some(DrawSortTier::Text),
            Self::ScissorStart | Self::ScissorEnd => None,
        }
    }

    /// Returns the GPU batch family for commands routed through a shared batch.
    #[must_use]
    pub(crate) const fn draw_batch_family(&self) -> Option<DrawBatchFamily> {
        match self {
            Self::Rectangle { .. } | Self::Border { .. } => Some(DrawBatchFamily::SdfSurface),
            Self::PanelShapes { .. } => Some(DrawBatchFamily::PanelShape),
            Self::Text { .. } => Some(DrawBatchFamily::Text),
            Self::Image { .. } | Self::PrecomposeLdr => Some(DrawBatchFamily::Image),
            Self::ScissorStart | Self::ScissorEnd => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILL_ORDINAL: u8 = 0;
    const SHAPES_ORDINAL: u8 = 1;
    const TEXT_ORDINAL: u8 = 2;

    #[test]
    fn draw_step_ordinals_are_fixed() {
        assert_eq!(DrawSortTier::Surface.sort_order(), FILL_ORDINAL);
        assert_eq!(DrawSortTier::PanelShape.sort_order(), SHAPES_ORDINAL);
        assert_eq!(DrawSortTier::Text.sort_order(), TEXT_ORDINAL);
    }

    #[test]
    fn render_command_kinds_map_to_draw_steps() {
        let cases = [
            (
                RenderCommandKind::Rectangle {
                    color:  Color::WHITE,
                    source: RectangleSource::Background,
                },
                Some(DrawSortTier::Surface),
            ),
            (
                RenderCommandKind::Border {
                    border: Border::default(),
                },
                Some(DrawSortTier::Surface),
            ),
            (
                RenderCommandKind::Image {
                    handle: Handle::<Image>::default(),
                    tint:   Color::WHITE,
                },
                Some(DrawSortTier::Surface),
            ),
            (
                RenderCommandKind::PanelShapes { shapes: Vec::new() },
                Some(DrawSortTier::PanelShape),
            ),
            (
                RenderCommandKind::PrecomposeLdr,
                Some(DrawSortTier::Surface),
            ),
            (
                RenderCommandKind::Text {
                    text:   String::new(),
                    config: TextStyle::default(),
                },
                Some(DrawSortTier::Text),
            ),
            (RenderCommandKind::ScissorStart, None),
            (RenderCommandKind::ScissorEnd, None),
        ];

        for (kind, expected) in cases {
            assert_eq!(kind.draw_sort_tier(), expected);
        }
    }

    #[test]
    fn render_command_kinds_map_to_batch_families() {
        let cases = [
            (
                RenderCommandKind::Rectangle {
                    color:  Color::WHITE,
                    source: RectangleSource::Background,
                },
                Some(DrawBatchFamily::SdfSurface),
            ),
            (
                RenderCommandKind::Border {
                    border: Border::default(),
                },
                Some(DrawBatchFamily::SdfSurface),
            ),
            (
                RenderCommandKind::PanelShapes { shapes: Vec::new() },
                Some(DrawBatchFamily::PanelShape),
            ),
            (
                RenderCommandKind::Text {
                    text:   String::new(),
                    config: TextStyle::default(),
                },
                Some(DrawBatchFamily::Text),
            ),
            (
                RenderCommandKind::Image {
                    handle: Handle::<Image>::default(),
                    tint:   Color::WHITE,
                },
                Some(DrawBatchFamily::Image),
            ),
            (
                RenderCommandKind::PrecomposeLdr,
                Some(DrawBatchFamily::Image),
            ),
            (RenderCommandKind::ScissorStart, None),
            (RenderCommandKind::ScissorEnd, None),
        ];

        for (kind, expected) in cases {
            assert_eq!(kind.draw_batch_family(), expected);
        }
    }
}
