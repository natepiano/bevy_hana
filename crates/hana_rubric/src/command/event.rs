/// Generates a Bevy `Event` struct with `Reflect` support.
///
/// Supports both unit events and events with payload fields.
/// Events generated this way are compatible with the Bevy Remote Protocol's
/// `world.trigger_event`.
///
/// # Examples
///
/// Unit event:
///
/// ```ignore
/// use hana_rubric::event;
///
/// event!(PauseEvent);
/// ```
///
/// Payload event:
///
/// ```ignore
/// use hana_rubric::event;
///
/// event!(ZoomToTarget { entity: Entity });
/// ```
///
/// Event with additional reflected trait data:
///
/// ```ignore
/// use hana_rubric::event;
///
/// event!(PauseEvent, reflect: PauseMetadata);
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! event {
    ($(#[$meta:meta])* $event:ident, reflect: $($extra:ident),+ $(,)?) => {
        $(#[$meta])*
        #[derive(Event, Reflect, Default)]
        #[reflect(Event, $($extra),+)]
        pub struct $event;
    };
    ($(#[$meta:meta])* $event:ident { $($field:ident : $ty:ty),+ $(,)? }, reflect: $($extra:ident),+ $(,)?) => {
        $(#[$meta])*
        #[derive(Event, Reflect)]
        #[reflect(Event, $($extra),+)]
        pub struct $event {
            $(pub $field: $ty,)+
        }
    };
    ($(#[$meta:meta])* $event:ident) => {
        $(#[$meta])*
        #[derive(Event, Reflect, Default)]
        #[reflect(Event)]
        pub struct $event;
    };
    ($(#[$meta:meta])* $event:ident { $($field:ident : $ty:ty),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Event, Reflect)]
        #[reflect(Event)]
        pub struct $event {
            $(pub $field: $ty,)+
        }
    };
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::mem::size_of_val;

    use bevy::prelude::Event;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy::reflect::TypeRegistry;
    use bevy::reflect::reflect_trait;

    // expectations
    const EMPTY_EVENT_SIZE: usize = 0;
    const PAYLOAD_VALUE: u32 = 42;

    #[reflect_trait]
    trait TestEventMetadata {}

    crate::event!(TestEvent);
    crate::event!(TestPayloadEvent { value: u32 });
    crate::event!(TestReflectedEvent, reflect: TestEventMetadata);
    crate::event!(TestReflectedPayloadEvent { value: u32 }, reflect: TestEventMetadata);

    impl TestEventMetadata for TestReflectedEvent {}

    impl TestEventMetadata for TestReflectedPayloadEvent {}

    #[test]
    fn payload_event_fields() {
        let test_payload_event = TestPayloadEvent {
            value: PAYLOAD_VALUE,
        };
        assert_eq!(test_payload_event.value, PAYLOAD_VALUE);
    }

    #[test]
    fn unit_event_defaults() {
        let test_event = TestEvent;
        assert_eq!(size_of_val(&test_event), EMPTY_EVENT_SIZE);
    }

    #[test]
    fn reflected_events_register_extra_type_data() {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<TestReflectedEvent>();
        type_registry.register::<TestReflectedPayloadEvent>();

        assert!(
            type_registry
                .get_type_data::<ReflectTestEventMetadata>(TypeId::of::<TestReflectedEvent>())
                .is_some()
        );
        assert!(type_registry
            .get_type_data::<ReflectTestEventMetadata>(TypeId::of::<TestReflectedPayloadEvent>())
            .is_some());
    }
}
