//! Capability: a singleton screen-space `hana_diegetic` camera control panel
//! that follows the currently selected lagrange camera.
//!
//! Single-camera examples and multi-camera examples both produce exactly one
//! panel at the bottom-right; its contents swap to reflect the routed
//! `OrbitCam`, or the currently-selected `FreeCam` when Fairy Dust switches
//! camera kind. Cameras may optionally carry a `Name` component — when present,
//! it drives the `CAMERA: <name>` title.

mod constants;
mod display;
mod free_settings;
mod guidance;
mod layout;
mod preset_switch;
mod snapshot;

use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Name;
use bevy::prelude::ObserverSystemExt;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::any_with_component;
use bevy::prelude::error;
use bevy::prelude::warn;
use display::CameraGuidanceDisplay;
use display::CameraGuidanceDisplayState;
use display::RenderState;
pub use guidance::CameraGuidance;
pub use guidance::CameraGuidanceAction;
pub use guidance::CameraGuidanceRow;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::Fit;
use hana_diegetic::FontRegistry;
use hana_diegetic::PanelChanged;
use hana_lagrange::CameraHomed;
use hana_lagrange::CameraSlowModeState;
use hana_lagrange::FreeCam;
use hana_lagrange::FreeCamInputMode;
use hana_lagrange::FreeCamInteractionEnded;
use hana_lagrange::FreeCamInteractionSourcesChanged;
use hana_lagrange::FreeCamInteractionSpeedChanged;
use hana_lagrange::FreeCamInteractionStarted;
use hana_lagrange::FreeCamInteractionState;
use hana_lagrange::InteractionSources;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamInteractionEnded;
use hana_lagrange::OrbitCamInteractionSourcesChanged;
use hana_lagrange::OrbitCamInteractionSpeedChanged;
use hana_lagrange::OrbitCamInteractionStarted;
use hana_lagrange::OrbitCamInteractionState;
use hana_lagrange::ResolvedCameraInputRoute;
use layout::build_guidance_tree;
pub(crate) use preset_switch::CameraPresetSwitching;
pub(crate) use preset_switch::CyclePresetEvent;
use snapshot::CameraGuidanceSnapshot;
use snapshot::resolve_guidance_snapshot;

use crate::constants::INNER_BACKGROUND;
use crate::ensure_plugin;
use crate::screen_panels;

/// Singleton marker for the camera control panel. `bound_camera` records
/// which lagrange camera the panel is currently showing — updated each frame
/// from routing and panel camera candidates.
#[derive(Component)]
pub(crate) struct CameraGuidancePanel {
    bound_camera: Option<Entity>,
}

#[derive(Component)]
struct CameraGuidanceRevealPending;

/// Inner background color for the camera control panel. Defaults to the
/// crate's `INNER_BACKGROUND` constant; override via
/// [`SprinkleBuilder::with_camera_control_panel_background_color`].
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct CameraControlPanelBackground(pub Color);

impl Default for CameraControlPanelBackground {
    fn default() -> Self { Self(INNER_BACKGROUND) }
}

pub(crate) fn install(app: &mut App) {
    ensure_panel_plugins(app);
    free_settings::install(app);
    preset_switch::install(app);
    app.init_resource::<CameraControlPanelBackground>();
    app.add_systems(Startup, spawn_panel);
    app.add_systems(
        PostUpdate,
        (
            rebind_panel_on_route_change,
            track_slow_mode_state,
            track_live_zoom_direction,
            track_live_free_direction,
            tick_highlight_release,
            repaint_panel_display,
        )
            .chain(),
    );
    app.add_observer(refresh_on_interaction_started)
        .add_observer(refresh_on_interaction_ended)
        .add_observer(refresh_on_sources_changed)
        .add_observer(refresh_on_speed_changed)
        .add_observer(refresh_on_free_interaction_started)
        .add_observer(refresh_on_free_interaction_ended)
        .add_observer(refresh_on_free_sources_changed)
        .add_observer(refresh_on_free_speed_changed)
        .add_observer(refresh_on_camera_homed);
}

fn ensure_panel_plugins(app: &mut App) {
    ensure_plugin(app, DiegeticUiPlugin);
    ensure_plugin(app, MeshPickingPlugin);
}

fn spawn_panel(
    mut commands: Commands,
    background: Res<CameraControlPanelBackground>,
    fonts: Res<FontRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let snapshot = default_snapshot();
    let display = CameraGuidanceDisplay::default();
    let unlit = screen_panels::screen_panel_material_handle(&mut materials);
    let panel = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomRight)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_guidance_tree(
            &snapshot,
            display,
            background.0,
            &fonts,
        ))
        .build();

    match panel {
        Ok(panel) => {
            commands
                .spawn((
                    CameraGuidancePanel { bound_camera: None },
                    snapshot,
                    CameraGuidanceDisplayState::from_display(display),
                    panel,
                    Transform::default(),
                ))
                .observe(
                    reveal_panel_after_rebuild
                        .run_if(any_with_component::<CameraGuidanceRevealPending>),
                );
        },
        Err(error) => {
            error!("fairy_dust: failed to build camera control panel: {error}");
        },
    }
}

/// Placeholder snapshot rendered for the one frame before the panel binds to a
/// camera. Uses the default input mode so it never claims an unrelated preset.
fn default_snapshot() -> CameraGuidanceSnapshot {
    resolve_guidance_snapshot(None, None, Some(&OrbitCamInputMode::default()), None, None)
}

/// Rebuilds the panel snapshot when the selected camera changes, or when the
/// currently-bound camera's input-mode components change.
fn rebind_panel_on_route_change(
    mut commands: Commands,
    route: Res<ResolvedCameraInputRoute>,
    cameras: Query<(
        Option<&Name>,
        Option<&CameraGuidance>,
        Option<&OrbitCamInteractionState>,
        Option<&FreeCamInteractionState>,
        Option<&CameraSlowModeState>,
        Option<&OrbitCamInputMode>,
        Option<&FreeCam>,
        Option<&FreeCamInputMode>,
    )>,
    panel_cameras: Query<Entity, Or<(With<OrbitCamInputMode>, With<FreeCamInputMode>)>>,
    changed_cameras: Query<
        Entity,
        Or<(
            Changed<CameraGuidance>,
            Changed<FreeCam>,
            Changed<OrbitCamInputMode>,
            Changed<FreeCamInputMode>,
        )>,
    >,
    panel: Single<(
        Entity,
        &mut CameraGuidancePanel,
        &mut CameraGuidanceDisplayState,
    )>,
    background: Res<CameraControlPanelBackground>,
    fonts: Res<FontRegistry>,
) {
    // Before routing selects a camera, follow the first panel-capable camera so
    // the panel shows a real preset instead of the fabricated default.
    let routed = route.routed_camera().or_else(|| panel_cameras.iter().min());
    let (panel_entity, mut panel_marker, mut display_state) = panel.into_inner();

    let route_changed = panel_marker.bound_camera != routed;
    let data_changed = routed.is_some_and(|cam| changed_cameras.get(cam).is_ok());
    if !route_changed && !data_changed {
        return;
    }

    panel_marker.bound_camera = routed;

    let Some(cam) = routed else {
        return;
    };
    let Ok((name, guidance, orbit_state, free_state, slow_mode, orbit_mode, free_cam, free_mode)) =
        cameras.get(cam)
    else {
        return;
    };

    let snapshot = resolve_guidance_snapshot(name, guidance, orbit_mode, free_cam, free_mode);
    let slow_mode_active = snapshot.slow_mode_binding_label.is_some()
        && slow_mode.is_some_and(|state| state.is_active());
    let display = if free_mode.is_some() {
        CameraGuidanceDisplay::from_free_camera_state(
            free_state.copied().unwrap_or_default(),
            slow_mode_active,
        )
    } else {
        CameraGuidanceDisplay::from_orbit_camera_state(
            orbit_state.copied().unwrap_or_default(),
            slow_mode_active,
        )
    };
    // A same-camera rebuild (the camera is easing home) re-derives interaction
    // rows from camera state but must keep the panel-side home pulse; a camera
    // switch drops it so the new camera starts unlit.
    let home_highlight = (!route_changed).then(|| display_state.home_highlight());
    *display_state = CameraGuidanceDisplayState::from_display(display);
    if let Some((home, home_release)) = home_highlight {
        display_state.restore_home_highlight(home, home_release);
    }
    commands.entity(panel_entity).insert(snapshot.clone());
    if let Err(error) = commands.set_tree(
        panel_entity,
        build_guidance_tree(&snapshot, display_state.display(), background.0, &fonts),
    ) {
        warn!("failed to replace camera control panel tree: {error}");
    }
}

fn refresh_on_interaction_started(
    event: On<OrbitCamInteractionStarted>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    let action = CameraGuidanceAction::from_orbit_interaction(event.kind);
    display.set_sources(action, event.sources);
    // A fresh interaction's speed is unsettled until lagrange reports it, so the
    // singular variant does not flash before a slow-gate chord registers.
    display.set_speed(action, None);
}

fn refresh_on_interaction_ended(
    event: On<OrbitCamInteractionEnded>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    let action = CameraGuidanceAction::from_orbit_interaction(event.kind);
    display.set_sources(action, InteractionSources::NONE);
}

fn refresh_on_sources_changed(
    event: On<OrbitCamInteractionSourcesChanged>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    let action = CameraGuidanceAction::from_orbit_interaction(event.kind);
    display.set_sources(action, event.current);
}

fn track_slow_mode_state(
    cameras: Query<&CameraSlowModeState>,
    panel: Single<(
        &CameraGuidancePanel,
        &CameraGuidanceSnapshot,
        &mut CameraGuidanceDisplayState,
    )>,
) {
    let (panel_marker, snapshot, mut display) = panel.into_inner();
    let Some(camera) = panel_marker.bound_camera else {
        return;
    };
    display.set_slow_mode_active(
        snapshot.slow_mode_binding_label.is_some()
            && cameras
                .get(camera)
                .is_ok_and(|slow_mode| slow_mode.is_active()),
    );
}

/// Mirrors the bound camera's reported zoom direction onto the panel display
/// each frame. lagrange computes and holds the direction in
/// [`OrbitCamInteractionState`], so reversing direction (zoom-in to zoom-out)
/// flips the highlighted row at once without waiting on the source debounce.
fn track_live_zoom_direction(
    cameras: Query<&OrbitCamInteractionState>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    let Some(camera) = panel_marker.bound_camera else {
        return;
    };
    if let Ok(state) = cameras.get(camera) {
        display.set_zoom_direction(state.zoom_direction());
    }
}

/// Mirrors the bound `FreeCam`'s reported move directions onto the panel display
/// each frame, so a decomposed row lights only while its own affordance — the
/// stick, its boost gate, a vertical trigger, or a roll direction — is engaged,
/// instead of every row that shares the `Translate` or `Roll` action.
fn track_live_free_direction(
    cameras: Query<&FreeCamInteractionState>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    let Some(camera) = panel_marker.bound_camera else {
        return;
    };
    if let Ok(state) = cameras.get(camera) {
        display.set_free_directions(state.directions());
    }
}

fn tick_highlight_release(time: Res<Time>, panel: Single<&mut CameraGuidanceDisplayState>) {
    panel.into_inner().tick_highlight_release(time.delta());
}

fn refresh_on_speed_changed(
    event: On<OrbitCamInteractionSpeedChanged>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    let action = CameraGuidanceAction::from_orbit_interaction(event.kind);
    display.set_speed(action, Some(event.speed));
}

fn refresh_on_free_interaction_started(
    event: On<FreeCamInteractionStarted>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    let action = CameraGuidanceAction::from_free_interaction(event.kind);
    display.set_sources(action, event.sources);
    display.set_speed(action, None);
}

fn refresh_on_free_interaction_ended(
    event: On<FreeCamInteractionEnded>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    let action = CameraGuidanceAction::from_free_interaction(event.kind);
    display.set_sources(action, InteractionSources::NONE);
}

fn refresh_on_free_sources_changed(
    event: On<FreeCamInteractionSourcesChanged>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    let action = CameraGuidanceAction::from_free_interaction(event.kind);
    display.set_sources(action, event.current);
}

fn refresh_on_free_speed_changed(
    event: On<FreeCamInteractionSpeedChanged>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    let action = CameraGuidanceAction::from_free_interaction(event.kind);
    display.set_speed(action, Some(event.speed));
}

/// Lights the home row when a camera home is invoked. The row holds through the
/// eased glide via the display's home countdown, then fades.
fn refresh_on_camera_homed(
    event: On<CameraHomed>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if panel_marker.bound_camera != Some(event.camera) {
        return;
    }
    display.pulse_home(event.sources);
}

/// Rebuilds the panel tree when the mirrored display state changes between
/// frames.
fn repaint_panel_display(
    mut commands: Commands,
    panel: Single<(
        Entity,
        &CameraGuidanceSnapshot,
        &mut CameraGuidanceDisplayState,
    )>,
    background: Res<CameraControlPanelBackground>,
    fonts: Res<FontRegistry>,
) {
    let (panel_entity, snapshot, mut display) = panel.into_inner();
    if display.render_state == RenderState::Idle {
        return;
    }
    if let Err(error) = commands.set_tree(
        panel_entity,
        build_guidance_tree(snapshot, display.display(), background.0, &fonts),
    ) {
        warn!("failed to replace camera control panel tree: {error}");
    }
    display.render_state = RenderState::Idle;
}

fn reveal_panel_after_rebuild(event: On<PanelChanged>, mut commands: Commands) {
    commands
        .entity(event.entity)
        .insert(Visibility::Inherited)
        .remove::<CameraGuidanceRevealPending>();
}
