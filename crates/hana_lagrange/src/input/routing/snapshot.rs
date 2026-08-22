//! Per-frame snapshots of windows and cameras used by the routing system.
//!
//! Types (all submodule-internal):
//! - [`WindowSnapshots`] — captured window sizes plus the single focused cursor surface.
//! - [`WindowSnapshot`] — captured window size.
//! - [`CameraRoutingSnapshot`] — captured per-camera routing inputs (entity, draw order, surface
//!   metrics, and the bit flags below).
//! - [`CameraRoutingSnapshotFlags`] — `ACTIVE`/`MANUAL`/`DISABLED`/`ANIMATION_IGNORE`/`CURSOR_HIT`
//!   bitset.
//!
//! [`collect_window_snapshots`] and [`collect_camera_snapshots`] are the entry points the
//! parent `resolve_camera_input_routing` system calls each frame.

use std::collections::HashMap;

use bevy::camera::RenderTarget;
use bevy::prelude::Camera;
use bevy::prelude::Entity;
use bevy::prelude::Rect;
use bevy::prelude::Vec2;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::World;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;

use crate::CameraInputInterruptBehavior;
use crate::FreeCam;
use crate::FreeCamKind;
use crate::OrbitCam;
use crate::OrbitCamKind;
use crate::animation::CameraSequencePlayback;
use crate::input::CameraInputDisabled;
use crate::input::CameraInputSurfaceMetrics;
use crate::input::CameraManual;

#[derive(Clone, Copy)]
pub(super) struct WindowSnapshot {
    size: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WindowKey {
    Primary,
    Entity(Entity),
}

#[derive(Clone, Copy)]
enum ActiveWindowCursor {
    None,
    Window { window: WindowKey, cursor: Vec2 },
    Ambiguous,
}

impl ActiveWindowCursor {
    const fn add(self, window: WindowKey, cursor: Vec2) -> Self {
        match self {
            Self::None => Self::Window { window, cursor },
            Self::Window { .. } | Self::Ambiguous => Self::Ambiguous,
        }
    }

    const fn cursor_for(self, window: WindowKey) -> Option<Vec2> {
        match self {
            Self::Window {
                window: active_window,
                cursor,
            } if window_key_eq(active_window, window) => Some(cursor),
            Self::None | Self::Window { .. } | Self::Ambiguous => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct WindowSnapshots {
    windows: HashMap<WindowKey, WindowSnapshot>,
    cursor:  ActiveWindowCursor,
}

impl Default for WindowSnapshots {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            cursor:  ActiveWindowCursor::None,
        }
    }
}

impl WindowSnapshots {
    fn insert(&mut self, key: WindowKey, window: &Window) {
        self.windows.insert(
            key,
            WindowSnapshot {
                size: Vec2::new(window.width(), window.height()),
            },
        );
        if window.focused
            && let Some(cursor) = window.cursor_position()
        {
            self.cursor = self.cursor.add(key, cursor);
        }
    }

    fn snapshot_for(&self, target: &RenderTarget) -> Option<&WindowSnapshot> {
        window_key(target).and_then(|key| self.windows.get(&key))
    }

    fn cursor_for(&self, target: &RenderTarget) -> Option<Vec2> {
        window_key(target).and_then(|key| self.cursor.cursor_for(key))
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(super) struct CameraRoutingSnapshotFlags: u8 {
        const ACTIVE = 1 << 0;
        const MANUAL = 1 << 1;
        const DISABLED = 1 << 2;
        const ANIMATION_IGNORE = 1 << 3;
        const CURSOR_HIT = 1 << 4;
    }
}

pub(super) struct CameraRoutingSnapshot {
    pub(super) entity:  Entity,
    pub(super) order:   isize,
    pub(super) flags:   CameraRoutingSnapshotFlags,
    pub(super) metrics: CameraInputSurfaceMetrics,
}

impl CameraRoutingSnapshot {
    pub(super) const fn has(&self, flag: CameraRoutingSnapshotFlags) -> bool {
        self.flags.contains(flag)
    }
}

struct CameraSnapshotInputs<'a> {
    entity:           Entity,
    camera:           &'a Camera,
    target:           &'a RenderTarget,
    manual:           bool,
    disabled:         Option<&'a CameraInputDisabled>,
    playback:         Option<&'a CameraSequencePlayback>,
    interrupt:        Option<&'a CameraInputInterruptBehavior>,
    explicit_metrics: Option<&'a CameraInputSurfaceMetrics>,
}

pub(super) fn collect_window_snapshots(world: &mut World) -> WindowSnapshots {
    let mut snapshots = WindowSnapshots::default();

    let mut primary_query = world.query_filtered::<&Window, With<PrimaryWindow>>();
    if let Ok(window) = primary_query.single(world) {
        snapshots.insert(WindowKey::Primary, window);
    }

    let mut other_query = world.query_filtered::<(Entity, &Window), Without<PrimaryWindow>>();
    for (entity, window) in other_query.iter(world) {
        snapshots.insert(WindowKey::Entity(entity), window);
    }

    snapshots
}

pub(super) fn collect_camera_snapshots(
    world: &mut World,
    windows: &WindowSnapshots,
) -> Vec<CameraRoutingSnapshot> {
    let mut snapshots = Vec::new();

    let mut orbit_query = world.query_filtered::<(
        Entity,
        &Camera,
        &RenderTarget,
        Option<&CameraManual<OrbitCamKind>>,
        Option<&CameraInputDisabled>,
        Option<&CameraSequencePlayback>,
        Option<&CameraInputInterruptBehavior>,
        Option<&CameraInputSurfaceMetrics>,
    ), With<OrbitCam>>();

    snapshots.extend(orbit_query.iter(world).map(
        |(entity, camera, target, manual, disabled, playback, interrupt, explicit_metrics)| {
            camera_snapshot(
                CameraSnapshotInputs {
                    entity,
                    camera,
                    target,
                    manual: manual.is_some(),
                    disabled,
                    playback,
                    interrupt,
                    explicit_metrics,
                },
                windows,
            )
        },
    ));

    let mut free_query = world.query_filtered::<(
        Entity,
        &Camera,
        &RenderTarget,
        Option<&CameraManual<FreeCamKind>>,
        Option<&CameraInputDisabled>,
        Option<&CameraSequencePlayback>,
        Option<&CameraInputInterruptBehavior>,
        Option<&CameraInputSurfaceMetrics>,
    ), (With<FreeCam>, Without<OrbitCam>)>();

    snapshots.extend(free_query.iter(world).map(
        |(entity, camera, target, manual, disabled, playback, interrupt, explicit_metrics)| {
            camera_snapshot(
                CameraSnapshotInputs {
                    entity,
                    camera,
                    target,
                    manual: manual.is_some(),
                    disabled,
                    playback,
                    interrupt,
                    explicit_metrics,
                },
                windows,
            )
        },
    ));

    snapshots
}

fn camera_snapshot(
    camera_snapshot_inputs: CameraSnapshotInputs<'_>,
    windows: &WindowSnapshots,
) -> CameraRoutingSnapshot {
    let CameraSnapshotInputs {
        entity,
        camera,
        target,
        manual,
        disabled,
        playback,
        interrupt,
        explicit_metrics,
    } = camera_snapshot_inputs;
    let window = windows.snapshot_for(target);
    let metrics = camera_input_surface_metrics(camera, window, explicit_metrics.copied());
    let cursor_hit = windows
        .cursor_for(target)
        .is_some_and(|cursor| cursor_hits_camera(cursor, camera));
    let animation = playback.is_some_and(CameraSequencePlayback::is_native_playing)
        && interrupt.copied().unwrap_or_default() == CameraInputInterruptBehavior::Ignore;
    let mut flags = CameraRoutingSnapshotFlags::empty();
    flags.set(CameraRoutingSnapshotFlags::ACTIVE, camera.is_active);
    flags.set(CameraRoutingSnapshotFlags::MANUAL, manual);
    flags.set(CameraRoutingSnapshotFlags::DISABLED, disabled.is_some());
    flags.set(CameraRoutingSnapshotFlags::ANIMATION_IGNORE, animation);
    flags.set(CameraRoutingSnapshotFlags::CURSOR_HIT, cursor_hit);

    CameraRoutingSnapshot {
        entity,
        order: camera.order,
        flags,
        metrics,
    }
}

fn camera_input_surface_metrics(
    camera: &Camera,
    window: Option<&WindowSnapshot>,
    explicit: Option<CameraInputSurfaceMetrics>,
) -> CameraInputSurfaceMetrics {
    let mut metrics = CameraInputSurfaceMetrics {
        camera_view_size:   camera.logical_viewport_size(),
        input_surface_size: window
            .map(|window| window.size)
            .or_else(|| camera.logical_viewport_size()),
    };

    if let Some(explicit) = explicit {
        if explicit.camera_view_size.is_some() {
            metrics.camera_view_size = explicit.camera_view_size;
        }
        if explicit.input_surface_size.is_some() {
            metrics.input_surface_size = explicit.input_surface_size;
        }
    }

    metrics
}

const fn window_key(target: &RenderTarget) -> Option<WindowKey> {
    let RenderTarget::Window(window_ref) = target else {
        return None;
    };

    Some(match window_ref {
        WindowRef::Primary => WindowKey::Primary,
        WindowRef::Entity(entity) => WindowKey::Entity(*entity),
    })
}

const fn window_key_eq(left: WindowKey, right: WindowKey) -> bool {
    match (left, right) {
        (WindowKey::Primary, WindowKey::Primary) => true,
        (WindowKey::Entity(left), WindowKey::Entity(right)) => left.to_bits() == right.to_bits(),
        (WindowKey::Primary, WindowKey::Entity(_)) | (WindowKey::Entity(_), WindowKey::Primary) => {
            false
        },
    }
}

fn cursor_hits_camera(cursor: Vec2, camera: &Camera) -> bool {
    camera
        .logical_viewport_rect()
        .is_some_and(|Rect { min, max }| {
            cursor.x > min.x && cursor.x < max.x && cursor.y > min.y && cursor.y < max.y
        })
}
