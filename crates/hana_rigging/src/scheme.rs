use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result as FormatResult;

use bevy::ecs::reflect::ReflectResource;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;
use bevy::reflect::ReflectDeserialize;
use bevy::reflect::ReflectSerialize;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de::Error as DeserializeError;
use thiserror::Error;

use super::identity::DeviceIdSource;
use super::identity::DeviceKey;

/// Name of a provider-defined identity space that is registered during app construction.
///
/// `SchemeName` rejects malformed syntax while deserializing so invalid persisted configuration
/// never reaches startup validation. Registration is separate because deserialization cannot see
/// the app's `RegisteredSchemes` resource.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct SchemeName(String);

impl SchemeName {
    /// Create a scheme name whose lowercase ASCII words and hyphens can name one identity space.
    ///
    /// Empty names, uppercase letters, repeated hyphens, and leading or trailing hyphens are
    /// rejected because providers would otherwise spell one shared identity space differently.
    ///
    /// # Errors
    ///
    /// Returns `SchemeNameError` when `value` is empty or fails the lowercase ASCII and hyphen
    /// syntax shared by registered provider names.
    pub fn new(value: impl Into<String>) -> Result<Self, SchemeNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SchemeNameError::Empty);
        }
        if !has_valid_scheme_syntax(&value) {
            return Err(SchemeNameError::InvalidSyntax);
        }

        Ok(Self(value))
    }

    /// Borrow the registered name when a provider needs to display or log its identity space.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl Display for SchemeName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult { formatter.write_str(&self.0) }
}

impl<'de> Deserialize<'de> for SchemeName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(<D::Error as DeserializeError>::custom)
    }
}

/// Reason a `SchemeName::new` call rejected text before it could name a device identity space.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SchemeNameError {
    /// No provider can use an empty name to state which identity space produced a value.
    #[error("scheme names must not be empty")]
    Empty,
    /// The text cannot be a shared name because it is not lowercase ASCII words separated by one
    /// hyphen.
    #[error("scheme names must use lowercase ASCII letters, digits, and single hyphens")]
    InvalidSyntax,
}

/// Value reported by a unit within a `SchemeName` identity space.
///
/// The kernel preserves this text without interpreting it: an EDID serial, a `CoreAudio` UID, and
/// a network dock child address can all be valid values for their respective schemes.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct ReportedId(String);

impl ReportedId {
    /// Create a reported identifier that can be retained and serialized without a blank or control
    /// character obscuring the value the unit supplied.
    ///
    /// # Errors
    ///
    /// Returns `ReportedIdError` when `value` is empty or has a control character that would make
    /// persisted configuration and diagnostics ambiguous.
    pub fn new(value: impl Into<String>) -> Result<Self, ReportedIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ReportedIdError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(ReportedIdError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    /// Borrow the provider-defined value for diagnostics without assigning meaning outside its
    /// scheme.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl<'de> Deserialize<'de> for ReportedId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(<D::Error as DeserializeError>::custom)
    }
}

/// Reason a `ReportedId::new` call rejected text that cannot safely represent a unit's report.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReportedIdError {
    /// A blank value cannot distinguish any physical unit within the selected scheme.
    #[error("reported identifiers must not be empty")]
    Empty,
    /// Control characters make a reported value unsafe to show in persisted configuration or logs.
    #[error("reported identifiers must not contain control characters")]
    ContainsControlCharacter,
}

/// Persisted identity value that application configuration assigns to one authored device.
///
/// `AuthoredId` is separate from `ReportedId` because an operator label is authorization intent,
/// not a value supplied by the physical unit or one of its identity schemes.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct AuthoredId(String);

impl AuthoredId {
    /// Create one persisted authored identifier without blank or control-character text.
    ///
    /// # Errors
    ///
    /// Returns `AuthoredIdError` when `value` cannot distinguish an authored inventory entry in
    /// configuration or diagnostics.
    pub fn new(value: impl Into<String>) -> Result<Self, AuthoredIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(AuthoredIdError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(AuthoredIdError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    /// Borrow the application-authored value without treating it as reporter evidence.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl<'de> Deserialize<'de> for AuthoredId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(<D::Error as DeserializeError>::custom)
    }
}

/// Reason `AuthoredId::new` rejected text before it could name a persisted inventory entry.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AuthoredIdError {
    /// A blank value cannot identify a distinct device in authored application inventory.
    #[error("authored identifiers must not be empty")]
    Empty,
    /// Control characters make an authored value ambiguous in configuration and diagnostics.
    #[error("authored identifiers must not contain control characters")]
    ContainsControlCharacter,
}

/// Fixed-width FNV-1a result synthesized from descriptors when a unit reports no unique identity.
///
/// `Digest` uses `u64` instead of text because every `u64` is representable as an FNV-1a result;
/// parsing cannot create a malformed digest or allocate a string for it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct Digest(u64);

impl Digest {
    /// Wrap an FNV-1a result after a provider hashes its descriptors into the required 64-bit
    /// value.
    #[must_use]
    pub const fn new(value: u64) -> Self { Self(value) }
}

/// App-build registry of identity spaces providers are allowed to report.
///
/// `RegisteredSchemes` keeps syntax and startup registration separate: `SchemeName` can reject
/// malformed persisted text during deserialization, while this resource rejects well-formed names
/// that no provider registered before that state becomes visible.
#[derive(Debug, Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct RegisteredSchemes {
    names: Vec<SchemeName>,
}

impl RegisteredSchemes {
    /// Register a provider's identity space during app construction.
    ///
    /// Repeating an existing name succeeds because two providers using the same `SchemeName` assert
    /// that their reported values are comparable in one identity space.
    pub fn register(&mut self, name: SchemeName) {
        if !self.names.contains(&name) {
            self.names.push(name);
        }
    }

    /// Report whether app construction registered this identity space before providers publish
    /// keys.
    #[must_use]
    pub fn contains(&self, name: &SchemeName) -> bool { self.names.contains(name) }

    #[cfg(test)]
    pub(crate) const fn count(&self) -> usize { self.names.len() }

    /// Reject a reported `DeviceKey` whose scheme was absent from app-build registration.
    ///
    /// Synthesized keys need no registration because their digest has no provider-defined identity
    /// space. Call this before publishing deserialized keys so a typo fails during startup.
    ///
    /// # Errors
    ///
    /// Returns `UnregisteredSchemeError` when a reported key names a scheme that no provider
    /// registered during app construction.
    pub fn validate(&self, key: &DeviceKey) -> Result<(), UnregisteredSchemeError> {
        let DeviceIdSource::Reported { scheme, .. } = &key.id else {
            return Ok(());
        };

        if self.contains(scheme) {
            Ok(())
        } else {
            Err(UnregisteredSchemeError {
                scheme: scheme.clone(),
            })
        }
    }
}

/// Failure produced when persisted configuration uses a well-formed but unregistered scheme name.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("device key uses unregistered scheme `{scheme}`")]
pub struct UnregisteredSchemeError {
    scheme: SchemeName,
}

impl UnregisteredSchemeError {
    /// Borrow the name that startup registration did not receive from any provider.
    #[must_use]
    pub(crate) const fn scheme(&self) -> &SchemeName { &self.scheme }
}

fn has_valid_scheme_syntax(value: &str) -> bool {
    !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use bevy::reflect::PartialReflect;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;

    use super::RegisteredSchemes;
    use super::SchemeName;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::ReportedId;

    #[test]
    fn malformed_scheme_name_fails_ron_deserialization() {
        assert!(ron::from_str::<SchemeName>("\"EDID-SERIAL\"").is_err());
    }

    #[test]
    fn empty_reported_id_fails_ron_deserialization() {
        assert!(ron::from_str::<ReportedId>("\"\"").is_err());
    }

    #[test]
    fn reported_id_with_control_character_fails_ron_deserialization() {
        assert!(ron::from_str::<ReportedId>(r#""device\nserial""#).is_err());
    }

    #[test]
    fn reflection_rejects_malformed_identity_text() -> Result<(), Box<dyn Error>> {
        let mut malformed_scheme = DynamicTupleStruct::default();
        malformed_scheme.insert(String::from("EDID-SERIAL"));
        let mut scheme = SchemeName::new("edid-serial")?;

        assert!(scheme.try_apply(&malformed_scheme).is_err());
        assert_eq!(scheme.as_str(), "edid-serial");

        let mut malformed_reported_id = DynamicTupleStruct::default();
        malformed_reported_id.insert(String::from("device\nserial"));
        let mut reported_id = ReportedId::new("DELL-U2723QE-9J4K2H3")?;

        assert!(reported_id.try_apply(&malformed_reported_id).is_err());
        assert_eq!(reported_id.as_str(), "DELL-U2723QE-9J4K2H3");

        Ok(())
    }

    #[test]
    fn unregistered_scheme_fails_startup_validation() -> Result<(), Box<dyn Error>> {
        let key = reported_display_key("edid-serial")?;

        assert!(RegisteredSchemes::default().validate(&key).is_err());

        Ok(())
    }

    #[test]
    fn duplicate_scheme_registration_accepts_co_reporting_providers() -> Result<(), Box<dyn Error>>
    {
        let scheme = SchemeName::new("edid-serial")?;
        let mut schemes = RegisteredSchemes::default();
        schemes.register(scheme.clone());
        schemes.register(scheme);

        assert!(
            schemes
                .validate(&reported_display_key("edid-serial")?)
                .is_ok()
        );

        Ok(())
    }

    fn reported_display_key(scheme: &str) -> Result<DeviceKey, Box<dyn Error>> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: SchemeName::new(scheme)?,
                value:  ReportedId::new("DELL-U2723QE-9J4K2H3")?,
            },
        })
    }
}
