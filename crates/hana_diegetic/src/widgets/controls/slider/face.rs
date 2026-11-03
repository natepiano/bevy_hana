//! Shared System Settings face for an authored slider.

use bevy::color::Alpha;
use bevy::color::Color;

use super::Slider;
use crate::AlignX;
use crate::AlignY;
use crate::ControlLength;
use crate::ControlMetrics;
use crate::ControlPalette;
use crate::CornerRadius;
use crate::El;
use crate::LayoutBuilder;
use crate::PanelElementId;
use crate::Sizing;
use crate::SliderMetrics;
use crate::WidgetBuilder;
use crate::WidgetInteractivity;
use crate::widgets::VisualSlotId;

/// Whether a slider declares a reachable live-signal band.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SliderReachableBand {
    /// The slider has no reachable signal band.
    Hidden,
    /// Declare the reachable band; live state replaces the authored fill.
    Drawn,
}

impl LayoutBuilder {
    /// Adds a System Settings slider face with an optional reachable signal band.
    pub fn slider<L: ControlLength>(
        &mut self,
        id: impl Into<PanelElementId>,
        slider: Slider,
        metrics: &ControlMetrics<L>,
        band: SliderReachableBand,
    ) {
        let size = metrics.slider;
        let shadow_scale = crate::SliderMetrics::<L>::THUMB_SHADOW_SCALE;
        let palette = ControlPalette::system_dark();
        self.with(
            El::overlay()
                .width(Sizing::fixed(size.width))
                .height(Sizing::fixed(size.thumb_hit_height))
                .alignment(AlignX::Center, AlignY::Center)
                .widget(id, slider),
            |builder| {
                // The taller capsule stays behind the whole track; live fill
                // and the knob draw over both signal surfaces.
                if band == SliderReachableBand::Drawn {
                    builder.with(
                        El::new()
                            .width(Sizing::fixed(size.width))
                            .height(Sizing::fixed(size.reach_band_height()))
                            .corner_radius(CornerRadius::all(size.reach_band_height().scaled(0.5)))
                            .background(palette.slider_reachable_band)
                            .widget_interactivity(WidgetInteractivity::Disabled)
                            .visual_slot(VisualSlotId::SLIDER_BAND),
                        |_| {},
                    );
                }
                builder.with(
                    El::new()
                        .width(Sizing::fixed(size.width))
                        .height(Sizing::fixed(size.track_height))
                        .corner_radius(CornerRadius::all(size.track_radius()))
                        .background(palette.track)
                        .widget_interactivity(WidgetInteractivity::Disabled),
                    |_| {},
                );
                if band == SliderReachableBand::Drawn {
                    builder.with(
                        El::new()
                            .width(Sizing::fixed(size.thumb.width.scaled(shadow_scale)))
                            .height(Sizing::fixed(size.thumb.height.scaled(shadow_scale)))
                            .corner_radius(CornerRadius::all(
                                size.thumb.corner_radius().scaled(shadow_scale),
                            ))
                            .background(Color::BLACK.with_alpha(0.35))
                            .widget_interactivity(WidgetInteractivity::Disabled)
                            .visual_slot(VisualSlotId::SLIDER_KNOB_SHADOW),
                        |_| {},
                    );
                }
                builder.with(
                    El::new()
                        .width(Sizing::fixed(size.width))
                        .height(Sizing::fixed(size.track_height))
                        .corner_radius(CornerRadius::all(size.track_radius()))
                        .background(palette.accent)
                        .widget_interactivity(WidgetInteractivity::Disabled)
                        .slider_fill(),
                    |_| {},
                );
                builder.slider_thumb(&size);
            },
        );
    }
}

impl WidgetBuilder<'_, Slider> {
    /// Adds the shared capsule and its independent pointer/travel envelope.
    pub fn slider_thumb<L: ControlLength>(&mut self, metrics: &SliderMetrics<L>) {
        self.with(
            El::new()
                .width(Sizing::fixed(metrics.thumb_hit_width))
                .height(Sizing::fixed(metrics.thumb_hit_height))
                .widget_interactivity(WidgetInteractivity::Disabled)
                .visual_slot(VisualSlotId::SLIDER_THUMB_HIT),
            |_| {},
        );
        self.with(metrics.thumb.element(self).slider_thumb(), |_| {});
    }
}
