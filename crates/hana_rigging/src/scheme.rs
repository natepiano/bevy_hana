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
use serde::de::IgnoredAny;
use serde::de::SeqAccess;
use serde::de::Visitor;
use thiserror::Error;

use super::identity::DeviceIdSource;
use super::identity::DeviceKey;

/// Name of a provider-defined identity space that is registered during app construction.
///
/// [`SchemeName`] rejects malformed syntax while deserializing so invalid persisted configuration
/// never reaches startup validation. Registration is separate because deserialization cannot reach
/// the app's [`RegisteredSchemes`] resource.
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
    /// Returns [`SchemeNameError`] when `value` is empty or fails the lowercase ASCII and hyphen
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

/// Reason a [`SchemeName::new`] call rejected text before it could name a device identity space.
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

/// Value reported by a unit within a [`SchemeName`] identity space.
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
    /// Returns [`ReportedIdError`] when `value` is empty or has a control character that would
    /// make persisted configuration and diagnostics ambiguous.
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

/// Reason a [`ReportedId::new`] call rejected text that cannot safely represent a unit's report.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReportedIdError {
    /// A blank value cannot distinguish any physical unit within the selected scheme.
    #[error("reported identifiers must not be empty")]
    Empty,
    /// Control characters make a reported value unsafe to show in persisted configuration or logs.
    #[error("reported identifiers must not contain control characters")]
    ContainsControlCharacter,
}

/// The name a unit reported for itself, such as an operating-system product name.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct ReportedDeviceName(String);

impl ReportedDeviceName {
    /// Create a unit-reported name without blank or control-character text.
    ///
    /// # Errors
    ///
    /// Returns [`ReportedDeviceNameError`] when `value` cannot safely identify the unit in
    /// persisted configuration or operator diagnostics.
    pub fn new(value: impl Into<String>) -> Result<Self, ReportedDeviceNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ReportedDeviceNameError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(ReportedDeviceNameError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    /// Borrow the name exactly as the unit reported it.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl<'de> Deserialize<'de> for ReportedDeviceName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_device_name_text(deserializer)?;
        Self::new(value).map_err(<D::Error as DeserializeError>::custom)
    }
}

/// Reason [`ReportedDeviceName::new`] rejected text that a unit supplied as its name.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReportedDeviceNameError {
    /// An empty report gives an operator no name by which to recognize the unit.
    #[error("reported device names must not be empty")]
    Empty,
    /// Control characters make a reported name unsafe to show in configuration or diagnostics.
    #[error("reported device names must not contain control characters")]
    ContainsControlCharacter,
}

/// Domain-formatted text that separates two units of one class reporting the same name.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct DeviceNameDisambiguationText(String);

impl DeviceNameDisambiguationText {
    /// Create distinguishing text without blank or control-character content.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceNameDisambiguationTextError`] when `value` cannot safely distinguish
    /// otherwise identical units in persisted configuration or operator diagnostics.
    pub fn new(value: impl Into<String>) -> Result<Self, DeviceNameDisambiguationTextError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DeviceNameDisambiguationTextError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(DeviceNameDisambiguationTextError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    /// Borrow the domain-formatted distinction text.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl<'de> Deserialize<'de> for DeviceNameDisambiguationText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_device_name_text(deserializer)?;
        Self::new(value).map_err(<D::Error as DeserializeError>::custom)
    }
}

/// Reason [`DeviceNameDisambiguationText::new`] rejected text intended to distinguish similar
/// units.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DeviceNameDisambiguationTextError {
    /// Empty text cannot distinguish one otherwise identical unit from another.
    #[error("device-name disambiguation text must not be empty")]
    Empty,
    /// Control characters make distinguishing text unsafe to show or persist.
    #[error("device-name disambiguation text must not contain control characters")]
    ContainsControlCharacter,
}

/// The name a person assigned to a unit that cannot name itself.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct OperatorAssignedDeviceName(String);

impl OperatorAssignedDeviceName {
    /// Create a person-assigned device name without blank or control-character text.
    ///
    /// # Errors
    ///
    /// Returns [`OperatorAssignedDeviceNameError`] when `value` cannot safely name the unit in
    /// persisted configuration or operator diagnostics.
    #[cfg(feature = "test-support")]
    pub fn new(value: impl Into<String>) -> Result<Self, OperatorAssignedDeviceNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(OperatorAssignedDeviceNameError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(OperatorAssignedDeviceNameError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    #[cfg(not(feature = "test-support"))]
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, OperatorAssignedDeviceNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(OperatorAssignedDeviceNameError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(OperatorAssignedDeviceNameError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    /// Borrow the name exactly as a person assigned it.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl<'de> Deserialize<'de> for OperatorAssignedDeviceName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_device_name_text(deserializer)?;
        Self::new(value).map_err(<D::Error as DeserializeError>::custom)
    }
}

/// Accepts both ordinary string input and the one-field tuple representation emitted when a
/// reflected component contains one of the opaque validated text types.
fn deserialize_device_name_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct DeviceNameTextVisitor;

    impl<'de> Visitor<'de> for DeviceNameTextVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut Formatter<'_>) -> FormatResult {
            formatter.write_str("a string or a one-field device-name text tuple")
        }

        fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
        where
            E: DeserializeError,
        {
            Ok(value.to_owned())
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: DeserializeError,
        {
            Ok(value.to_owned())
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: DeserializeError,
        {
            Ok(value)
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let Some(value) = sequence.next_element::<String>()? else {
                return Err(<A::Error as DeserializeError>::invalid_length(0, &self));
            };
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(<A::Error as DeserializeError>::invalid_length(2, &self));
            }
            Ok(value)
        }

        fn visit_newtype_struct<N>(self, deserializer: N) -> Result<Self::Value, N::Error>
        where
            N: Deserializer<'de>,
        {
            deserializer.deserialize_string(self)
        }
    }

    deserializer.deserialize_any(DeviceNameTextVisitor)
}

/// Reason [`OperatorAssignedDeviceName::new`] rejected text assigned by a person.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum OperatorAssignedDeviceNameError {
    /// An empty assignment gives an operator no name by which to recognize the unit.
    #[error("operator-assigned device names must not be empty")]
    Empty,
    /// Control characters make a person-assigned name unsafe to show or persist.
    #[error("operator-assigned device names must not contain control characters")]
    ContainsControlCharacter,
}

/// Persisted identity value that application configuration assigns to one authored device.
///
/// [`AuthoredId`] is separate from [`ReportedId`] because an operator label is authorization
/// intent, not a value supplied by the physical unit or one of its identity schemes.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct AuthoredId(String);

impl AuthoredId {
    /// Create one persisted authored identifier without blank or control-character text.
    ///
    /// # Errors
    ///
    /// Returns [`AuthoredIdError`] when `value` cannot distinguish an authored inventory entry in
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

/// Reason [`AuthoredId::new`] rejected text before it could name a persisted inventory entry.
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
/// [`Digest`] uses `u64` instead of text because every `u64` is representable as an FNV-1a result;
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
/// [`RegisteredSchemes`] keeps syntax and startup registration separate: [`SchemeName`] can reject
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
    /// Repeating an existing name succeeds because two providers using the same [`SchemeName`]
    /// assert that their reported values are comparable in one identity space.
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

    /// Reject a reported [`DeviceKey`] whose scheme was absent from app-build registration.
    ///
    /// Synthesized keys need no registration because their digest has no provider-defined identity
    /// space. Call this before publishing deserialized keys so a typo fails during startup.
    ///
    /// # Errors
    ///
    /// Returns [`UnregisteredSchemeError`] when a reported key names a scheme that no provider
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

    use super::DeviceNameDisambiguationText;
    use super::DeviceNameDisambiguationTextError;
    use super::OperatorAssignedDeviceName;
    use super::OperatorAssignedDeviceNameError;
    use super::RegisteredSchemes;
    use super::ReportedDeviceName;
    use super::ReportedDeviceNameError;
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
    fn device_name_text_types_accept_visible_nonempty_text() -> Result<(), Box<dyn Error>> {
        let reported = ReportedDeviceName::new("Studio Display")?;
        let disambiguation = DeviceNameDisambiguationText::new("5120x2880 display at (0, 0)")?;
        let operator = OperatorAssignedDeviceName::new("Stage left laser")?;

        assert_eq!(reported.as_str(), "Studio Display");
        assert_eq!(disambiguation.as_str(), "5120x2880 display at (0, 0)");
        assert_eq!(operator.as_str(), "Stage left laser");

        Ok(())
    }

    #[test]
    fn device_name_text_types_reject_empty_and_control_character_text() {
        assert_eq!(
            ReportedDeviceName::new(""),
            Err(ReportedDeviceNameError::Empty)
        );
        assert_eq!(
            ReportedDeviceName::new("Studio\nDisplay"),
            Err(ReportedDeviceNameError::ContainsControlCharacter)
        );
        assert_eq!(
            DeviceNameDisambiguationText::new(""),
            Err(DeviceNameDisambiguationTextError::Empty)
        );
        assert_eq!(
            DeviceNameDisambiguationText::new("display\tat origin"),
            Err(DeviceNameDisambiguationTextError::ContainsControlCharacter)
        );
        assert_eq!(
            OperatorAssignedDeviceName::new(""),
            Err(OperatorAssignedDeviceNameError::Empty)
        );
        assert_eq!(
            OperatorAssignedDeviceName::new("Stage\rleft laser"),
            Err(OperatorAssignedDeviceNameError::ContainsControlCharacter)
        );
    }

    #[test]
    fn malformed_device_names_fail_ron_deserialization() {
        assert!(ron::from_str::<ReportedDeviceName>(r#""Studio\nDisplay""#).is_err());
        assert!(ron::from_str::<DeviceNameDisambiguationText>("\"\"").is_err());
        assert!(ron::from_str::<OperatorAssignedDeviceName>(r#""Stage\rlaser""#).is_err());
    }

    #[test]
    fn device_name_text_types_accept_reflected_newtype_encoding() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            ron::from_str::<ReportedDeviceName>(r#"("Studio Display")"#)?,
            ReportedDeviceName::new("Studio Display")?
        );
        assert_eq!(
            ron::from_str::<DeviceNameDisambiguationText>(r#"("5120x2880 display at (0, 0)")"#,)?,
            DeviceNameDisambiguationText::new("5120x2880 display at (0, 0)")?
        );
        assert_eq!(
            ron::from_str::<OperatorAssignedDeviceName>(r#"("Stage left laser")"#)?,
            OperatorAssignedDeviceName::new("Stage left laser")?
        );

        Ok(())
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
