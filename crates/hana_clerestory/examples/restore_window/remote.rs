use bevy::prelude::AppExit;
use bevy::prelude::Entity;
use bevy::prelude::In;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::window::PrimaryWindow;
use bevy_remote::BrpResult;
use bevy_remote::RemotePlugin;
use bevy_remote::http::RemoteHttpPlugin;
use hana_clerestory::Monitors;
use serde::Serialize;
use serde_json::Value;

use super::constants::TEST_HTTP_PORT_ENVIRONMENT_VARIABLE;
use super::constants::TEST_MONITOR_SNAPSHOT_METHOD;
use super::constants::TEST_SHUTDOWN_METHOD;
use super::constants::TEST_WINDOW_SNAPSHOT_METHOD;

const DEFAULT_HTTP_PORT: u16 = 15702;

pub(super) fn plugin() -> RemotePlugin {
    RemotePlugin::default()
        .with_method_main(TEST_SHUTDOWN_METHOD, shutdown)
        .with_method_main(TEST_MONITOR_SNAPSHOT_METHOD, monitor_snapshot)
        .with_method_main(TEST_WINDOW_SNAPSHOT_METHOD, window_snapshot)
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum Presence {
    Present,
    Absent,
}

impl From<bool> for Presence {
    fn from(present: bool) -> Self { if present { Self::Present } else { Self::Absent } }
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum NativeFullscreen {
    Fullscreen,
    Windowed,
    Unavailable,
}

impl From<Option<bool>> for NativeFullscreen {
    fn from(state: Option<bool>) -> Self {
        match state {
            Some(true) => Self::Fullscreen,
            Some(false) => Self::Windowed,
            None => Self::Unavailable,
        }
    }
}

#[derive(Serialize)]
struct TestWindowSnapshot {
    mode:              String,
    decorated:         Presence,
    native_fullscreen: NativeFullscreen,
}

#[derive(Serialize)]
struct TestMonitorSnapshot {
    entity:                  u64,
    name:                    Option<String>,
    index:                   usize,
    scale:                   f64,
    refresh_rate_millihertz: Option<u32>,
    physical_position:       [i32; 2],
    physical_size:           [u32; 2],
}

fn monitor_snapshot(In(_params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let values: Vec<_> = world
        .resource::<Monitors>()
        .iter()
        .map(|monitor| (monitor.entity, *monitor.descriptor))
        .map(|(entity, info)| {
            let native_monitor = world.get::<bevy::window::Monitor>(entity);
            TestMonitorSnapshot {
                entity:                  entity.to_bits(),
                name:                    native_monitor.and_then(|monitor| monitor.name.clone()),
                index:                   info.index.adapter_value(),
                scale:                   info.scale,
                refresh_rate_millihertz: native_monitor
                    .and_then(|monitor| monitor.refresh_rate_millihertz),
                physical_position:       [info.physical_position.x, info.physical_position.y],
                physical_size:           [info.physical_size.x, info.physical_size.y],
            }
        })
        .collect();
    serde_json::to_value(serde_json::json!({ "monitors": values }))
        .map_err(bevy_remote::BrpError::internal)
}

fn window_snapshot(In(_params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let mut primary = world.query_filtered::<(Entity, &Window), With<PrimaryWindow>>();
    let (entity, window) = primary
        .single(world)
        .map_err(bevy_remote::BrpError::internal)?;
    serde_json::to_value(TestWindowSnapshot {
        mode:              format!("{:?}", window.mode),
        decorated:         window.decorations.into(),
        native_fullscreen: native_fullscreen(entity).into(),
    })
    .map_err(bevy_remote::BrpError::internal)
}

pub(super) fn http_plugin() -> RemoteHttpPlugin {
    let port = std::env::var(TEST_HTTP_PORT_ENVIRONMENT_VARIABLE)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_HTTP_PORT);
    RemoteHttpPlugin::default().with_port(port)
}

fn shutdown(In(_params): In<Option<Value>>, world: &mut World) -> BrpResult {
    world.write_message(AppExit::Success);
    serde_json::to_value(serde_json::json!({ "accepted": true }))
        .map_err(bevy_remote::BrpError::internal)
}

#[cfg(target_os = "macos")]
fn native_fullscreen(entity: Entity) -> Option<bool> {
    use bevy::winit::WINIT_WINDOWS;
    use objc2_app_kit::NSView;
    use objc2_app_kit::NSWindowStyleMask;
    use raw_window_handle::HasWindowHandle;
    use raw_window_handle::RawWindowHandle;

    WINIT_WINDOWS.with_borrow(|winit_windows| {
        let winit_window = winit_windows.get_window(entity)?;
        let handle = winit_window.window_handle().ok()?;
        let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: `ns_view` comes from the live winit window handle above.
        let ns_view: &NSView = unsafe { appkit_handle.ns_view.cast().as_ref() };
        let window = ns_view.window()?;
        Some(window.styleMask().contains(NSWindowStyleMask::FullScreen))
    })
}

#[cfg(not(target_os = "macos"))]
const fn native_fullscreen(_entity: Entity) -> Option<bool> { None }
