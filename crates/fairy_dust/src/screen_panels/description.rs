//! Static side-panel that explains what an example demonstrates.

use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::error;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::FontRegistry;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;

use super::constants::BODY_COLOR;
use super::constants::DESCRIPTION_CHILD_GAP;
use super::constants::DESCRIPTION_WIDTH;
use super::screen_panel_frame;
use crate::constants::LABEL_SIZE;
use crate::constants::TITLE_COLOR;
use crate::constants::TITLE_SIZE;

/// A static side panel that explains what an example demonstrates.
#[derive(Clone, Debug, PartialEq)]
pub struct DescriptionPanel {
    anchor:           Anchor,
    title:            String,
    lines:            Vec<String>,
    background_color: Option<Color>,
    width:            Sizing,
    body_size:        f32,
}

impl DescriptionPanel {
    /// Creates a description panel with a title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            anchor:           Anchor::BottomLeft,
            title:            title.into(),
            lines:            Vec::new(),
            background_color: None,
            width:            Sizing::fixed(DESCRIPTION_WIDTH),
            body_size:        LABEL_SIZE.0,
        }
    }

    /// Sets the panel screen anchor.
    #[must_use]
    pub const fn with_anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Overrides the inner background color (including alpha) for this
    /// description panel. Defaults to the crate's `INNER_BACKGROUND` constant.
    #[must_use]
    pub const fn with_background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    /// Uses content-fit width instead of the default fixed description width.
    #[must_use]
    pub const fn with_fit_width(mut self) -> Self {
        self.width = Sizing::FIT;
        self
    }

    /// Overrides the body text size while preserving the shared panel styling.
    #[must_use]
    pub const fn with_body_size(mut self, size: f32) -> Self {
        self.body_size = size;
        self
    }

    /// Adds one display line to the panel.
    #[must_use]
    pub fn line(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    /// Adds multiple display lines to the panel.
    #[must_use]
    pub fn lines(mut self, lines: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.lines.extend(lines.into_iter().map(Into::into));
        self
    }
}

#[derive(Component)]
struct DescriptionPanelMarker;

pub(super) fn spawn_description_panel(
    commands: &mut Commands,
    panel: &DescriptionPanel,
    fonts: &FontRegistry,
    materials: &mut Assets<StandardMaterial>,
) {
    let unlit = super::screen_panel_material_handle(materials);
    let built = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(panel.anchor)
        .material(unlit.clone())
        .text_material(unlit)
        .layout(|builder| build_description_layout(builder, panel, fonts))
        .build();

    match built {
        Ok(built) => {
            commands.spawn((DescriptionPanelMarker, built, Transform::default()));
        },
        Err(error) => {
            error!("fairy_dust: failed to build description panel: {error}");
        },
    }
}

fn build_description_layout(
    builder: &mut LayoutBuilder,
    panel: &DescriptionPanel,
    fonts: &FontRegistry,
) {
    let title =
        TextStyle::new(super::integral_advance_size(fonts, TITLE_SIZE)).with_color(TITLE_COLOR);
    let body = TextStyle::new(super::integral_advance_size(fonts, Pt(panel.body_size)))
        .with_color(BODY_COLOR);

    let background = panel
        .background_color
        .unwrap_or_else(super::default_inner_background);
    screen_panel_frame(builder, panel.width, Sizing::FIT, background, |builder| {
        builder.with(
            El::column().width(Sizing::GROW).gap(DESCRIPTION_CHILD_GAP),
            |builder| {
                builder.text((&panel.title, title));
                for line in &panel.lines {
                    builder.text((line, body.clone()));
                }
            },
        );
    });
}
