//! The bottom-left info panel. For the anchor demo it shows the Navigation
//! legend over the followed/following anchor sections; for the hinge chain it
//! shows the arrangement (`A`/`C`), direction (`F`/`B`), and action (`U`/`D`/`R`)
//! control rows. Rebuilt only when a shown field changes.

use bevy::prelude::Alpha;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::Mix;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Transform;
use bevy::prelude::With;
use bevy::prelude::error;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::screen_panel_frame;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;

use crate::anchor_demo::AnchorDirection;
use crate::anchor_demo::AnchorSelection;
use crate::anchor_demo::LegendGlow;
use crate::anchor_demo::LegendHighlight;
use crate::anchor_demo::SelectedPanel;
use crate::constants::*;
use crate::hinge::ChainArrangement;
use crate::hinge::FoldAction;
use crate::hinge::FoldDirection;
use crate::hinge::FoldTravel;
use crate::hinge::HingeChain;
use crate::presentation::AnchorPanelMaterials;
use crate::scene::ActiveCapability;

#[derive(Component)]
pub(crate) struct AnchorInfoPanel;

pub(crate) fn spawn_info_panel(
    commands: &mut Commands,
    selection: AnchorSelection,
    selected: SelectedPanel,
    active_index: usize,
    materials: &AnchorPanelMaterials,
) {
    let built = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(materials.screen.clone())
        .text_material(materials.screen.clone())
        .with_tree(build_info_panel_tree(
            selection,
            selected,
            LegendGlow::default(),
            active_index,
            HingeChain::default(),
        ))
        .build();
    match built {
        Ok(panel) => {
            commands.spawn((AnchorInfoPanel, panel, Transform::default()));
        },
        Err(error) => error!("panel_anchoring: failed to build anchor info panel: {error}"),
    }
}

/// Sole rebuilder of the info panel. Rebuilds only when a displayed field
/// changes: the active anchors, which panel `Tab` selects, or the legend glow.
/// The depth offset is not shown here, so a held depth change rebuilds the panel
/// only through its glow turning on and off.
pub(crate) fn reconcile_info_panel(
    selection: Res<AnchorSelection>,
    selected: Res<SelectedPanel>,
    active: Res<ActiveCapability>,
    highlight: Res<LegendHighlight>,
    hinge: Res<HingeChain>,
    info_panels: Query<Entity, With<AnchorInfoPanel>>,
    mut last: Local<
        Option<(
            usize,
            usize,
            SelectedPanel,
            LegendGlow,
            usize,
            ChainArrangement,
            FoldDirection,
            FoldTravel,
            FoldAction,
        )>,
    >,
    mut commands: Commands,
) {
    let glow = highlight.glow();
    let chain = *hinge;
    let current = (
        selection.source_index,
        selection.target_index,
        *selected,
        glow,
        active.index,
        chain.arrangement(),
        chain.direction(),
        chain.travel(),
        chain.action(),
    );
    if *last == Some(current) {
        return;
    }
    *last = Some(current);
    if let Ok(info_panel) = info_panels.single()
        && let Err(error) = commands.set_tree(
            info_panel,
            build_info_panel_tree(*selection, *selected, glow, active.index, chain),
        )
    {
        error!("failed to replace anchor info panel tree: {error}");
    }
}

fn build_info_panel_tree(
    selection: AnchorSelection,
    selected: SelectedPanel,
    glow: LegendGlow,
    active_index: usize,
    hinge: HingeChain,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::fixed(INFO_PANEL_WIDTH),
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(INFO_SECTION_GAP),
                |builder| {
                    if active_index == HINGE_CHAIN_INDEX {
                        hinge_info_section(builder, hinge);
                    } else {
                        anchor_info_sections(builder, selection, selected, glow);
                    }
                },
            );
        },
    );
    builder.build()
}

/// The spin / anchor-selection info body: the Navigation legend over
/// the followed/following anchor sections.
fn anchor_info_sections(
    builder: &mut LayoutBuilder,
    selection: AnchorSelection,
    selected: SelectedPanel,
    glow: LegendGlow,
) {
    info_direction_legend(builder, glow);
    info_divider(builder);
    info_section(
        builder,
        PanelRole::Target,
        "followed:",
        selection.target_label(),
        selection.target_index,
        selected == SelectedPanel::Target,
    );
    info_section(
        builder,
        PanelRole::Dependent,
        "following:",
        selection.source_label(),
        selection.source_index,
        selected == SelectedPanel::Anchored,
    );
}

/// The hinge-chain info body: a `Modes` section (arrangement `A`/`C`, direction
/// `F`/`B`, travel `G`/`S`), a divider, then an `Actions` section (`U`/`D` fold,
/// `+`/`-` resize, and a centered `R Reset`). The lit option in each mode pair
/// tracks the chain's current selection.
fn hinge_info_section(builder: &mut LayoutBuilder, hinge: HingeChain) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(INFO_SECTION_GAP),
        |builder| {
            hinge_mode_section(builder, hinge);
            info_divider(builder);
            hinge_action_section(builder, hinge.action());
        },
    );
}

/// The `Modes` section: a header over two-column rows of the persistent toggles.
fn hinge_mode_section(builder: &mut LayoutBuilder, hinge: HingeChain) {
    let arrangement = hinge.arrangement();
    let direction = hinge.direction();
    let travel = hinge.travel();
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(NAV_TITLE_GAP),
        |builder| {
            builder.text(("Modes", nav_text_style(INFO_TITLE_SIZE, false)));
            hinge_option_columns(
                builder,
                ("A", "Accordion", arrangement == ChainArrangement::Accordion),
                ("C", "Coil", arrangement == ChainArrangement::Coil),
            );
            hinge_option_columns(
                builder,
                ("F", "Front", direction == FoldDirection::Front),
                ("B", "Back", direction == FoldDirection::Back),
            );
            hinge_option_columns(
                builder,
                ("S", "Step", travel == FoldTravel::Step),
                ("G", "Glide", travel == FoldTravel::Glide),
            );
        },
    );
}

/// The `Actions` section: a header over the `U`/`D` fold and `+`/`-` resize pairs
/// in two columns, with a centered `R Reset` on its own line below.
fn hinge_action_section(builder: &mut LayoutBuilder, action: FoldAction) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(NAV_TITLE_GAP),
        |builder| {
            builder.text(("Actions", nav_text_style(INFO_TITLE_SIZE, false)));
            hinge_option_columns(
                builder,
                ("U", "Up", action == FoldAction::Up),
                ("D", "Down", action == FoldAction::Down),
            );
            hinge_option_columns(builder, ("+", "Add", false), ("-", "Remove", false));
            builder.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    hinge_option(builder, "R", "Reset", false);
                },
            );
        },
    );
}

/// A row of two equal-width option cells, so the second option's `key label`
/// lines up in a column across the stacked rows.
fn hinge_option_columns(
    builder: &mut LayoutBuilder,
    left: (&str, &str, bool),
    right: (&str, &str, bool),
) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(INFO_COL_GAP)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            hinge_option_cell(builder, left);
            hinge_option_cell(builder, right);
        },
    );
}

/// One column cell: a grow-width `key label` pair, lit yellow when selected.
fn hinge_option_cell(builder: &mut LayoutBuilder, option: (&str, &str, bool)) {
    let (key, label, lit) = option;
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(NAV_HINT_WORD_GAP)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text((key, nav_text_style(NAV_HINT_SIZE, lit)));
            builder.text((label, nav_text_style(NAV_HINT_SIZE, lit)));
        },
    );
}

/// A single `key label` pair sized to its content, lit yellow when selected.
fn hinge_option(builder: &mut LayoutBuilder, key: &str, label: &str, lit: bool) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(NAV_HINT_WORD_GAP)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text((key, nav_text_style(NAV_HINT_SIZE, lit)));
            builder.text((label, nav_text_style(NAV_HINT_SIZE, lit)));
        },
    );
}

/// A thin horizontal rule separating the anchor sections from the Navigation
/// section.
fn info_divider(builder: &mut LayoutBuilder) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .padding(Padding::new(0.0, 0.0, INFO_DIVIDER_PAD, INFO_DIVIDER_PAD)),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(INFO_DIVIDER_THICKNESS))
                    .background(INFO_DIVIDER_COLOR),
                |_| {},
            );
        },
    );
}

/// The "Navigation" section: a `Tab to change Panel` hint over a centered cross
/// of arrows (`↑` above, `↓` below, `← R Reset →` across the middle). The
/// controls named by `glow` light yellow. Depth (`[`/`]`) lives on the title bar.
fn info_direction_legend(builder: &mut LayoutBuilder, glow: LegendGlow) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(NAV_TITLE_GAP),
        |builder| {
            builder.text(("Navigation", nav_text_style(INFO_TITLE_SIZE, false)));
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(NAV_GAP)
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    nav_tab_hint(builder);
                    nav_arrow(builder, AnchorDirection::Top, glow);
                    nav_middle_row(builder, glow);
                    nav_arrow(builder, AnchorDirection::Bottom, glow);
                },
            );
        },
    );
}

/// The two static hint lines above the arrow cross: what `Tab` does and what the
/// arrow keys do. Left-aligned, word-wrapped across the panel width, with a
/// paragraph gap between them.
fn nav_tab_hint(builder: &mut LayoutBuilder) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(NAV_HINT_LINE_GAP),
        |builder| {
            builder.text(
                Text::new(
                    "Tab to change which panel's anchor point you are adjusting",
                    nav_text_style(NAV_HINT_SIZE, false),
                )
                .layout(El::new().width(Sizing::GROW).height(Sizing::FIT)),
            );
            builder.text(
                Text::new(
                    "Use arrow keys to change the anchor point",
                    nav_text_style(NAV_HINT_SIZE, false),
                )
                .layout(El::new().width(Sizing::GROW).height(Sizing::FIT)),
            );
        },
    );
}

/// A single arrow glyph, glowing yellow while it is the lit direction.
fn nav_arrow(builder: &mut LayoutBuilder, direction: AnchorDirection, glow: LegendGlow) {
    builder.text((
        direction.glyph(),
        nav_text_style(NAV_GLYPH_SIZE, glow.direction == Some(direction)),
    ));
}

/// The middle row of the cross: `←`, a centered `R Reset`, and `→`. The arrows
/// are equal-width glyphs, so `R Reset` sits at the row center and the up/down
/// arrows line up over it.
fn nav_middle_row(builder: &mut LayoutBuilder, glow: LegendGlow) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(NAV_MIDDLE_GAP)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            nav_arrow(builder, AnchorDirection::Left, glow);
            builder.text((
                AnchorDirection::Reset.glyph(),
                nav_text_style(
                    NAV_CENTER_SIZE,
                    glow.direction == Some(AnchorDirection::Reset),
                ),
            ));
            nav_arrow(builder, AnchorDirection::Right, glow);
        },
    );
}

fn nav_text_style(size: f32, active: bool) -> TextStyle {
    let color = if active {
        INFO_LEGEND_ACTIVE
    } else {
        INFO_LABEL_COLOR
    };
    TextStyle::new(size)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn info_section(
    builder: &mut LayoutBuilder,
    role: PanelRole,
    label: &str,
    value: &str,
    active_index: usize,
    selected: bool,
) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(INFO_ROW_GAP),
        |builder| {
            builder.text((
                role.title(),
                info_title_style(PanelRole::accent_color(), selected),
            ));
            info_anchor_row(
                builder,
                label,
                value,
                active_index,
                PanelRole::accent_color(),
            );
        },
    );
}

fn info_anchor_row(
    builder: &mut LayoutBuilder,
    label: &str,
    value: &str,
    active_index: usize,
    accent: Color,
) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(INFO_COL_GAP)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            info_anchor_text(builder, label, value, accent);
            info_anchor_grid(builder, active_index, accent);
        },
    );
}

fn info_anchor_text(builder: &mut LayoutBuilder, label: &str, value: &str, accent: Color) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(1.0)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text((label, info_label_style()));
            builder.text((value, info_value_style(accent)));
        },
    );
}

fn info_anchor_grid(builder: &mut LayoutBuilder, active_index: usize, accent: Color) {
    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(INFO_GRID_GAP),
        |builder| {
            for row in 0..INFO_GRID_SIDE {
                info_anchor_grid_row(builder, row, active_index, accent);
            }
        },
    );
}

fn info_anchor_grid_row(
    builder: &mut LayoutBuilder,
    row: usize,
    active_index: usize,
    accent: Color,
) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(INFO_GRID_GAP),
        |builder| {
            for column in 0..INFO_GRID_SIDE {
                let index = row * INFO_GRID_SIDE + column;
                info_anchor_grid_cell(builder, index == active_index, accent);
            }
        },
    );
}

fn info_anchor_grid_cell(builder: &mut LayoutBuilder, active: bool, accent: Color) {
    let background = if active { accent } else { INFO_GRID_INACTIVE };
    builder.with(
        El::new()
            .width(Sizing::fixed(INFO_GRID_CELL_SIZE))
            .height(Sizing::fixed(INFO_GRID_CELL_SIZE))
            .background(background)
            .border(Border::all(INFO_GRID_BORDER_WIDTH, INFO_GRID_BORDER)),
        |_| {},
    );
}

fn info_title_style(accent: Color, selected: bool) -> TextStyle {
    let color = if selected {
        INFO_LEGEND_ACTIVE
    } else {
        accent.with_alpha(INFO_TITLE_DIM_ALPHA)
    };
    TextStyle::new(INFO_TITLE_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn info_label_style() -> TextStyle {
    TextStyle::new(INFO_BODY_SIZE)
        .with_color(INFO_LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn info_value_style(accent: Color) -> TextStyle {
    TextStyle::new(INFO_BODY_SIZE)
        .with_color(BODY_COLOR.mix(&accent, 0.22))
        .with_shadow_mode(GlyphShadowMode::None)
}

#[derive(Clone, Copy)]
enum PanelRole {
    Target,
    Dependent,
}

impl PanelRole {
    const fn title(self) -> &'static str {
        match self {
            Self::Target => "Target Panel",
            Self::Dependent => "Anchored Panel",
        }
    }

    /// Both roles read in the neutral body color; the tiles carry their own
    /// color-wheel accents, so the info sections stay uncolored and are told apart
    /// by their titles and the `Tab`-selected brightness alone.
    const fn accent_color() -> Color { BODY_COLOR }
}
