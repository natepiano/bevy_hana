use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use hana_rigging::Capabilities;

#[derive(Component, Reflect)]
#[reflect(Component)]
struct ReflectedComponent;

fn main() {
    let _ = Capabilities::new().with(ReflectedComponent);
}
