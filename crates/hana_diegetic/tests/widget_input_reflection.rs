//! Public-client coverage for widget input binding reflection.
#![allow(clippy::panic, reason = "tests should panic on invalid fixtures")]

use std::any::TypeId;

use bevy::prelude::App;
use bevy::prelude::AppTypeRegistry;
use bevy::prelude::KeyCode;
use bevy::prelude::MinimalPlugins;
use bevy::reflect::FromReflect;
use bevy::reflect::PartialReflect;
use bevy::reflect::serde::TypedReflectDeserializer;
use bevy::reflect::serde::TypedReflectSerializer;
use bevy_enhanced_input::prelude::Binding;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::WidgetInputBindings;
use hana_diegetic::WidgetInputMode;
use hana_diegetic::WidgetInputPlugin;
use serde::de::DeserializeSeed;

#[test]
fn custom_widget_input_bindings_round_trip_through_reflected_json() {
    let built = WidgetInputBindings::builder()
        .next(KeyCode::Tab)
        .previous(KeyCode::KeyZ)
        .first(KeyCode::Home)
        .last(KeyCode::End)
        .activate(KeyCode::Enter)
        .cancel(KeyCode::Escape)
        .step_left(KeyCode::ArrowLeft)
        .step_left(KeyCode::KeyA)
        .step_up(KeyCode::ArrowUp)
        .step_down(KeyCode::ArrowDown)
        .build();
    let Ok(bindings) = built else {
        panic!("custom widget bindings should be valid: {built:?}")
    };
    let original = WidgetInputMode::Bindings(bindings);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, HeadlessDiegeticUiPlugin, WidgetInputPlugin));
    let registry = app.world().resource::<AppTypeRegistry>().read();
    let Some(registration) = registry.get(TypeId::of::<WidgetInputMode>()) else {
        panic!("WidgetInputMode should be registered by the widget plugins")
    };
    let serialized = serde_json::to_string(&TypedReflectSerializer::new(
        original.as_partial_reflect(),
        &registry,
    ));
    let Ok(json) = serialized else {
        panic!("widget bindings should serialize to JSON: {serialized:?}")
    };
    let mut reader = serde_json::Deserializer::from_str(&json);
    let reflected = TypedReflectDeserializer::new(registration, &registry).deserialize(&mut reader);
    drop(registry);
    let Ok(reflected) = reflected else {
        panic!("widget bindings should deserialize from JSON: {reflected:?}")
    };
    let Some(restored) = WidgetInputMode::from_reflect(reflected.as_partial_reflect()) else {
        panic!("reflected widget bindings should convert to WidgetInputMode")
    };

    assert_eq!(restored, original);
    assert_eq!(restored.control_summary(), original.control_summary());
    assert_eq!(
        restored.control_summary().step_left,
        [
            Binding::from(KeyCode::ArrowLeft).to_string(),
            Binding::from(KeyCode::KeyA).to_string(),
        ]
    );
    assert_eq!(restored.control_summary().step_right.len(), 0);
    assert_eq!(
        restored.control_summary().step_up,
        [Binding::from(KeyCode::ArrowUp).to_string()]
    );
    assert_eq!(
        restored.control_summary().step_down,
        [Binding::from(KeyCode::ArrowDown).to_string()]
    );
}
