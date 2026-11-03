//! Static side-panel that explains what an example demonstrates.

use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::Res;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::prelude::warn;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::FontRegistry;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;

use super::PanelTextEdge;
use super::constants::BODY_COLOR;
use super::constants::DESCRIPTION_WIDTH;
use super::screen_panel_frame;
use super::separation::EdgePanelPrecedence;
use super::title_bar::TitleBarOverflow;
use crate::constants;
use crate::constants::LABEL_SIZE;
use crate::constants::TITLE_COLOR;
use crate::constants::TITLE_SIZE;

/// A static side panel that explains what an example demonstrates.
#[derive(Clone, Debug, PartialEq, Component)]
pub struct DescriptionPanel {
    anchor:           Anchor,
    title:            String,
    lines:            Vec<String>,
    overflow_lines:   Vec<(String, String)>,
    background_color: DescriptionFill,
    width:            Sizing,
    body_size:        f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DescriptionFill {
    Shared,
    Custom(Color),
}

impl DescriptionPanel {
    /// Creates a description panel with a title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            anchor:           Anchor::BottomLeft,
            title:            title.into(),
            lines:            Vec::new(),
            overflow_lines:   Vec::new(),
            background_color: DescriptionFill::Shared,
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

    /// Overrides the inner background color for this description panel.
    /// The fill stays opaque over the scene.
    #[must_use]
    pub const fn with_background_color(mut self, color: Color) -> Self {
        self.background_color = DescriptionFill::Custom(color);
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

    /// Adds a line that appears when the named control moves out of the title bar.
    #[must_use]
    pub fn line_when_title_control_hidden(
        mut self,
        control: impl Into<String>,
        line: impl Into<String>,
    ) -> Self {
        self.overflow_lines.push((control.into(), line.into()));
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
pub(super) struct DescriptionPanelMarker;

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
            commands.spawn((
                DescriptionPanelMarker,
                panel.clone(),
                EdgePanelPrecedence::Description,
                built,
                Transform::default(),
            ));
        },
        Err(error) => {
            error!("fairy_dust: failed to build description panel: {error}");
        },
    }
}

pub(super) fn refresh_description_for_title_bar_overflow(
    mut commands: Commands,
    panels: Query<(Entity, &DescriptionPanel), With<DescriptionPanelMarker>>,
    bars: Query<Ref<TitleBarOverflow>>,
    fonts: Res<FontRegistry>,
) {
    let Ok(overflow) = bars.single() else {
        return;
    };
    if !overflow.is_changed() {
        return;
    }
    for (entity, panel) in &panels {
        let mut builder =
            LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
        build_description_layout_with_overflow(&mut builder, panel, &fonts, &overflow);
        if let Err(error) = commands.set_tree(entity, builder.build()) {
            warn!("failed to replace description panel {entity:?} tree: {error}");
        }
    }
}

pub(crate) fn build_description_layout(
    builder: &mut LayoutBuilder,
    panel: &DescriptionPanel,
    fonts: &FontRegistry,
) {
    build_description_layout_with_overflow(builder, panel, fonts, &TitleBarOverflow::default());
}

fn build_description_layout_with_overflow(
    builder: &mut LayoutBuilder,
    panel: &DescriptionPanel,
    fonts: &FontRegistry,
    overflow: &TitleBarOverflow,
) {
    let lines: Vec<&str> = panel
        .lines
        .iter()
        .map(String::as_str)
        .chain(
            panel
                .overflow_lines
                .iter()
                .filter_map(|(control, line)| overflow.contains(control).then_some(line.as_str())),
        )
        .collect();
    let title_size = super::integral_advance_size(fonts, TITLE_SIZE);
    let body_size = super::integral_advance_size(fonts, Pt(panel.body_size));
    let title = TextStyle::new(title_size).with_color(TITLE_COLOR);
    let body = TextStyle::new(body_size).with_color(BODY_COLOR);

    let background = match panel.background_color {
        DescriptionFill::Shared => super::default_inner_background(),
        DescriptionFill::Custom(color) => color,
    };
    screen_panel_frame(builder, panel.width, Sizing::FIT, background, |builder| {
        builder.with(
            El::column()
                .width(Sizing::GROW)
                .gap(constants::screen_spacing().title_gap),
            |builder| {
                builder.text(super::screen_panel_ink_text(
                    &panel.title,
                    title,
                    fonts,
                    title_size,
                    if lines.is_empty() {
                        PanelTextEdge::Both
                    } else {
                        PanelTextEdge::First
                    },
                ));
                if !lines.is_empty() {
                    builder.with(
                        El::column()
                            .width(Sizing::GROW)
                            .gap(constants::screen_spacing().row_gap),
                        |builder| {
                            for (index, line) in lines.iter().enumerate() {
                                let last = index + 1 == lines.len();
                                builder.with(
                                    El::new()
                                        .width(Sizing::GROW)
                                        .height(Sizing::fit_min(constants::SCREEN_PANEL_ROW_HEIGHT))
                                        .align_y(if last {
                                            AlignY::Bottom
                                        } else {
                                            AlignY::Center
                                        }),
                                    |builder| {
                                        if last {
                                            builder.text(super::screen_panel_ink_text(
                                                *line,
                                                body.clone(),
                                                fonts,
                                                body_size,
                                                PanelTextEdge::Last,
                                            ));
                                        } else {
                                            builder.text(Text::new(*line, body.clone()));
                                        }
                                    },
                                );
                            }
                        },
                    );
                }
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use bevy::prelude::MinimalPlugins;
    use hana_diegetic::ComputedDiegeticPanel;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::FontId;
    use hana_diegetic::HeadlessDiegeticUiPlugin;
    use hana_diegetic::LayoutBuilder;
    use hana_diegetic::TextInkMeasurement;
    use hana_diegetic::Unit;

    use super::*;

    #[test]
    fn help_lists_only_controls_dropped_from_the_title_bar() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let panel = DescriptionPanel::new("Keys")
            .line("Tab: next")
            .line_when_title_control_hidden("H Home", "H Home")
            .line_when_title_control_hidden("D Enable/disable", "D Enable/disable");
        let render = |dropped: &[&str]| {
            let mut builder =
                LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
            build_description_layout_with_overflow(
                &mut builder,
                &panel,
                &fonts,
                &TitleBarOverflow {
                    dropped: dropped.iter().map(|line| (*line).to_owned()).collect(),
                },
            );
            format!("{:?}", builder.build())
        };
        let wide = render(&[]);
        assert!(!wide.contains("H Home"));
        assert!(!wide.contains("D Enable/disable"));
        let narrow = render(&["H Home", "D Enable/disable"]);
        assert!(narrow.contains("H Home"));
        assert!(narrow.contains("D Enable/disable"));
        Ok(())
    }

    #[test]
    fn help_title_gap_exceeds_its_shared_row_gap() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let description = DescriptionPanel::new("Keys").line("First").line("Second");
        let mut builder =
            LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
        build_description_layout(&mut builder, &description, &fonts);
        let tree = builder.build();
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(tree.clone())
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("help panel did not solve")?;
        let find = |label: &str| {
            solved
                .commands
                .iter()
                .find(|command| {
                    format!("{:?}", command.kind).starts_with(&format!("Text {{ text: {label:?},"))
                })
                .map(|command| command.element_idx)
        };
        let title = solved.computed[find("Keys").ok_or("title missing")?].bounds;
        let row = |label: &str| -> Result<_, String> {
            let text = find(label).ok_or_else(|| format!("{label} missing"))?;
            let cell = (0..solved.computed.len())
                .find(|&index| tree.children_of(index).contains(&text))
                .ok_or_else(|| format!("{label} cell missing"))?;
            Ok(solved.computed[cell].bounds)
        };
        let first = row("First")?;
        let second = row("Second")?;
        let point_per_pixel = Unit::Pixels.to_points();
        let spacing = crate::constants::screen_spacing();
        let title_gap = first.y - title.y - title.height;
        let row_gap = second.y - first.y - first.height;
        assert!(
            spacing
                .title_gap
                .0
                .mul_add(-point_per_pixel, title_gap)
                .abs()
                < 0.1,
            "title gap {title_gap}"
        );
        assert!(
            spacing.row_gap.0.mul_add(-point_per_pixel, row_gap).abs() < 0.1,
            "row gap {row_gap}"
        );
        assert!(title_gap > row_gap);
        Ok(())
    }

    #[test]
    fn wrapped_description_lines_expand_their_rows_and_panel() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let description = DescriptionPanel::new("What to compare")
            .line("First description line has enough words to wrap across several lines in the panel")
            .line("Second description line also wraps so its final words must stay above the panel edge")
            .line("Short first")
            .line("Short second");
        let mut builder =
            LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
        build_description_layout(&mut builder, &description, &fonts);
        let tree = builder.build();
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(tree.clone())
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("description panel did not solve")?;
        let frame = *tree.children_of(0).first().ok_or("frame missing")?;
        let inner = *tree
            .children_of(frame)
            .first()
            .ok_or("inner frame missing")?;
        let content = *tree.children_of(inner).first().ok_or("content missing")?;
        let rows = *tree
            .children_of(content)
            .get(1)
            .ok_or("description rows missing")?;
        let cells = tree.children_of(rows);
        let [first_cell, second_cell, short_first_cell, short_second_cell] = cells else {
            return Err(format!(
                "expected four description rows, found {}",
                cells.len()
            ));
        };
        let first_text = *tree
            .children_of(*first_cell)
            .first()
            .ok_or("first text missing")?;
        let second_text = *tree
            .children_of(*second_cell)
            .first()
            .ok_or("second text missing")?;
        let first = solved.computed[*first_cell].bounds;
        let second = solved.computed[*second_cell].bounds;
        let short_first = solved.computed[*short_first_cell].bounds;
        let short_second = solved.computed[*short_second_cell].bounds;
        let minimum = constants::SCREEN_PANEL_ROW_HEIGHT.0 * Unit::Pixels.to_points();
        assert!(first.height > minimum * 1.5, "first row: {first:?}");
        assert!(second.height > minimum * 1.5, "second row: {second:?}");
        assert!(
            second.y >= first.y + first.height,
            "rows overlap: {first:?}, {second:?}"
        );
        let row_gap = constants::screen_spacing().row_gap.0 * Unit::Pixels.to_points();
        assert!(
            (short_first.height - minimum).abs() < 0.1,
            "one-line row: {short_first:?}"
        );
        assert!(
            (short_second.height - minimum).abs() < 0.1,
            "one-line row: {short_second:?}"
        );
        assert!(
            (short_second.y - short_first.y - minimum - row_gap).abs() < 0.1,
            "one-line row pitch changed: {short_first:?}, {short_second:?}"
        );
        for text in [first_text, second_text] {
            let cell = if text == first_text { first } else { second };
            for command in solved
                .commands
                .iter()
                .filter(|command| command.element_idx == text)
            {
                assert!(
                    command.bounds.y + command.bounds.height <= cell.y + cell.height + 0.1,
                    "wrapped ink escapes row: {:?} in {cell:?}",
                    command.bounds
                );
            }
        }
        let panel = solved.computed[0].bounds;
        assert!(
            short_second.y + short_second.height <= panel.y + panel.height,
            "last row escapes panel: {short_second:?} in {panel:?}"
        );
        Ok(())
    }

    #[test]
    fn title_only_help_panel_has_equal_visible_top_and_bottom_margins() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let mut builder =
            LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
        build_description_layout(&mut builder, &DescriptionPanel::new("Keys"), &fonts);
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(builder.build())
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("help panel did not solve")?;
        let root = solved.computed[0].bounds;
        let title = solved
            .commands
            .iter()
            .find(|command| format!("{:?}", command.kind).starts_with("Text { text: \"Keys\","))
            .ok_or("title missing")?;
        let size = super::super::integral_advance_size(&fonts, TITLE_SIZE);
        let font = fonts
            .font(FontId::MONOSPACE)
            .map_err(|error| error.to_string())?;
        let metrics = font.metrics(size.0 * 4.0 / 3.0);
        let ppp = Unit::Pixels.to_points();
        let top = (metrics.line_gap.mul_add(0.5, metrics.ascent) - metrics.cap_height)
            .mul_add(ppp, title.bounds.y)
            - root.y;
        let bottom = metrics.line_gap.mul_add(0.5, metrics.descent).mul_add(
            ppp,
            root.y + root.height - title.bounds.y - title.bounds.height,
        );
        assert!((top - bottom).abs() < 0.5, "top {top}, bottom {bottom}");
        Ok(())
    }

    #[test]
    fn fit_description_width_balances_widest_line_ink_margins() -> Result<(), String> {
        const WIDEST_LINE: &str = "Both layers stay in the same panel and keep their layout.";
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let description = DescriptionPanel::new("Panel draw order")
            .with_body_size(LABEL_SIZE.0)
            .with_fit_width()
            .lines([WIDEST_LINE, "Only their drawing order changes."]);
        let mut builder =
            LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
        build_description_layout(&mut builder, &description, &fonts);
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(builder.build())
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("description panel did not solve")?;
        let root = solved.computed[0].bounds;
        let line = solved
            .commands
            .iter()
            .find(|command| {
                format!("{:?}", command.kind)
                    .starts_with(&format!("Text {{ text: {WIDEST_LINE:?},"))
            })
            .ok_or("widest line missing")?;
        let size = super::super::integral_advance_size(&fonts, Pt(LABEL_SIZE.0));
        let TextInkMeasurement::Visible(ink) = fonts
            .measure_ink(WIDEST_LINE, &TextStyle::new(size).as_measure())
            .map_err(|error| error.to_string())?
        else {
            return Err("widest line has no ink".to_string());
        };
        let left = line.bounds.x + ink.left - root.x;
        let right = root.x + root.width - line.bounds.x - ink.right;
        let tolerance = 2.0 * Unit::Pixels.to_points();
        assert!(
            (left - right).abs() <= tolerance,
            "left {left}, right {right}"
        );
        Ok(())
    }
}
