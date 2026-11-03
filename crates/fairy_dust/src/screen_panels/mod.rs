//! Capability: small screen-space panels for examples.

mod constants;
pub(crate) mod description;
mod help_overlay;
mod performance;
pub(crate) mod separation;
mod title_bar;

use bevy::color::Alpha;
use bevy::picking::Pickable;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::ApplyDeferred;
use bevy::prelude::Assets;
use bevy::prelude::ChildOf;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Handle;
use bevy::prelude::Has;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mesh3d;
use bevy::prelude::On;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::SystemSet;
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
use hana_diegetic::PanelSystems;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
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
use separation::EdgePanelPlacementState;
use separation::EdgePanelPlacementStatus;
use separation::EdgePanelSpawnOrder;
pub use separation::SCREEN_PANEL_GAP;
pub(crate) use separation::SCREEN_PANEL_WINDOW_INSET;
pub use title_bar::ControlActivation;
pub(crate) use title_bar::HomeTitleBarFlash;
pub use title_bar::TitleBar;
pub use title_bar::TitleBarControl;
pub(crate) use title_bar::TitleBarControlRegistry;
pub(crate) use title_bar::TitleBarControlState;
pub use title_bar::TitleBarOrientation;
pub(crate) use title_bar::TitleBarOverflow;
pub use title_bar::TitleBarSegment;
pub use title_bar::TitleChip;
pub use title_bar::TitleChipActivation;

use crate::camera_home::CameraHomeConfig;
use crate::constants::BORDER;
use crate::constants::BORDER_ACCENT;
use crate::constants::BORDER_DIM;
use crate::constants::FRAME_PAD;
use crate::constants::INNER_BACKGROUND;
use crate::constants::INNER_BORDER_WIDTH;
use crate::constants::INNER_RADIUS;
use crate::constants::RADIUS;
use crate::constants::screen_spacing;
use crate::ensure_plugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TitleBarOverflowSystems {
    WriteState,
    Decide,
    Read,
}

/// App systems that change resources mirrored by title-bar chips in this update.
/// Fairy Dust shortcut handlers run before the chip sync already; callers do
/// not need to add those handlers to this set.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TitleBarStateWriters;

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

/// Install the shared title bar on an already configured Bevy app.
pub(crate) fn install_title_bar(app: &mut App, title_bar: TitleBar) {
    ensure_plugin(app, DiegeticUiPlugin);
    help_overlay::install(app);
    app.init_resource::<HomeTitleBarFlash>();
    install_title_bar_overflow_refresh(app);
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

fn install_title_bar_overflow_refresh(app: &mut App) {
    app.add_systems(
        Update,
        title_bar::tick_home_title_bar_flash.in_set(TitleBarOverflowSystems::WriteState),
    );
    app.configure_sets(
        Update,
        (
            TitleBarOverflowSystems::WriteState,
            TitleBarOverflowSystems::Decide,
            TitleBarOverflowSystems::Read,
        )
            .chain()
            .before(PanelSystems::ApplyTreeChanges),
    );
    app.configure_sets(
        Update,
        TitleBarStateWriters.before(TitleBarOverflowSystems::WriteState),
    );
    app.add_systems(
        Update,
        title_bar::refresh_changed_title_bar.in_set(TitleBarOverflowSystems::Decide),
    );
    app.add_systems(
        Update,
        ApplyDeferred
            .after(TitleBarOverflowSystems::Decide)
            .before(TitleBarOverflowSystems::Read),
    );
    app.add_systems(
        Update,
        description::refresh_description_for_title_bar_overflow
            .in_set(TitleBarOverflowSystems::Read),
    );
}

pub(crate) fn install_overlay_picking(app: &mut App) {
    app.init_resource::<EdgePanelSpawnOrder>()
        .init_resource::<EdgePanelPlacementStatus>()
        .add_observer(separation::record_edge_panel_spawn);
    app.add_observer(ignore_screen_panel_picking)
        .add_observer(ignore_overlay_mesh_picking_on_mesh_added)
        .add_observer(ignore_overlay_mesh_picking_on_parent_added);
    app.add_systems(
        Update,
        separation::place_edge_panels
            .after(PanelSystems::ResolvePanelAttachments)
            .before(PanelSystems::PositionScreenSpace),
    );
    app.add_systems(
        PostUpdate,
        separation::verify_edge_panel_positions
            .run_if(|status: Res<EdgePanelPlacementStatus>| placement_needs_verification(&status)),
    );
}

const fn placement_needs_verification(status: &EdgePanelPlacementStatus) -> bool {
    matches!(status.state, EdgePanelPlacementState::Pending)
}

#[cfg(test)]
mod placement_tests {
    use super::*;

    #[test]
    fn placed_panels_skip_position_verification() {
        let mut status = EdgePanelPlacementStatus::default();
        assert!(placement_needs_verification(&status));
        status.state = EdgePanelPlacementState::Placed;
        assert!(!placement_needs_verification(&status));
    }
}

#[cfg(test)]
mod border_pixel_tests {
    use std::sync::mpsc;

    use bevy::camera::Hdr;
    use bevy::camera::RenderTarget;
    use bevy::prelude::*;
    use bevy::render::RenderPlugin;
    use bevy::render::render_resource::PollType;
    use bevy::render::render_resource::TextureFormat;
    use bevy::render::renderer::RenderDevice;
    use bevy::render::view::Tonemapping;
    use bevy::render::view::screenshot::Screenshot;
    use bevy::render::view::screenshot::ScreenshotCaptured;
    use bevy::window::ExitCondition;
    use bevy::winit::WinitPlugin;
    use hana_diegetic::Anchor;
    use hana_diegetic::CascadeEntityCommandsExt as _;
    use hana_diegetic::DisplayCompensationReadiness;
    use hana_diegetic::Lighting;
    use hana_diegetic::Mm;

    use super::*;

    const SIZE: u32 = 256;

    fn drawn_border_image(background: Color) -> Result<Vec<u8>, String> {
        let mut app = crate::test_app();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        );
        app.add_plugins(DiegeticUiPlugin);
        app.world_mut().insert_resource(Lighting::Unlit);
        app.insert_resource(ClearColor(background));
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    SIZE,
                    SIZE,
                    TextureFormat::Rgba8UnormSrgb,
                    None,
                ));
        app.world_mut().spawn((
            Camera3d::default(),
            RenderTarget::Image(target.clone().into()),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            Msaa::Off,
            Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let mut layout = LayoutBuilder::new(100.0, 100.0);
        screen_panel_frame(&mut layout, Sizing::GROW, Sizing::GROW, background, |_| {});
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(100.0))
            .anchor(Anchor::Center)
            .material(screen_panel_material_handle(
                &mut app.world_mut().resource_mut::<Assets<StandardMaterial>>(),
            ))
            .with_tree(layout.build())
            .build()
            .map_err(|error| error.to_string())?;
        let panel = app.world_mut().spawn((panel, Transform::default())).id();
        app.world_mut()
            .commands()
            .entity(panel)
            .override_lighting(Lighting::Unlit);
        app.finish();
        app.cleanup();
        for _ in 0..50 {
            if *app.world().resource::<DisplayCompensationReadiness>()
                == DisplayCompensationReadiness::Compensated
            {
                break;
            }
            app.update();
        }
        for _ in 0..4 {
            app.update();
        }
        let (sender, receiver) = mpsc::channel();
        app.world_mut().spawn(Screenshot::image(target)).observe(
            move |event: On<ScreenshotCaptured>| {
                let _ = sender.send(
                    event
                        .image
                        .data
                        .clone()
                        .ok_or_else(|| "screenshot has no pixels".to_string()),
                );
            },
        );
        // Bevy finishes the readback on its async compute pool, which shader
        // compilation can hold for many frames, so this waits on the capture itself.
        loop {
            app.update();
            app.world()
                .resource::<RenderDevice>()
                .wgpu_device()
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout:          None,
                })
                .map_err(|error| format!("GPU screenshot readback failed: {error}"))?;
            if let Ok(image) = receiver.try_recv() {
                return image;
            }
        }
    }

    fn relative_luminance(color: [u8; 3]) -> f32 {
        let linear = color.map(|channel| {
            let srgb = f32::from(channel) / 255.0;
            if srgb <= 0.04045 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            }
        });
        f32::mul_add(
            linear[2],
            0.0722,
            f32::mul_add(linear[1], 0.7152, linear[0] * 0.2126),
        )
    }

    #[test]
    fn screen_panel_border_draws_opaque_pixels_over_light_and_dark_backgrounds()
    -> Result<(), String> {
        let dark = drawn_border_image(Color::BLACK)?;
        let light = drawn_border_image(Color::srgb_u8(89, 91, 95))?;
        let border_offset = dark
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, pixel)| {
                pixel[1] > pixel[0].saturating_add(20) && pixel[2] > pixel[0].saturating_add(40)
            })
            .min_by_key(|(_, pixel)| {
                usize::from(pixel[0].abs_diff(128))
                    + usize::from(pixel[1].abs_diff(191))
                    + usize::from(pixel[2].abs_diff(227))
            })
            .map(|(index, _)| index * 4)
            .ok_or("rendered frame has no cyan border pixel")?;
        let dark = &dark[border_offset..border_offset + 4];
        let light = &light[border_offset..border_offset + 4];
        for channel in 0..3 {
            assert!(
                dark[channel].abs_diff(light[channel]) <= 2,
                "{dark:?} {light:?}"
            );
        }
        let border = [dark[0], dark[1], dark[2]];
        let contrast =
            (relative_luminance(border) + 0.05) / (relative_luminance([89, 91, 95]) + 0.05);
        assert!(contrast >= 3.0, "border={border:?}, contrast={contrast}");
        Ok(())
    }
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
    let Ok(font) = fonts.font(FontId::MONOSPACE) else {
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
    let background = background.with_alpha(1.0);
    builder.with(
        El::new()
            .width(width)
            .height(height)
            .padding(Padding::all(FRAME_PAD))
            .background(background)
            .corner_radius(CornerRadius::all(RADIUS))
            .border(Border::all(BORDER, BORDER_ACCENT)),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .padding(Padding::all(screen_panel_inner_padding()))
                    .corner_radius(CornerRadius::all(INNER_RADIUS))
                    .background(background)
                    .border(Border::all(INNER_BORDER_WIDTH, BORDER_DIM)),
                content,
            );
        },
    );
}

/// Which outer edge of a panel's text stack meets the panel padding.
#[derive(Clone, Copy)]
pub(crate) enum PanelTextEdge {
    Interior,
    First,
    Last,
    Both,
}

/// Text with its outer line-box edge trimmed to visible ink.
pub(crate) fn screen_panel_ink_text(
    text: impl Into<String>,
    style: TextStyle,
    fonts: &FontRegistry,
    size: Pt,
    edge: PanelTextEdge,
) -> Text {
    let (top, bottom) = screen_panel_text_edge_trim(fonts, size);
    let (top, bottom) = match edge {
        PanelTextEdge::Interior => (Px(0.0), Px(0.0)),
        PanelTextEdge::First => (top, Px(0.0)),
        PanelTextEdge::Last => (Px(0.0), bottom),
        PanelTextEdge::Both => (top, bottom),
    };
    Text::new(text, style).trim_text_box(top, bottom)
}

/// Distances from a monospace line box to its visible cap line and baseline.
#[must_use]
pub fn screen_panel_text_edge_trim(fonts: &FontRegistry, size: Pt) -> (Px, Px) {
    let Ok(font) = fonts.font(FontId::MONOSPACE) else {
        return (Px(0.0), Px(0.0));
    };
    let metrics = font.metrics(size.0 * 4.0 / 3.0);
    (
        Px(metrics.line_gap.mul_add(0.5, metrics.ascent) - metrics.cap_height),
        Px(metrics.line_gap.mul_add(0.5, metrics.descent)),
    )
}

fn screen_panel_inner_padding() -> Px {
    let edge = screen_spacing().panel_padding.0;
    Px(edge - FRAME_PAD.0 - BORDER.0 - INNER_BORDER_WIDTH.0)
}

/// Default background color for screen panels — exposed so per-panel
/// builders can substitute it when no override is provided.
pub(super) const fn default_inner_background() -> Color { INNER_BACKGROUND }

fn ignore_screen_panel_picking(
    trigger: On<Add<DiegeticPanel>>,
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
    trigger: On<Add<Mesh3d>>,
    parents: Query<&ChildOf>,
    panels: Query<(), With<FairyDustOverlayPanel>>,
    mut commands: Commands,
) {
    ignore_overlay_mesh_picking(trigger.event_target(), &parents, &panels, &mut commands);
}

fn ignore_overlay_mesh_picking_on_parent_added(
    trigger: On<Add<ChildOf>>,
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
    use bevy::color::Alpha;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::Pickable;
    use bevy::prelude::App;
    use bevy::prelude::ChildOf;
    use bevy::prelude::Color;
    use bevy::prelude::Entity;
    use bevy::prelude::Mesh3d;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Query;
    use bevy::prelude::Visibility;
    use bevy::prelude::Window;
    use bevy::window::PrimaryWindow;
    use hana_diegetic::ComputedDiegeticPanel;
    use hana_diegetic::DiegeticPanel;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::El;
    use hana_diegetic::Fit;
    use hana_diegetic::FontId;
    use hana_diegetic::FontRegistry;
    use hana_diegetic::HeadlessDiegeticUiPlugin;
    use hana_diegetic::LayoutBuilder;
    use hana_diegetic::PanelPicking;
    use hana_diegetic::Sizing;
    use hana_diegetic::TextStyle;
    use hana_diegetic::Unit;

    use super::DescriptionPanel;
    use super::FairyDustOverlayPanel;
    use super::TitleBar;
    use super::TitleBarControlState;
    use super::TitleBarOverflow;
    use super::description::DescriptionPanelMarker;
    use super::install_overlay_picking;
    use super::install_title_bar_overflow_refresh;
    use super::screen_panel_inner_padding;
    use crate::constants::BORDER;
    use crate::constants::FRAME_PAD;
    use crate::constants::INNER_BORDER_WIDTH;
    use crate::constants::screen_spacing;

    #[test]
    fn one_resize_update_moves_each_shortcut_between_bar_and_panels() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(fonts)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        install_title_bar_overflow_refresh(&mut app);
        app.init_resource::<super::HomeTitleBarFlash>();
        let mut window = Window::default();
        window.resolution.set(1200.0, 800.0);
        let window_entity = app.world_mut().spawn((window, PrimaryWindow)).id();
        let empty_panel = || {
            DiegeticPanel::screen()
                .size(Sizing::FIT, Sizing::FIT)
                .build()
                .map_err(|error| error.to_string())
        };
        let bar = TitleBar::new()
            .with_title("Widget Lab")
            .control("H Home")
            .control("D Enable/disable");
        let bar_entity = app
            .world_mut()
            .spawn((
                bar,
                TitleBarControlState::default(),
                TitleBarOverflow::default(),
                Visibility::Inherited,
                empty_panel()?,
            ))
            .id();
        let keys_entity = app
            .world_mut()
            .spawn((
                DescriptionPanelMarker,
                DescriptionPanel::new("Keys")
                    .line_when_title_control_hidden("H Home", "H Home")
                    .line_when_title_control_hidden("D Enable/disable", "D Enable/disable"),
                empty_panel()?,
            ))
            .id();
        for width in [1200.0, 400.0, 200.0, 1200.0] {
            app.world_mut()
                .get_mut::<Window>(window_entity)
                .ok_or("window missing")?
                .resolution
                .set(width, 800.0);
            app.update();
            let rendered = |entity| -> Result<String, String> {
                let solved = app
                    .world()
                    .get::<ComputedDiegeticPanel>(entity)
                    .and_then(|panel| panel.layout().solved())
                    .ok_or("panel did not solve")?;
                Ok(format!("{:?}", solved.commands))
            };
            let bar = rendered(bar_entity)?;
            let keys = rendered(keys_entity)?;
            let home_location = app
                .world_mut()
                .run_system_once(
                    |bars: Query<(&TitleBar, &TitleBarOverflow, Option<&Visibility>)>| {
                        crate::camera_control_panel::home_hint_location(&bars)
                    },
                )
                .map_err(|error| error.to_string())?;
            assert_ne!(
                bar.contains("D Enable/disable"),
                keys.contains("D Enable/disable"),
                "D duplicated or missing at {width}"
            );
            assert_ne!(
                bar.contains("H Home"),
                home_location == crate::camera_control_panel::HomeHintLocation::CameraPanel,
                "H duplicated or missing at {width}"
            );
        }
        Ok(())
    }

    #[test]
    fn screen_frame_has_equal_scale_padding_on_every_edge() {
        let inset = screen_panel_inner_padding().0 + FRAME_PAD.0 + BORDER.0 + INNER_BORDER_WIDTH.0;
        let spacing = screen_spacing();
        assert!((inset - spacing.panel_padding.0).abs() < 1e-4);
        assert!(spacing.panel_bottom.0 >= inset);
    }

    #[test]
    fn screen_frame_padding_reaches_visible_text_ink_on_all_sides() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let size = super::integral_advance_size(&fonts, crate::constants::LABEL_SIZE);
        let mut builder =
            LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
        super::screen_panel_frame(
            &mut builder,
            Sizing::FIT,
            Sizing::FIT,
            crate::constants::DEFAULT_PANEL_BACKGROUND,
            |builder| {
                builder.text(super::screen_panel_ink_text(
                    "Visible ink",
                    TextStyle::new(size),
                    &fonts,
                    size,
                    super::PanelTextEdge::Both,
                ));
            },
        );
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
            .ok_or("ink panel did not solve")?;
        let root = solved.computed[0].bounds;
        let ink_command = solved
            .commands
            .iter()
            .find(|command| format!("{:?}", command.kind).starts_with("Text {"))
            .ok_or("ink command missing")?;
        let ink = ink_command.bounds;
        let font = fonts
            .font(FontId::MONOSPACE)
            .map_err(|error| error.to_string())?;
        let metrics = font.metrics(size.0 * 4.0 / 3.0);
        let padding = screen_spacing().panel_padding.0;
        let point_per_pixel = Unit::Pixels.to_points();
        let expected = padding * point_per_pixel;
        let top_offset = metrics.line_gap.mul_add(0.5, metrics.ascent) - metrics.cap_height;
        let ink_top = top_offset.mul_add(point_per_pixel, ink.y);
        let ink_bottom = metrics
            .line_gap
            .mul_add(0.5, metrics.descent)
            .mul_add(-point_per_pixel, ink.y + ink.height);
        for margin in [
            ink.x - root.x,
            ink_top - root.y,
            root.x + root.width - ink.x - ink.width,
            root.y + root.height - ink_bottom,
        ] {
            assert!(
                (margin - expected).abs() <= 1.0,
                "visible ink margin {margin} vs {expected}"
            );
        }
        Ok(())
    }

    #[test]
    fn panel_fill_preserves_body_and_title_contrast_over_white() {
        fn luminance(color: Color) -> f32 {
            let rgb = color.to_srgba();
            let channel = |value: f32| {
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            channel(rgb.blue).mul_add(
                0.0722,
                channel(rgb.green).mul_add(0.7152, channel(rgb.red) * 0.2126),
            )
        }
        fn over(foreground: Color, background: Color) -> Color {
            let foreground = foreground.to_srgba();
            let background = background.to_srgba();
            let blend = |text: f32, panel: f32| {
                panel.mul_add(1.0 - foreground.alpha, text * foreground.alpha)
            };
            bevy::prelude::Color::srgb(
                blend(foreground.red, background.red),
                blend(foreground.green, background.green),
                blend(foreground.blue, background.blue),
            )
        }
        let white = bevy::prelude::Color::WHITE;
        for fill in [
            crate::constants::DEFAULT_PANEL_BACKGROUND,
            crate::camera_control_panel::CameraControlPanelBackground::default().0,
        ] {
            let composited_fill = over(fill, white);
            assert!((fill.alpha() - 1.0).abs() < f32::EPSILON);
            for text in [super::constants::BODY_COLOR, crate::constants::TITLE_COLOR] {
                let composited_text = over(text, composited_fill);
                let contrast =
                    (luminance(composited_text) + 0.05) / (luminance(composited_fill) + 0.05);
                assert!(contrast >= 4.5, "contrast {contrast} is below 4.5:1");
            }
        }
    }

    #[test]
    fn screen_panel_root_passes_pointer_through() -> Result<(), String> {
        let mut app = crate::test_app();
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
        let mut app = crate::test_app();
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
        let mut app = crate::test_app();
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
        let mut app = crate::test_app();
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
        let mut app = crate::test_app();
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
