//! The editor's windowed placement survives full-screen driver readback.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::window::WindowMode;
use bevy::window::WindowResolution;
use bevy::winit::WINIT_WINDOWS;
use hana_kana::ToF32;
use hana_rigging::prelude::BindingAuthoring;
use hana_rigging::prelude::BindingError;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;

use super::EstablishedWindowPlacement;
use super::EstablishedWindowPosition;
use super::PersistedPosition;
use super::PersistedWindowPlacements;
use super::ReadbackWindowPosition;
use super::RestorableWindowPosition;
use super::SavedWindowMode;
use super::WindowPlacementConfiguration;
use crate::CurrentMonitor;
use crate::Platform;
use crate::constants::X11_WINDOWED_RETURN_MAX_FRAMES;
use crate::restore::NativeWindowReadbackPosition;
use crate::restore::RetainedWindowedPhysicalPosition;
use crate::restore::WindowRestoreAttempt;

/// The native mode change and driver placement that restore the windowed view.
#[derive(Component)]
pub(crate) enum WindowedPlacementRestore {
    LeavingFullScreen(Box<BindingAuthoring<WindowPlacementConfiguration>>),
    AwaitingDriver,
    Applying,
    RestoringDirect { remaining_frames: u8 },
}

enum WindowedRestoreDispatch {
    DirectWindow,
    WindowDriver,
}

#[derive(Component, Clone)]
pub(crate) struct WindowedPlacement {
    placement:  EstablishedWindowPlacement,
    resolution: WindowResolution,
    position:   WindowPosition,
    monitor:    crate::MonitorDescriptor,
}

impl WindowedPlacement {
    fn from_saved(
        saved: &super::PersistedWindowState,
        window: &Window,
        monitor: &CurrentMonitor,
    ) -> Self {
        let mut placement = EstablishedWindowPlacement::from(saved);
        placement.saved_window_mode = SavedWindowMode::Windowed;
        let mut resolution = window.resolution.clone();
        resolution.set(
            placement.logical_size.x.to_f32(),
            placement.logical_size.y.to_f32(),
        );
        let position = match placement.restorable_position(&monitor.descriptor) {
            RestorableWindowPosition::Restorable {
                physical_position, ..
            } => WindowPosition::At(physical_position),
            RestorableWindowPosition::CompositorControlled => window.position,
        };
        Self {
            placement,
            resolution,
            position,
            monitor: monitor.descriptor,
        }
    }

    pub(super) const fn retain_in(&self, saved: &mut super::PersistedWindowState) {
        saved.position = match self.placement.position {
            EstablishedWindowPosition::Restorable { logical_offset } => {
                PersistedPosition::MonitorOffset(logical_offset)
            },
            EstablishedWindowPosition::CompositorControlled => PersistedPosition::Unpositioned,
        };
        saved.logical_width = self.placement.logical_size.x;
        saved.logical_height = self.placement.logical_size.y;
    }

    fn read(window: &Window, monitor: &CurrentMonitor, platform: Platform) -> Self {
        let position = match window.position {
            WindowPosition::At(position) => ReadbackWindowPosition::Observed(position),
            _ => ReadbackWindowPosition::Unavailable,
        };
        let mut placement =
            EstablishedWindowPlacement::from_readback(window, monitor, position, platform);
        placement.saved_window_mode = SavedWindowMode::Windowed;
        Self {
            placement,
            resolution: window.resolution.clone(),
            position: window.position,
            monitor: monitor.descriptor,
        }
    }

    fn restore(&self, window: &mut Window, monitor: &CurrentMonitor) {
        window.mode = WindowMode::Windowed;
        self.restore_resolution(window);
        if let ReadbackWindowPosition::Observed(position) = self.physical_position_on(monitor) {
            window.position = WindowPosition::At(position);
        }
    }

    fn physical_position_on(&self, monitor: &CurrentMonitor) -> ReadbackWindowPosition {
        match self.placement.restorable_position(&monitor.descriptor) {
            RestorableWindowPosition::Restorable {
                physical_position, ..
            } => {
                if self.monitor != monitor.descriptor {
                    ReadbackWindowPosition::Observed(physical_position)
                } else if let WindowPosition::At(position) = self.position {
                    ReadbackWindowPosition::Observed(position)
                } else {
                    ReadbackWindowPosition::Unavailable
                }
            },
            RestorableWindowPosition::CompositorControlled => ReadbackWindowPosition::Unavailable,
        }
    }

    fn restore_resolution(&self, window: &mut Window) {
        if window.resolution.scale_factor().to_bits() == self.resolution.scale_factor().to_bits() {
            // Keep native pixels exact when no scale conversion is needed.
            window.resolution.set_physical_resolution(
                self.resolution.physical_width(),
                self.resolution.physical_height(),
            );
        } else {
            window
                .resolution
                .set(self.resolution.width(), self.resolution.height());
        }
    }
}

fn request_windowed_restore(
    world: &mut World,
    entity: Entity,
    retained: &WindowedPlacement,
) -> Result<WindowedRestoreDispatch, BindingError> {
    let Ok(driver) = crate::window_placement_driver(world) else {
        return Ok(WindowedRestoreDispatch::DirectWindow);
    };
    let Ok(role) = super::primary_window_role() else {
        return Ok(WindowedRestoreDispatch::DirectWindow);
    };
    let Some(bindings) = world.get_resource::<Bindings>() else {
        return Ok(WindowedRestoreDispatch::DirectWindow);
    };
    let Ok(binding) = bindings.binding(&role) else {
        return Ok(WindowedRestoreDispatch::DirectWindow);
    };
    let authoring = BindingAuthoring::new(
        role,
        binding.endpoint.clone(),
        driver,
        WindowPlacementConfiguration::Established(retained.placement.clone()),
        binding.policy(),
    );
    let restore = if native_window_is_full_screen(entity) {
        WindowedPlacementRestore::LeavingFullScreen(Box::new(authoring))
    } else {
        world
            .resource_mut::<Bindings>()
            .replace_authoring(authoring)?;
        WindowedPlacementRestore::AwaitingDriver
    };
    world.entity_mut(entity).insert((retained.clone(), restore));
    if let WindowPosition::At(position) = retained.position {
        RetainedWindowedPhysicalPosition::retain(
            world,
            entity,
            NativeWindowReadbackPosition::from_physical_pixels(position),
            retained.monitor,
        );
    } else {
        RetainedWindowedPhysicalPosition::clear(world, entity);
    }
    if let Some(mut window) = world.get_mut::<Window>(entity) {
        window.mode = WindowMode::Windowed;
    }
    leave_native_full_screen(entity);
    Ok(WindowedRestoreDispatch::WindowDriver)
}

fn native_window_is_full_screen(entity: Entity) -> bool {
    WINIT_WINDOWS.with_borrow(|windows| {
        windows
            .get_window(entity)
            .is_some_and(|window| window.fullscreen().is_some())
    })
}

fn leave_native_full_screen(entity: Entity) {
    // A native macOS toggle does not update Bevy's cached Window.mode, so assigning
    // Windowed alone does not make bevy_winit send a native mode request.
    WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(window) = windows.get_window(entity)
            && window.fullscreen().is_some()
        {
            window.set_fullscreen(None);
        }
    });
}

/// Runs on the main thread after the preceding frame applied the native mode request.
/// The driver then measures windowed frame extents, and its completion retains native pixels.
pub(super) fn advance_windowed_placement_restore(world: &mut World) {
    let entities = world
        .query_filtered::<Entity, With<WindowedPlacementRestore>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        match world.get::<WindowedPlacementRestore>(entity) {
            Some(WindowedPlacementRestore::LeavingFullScreen(_))
                if !native_window_is_full_screen(entity) =>
            {
                apply_waiting_windowed_placement(world, entity);
            },
            Some(WindowedPlacementRestore::AwaitingDriver)
                if world.get::<WindowRestoreAttempt>(entity).is_some() =>
            {
                world
                    .entity_mut(entity)
                    .insert(WindowedPlacementRestore::Applying);
            },
            Some(WindowedPlacementRestore::Applying)
                if world.get::<WindowRestoreAttempt>(entity).is_none() =>
            {
                finish_windowed_placement_restore(world, entity);
            },
            Some(WindowedPlacementRestore::RestoringDirect { .. }) => {
                finish_direct_windowed_placement_restore(world, entity);
            },
            _ => {},
        }
    }
}

fn apply_waiting_windowed_placement(world: &mut World, entity: Entity) {
    let Some(WindowedPlacementRestore::LeavingFullScreen(authoring)) =
        world.entity_mut(entity).take::<WindowedPlacementRestore>()
    else {
        return;
    };
    match world
        .resource_mut::<Bindings>()
        .replace_authoring(*authoring)
    {
        Ok(_) => {
            world
                .entity_mut(entity)
                .insert(WindowedPlacementRestore::AwaitingDriver);
        },
        Err(error) => {
            drop_windowed_return(world, entity);
            warn!("windowed placement could not be requested: {error}");
        },
    }
}

fn finish_windowed_placement_restore(world: &mut World, entity: Entity) {
    let Some(placement) = world.get::<WindowedPlacement>(entity).cloned() else {
        return;
    };
    let world_platform_needs_frame_compensation = uses_x11_frame_compensation(world);
    if let Some(monitor) = world.get::<CurrentMonitor>(entity).copied() {
        let Some(mut window) = world.get_mut::<Window>(entity) else {
            return;
        };
        if window.mode != WindowMode::Windowed {
            return;
        }
        if world_platform_needs_frame_compensation {
            placement.restore_resolution(&mut window);
        } else {
            placement.restore(&mut window, &monitor);
        }
    }
    drop_windowed_return(world, entity);
}

fn drop_windowed_return(world: &mut World, entity: Entity) {
    RetainedWindowedPhysicalPosition::clear(world, entity);
    world
        .entity_mut(entity)
        .remove::<WindowedPlacementRestore>();
}

fn uses_x11_frame_compensation(world: &World) -> bool {
    cfg!(all(target_os = "linux", feature = "workaround-winit-4445"))
        && world
            .get_resource::<Platform>()
            .is_some_and(|platform| platform.needs_frame_compensation())
}

fn finish_direct_windowed_placement_restore(world: &mut World, entity: Entity) {
    let Some(placement) = world.get::<WindowedPlacement>(entity).cloned() else {
        drop_windowed_return(world, entity);
        return;
    };
    if !direct_return_has_budget(world, entity) {
        finish_uncompensated_windowed_return(world, entity, &placement);
        return;
    }
    let Some(monitor) = world.get::<CurrentMonitor>(entity).copied() else {
        return;
    };
    if let Some(mut window) = world.get_mut::<Window>(entity) {
        window.mode = WindowMode::Windowed;
        placement.restore_resolution(&mut window);
    }
    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    if uses_x11_frame_compensation(world) {
        if let ReadbackWindowPosition::Observed(position) = placement.physical_position_on(&monitor)
        {
            let progress = WindowPlacementConfiguration::restore_x11_windowed_position(
                world,
                entity,
                NativeWindowReadbackPosition::from_physical_pixels(position),
            );
            if progress.requires_fallback() {
                finish_uncompensated_windowed_return(world, entity, &placement);
                return;
            }
            if !progress.is_complete() {
                return;
            }
        }
    } else if let Some(mut window) = world.get_mut::<Window>(entity) {
        placement.restore(&mut window, &monitor);
    }
    #[cfg(not(all(target_os = "linux", feature = "workaround-winit-4445")))]
    if let Some(mut window) = world.get_mut::<Window>(entity) {
        placement.restore(&mut window, &monitor);
    }
    drop_windowed_return(world, entity);
}

fn direct_return_has_budget(world: &mut World, entity: Entity) -> bool {
    let Some(mut restore) = world.get_mut::<WindowedPlacementRestore>(entity) else {
        return false;
    };
    let WindowedPlacementRestore::RestoringDirect { remaining_frames } = &mut *restore else {
        return false;
    };
    if *remaining_frames == 0 {
        return false;
    }
    *remaining_frames -= 1;
    true
}

fn finish_uncompensated_windowed_return(
    world: &mut World,
    entity: Entity,
    placement: &WindowedPlacement,
) {
    let monitor = world.get::<CurrentMonitor>(entity).copied();
    if let Some(mut window) = world.get_mut::<Window>(entity) {
        if let Some(monitor) = monitor {
            placement.restore(&mut window, &monitor);
        } else {
            window.mode = WindowMode::Windowed;
            placement.restore_resolution(&mut window);
            window.position = placement.position;
        }
    }
    drop_windowed_return(world, entity);
}

pub(super) fn remember_windowed_placement(
    mut commands: Commands,
    windows: Query<
        (Entity, &Window, &CurrentMonitor, Option<&WindowedPlacement>),
        (
            With<PrimaryWindow>,
            Or<(
                Changed<Window>,
                Changed<CurrentMonitor>,
                Without<WindowedPlacement>,
            )>,
            Without<WindowRestoreAttempt>,
            Without<WindowedPlacementRestore>,
        ),
    >,
    platform: Res<Platform>,
    persisted: Res<PersistedWindowPlacements>,
    bindings: Res<Bindings>,
    statuses: Query<&RoleStatus>,
) {
    let Ok(role) = super::primary_window_role() else {
        return;
    };
    let saved = persisted.states.get(&role);
    let established = bindings
        .role_entity(&role)
        .ok()
        .and_then(|entity| statuses.get(entity).ok())
        .is_some_and(|status| matches!(status.view(), RoleStatusView::Established { .. }));
    for (entity, window, monitor, retained) in &windows {
        if let Some(saved) = saved {
            if retained.is_none() {
                commands
                    .entity(entity)
                    .insert(WindowedPlacement::from_saved(
                        &saved.window_state,
                        window,
                        monitor,
                    ));
                continue;
            }
            // Before the first restore attempt exists, live geometry is still the
            // application's opening default. It cannot replace a saved placement.
            if !established {
                continue;
            }
        }
        if !is_full_screen(window, monitor) {
            commands
                .entity(entity)
                .insert(WindowedPlacement::read(window, monitor, *platform));
        }
    }
}

fn is_full_screen(window: &Window, monitor: &CurrentMonitor) -> bool {
    window.mode != WindowMode::Windowed || monitor.effective_window_mode != WindowMode::Windowed
}

impl WindowPlacementConfiguration {
    /// Whether either the requested mode or native monitor readback is full screen.
    #[must_use]
    pub fn is_full_screen(world: &World, entity: Entity) -> bool {
        world.get::<Window>(entity).is_some_and(|window| {
            world
                .get::<CurrentMonitor>(entity)
                .map_or(window.mode != WindowMode::Windowed, |monitor| {
                    is_full_screen(window, monitor)
                })
        })
    }

    /// Toggle an editor window on its current monitor, retaining its windowed placement.
    ///
    /// A window without current-monitor readback is left untouched. Output windows are ignored.
    /// On a full-screen launch the persisted windowed placement supplies the return geometry.
    pub fn toggle_full_screen(world: &mut World, entity: Entity) {
        if world.get::<PrimaryWindow>(entity).is_none()
            || world.get::<WindowedPlacementRestore>(entity).is_some()
        {
            return;
        }
        let Some(monitor) = world.get::<CurrentMonitor>(entity).copied() else {
            return;
        };
        let Some(window) = world.get::<Window>(entity) else {
            return;
        };
        if !is_full_screen(window, &monitor) {
            let platform = *world.resource::<Platform>();
            let placement = WindowedPlacement::read(window, &monitor, platform);
            world.entity_mut(entity).insert(placement);
            if let Some(mut window) = world.get_mut::<Window>(entity) {
                window.mode = WindowMode::BorderlessFullscreen(monitor.index.selection());
            }
            return;
        }
        let retained = world.get::<WindowedPlacement>(entity).cloned().or_else(|| {
            let role = super::primary_window_role().ok()?;
            let saved = world
                .get_resource::<PersistedWindowPlacements>()?
                .states
                .get(&role)?;
            Some(WindowedPlacement::from_saved(
                &saved.window_state,
                window,
                &monitor,
            ))
        });
        if let Some(retained) = retained {
            match request_windowed_restore(world, entity, &retained) {
                Ok(WindowedRestoreDispatch::WindowDriver) => return,
                Err(error) => {
                    warn!("windowed placement could not be requested: {error}");
                },
                Ok(WindowedRestoreDispatch::DirectWindow) => {},
            }
            leave_native_full_screen(entity);
            world.entity_mut(entity).insert((
                retained.clone(),
                WindowedPlacementRestore::RestoringDirect {
                    remaining_frames: X11_WINDOWED_RETURN_MAX_FRAMES,
                },
            ));
            if let Some(mut window) = world.get_mut::<Window>(entity) {
                window.mode = WindowMode::Windowed;
                retained.restore_resolution(&mut window);
            }
            finish_direct_windowed_placement_restore(world, entity);
        } else if let Some(mut window) = world.get_mut::<Window>(entity) {
            window.mode = WindowMode::Windowed;
            leave_native_full_screen(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use bevy::ecs::system::RunSystemOnce;
    use bevy::time::TimeUpdateStrategy;
    use bevy::window::ExitCondition;
    use bevy::window::Monitor;
    use bevy::window::MonitorSelection;
    use bevy::window::OnMonitor;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::PlatformDeviceHandle;
    use hana_rigging::prelude::ReportedId;
    use hana_rigging::prelude::RoleKey;
    use tempfile::TempDir;

    use super::*;
    use crate::DisplayTestAdapter;
    use crate::DisplayTestDescriptor;
    use crate::DisplayTestDeviceKey;
    use crate::WindowManagerPlugin;
    use crate::persistence::PersistedWindowState;
    use crate::persistence::PersistedWindowStateDecodeOutcome;
    use crate::restore::InjectedWinitWindows;

    const RETAINED_PHYSICAL_POSITION: IVec2 = IVec2::new(101, 81);
    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    const X11_PHYSICAL_FRAME_TOP: i32 = 28;

    struct PlacementApp {
        app:       App,
        editor:    Entity,
        directory: TempDir,
    }

    impl PlacementApp {
        fn new(
            window: Window,
            saved: HashMap<RoleKey, PersistedWindowState>,
            display: DisplayTestDescriptor,
        ) -> Result<Self, String> {
            let scale = window.resolution.scale_factor();
            let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
            let path = directory.path().join("windows.ron");
            let states = saved
                .into_iter()
                .map(|(role, state)| (role, state.into()))
                .collect();
            assert_eq!(
                super::super::save::save_all_states(&path, &states),
                super::super::save::StateFileWrite::Written
            );
            let displays = DisplayTestAdapter::new();
            displays.observe_displays(vec![display]);
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                },
                displays,
            ))
            .insert_resource(Platform::MacOs)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                50,
            )))
            .add_plugins(WindowManagerPlugin::with_path(path))
            .add_observer(
                move |added: On<Add<Monitor>>, mut monitors: Query<&mut Monitor>| {
                    if let Ok(mut monitor) = monitors.get_mut(added.entity) {
                        monitor.scale_factor = f64::from(scale);
                    }
                },
            );
            // Let persistence load and display enumeration finish before this window opens.
            app.update();
            assert_eq!(
                app.world().resource::<crate::Monitors>().first().scale,
                f64::from(scale),
                "the installed topology must carry the window's native scale"
            );
            let monitor_entity = app
                .world_mut()
                .query_filtered::<Entity, With<Monitor>>()
                .single(app.world())
                .map_err(|error| error.to_string())?;
            let monitor = CurrentMonitor {
                descriptor:            crate::MonitorDescriptor::for_current_enumeration(
                    0,
                    f64::from(scale),
                    IVec2::ZERO,
                    UVec2::new(1920, 1080),
                ),
                effective_window_mode: WindowMode::Windowed,
            };
            let editor = app
                .world_mut()
                .spawn((window, PrimaryWindow, OnMonitor(monitor_entity), monitor))
                .id();
            app.world_mut().init_resource::<InjectedWinitWindows>();
            app.world_mut()
                .resource_mut::<InjectedWinitWindows>()
                .insert(editor, UVec2::ZERO);
            Ok(Self {
                app,
                editor,
                directory,
            })
        }

        fn settle(&mut self) -> Result<bool, String> {
            let role = super::super::primary_window_role().map_err(|error| error.to_string())?;
            let mut attempted = false;
            for _ in 0..120 {
                self.app.update();
                let world = self.app.world();
                attempted |= world.get::<WindowRestoreAttempt>(self.editor).is_some();
                let established = world
                    .resource::<Bindings>()
                    .role_entity(&role)
                    .ok()
                    .and_then(|entity| world.get::<RoleStatus>(entity))
                    .is_some_and(|status| {
                        matches!(status.view(), RoleStatusView::Established { .. })
                    });
                if established
                    && world.get::<WindowRestoreAttempt>(self.editor).is_none()
                    && world.get::<WindowedPlacementRestore>(self.editor).is_none()
                {
                    return Ok(attempted);
                }
            }
            Err("window placement did not finish".into())
        }

        fn window(&self) -> Result<&Window, String> {
            self.app
                .world()
                .get::<Window>(self.editor)
                .ok_or_else(|| "editor missing".into())
        }

        fn saved(&self) -> Result<PersistedWindowState, String> {
            let contents = std::fs::read_to_string(self.directory.path().join("windows.ron"))
                .map_err(|error| error.to_string())?;
            let PersistedWindowStateDecodeOutcome::Decoded(mut states) =
                super::super::decode_persisted_state_for_test(&contents)
            else {
                return Err("saved placements rejected".into());
            };
            states
                .remove(&super::super::primary_window_role().map_err(|error| error.to_string())?)
                .map(|placement| placement.window_state)
                .ok_or_else(|| "saved editor missing".into())
        }
    }

    fn display(published: bool) -> Result<DisplayTestDescriptor, String> {
        let handle = PlatformDeviceHandle::Reported(
            ReportedId::new("placement-display").map_err(|error| error.to_string())?,
        );
        let display = if published {
            DisplayTestDescriptor::new("placement", b"placement-display".to_vec(), handle)
        } else {
            DisplayTestDescriptor::without_published_identity("placement", handle)
        };
        Ok(display.placed_at(IVec2::ZERO, UVec2::new(1920, 1080)))
    }

    #[test]
    fn full_screen_startup_retains_saved_geometry_before_the_first_driver_attempt()
    -> Result<(), String> {
        let descriptor = display(true)?;
        let adapter = DisplayTestAdapter::new();
        adapter.observe_displays(vec![descriptor.clone()]);
        let DisplayTestDeviceKey::Keyed(key) = adapter.display_device_key("placement") else {
            return Err("display key missing".into());
        };
        let saved = EstablishedWindowPlacement {
            position:          EstablishedWindowPosition::Restorable {
                logical_offset: IVec2::new(203, 117),
            },
            logical_size:      UVec2::new(873, 611),
            saved_window_mode: SavedWindowMode::BorderlessFullscreen,
        }
        .project(key, "startup");
        let role = super::super::primary_window_role().map_err(|error| error.to_string())?;
        let mut fixture = PlacementApp::new(
            Window::default(),
            HashMap::from([(role, saved)]),
            descriptor,
        )?;
        assert!(
            fixture
                .app
                .world()
                .get::<WindowRestoreAttempt>(fixture.editor)
                .is_none()
        );
        // Run the production capture before the first driver restore attempt exists.
        for _ in 0..2 {
            fixture
                .app
                .world_mut()
                .run_system_once(remember_windowed_placement)
                .map_err(|error| error.to_string())?;
        }
        fixture.settle()?;
        assert!(WindowPlacementConfiguration::is_full_screen(
            fixture.app.world(),
            fixture.editor
        ));
        let saved = fixture.saved()?;
        assert_eq!((saved.logical_width, saved.logical_height), (873, 611));
        assert_eq!(
            saved.position,
            PersistedPosition::MonitorOffset(IVec2::new(203, 117))
        );
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        fixture.settle()?;
        assert_eq!(fixture.window()?.mode, WindowMode::Windowed);
        assert_eq!(
            fixture.window()?.resolution.physical_size(),
            UVec2::new(873, 611)
        );
        assert_eq!(
            fixture.window()?.position,
            WindowPosition::At(IVec2::new(203, 117))
        );
        Ok(())
    }

    #[test]
    fn full_screen_exit_without_a_display_binding_restores_or_requests_windowed()
    -> Result<(), String> {
        for retain in [true, false] {
            let mut fixture = PlacementApp::new(
                Window {
                    resolution: (901, 703).into(),
                    position: WindowPosition::At(IVec2::new(103, 87)),
                    ..default()
                },
                HashMap::new(),
                display(false)?,
            )?;
            for _ in 0..4 {
                fixture.app.update();
            }
            let role = super::super::primary_window_role().map_err(|error| error.to_string())?;
            assert!(crate::window_placement_driver(fixture.app.world()).is_ok());
            assert!(
                fixture
                    .app
                    .world()
                    .resource::<Bindings>()
                    .binding(&role)
                    .is_err()
            );
            WindowPlacementConfiguration::toggle_full_screen(
                fixture.app.world_mut(),
                fixture.editor,
            );
            fixture
                .app
                .world_mut()
                .get_mut::<Window>(fixture.editor)
                .ok_or("editor missing")?
                .resolution
                .set_physical_resolution(1920, 1080);
            if !retain {
                fixture
                    .app
                    .world_mut()
                    .entity_mut(fixture.editor)
                    .remove::<WindowedPlacement>();
            }
            WindowPlacementConfiguration::toggle_full_screen(
                fixture.app.world_mut(),
                fixture.editor,
            );
            assert_eq!(fixture.window()?.mode, WindowMode::Windowed);
            if retain {
                assert_eq!(
                    fixture.window()?.resolution.physical_size(),
                    UVec2::new(901, 703)
                );
                assert_eq!(
                    fixture.window()?.position,
                    WindowPosition::At(IVec2::new(103, 87))
                );
            }
        }
        Ok(())
    }

    #[test]
    fn native_full_screen_readback_does_not_replace_windowed_placement() -> Result<(), String> {
        let mut fixture = PlacementApp::new(
            Window {
                resolution: (901, 703).into(),
                position: WindowPosition::At(IVec2::new(103, 87)),
                ..default()
            },
            HashMap::new(),
            display(true)?,
        )?;
        fixture.settle()?;
        let before_size = fixture.window()?.resolution.physical_size();
        let before_position = fixture.window()?.position;
        {
            let mut window = fixture
                .app
                .world_mut()
                .get_mut::<Window>(fixture.editor)
                .ok_or("editor missing")?;
            assert_eq!(window.mode, WindowMode::Windowed);
            window.resolution.set_physical_resolution(1920, 1080);
            window.position = WindowPosition::At(IVec2::ZERO);
        }
        fixture
            .app
            .world_mut()
            .get_mut::<CurrentMonitor>(fixture.editor)
            .ok_or("monitor missing")?
            .effective_window_mode = WindowMode::BorderlessFullscreen(MonitorSelection::Index(0));
        fixture
            .app
            .world_mut()
            .run_system_once(remember_windowed_placement)
            .map_err(|error| error.to_string())?;
        assert!(WindowPlacementConfiguration::is_full_screen(
            fixture.app.world(),
            fixture.editor
        ));
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        assert!(
            fixture
                .app
                .world()
                .get::<WindowedPlacementRestore>(fixture.editor)
                .is_some()
        );
        fixture.settle()?;
        assert_eq!(fixture.window()?.mode, WindowMode::Windowed);
        assert_eq!(fixture.window()?.resolution.physical_size(), before_size);
        assert_eq!(fixture.window()?.position, before_position);
        Ok(())
    }

    #[test]
    fn full_screen_return_keeps_native_pixels_at_fractional_scale() -> Result<(), String> {
        for scale in [1.75, 2.0] {
            let mut window = Window {
                resolution: WindowResolution::new(454, 333),
                position: WindowPosition::At(RETAINED_PHYSICAL_POSITION),
                ..default()
            };
            window.resolution.set_scale_factor(scale);
            let mut fixture = PlacementApp::new(window, HashMap::new(), display(true)?)?;
            fixture.settle()?;
            // Set native pixel values after startup, as a user resize and move would.
            {
                let mut window = fixture
                    .app
                    .world_mut()
                    .get_mut::<Window>(fixture.editor)
                    .ok_or("editor missing")?;
                window.resolution.set_physical_resolution(454, 333);
                window.position = WindowPosition::At(RETAINED_PHYSICAL_POSITION);
            }
            fixture.app.update();
            WindowPlacementConfiguration::toggle_full_screen(
                fixture.app.world_mut(),
                fixture.editor,
            );
            assert_eq!(
                fixture.window()?.mode,
                WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Index(0))
            );
            {
                let mut full = fixture
                    .app
                    .world_mut()
                    .get_mut::<Window>(fixture.editor)
                    .ok_or("editor missing")?;
                full.resolution.set_physical_resolution(1920, 1080);
                full.position = WindowPosition::At(IVec2::ZERO);
            }
            WindowPlacementConfiguration::toggle_full_screen(
                fixture.app.world_mut(),
                fixture.editor,
            );
            assert!(
                fixture
                    .app
                    .world()
                    .get::<WindowedPlacementRestore>(fixture.editor)
                    .is_some()
            );
            assert!(
                fixture.settle()?,
                "the production placement driver must run"
            );
            let restored = fixture.window()?;
            assert_eq!(restored.mode, WindowMode::Windowed);
            assert_eq!(restored.resolution.physical_size(), UVec2::new(454, 333));
            assert_eq!(
                restored.position,
                WindowPosition::At(RETAINED_PHYSICAL_POSITION)
            );
        }
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    #[derive(Component)]
    struct X11WindowManagerReadback {
        observed:        IVec2,
        requests:        Vec<IVec2>,
        accept_requests: bool,
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    fn apply_x11_window_manager_requests(
        mut windows: Query<(&mut Window, &mut X11WindowManagerReadback)>,
    ) {
        for (mut window, mut native) in &mut windows {
            let WindowPosition::At(request) = window.position else {
                continue;
            };
            if request != native.observed {
                native.requests.push(request);
                if native.accept_requests {
                    native.observed = if window.mode == WindowMode::Windowed {
                        request + IVec2::new(0, X11_PHYSICAL_FRAME_TOP)
                    } else {
                        request
                    };
                }
            }
            window.position = WindowPosition::At(native.observed);
        }
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    fn x11_return_fixture(scale: f32, binding: bool) -> Result<PlacementApp, String> {
        let mut window = Window {
            resolution: (454, 333).into(),
            ..default()
        };
        window.resolution.set_scale_factor(scale);
        let mut fixture = PlacementApp::new(window, HashMap::new(), display(binding)?)?;
        if binding {
            fixture.settle()?;
        } else {
            for _ in 0..4 {
                fixture.app.update();
            }
        }
        {
            let mut window = fixture
                .app
                .world_mut()
                .get_mut::<Window>(fixture.editor)
                .ok_or("editor missing")?;
            window.resolution.set_physical_resolution(454, 333);
            window.position = WindowPosition::At(RETAINED_PHYSICAL_POSITION);
        }
        fixture
            .app
            .insert_resource(Platform::X11)
            .add_systems(Last, apply_x11_window_manager_requests);
        fixture
            .app
            .world_mut()
            .entity_mut(fixture.editor)
            .insert(X11WindowManagerReadback {
                observed:        RETAINED_PHYSICAL_POSITION,
                requests:        Vec::new(),
                accept_requests: true,
            });
        WindowPlacementConfiguration::observe_x11_frame_top_for_test(
            fixture.app.world_mut(),
            fixture.editor,
            X11_PHYSICAL_FRAME_TOP,
        );
        fixture.app.update();
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        {
            let mut window = fixture
                .app
                .world_mut()
                .get_mut::<Window>(fixture.editor)
                .ok_or("editor missing")?;
            window.resolution.set_physical_resolution(1920, 1080);
            window.position = WindowPosition::At(IVec2::ZERO);
        }
        fixture.app.update();
        Ok(fixture)
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    fn assert_x11_native_return(fixture: &PlacementApp) -> Result<(), String> {
        let native = fixture
            .app
            .world()
            .get::<X11WindowManagerReadback>(fixture.editor)
            .ok_or("native observations missing")?;
        assert_eq!(native.observed, RETAINED_PHYSICAL_POSITION);
        assert!(
            native
                .requests
                .contains(&(RETAINED_PHYSICAL_POSITION - IVec2::new(0, X11_PHYSICAL_FRAME_TOP))),
            "no exact compensated native request was applied: {:?}",
            native.requests
        );
        assert_eq!(
            fixture.window()?.resolution.physical_size(),
            UVec2::new(454, 333)
        );
        assert_eq!(fixture.window()?.mode, WindowMode::Windowed);
        assert!(
            fixture
                .app
                .world()
                .get::<WindowedPlacementRestore>(fixture.editor)
                .is_none()
        );
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    #[test]
    fn full_screen_x11_driver_returns_to_exact_native_pixels_after_frame_compensation()
    -> Result<(), String> {
        for scale in [2.0, 1.75] {
            let mut fixture = x11_return_fixture(scale, true)?;
            WindowPlacementConfiguration::toggle_full_screen(
                fixture.app.world_mut(),
                fixture.editor,
            );
            assert!(
                fixture.settle()?,
                "the production placement driver must run"
            );
            assert_x11_native_return(&fixture)?;
            // Later frames must not issue another uncompensated move after completion.
            for _ in 0..4 {
                fixture.app.update();
            }
            assert_x11_native_return(&fixture)?;
        }
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    #[test]
    fn full_screen_x11_without_binding_returns_to_exact_native_pixels_or_requests_windowed()
    -> Result<(), String> {
        for scale in [2.0, 1.75] {
            for retained in [true, false] {
                let mut fixture = x11_return_fixture(scale, false)?;
                let role =
                    super::super::primary_window_role().map_err(|error| error.to_string())?;
                assert!(
                    fixture
                        .app
                        .world()
                        .resource::<Bindings>()
                        .binding(&role)
                        .is_err()
                );
                if !retained {
                    fixture
                        .app
                        .world_mut()
                        .entity_mut(fixture.editor)
                        .remove::<WindowedPlacement>();
                }
                WindowPlacementConfiguration::toggle_full_screen(
                    fixture.app.world_mut(),
                    fixture.editor,
                );
                for _ in 0..4 {
                    fixture.app.update();
                }
                assert_eq!(fixture.window()?.mode, WindowMode::Windowed);
                assert!(
                    fixture
                        .app
                        .world()
                        .get::<WindowRestoreAttempt>(fixture.editor)
                        .is_none()
                );
                if retained {
                    assert_x11_native_return(&fixture)?;
                }
            }
        }
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    #[test]
    fn full_screen_x11_direct_return_waits_for_frame_extents_and_keeps_its_target()
    -> Result<(), String> {
        let mut fixture = x11_return_fixture(2.0, false)?;
        WindowPlacementConfiguration::await_x11_frame_extents_for_test(
            fixture.app.world_mut(),
            fixture.editor,
        );
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        for _ in 0..4 {
            fixture.app.update();
        }
        assert_eq!(fixture.window()?.mode, WindowMode::Windowed);
        assert!(
            fixture
                .app
                .world()
                .get::<WindowedPlacementRestore>(fixture.editor)
                .is_some()
        );
        let native = fixture
            .app
            .world()
            .get::<X11WindowManagerReadback>(fixture.editor)
            .ok_or("native observations missing")?;
        assert_eq!(
            native.observed,
            IVec2::ZERO,
            "a move needs known frame extents"
        );
        WindowPlacementConfiguration::observe_x11_frame_top_for_test(
            fixture.app.world_mut(),
            fixture.editor,
            X11_PHYSICAL_FRAME_TOP,
        );
        for _ in 0..4 {
            fixture.app.update();
        }
        assert_x11_native_return(&fixture)
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    #[test]
    fn full_screen_x11_missing_frame_extents_releases_return_and_resumes_capture()
    -> Result<(), String> {
        let mut fixture = x11_return_fixture(2.0, false)?;
        WindowPlacementConfiguration::await_x11_frame_extents_for_test(
            fixture.app.world_mut(),
            fixture.editor,
        );
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        for _ in 0..=X11_WINDOWED_RETURN_MAX_FRAMES {
            fixture.app.update();
        }
        assert_eq!(fixture.window()?.mode, WindowMode::Windowed);
        assert!(
            fixture
                .app
                .world()
                .get::<WindowedPlacementRestore>(fixture.editor)
                .is_none()
        );
        let user_position = IVec2::new(317, 209);
        fixture
            .app
            .world_mut()
            .get_mut::<Window>(fixture.editor)
            .ok_or("editor missing")?
            .position = WindowPosition::At(user_position);
        fixture
            .app
            .world_mut()
            .get_mut::<X11WindowManagerReadback>(fixture.editor)
            .ok_or("native observations missing")?
            .observed = user_position;
        fixture.app.update();
        let retained = fixture
            .app
            .world()
            .get::<WindowedPlacement>(fixture.editor)
            .ok_or("windowed placement missing")?;
        assert_eq!(retained.position, WindowPosition::At(user_position));
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        assert!(matches!(
            fixture.window()?.mode,
            WindowMode::BorderlessFullscreen(_)
        ));
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    #[test]
    fn full_screen_x11_failed_frame_query_applies_fallback_without_waiting() -> Result<(), String> {
        let mut fixture = x11_return_fixture(1.75, false)?;
        WindowPlacementConfiguration::fail_x11_frame_query_for_test(
            fixture.app.world_mut(),
            fixture.editor,
        );
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        assert!(
            fixture
                .app
                .world()
                .get::<WindowedPlacementRestore>(fixture.editor)
                .is_none()
        );
        assert_eq!(
            fixture.window()?.position,
            WindowPosition::At(RETAINED_PHYSICAL_POSITION)
        );
        fixture.app.update();
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        assert!(matches!(
            fixture.window()?.mode,
            WindowMode::BorderlessFullscreen(_)
        ));
        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    #[test]
    fn full_screen_x11_unconfirmed_position_releases_return_after_its_frame_limit()
    -> Result<(), String> {
        let mut fixture = x11_return_fixture(2.0, false)?;
        fixture
            .app
            .world_mut()
            .get_mut::<X11WindowManagerReadback>(fixture.editor)
            .ok_or("native observations missing")?
            .accept_requests = false;
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        for _ in 0..=X11_WINDOWED_RETURN_MAX_FRAMES {
            fixture.app.update();
        }
        assert!(
            fixture
                .app
                .world()
                .get::<WindowedPlacementRestore>(fixture.editor)
                .is_none()
        );
        let native = fixture
            .app
            .world()
            .get::<X11WindowManagerReadback>(fixture.editor)
            .ok_or("native observations missing")?;
        assert_eq!(native.requests.last(), Some(&RETAINED_PHYSICAL_POSITION));
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        assert!(matches!(
            fixture.window()?.mode,
            WindowMode::BorderlessFullscreen(_)
        ));
        Ok(())
    }

    #[test]
    fn full_screen_rejected_handoff_does_not_change_the_next_same_monitor_request()
    -> Result<(), String> {
        let mut fixture = PlacementApp::new(
            Window {
                position: WindowPosition::At(RETAINED_PHYSICAL_POSITION),
                ..default()
            },
            HashMap::new(),
            display(true)?,
        )?;
        fixture.settle()?;
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        WindowPlacementConfiguration::toggle_full_screen(fixture.app.world_mut(), fixture.editor);
        let retained = fixture
            .app
            .world()
            .get::<WindowedPlacement>(fixture.editor)
            .ok_or("retained placement missing")?
            .clone();
        let role = super::super::primary_window_role().map_err(|error| error.to_string())?;
        let (endpoint, policy) = {
            let bindings = fixture.app.world().resource::<Bindings>();
            let binding = bindings.binding(&role).map_err(|error| error.to_string())?;
            (binding.endpoint.clone(), binding.policy())
        };
        let driver = crate::window_placement_driver(fixture.app.world())
            .map_err(|error| error.to_string())?;
        let waiting = BindingAuthoring::new(
            role.clone(),
            endpoint.clone(),
            driver,
            WindowPlacementConfiguration::Established(retained.placement.clone()),
            policy,
        );
        fixture.app.world_mut().entity_mut(fixture.editor).insert(
            WindowedPlacementRestore::LeavingFullScreen(Box::new(waiting)),
        );
        fixture
            .app
            .world_mut()
            .resource_mut::<Bindings>()
            .retire(&role)
            .map_err(|error| error.to_string())?;
        advance_windowed_placement_restore(fixture.app.world_mut());
        let new_position = IVec2::new(317, 209);
        let mut next = retained.placement;
        next.position = EstablishedWindowPosition::Restorable {
            logical_offset: new_position,
        };
        hana_rigging::prelude::register_binding(
            fixture.app.world_mut(),
            BindingAuthoring::new(
                role.clone(),
                endpoint,
                driver,
                WindowPlacementConfiguration::Established(next),
                policy,
            ),
        )
        .map_err(|error| error.to_string())?;
        crate::attach_window_to_binding_role(fixture.app.world_mut(), fixture.editor, &role)
            .map_err(|error| error.to_string())?;
        assert!(fixture.settle()?, "the next placement must use the driver");
        assert_eq!(fixture.window()?.position, WindowPosition::At(new_position));
        Ok(())
    }

    #[test]
    fn full_screen_readback_keeps_windowed_geometry_on_disk_and_after_restart() -> Result<(), String>
    {
        let monitor = CurrentMonitor {
            descriptor:            crate::MonitorDescriptor::for_current_enumeration(
                1,
                2.0,
                IVec2::new(1920, 0),
                UVec2::new(3840, 2160),
            ),
            effective_window_mode: WindowMode::Windowed,
        };
        let mut world = World::new();
        world.insert_resource(Platform::MacOs);
        let mut window = Window {
            position: WindowPosition::At(IVec2::new(2120, 160)),
            ..default()
        };
        window.resolution.set_scale_factor(2.0);
        window.resolution.set(800.0, 600.0);
        let entity = world.spawn((window, PrimaryWindow, monitor)).id();
        WindowPlacementConfiguration::toggle_full_screen(&mut world, entity);
        {
            let mut full = world.get_mut::<Window>(entity).ok_or("window missing")?;
            full.resolution.set(1920.0, 1080.0);
            full.position = WindowPosition::At(IVec2::new(1920, 0));
        }
        let key = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("test-display").map_err(|error| error.to_string())?,
            },
        };
        let mut saved = EstablishedWindowPlacement::from_readback(
            world.get::<Window>(entity).ok_or("window missing")?,
            &CurrentMonitor {
                effective_window_mode: WindowMode::BorderlessFullscreen(monitor.index.selection()),
                ..monitor
            },
            ReadbackWindowPosition::Observed(IVec2::new(1920, 0)),
            Platform::MacOs,
        )
        .project(key, "test");
        world
            .get::<WindowedPlacement>(entity)
            .ok_or("return placement missing")?
            .retain_in(&mut saved);
        let role = super::super::primary_window_role().map_err(|error| error.to_string())?;
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let path = directory.path().join("windows.ron");
        assert_eq!(
            super::super::save::save_all_states(
                &path,
                &HashMap::from([(role.clone(), saved.into())])
            ),
            super::super::save::StateFileWrite::Written
        );
        let loaded = super::super::load::load_all_states(
            &path,
            &hana_rigging::prelude::RegisteredSchemes::default(),
        )
        .ok_or("saved file missing")?;
        let PersistedWindowStateDecodeOutcome::Decoded(states) = loaded else {
            return Err("saved file rejected".into());
        };
        let saved = states.get(&role).ok_or("saved editor missing")?;
        assert_eq!((saved.logical_width, saved.logical_height), (800, 600));
        assert_eq!(
            saved.position,
            super::super::PersistedPosition::MonitorOffset(IVec2::new(100, 80))
        );
        assert_eq!(
            saved.saved_window_mode,
            SavedWindowMode::BorderlessFullscreen
        );

        let mut restarted = World::new();
        let mut persisted = PersistedWindowPlacements::default();
        persisted.seed(states);
        restarted.insert_resource(persisted);
        let mut full = Window {
            mode: WindowMode::BorderlessFullscreen(monitor.index.selection()),
            ..default()
        };
        full.resolution.set_scale_factor(2.0);
        full.resolution.set(1920.0, 1080.0);
        let entity = restarted.spawn((full, monitor, PrimaryWindow)).id();
        WindowPlacementConfiguration::toggle_full_screen(&mut restarted, entity);
        let restored = restarted
            .get::<Window>(entity)
            .ok_or("restored editor missing")?;
        assert_eq!(restored.mode, WindowMode::Windowed);
        assert_eq!(restored.resolution.size(), Vec2::new(800.0, 600.0));
        assert_eq!(restored.position, WindowPosition::At(IVec2::new(2120, 160)));
        Ok(())
    }
}
