use bevy::prelude::Color;

use crate::Border;
use crate::Dimension;
use crate::Px;
use crate::ShadowCasting;
use crate::TextStyle;

const BORDER_WIDTH_RATIO: f32 = 3.0 / 44.0;
const FOCUS_BORDER_WIDTH_RATIO: f32 = 9.0 / 44.0;
const GROUP_INSET_RATIO: f32 = 9.0 / 13.0;
const ROW_HEIGHT_RATIO: f32 = 30.0 / 13.0;
const ROW_GAP_RATIO: f32 = 0.5;
const ROW_LABEL_WIDTH_RATIO: f32 = 82.0 / 13.0;
const BUTTON_HEIGHT_RATIO: f32 = 22.0 / 13.0;
const BUTTON_SIDE_PADDING_RATIO: f32 = 10.0 / 13.0;
const BUTTON_CORNER_RADIUS_RATIO: f32 = 11.0 / 26.0;
const POP_UP_CHEVRON_LEFT_INSET_RATIO: f32 = 16.0 / 13.0;
const POP_UP_CHEVRON_MIDDLE_INSET_RATIO: f32 = 1.0;
const POP_UP_CHEVRON_RIGHT_INSET_RATIO: f32 = 10.0 / 13.0;
const POP_UP_CHEVRON_STROKE_RATIO: f32 = 1.1 / 13.0;
const POP_UP_TEXT_TRAILING_PADDING_RATIO: f32 =
    POP_UP_CHEVRON_LEFT_INSET_RATIO + BUTTON_SIDE_PADDING_RATIO + 0.5 * POP_UP_CHEVRON_STROKE_RATIO;
const MENU_HIGHLIGHT_RADIUS_RATIO: f32 = 5.0 / 13.0;
const MENU_HIGHLIGHT_INSET_RATIO: f32 = 5.0 / 13.0;
const MENU_TEXT_INSET_RATIO: f32 = 10.0 / 13.0;
const MENU_CORNER_RADIUS_RATIO: f32 = MENU_HIGHLIGHT_RADIUS_RATIO + MENU_HIGHLIGHT_INSET_RATIO;
const SWITCH_TRACK_HEIGHT_RATIO: f32 = 15.0 / 13.0;
pub(super) const DISABLED_CAPSULE_FILL: Color = Color::srgb(0.65, 0.65, 0.66);
// The reference's on switch measures 88 x 40 pixels, with a 52 x 32 capsule
// and four pixels of clearance from the outer track edge on each resting side.
const SWITCH_TRACK_WIDTH_TO_HEIGHT: f32 = 88.0 / 40.0;
const SWITCH_KNOB_WIDTH_TO_TRACK_HEIGHT: f32 = 52.0 / 40.0;
const SWITCH_KNOB_HEIGHT_TO_TRACK_HEIGHT: f32 = 32.0 / 40.0;
const SWITCH_GAP_TO_TRACK_HEIGHT: f32 = 4.0 / 40.0;
const SWITCH_BORDER_WIDTH_RATIO: f32 = BORDER_WIDTH_RATIO * 0.5;
const SWITCH_TRACK_PADDING_RATIO: f32 =
    SWITCH_TRACK_HEIGHT_RATIO * SWITCH_GAP_TO_TRACK_HEIGHT - SWITCH_BORDER_WIDTH_RATIO;
const SLIDER_WIDTH_RATIO: f32 = 133.0 / 13.0;
// Pointer targets keep the original slider envelope independently of its paint.
const SLIDER_THUMB_HIT_HEIGHT_RATIO: f32 = 16.0 / 13.0;
const SLIDER_THUMB_HIT_WIDTH_RATIO: f32 = 20.0 / 13.0;
const SLIDER_TRACK_HEIGHT_RATIO: f32 = 6.0 / 13.0;
// Eight monospaced characters, side padding, field border and visual caret:
// ceil(8 * 13 * 0.6 + 13 + 2 * 13 * 3/44 + 13/16) = 79 points at a 13 pt label.
const SLIDER_VALUE_WIDTH_RATIO: f32 = 79.0 / 13.0;
const FIELD_WIDTH_RATIO: f32 = 165.0 / 13.0;
// Fit one label line and its border without filling compact row pitches.
const FIELD_HEIGHT_RATIO: f32 = 18.0 / 13.0;
const FIELD_SIDE_PADDING_RATIO: f32 = 0.5;
const FIELD_CORNER_RADIUS_RATIO: f32 = 11.0 / 26.0;

/// A control length in the panel's layout unit or an explicit layout unit.
pub trait ControlLength: Copy + Into<Dimension> {
    /// Scales a length without changing its unit.
    #[must_use]
    fn scaled(self, factor: f32) -> Self;
}

impl ControlLength for f32 {
    fn scaled(self, factor: f32) -> Self { self * factor }
}

impl ControlLength for Px {
    fn scaled(self, factor: f32) -> Self { Self(self.0 * factor) }
}

/// Control sizes whose ratios follow macOS System Settings, measured in points beside a 13 pt
/// label.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlMetrics<L: ControlLength> {
    /// Size of control labels.
    pub label_size:         L,
    /// Width of a control's regular border.
    pub border_width:       L,
    /// Width of a focused control's border.
    pub focus_border_width: L,
    /// Inset around a group of controls.
    pub group_inset:        L,
    /// Margins and gaps for panels and cards.
    pub spacing:            SpacingMetrics<L>,
    /// Sizes of control rows.
    pub row:                RowMetrics<L>,
    /// Sizes of buttons.
    pub button:             ButtonMetrics<L>,
    /// Sizes of pop-up chooser chevrons and their text clearance.
    pub pop_up:             PopUpMetrics<L>,
    /// Corners and row insets of an open menu.
    pub menu:               MenuMetrics<L>,
    /// Sizes of switches.
    pub switch:             SwitchMetrics<L>,
    /// Sizes of sliders.
    pub slider:             SliderMetrics<L>,
    /// Sizes of text fields.
    pub field:              FieldMetrics<L>,
}

impl<L: ControlLength> ControlMetrics<L> {
    /// Derives the complete control size set from a label size in the same unit.
    pub fn from_label_size(label_size: L) -> Self {
        let border_width = label_size.scaled(BORDER_WIDTH_RATIO);
        let switch_track_height = label_size.scaled(SWITCH_TRACK_HEIGHT_RATIO);
        let switch_track_padding = label_size.scaled(SWITCH_TRACK_PADDING_RATIO);
        let thumb = CapsuleThumb {
            width:         switch_track_height.scaled(SWITCH_KNOB_WIDTH_TO_TRACK_HEIGHT),
            height:        switch_track_height.scaled(SWITCH_KNOB_HEIGHT_TO_TRACK_HEIGHT),
            fill:          Color::srgb(0.95, 0.95, 0.96),
            disabled_fill: DISABLED_CAPSULE_FILL,
            border:        Border::default(),
            shadow:        ShadowCasting::On,
        };
        Self {
            label_size,
            border_width,
            focus_border_width: label_size.scaled(FOCUS_BORDER_WIDTH_RATIO),
            group_inset: label_size.scaled(GROUP_INSET_RATIO),
            spacing: SpacingMetrics {
                panel_padding: label_size.scaled(1.5),
                panel_bottom:  label_size.scaled(2.0),
                title_gap:     label_size,
                card_gap:      label_size,
                card_padding:  label_size,
                row_gap:       label_size.scaled(0.5),
            },
            row: RowMetrics {
                height:      label_size.scaled(ROW_HEIGHT_RATIO),
                gap:         label_size.scaled(ROW_GAP_RATIO),
                label_width: label_size.scaled(ROW_LABEL_WIDTH_RATIO),
            },
            button: ButtonMetrics {
                height:        label_size.scaled(BUTTON_HEIGHT_RATIO),
                side_padding:  label_size.scaled(BUTTON_SIDE_PADDING_RATIO),
                corner_radius: label_size.scaled(BUTTON_CORNER_RADIUS_RATIO),
            },
            pop_up: PopUpMetrics {
                chevron_left_inset:    label_size.scaled(POP_UP_CHEVRON_LEFT_INSET_RATIO),
                chevron_middle_inset:  label_size.scaled(POP_UP_CHEVRON_MIDDLE_INSET_RATIO),
                chevron_right_inset:   label_size.scaled(POP_UP_CHEVRON_RIGHT_INSET_RATIO),
                chevron_stroke:        label_size.scaled(POP_UP_CHEVRON_STROKE_RATIO),
                text_trailing_padding: label_size.scaled(POP_UP_TEXT_TRAILING_PADDING_RATIO),
            },
            menu: MenuMetrics {
                corner_radius:    label_size.scaled(MENU_CORNER_RADIUS_RATIO),
                highlight_radius: label_size.scaled(MENU_HIGHLIGHT_RADIUS_RATIO),
                highlight_inset:  label_size.scaled(MENU_HIGHLIGHT_INSET_RATIO),
                text_inset:       label_size.scaled(MENU_TEXT_INSET_RATIO),
            },
            switch: SwitchMetrics {
                track_width: switch_track_height.scaled(SWITCH_TRACK_WIDTH_TO_HEIGHT),
                track_height: switch_track_height,
                track_padding: switch_track_padding,
                border_width: label_size.scaled(SWITCH_BORDER_WIDTH_RATIO),
                thumb,
            },
            slider: SliderMetrics {
                width: label_size.scaled(SLIDER_WIDTH_RATIO),
                track_height: label_size.scaled(SLIDER_TRACK_HEIGHT_RATIO),
                thumb,
                thumb_hit_width: label_size.scaled(SLIDER_THUMB_HIT_WIDTH_RATIO),
                thumb_hit_height: label_size.scaled(SLIDER_THUMB_HIT_HEIGHT_RATIO),
                value_width: label_size.scaled(SLIDER_VALUE_WIDTH_RATIO),
            },
            field: FieldMetrics {
                width: label_size.scaled(FIELD_WIDTH_RATIO),
                height: label_size.scaled(FIELD_HEIGHT_RATIO),
                side_padding: label_size.scaled(FIELD_SIDE_PADDING_RATIO),
                border_width,
                corner_radius: label_size.scaled(FIELD_CORNER_RADIUS_RATIO),
            },
        }
    }
}

impl ControlMetrics<f32> {
    /// Fits shared text, fields and clearances within a compact paired-control width.
    /// Both metric sets must use the same layout unit.
    #[must_use]
    pub fn with_shared_text_metrics(mut self, words: &Self) -> Self {
        // Each track reserves its value gap and one gap beside the pair's label.
        self.slider.width = (self.row.gap - words.row.gap)
            .mul_add(2.0, self.slider.width)
            .max(self.slider.thumb_hit_width);
        self.label_size = words.label_size;
        self.spacing = words.spacing;
        self.group_inset = words.group_inset;
        self.button.side_padding = words.button.side_padding;
        self.row.height = words.row.height;
        self.row.gap = words.row.gap;
        self.button.height = words.button.height;
        self.field = words.field;
        self.slider.value_width = words.slider.value_width;
        self.pop_up.text_trailing_padding = self.pop_up.chevron_stroke.mul_add(
            0.5,
            self.pop_up.chevron_left_inset + words.button.side_padding,
        );
        self
    }

    /// Fits a control row to its painted field, thumb and switch envelope.
    #[must_use]
    pub const fn with_painted_row_height(mut self) -> Self {
        self.row.height = self
            .field
            .height
            .max(self.slider.thumb.height)
            .max(self.switch.track_height);
        self
    }

    /// One label-sized line for a field's text and caret, without paragraph leading.
    #[must_use]
    pub fn field_text_style(&self) -> TextStyle {
        TextStyle::new(self.label_size).with_line_height(self.label_size)
    }
}

/// Spacing for a panel and its nested cards, in the label's layout unit.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacingMetrics<L: ControlLength> {
    /// Inset from the panel's left, right, and top edges.
    pub panel_padding: L,
    /// Inset from the panel's bottom edge.
    pub panel_bottom:  L,
    /// Gap between a title and its first card.
    pub title_gap:     L,
    /// Gap between cards.
    pub card_gap:      L,
    /// Inset on every side of a card.
    pub card_padding:  L,
    /// Gap between related rows within a card.
    pub row_gap:       L,
}

impl<L: ControlLength> SpacingMetrics<L> {
    /// One step on the panel spacing scale: a quarter of the label size.
    #[must_use]
    pub fn step(&self) -> L { self.row_gap.scaled(0.5) }
}

/// Dimensions of one labeled control row.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowMetrics<L: ControlLength> {
    /// Height of a control row.
    pub height:      L,
    /// Gap between adjacent controls or rows.
    pub gap:         L,
    /// Width reserved for a control's label.
    pub label_width: L,
}

/// Dimensions of a button.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonMetrics<L: ControlLength> {
    /// Height of a button.
    pub height:        L,
    /// Padding on each side of a button's label.
    pub side_padding:  L,
    /// Radius of a button's corners.
    pub corner_radius: L,
}

impl<L: ControlLength> ButtonMetrics<L> {
    /// Shared em size of inline mode icons beside a value field.
    pub const fn mode_icon_size(&self) -> L { self.height }

    /// Drawn width of the bundled Plus and Equal mode glyphs.
    pub fn mode_icon_ink_width(&self) -> L { self.mode_icon_size().scaled(0.669) }
}

/// Trailing marks and text clearance of a pop-up chooser.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopUpMetrics<L: ControlLength> {
    /// Distance from the right edge to the left chevron endpoint.
    pub chevron_left_inset:    L,
    /// Distance from the right edge to the chevron tip.
    pub chevron_middle_inset:  L,
    /// Distance from the right edge to the right chevron endpoint.
    pub chevron_right_inset:   L,
    /// Width of each chevron stroke.
    pub chevron_stroke:        L,
    /// Padding that keeps text out of the chevron area.
    pub text_trailing_padding: L,
}

/// Corners and row insets of an open menu, as macOS draws a menu's highlighted row: a rounded
/// band across the menu, inset from its sides, with the row's text inset inside the band.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuMetrics<L: ControlLength> {
    /// Radius of the menu's corners, concentric with the highlight band's corners.
    pub corner_radius:    L,
    /// Radius of the highlighted row's band.
    pub highlight_radius: L,
    /// Gap between each side of the menu and the highlighted row's band.
    pub highlight_inset:  L,
    /// Gap between each side of a row's band and the row's text.
    pub text_inset:       L,
}

/// Dimensions of a fully rounded switch track and its capsule knob.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwitchMetrics<L: ControlLength> {
    /// Width of the switch track.
    pub track_width:   L,
    /// Height of the switch track.
    pub track_height:  L,
    /// Clear inset between the track's inner border edge and the knob.
    pub track_padding: L,
    /// Width of the track ring, half the standard control border.
    pub border_width:  L,
    /// Capsule shared with the slider's painted thumb.
    pub thumb:         CapsuleThumb<L>,
}

impl<L: ControlLength> SwitchMetrics<L> {
    /// Radius of the switch track's rounded ends.
    pub fn track_radius(&self) -> L { self.track_height.scaled(0.5) }
}

/// Shared paint and proportions of a switch knob and slider thumb.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapsuleThumb<L: ControlLength> {
    /// Width of the capsule.
    pub width:         L,
    /// Height of the capsule.
    pub height:        L,
    /// Rest, hover and pressed fill, matching the switch knob.
    pub fill:          Color,
    /// Fill while the enclosing control is disabled.
    pub disabled_fill: Color,
    /// Capsule outline; the reference switch knob has no border.
    pub border:        Border,
    /// Whether the capsule casts a shadow.
    pub shadow:        ShadowCasting,
}

impl<L: ControlLength> CapsuleThumb<L> {
    /// Radius of the capsule's semicircular ends.
    pub fn corner_radius(&self) -> L { self.height.scaled(0.5) }
}

/// Dimensions of a slider and its thumb.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderMetrics<L: ControlLength> {
    /// Minimum track length; fixed-length layouts draw at this length.
    pub width:            L,
    /// Height of the slider track.
    pub track_height:     L,
    /// Capsule shared with the switch knob.
    pub thumb:            CapsuleThumb<L>,
    /// Width of the invisible thumb target and travel envelope.
    pub thumb_hit_width:  L,
    /// Height of the invisible thumb target and slider press area.
    pub thumb_hit_height: L,
    /// Width reserved for the slider value label.
    pub value_width:      L,
}

impl<L: ControlLength> SliderMetrics<L> {
    /// Scale of the thumb shadow drawn behind a reachable slider.
    pub const THUMB_SHADOW_SCALE: f32 = 1.2;

    /// Height of the reach capsule behind the track, inside the thumb's height.
    pub fn reach_band_height(&self) -> L { self.thumb.height.scaled(0.75) }

    /// Monospaced characters that fit the value field with its padding and border.
    pub const VALUE_CHARACTER_CAPACITY: usize = 8;

    /// Radius of the slider track's rounded ends.
    pub fn track_radius(&self) -> L { self.track_height.scaled(0.5) }
}

/// Dimensions of a text field.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldMetrics<L: ControlLength> {
    /// Width of the text field.
    pub width:         L,
    /// Height of the text field.
    pub height:        L,
    /// Space between text and the field's side border.
    pub side_padding:  L,
    /// Width of the field's regular outline.
    pub border_width:  L,
    /// Radius of each field corner.
    pub corner_radius: L,
}

impl<L: ControlLength> FieldMetrics<L> {
    /// Vertical inset that leaves a label-sized caret inside the shared field frame.
    #[must_use]
    pub fn vertical_padding(&self) -> L { self.border_width.scaled(0.5) }
}

#[cfg(test)]
mod tests {
    use super::ControlMetrics;
    use crate::Px;

    #[test]
    fn menu_highlight_matches_macos_and_nests_inside_the_menu_corners() {
        let menu = ControlMetrics::from_label_size(Px(13.0)).menu;

        assert!((menu.highlight_radius.0 - 5.0).abs() < 1e-4);
        assert!((menu.highlight_inset.0 - 5.0).abs() < 1e-4);
        assert!((menu.text_inset.0 - 10.0).abs() < 1e-4);
        assert!(
            (menu.corner_radius.0 - (menu.highlight_radius.0 + menu.highlight_inset.0)).abs()
                < 1e-4
        );
    }
}
