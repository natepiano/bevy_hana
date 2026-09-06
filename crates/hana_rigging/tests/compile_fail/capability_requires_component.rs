use bevy::prelude::Reflect;
use hana_rigging::Capabilities;

#[derive(PartialEq, Reflect)]
struct ReflectedAttribute;

fn main() {
    let _ = Capabilities::new().with(ReflectedAttribute);
}
