//! Capability: Shift+C cycles the routed camera through `OrbitCam` presets,
//! a `FreeCam` preset, and a hidden camera-control panel state.
//!
//! Declared with `hana_rubric`'s `command!`, modeled on [`crate::restart`], so
//! the keymap owns the chord and the palette lists it.
//! Installed alongside the camera control panel, so every panel example gains
//! the switch. The bound system only acts when the camera is in a built-in
//! preset mode, leaving `Bindings`/`Manual` cameras untouched. The hidden panel
//! state leaves the camera on its current preset; the next Shift+C restores the
//! panel and returns to `SimpleMouse`.

use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::ObserverSystemExt;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use hana_lagrange::CameraControllerHandover;
use hana_lagrange::FreeCamInputMode;
use hana_lagrange::FreeCamLookPitch;
use hana_lagrange::FreeCamPreset;
use hana_lagrange::FreeCamPresetKind;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamHeldForReturn;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::OrbitCamPresetKind;
use hana_lagrange::ResolvedCameraInputRoute;
use hana_rubric::ReflectKeymapCommand;
use hana_rubric::command;

use super::CameraGuidancePanel;
use super::CameraGuidanceRevealPending;
use super::free_settings;
use super::free_settings::FreeCamLookPitchPreference;
use crate::ensure_plugin;
use crate::orbit_cam;

/// Whether the camera control panel's Shift+C preset cycling is wired up.
///
/// Defaults to [`Enabled`](Self::Enabled); the `lock_camera_preset` builder
/// method sets [`Disabled`](Self::Disabled) to pin the camera to one preset.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CameraPresetSwitching {
    /// Shift+C cycles presets.
    #[default]
    Enabled,
    /// Preset switching is suppressed.
    Disabled,
}

#[derive(Clone, Debug, PartialEq)]
enum PresetCycleEntry {
    OrbitPreset(OrbitCamPreset),
    FreePreset(FreeCamPreset),
    HideGuidanceKeepFreeCamera,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CameraCycleMode {
    OrbitPreset(OrbitCamPresetKind),
    FreePreset(FreeCamPresetKind),
}

command! {
    action:      CyclePreset,
    event:       CyclePresetEvent,
    id:          "fairy_dust::cycle_camera_preset",
    title:       "Cycle camera preset",
    description: "Step the active camera to the next input preset in its family.",
}

pub(super) fn install(app: &mut App) {
    ensure_plugin(app, EnhancedInputPlugin);
    app.init_resource::<CameraPresetSwitching>();
    app.add_observer(cycle_preset.run_if(preset_switching_enabled));
}

/// Whether [`lock_camera_preset`](crate::SprinkleBuilder::lock_camera_preset) has
/// pinned the camera to its spawned preset, which suppresses the cycle.
fn preset_switching_enabled(switching: Res<CameraPresetSwitching>) -> bool {
    *switching == CameraPresetSwitching::Enabled
}

fn cycle_preset(
    _: On<CyclePresetEvent>,
    mut commands: Commands,
    route: Res<ResolvedCameraInputRoute>,
    modes: Query<(
        Option<&OrbitCamInputMode>,
        Option<&FreeCamInputMode>,
        Option<&OrbitCam>,
        Option<&OrbitCamHeldForReturn>,
    )>,
    orbit_cameras: Query<Entity, With<OrbitCamInputMode>>,
    free_cameras: Query<Entity, With<FreeCamInputMode>>,
    mut panels: Query<(Entity, &mut CameraGuidancePanel, &mut Visibility)>,
    preference: Option<Res<FreeCamLookPitchPreference>>,
) {
    let Ok((panel_entity, mut panel_marker, mut panel_visibility)) = panels.single_mut() else {
        return;
    };
    let Some(camera) = active_cycle_camera(&route, &panel_marker, &orbit_cameras, &free_cameras)
    else {
        return;
    };
    let Ok((orbit_mode, free_mode, orbit_cam, held_orbit)) = modes.get(camera) else {
        return;
    };
    let Some(mode) = camera_cycle_mode(orbit_mode, free_mode) else {
        return;
    };

    let was_shown = panel_marker.owner_shown;
    match next_cycle_entry(mode, was_shown) {
        PresetCycleEntry::OrbitPreset(preset) => {
            panel_marker.owner_shown = true;
            switch_to_orbit_preset(
                &mut commands,
                camera,
                preset,
                orbit_cam.is_some(),
                held_orbit.is_some(),
            );
            if was_shown {
                commands
                    .entity(panel_entity)
                    .remove::<CameraGuidanceRevealPending>();
            } else {
                commands
                    .entity(panel_entity)
                    .insert(CameraGuidanceRevealPending);
            }
        },
        PresetCycleEntry::FreePreset(preset) => {
            panel_marker.owner_shown = true;
            let look_pitch = preference.as_ref().map_or_else(
                || FreeCamLookPitchPreference::default().look_pitch(),
                |preference| preference.look_pitch(),
            );
            switch_to_free_preset(&mut commands, camera, preset, look_pitch);
            commands
                .entity(panel_entity)
                .remove::<CameraGuidanceRevealPending>();
        },
        PresetCycleEntry::HideGuidanceKeepFreeCamera => {
            panel_marker.owner_shown = false;
            *panel_visibility = Visibility::Hidden;
            commands
                .entity(panel_entity)
                .remove::<CameraGuidanceRevealPending>();
        },
    }
}

fn active_cycle_camera(
    route: &ResolvedCameraInputRoute,
    panel_marker: &CameraGuidancePanel,
    orbit_cameras: &Query<Entity, With<OrbitCamInputMode>>,
    free_cameras: &Query<Entity, With<FreeCamInputMode>>,
) -> Option<Entity> {
    route
        .routed_camera()
        .or(match panel_marker.bound_camera {
            super::BoundCamera::Showing(camera) => Some(camera),
            super::BoundCamera::AwaitingCamera => None,
        })
        .or_else(|| orbit_cameras.iter().min())
        .or_else(|| free_cameras.iter().min())
}

const fn camera_cycle_mode(
    orbit_mode: Option<&OrbitCamInputMode>,
    free_mode: Option<&FreeCamInputMode>,
) -> Option<CameraCycleMode> {
    match (orbit_mode, free_mode) {
        (Some(OrbitCamInputMode::Preset(preset)), _) => {
            Some(CameraCycleMode::OrbitPreset(preset.kind()))
        },
        (_, Some(FreeCamInputMode::Preset(preset))) => {
            Some(CameraCycleMode::FreePreset(preset.kind()))
        },
        _ => None,
    }
}

fn next_cycle_entry(mode: CameraCycleMode, owner_shown: bool) -> PresetCycleEntry {
    if !owner_shown {
        return PresetCycleEntry::OrbitPreset(OrbitCamPreset::simple_mouse());
    }
    match mode {
        CameraCycleMode::OrbitPreset(OrbitCamPresetKind::SimpleMouse) => {
            PresetCycleEntry::OrbitPreset(OrbitCamPreset::blender_like())
        },
        CameraCycleMode::OrbitPreset(OrbitCamPresetKind::BlenderLike) => {
            PresetCycleEntry::FreePreset(FreeCamPreset::keyboard_mouse())
        },
        CameraCycleMode::FreePreset(FreeCamPresetKind::KeyboardMouse) => {
            PresetCycleEntry::HideGuidanceKeepFreeCamera
        },
        CameraCycleMode::FreePreset(_) | CameraCycleMode::OrbitPreset(_) => {
            PresetCycleEntry::OrbitPreset(OrbitCamPreset::simple_mouse())
        },
    }
}

fn switch_to_orbit_preset(
    commands: &mut Commands,
    camera: Entity,
    preset: OrbitCamPreset,
    already_orbiting: bool,
    return_orbit_is_held: bool,
) {
    let mut entity = commands.entity(camera);
    let input_mode = OrbitCamInputMode::with_preset(preset);
    if already_orbiting {
        entity.insert(input_mode);
    } else {
        if !return_orbit_is_held {
            let mut return_orbit = OrbitCam::default();
            orbit_cam::apply_example_orbit_cam_limits(&mut return_orbit);
            entity.insert(OrbitCamHeldForReturn::preserve_for_return(return_orbit));
        }
        entity.switch_free_cam_to_orbit_cam(input_mode);
    }
}

fn switch_to_free_preset(
    commands: &mut Commands,
    camera: Entity,
    preset: FreeCamPreset,
    look_pitch: FreeCamLookPitch,
) {
    let preset = free_settings::preset_with_look_pitch(preset, look_pitch);
    commands
        .entity(camera)
        .switch_orbit_cam_to_free_cam(FreeCamInputMode::with_preset(preset));
}

#[cfg(test)]
mod tests {
    use bevy::camera::RenderTarget;
    use bevy::prelude::Camera;
    use bevy::prelude::InheritedVisibility;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::OrthographicProjection;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::Projection;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::Visibility;
    use bevy::prelude::Window;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowRef;
    use bevy_enhanced_input::prelude::Binding;
    use hana_diegetic::DiegeticPanel;
    use hana_diegetic::Px;
    use hana_lagrange::AnimationEnd;
    use hana_lagrange::AnimationReason;
    use hana_lagrange::AnimationSource;
    use hana_lagrange::CameraBasis;
    use hana_lagrange::CameraEventTiming;
    use hana_lagrange::CameraHomePending;
    use hana_lagrange::CameraInputPhase;
    use hana_lagrange::CameraInputRoutingConfig;
    use hana_lagrange::Focus;
    use hana_lagrange::FreeCam;
    use hana_lagrange::FreeCamHomePose;
    use hana_lagrange::FreeCamInputMode;
    use hana_lagrange::FreeCamLookPitch;
    use hana_lagrange::FreeCamPresetKind;
    use hana_lagrange::InputGain;
    use hana_lagrange::LagrangePlugin;
    use hana_lagrange::LookAngles;
    use hana_lagrange::OrbitAngles;
    use hana_lagrange::OrbitCam;
    use hana_lagrange::OrbitCamBlenderLikePreset;
    use hana_lagrange::OrbitCamHomePose;
    use hana_lagrange::OrbitCamInputGain;
    use hana_lagrange::OrbitCamSimpleMousePreset;
    use hana_lagrange::Position;
    use hana_lagrange::Radius;
    use hana_lagrange::Roll;
    use hana_lagrange::ScalarLimit;
    use hana_lagrange::SequenceDirection;
    use hana_lagrange::SequenceOwner;
    use hana_lagrange::SequencePosition;
    use hana_lagrange::SequenceTime;

    use super::*;
    use crate::Anchor;
    use crate::camera_control_panel::snapshot::HomeHintLocation;
    use crate::camera_home;
    use crate::camera_home::CameraHomeConfig;
    use crate::camera_home::HomeTitleBarControl;
    use crate::constants::HOME_KEY;
    use crate::screen_panels;
    use crate::screen_panels::separation::EdgePanelPlacementRecord;
    use crate::screen_panels::separation::PanelPlacement;

    const TUNED_SENSITIVITY: f32 = 0.5;

    /// Fires the command the keymap fires, then runs the frame the observer's
    /// deferred commands land in.
    fn cycle_preset_once(app: &mut App) {
        app.world_mut().trigger(CyclePresetEvent);
        app.update();
    }

    fn camera_preset_kind(app: &App, camera: Entity) -> Option<OrbitCamPresetKind> {
        match app.world().get::<OrbitCamInputMode>(camera)? {
            OrbitCamInputMode::Preset(preset) => Some(preset.kind()),
            _ => None,
        }
    }

    fn free_camera_preset_kind(app: &App, camera: Entity) -> Option<FreeCamPresetKind> {
        match app.world().get::<FreeCamInputMode>(camera)? {
            FreeCamInputMode::Preset(preset) => Some(preset.kind()),
            _ => None,
        }
    }

    fn free_camera_look_pitch(app: &App, camera: Entity) -> Option<FreeCamLookPitch> {
        match app.world().get::<FreeCamInputMode>(camera)? {
            FreeCamInputMode::Preset(FreeCamPreset::KeyboardMouse(preset)) => {
                Some(preset.look_pitch())
            },
            FreeCamInputMode::Bindings(bindings) => Some(bindings.look_pitch()),
            _ => None,
        }
    }

    fn orbit_home_bindings(app: &App, camera: Entity) -> Result<Vec<Binding>, String> {
        let Some(OrbitCamInputMode::Preset(preset)) = app.world().get::<OrbitCamInputMode>(camera)
        else {
            return Err("expected OrbitCam preset mode".to_string());
        };
        preset
            .to_bindings()
            .map(|bindings| bindings.home().to_vec())
            .map_err(|error| error.to_string())
    }

    fn test_camera_home_config() -> CameraHomeConfig {
        CameraHomeConfig {
            yaw:               0.0,
            pitch:             0.0,
            margin:            0.0,
            anchor:            Anchor::Center,
            offset_px:         Vec2::ZERO,
            title_bar_control: HomeTitleBarControl::Hidden,
        }
    }

    fn tuned_blender_like_preset() -> OrbitCamBlenderLikePreset {
        let disabled = InputGain::DISABLED.0;
        OrbitCamBlenderLikePreset::default()
            .mouse_input_gain(OrbitCamInputGain::uniform(disabled))
            .smooth_scroll_input_gain(OrbitCamInputGain::uniform(disabled))
    }

    #[test]
    fn visible_panel_cycles_simple_to_blender() {
        assert_eq!(
            next_cycle_entry(
                CameraCycleMode::OrbitPreset(OrbitCamPreset::simple_mouse().kind()),
                true
            ),
            PresetCycleEntry::OrbitPreset(OrbitCamPreset::blender_like())
        );
    }

    #[test]
    fn visible_panel_cycles_blender_to_free_cam() {
        assert_eq!(
            next_cycle_entry(
                CameraCycleMode::OrbitPreset(OrbitCamPreset::blender_like().kind()),
                true
            ),
            PresetCycleEntry::FreePreset(FreeCamPreset::keyboard_mouse())
        );
    }

    #[test]
    fn visible_panel_cycles_free_cam_to_off() {
        assert_eq!(
            next_cycle_entry(
                CameraCycleMode::FreePreset(FreeCamPreset::keyboard_mouse().kind()),
                true
            ),
            PresetCycleEntry::HideGuidanceKeepFreeCamera
        );
    }

    #[test]
    fn hidden_panel_cycles_back_to_simple_mouse() {
        assert_eq!(
            next_cycle_entry(
                CameraCycleMode::OrbitPreset(OrbitCamPreset::blender_like().kind()),
                false
            ),
            PresetCycleEntry::OrbitPreset(OrbitCamPreset::simple_mouse())
        );
    }

    #[test]
    fn explicit_cycle_constructs_default_target_preset() {
        let tuned = OrbitCamSimpleMousePreset::default()
            .mouse_input_gain(OrbitCamInputGain::uniform(TUNED_SENSITIVITY));

        assert_eq!(
            next_cycle_entry(
                CameraCycleMode::OrbitPreset(OrbitCamPreset::from(tuned).kind()),
                true
            ),
            PresetCycleEntry::OrbitPreset(OrbitCamPreset::blender_like())
        );
    }

    #[test]
    fn separation_hidden_tuned_blender_like_cycles_to_free_cam() {
        let mut app = crate::test_app();
        app.add_plugins((MinimalPlugins, LagrangePlugin));
        app.finish();
        let tuned = tuned_blender_like_preset();
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                OrbitCamInputMode::with_preset(tuned),
            ))
            .id();
        let panel = app
            .world_mut()
            .spawn((
                CameraGuidancePanel {
                    bound_camera: super::super::BoundCamera::AwaitingCamera,
                    owner_shown:  true,
                    home_hint:    HomeHintLocation::CameraPanel,
                },
                Visibility::Hidden,
                EdgePanelPlacementRecord {
                    placement: PanelPlacement::Hidden,
                },
            ))
            .id();
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        app.add_observer(cycle_preset);
        cycle_preset_once(&mut app);

        assert_eq!(
            free_camera_preset_kind(&app, camera),
            Some(FreeCamPresetKind::KeyboardMouse)
        );
        assert_eq!(
            free_camera_look_pitch(&app, camera),
            Some(FreeCamLookPitch::Inverted)
        );
        assert_eq!(
            app.world().get::<Visibility>(panel),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn camera_off_remains_hidden_when_window_grows() -> Result<(), String> {
        let mut app = crate::test_app();
        app.add_plugins((MinimalPlugins, LagrangePlugin));
        screen_panels::install_overlay_picking(&mut app);
        app.finish();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: (150_u32, 100_u32).into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
            ))
            .id();
        let mut diegetic = DiegeticPanel::screen()
            .size(Px(180.0), Px(90.0))
            .anchor(Anchor::BottomRight)
            .build()
            .map_err(|error| error.to_string())?;
        diegetic.set_width(180.0);
        diegetic.set_height(90.0);
        let panel = app
            .world_mut()
            .spawn((
                CameraGuidancePanel {
                    bound_camera: super::super::BoundCamera::AwaitingCamera,
                    owner_shown:  true,
                    home_hint:    HomeHintLocation::CameraPanel,
                },
                diegetic,
                Visibility::Inherited,
                InheritedVisibility::VISIBLE,
            ))
            .id();
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.add_observer(cycle_preset);
        app.update();
        assert_eq!(
            app.world()
                .get::<EdgePanelPlacementRecord>(panel)
                .map(|record| record.placement),
            Some(PanelPlacement::Hidden)
        );
        cycle_preset_once(&mut app);
        cycle_preset_once(&mut app);
        cycle_preset_once(&mut app);
        assert!(
            !app.world()
                .get::<CameraGuidancePanel>(panel)
                .ok_or("camera panel missing")?
                .owner_shown
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .ok_or("window missing")?
            .resolution
            .set(300.0, 100.0);
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(panel),
            Some(&Visibility::Hidden)
        );
        Ok(())
    }

    /// `lock_camera_preset` pins the camera by suppressing the observer's run
    /// condition, so the test drives the installed path rather than the bare
    /// observer every other test adds.
    #[test]
    fn a_locked_preset_suppresses_the_cycle_the_installed_path_runs() {
        for (switching, expected_preset_kind) in [
            (
                CameraPresetSwitching::Disabled,
                OrbitCamPresetKind::SimpleMouse,
            ),
            (
                CameraPresetSwitching::Enabled,
                OrbitCamPresetKind::BlenderLike,
            ),
        ] {
            let mut app = crate::test_app();
            app.add_plugins((MinimalPlugins, LagrangePlugin));
            install(&mut app);
            app.insert_resource(switching);
            app.finish();
            let camera = app
                .world_mut()
                .spawn((
                    OrbitCam::default(),
                    Camera::default(),
                    RenderTarget::Window(WindowRef::Primary),
                    OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                ))
                .id();
            app.world_mut().spawn((
                CameraGuidancePanel {
                    bound_camera: super::super::BoundCamera::AwaitingCamera,
                    owner_shown:  true,
                    home_hint:    HomeHintLocation::CameraPanel,
                },
                Visibility::Inherited,
            ));
            app.insert_resource(CameraInputRoutingConfig::explicit(camera));
            app.update();

            cycle_preset_once(&mut app);

            assert_eq!(camera_preset_kind(&app, camera), Some(expected_preset_kind));
        }
    }

    #[test]
    fn preset_cycle_advances_from_simple_mouse() {
        let mut app = crate::test_app();
        app.add_plugins((MinimalPlugins, LagrangePlugin));
        app.finish();
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
            ))
            .id();
        app.world_mut().spawn((
            CameraGuidancePanel {
                bound_camera: super::super::BoundCamera::AwaitingCamera,
                owner_shown:  true,
                home_hint:    HomeHintLocation::CameraPanel,
            },
            Visibility::Inherited,
        ));
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        assert_eq!(
            app.world()
                .resource::<ResolvedCameraInputRoute>()
                .routed_camera(),
            Some(camera)
        );
        assert_eq!(
            camera_preset_kind(&app, camera),
            Some(OrbitCamPresetKind::SimpleMouse)
        );

        app.add_observer(cycle_preset);
        cycle_preset_once(&mut app);
        assert_eq!(
            camera_preset_kind(&app, camera),
            Some(OrbitCamPresetKind::BlenderLike)
        );

        cycle_preset_once(&mut app);
        assert_eq!(
            free_camera_preset_kind(&app, camera),
            Some(FreeCamPresetKind::KeyboardMouse)
        );
    }

    #[test]
    fn shift_c_cycle_preserves_camera_view_and_orbit_limits() -> Result<(), &'static str> {
        let mut app = crate::test_app();
        app.add_plugins((MinimalPlugins, LagrangePlugin));
        app.finish();
        let mut orbit_cam = OrbitCam::from_pose(Vec3::new(0.5, 0.25, -0.5), (0.2, -0.1), 4.0);
        orbit_cam.orbit.limit_mut().pitch = ScalarLimit::None;
        *orbit_cam.zoom.limit_mut() = ScalarLimit::Clamp {
            min: 1.0e-9,
            max: f32::INFINITY,
        };
        let camera = app
            .world_mut()
            .spawn((
                orbit_cam,
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
            ))
            .id();
        app.world_mut().spawn((
            CameraGuidancePanel {
                bound_camera: super::super::BoundCamera::AwaitingCamera,
                owner_shown:  true,
                home_hint:    HomeHintLocation::CameraPanel,
            },
            Visibility::Inherited,
        ));
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.add_observer(cycle_preset);
        app.update();

        let initial_transform = *app
            .world()
            .get::<Transform>(camera)
            .ok_or("camera transform missing")?;
        let initial_orbit = *app.world().get::<OrbitCam>(camera).ok_or("orbit missing")?;
        for step in 0..4 {
            cycle_preset_once(&mut app);
            let transform = app
                .world()
                .get::<Transform>(camera)
                .ok_or("camera transform missing after cycle")?;
            assert!((transform.translation - initial_transform.translation).length() < 1.0e-5);
            assert!(transform.rotation.dot(initial_transform.rotation).abs() > 1.0 - 1.0e-5);
            if step == 0 {
                let blender = app.world().get::<OrbitCam>(camera).ok_or("orbit missing")?;
                assert_eq!(blender.orbit.limit(), initial_orbit.orbit.limit());
                assert_eq!(blender.zoom.limit(), initial_orbit.zoom.limit());
            }
        }
        let restored = app.world().get::<OrbitCam>(camera).ok_or("orbit missing")?;
        assert_eq!(restored.orbit.limit(), initial_orbit.orbit.limit());
        assert_eq!(restored.pan.limit(), initial_orbit.pan.limit());
        assert_eq!(restored.zoom.limit(), initial_orbit.zoom.limit());
        assert_eq!(
            restored.upside_down_policy,
            initial_orbit.upside_down_policy
        );
        assert_eq!(
            camera_preset_kind(&app, camera),
            Some(OrbitCamPresetKind::SimpleMouse)
        );
        Ok(())
    }

    #[test]
    fn orbit_controller_preserves_view_after_full_cycle_in_both_projections() -> Result<(), String>
    {
        for projection in [
            Projection::Perspective(PerspectiveProjection::default()),
            Projection::Orthographic(OrthographicProjection::default_3d()),
        ] {
            let mut app = crate::test_app();
            app.add_plugins((MinimalPlugins, LagrangePlugin));
            app.finish();
            let camera = app
                .world_mut()
                .spawn((
                    OrbitCam::from_pose(Vec3::new(0.5, 0.25, -0.5), (0.2, -0.1), 4.0),
                    Camera::default(),
                    projection,
                    RenderTarget::Window(WindowRef::Primary),
                    OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                ))
                .id();
            app.world_mut().spawn((
                CameraGuidancePanel {
                    bound_camera: super::super::BoundCamera::AwaitingCamera,
                    owner_shown:  true,
                    home_hint:    HomeHintLocation::CameraPanel,
                },
                Visibility::Inherited,
            ));
            app.insert_resource(CameraInputRoutingConfig::explicit(camera));
            app.add_observer(cycle_preset);
            app.update();
            let before = *app
                .world()
                .get::<Transform>(camera)
                .ok_or("camera transform missing")?;
            for _ in 0..4 {
                cycle_preset_once(&mut app);
            }
            app.update();
            let after = app
                .world()
                .get::<Transform>(camera)
                .ok_or("camera transform missing")?;
            assert!((after.translation - before.translation).length() < 1.0e-4);
            assert!(after.rotation.dot(before.rotation).abs() > 1.0 - 1.0e-5);
        }
        Ok(())
    }

    #[test]
    fn cycled_orbit_preset_is_filled_when_camera_home_is_enabled() -> Result<(), String> {
        let mut app = crate::test_app();
        app.add_plugins((MinimalPlugins, LagrangePlugin))
            .insert_resource(test_camera_home_config())
            .add_systems(
                PreUpdate,
                camera_home::fill_camera_home_presets.before(CameraInputPhase::PreInput),
            );
        app.finish();
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
            ))
            .id();
        app.world_mut().spawn((
            CameraGuidancePanel {
                bound_camera: super::super::BoundCamera::AwaitingCamera,
                owner_shown:  true,
                home_hint:    HomeHintLocation::CameraPanel,
            },
            Visibility::Inherited,
        ));
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        app.add_observer(cycle_preset);
        cycle_preset_once(&mut app);

        assert_eq!(
            camera_preset_kind(&app, camera),
            Some(OrbitCamPresetKind::BlenderLike)
        );
        assert_eq!(
            orbit_home_bindings(&app, camera)?,
            vec![Binding::from(HOME_KEY)]
        );
        Ok(())
    }

    #[test]
    fn cycling_to_free_cam_keeps_orbit_home_as_fixed_free_home() -> Result<(), String> {
        let mut app = crate::test_app();
        app.add_plugins((MinimalPlugins, LagrangePlugin));
        app.finish();
        let orbit_home = OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   0.25,
                pitch: -0.125,
            },
            pan:   Focus(Vec3::new(1.0, 2.0, 3.0)),
            zoom:  Radius(7.0),
        };
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
                orbit_home,
            ))
            .id();
        app.world_mut().spawn((
            CameraGuidancePanel {
                bound_camera: super::super::BoundCamera::AwaitingCamera,
                owner_shown:  true,
                home_hint:    HomeHintLocation::CameraPanel,
            },
            Visibility::Inherited,
        ));
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        app.add_observer(cycle_preset);
        cycle_preset_once(&mut app);

        let projection = app
            .world()
            .get::<Projection>(camera)
            .ok_or("camera should have a projection")?;
        let expected_home =
            FreeCamHomePose::from_orbit_home(orbit_home, CameraBasis::Y_UP, projection);
        assert_eq!(
            free_camera_preset_kind(&app, camera),
            Some(FreeCamPresetKind::KeyboardMouse)
        );
        assert_eq!(
            app.world().get::<FreeCamHomePose>(camera),
            Some(&expected_home)
        );
        assert!(app.world().get::<CameraHomePending>(camera).is_none());

        let target = app.world_mut().spawn_empty().id();
        {
            let mut free_cam = app
                .world_mut()
                .get_mut::<FreeCam>(camera)
                .ok_or("camera should have switched to FreeCam")?;
            free_cam
                .translate
                .snap_to(Position(Vec3::new(50.0, 60.0, 70.0)));
            free_cam.look.snap_to(LookAngles {
                yaw:   1.5,
                pitch: 0.75,
            });
            free_cam.roll.snap_to(Roll(0.5));
        }
        app.world_mut().trigger(AnimationEnd {
            camera,
            source: AnimationSource::ZoomToFit,
            target: Some(target),
            owner: SequenceOwner::NativePlayback,
            direction: SequenceDirection::Forward,
            timing: CameraEventTiming::new(SequencePosition::START, SequenceTime::ZERO),
            reason: AnimationReason::Completed,
        });
        app.world_mut().flush();

        assert_eq!(
            app.world().get::<FreeCamHomePose>(camera),
            Some(&expected_home)
        );
        Ok(())
    }
}
