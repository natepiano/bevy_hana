use bevy::prelude::Reflect;
use hana_rigging::BindingAuthoring;
use hana_rigging::BindingPolicy;
use hana_rigging::DeviceEndpoint;
use hana_rigging::EndpointDriverRegistration;
use hana_rigging::RoleKey;

#[derive(Reflect)]
struct WindowPlacement;

#[derive(Reflect)]
struct CameraSettings;

fn author_camera_settings_with_window_driver(
    role: RoleKey,
    endpoint: DeviceEndpoint,
    driver: EndpointDriverRegistration<WindowPlacement>,
    policy: BindingPolicy,
) {
    let _ = BindingAuthoring::new(role, endpoint, driver, CameraSettings, policy);
}

fn main() {}
