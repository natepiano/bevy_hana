//! Capability: a singleton screen-space `hana_diegetic` camera control panel
//! that follows the currently selected lagrange camera.
//!
//! Single-camera examples and multi-camera examples both produce exactly one
//! panel at the bottom-right; its contents swap to reflect the routed
//! `OrbitCam`, or the currently-selected `FreeCam` when Fairy Dust switches
//! camera kind. Cameras may optionally carry a `Name` component — when present,
//! it drives the camera-name title.

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
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::any_with_component;
use bevy::prelude::error;
use bevy::prelude::warn;
use constants::CAMERA_PANEL_BACKGROUND;
use display::CameraGuidanceDisplay;
use display::CameraGuidanceDisplayState;
use display::RenderState;
pub use guidance::CameraGuidance;
pub use guidance::CameraGuidanceAction;
pub use guidance::CameraGuidanceRow;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticTextMeasurer;
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
use hana_lagrange::SlowModeHint;
use layout::build_guidance_tree;
pub(crate) use preset_switch::CameraPresetSwitching;
pub(crate) use preset_switch::CyclePresetEvent;
use snapshot::CameraGuidanceSnapshot;
pub(crate) use snapshot::HomeHintLocation;
use snapshot::resolve_guidance_snapshot;

use crate::ensure_plugin;
use crate::screen_panels;
use crate::screen_panels::TitleBar;
use crate::screen_panels::TitleBarOverflow;
use crate::screen_panels::TitleBarOverflowSystems;
use crate::screen_panels::separation::EdgePanelPlacementRecord;
use crate::screen_panels::separation::EdgePanelPrecedence;
use crate::screen_panels::separation::PanelPlacement;

/// Singleton marker for the camera control panel. `bound_camera` records
/// which lagrange camera the panel is currently showing — updated each frame
/// from routing and panel camera candidates.
#[derive(Component)]
pub(crate) struct CameraGuidancePanel {
    bound_camera: BoundCamera,
    owner_shown:  bool,
    home_hint:    HomeHintLocation,
}

pub(crate) fn home_hint_location(
    title_bars: &Query<(&TitleBar, &TitleBarOverflow, Option<&Visibility>)>,
) -> HomeHintLocation {
    if title_bars.iter().any(|(bar, overflow, visibility)| {
        bar.shows_home_control()
            && !overflow.contains("H Home")
            && !matches!(visibility, Some(Visibility::Hidden))
    }) {
        HomeHintLocation::TitleBar
    } else {
        HomeHintLocation::CameraPanel
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundCamera {
    Showing(Entity),
    AwaitingCamera,
}

impl BoundCamera {
    fn shows(self, camera: Entity) -> bool {
        matches!(self, Self::Showing(bound) if bound == camera)
    }
}

#[derive(Component)]
struct CameraGuidanceRevealPending;

/// Inner background color for the camera control panel. Defaults to the
/// camera-specific `CAMERA_PANEL_BACKGROUND` constant; override via
/// [`SprinkleBuilder::with_camera_control_panel_background_color`].
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct CameraControlPanelBackground(pub Color);

impl Default for CameraControlPanelBackground {
    fn default() -> Self { Self(CAMERA_PANEL_BACKGROUND) }
}

pub(crate) fn install(app: &mut App) {
    ensure_panel_plugins(app);
    free_settings::install(app);
    preset_switch::install(app);
    app.init_resource::<CameraControlPanelBackground>();
    app.add_systems(Startup, spawn_panel);
    app.add_systems(
        Update,
        (
            rebind_panel_on_route_change,
            track_slow_mode_state,
            track_live_zoom_direction,
            track_live_free_direction,
            tick_highlight_release,
            repaint_panel_display,
        )
            .chain()
            .in_set(TitleBarOverflowSystems::Read),
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
    measurer: Res<DiegeticTextMeasurer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    title_bars: Query<(&TitleBar, &TitleBarOverflow, Option<&Visibility>)>,
) {
    let home_hint = home_hint_location(&title_bars);
    let snapshot = default_snapshot(home_hint);
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
            &measurer,
        ))
        .build();

    match panel {
        Ok(panel) => {
            commands
                .spawn((
                    EdgePanelPrecedence::CameraControl,
                    CameraGuidancePanel {
                        bound_camera: BoundCamera::AwaitingCamera,
                        owner_shown: true,
                        home_hint,
                    },
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
fn default_snapshot(home_hint: HomeHintLocation) -> CameraGuidanceSnapshot {
    resolve_guidance_snapshot(
        None,
        None,
        Some(&OrbitCamInputMode::default()),
        None,
        None,
        home_hint,
    )
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
    measurer: Res<DiegeticTextMeasurer>,
    title_bars: Query<(&TitleBar, &TitleBarOverflow, Option<&Visibility>)>,
) {
    // Before routing selects a camera, follow the first panel-capable camera so
    // the panel shows a real preset instead of the fabricated default.
    let routed = route
        .routed_camera()
        .filter(|&camera| cameras.contains(camera))
        .or_else(|| panel_cameras.iter().min());
    let bound = routed.map_or(BoundCamera::AwaitingCamera, BoundCamera::Showing);
    let (panel_entity, mut panel_marker, mut display_state) = panel.into_inner();

    let route_changed = panel_marker.bound_camera != bound;
    let data_changed = routed.is_some_and(|cam| changed_cameras.get(cam).is_ok());
    let home_hint = home_hint_location(&title_bars);
    let hint_changed = panel_marker.home_hint != home_hint;
    if !route_changed && !data_changed && !hint_changed {
        return;
    }

    panel_marker.bound_camera = bound;
    panel_marker.home_hint = home_hint;

    let BoundCamera::Showing(cam) = bound else {
        let snapshot = default_snapshot(home_hint);
        *display_state = CameraGuidanceDisplayState::default();
        commands.entity(panel_entity).insert(snapshot.clone());
        if let Err(error) = commands.set_tree(
            panel_entity,
            build_guidance_tree(
                &snapshot,
                display_state.display(),
                background.0,
                &fonts,
                &measurer,
            ),
        ) {
            warn!("failed to reset camera control panel tree: {error}");
        }
        return;
    };
    let Ok((name, guidance, orbit_state, free_state, slow_mode, orbit_mode, free_cam, free_mode)) =
        cameras.get(cam)
    else {
        return;
    };

    let snapshot =
        resolve_guidance_snapshot(name, guidance, orbit_mode, free_cam, free_mode, home_hint);
    let slow_mode_active = matches!(snapshot.slow_mode, SlowModeHint::ToggledBy(_))
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
        build_guidance_tree(
            &snapshot,
            display_state.display(),
            background.0,
            &fonts,
            &measurer,
        ),
    ) {
        warn!("failed to replace camera control panel tree: {error}");
    }
}

fn refresh_on_interaction_started(
    event: On<OrbitCamInteractionStarted>,
    panel: Single<(&CameraGuidancePanel, &mut CameraGuidanceDisplayState)>,
) {
    let (panel_marker, mut display) = panel.into_inner();
    if !panel_marker.bound_camera.shows(event.camera) {
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
    if !panel_marker.bound_camera.shows(event.camera) {
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
    if !panel_marker.bound_camera.shows(event.camera) {
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
    let BoundCamera::Showing(camera) = panel_marker.bound_camera else {
        return;
    };
    display.set_slow_mode_active(
        matches!(snapshot.slow_mode, SlowModeHint::ToggledBy(_))
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
    let BoundCamera::Showing(camera) = panel_marker.bound_camera else {
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
    let BoundCamera::Showing(camera) = panel_marker.bound_camera else {
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
    if !panel_marker.bound_camera.shows(event.camera) {
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
    if !panel_marker.bound_camera.shows(event.camera) {
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
    if !panel_marker.bound_camera.shows(event.camera) {
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
    if !panel_marker.bound_camera.shows(event.camera) {
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
    if !panel_marker.bound_camera.shows(event.camera) {
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
    if !panel_marker.bound_camera.shows(event.camera) {
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
    measurer: Res<DiegeticTextMeasurer>,
) {
    let (panel_entity, snapshot, mut display) = panel.into_inner();
    if display.render_state == RenderState::Idle {
        return;
    }
    if let Err(error) = commands.set_tree(
        panel_entity,
        build_guidance_tree(snapshot, display.display(), background.0, &fonts, &measurer),
    ) {
        warn!("failed to replace camera control panel tree: {error}");
    }
    display.render_state = RenderState::Idle;
}

fn reveal_panel_after_rebuild(
    event: On<PanelChanged>,
    mut commands: Commands,
    placements: Query<&EdgePanelPlacementRecord>,
) {
    let hidden_by_separation = placements
        .get(event.entity)
        .is_ok_and(|record| record.placement == PanelPlacement::Hidden);
    let mut entity = commands.entity(event.entity);
    if !hidden_by_separation {
        entity.insert(Visibility::Inherited);
    }
    entity.remove::<CameraGuidanceRevealPending>();
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::SystemState;
    use bevy::prelude::Alpha;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Name;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Transform;
    use hana_diegetic::ComputedDiegeticPanel;
    use hana_diegetic::DiegeticPanel;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::El;
    use hana_diegetic::Fit;
    use hana_diegetic::FontRegistry;
    use hana_diegetic::HeadlessDiegeticUiPlugin;
    use hana_diegetic::LayoutBuilder;
    use hana_diegetic::Sizing;
    use hana_diegetic::Unit;
    use hana_lagrange::OrbitCamInputMode;
    use hana_lagrange::OrbitCamPreset;
    use hana_lagrange::ResolvedCameraInputRoute;

    use super::*;
    use crate::constants::DEFAULT_PANEL_BACKGROUND;

    #[test]
    fn home_hint_location_follows_visible_title_bar_control() -> Result<(), String> {
        let mut world = bevy::prelude::World::new();
        let mut state: SystemState<Query<(&TitleBar, &TitleBarOverflow, Option<&Visibility>)>> =
            SystemState::new(&mut world);
        assert_eq!(
            home_hint_location(&state.get(&world).map_err(|error| error.to_string())?),
            HomeHintLocation::CameraPanel
        );

        let bar = world
            .spawn((
                TitleBar::new().control(crate::constants::HOME_CONTROL),
                TitleBarOverflow::default(),
                Visibility::Inherited,
            ))
            .id();
        assert_eq!(
            home_hint_location(&state.get(&world).map_err(|error| error.to_string())?),
            HomeHintLocation::TitleBar
        );

        world.entity_mut(bar).insert(Visibility::Hidden);
        assert_eq!(
            home_hint_location(&state.get(&world).map_err(|error| error.to_string())?),
            HomeHintLocation::CameraPanel
        );
        Ok(())
    }

    #[test]
    fn help_and_camera_rows_share_baseline_pitch() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let mut help_builder =
            LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
        let help = screen_panels::DescriptionPanel::new("Keys")
            .line("First")
            .line("Second")
            .line("Third");
        screen_panels::description::build_description_layout(&mut help_builder, &help, &fonts);
        let help_panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(help_builder.build())
            .build()
            .map_err(|error| error.to_string())?;
        let camera_snapshot = CameraGuidanceSnapshot {
            camera_label: "Camera".to_string(),
            mode_label:   "Controls".to_string(),
            mode_value:   "Mouse".to_string(),
            slow_mode:    SlowModeHint::NoDisplayableSlowToggle,
            settings:     Vec::new(),
            rows:         ["First", "Second", "Third"]
                .into_iter()
                .map(|label| CameraGuidanceRow::new(CameraGuidanceAction::Orbit, label))
                .collect(),
        };
        let camera_panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(build_guidance_tree(
                &camera_snapshot,
                CameraGuidanceDisplay::default(),
                CAMERA_PANEL_BACKGROUND,
                &fonts,
                &DiegeticTextMeasurer::default(),
            ))
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let help_entity = app.world_mut().spawn(help_panel).id();
        let camera_entity = app.world_mut().spawn(camera_panel).id();
        app.update();
        let pitch = |entity| -> Result<f32, String> {
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(|panel| panel.layout().solved())
                .ok_or("panel did not solve")?;
            let baseline = |label: &str| -> Result<f32, String> {
                let command = solved
                    .commands
                    .iter()
                    .find(|command| {
                        format!("{:?}", command.kind)
                            .starts_with(&format!("Text {{ text: {label:?},"))
                    })
                    .ok_or_else(|| format!("{label} missing"))?;
                let font = fonts
                    .font(hana_diegetic::FontId::MONOSPACE)
                    .map_err(|error| error.to_string())?;
                let size =
                    screen_panels::integral_advance_size(&fonts, crate::constants::LABEL_SIZE);
                let metrics = font.metrics(size.0 * 4.0 / 3.0);
                Ok(metrics
                    .line_gap
                    .mul_add(0.5, metrics.ascent)
                    .mul_add(Unit::Pixels.to_points(), command.bounds.y))
            };
            Ok(baseline("Second")? - baseline("First")?)
        };
        let help_pitch = pitch(help_entity)?;
        let camera_pitch = pitch(camera_entity)?;
        assert!(
            (help_pitch - camera_pitch).abs() < 0.1,
            "help pitch {help_pitch}, camera pitch {camera_pitch}"
        );
        Ok(())
    }

    #[test]
    fn camera_and_shared_panel_backgrounds_are_opaque() {
        assert_eq!(
            CameraControlPanelBackground::default().0,
            CAMERA_PANEL_BACKGROUND
        );
        assert!((CAMERA_PANEL_BACKGROUND.alpha() - 1.0).abs() < f32::EPSILON);
        assert!((DEFAULT_PANEL_BACKGROUND.alpha() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn removing_last_camera_restores_startup_snapshot_and_text() {
        let fonts = FontRegistry::new();
        assert!(fonts.is_ok());
        let Ok(fonts) = fonts else { return };
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin)
            .insert_resource(fonts)
            .insert_resource(ResolvedCameraInputRoute::default())
            .insert_resource(CameraControlPanelBackground::default())
            .add_systems(PostUpdate, rebind_panel_on_route_change);
        let camera = app
            .world_mut()
            .spawn((
                Name::new("Departing camera"),
                OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
            ))
            .id();
        let snapshot = default_snapshot(HomeHintLocation::CameraPanel);
        let tree = build_guidance_tree(
            &snapshot,
            CameraGuidanceDisplay::default(),
            CAMERA_PANEL_BACKGROUND,
            app.world().resource::<FontRegistry>(),
            app.world().resource::<DiegeticTextMeasurer>(),
        );
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(tree)
            .build();
        assert!(panel.is_ok());
        let Ok(panel) = panel else { return };
        let panel_entity = app
            .world_mut()
            .spawn((
                CameraGuidancePanel {
                    bound_camera: BoundCamera::AwaitingCamera,
                    owner_shown:  true,
                    home_hint:    HomeHintLocation::CameraPanel,
                },
                snapshot,
                CameraGuidanceDisplayState::default(),
                panel,
                Transform::default(),
            ))
            .id();

        app.update();
        assert_eq!(
            app.world()
                .get::<CameraGuidancePanel>(panel_entity)
                .map(|panel| panel.bound_camera),
            Some(BoundCamera::Showing(camera))
        );
        assert_eq!(
            app.world()
                .get::<CameraGuidanceSnapshot>(panel_entity)
                .map(|snapshot| snapshot.camera_label.as_str()),
            Some("Departing camera")
        );
        let bound_panel = app.world().get::<DiegeticPanel>(panel_entity);
        assert!(bound_panel.is_some());
        let Some(bound_panel) = bound_panel else {
            return;
        };
        let bound_tree = format!("{:?}", bound_panel.tree());
        assert!(bound_tree.contains("Departing camera"));
        assert!(bound_tree.contains("Controls: Blender"));
        assert!(!bound_tree.contains("Orbit camera"));

        app.world_mut().despawn(camera);
        app.update();
        assert_eq!(
            app.world()
                .get::<CameraGuidancePanel>(panel_entity)
                .map(|panel| panel.bound_camera),
            Some(BoundCamera::AwaitingCamera)
        );
        assert_eq!(
            app.world().get::<CameraGuidanceSnapshot>(panel_entity),
            Some(&default_snapshot(HomeHintLocation::CameraPanel))
        );
        let panel = app.world().get::<DiegeticPanel>(panel_entity);
        assert!(panel.is_some());
        let Some(panel) = panel else { return };
        let text = format!("{:?}", panel.tree());
        assert!(text.contains("Orbit camera"));
        assert!(!text.contains("Camera: Orbit camera"));
        assert!(!text.contains("Controls: Blender"));
        assert!(!text.contains("Departing camera"));
    }
}
