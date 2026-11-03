use bevy::prelude::Color;

use super::Button;
use super::ButtonPaintBounds;
use crate::AlignX;
use crate::AlignY;
use crate::Appearance;
use crate::Border;
use crate::ControlLength;
use crate::ControlMetrics;
use crate::CornerRadius;
use crate::El;
use crate::Padding;
use crate::PanelCoord;
use crate::PanelDraw;
use crate::PanelElementId;
use crate::PanelLine;
use crate::PanelPoint;
use crate::Sizing;
use crate::Text;
use crate::TextStyle;
use crate::WidgetElement;

const POP_UP_BORDER_FOCUSED: Color = Color::srgb(0.41, 0.68, 1.0);
const POP_UP_BORDER_HOVERED: Color = Color::srgb(0.75, 0.78, 0.83);
const POP_UP_BORDER_PRESSED: Color = Color::srgb(0.60, 0.62, 0.66);
const POP_UP_BORDER_DISABLED: Color = Color::srgb(0.26, 0.28, 0.31);
const POP_UP_BORDER: Color = Color::srgb(0.66, 0.69, 0.74);
const POP_UP_FILL: Color = Color::srgb(0.20, 0.22, 0.26);
const POP_UP_FILL_DISABLED: Color = Color::srgb(0.11, 0.12, 0.14);
const POP_UP_FILL_HOVERED: Color = Color::srgb(0.27, 0.30, 0.35);
const POP_UP_FILL_PRESSED: Color = Color::srgb(0.16, 0.18, 0.21);
const POP_UP_TEXT: Color = Color::srgb(0.94, 0.95, 0.97);
const POP_UP_TEXT_DISABLED: Color = Color::srgb(0.52, 0.54, 0.57);
/// Heights of the chevron ends, as fractions of the button height from its top.
const POP_UP_UPPER_CHEVRON_BASE: f32 = 0.45;
const POP_UP_UPPER_CHEVRON_TIP: f32 = 0.34;
const POP_UP_LOWER_CHEVRON_BASE: f32 = 0.55;
const POP_UP_LOWER_CHEVRON_TIP: f32 = 0.66;

impl Button {
    /// Width of a pop-up value and its chrome, in the metrics' layout unit.
    ///
    /// Pass the measured width of the widest choice at `metrics.label_size`.
    #[must_use]
    pub fn pop_up_width(text_width: f32, metrics: &ControlMetrics<f32>) -> f32 {
        2.0_f32.mul_add(
            metrics.border_width,
            metrics.button.side_padding + metrics.pop_up.text_trailing_padding + text_width,
        )
    }

    /// Presents a pop-up chooser's current value as the button itself.
    ///
    /// The caller places a separate row label beside this text leaf and names
    /// the widest choice. The button measures that choice and adds its own
    /// value padding and chevron clearance. Height, border, radius, and text
    /// size come from the same `ControlMetrics` as the surrounding controls.
    pub fn pop_up<L: ControlLength>(
        mut self,
        id: impl Into<PanelElementId>,
        current_value: impl Into<String>,
        widest_choice: impl Into<String>,
        metrics: &ControlMetrics<L>,
    ) -> Text<WidgetElement<Self>> {
        self.paint_bounds = ButtonPaintBounds::InsideLayout;
        let chevron = |from: (PanelCoord, PanelCoord), to: (PanelCoord, PanelCoord)| {
            PanelLine::new(PanelPoint::new(from.0, from.1), PanelPoint::new(to.0, to.1))
                .width(metrics.pop_up.chevron_stroke)
                .color(POP_UP_TEXT)
        };
        let left = || PanelCoord::end(metrics.pop_up.chevron_left_inset);
        let middle = || PanelCoord::end(metrics.pop_up.chevron_middle_inset);
        let right = || PanelCoord::end(metrics.pop_up.chevron_right_inset);
        let y = PanelCoord::percent;
        let value_style = TextStyle::new(metrics.label_size).with_color(POP_UP_TEXT);
        Text::new(current_value, value_style)
            .measure_as(widest_choice)
            .layout(
                El::new()
                    .height(Sizing::fixed(metrics.button.height))
                    .padding(Padding::new(
                        metrics.button.side_padding,
                        metrics.pop_up.text_trailing_padding,
                        metrics.border_width,
                        metrics.border_width,
                    ))
                    .alignment(AlignX::Left, AlignY::Center)
                    .background(POP_UP_FILL)
                    .border(Border::all(metrics.border_width, POP_UP_BORDER))
                    .draw(PanelDraw::lines([
                        chevron(
                            (left(), y(POP_UP_UPPER_CHEVRON_BASE)),
                            (middle(), y(POP_UP_UPPER_CHEVRON_TIP)),
                        ),
                        chevron(
                            (middle(), y(POP_UP_UPPER_CHEVRON_TIP)),
                            (right(), y(POP_UP_UPPER_CHEVRON_BASE)),
                        ),
                        chevron(
                            (left(), y(POP_UP_LOWER_CHEVRON_BASE)),
                            (middle(), y(POP_UP_LOWER_CHEVRON_TIP)),
                        ),
                        chevron(
                            (middle(), y(POP_UP_LOWER_CHEVRON_TIP)),
                            (right(), y(POP_UP_LOWER_CHEVRON_BASE)),
                        ),
                    ]))
                    .corner_radius(CornerRadius::all(metrics.button.corner_radius))
                    .widget(id, self)
                    .hovered(
                        Appearance::new()
                            .background(POP_UP_FILL_HOVERED)
                            .border_color(POP_UP_BORDER_HOVERED),
                    )
                    .pressed(
                        Appearance::new()
                            .background(POP_UP_FILL_PRESSED)
                            .border_color(POP_UP_BORDER_PRESSED),
                    )
                    .focused(
                        Appearance::new()
                            .border_color(POP_UP_BORDER_FOCUSED)
                            .border_width(metrics.focus_border_width),
                    )
                    .disabled(
                        Appearance::new()
                            .background(POP_UP_FILL_DISABLED)
                            .border_color(POP_UP_BORDER_DISABLED)
                            .text_color(POP_UP_TEXT_DISABLED)
                            .path_color(POP_UP_TEXT_DISABLED),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hana_kana::ToF32;

    use super::Button;
    use super::Color;
    use super::POP_UP_BORDER;
    use super::POP_UP_BORDER_FOCUSED;
    use super::POP_UP_BORDER_HOVERED;
    use super::POP_UP_BORDER_PRESSED;
    use super::POP_UP_FILL;
    use super::POP_UP_TEXT;
    use crate::ControlMetrics;
    use crate::LayoutBuilder;
    use crate::layout::LayoutEngine;
    use crate::layout::RectangleSource;
    use crate::layout::RenderCommandKind;
    use crate::layout::TextDimensions;

    fn contrast(foreground: Color, background: Color) -> f32 {
        let luminance = |color: Color| {
            let rgba = color.to_srgba();
            let linear = |channel: f32| {
                if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126_f32.mul_add(
                linear(rgba.red),
                0.7152_f32.mul_add(linear(rgba.green), 0.0722 * linear(rgba.blue)),
            )
        };
        let (light, dark) = (luminance(foreground), luminance(background));
        (light.max(dark) + 0.05) / (light.min(dark) + 0.05)
    }

    #[test]
    fn shared_outlines_and_tracks_clear_card_and_group_stock() {
        let stroke = include_str!("../../../render/sdf_stroke.wesl");
        let panel = include_str!("../../../shaders/sdf_panel.wesl");
        assert!(stroke.contains("return length(vec2<f32>(dpdx(dist), dpdy(dist)));"));
        assert!(
            stroke.contains("let stroke_aa = max(distance_field_band(stroke_center), 0.0001);")
        );
        assert!(
            stroke.contains("let effective_half_width = max(stroke_half_width, 0.5 * stroke_aa);")
        );
        assert!(stroke.contains(
            "let stroke_shape = 1.0 - smoothstep(\n        effective_half_width - stroke_aa,\n        effective_half_width + stroke_aa,\n        abs(stroke_center),\n    );"
        ));
        assert!(stroke.contains("return 1.0 - pow(1.0 - stroke_shape, 1.0 + bias);"));
        assert!(panel.contains(
            "let thin_border_mix = select(1.0, 0.0, 2.0 * stroke_half_width >= thin_stroke_limit);"
        ));
        assert!(panel.contains("outer_alpha = max(outer_alpha, border_alpha);"));
        let bias = 4.0_f32;
        assert!(stroke.contains(&format!("let bias = {bias:.1};")));
        let smoothstep = |edge0: f32, edge1: f32, value: f32| {
            let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * 2.0_f32.mul_add(-t, 3.0)
        };
        let border_coverage = |width_px: f32, distance_px: f32| {
            let half_width = width_px * 0.5;
            let effective_half_width = half_width.max(0.5);
            let shape = 1.0
                - smoothstep(
                    effective_half_width - 1.0,
                    effective_half_width + 1.0,
                    distance_px,
                );
            let thin_mix = if width_px < 1.0 { 1.0 } else { 0.0 };
            let thin = 1.0 - (1.0 - shape).powf(1.0 + bias);
            shape.mul_add(1.0 - thin_mix, thin * thin_mix)
        };
        let coverage = border_coverage(0.25, 0.5);
        let blend = |ink: Color, stock: Color| {
            let ink = ink.to_linear();
            let stock = stock.to_linear();
            Color::linear_rgb(
                ink.red.mul_add(coverage, stock.red * (1.0 - coverage)),
                ink.green.mul_add(coverage, stock.green * (1.0 - coverage)),
                ink.blue.mul_add(coverage, stock.blue * (1.0 - coverage)),
            )
        };
        for stock in [
            Color::srgb(11.0 / 255.0, 25.0 / 255.0, 46.0 / 255.0),
            Color::srgb(46.0 / 255.0, 52.0 / 255.0, 64.0 / 255.0),
            Color::srgb(57.0 / 255.0, 63.0 / 255.0, 75.0 / 255.0),
            Color::srgb(0.20, 0.225, 0.27),
        ] {
            for outline in [
                POP_UP_BORDER,
                POP_UP_BORDER_HOVERED,
                POP_UP_BORDER_PRESSED,
                POP_UP_BORDER_FOCUSED,
            ] {
                assert!(contrast(blend(outline, stock), stock) >= 3.0);
            }
        }
        assert_eq!(border_coverage(3.0, 0.0), 1.0);
        assert_eq!(border_coverage(3.0, 0.5), 1.0);
        assert_ne!(POP_UP_BORDER, POP_UP_BORDER_HOVERED);
    }

    #[test]
    fn pop_up_draws_its_resting_surface_and_current_value() {
        let metrics = ControlMetrics::from_label_size(13.0_f32);
        let mut builder = LayoutBuilder::new(80.0, 30.0);
        builder.text(Button::new().pop_up("choice", "Opaque", "Transparent", &metrics));
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions {
            width:       20.0,
            height:      13.0,
            line_height: 13.0,
        }));
        let result = engine.compute(&tree, 80.0, 30.0, 1.0);

        assert!(result.commands.iter().any(|command| matches!(
            &command.kind,
            RenderCommandKind::Rectangle {
                color,
                source: RectangleSource::Background,
            } if *color == POP_UP_FILL
        )));
        assert!(result.commands.iter().any(|command| matches!(
            &command.kind,
            RenderCommandKind::Border { border }
                if border.color == POP_UP_BORDER
                    && (border.left.value - metrics.border_width).abs() < 1e-4
        )));
        assert!(result.commands.iter().any(|command| matches!(
            &command.kind,
            RenderCommandKind::Text { text, .. } if text == "Opaque"
        )));
    }

    #[test]
    fn pop_up_width_comes_from_its_widest_choice_and_chrome() {
        let metrics = ControlMetrics::from_label_size(13.0_f32);
        let mut builder = LayoutBuilder::new(120.0, 30.0);
        builder.text(Button::new().pop_up("choice", "A", "Transparent", &metrics));
        let result = LayoutEngine::new(Arc::new(|text, _| TextDimensions {
            width:       text.chars().count().to_f32() * 5.0,
            height:      13.0,
            line_height: 13.0,
        }))
        .compute(&builder.build(), 120.0, 30.0, 1.0);
        let button = result.commands.iter().find(|command| {
            matches!(
                command.kind,
                RenderCommandKind::Rectangle {
                    source: RectangleSource::Background,
                    ..
                }
            )
        });
        assert!(button.is_some());
        let Some(button) = button else { return };
        let expected = Button::pop_up_width(11.0 * 5.0, &metrics);

        assert!((button.bounds.width - expected).abs() < 1e-4);
    }

    #[test]
    fn pop_up_value_keeps_side_inset_and_centres_in_control_content() {
        let metrics = ControlMetrics::from_label_size(13.0_f32);
        let mut builder = LayoutBuilder::new(80.0, 30.0);
        builder.text(Button::new().pop_up("choice", "Opaque", "Transparent", &metrics));
        let result = LayoutEngine::new(Arc::new(|_, _| TextDimensions {
            width:       20.0,
            height:      13.0,
            line_height: 13.0,
        }))
        .compute(&builder.build(), 80.0, 30.0, 1.0);
        let button = result.commands.iter().find(|command| {
            matches!(
                command.kind,
                RenderCommandKind::Rectangle {
                    source: RectangleSource::Background,
                    ..
                }
            )
        });
        let value = result.commands.iter().find(|command| {
            matches!(
                &command.kind,
                RenderCommandKind::Text { text, .. } if text == "Opaque"
            )
        });
        assert!(button.is_some() && value.is_some());
        let (Some(button), Some(value)) = (button, value) else {
            return;
        };
        assert!(
            (value.bounds.x - button.bounds.x - metrics.border_width - metrics.button.side_padding)
                .abs()
                < 1e-4
        );
        // This dimension-only measurer uses the layout fallback. The drawn
        // control test supplies real font metrics and checks cap-to-baseline.
        assert!(
            f32::mul_add(
                button.bounds.height,
                -0.5,
                f32::mul_add(value.bounds.height, 0.5, value.bounds.y) - button.bounds.y
            )
            .abs()
                < 1e-4
        );
    }

    #[test]
    fn pop_up_draws_trailing_up_down_chevrons() {
        let metrics = ControlMetrics::from_label_size(13.0_f32);
        let mut builder = LayoutBuilder::new(80.0, 30.0);
        builder.text(Button::new().pop_up(
            "choice",
            "A long choice value",
            "A long choice value",
            &metrics,
        ));
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions {
            width:       80.0,
            height:      13.0,
            line_height: 13.0,
        }));
        let result = engine.compute(&tree, 80.0, 30.0, 1.0);
        let shapes = result
            .commands
            .iter()
            .find_map(|command| match &command.kind {
                RenderCommandKind::PanelShapes { shapes } => Some(shapes),
                _ => None,
            });
        assert!(
            shapes.is_some(),
            "the pop-up should emit a paint-only chevron pair"
        );
        let Some(shapes) = shapes else { return };
        assert_eq!(shapes.len(), 4);
        let owner = shapes[0].owner_bounds();
        let right_edge = owner.x + owner.width;
        for shape in shapes {
            assert_eq!(shape.color(), POP_UP_TEXT);
            assert!((shape.width() - metrics.pop_up.chevron_stroke).abs() < 1e-4);
            for point in [shape.start(), shape.end()] {
                assert!(point.x >= right_edge - metrics.pop_up.chevron_left_inset - 1e-4);
                assert!(point.x <= right_edge - metrics.pop_up.chevron_right_inset + 1e-4);
            }
        }
        assert!(metrics.pop_up.text_trailing_padding > metrics.pop_up.chevron_left_inset);
        assert!(shapes[0].start().y > shapes[0].end().y);
        assert!(shapes[2].start().y < shapes[2].end().y);
    }

    #[test]
    fn pop_up_value_stays_clear_of_its_chevrons() {
        let metrics = ControlMetrics::from_label_size(13.0_f32);
        let mut builder = LayoutBuilder::new(80.0, 30.0);
        builder.text(Button::new().pop_up("choice", "Opaque", "Opaque", &metrics));
        let result = LayoutEngine::new(Arc::new(|text, _| TextDimensions {
            width:       text.chars().count().to_f32() * 5.0,
            height:      13.0,
            line_height: 13.0,
        }))
        .compute(&builder.build(), 80.0, 30.0, 1.0);
        let button = result.commands.iter().find(|command| {
            matches!(
                command.kind,
                RenderCommandKind::Rectangle {
                    source: RectangleSource::Background,
                    ..
                }
            )
        });
        assert!(button.is_some());
        let Some(button) = button else { return };
        let values: Vec<_> = result
            .commands
            .iter()
            .filter(|command| matches!(command.kind, RenderCommandKind::Text { .. }))
            .collect();
        assert_eq!(values.len(), 1);
        assert!(
            values[0].bounds.x + values[0].bounds.width
                <= button.bounds.x + button.bounds.width - metrics.pop_up.chevron_left_inset + 1e-4
        );
    }
    #[test]
    fn pop_up_occlusion_keeps_the_standard_inset_before_the_chevron_stroke() {
        let metrics = ControlMetrics::from_label_size(13.0_f32);
        let mut builder = LayoutBuilder::new(120.0, 30.0);
        builder.text(Button::new().pop_up("choice", "Occlusion", "Occlusion", &metrics));
        let result = LayoutEngine::new(Arc::new(|text, _| TextDimensions {
            width:       text.chars().count().to_f32() * 5.0,
            height:      13.0,
            line_height: 13.0,
        }))
        .compute(&builder.build(), 120.0, 30.0, 1.0);
        let text = result.commands.iter().find(|command| {
            matches!(
                &command.kind, RenderCommandKind::Text { text, .. } if text == "Occlusion"
            )
        });
        assert!(text.is_some());
        let Some(text) = text else { return };
        let shapes = result
            .commands
            .iter()
            .find_map(|command| match &command.kind {
                RenderCommandKind::PanelShapes { shapes } => Some(shapes),
                _ => None,
            });
        assert!(shapes.is_some());
        let Some(shapes) = shapes else { return };
        let arrow_left = shapes
            .iter()
            .map(|shape| {
                shape
                    .width()
                    .mul_add(-0.5, shape.start().x.min(shape.end().x))
            })
            .fold(f32::INFINITY, f32::min);
        let gap = arrow_left - text.bounds.x - text.bounds.width;
        eprintln!(
            "Occlusion text-to-arrow gap={gap}, standard inset={}",
            metrics.button.side_padding
        );
        assert!(
            gap + 1e-4 >= metrics.button.side_padding,
            "text-to-arrow gap: {gap}"
        );
    }
}
