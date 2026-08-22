//! Reusable connector-line geometry for `fairy_dust` panels.

use bevy::prelude::Color;
use hana_diegetic::CalloutCap;
use hana_diegetic::PanelCoord;
use hana_diegetic::PanelLine;
use hana_diegetic::PanelPoint;
use hana_diegetic::Px;

use crate::constants::CENTER_FRACTION;
use crate::constants::SPACER_EDGE_OFFSET;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ConnectorColors {
    pub(crate) active: Color,
    pub(crate) idle:   Color,
}

impl ConnectorColors {
    const fn for_active(self, active: bool) -> Color {
        if active { self.active } else { self.idle }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SpacerLayout {
    pub(crate) word_count:        usize,
    pub(crate) label_line_height: Px,
    pub(crate) row_gap:           Px,
    pub(crate) line_width:        Px,
    pub(crate) cap_size:          Px,
    pub(crate) level_epsilon:     Px,
    pub(crate) trunk_end_gap:     Px,
    pub(crate) colors:            ConnectorColors,
}

#[must_use]
pub(crate) fn feeder_line(start_gap: Px, line_width: Px, color: Color) -> PanelLine {
    base_line(
        PanelCoord::start(start_gap),
        PanelCoord::percent(CENTER_FRACTION),
        PanelCoord::end(SPACER_EDGE_OFFSET),
        PanelCoord::percent(CENTER_FRACTION),
        line_width,
        color,
    )
}

#[must_use]
pub(crate) fn spacer_lines(
    layout: SpacerLayout,
    word_active: &[bool],
    group_active: bool,
) -> Vec<PanelLine> {
    assert_eq!(
        word_active.len(),
        layout.word_count,
        "connector activity must match word count",
    );
    if layout.word_count == 0 {
        return Vec::new();
    }

    let group_height = group_height(layout);
    let intent_center = group_height * CENTER_FRACTION;
    let mut word_center = layout.label_line_height.0 * CENTER_FRACTION;
    let mut lines = Vec::with_capacity(layout.word_count + 1);

    for &is_active in word_active {
        let word_y = PanelCoord::percent(word_center / group_height);
        if (word_center - intent_center).abs() >= layout.level_epsilon.0 {
            lines.push(spacer_line(
                PanelCoord::start(SPACER_EDGE_OFFSET),
                word_y,
                PanelCoord::start(SPACER_EDGE_OFFSET),
                PanelCoord::percent(CENTER_FRACTION),
                layout,
                layout.colors.for_active(is_active),
            ));
        }
        word_center += layout.label_line_height.0 + layout.row_gap.0;
    }

    lines.push(
        spacer_line(
            PanelCoord::start(SPACER_EDGE_OFFSET),
            PanelCoord::percent(CENTER_FRACTION),
            PanelCoord::end(layout.trunk_end_gap),
            PanelCoord::percent(CENTER_FRACTION),
            layout,
            layout.colors.for_active(group_active),
        )
        .end_cap(CalloutCap::arrow().solid()),
    );
    lines
}

fn group_height(layout: SpacerLayout) -> f32 {
    let mut height = 0.0;
    for index in 0..layout.word_count {
        if index > 0 {
            height += layout.row_gap.0;
        }
        height += layout.label_line_height.0;
    }
    height
}

fn spacer_line(
    start_x: PanelCoord,
    start_y: PanelCoord,
    end_x: PanelCoord,
    end_y: PanelCoord,
    layout: SpacerLayout,
    color: Color,
) -> PanelLine {
    base_line(start_x, start_y, end_x, end_y, layout.line_width, color).cap_size(layout.cap_size)
}

fn base_line(
    start_x: PanelCoord,
    start_y: PanelCoord,
    end_x: PanelCoord,
    end_y: PanelCoord,
    line_width: Px,
    color: Color,
) -> PanelLine {
    PanelLine::new(
        PanelPoint::new(start_x, start_y),
        PanelPoint::new(end_x, end_y),
    )
    .width(line_width)
    .color(color)
}
