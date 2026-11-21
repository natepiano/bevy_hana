use super::Button;
use super::ButtonPaintBounds;
use super::face;
use crate::AlignX;
use crate::AlignY;
use crate::ControlLength;
use crate::ControlMetrics;
use crate::El;
use crate::Padding;
use crate::PanelElementId;
use crate::Sizing;
use crate::Text;
use crate::TextStyle;
use crate::WidgetElement;

impl Button {
    /// Draws a labeled push button with the shared control sizes and appearances.
    pub fn push<L: ControlLength>(
        self,
        id: impl Into<PanelElementId>,
        label: impl Into<String>,
        metrics: &ControlMetrics<L>,
    ) -> Text<WidgetElement<Self>> {
        Text::new(
            label,
            TextStyle::new(metrics.label_size).with_color(face::TEXT),
        )
        .layout(self.push_face(id, metrics))
        .map_layout(|face| face.alignment(AlignX::Center, AlignY::Center))
    }

    pub(in crate::widgets::controls) fn push_face<L: ControlLength>(
        mut self,
        id: impl Into<PanelElementId>,
        metrics: &ControlMetrics<L>,
    ) -> El<crate::Row, WidgetElement<Self>> {
        self.paint_bounds = ButtonPaintBounds::InsideLayout;
        face::button_face(
            El::new()
                .width(Sizing::FIT)
                .height(Sizing::fixed(metrics.button.height))
                .padding(Padding::xy(
                    metrics.button.side_padding,
                    metrics.border_width,
                ))
                .align_y(AlignY::Center)
                .widget(id, self),
            metrics,
        )
    }
}
