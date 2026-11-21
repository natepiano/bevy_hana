//! Shared button surfaces and interaction appearances.

use bevy::prelude::Color;

use super::Button;
use crate::Appearance;
use crate::Border;
use crate::ControlLength;
use crate::ControlMetrics;
use crate::CornerRadius;
use crate::El;
use crate::WidgetElement;

pub(super) const BORDER_FOCUSED: Color = Color::srgb(0.41, 0.68, 1.0);
pub(super) const BORDER_HOVERED: Color = Color::srgb(0.75, 0.78, 0.83);
pub(super) const BORDER_PRESSED: Color = Color::srgb(0.60, 0.62, 0.66);
const BORDER_DISABLED: Color = Color::srgb(0.26, 0.28, 0.31);
pub(super) const BORDER: Color = Color::srgb(0.66, 0.69, 0.74);
pub(super) const FILL: Color = Color::srgb(0.20, 0.22, 0.26);
const FILL_DISABLED: Color = Color::srgb(0.11, 0.12, 0.14);
const FILL_HOVERED: Color = Color::srgb(0.27, 0.30, 0.35);
const FILL_PRESSED: Color = Color::srgb(0.16, 0.18, 0.21);
pub(super) const TEXT: Color = Color::srgb(0.94, 0.95, 0.97);
const TEXT_DISABLED: Color = Color::srgb(0.52, 0.54, 0.57);

pub(super) fn button_face<L, Length: ControlLength>(
    element: El<L, WidgetElement<Button>>,
    metrics: &ControlMetrics<Length>,
) -> El<L, WidgetElement<Button>> {
    face_with_colors(
        element,
        metrics,
        FaceColors {
            fill:    FILL,
            hovered: FILL_HOVERED,
            pressed: FILL_PRESSED,
        },
        InteractionBorder::ChangesWithState,
    )
}

/// Draws a button face filled with `fill`, keeping that tone when hovered and pressed.
pub(in crate::widgets::controls) fn tinted_button_face<L, Length: ControlLength>(
    element: El<L, WidgetElement<Button>>,
    metrics: &ControlMetrics<Length>,
    fill: Color,
) -> El<L, WidgetElement<Button>> {
    tinted_face(element, metrics, fill, InteractionBorder::ChangesWithState)
}

/// Keeps the default ring fixed while the shared face fill shows interaction.
pub(in crate::widgets::controls) fn default_button_face<L, Length: ControlLength>(
    element: El<L, WidgetElement<Button>>,
    metrics: &ControlMetrics<Length>,
    fill: Color,
    ring: Color,
) -> El<L, WidgetElement<Button>> {
    tinted_face(element, metrics, fill, InteractionBorder::Fixed(ring))
        .border(Border::all(metrics.focus_border_width, ring))
}

fn tinted_face<L, Length: ControlLength>(
    element: El<L, WidgetElement<Button>>,
    metrics: &ControlMetrics<Length>,
    fill: Color,
    border: InteractionBorder,
) -> El<L, WidgetElement<Button>> {
    let tint = fill.to_srgba();
    let scaled = |factor: f32| {
        Color::srgba(
            (tint.red * factor).min(1.0),
            (tint.green * factor).min(1.0),
            (tint.blue * factor).min(1.0),
            tint.alpha,
        )
    };
    face_with_colors(
        element,
        metrics,
        FaceColors {
            fill,
            hovered: scaled(FILL.to_srgba().red / FILL_HOVERED.to_srgba().red),
            pressed: scaled(FILL_PRESSED.to_srgba().red / FILL_HOVERED.to_srgba().red),
        },
        border,
    )
}

#[derive(Clone, Copy)]
enum InteractionBorder {
    ChangesWithState,
    Fixed(Color),
}

impl InteractionBorder {
    const fn color(self, ordinary: Color) -> Color {
        match self {
            Self::ChangesWithState => ordinary,
            Self::Fixed(color) => color,
        }
    }
}

struct FaceColors {
    fill:    Color,
    hovered: Color,
    pressed: Color,
}

fn face_with_colors<L, Length: ControlLength>(
    element: El<L, WidgetElement<Button>>,
    metrics: &ControlMetrics<Length>,
    colors: FaceColors,
    border: InteractionBorder,
) -> El<L, WidgetElement<Button>> {
    element
        .background(colors.fill)
        .border(Border::all(metrics.border_width, BORDER))
        .corner_radius(CornerRadius::all(metrics.button.corner_radius))
        .hovered(
            Appearance::new()
                .background(colors.hovered)
                .border_color(border.color(BORDER_HOVERED)),
        )
        .pressed(
            Appearance::new()
                .background(colors.pressed)
                .border_color(border.color(BORDER_PRESSED)),
        )
        .focused(
            Appearance::new()
                .border_color(border.color(BORDER_FOCUSED))
                .border_width(metrics.focus_border_width),
        )
        .disabled(
            Appearance::new()
                .background(FILL_DISABLED)
                .border_color(BORDER_DISABLED)
                .text_color(TEXT_DISABLED)
                .path_color(TEXT_DISABLED),
        )
}
