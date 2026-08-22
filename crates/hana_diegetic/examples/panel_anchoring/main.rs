//! Panel-to-panel anchoring example.
//!
//! `Tab` selects which panel the arrow keys move; the arrows then step that
//! panel's anchor around its 3×3 grid without wrapping. Holding `[`/`]`
//! moves the depth offset continuously along the target's plane normal, four
//! times as fast while `Ctrl` is held. `I` toggles autofit (on by default):
//! while on, the camera holds its pose until the panel union overflows the
//! viewport, then reframes it back inside. The
//! active anchor markers are elements of each panel's layout tree and ease to
//! their new positions in step with the panel's slide; a gizmo line links the
//! two anchor points when a depth offset separates them. The bottom-left info
//! panel names the active anchors and shows the Navigation legend (the selected
//! panel's title is at full strength); the depth label stays attached to the
//! anchored panel itself, near its bottom edge.

mod anchor_demo;
mod constants;
mod hinge;
mod info_panel;
mod menu;
mod presentation;
mod scene;

use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::PostUpdate;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Update;
use bevy::transform::TransformSystems;
use fairy_dust::OrbitCamPose;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use fairy_dust::screen_panel_material;
use hana_diegetic::PanelSystems;
use hana_lagrange::OrbitCamPreset;

use crate::anchor_demo::AnchorChain;
use crate::anchor_demo::AnchorSelection;
use crate::anchor_demo::AnchorTransition;
use crate::anchor_demo::Autofit;
use crate::anchor_demo::LegendHighlight;
use crate::anchor_demo::SelectedPanel;
use crate::anchor_demo::ShowAnchorMarkers;
use crate::anchor_demo::Spin;
use crate::anchor_demo::TileCountFlash;
use crate::anchor_demo::advance_animations;
use crate::anchor_demo::advance_legend_highlight;
use crate::anchor_demo::advance_tile_count_flash;
use crate::anchor_demo::cycle_anchor_selection;
use crate::anchor_demo::cycle_selected_panel;
use crate::anchor_demo::draw_anchor_link;
use crate::anchor_demo::drive_anchor_pose;
use crate::anchor_demo::flash_activation;
use crate::anchor_demo::handle_anchor_count_input;
use crate::anchor_demo::reconcile_anchor_chain;
use crate::anchor_demo::reconcile_fan_attachments;
use crate::anchor_demo::reconcile_panels;
use crate::anchor_demo::toggle_animation_pause;
use crate::anchor_demo::toggle_autofit;
use crate::anchor_demo::toggle_show_anchor_markers;
use crate::constants::ADD_TILE_CONTROL;
use crate::constants::AUTOFIT_CONTROL;
use crate::constants::CAMERA_FOCUS;
use crate::constants::CAMERA_PITCH;
use crate::constants::CAMERA_RADIUS;
use crate::constants::CAMERA_YAW;
use crate::constants::DEPTH_IN_CONTROL;
use crate::constants::DEPTH_OUT_CONTROL;
use crate::constants::HOME_MARGIN;
use crate::constants::PAUSE_CONTROL;
use crate::constants::REMOVE_TILE_CONTROL;
use crate::constants::SHOW_ANCHOR_CONTROL;
use crate::hinge::HingeChain;
use crate::hinge::reconcile_hinge_arrangement;
use crate::info_panel::reconcile_info_panel;
use crate::info_panel::spawn_info_panel;
use crate::menu::reconcile_menu;
use crate::menu::spawn_capability_menu;
use crate::presentation::AnchorPanelMaterials;
use crate::presentation::panel_material;
use crate::presentation::text_material;
use crate::scene::ActiveCapability;
use crate::scene::ModeMorph;
use crate::scene::advance_mode_morph;
use crate::scene::autofit_to_panels;
use crate::scene::handle_capability_input;
use crate::scene::spawn_scene;

fn main() { build_panel_anchoring_app().run(); }

fn build_panel_anchoring_app() -> fairy_dust::SprinkleBuilder<fairy_dust::WithOrbitCam> {
    let app = fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_orbit_cam_preset_pose(
            OrbitCamPose {
                focus:  CAMERA_FOCUS,
                yaw:    CAMERA_YAW,
                pitch:  CAMERA_PITCH,
                radius: CAMERA_RADIUS,
            },
            OrbitCamPreset::blender_like(),
        )
        .with_stable_transparency()
        .with_camera_home()
        .margin(HOME_MARGIN)
        .with_title_bar(panel_anchoring_title_bar());
    let app = configure_panel_anchoring_inputs(app).with_camera_control_panel();
    configure_panel_anchoring_systems(app)
}

fn panel_anchoring_title_bar() -> TitleBar {
    TitleBar::new()
        .with_title("Panel Anchoring")
        .control(TitleBarControl::segmented(
            "P / Space",
            [TitleBarSegment::new(PAUSE_CONTROL, "Pause")],
        ))
        .control(TitleBarControl::segmented(
            "O",
            [TitleBarSegment::new(SHOW_ANCHOR_CONTROL, "Show Anchor")],
        ))
        .control(TitleBarControl::segmented(
            "Tiles",
            [
                TitleBarSegment::new(ADD_TILE_CONTROL, "+"),
                TitleBarSegment::new(REMOVE_TILE_CONTROL, "-"),
            ],
        ))
        .control(TitleBarControl::segmented(
            "Depth",
            [
                TitleBarSegment::new(DEPTH_OUT_CONTROL, "["),
                TitleBarSegment::new(DEPTH_IN_CONTROL, "]"),
            ],
        ))
        .control(TitleBarControl::segmented(
            "I",
            [TitleBarSegment::new(AUTOFIT_CONTROL, "Autofit")],
        ))
}

fn configure_panel_anchoring_inputs(
    app: fairy_dust::TitleBarBuilder<fairy_dust::WithOrbitCam>,
) -> fairy_dust::TitleBarBuilder<fairy_dust::WithOrbitCam> {
    app.wire_chip_to_activation::<Spin>(PAUSE_CONTROL)
        .wire_chip_to_activation::<HingeChain>(PAUSE_CONTROL)
        .wire_chip_to_activation::<ShowAnchorMarkers>(SHOW_ANCHOR_CONTROL)
        .wire_chip_to_activation::<Autofit>(AUTOFIT_CONTROL)
        .wire_chip_to_state::<TileCountFlash, _>(ADD_TILE_CONTROL, |flash| {
            flash_activation(flash.plus)
        })
        .wire_chip_to_state::<TileCountFlash, _>(REMOVE_TILE_CONTROL, |flash| {
            flash_activation(flash.minus)
        })
        .wire_chip_to_state::<LegendHighlight, _>(DEPTH_OUT_CONTROL, |highlight| {
            flash_activation(highlight.glow().depth_out)
        })
        .wire_chip_to_state::<LegendHighlight, _>(DEPTH_IN_CONTROL, |highlight| {
            flash_activation(highlight.glow().depth_in)
        })
}

fn configure_panel_anchoring_systems(
    mut builder: fairy_dust::SprinkleBuilder<fairy_dust::WithOrbitCam>,
) -> fairy_dust::SprinkleBuilder<fairy_dust::WithOrbitCam> {
    builder
        .app_mut()
        .init_resource::<AnchorSelection>()
        .init_resource::<AnchorChain>()
        .init_resource::<SelectedPanel>()
        .init_resource::<ActiveCapability>()
        .init_resource::<Spin>()
        .init_resource::<HingeChain>()
        .init_resource::<ModeMorph>()
        .init_resource::<AnchorTransition>()
        .init_resource::<LegendHighlight>()
        .init_resource::<ShowAnchorMarkers>()
        .init_resource::<Autofit>()
        .init_resource::<TileCountFlash>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                cycle_selected_panel,
                cycle_anchor_selection.after(cycle_selected_panel),
                handle_capability_input,
                handle_anchor_count_input,
                toggle_animation_pause,
                toggle_show_anchor_markers,
                toggle_autofit,
                advance_tile_count_flash,
                advance_legend_highlight,
                // Owns every tile pose while a mode switch eases between layouts.
                advance_mode_morph.after(handle_capability_input),
                advance_animations
                    .after(cycle_anchor_selection)
                    .after(handle_capability_input)
                    .after(toggle_animation_pause)
                    .after(advance_mode_morph),
                reconcile_menu.after(handle_capability_input),
                reconcile_info_panel
                    .after(cycle_selected_panel)
                    .after(cycle_anchor_selection)
                    .after(advance_legend_highlight)
                    .after(handle_capability_input)
                    .before(PanelSystems::ApplyTreeChanges),
                reconcile_panels
                    .after(advance_animations)
                    .after(handle_capability_input)
                    .before(PanelSystems::ApplyTreeChanges),
                // Grows/shrinks the tile chain after `reconcile_panels` has
                // repainted the surviving tiles, so a shrink never paints a tile
                // this system then despawns.
                reconcile_anchor_chain
                    .after(handle_capability_input)
                    .after(handle_anchor_count_input)
                    .after(advance_mode_morph)
                    .after(reconcile_panels),
                // Commits capability `3` through the arrangement API after the
                // shared tile count and mode-morph ownership are settled.
                reconcile_hinge_arrangement
                    .after(advance_animations)
                    .after(advance_mode_morph)
                    .after(reconcile_anchor_chain),
                reconcile_fan_attachments
                    .after(reconcile_hinge_arrangement)
                    .after(cycle_anchor_selection),
                // Runs after the chain reconciles so a just-spawned tile is in
                // the panel union this frame; the home cube it reads is updated
                // by fairy_dust's camera-home systems (at most one frame stale).
                autofit_to_panels.after(reconcile_anchor_chain),
            ),
        )
        .add_systems(
            PostUpdate,
            drive_anchor_pose.in_set(PanelSystems::AnimateAnchorPose),
        )
        .add_systems(
            PostUpdate,
            draw_anchor_link.after(TransformSystems::Propagate),
        );
    builder
}

fn setup(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    selection: Res<AnchorSelection>,
    chain: Res<AnchorChain>,
    selected: Res<SelectedPanel>,
    active: Res<ActiveCapability>,
    show: Res<ShowAnchorMarkers>,
) {
    let panel_materials = AnchorPanelMaterials {
        panel:  materials.add(panel_material()),
        text:   materials.add(text_material()),
        screen: materials.add(screen_panel_material()),
    };
    commands.insert_resource(panel_materials.clone());
    spawn_scene(
        &mut commands,
        *selection,
        chain.count(),
        show.0,
        &panel_materials,
    );
    spawn_info_panel(
        &mut commands,
        *selection,
        *selected,
        active.index,
        &panel_materials,
    );
    spawn_capability_menu(&mut commands, *active, &panel_materials);
}
