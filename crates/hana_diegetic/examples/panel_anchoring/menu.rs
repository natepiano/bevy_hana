//! The top-right capability menu: one highlighted row per animation, rebuilt
//! when the active capability changes.

use bevy::prelude::Alpha;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
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
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;

use crate::constants::*;
use crate::presentation::AnchorPanelMaterials;
use crate::scene::ActiveCapability;

#[derive(Component)]
pub(crate) struct CapabilityMenuPanel;

pub(crate) fn spawn_capability_menu(
    commands: &mut Commands,
    active: ActiveCapability,
    materials: &AnchorPanelMaterials,
) {
    let built = DiegeticPanel::screen()
        .size(Sizing::FIT, Sizing::FIT)
        .anchor(Anchor::TopRight)
        .material(materials.screen.clone())
        .text_material(materials.screen.clone())
        .with_tree(build_capability_menu_tree(active))
        .build();
    match built {
        Ok(panel) => {
            commands.spawn((CapabilityMenuPanel, panel, Transform::default()));
        },
        Err(error) => error!("panel_anchoring: failed to build capability menu: {error}"),
    }
}

/// Rebuilds the capability menu when the active capability changes, so the new
/// scene's entry highlights.
pub(crate) fn reconcile_menu(
    active: Res<ActiveCapability>,
    menus: Query<Entity, With<CapabilityMenuPanel>>,
    mut commands: Commands,
) {
    if !active.is_changed() {
        return;
    }
    if let Ok(menu) = menus.single()
        && let Err(error) = commands.set_tree(menu, build_capability_menu_tree(*active))
    {
        error!("failed to replace capability menu tree: {error}");
    }
}

fn build_capability_menu_tree(active: ActiveCapability) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::FIT,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(MENU_SECTION_GAP),
                |builder| {
                    builder.text(("Animations", menu_header_style()));
                    builder.with(
                        El::column()
                            .width(Sizing::GROW)
                            .height(Sizing::FIT)
                            .gap(MENU_ROW_GAP),
                        |builder| {
                            for (index, name) in CAPABILITY_NAMES.iter().enumerate() {
                                capability_menu_row(builder, index, name, active.index == index);
                            }
                        },
                    );
                },
            );
        },
    );
    builder.build()
}

fn capability_menu_row(builder: &mut LayoutBuilder, index: usize, name: &str, active: bool) {
    let background = if active {
        MENU_HIGHLIGHT.with_alpha(MENU_HIGHLIGHT_ALPHA)
    } else {
        Color::NONE
    };
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(MENU_ROW_COL_GAP)
            .padding(Padding::all(MENU_ROW_PADDING))
            .corner_radius(MENU_ROW_CORNER)
            .background(background)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text(((index + 1).to_string(), menu_number_style(active)));
            builder.text((name, menu_name_style(active)));
        },
    );
}

fn menu_header_style() -> TextStyle {
    TextStyle::new(MENU_TITLE_SIZE)
        .with_color(MENU_HEADER_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn menu_number_style(active: bool) -> TextStyle {
    let color = if active {
        MENU_HIGHLIGHT
    } else {
        MENU_IDLE_COLOR
    };
    TextStyle::new(MENU_ROW_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn menu_name_style(active: bool) -> TextStyle {
    let color = if active {
        MENU_ACTIVE_COLOR
    } else {
        MENU_IDLE_COLOR
    };
    TextStyle::new(MENU_ROW_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}
