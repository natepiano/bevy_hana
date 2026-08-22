//! Public API events for window restoration.

use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::IVec2;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::UVec2;
use bevy::window::WindowMode;
use hana_rigging::prelude::RoleKey;

/// A physical window position read back from the active platform.
///
/// A compositor that cannot report position is not the same as an observed origin at `(0, 0)`;
/// this type keeps that boundary visible to restore diagnostics and remote observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum ObservedPhysicalPosition {
    /// The platform reported this physical-pixel origin for the window.
    Observed(IVec2),
    /// The active platform cannot provide physical window coordinates.
    PlatformCannotReport,
}

/// A physical window position that Clerestory asked the platform to reach.
///
/// The variants retain why an expected coordinate does not exist, rather than implying that every
/// missing value is a platform failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum ExpectedPhysicalPosition {
    /// A saved placement and a live monitor produced this physical-pixel target.
    Specified(IVec2),
    /// The compositor owns window placement and cannot accept a physical target.
    PlatformCannotPosition,
    /// The saved window record did not contain a position to apply.
    NotSaved,
    /// A legacy absolute coordinate could not safely be rebased and was discarded.
    DiscardedLegacy,
}

/// A logical window position that Clerestory expected after restoring saved state.
///
/// This is distinct from [`ExpectedPhysicalPosition`] because a logical origin can be absent for
/// saved-state reasons even when the current platform can report physical coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum ExpectedLogicalPosition {
    /// A saved monitor-relative offset produced this logical desktop coordinate.
    Specified(IVec2),
    /// The compositor owns window placement and cannot accept a logical target.
    PlatformCannotPosition,
    /// The saved window record did not contain a logical position.
    NotSaved,
    /// A legacy absolute coordinate could not safely be rebased and was discarded.
    DiscardedLegacy,
}

/// A logical window position derived from a live position readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum ObservedLogicalPosition {
    /// The platform reported a physical position and its current scale produced this coordinate.
    Observed(IVec2),
    /// The active platform cannot provide the physical position needed for this derivation.
    PlatformCannotReport,
}

/// Event fired when a window restore completes and the window becomes visible.
///
/// This is an [`EntityEvent`] triggered on the window entity at the end of the restore
/// process, after position, size, and window mode have been applied. Dependent crates can
/// observe this event to know the final restored window state.
///
/// Use an observer to receive this event:
/// ```ignore
/// // For all windows
/// app.add_observer(|trigger: On<WindowRestored>| {
///     let event = trigger.event();
///     // Use `event.entity`, `event.physical_size`, `event.window_mode`, etc.
/// });
///
/// // For primary window only - check event.entity against PrimaryWindow query
/// fn on_window_restored(
///     trigger: On<WindowRestored>,
///     primary_window: Query<(), With<PrimaryWindow>>,
/// ) {
///     let event = trigger.event();
///     if primary_window.get(event.entity).is_ok() {
///         // Handle primary window only
///     }
/// }
/// ```
#[derive(EntityEvent, Debug, Clone, Reflect)]
#[reflect(Event)]
pub struct WindowRestored {
    /// The window entity this event targets.
    pub entity:            Entity,
    /// Kernel role whose window driver completed the request.
    pub role:              RoleKey,
    /// Physical position Clerestory asked the platform to reach.
    pub physical_position: ExpectedPhysicalPosition,
    /// Target position in logical pixels, as the desktop numbers it at the target monitor's
    /// live scale.
    ///
    /// Logical position Clerestory expected after applying the saved placement.
    pub logical_position:  ExpectedLogicalPosition,
    /// Target physical size that was applied (content area).
    pub physical_size:     UVec2,
    /// Target logical size that was applied (content area).
    pub logical_size:      UVec2,
    /// Window mode that was applied.
    pub window_mode:       WindowMode,
    /// Monitor index the window was restored to.
    pub monitor_index:     usize,
}

/// Event fired when the actual window state doesn't match what was requested.
///
/// After `try_apply_restore` completes, the library compares the intended restore
/// target against the live window state. If any field differs, this event fires
/// instead of [`WindowRestored`].
///
/// ## Sources
///
/// **Expected values** come from `TargetPosition`, which is computed
/// from the saved RON state file at startup. These represent what the restore *intended* to
/// achieve.
///
/// **Actual values** come from two live ECS sources, each chosen for accuracy:
///
/// - **`monitor_index`** → [`CurrentMonitor`](crate::CurrentMonitor) component, maintained by
///   `update_current_monitor`, which queries winit's `current_monitor()` and maps it to the
///   `Monitors` list. This updates quickly when the compositor moves the window.
///
/// - **`physical_position`, `logical_position`, `physical_size`, `logical_size`, `window_mode`,
///   `scale`** → the [`Window`](bevy::window::Window) component. Position and size reflect
///   `Window.position` / `Window.resolution`, and scale comes from
///   `Window.resolution.scale_factor()`. These lag behind the compositor because they only update
///   when winit fires corresponding events (`ScaleFactorChanged`, `Resized`, `Moved`). A common
///   mismatch is the scale factor still reflecting the launch monitor while `CurrentMonitor` has
///   already updated to the target monitor.
///
/// This intentional split means a mismatch signals that the window hasn't fully settled
/// — the compositor accepted the request but winit hasn't yet delivered all the
/// resulting state changes.
///
/// ## Field layout
///
/// The `expected_*` / `actual_*` pairs are deliberately flat rather than grouped into
/// nested comparison structs — the event is primarily consumed via reflection (BRP /
/// observers), where flat fields are easier to address than nested ones. The
/// `restore_window` example adapts this flat shape into nested `*Mismatch` types in
/// `examples/restore_window/events.rs`; any future reshape of the fields here must
/// update that adapter in tandem.
#[derive(EntityEvent, Debug, Clone, Reflect)]
#[reflect(Event)]
pub struct WindowRestoreMismatch {
    /// The window entity this event targets.
    pub entity:                     Entity,
    /// Kernel role whose window driver reported the mismatch.
    pub role:                       RoleKey,
    /// Physical position from the target preparation.
    pub expected_physical_position: ExpectedPhysicalPosition,
    /// Physical position reported by `Window.position` after the apply request.
    pub actual_physical_position:   ObservedPhysicalPosition,
    /// Logical position from the saved state and target preparation.
    pub expected_logical_position:  ExpectedLogicalPosition,
    /// Logical position derived from the platform's physical readback and live scale.
    pub actual_logical_position:    ObservedLogicalPosition,
    /// Target physical size from `TargetPosition`.
    pub expected_physical_size:     UVec2,
    /// Actual physical size from `Window.resolution`.
    pub actual_physical_size:       UVec2,
    /// Expected logical size from `TargetPosition`.
    pub expected_logical_size:      UVec2,
    /// Actual logical size from `Window.resolution.width()`/`height()`.
    pub actual_logical_size:        UVec2,
    /// Target window mode from `TargetPosition`.
    pub expected_window_mode:       WindowMode,
    /// Actual window mode from `Window.mode`.
    pub actual_window_mode:         WindowMode,
    /// Target monitor index from `TargetPosition`.
    pub expected_monitor:           usize,
    /// Actual monitor index from `CurrentMonitor` (winit `current_monitor()`).
    pub actual_monitor:             usize,
    /// Target scale factor from `TargetPosition.target_scale`.
    pub expected_scale:             f64,
    /// Actual scale factor from `Window.resolution.scale_factor()`.
    /// Lags behind monitor changes; updates only on winit `ScaleFactorChanged`.
    pub actual_scale:               f64,
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectEvent;
    use bevy::ecs::system::In;
    use bevy::prelude::App;
    use bevy::prelude::Event;
    use bevy::reflect::TypePath;
    use bevy::reflect::TypeRegistry;
    use bevy_remote::builtin_methods::process_remote_observe_watching_request;
    use serde_json::Value;
    use serde_json::json;

    use super::*;
    use crate::monitors::MonitorConnected;
    use crate::monitors::MonitorDisconnected;

    fn has_reflect_event<T: 'static>(registry: &TypeRegistry) -> bool {
        registry
            .get(TypeId::of::<T>())
            .and_then(|registration| registration.data::<ReflectEvent>())
            .is_some()
    }

    fn observe_event<T>(app: &mut App, event: T) -> Value
    where
        T: Event + Reflect + TypePath,
        for<'a> T::Trigger<'a>: Default,
    {
        let observe_params = json!({
            "event": T::type_path(),
            "entity": null,
        });
        assert_eq!(
            process_remote_observe_watching_request(
                In(Some(observe_params.clone())),
                app.world_mut(),
            ),
            Ok(None)
        );

        app.world_mut().trigger(event);

        let observed =
            process_remote_observe_watching_request(In(Some(observe_params)), app.world_mut());
        assert!(matches!(observed, Ok(Some(_))), "observed: {observed:?}");
        match observed {
            Ok(Some(observed)) => {
                let events = observed.as_array();
                assert_eq!(events.map(Vec::len), Some(1));
                events
                    .and_then(|events| events.first())
                    .cloned()
                    .unwrap_or(Value::Null)
            },
            Ok(None) | Err(_) => Value::Null,
        }
    }

    fn assert_public_fields(event: &Value, expected: &[&str]) {
        let fields = event.as_object();
        assert_eq!(fields.map(serde_json::Map::len), Some(expected.len()));
        assert!(
            expected
                .iter()
                .all(|field| fields.is_some_and(|fields| fields.contains_key(*field)))
        );
    }
    #[test]
    fn public_window_events_auto_register_reflected_event_type_data() {
        let app = App::new();
        let registry = app.world().resource::<AppTypeRegistry>().read();

        assert!(has_reflect_event::<WindowRestored>(&registry));
        assert!(has_reflect_event::<WindowRestoreMismatch>(&registry));
    }

    fn assert_restore_event_observations(app: &mut App) {
        let restored_role = RoleKey::new("inspector");
        assert!(restored_role.is_ok());
        let Ok(restored_role) = restored_role else {
            return;
        };
        let restored_entity = app.world_mut().spawn_empty().id();
        let restored = observe_event(
            app,
            WindowRestored {
                entity:            restored_entity,
                role:              restored_role,
                physical_position: ExpectedPhysicalPosition::Specified(IVec2::new(20, 40)),
                logical_position:  ExpectedLogicalPosition::Specified(IVec2::new(10, 20)),
                physical_size:     UVec2::new(1_600, 1_200),
                logical_size:      UVec2::new(800, 600),
                window_mode:       WindowMode::Windowed,
                monitor_index:     2,
            },
        );
        assert_public_fields(
            &restored,
            &[
                "entity",
                "role",
                "physical_position",
                "logical_position",
                "physical_size",
                "logical_size",
                "window_mode",
                "monitor_index",
            ],
        );
        assert_eq!(
            restored.get("entity"),
            Some(&json!(restored_entity.to_bits()))
        );
        assert_eq!(restored.get("monitor_index"), Some(&json!(2)));

        let mismatch_role = RoleKey::new("dashboard");
        assert!(mismatch_role.is_ok());
        let Ok(mismatch_role) = mismatch_role else {
            return;
        };
        let mismatch_entity = app.world_mut().spawn_empty().id();
        let mismatch = observe_event(
            app,
            WindowRestoreMismatch {
                entity:                     mismatch_entity,
                role:                       mismatch_role,
                expected_physical_position: ExpectedPhysicalPosition::Specified(IVec2::new(
                    200, 400,
                )),
                actual_physical_position:   ObservedPhysicalPosition::Observed(IVec2::new(
                    220, 440,
                )),
                expected_logical_position:  ExpectedLogicalPosition::Specified(IVec2::new(
                    100, 200,
                )),
                actual_logical_position:    ObservedLogicalPosition::Observed(IVec2::new(110, 220)),
                expected_physical_size:     UVec2::new(1_600, 1_200),
                actual_physical_size:       UVec2::new(1_920, 1_080),
                expected_logical_size:      UVec2::new(800, 600),
                actual_logical_size:        UVec2::new(960, 540),
                expected_window_mode:       WindowMode::Windowed,
                actual_window_mode:         WindowMode::Windowed,
                expected_monitor:           2,
                actual_monitor:             4,
                expected_scale:             2.0,
                actual_scale:               1.5,
            },
        );
        assert_public_fields(
            &mismatch,
            &[
                "entity",
                "role",
                "expected_physical_position",
                "actual_physical_position",
                "expected_logical_position",
                "actual_logical_position",
                "expected_physical_size",
                "actual_physical_size",
                "expected_logical_size",
                "actual_logical_size",
                "expected_window_mode",
                "actual_window_mode",
                "expected_monitor",
                "actual_monitor",
                "expected_scale",
                "actual_scale",
            ],
        );
        assert_eq!(mismatch.get("actual_monitor"), Some(&json!(4)));
    }

    fn assert_monitor_event_observations(app: &mut App) {
        let connected_entity = app.world_mut().spawn_empty().id();
        let connected = observe_event(
            app,
            MonitorConnected {
                entity: connected_entity,
            },
        );
        assert_public_fields(&connected, &["entity"]);
        assert_eq!(
            connected.get("entity"),
            Some(&json!(connected_entity.to_bits()))
        );

        let former_entity = app.world_mut().spawn_empty().id();
        let disconnected = observe_event(app, MonitorDisconnected { former_entity });
        assert_public_fields(&disconnected, &["former_entity"]);
        assert_eq!(
            disconnected.get("former_entity"),
            Some(&json!(former_entity.to_bits()))
        );
    }

    #[test]
    fn remote_observation_serializes_driver_and_monitor_events() {
        let mut app = App::new();

        assert_restore_event_observations(&mut app);
        assert_monitor_event_observations(&mut app);
    }
}
