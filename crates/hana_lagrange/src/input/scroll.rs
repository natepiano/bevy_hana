//! One shared classification of Bevy's accumulated scroll input for camera adapters.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::input::mouse::MouseScrollUnit;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;

/// Scroll input accumulated for this frame, classified once by Bevy's scroll unit.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum FrameScrollInput {
    /// No scroll input was available.
    #[default]
    None,
    /// Pixel-unit scrolling, normally emitted by a trackpad or precision scrolling device.
    Smooth(Vec2),
    /// Line-unit scrolling, normally emitted by a mouse wheel.
    Wheel(Vec2),
}

impl FrameScrollInput {
    /// Returns the smooth-scroll delta, or zero for line scroll and an idle frame.
    pub(crate) const fn smooth_delta(self) -> Vec2 {
        match self {
            Self::Smooth(delta) => delta,
            Self::None | Self::Wheel(_) => Vec2::ZERO,
        }
    }

    /// Converts back to Bevy's carrier for the orbit adapter's existing dispatch path.
    pub(crate) const fn accumulated(self) -> AccumulatedMouseScroll {
        match self {
            Self::None => AccumulatedMouseScroll {
                unit:  MouseScrollUnit::Line,
                delta: Vec2::ZERO,
            },
            Self::Smooth(delta) => AccumulatedMouseScroll {
                unit: MouseScrollUnit::Pixel,
                delta,
            },
            Self::Wheel(delta) => AccumulatedMouseScroll {
                unit: MouseScrollUnit::Line,
                delta,
            },
        }
    }
}

/// Captures Bevy's scroll accumulator once for every camera adapter.
pub(super) fn stage_frame_scroll_input(
    scroll: Option<Res<AccumulatedMouseScroll>>,
    mut frame_scroll: ResMut<FrameScrollInput>,
) {
    *frame_scroll = scroll
        .as_deref()
        .map_or(FrameScrollInput::None, |scroll| match scroll.unit {
            MouseScrollUnit::Pixel => FrameScrollInput::Smooth(scroll.delta),
            MouseScrollUnit::Line => FrameScrollInput::Wheel(scroll.delta),
        });
}
