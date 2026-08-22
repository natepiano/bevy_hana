//! Per-frame camera input routing: which lagrange camera, if any, owns input this frame.
//!
//! Public surface (re-exported here from submodules):
//! - [`CameraInputRouting`], [`NoPositionFallback`], [`CameraInputRoutingConfig`] — caller-facing
//!   configuration (see [`config`]).
//! - [`ResolvedCameraInputRoute`] — the per-frame result: routed camera, surface metrics, and
//!   per-camera blocker reasons.
//!
//! Internal surface (this file):
//! - [`CameraInputRoutingPlugin`] — registers the resources, the `clear_latches_on_mode_replaced`
//!   observer, and the `resolve_camera_input_routing` system in
//!   [`CameraInputInternalSet::Routing`].
//! - `resolve_camera_input_routing` — the system itself; collects window/camera snapshots, picks
//!   the routed camera, computes blockers, writes the resolved route and the per-camera gate
//!   components.
//! - `select_routed_camera` / `latched_camera` / `cursor_hit_camera` — selection helpers used by
//!   the system.
//!
//! Submodules:
//! - [`config`] — public config types.
//! - [`latches`] — per-source ownership state (`CameraInputSourceLatches`).
//! - [`blockers`] — per-camera gating components (`CameraInputBlockers`,
//!   `CameraInputContextGated`).
//! - [`snapshot`] — per-frame window/camera capture used internally by the system.

mod blockers;
mod config;
mod latches;
mod snapshot;

use std::collections::HashMap;

use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::Resource;
use bevy::prelude::World;
pub(crate) use blockers::CameraInputBlockers;
pub(crate) use blockers::CameraInputContextGated;
use blockers::ContextGate;
pub use config::CameraInputRouting;
pub use config::CameraInputRoutingConfig;
pub use config::NoPositionFallback;
pub(crate) use latches::CameraInputSourceLatches;
pub(crate) use latches::CameraSlowModeLatches;
pub use latches::CameraSlowModeState;
use latches::clear_latches_on_mode_replaced;
use snapshot::CameraRoutingSnapshot;
use snapshot::CameraRoutingSnapshotFlags;
use snapshot::collect_camera_snapshots;
use snapshot::collect_window_snapshots;

use super::CameraInputSurfaceMetrics;
use crate::system_sets::CameraInputInternalSet;

pub(super) fn is_slow_mode_active(slow_latches: &CameraSlowModeLatches, camera: Entity) -> bool {
    slow_latches.is_active(camera)
}

pub(super) fn toggle_slow_mode_latch(slow_latches: &mut CameraSlowModeLatches, camera: Entity) {
    slow_latches.toggle(camera);
}

/// Result of the per-frame input routing pass: which lagrange camera, if any,
/// currently owns input, plus the cursor-surface metrics and blocker reasons
/// for every candidate camera.
///
/// Read this as a resource to discover the active camera — useful for
/// multi-camera HUDs, per-camera home-key bindings, and any feature that
/// needs to react to "which viewport is the cursor over."
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct ResolvedCameraInputRoute {
    routed_camera: Option<Entity>,
    metrics:       HashMap<Entity, CameraInputSurfaceMetrics>,
    blockers:      HashMap<Entity, CameraInputBlockers>,
}

impl ResolvedCameraInputRoute {
    /// The lagrange camera entity currently receiving input, if any. Returns
    /// `None` when no eligible camera viewport owns input.
    #[must_use]
    pub const fn routed_camera(&self) -> Option<Entity> { self.routed_camera }

    pub(crate) fn metrics_for(&self, camera: Entity) -> Option<CameraInputSurfaceMetrics> {
        self.metrics.get(&camera).copied()
    }

    pub(crate) fn blockers_for(&self, camera: Entity) -> Option<CameraInputBlockers> {
        self.blockers.get(&camera).copied()
    }
}

pub(crate) struct CameraInputRoutingPlugin;

impl Plugin for CameraInputRoutingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraInputRoutingConfig>()
            .init_resource::<CameraInputSourceLatches>()
            .init_resource::<CameraSlowModeLatches>()
            .init_resource::<ResolvedCameraInputRoute>()
            .add_observer(clear_latches_on_mode_replaced)
            .add_systems(
                PreUpdate,
                resolve_camera_input_routing.in_set(CameraInputInternalSet::Routing),
            );
    }
}

fn resolve_camera_input_routing(world: &mut World) {
    let camera_input_routing_config = *world.resource::<CameraInputRoutingConfig>();
    let windows = collect_window_snapshots(world);
    let snapshots = collect_camera_snapshots(world, &windows);
    let available_cameras = snapshots
        .iter()
        .filter(|snapshot| {
            snapshot.has(CameraRoutingSnapshotFlags::ACTIVE)
                && !snapshot.has(CameraRoutingSnapshotFlags::DISABLED)
        })
        .map(|snapshot| snapshot.entity)
        .collect::<Vec<_>>();

    let latches = {
        let mut latches = world.resource_mut::<CameraInputSourceLatches>();
        latches.recover_unavailable_latches(&available_cameras);
        latches.clone()
    };
    let slow_latches = {
        let mut slow_latches = world.resource_mut::<CameraSlowModeLatches>();
        slow_latches.recover_unavailable_latches(&available_cameras);
        slow_latches.clone()
    };

    let routed_camera = select_routed_camera(
        &camera_input_routing_config,
        &snapshots,
        &available_cameras,
        &latches,
    );
    let mut resolved = ResolvedCameraInputRoute {
        routed_camera,
        metrics: HashMap::new(),
        blockers: HashMap::new(),
    };

    for snapshot in snapshots {
        let blockers = CameraInputBlockers::from_snapshot(&snapshot, routed_camera);

        world.entity_mut(snapshot.entity).insert((
            blockers,
            CameraInputContextGated {
                context_gate: ContextGate::from(!blockers.is_blocked()),
            },
            CameraSlowModeState::from_active(slow_latches.is_active(snapshot.entity)),
        ));
        resolved.metrics.insert(snapshot.entity, snapshot.metrics);
        resolved.blockers.insert(snapshot.entity, blockers);
    }

    world.insert_resource(resolved);
}

fn select_routed_camera(
    config: &CameraInputRoutingConfig,
    snapshots: &[CameraRoutingSnapshot],
    available_cameras: &[Entity],
    latches: &CameraInputSourceLatches,
) -> Option<Entity> {
    match config.mode {
        CameraInputRouting::Explicit => config
            .explicit_camera
            .filter(|camera| available_cameras.contains(camera)),
        CameraInputRouting::CursorHitTest => latched_camera(latches, available_cameras)
            .or_else(|| cursor_hit_camera(snapshots))
            .or_else(|| {
                if config.no_position_fallback == NoPositionFallback::OnlyEligibleCamera
                    && available_cameras.len() == 1
                {
                    available_cameras.first().copied()
                } else {
                    None
                }
            }),
    }
}

fn latched_camera(
    latches: &CameraInputSourceLatches,
    available_cameras: &[Entity],
) -> Option<Entity> {
    let mut camera = None;
    for latch in [latches.mouse_latch(), latches.keyboard_latch()]
        .into_iter()
        .flatten()
    {
        let latched_camera = latch.camera();
        if !available_cameras.contains(&latched_camera) {
            continue;
        }
        match camera {
            Some(existing) if existing != latched_camera => return None,
            Some(_) => {},
            None => camera = Some(latched_camera),
        }
    }
    camera
}

fn cursor_hit_camera(snapshots: &[CameraRoutingSnapshot]) -> Option<Entity> {
    snapshots
        .iter()
        .filter(|snapshot| {
            snapshot.has(CameraRoutingSnapshotFlags::ACTIVE)
                && !snapshot.has(CameraRoutingSnapshotFlags::DISABLED)
                && snapshot.has(CameraRoutingSnapshotFlags::CURSOR_HIT)
        })
        .max_by_key(|snapshot| snapshot.order)
        .map(|snapshot| snapshot.entity)
}

#[cfg(test)]
mod tests {
    use bevy::camera::RenderTarget;
    use bevy::camera::RenderTargetInfo;
    use bevy::prelude::Bundle;
    use bevy::prelude::Camera;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::MouseButton;
    use bevy::prelude::UVec2;
    use bevy::prelude::Vec2;
    use bevy::prelude::Window;
    use bevy::prelude::default;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowRef;
    use bevy_enhanced_input::prelude::ModKeys;

    use super::blockers::CameraInputBlockerBits;
    use super::*;
    use crate::FreeCam;
    use crate::FreeCamKind;
    use crate::OrbitCam;
    use crate::OrbitCamKind;
    use crate::input::BindingsError;
    use crate::input::CUSTOM_SLOW_SCALE;
    use crate::input::CameraInputDisabled;
    use crate::input::CameraInputModeReplaced;
    use crate::input::CameraInputScalePolicy;
    use crate::input::CameraResolvedBindings;
    use crate::input::CameraSlowMode;
    use crate::input::FreeCamBindings;
    use crate::input::FreeCamInputMode;
    use crate::input::FreeCamMouseLook;
    use crate::input::FreeCamRollBinding;
    use crate::input::FreeCamTranslateKeys;
    use crate::input::InputBinding;
    use crate::input::InputGain;
    use crate::input::InteractionSources;
    use crate::input::OrbitCamBindings;
    use crate::input::OrbitCamInputMode;
    use crate::input::OrbitCamMouseDrag;
    use crate::input::OrbitCamPreset;
    use crate::system_sets::LagrangeSystemSetsPlugin;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            LagrangeSystemSetsPlugin,
            CameraInputRoutingPlugin,
        ));
        app
    }

    fn spawn_camera(world: &mut World, components: impl Bundle) -> Entity {
        world
            .spawn((
                OrbitCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                components,
            ))
            .id()
    }

    fn spawn_non_orbit_camera(world: &mut World, order: isize) -> Entity {
        world
            .spawn((
                Camera { order, ..default() },
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id()
    }

    fn spawn_free_camera(world: &mut World, bindings: FreeCamBindings) -> Entity {
        world
            .spawn((
                FreeCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                FreeCamInputMode::Bindings(bindings.clone()),
                CameraResolvedBindings::<FreeCamKind>(bindings),
            ))
            .id()
    }

    fn disabled_free_bindings_with_slow_mode() -> Result<FreeCamBindings, BindingsError> {
        let disabled = InputGain::DISABLED.0;
        FreeCamBindings::builder()
            .slow_mode(CameraSlowMode {
                toggle_key: KeyCode::KeyS,
                mod_keys:   ModKeys::ALT,
                scale:      CameraInputScalePolicy {
                    normal: InputGain::DEFAULT.0,
                    slow:   CUSTOM_SLOW_SCALE,
                },
            })
            .translate(FreeCamTranslateKeys::default().with_input_gain(disabled))
            .look(FreeCamMouseLook::button(MouseButton::Right).with_input_gain(disabled))
            .roll(
                FreeCamRollBinding::from(InputBinding::bidirectional_keys(
                    KeyCode::KeyQ,
                    KeyCode::KeyE,
                ))
                .with_input_gain(disabled),
            )
            .build()
    }

    fn test_window(focused: bool) -> Window {
        let mut window = Window {
            focused,
            ..default()
        };
        window.set_cursor_position(Some(Vec2::new(100.0, 100.0)));
        window
    }

    fn test_camera(target: RenderTarget) -> (OrbitCam, Camera, RenderTarget, OrbitCamInputMode) {
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(1280, 720),
            scale_factor:  1.0,
        });
        (
            OrbitCam::default(),
            camera,
            target,
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        )
    }

    #[test]
    fn explicit_routing_config_applies_in_preinput() {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));

        app.update();

        assert_eq!(
            app.world()
                .resource::<ResolvedCameraInputRoute>()
                .routed_camera,
            Some(camera)
        );
    }

    #[test]
    fn mode_replacement_clears_matching_latches() {
        let mut app = test_app();
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Manual);
        app.world_mut()
            .resource_mut::<CameraInputSourceLatches>()
            .acquire_sources(camera, InteractionSources::MOUSE);
        app.world_mut()
            .entity_mut(camera)
            .trigger(|camera| CameraInputModeReplaced { camera });

        assert!(
            app.world()
                .resource::<CameraInputSourceLatches>()
                .mouse_latch()
                .is_none()
        );
    }

    #[test]
    fn mode_replacement_clears_slow_latch_without_slow_mode() {
        let mut app = test_app();
        let camera = spawn_camera(app.world_mut(), OrbitCamInputMode::Manual);
        app.world_mut()
            .resource_mut::<CameraSlowModeLatches>()
            .toggle(camera);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|camera| CameraInputModeReplaced { camera });

        assert!(
            !app.world()
                .resource::<CameraSlowModeLatches>()
                .is_active(camera)
        );
    }

    #[test]
    fn mode_replacement_clears_slow_latch_without_effective_slow_controls()
    -> Result<(), BindingsError> {
        let bindings = OrbitCamBindings::builder()
            .slow_mode(CameraSlowMode {
                toggle_key: KeyCode::KeyS,
                mod_keys:   ModKeys::ALT,
                scale:      CameraInputScalePolicy {
                    normal: InputGain::DEFAULT.0,
                    slow:   CUSTOM_SLOW_SCALE,
                },
            })
            .orbit(
                OrbitCamMouseDrag::new(MouseButton::Middle).with_input_gain(InputGain::DISABLED.0),
            )
            .build()?;
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::Bindings(bindings.clone()),
        );
        app.world_mut()
            .entity_mut(camera)
            .insert(CameraResolvedBindings::<OrbitCamKind>(bindings));
        app.world_mut()
            .resource_mut::<CameraSlowModeLatches>()
            .toggle(camera);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|camera| CameraInputModeReplaced { camera });

        assert!(
            !app.world()
                .resource::<CameraSlowModeLatches>()
                .is_active(camera)
        );
        Ok(())
    }

    #[test]
    fn mode_replacement_clears_free_slow_latch_without_effective_slow_controls()
    -> Result<(), BindingsError> {
        let bindings = disabled_free_bindings_with_slow_mode()?;
        let mut app = test_app();
        let camera = spawn_free_camera(app.world_mut(), bindings);
        app.world_mut()
            .resource_mut::<CameraSlowModeLatches>()
            .toggle(camera);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|camera| CameraInputModeReplaced { camera });

        assert!(
            !app.world()
                .resource::<CameraSlowModeLatches>()
                .is_active(camera)
        );
        Ok(())
    }

    #[test]
    fn stale_slow_latches_are_recovered_during_routing() {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        app.world_mut()
            .resource_mut::<CameraSlowModeLatches>()
            .toggle(camera);
        let _ = app.world_mut().despawn(camera);

        app.update();

        assert!(
            !app.world()
                .resource::<CameraSlowModeLatches>()
                .is_active(camera)
        );
    }

    #[test]
    fn stale_latches_are_recovered_during_routing() {
        let mut app = test_app();
        let stale_camera = Entity::PLACEHOLDER;
        app.world_mut()
            .resource_mut::<CameraInputSourceLatches>()
            .acquire_sources(stale_camera, InteractionSources::KEYBOARD);

        app.update();

        assert!(
            app.world()
                .resource::<CameraInputSourceLatches>()
                .keyboard_latch()
                .is_none()
        );
    }

    #[test]
    fn source_latch_routes_without_cursor_hit() {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        app.world_mut()
            .resource_mut::<CameraInputSourceLatches>()
            .acquire_sources(camera, InteractionSources::KEYBOARD);

        app.update();

        assert_eq!(
            app.world()
                .resource::<ResolvedCameraInputRoute>()
                .routed_camera,
            Some(camera)
        );
    }

    #[test]
    fn focused_window_cursor_selects_camera_when_window_cursors_overlap() {
        let mut app = test_app();
        let primary_window = app
            .world_mut()
            .spawn((test_window(true), PrimaryWindow))
            .id();
        let second_window = app.world_mut().spawn(test_window(false)).id();
        let primary_camera = app
            .world_mut()
            .spawn(test_camera(RenderTarget::Window(WindowRef::Primary)))
            .id();
        let second_camera = app
            .world_mut()
            .spawn(test_camera(RenderTarget::Window(WindowRef::Entity(
                second_window,
            ))))
            .id();

        app.update();

        assert_eq!(
            app.world()
                .resource::<ResolvedCameraInputRoute>()
                .routed_camera,
            Some(primary_camera)
        );

        if let Some(mut window) = app.world_mut().get_mut::<Window>(primary_window) {
            window.focused = false;
        }
        if let Some(mut window) = app.world_mut().get_mut::<Window>(second_window) {
            window.focused = true;
        }

        app.update();

        assert_eq!(
            app.world()
                .resource::<ResolvedCameraInputRoute>()
                .routed_camera,
            Some(second_camera)
        );
    }

    #[test]
    fn non_orbit_cameras_do_not_count_as_routing_targets() {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        let overlay = spawn_non_orbit_camera(app.world_mut(), 1);
        app.insert_resource(
            CameraInputRoutingConfig::cursor_hit_test()
                .with_no_position_fallback(NoPositionFallback::OnlyEligibleCamera),
        );

        app.update();

        let resolved = app.world().resource::<ResolvedCameraInputRoute>();
        assert_eq!(resolved.routed_camera, Some(camera));
        assert!(!resolved.metrics.contains_key(&overlay));
        assert!(
            app.world()
                .get::<CameraInputContextGated>(overlay)
                .is_none()
        );
    }

    #[test]
    fn metrics_are_recorded_for_non_routed_manual_cameras() {
        let mut app = test_app();
        let routed = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        let manual = spawn_camera(app.world_mut(), OrbitCamInputMode::Manual);
        app.insert_resource(CameraInputRoutingConfig::explicit(routed));

        app.update();

        let resolved = app.world().resource::<ResolvedCameraInputRoute>();
        assert!(resolved.metrics.contains_key(&manual));
        assert_eq!(resolved.routed_camera, Some(routed));
    }

    #[test]
    fn explicit_surface_metrics_are_recorded_in_route_resource() {
        let mut app = test_app();
        let metrics = CameraInputSurfaceMetrics::camera_view_and_input_surface(
            Vec2::new(120.0, 80.0),
            Vec2::new(240.0, 160.0),
        );
        let camera = spawn_camera(
            app.world_mut(),
            (
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                metrics,
            ),
        );
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));

        app.update();

        assert_eq!(
            app.world()
                .resource::<ResolvedCameraInputRoute>()
                .metrics_for(camera),
            Some(metrics)
        );
    }

    #[test]
    fn disabled_camera_sets_blocker_and_gates_context() {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            (
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                CameraInputDisabled,
            ),
        );
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));

        app.update();

        assert_eq!(
            app.world()
                .resource::<ResolvedCameraInputRoute>()
                .blockers
                .get(&camera)
                .map(|blockers| blockers.bits.contains(CameraInputBlockerBits::DISABLED)),
            Some(true)
        );
        assert_eq!(
            app.world()
                .resource::<ResolvedCameraInputRoute>()
                .blockers
                .get(&camera)
                .copied()
                .map(CameraInputBlockers::is_blocked),
            Some(true)
        );
        assert_eq!(
            app.world()
                .get::<CameraInputContextGated>(camera)
                .map(|gated| gated.context_gate),
            Some(ContextGate::Blocked)
        );
    }
}
