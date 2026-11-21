//! Shared action buttons for a confirmation dialog.

use std::sync::LazyLock;

use bevy::color::Mix;
use bevy::prelude::Color;
use bevy::prelude::Component;

use super::button;
use crate::AlignX;
use crate::AlignY;
use crate::Appearance;
use crate::Border;
use crate::Button;
use crate::ControlLength;
use crate::ControlMetrics;
use crate::ControlPalette;
use crate::CornerRadius;
use crate::DiegeticTextMeasurer;
use crate::Dimension;
use crate::El;
use crate::FontRegistry;
use crate::FontRegistryError;
use crate::LayoutBuilder;
use crate::Padding;
use crate::PanelElementId;
use crate::Pt;
use crate::Sizing;
use crate::Text;
use crate::TextInkMeasurement;
use crate::TextStyle;
use crate::Unit;

static TITLE_FONTS: LazyLock<Result<FontRegistry, FontRegistryError>> =
    LazyLock::new(FontRegistry::new);

const NEUTRAL_HOVER_HINT_LABEL_WEIGHT: f32 = 0.3;

fn title_cap_ink(size: Pt) -> TextInkMeasurement {
    let style = TextStyle::new(size).bold();
    TITLE_FONTS
        .as_ref()
        .ok()
        .and_then(|fonts| fonts.measure_ink("H", &style.as_measure()).ok())
        .unwrap_or(TextInkMeasurement::NoVisibleInk)
}

static TITLE_LEFT_BEARING_RATIO: LazyLock<f32> = LazyLock::new(|| match title_cap_ink(Pt(1.0)) {
    TextInkMeasurement::Visible(ink) => ink.left,
    TextInkMeasurement::NoVisibleInk => 0.0,
});

/// One labeled action and its ordinary button behavior.
#[derive(Clone, Debug)]
pub struct ConfirmChoice {
    id:     PanelElementId,
    label:  String,
    button: Button,
}

impl ConfirmChoice {
    /// Creates an action with an existing button declaration.
    #[must_use]
    pub fn new(id: impl Into<PanelElementId>, label: impl Into<String>, button: Button) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            button,
        }
    }
}

/// The tone and keyboard policy of the primary action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmTone {
    /// A normal operation, such as saving a tool.
    Normal,
    /// A pointer-only operation that removes user content. Cancel receives both keys.
    Destructive,
}

#[derive(Clone, Copy)]
enum ConfirmRole {
    Default,
    Cancel,
    SafeDefault,
    Destructive,
}

impl ConfirmRole {
    const fn hint(self) -> &'static str {
        match (self, cfg!(target_os = "macos")) {
            (Self::Default, true) => "↩",
            (Self::Default, false) => "Return",
            (Self::Cancel, true) => "⎋",
            (Self::Cancel, false) => "Esc",
            (Self::SafeDefault, true) => "↩ / ⎋",
            (Self::SafeDefault, false) => "Return / Esc",
            (Self::Destructive, _) => "",
        }
    }

    /// Returns the border width of this role's face, which the layout counts in its width.
    const fn border_width<L: ControlLength>(self, metrics: &ControlMetrics<L>) -> L {
        match self {
            Self::Default | Self::SafeDefault => metrics.focus_border_width,
            Self::Cancel | Self::Destructive => metrics.border_width,
        }
    }
}

/// A dialog's actions and keyboard destinations.
///
/// Attach this component to the panel that draws its action row.
#[derive(Component, Clone, Debug)]
pub struct ConfirmDialog {
    action: ConfirmChoice,
    cancel: ConfirmChoice,
    tone:   ConfirmTone,
}

impl ConfirmDialog {
    /// Creates the shared plate for a confirmation dialog.
    pub fn plate<L: ControlLength>(
        metrics: &ControlMetrics<L>,
        palette: &ControlPalette,
    ) -> El<crate::Column> {
        let mut inset: Dimension = metrics.spacing.confirm_outer_padding.into();
        let border: Dimension = metrics.border_width.into();
        inset.value -= border.value;
        // The border and the cap bearing count toward the visible margins.
        let mut left = inset;
        let bearing: Dimension = metrics.label_size.scaled(*TITLE_LEFT_BEARING_RATIO).into();
        left.value -= bearing.value;
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::new(left, inset, inset, inset))
            .gap(metrics.spacing.row_gap)
            .background(palette.field_fill)
            .border(Border::all(metrics.border_width, palette.field_border))
            .corner_radius(CornerRadius::all(metrics.button.corner_radius))
    }

    /// Draws a title with its cap line at the top of the title box.
    /// Pass the panel font unit for a label size with no explicit unit.
    pub fn title<L: ControlLength>(
        text: impl Into<String>,
        metrics: &ControlMetrics<L>,
        palette: &ControlPalette,
        font_unit: Unit,
    ) -> Text {
        let size: Dimension = metrics.label_size.into();
        let cap_top = match title_cap_ink(Pt(size.to_points(font_unit.to_points()))) {
            TextInkMeasurement::Visible(ink) => ink.top,
            TextInkMeasurement::NoVisibleInk => 0.0,
        };
        Text::new(text, TextStyle::new(size).bold().with_color(palette.knob))
            .trim_text_box(Pt(cap_top), Pt(0.0))
    }

    /// Requires a primary action and Cancel. A destructive action receives no key.
    #[must_use]
    pub const fn new(action: ConfirmChoice, cancel: ConfirmChoice, tone: ConfirmTone) -> Self {
        Self {
            action,
            cancel,
            tone,
        }
    }

    /// Returns the Return destination.
    #[must_use]
    pub const fn default_id(&self) -> &PanelElementId {
        match self.tone {
            ConfirmTone::Normal => &self.action.id,
            ConfirmTone::Destructive => &self.cancel.id,
        }
    }

    /// Returns the Esc destination.
    #[must_use]
    pub const fn cancel_id(&self) -> &PanelElementId { &self.cancel.id }

    const fn choices(&self) -> [(&ConfirmChoice, ConfirmRole); 2] {
        match self.tone {
            ConfirmTone::Normal => [
                (&self.cancel, ConfirmRole::Cancel),
                (&self.action, ConfirmRole::Default),
            ],
            ConfirmTone::Destructive => [
                (&self.action, ConfirmRole::Destructive),
                (&self.cancel, ConfirmRole::SafeDefault),
            ],
        }
    }

    /// Returns the natural width of the row that [`Self::append_actions`] draws.
    #[must_use]
    pub fn actions_width(
        &self,
        metrics: &ControlMetrics<f32>,
        measurer: &DiegeticTextMeasurer,
    ) -> f32 {
        let style = TextStyle::new(metrics.label_size).as_measure();
        let face_width = |choice: &ConfirmChoice, role: ConfirmRole| {
            let label = (measurer.measure_fn)(&choice.label, &style).width;
            let hint = if role.hint().is_empty() {
                0.0
            } else {
                metrics.button.hint_gap + (measurer.measure_fn)(role.hint(), &style).width
            };
            (metrics.button.side_padding + role.border_width(metrics)).mul_add(2.0, label + hint)
        };
        self.choices()
            .into_iter()
            .map(|(choice, role)| face_width(choice, role))
            .sum::<f32>()
            + metrics.row.gap
    }

    /// Draws the actions with the shared button face and control metrics.
    pub fn append_actions<L: ControlLength>(
        &self,
        builder: &mut LayoutBuilder,
        metrics: &ControlMetrics<L>,
        palette: &ControlPalette,
    ) {
        builder.with(
            El::row()
                .width(Sizing::GROW)
                .height(Sizing::fixed(metrics.button.height))
                .gap(metrics.row.gap)
                .alignment(AlignX::Right, AlignY::Center),
            |builder| {
                for (choice, role) in self.choices() {
                    append_choice(builder, choice, role, metrics, palette);
                }
            },
        );
    }
}

fn append_choice<L: ControlLength>(
    builder: &mut LayoutBuilder,
    choice: &ConfirmChoice,
    role: ConfirmRole,
    metrics: &ControlMetrics<L>,
    palette: &ControlPalette,
) {
    let face = choice
        .button
        .clone()
        .push_face(choice.id.clone(), metrics)
        .gap(metrics.button.hint_gap);
    let face = match role {
        ConfirmRole::Cancel => face,
        ConfirmRole::Destructive => {
            button::tinted_button_face(face, metrics, palette.warning_behind_text)
        },
        ConfirmRole::Default | ConfirmRole::SafeDefault => {
            button::default_button_face(face, metrics, palette.accent_behind_text, palette.knob)
        },
    };
    builder.with(face, |builder| {
        builder.text(Text::new(
            &choice.label,
            TextStyle::new(metrics.label_size).with_color(palette.knob),
        ));
        let hint = role.hint();
        let style = TextStyle::new(metrics.label_size);
        match role {
            ConfirmRole::Cancel => {
                let hover_ink = Color::LinearRgba(
                    palette
                        .confirm_hint
                        .to_linear()
                        .mix(&palette.knob.to_linear(), NEUTRAL_HOVER_HINT_LABEL_WEIGHT),
                );
                let appearance = builder
                    .child(El::new())
                    .hovered(Appearance::new().text_color(hover_ink))
                    .pressed(Appearance::new().text_color(palette.confirm_hint));
                builder.text(
                    Text::new(hint, style.with_color(palette.confirm_hint)).layout(appearance),
                );
            },
            ConfirmRole::Default | ConfirmRole::SafeDefault => {
                builder.text(Text::new(hint, style.with_color(palette.knob)));
            },
            ConfirmRole::Destructive => {},
        }
    });
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod tests;
