//! Capability: small screen-space panels for examples.

mod constants;
mod description;
mod help_overlay;
mod performance;
mod title_bar;

use bevy::picking::Pickable;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::ChildOf;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Handle;
use bevy::prelude::Has;
use bevy::prelude::Mesh3d;
use bevy::prelude::On;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Update;
use bevy::prelude::With;
pub use description::DescriptionPanel;
use hana_diegetic::Border;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::FontId;
use hana_diegetic::FontRegistry;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::PanelPicking;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::default_panel_material;
pub use performance::StatsPanelRow;
pub use performance::StatsPanelSection;
pub use performance::diegetic_stats_panel;
pub use performance::diegetic_stats_panel_with_integral_advance;
pub use performance::diegetic_stats_sections_panel;
pub use performance::diegetic_stats_sections_panel_with_integral_advance;
pub use performance::diegetic_stats_sections_tree;
pub use performance::diegetic_stats_sections_tree_with_integral_advance;
pub use performance::diegetic_stats_tree;
pub use performance::diegetic_stats_tree_with_integral_advance;
pub use performance::fps_stats_panel;
pub use performance::gpu_meter_panel;
pub use title_bar::ControlActivation;
pub(crate) use title_bar::HomeTitleBarFlash;
pub use title_bar::TitleBar;
pub use title_bar::TitleBarControl;
pub(crate) use title_bar::TitleBarControlRegistry;
pub(crate) use title_bar::TitleBarControlState;
pub use title_bar::TitleBarOrientation;
pub use title_bar::TitleBarSegment;
pub use title_bar::TitleChip;
pub use title_bar::TitleChipActivation;
pub(crate) use title_bar::refresh_changed_title_bar;

use crate::camera_home::CameraHomeConfig;
use crate::constants::BORDER;
use crate::constants::BORDER_ACCENT;
use crate::constants::BORDER_DIM;
use crate::constants::FRAME_PAD;
use crate::constants::INNER_BACKGROUND;
use crate::constants::INNER_BORDER_WIDTH;
use crate::constants::INNER_PAD;
use crate::constants::INNER_RADIUS;
use crate::constants::RADIUS;
use crate::ensure_plugin;

#[derive(Component)]
struct FairyDustOverlayPanel;

pub(crate) fn install_description(app: &mut App, panel: DescriptionPanel) {
    ensure_plugin(app, DiegeticUiPlugin);
    app.add_systems(
        Startup,
        move |mut commands: Commands,
              fonts: Res<FontRegistry>,
              mut materials: ResMut<Assets<StandardMaterial>>| {
            description::spawn_description_panel(&mut commands, &panel, &fonts, &mut materials);
        },
    );
}

pub(crate) fn install_title_bar(app: &mut App, title_bar: TitleBar) {
    ensure_plugin(app, DiegeticUiPlugin);
    help_overlay::install(app);
    app.init_resource::<HomeTitleBarFlash>();
    app.add_systems(Update, title_bar::tick_home_title_bar_flash);
    app.add_systems(PostUpdate, title_bar::refresh_changed_title_bar);
    app.add_systems(
        Startup,
        move |mut commands: Commands,
              mut materials: ResMut<Assets<StandardMaterial>>,
              fonts: Res<FontRegistry>,
              home: Option<Res<CameraHomeConfig>>,
              control_registry: Option<Res<TitleBarControlRegistry>>| {
            title_bar::spawn_title_bar_with_home_chip(
                &mut commands,
                &mut materials,
                &title_bar,
                &fonts,
                home.as_deref(),
                control_registry.as_deref(),
            );
        },
    );
}

pub(crate) fn install_overlay_picking(app: &mut App) {
    app.add_observer(ignore_screen_panel_picking)
        .add_observer(ignore_overlay_mesh_picking_on_mesh_added)
        .add_observer(ignore_overlay_mesh_picking_on_parent_added);
}

pub(crate) fn register_title_control(app: &mut App, control: impl Into<TitleBarControl>) {
    let mut registry = app
        .world_mut()
        .get_resource_or_insert_with(TitleBarControlRegistry::default);
    registry.push(control);
}

/// Material used by Fairy Dust screen-space panels.
#[must_use]
pub fn screen_panel_material() -> StandardMaterial {
    StandardMaterial {
        unlit: true,
        ..default_panel_material()
    }
}

/// Registers and returns the material used by Fairy Dust screen-space panels.
pub fn screen_panel_material_handle(
    materials: &mut Assets<StandardMaterial>,
) -> Handle<StandardMaterial> {
    materials.add(screen_panel_material())
}

/// Resolves a screen-panel request to the nearest size whose monospace glyph
/// advance spans a whole logical pixel.
pub(crate) fn integral_advance_size(fonts: &FontRegistry, requested: Pt) -> Pt {
    let Some(font) = fonts.font(FontId::MONOSPACE) else {
        bevy::log::warn_once!(
            "fairy_dust: embedded monospace font is unavailable; using requested text size"
        );
        return requested;
    };
    match font.nearest_integral_advance_size(requested) {
        Ok(size) => size,
        Err(error) => {
            bevy::log::warn_once!(
                "fairy_dust: cannot resolve an integral-advance screen text size: {error}; \
                 using requested size"
            );
            requested
        },
    }
}

/// Adds the standard Fairy Dust screen-panel frame, then lets the caller
/// populate the inner panel contents.
pub fn screen_panel_frame(
    builder: &mut LayoutBuilder,
    width: Sizing,
    height: Sizing,
    background: Color,
    content: impl FnOnce(&mut LayoutBuilder),
) {
    builder.with(
        El::new()
            .width(width)
            .height(height)
            .padding(Padding::all(FRAME_PAD))
            .corner_radius(CornerRadius::all(RADIUS))
            .border(Border::all(BORDER, BORDER_ACCENT)),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .padding(Padding::all(INNER_PAD))
                    .corner_radius(CornerRadius::all(INNER_RADIUS))
                    .background(background)
                    .border(Border::all(INNER_BORDER_WIDTH, BORDER_DIM)),
                content,
            );
        },
    );
}

/// Default background color for screen panels — exposed so per-panel
/// builders can substitute it when no override is provided.
pub(super) const fn default_inner_background() -> Color { INNER_BACKGROUND }

fn ignore_screen_panel_picking(
    trigger: On<Add, DiegeticPanel>,
    panels: Query<(&DiegeticPanel, Has<PanelPicking>)>,
    mut commands: Commands,
) {
    let entity = trigger.event_target();
    let Ok((panel, has_authored_picking)) = panels.get(entity) else {
        return;
    };
    if panel.coordinate_space().is_screen() && !has_authored_picking {
        commands
            .entity(entity)
            .insert((FairyDustOverlayPanel, PanelPicking::PASS_THROUGH));
    }
}

fn ignore_overlay_mesh_picking_on_mesh_added(
    trigger: On<Add, Mesh3d>,
    parents: Query<&ChildOf>,
    panels: Query<(), With<FairyDustOverlayPanel>>,
    mut commands: Commands,
) {
    ignore_overlay_mesh_picking(trigger.event_target(), &parents, &panels, &mut commands);
}

fn ignore_overlay_mesh_picking_on_parent_added(
    trigger: On<Add, ChildOf>,
    meshes: Query<(), With<Mesh3d>>,
    parents: Query<&ChildOf>,
    panels: Query<(), With<FairyDustOverlayPanel>>,
    mut commands: Commands,
) {
    let entity = trigger.event_target();
    if meshes.get(entity).is_ok() {
        ignore_overlay_mesh_picking(entity, &parents, &panels, &mut commands);
    }
}

fn ignore_overlay_mesh_picking(
    entity: Entity,
    parents: &Query<&ChildOf>,
    panels: &Query<(), With<FairyDustOverlayPanel>>,
    commands: &mut Commands,
) {
    if has_overlay_panel_ancestor(entity, parents, panels) {
        commands.entity(entity).insert(Pickable::IGNORE);
    }
}

fn has_overlay_panel_ancestor(
    entity: Entity,
    parents: &Query<&ChildOf>,
    panels: &Query<(), With<FairyDustOverlayPanel>>,
) -> bool {
    let mut current = entity;
    while let Ok(parent) = parents.get(current) {
        let parent_entity = parent.parent();
        if panels.get(parent_entity).is_ok() {
            return true;
        }
        current = parent_entity;
    }
    false
}

#[cfg(test)]
mod tests {
    use bevy::picking::Pickable;
    use bevy::prelude::App;
    use bevy::prelude::ChildOf;
    use bevy::prelude::Entity;
    use bevy::prelude::Mesh3d;
    use hana_diegetic::DiegeticPanel;
    use hana_diegetic::PanelPicking;
    use hana_diegetic::Sizing;

    use super::FairyDustOverlayPanel;
    use super::install_overlay_picking;

    #[test]
    fn screen_panel_root_passes_pointer_through() -> Result<(), String> {
        let mut app = App::new();
        install_overlay_picking(&mut app);

        let panel = spawn_screen_panel(&mut app)?;

        // The overlay root installs pass-through picking so it cannot block the
        // diegetic backend for the widgets example or any panel behind it.
        assert_eq!(
            app.world().get::<PanelPicking>(panel),
            Some(&PanelPicking::PASS_THROUGH),
        );
        assert!(app.world().get::<FairyDustOverlayPanel>(panel).is_some());
        Ok(())
    }

    #[test]
    fn explicit_screen_panel_picking_remains_authoritative() -> Result<(), String> {
        let mut app = App::new();
        install_overlay_picking(&mut app);

        let panel = DiegeticPanel::screen()
            .size(Sizing::FIT, Sizing::FIT)
            .build()
            .map_err(|error| format!("{error}"))?;
        let entity = app
            .world_mut()
            .spawn((panel, PanelPicking::INTERACTIVE))
            .id();

        assert_eq!(
            app.world().get::<PanelPicking>(entity),
            Some(&PanelPicking::INTERACTIVE),
        );
        assert!(app.world().get::<FairyDustOverlayPanel>(entity).is_none());
        Ok(())
    }

    #[test]
    fn mesh_child_under_screen_panel_ignores_picking() -> Result<(), String> {
        let mut app = App::new();
        install_overlay_picking(&mut app);

        let panel = spawn_screen_panel(&mut app)?;
        let child = app
            .world_mut()
            .spawn((Mesh3d::default(), ChildOf(panel)))
            .id();

        assert_eq!(app.world().get::<Pickable>(child), Some(&Pickable::IGNORE));
        Ok(())
    }

    #[test]
    fn mesh_grandchild_under_screen_panel_ignores_picking() -> Result<(), String> {
        let mut app = App::new();
        install_overlay_picking(&mut app);

        let panel = spawn_screen_panel(&mut app)?;
        let child = app.world_mut().spawn(ChildOf(panel)).id();
        let grandchild = app
            .world_mut()
            .spawn((Mesh3d::default(), ChildOf(child)))
            .id();

        assert_eq!(
            app.world().get::<Pickable>(grandchild),
            Some(&Pickable::IGNORE)
        );
        Ok(())
    }

    #[test]
    fn unrelated_mesh_keeps_default_pickability() {
        let mut app = App::new();
        install_overlay_picking(&mut app);

        let entity = app.world_mut().spawn(Mesh3d::default()).id();

        assert!(app.world().get::<Pickable>(entity).is_none());
    }

    fn spawn_screen_panel(app: &mut App) -> Result<Entity, String> {
        let panel = DiegeticPanel::screen()
            .size(Sizing::FIT, Sizing::FIT)
            .build()
            .map_err(|error| format!("{error}"))?;
        Ok(app.world_mut().spawn(panel).id())
    }
}
