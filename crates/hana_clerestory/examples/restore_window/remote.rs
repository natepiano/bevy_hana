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
use serde::Serializer;
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

#[cfg_attr(
    not(target_os = "macos"),
    allow(
        dead_code,
        reason = "only the macOS query can tell fullscreen from windowed"
    )
)]
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum NativeFullscreen {
    Fullscreen,
    Windowed,
    Unavailable,
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
    name:                    TestMonitorName,
    index:                   usize,
    scale:                   f64,
    refresh_rate_millihertz: TestMonitorRefreshRate,
    physical_position:       [i32; 2],
    physical_size:           [u32; 2],
}

enum TestMonitorName {
    Reported(String),
    Unavailable,
}

impl From<Option<String>> for TestMonitorName {
    fn from(name: Option<String>) -> Self { name.map_or(Self::Unavailable, Self::Reported) }
}

impl Serialize for TestMonitorName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Reported(name) => serializer.serialize_some(name),
            Self::Unavailable => serializer.serialize_none(),
        }
    }
}

enum TestMonitorRefreshRate {
    Reported(u32),
    Unavailable,
}

impl From<Option<u32>> for TestMonitorRefreshRate {
    fn from(refresh_rate: Option<u32>) -> Self {
        refresh_rate.map_or(Self::Unavailable, Self::Reported)
    }
}

impl Serialize for TestMonitorRefreshRate {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Reported(refresh_rate) => serializer.serialize_some(refresh_rate),
            Self::Unavailable => serializer.serialize_none(),
        }
    }
}

enum RemoteRequestParameters {
    Provided,
    Omitted,
}

impl From<Option<Value>> for RemoteRequestParameters {
    fn from(params: Option<Value>) -> Self { params.map_or(Self::Omitted, |_| Self::Provided) }
}

fn monitor_snapshot(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let _: RemoteRequestParameters = params.into();
    let values: Vec<_> = world
        .resource::<Monitors>()
        .iter()
        .map(|monitor| (monitor.entity, *monitor.descriptor))
        .map(|(entity, info)| {
            let native_monitor = world.get::<bevy::window::Monitor>(entity);
            TestMonitorSnapshot {
                entity:                  entity.to_bits(),
                name:                    native_monitor
                    .and_then(|monitor| monitor.name.clone())
                    .into(),
                index:                   info.index.adapter_value(),
                scale:                   info.scale,
                refresh_rate_millihertz: native_monitor
                    .and_then(|monitor| monitor.refresh_rate_millihertz)
                    .into(),
                physical_position:       [info.physical_position.x, info.physical_position.y],
                physical_size:           [info.physical_size.x, info.physical_size.y],
            }
        })
        .collect();
    serde_json::to_value(serde_json::json!({ "monitors": values }))
        .map_err(bevy_remote::BrpError::internal)
}

fn window_snapshot(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let _: RemoteRequestParameters = params.into();
    let mut primary = world.query_filtered::<(Entity, &Window), With<PrimaryWindow>>();
    let (entity, window) = primary
        .single(world)
        .map_err(bevy_remote::BrpError::internal)?;
    serde_json::to_value(TestWindowSnapshot {
        mode:              format!("{:?}", window.mode),
        decorated:         window.decorations.into(),
        native_fullscreen: native_fullscreen(entity),
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

fn shutdown(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let _: RemoteRequestParameters = params.into();
    world.write_message(AppExit::Success);
    serde_json::to_value(serde_json::json!({ "accepted": true }))
        .map_err(bevy_remote::BrpError::internal)
}

#[cfg(target_os = "macos")]
fn native_fullscreen(entity: Entity) -> NativeFullscreen {
    use bevy::winit::WINIT_WINDOWS;
    use objc2_app_kit::NSView;
    use objc2_app_kit::NSWindowStyleMask;
    use raw_window_handle::HasWindowHandle;
    use raw_window_handle::RawWindowHandle;

    WINIT_WINDOWS.with_borrow(|winit_windows| {
        let Some(winit_window) = winit_windows.get_window(entity) else {
            return NativeFullscreen::Unavailable;
        };
        let Ok(handle) = winit_window.window_handle() else {
            return NativeFullscreen::Unavailable;
        };
        let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() else {
            return NativeFullscreen::Unavailable;
        };
        // SAFETY: `ns_view` comes from the live winit window handle above.
        let ns_view: &NSView = unsafe { appkit_handle.ns_view.cast().as_ref() };
        let Some(window) = ns_view.window() else {
            return NativeFullscreen::Unavailable;
        };
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            NativeFullscreen::Fullscreen
        } else {
            NativeFullscreen::Windowed
        }
    })
}

#[cfg(not(target_os = "macos"))]
const fn native_fullscreen(_: Entity) -> NativeFullscreen { NativeFullscreen::Unavailable }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_snapshot_keeps_unavailable_fields_as_null() -> Result<(), serde_json::Error> {
        let test_monitor_snapshot = TestMonitorSnapshot {
            entity:                  1,
            name:                    TestMonitorName::Unavailable,
            index:                   0,
            scale:                   1.0,
            refresh_rate_millihertz: TestMonitorRefreshRate::Unavailable,
            physical_position:       [0, 0],
            physical_size:           [1_920, 1_080],
        };

        let value = serde_json::to_value(test_monitor_snapshot)?;
        assert!(value["name"].is_null());
        assert!(value["refresh_rate_millihertz"].is_null());
        Ok(())
    }
}
