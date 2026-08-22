//! Monitor detection logic.
//!
//! Maintains `CurrentMonitor` on all managed windows using winit detection
//! with position-based fallback.

#[cfg(test)]
use std::collections::HashMap;
use std::ops::Deref;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Has;
#[cfg(test)]
use bevy::prelude::IVec2;
use bevy::prelude::Insert;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Remove;
use bevy::prelude::Res;
#[cfg(test)]
use bevy::prelude::ResMut;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use bevy::prelude::WindowPosition;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::debug;
use bevy::window::OnMonitor;
use bevy::window::PrimaryWindow;
use bevy::window::WindowMode;
use bevy::winit::WINIT_WINDOWS;
use bevy_kana::ToI32;

use super::MonitorDescriptor;
use super::Monitors;
use crate::ManagedWindow;
use crate::constants::MONITOR_SOURCE_EXISTING;
use crate::constants::MONITOR_SOURCE_FALLBACK;
use crate::constants::MONITOR_SOURCE_POSITION;
use crate::constants::MONITOR_SOURCE_WINIT;
use crate::restore::RestorePreparation;

/// Component storing the current monitor and effective window mode.
///
/// This is the single source of truth for which monitor a window is on and its
/// effective display mode. Updated automatically by the plugin's unified monitor
/// detection system.
///
/// The `effective_window_mode` field reflects what the user actually sees, even when
/// `window.mode` is stale (e.g., macOS green button fullscreen reports `Windowed`).
///
/// Derefs to [`MonitorDescriptor`] for convenient access to monitor geometry:
/// ```ignore
/// fn my_system(query: Query<(&Window, &CurrentMonitor), With<PrimaryWindow>>) {
///     let (window, monitor) = query.single();
///     println!("Monitor {} at scale {}, mode: {:?}", monitor.index, monitor.scale, monitor.effective_window_mode);
/// }
/// ```
#[derive(Component, Clone, Copy, Debug, Reflect)]
#[reflect(Component)]
#[type_path = "hana_clerestory::monitors"]
pub struct CurrentMonitor {
    /// Geometry copied from the monitor entity associated with this window.
    pub descriptor:            MonitorDescriptor,
    /// The effective window mode, accounting for OS-level fullscreen changes.
    pub effective_window_mode: WindowMode,
}

impl Deref for CurrentMonitor {
    type Target = MonitorDescriptor;

    fn deref(&self) -> &Self::Target { &self.descriptor }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowRegistration {
    Unmanaged,
    Primary,
    Managed,
    PrimaryAndManaged,
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub(crate) struct MonitorSelectionInputs {
    registration:          WindowRegistration,
    position:              WindowPosition,
    physical_size:         UVec2,
    base_scale_factor:     f32,
    scale_factor_override: Option<f32>,
    window_mode:           WindowMode,
}

impl MonitorSelectionInputs {
    fn from_window(window: &Window, registration: WindowRegistration) -> Self {
        Self {
            registration,
            position: window.position,
            physical_size: window.resolution.physical_size(),
            base_scale_factor: window.resolution.base_scale_factor(),
            scale_factor_override: window.resolution.scale_factor_override(),
            window_mode: window.mode,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NativeQueryActivity {
    window_map:       usize,
    monitor_metadata: usize,
}

#[cfg(test)]
#[derive(Default, Resource)]
pub(crate) struct InjectedCurrentMonitorSource {
    positions:   HashMap<Entity, Option<IVec2>>,
    activity:    NativeQueryActivity,
    pub lookups: usize,
}

#[cfg(test)]
impl InjectedCurrentMonitorSource {
    pub(super) fn set_position(&mut self, entity: Entity, position: Option<IVec2>) {
        self.positions.insert(entity, position);
    }

    const fn activity(&self) -> NativeQueryActivity { self.activity }

    const fn reset_activity(&mut self) {
        self.activity = NativeQueryActivity {
            window_map:       0,
            monitor_metadata: 0,
        };
        self.lookups = 0;
    }
}

pub(super) fn clear_monitor_selection_inputs(
    removed: On<Remove, (Window, PrimaryWindow, ManagedWindow)>,
    mut commands: Commands,
) {
    commands
        .entity(removed.entity)
        .try_remove::<MonitorSelectionInputs>();
}

/// Install the exact monitor observation when Bevy inserts or replaces [`OnMonitor`].
pub(crate) fn install_current_monitor_from_association(
    insert: On<Insert, OnMonitor>,
    windows: Query<
        (
            &Window,
            &OnMonitor,
            Option<&CurrentMonitor>,
            Has<PrimaryWindow>,
            Has<ManagedWindow>,
        ),
        Or<(With<PrimaryWindow>, With<ManagedWindow>)>,
    >,
    monitors: Res<Monitors>,
    mut commands: Commands,
) {
    let Ok((window, on_monitor, existing, primary, managed)) = windows.get(insert.entity) else {
        return;
    };
    let Some(current_monitor) = current_monitor_from_association(window, on_monitor, &monitors)
    else {
        return;
    };
    if !current_monitor_changed(existing, &current_monitor) {
        return;
    }

    debug!(
        "[install_current_monitor_from_association] monitor_entity={:?} index={} scale={} effective_window_mode={:?}",
        on_monitor.0,
        current_monitor.index,
        current_monitor.scale,
        current_monitor.effective_window_mode,
    );
    let registration = match (primary, managed) {
        (true, false) => WindowRegistration::Primary,
        (false, true) => WindowRegistration::Managed,
        (true, true) => WindowRegistration::PrimaryAndManaged,
        (false, false) => WindowRegistration::Unmanaged,
    };
    commands.entity(insert.entity).insert((
        current_monitor,
        MonitorSelectionInputs::from_window(window, registration),
    ));
}

pub(crate) fn current_monitor_from_association(
    window: &Window,
    on_monitor: &OnMonitor,
    monitors: &Monitors,
) -> Option<CurrentMonitor> {
    let descriptor = monitors
        .iter()
        .find(|monitor| monitor.entity == on_monitor.0)
        .map(|monitor| *monitor.descriptor)?;
    Some(CurrentMonitor {
        descriptor,
        effective_window_mode: compute_effective_window_mode(window, &descriptor, monitors),
    })
}

pub(crate) fn exact_monitor_association(
    on_monitor: &OnMonitor,
    current_monitor: &CurrentMonitor,
    monitors: &Monitors,
) -> Option<MonitorDescriptor> {
    let installed = monitors
        .iter()
        .find(|monitor| monitor.entity == on_monitor.0)
        .map(|monitor| *monitor.descriptor)?;
    (current_monitor.descriptor == installed).then_some(installed)
}

pub(super) fn remove_current_monitors_for_empty_topology(world: &mut World) {
    let mut query = world.query_filtered::<Entity, (
        With<CurrentMonitor>,
        Or<(With<PrimaryWindow>, With<ManagedWindow>)>,
    )>();
    let entities: Vec<_> = query.iter(world).collect();
    for entity in entities {
        world.entity_mut(entity).remove::<CurrentMonitor>();
    }
}

/// Unified monitor detection system. Maintains `CurrentMonitor` on all managed windows.
///
/// Detection priority:
/// 1. Bevy's [`OnMonitor`] association — exact even when registration arrives afterward
/// 2. winit's `current_monitor()` — works before `window.position` is set
/// 3. Position-based center-point detection — uses `window.position` when available
/// 4. Existing `CurrentMonitor` value — preserves last-known monitor during transient states
/// 5. `monitors.first()` — last resort fallback
///
/// All platforms: computes `effective_window_mode` (handles macOS green button fullscreen).
pub(crate) fn update_current_monitor(
    mut commands: Commands,
    windows: Query<
        (
            Entity,
            &Window,
            Option<&CurrentMonitor>,
            Option<&MonitorSelectionInputs>,
            Option<&OnMonitor>,
            Has<PrimaryWindow>,
            Has<ManagedWindow>,
            Has<RestorePreparation>,
        ),
        Or<(With<PrimaryWindow>, With<ManagedWindow>)>,
    >,
    monitors: Res<Monitors>,
    #[cfg(test)] mut injected_source: Option<ResMut<InjectedCurrentMonitorSource>>,
    _: NonSendMarker,
) {
    if monitors.is_empty() {
        return;
    }

    let topology_changed = monitors.is_changed();
    for (entity, window, existing, previous_inputs, on_monitor, primary, managed, restoring) in
        &windows
    {
        if restoring {
            continue;
        }

        let registration = match (primary, managed) {
            (true, false) => WindowRegistration::Primary,
            (false, true) => WindowRegistration::Managed,
            (true, true) => WindowRegistration::PrimaryAndManaged,
            (false, false) => WindowRegistration::Unmanaged,
        };
        let current_inputs = MonitorSelectionInputs::from_window(window, registration);
        let exact_association =
            on_monitor
                .zip(existing)
                .is_some_and(|(on_monitor, current_monitor)| {
                    exact_monitor_association(on_monitor, current_monitor, &monitors).is_some()
                });
        if existing.is_some()
            && previous_inputs == Some(&current_inputs)
            && (!topology_changed || exact_association)
        {
            continue;
        }

        let association_result = on_monitor
            .and_then(|on_monitor| current_monitor_from_association(window, on_monitor, &monitors))
            .map(|current_monitor| current_monitor.descriptor);
        let winit_result = association_result.or_else(|| {
            winit_detect_monitor(
                entity,
                &monitors,
                #[cfg(test)]
                injected_source.as_deref_mut(),
            )
        });
        let position_result = if winit_result.is_none() {
            position_detect_monitor(window, &monitors)
        } else {
            None
        };
        let existing_result =
            existing.and_then(|current_monitor| monitor_from_existing(current_monitor, &monitors));

        let (descriptor, source) = match (winit_result, position_result, existing_result) {
            (Some(descriptor), _, _) => (descriptor, MONITOR_SOURCE_WINIT),
            (_, Some(descriptor), _) => (descriptor, MONITOR_SOURCE_POSITION),
            (_, _, Some(descriptor)) => (descriptor, MONITOR_SOURCE_EXISTING),
            _ => (*monitors.first(), MONITOR_SOURCE_FALLBACK),
        };

        let effective_window_mode = compute_effective_window_mode(window, &descriptor, &monitors);

        let current_monitor = CurrentMonitor {
            descriptor,
            effective_window_mode,
        };
        // `changed` prevents redundant `CurrentMonitor` inserts and Bevy change detection.
        let changed = current_monitor_changed(existing, &current_monitor);

        let mut entity_commands = commands.entity(entity);
        if changed {
            debug!(
                "[update_current_monitor] source={} index={} scale={} effective_window_mode={:?}",
                source, descriptor.index, descriptor.scale, effective_window_mode
            );
            entity_commands.insert(current_monitor);
        }
        entity_commands.insert(current_inputs);
    }
}

fn current_monitor_changed(existing: Option<&CurrentMonitor>, current: &CurrentMonitor) -> bool {
    existing.is_none_or(|existing| {
        existing.descriptor != current.descriptor
            || existing.effective_window_mode != current.effective_window_mode
    })
}

fn monitor_from_existing(
    current_monitor: &CurrentMonitor,
    monitors: &Monitors,
) -> Option<MonitorDescriptor> {
    monitors
        .iter()
        .find(|monitor| monitor.descriptor == &current_monitor.descriptor)
        .map(|monitor| *monitor.descriptor)
}

/// Detect monitor via winit's `current_monitor()`.
fn winit_detect_monitor(
    entity: Entity,
    monitors: &Monitors,
    #[cfg(test)] injected_source: Option<&mut InjectedCurrentMonitorSource>,
) -> Option<MonitorDescriptor> {
    #[cfg(test)]
    if let Some(source) = injected_source {
        source.activity.window_map += 1;
        source.lookups += 1;
        let current_monitor_position = source.positions.get(&entity).copied()?;
        source.activity.monitor_metadata += 1;
        return current_monitor_position
            .and_then(|position| monitors.at(position.x, position.y))
            .copied();
    }

    WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        winit_windows.get_window(entity).and_then(|winit_window| {
            winit_window.current_monitor().and_then(|current_monitor| {
                let physical_position = current_monitor.position();
                monitors
                    .at(physical_position.x, physical_position.y)
                    .copied()
            })
        })
    })
}

/// Detect monitor from `window.position` using center-point logic.
fn position_detect_monitor(window: &Window, monitors: &Monitors) -> Option<MonitorDescriptor> {
    if let WindowPosition::At(physical_position) = window.position {
        Some(*monitors.monitor_for_window(
            physical_position,
            window.physical_width(),
            window.physical_height(),
        ))
    } else {
        None
    }
}

/// Compute the effective window mode, including macOS green button detection.
///
/// On macOS, clicking the green "maximize" button fills the screen but `window.mode`
/// remains `Windowed`. This detects that case and returns `BorderlessFullscreen`.
fn compute_effective_window_mode(
    window: &Window,
    descriptor: &MonitorDescriptor,
    monitors: &Monitors,
) -> WindowMode {
    // `WindowMode::Fullscreen` stays authoritative because the OS controls exclusive fullscreen.
    if matches!(window.mode, WindowMode::Fullscreen(_, _)) {
        return window.mode;
    }

    // An empty `Monitors` resource leaves no descriptor for fullscreen inference.
    if monitors.is_empty() {
        return window.mode;
    }

    // `WindowPosition::Automatic` leaves no physical position, so `window.mode` stays
    // authoritative.
    let WindowPosition::At(physical_position) = window.position else {
        return window.mode;
    };

    // `full_width`, `left_aligned`, and `reaches_bottom` model macOS fullscreen detection.
    let full_width = window.physical_width() == descriptor.physical_size.x;
    let left_aligned = physical_position.x == descriptor.physical_position.x;
    let reaches_bottom = physical_position.y + window.physical_height().to_i32()
        == descriptor.physical_position.y + descriptor.physical_size.y.to_i32();

    if full_width && left_aligned && reaches_bottom {
        WindowMode::BorderlessFullscreen(descriptor.index.selection())
    } else {
        WindowMode::Windowed
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::IVec2;
    use bevy::prelude::Update;
    use bevy::window::MonitorSelection;
    use bevy::window::VideoModeSelection;
    use bevy::window::WindowMode;
    use bevy::window::WindowPosition;

    use super::*;
    fn monitor_0() -> MonitorDescriptor {
        MonitorDescriptor::for_current_enumeration(0, 2.0, IVec2::ZERO, UVec2::new(3456, 2234))
    }

    fn monitors_with(descriptor: MonitorDescriptor) -> Monitors {
        Monitors::from_test_monitors([(Entity::from_bits(1), descriptor)])
    }

    fn window_at(physical_position: IVec2, physical_width: u32, physical_height: u32) -> Window {
        let mut window = Window {
            position: WindowPosition::At(physical_position),
            mode: WindowMode::Windowed,
            ..Default::default()
        };
        window
            .resolution
            .set_physical_resolution(physical_width, physical_height);
        window
    }

    #[test]
    fn effective_window_mode_fullscreen_when_window_fills_monitor() {
        let descriptor = monitor_0();
        let monitors = monitors_with(descriptor);
        let window = window_at(
            descriptor.physical_position,
            descriptor.physical_size.x,
            descriptor.physical_size.y,
        );

        let effective_window_mode = compute_effective_window_mode(&window, &descriptor, &monitors);
        assert_eq!(
            effective_window_mode,
            WindowMode::BorderlessFullscreen(MonitorSelection::Index(0))
        );
    }

    #[test]
    fn effective_window_mode_windowed_when_window_smaller_than_monitor() {
        let descriptor = monitor_0();
        let monitors = monitors_with(descriptor);
        let window = window_at(IVec2::new(100, 100), 1600, 1200);

        let effective_window_mode = compute_effective_window_mode(&window, &descriptor, &monitors);
        assert_eq!(effective_window_mode, WindowMode::Windowed);
    }

    #[test]
    fn effective_window_mode_windowed_when_not_left_aligned() {
        let descriptor = monitor_0();
        let monitors = monitors_with(descriptor);
        // This `window_at` value sets `full_width` and `reaches_bottom` while
        // keeping `left_aligned` false.
        let window = window_at(
            IVec2::new(1, 0),
            descriptor.physical_size.x,
            descriptor.physical_size.y,
        );

        let effective_window_mode = compute_effective_window_mode(&window, &descriptor, &monitors);
        assert_eq!(effective_window_mode, WindowMode::Windowed);
    }

    #[test]
    fn effective_window_mode_trusts_exclusive_fullscreen() {
        let descriptor = monitor_0();
        let monitors = monitors_with(descriptor);
        let mut window = window_at(IVec2::ZERO, 800, 600);
        window.mode =
            WindowMode::Fullscreen(MonitorSelection::Index(0), VideoModeSelection::Current);

        let effective_window_mode = compute_effective_window_mode(&window, &descriptor, &monitors);
        assert!(matches!(
            effective_window_mode,
            WindowMode::Fullscreen(_, _)
        ));
    }

    #[test]
    fn effective_window_mode_returns_mode_when_no_position() {
        let descriptor = monitor_0();
        let monitors = monitors_with(descriptor);
        let mut window = Window::default();
        window
            .resolution
            .set_physical_resolution(descriptor.physical_size.x, descriptor.physical_size.y);
        // `WindowPosition::Automatic` leaves no position for `compute_effective_window_mode`.

        let effective_window_mode = compute_effective_window_mode(&window, &descriptor, &monitors);
        assert_eq!(effective_window_mode, WindowMode::Windowed);
    }

    #[test]
    fn effective_window_mode_returns_mode_when_no_monitors() {
        let descriptor = monitor_0();
        let empty_monitors = Monitors::from_test_monitors([]);
        let window = window_at(
            IVec2::ZERO,
            descriptor.physical_size.x,
            descriptor.physical_size.y,
        );

        let effective_window_mode =
            compute_effective_window_mode(&window, &descriptor, &empty_monitors);
        assert_eq!(effective_window_mode, WindowMode::Windowed);
    }

    #[test]
    fn geometry_change_updates_current_monitor_at_same_index() {
        let descriptor = monitor_0();
        let existing = CurrentMonitor {
            descriptor:            MonitorDescriptor {
                scale: 1.0,
                ..descriptor
            },
            effective_window_mode: WindowMode::Windowed,
        };
        let updated = CurrentMonitor {
            descriptor,
            effective_window_mode: WindowMode::Windowed,
        };

        assert!(current_monitor_changed(Some(&existing), &updated));
    }

    #[test]
    fn removed_current_monitor_is_reinstalled_once() {
        let mut app = App::new();
        app.insert_resource(monitors_with(monitor_0()))
            .init_resource::<InjectedCurrentMonitorSource>()
            .add_systems(Update, update_current_monitor);
        let entity = app
            .world_mut()
            .spawn((window_at(IVec2::ZERO, 800, 600), PrimaryWindow))
            .id();
        app.world_mut()
            .resource_mut::<InjectedCurrentMonitorSource>()
            .set_position(entity, Some(IVec2::ZERO));
        app.update();
        assert!(app.world().entity(entity).contains::<CurrentMonitor>());
        app.world_mut()
            .entity_mut(entity)
            .remove::<CurrentMonitor>();
        app.world_mut()
            .resource_mut::<InjectedCurrentMonitorSource>()
            .reset_activity();

        app.update();

        assert!(app.world().entity(entity).contains::<CurrentMonitor>());
        assert_eq!(
            app.world()
                .resource::<InjectedCurrentMonitorSource>()
                .activity(),
            NativeQueryActivity {
                window_map:       1,
                monitor_metadata: 1,
            }
        );

        app.update();

        assert_eq!(
            app.world()
                .resource::<InjectedCurrentMonitorSource>()
                .activity(),
            NativeQueryActivity {
                window_map:       1,
                monitor_metadata: 1,
            }
        );
    }

    #[test]
    fn removing_window_registration_clears_monitor_selection_inputs() {
        let mut app = App::new();
        app.add_observer(clear_monitor_selection_inputs);
        let window = Window::default();
        let inputs = MonitorSelectionInputs::from_window(&window, WindowRegistration::Primary);
        let entity = app.world_mut().spawn((window, PrimaryWindow, inputs)).id();

        app.world_mut().entity_mut(entity).remove::<PrimaryWindow>();
        app.world_mut().flush();

        assert!(
            app.world()
                .entity(entity)
                .get::<MonitorSelectionInputs>()
                .is_none()
        );
    }
}
