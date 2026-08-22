use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result as FormatResult;

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::reflect::ReflectDeserialize;
use bevy::reflect::ReflectSerialize;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de::Error as DeserializeError;
use thiserror::Error;

use crate::AttemptId;

/// Recovery state of one registered application role.
///
/// `RoleState` describes the work owed for a binding such as a window, camera slot, or control
/// panel key. It does not describe the device itself: one role can wait for a replacement unit
/// after the prior one departed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Component, Reflect)]
#[reflect(Component, PartialEq)]
pub enum RoleState {
    /// The role cannot start an attempt because its device is missing, contended, or gated by
    /// policy. A newly authored binding starts here because no usable hardware has been observed.
    /// `RecoveryPolicy::Retain` remains here until application code changes the binding.
    #[default]
    Waiting,
    /// The role has a present, usable unit and no configuration operation is pending.
    Ready,
    /// The provider has started an apply operation for this role.
    ///
    /// Deadline status is not stored here because polling continues after the deadline. The
    /// attempt registry calculates overdue status from this identifier and the current time.
    Applying(AttemptId),
    /// Three consecutive attempts failed, so the kernel stopped dispatching for this role.
    ///
    /// Counting attempts rather than frames is what makes this kernel work: a driver reporting
    /// arrival evidence cannot know which of its calls belong to one role's run of failures.
    ///
    /// There are two ways out and no third. An explicit
    /// `crate::Bindings::restart_after_repeated_failures` returns the role to `Self::Waiting` and
    /// clears the run outright. Otherwise the kernel waits for reacquisition: once the role's
    /// endpoint has gone unresolvable or not present and then come back, the role returns to
    /// `Self::Waiting` for exactly one more attempt, still carrying its run of failures — a success
    /// clears it, a failure stops the role again with nothing further dispatched. A wedged camera
    /// that never leaves is therefore never reopened on its own.
    StoppedAfterRepeatedFailures,
    /// The application retired this role, so later device reports cannot reactivate its binding.
    Retired,
}

/// Application-assigned handle for the work a binding performs, independent of every device that
/// may fill it.
///
/// `RoleKey` lets a window, camera slot, or control panel key retain its application identity
/// when the physical unit is unplugged and replaced. It differs from `DeviceKey`, which names a
/// particular unit and must not survive that replacement.
///
/// It is also the component that names a binding entity, so a query or the Bevy Remote Protocol can
/// read which role a binding entity stands for without consulting `crate::BindingEntities`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Component, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Component, PartialEq, Serialize, Deserialize)]
pub struct RoleKey(String);

impl RoleKey {
    /// Create a role handle that remains readable in diagnostics and retained configuration.
    ///
    /// # Errors
    ///
    /// Returns `RoleKeyError` when `value` is blank or includes control characters, either of
    /// which would make an application-assigned role ambiguous in logs or configuration.
    pub fn new(value: impl Into<String>) -> Result<Self, RoleKeyError> {
        let value = value.into();
        if value.is_empty() {
            return Err(RoleKeyError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(RoleKeyError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    /// Borrow the application-assigned role handle without attaching it to a particular device.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl Display for RoleKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult { formatter.write_str(&self.0) }
}

impl<'de> Deserialize<'de> for RoleKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(<D::Error as DeserializeError>::custom)
    }
}

/// Reason `RoleKey::new` rejected text before it could identify an application role.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RoleKeyError {
    /// An empty value cannot distinguish this role from another application binding.
    #[error("role keys must not be empty")]
    Empty,
    /// Control characters make role labels ambiguous in configuration and diagnostics.
    #[error("role keys must not contain control characters")]
    ContainsControlCharacter,
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;

    use super::RoleKey;
    use super::RoleKeyError;
    use super::RoleState;

    #[test]
    fn role_key_retains_valid_application_handle_text() {
        assert_eq!(
            RoleKey::new("primary-window").as_ref().map(RoleKey::as_str),
            Ok("primary-window")
        );
    }

    #[test]
    fn empty_role_key_returns_empty_error() {
        assert_eq!(RoleKey::new(""), Err(RoleKeyError::Empty));
    }

    #[test]
    fn role_key_with_control_character_returns_error() {
        assert_eq!(
            RoleKey::new("primary\nwindow"),
            Err(RoleKeyError::ContainsControlCharacter)
        );
    }

    #[test]
    fn role_state_registers_component_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();
        let type_id = TypeId::of::<RoleState>();

        assert!(type_registry.contains(type_id));
        assert!(
            type_registry
                .get_type_data::<ReflectComponent>(type_id)
                .is_some()
        );

        drop(type_registry);
    }
}
