use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result as FormatResult;

use bevy::prelude::Reflect;
use bevy::reflect::ReflectDeserialize;
use bevy::reflect::ReflectSerialize;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de::Error as DeserializeError;
use thiserror::Error;

use crate::DeviceKey;

/// Binding target that identifies one provider-defined endpoint on one durable device.
///
/// `DeviceEndpoint` permits several bindings to address different channels of one audio
/// interface, lighting controller, or HID panel. A display uses the same type with
/// `EndpointId::Whole`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Serialize, Deserialize)]
pub struct DeviceEndpoint {
    /// Durable device designation that keeps the endpoint associated with one physical unit.
    pub device: DeviceKey,
    /// Provider-defined address within `device`, distinguishing a whole display from one named
    /// part such as `ch/7` or `key/3`.
    pub id:     EndpointId,
}

/// Address within a device that distinguishes its whole surface from one provider-named part.
///
/// A display has one endpoint, so it uses `Whole` rather than inventing a part name. `Part` keeps
/// the provider's vocabulary because the kernel cannot know whether `ch/7` is an audio channel,
/// `key/3` is a control surface key, or `nfc` is a reader.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Serialize, Deserialize)]
pub enum EndpointId {
    /// The device exposes one undivided endpoint, as with a display surface.
    Whole,
    /// A provider-defined endpoint within a multi-part device, such as one audio channel.
    Part(PartName),
}

/// Provider-defined name for one named part within a device.
///
/// `PartName` retains the provider's vocabulary because the kernel cannot know whether `ch/7` is
/// an audio channel, `key/3` is a control surface key, or `nfc` is a reader. Its checked
/// constructor rejects blank and control-character values before endpoint configuration is
/// retained.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize, Deserialize)]
pub struct PartName(String);

impl PartName {
    /// Create an endpoint name that can be retained without blank or control-character text.
    ///
    /// # Errors
    ///
    /// Returns `PartNameError` when `value` cannot identify an endpoint in diagnostics or
    /// persisted configuration.
    pub fn new(value: impl Into<String>) -> Result<Self, PartNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PartNameError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(PartNameError::ContainsControlCharacter);
        }

        Ok(Self(value))
    }

    /// Borrow the provider-defined endpoint name without assigning device-class meaning to it.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl Display for PartName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult { formatter.write_str(&self.0) }
}

impl<'de> Deserialize<'de> for PartName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(<D::Error as DeserializeError>::custom)
    }
}

/// Reason `PartName::new` rejected text before it could name an endpoint.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PartNameError {
    /// An empty value cannot select a named device endpoint.
    #[error("endpoint part names must not be empty")]
    Empty,
    /// Control characters make part labels ambiguous in configuration and diagnostics.
    #[error("endpoint part names must not contain control characters")]
    ContainsControlCharacter,
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use ron::Options;
    use ron::extensions::Extensions;

    use super::DeviceEndpoint;
    use super::EndpointId;
    use super::PartName;
    use super::PartNameError;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::ReportedId;
    use crate::SchemeName;

    #[test]
    fn whole_display_endpoint_survives_serde_round_trip() -> Result<(), Box<dyn Error>> {
        let device_endpoint = DeviceEndpoint {
            device: display_key()?,
            id:     EndpointId::Whole,
        };
        let options = Options::default().with_default_extension(Extensions::UNWRAP_NEWTYPES);
        let encoded = options.to_string(&device_endpoint)?;
        let decoded: DeviceEndpoint = options.from_str(&encoded)?;

        assert_eq!(decoded, device_endpoint);

        Ok(())
    }

    #[test]
    fn named_part_endpoint_survives_serde_round_trip() -> Result<(), Box<dyn Error>> {
        let device_endpoint = DeviceEndpoint {
            device: display_key()?,
            id:     EndpointId::Part(PartName::new("ch/7")?),
        };
        let options = Options::default().with_default_extension(Extensions::UNWRAP_NEWTYPES);
        let encoded = options.to_string(&device_endpoint)?;
        let decoded: DeviceEndpoint = options.from_str(&encoded)?;

        assert_eq!(decoded, device_endpoint);

        Ok(())
    }

    #[test]
    fn empty_part_name_returns_empty_error() {
        assert_eq!(PartName::new(""), Err(PartNameError::Empty));
    }

    #[test]
    fn part_name_with_control_character_returns_error() {
        assert_eq!(
            PartName::new("input\nchannel"),
            Err(PartNameError::ContainsControlCharacter)
        );
    }

    #[test]
    fn empty_part_name_fails_ron_deserialization() {
        assert!(ron::from_str::<PartName>("\"\"").is_err());
    }

    #[test]
    fn part_name_with_control_character_fails_ron_deserialization() {
        assert!(ron::from_str::<PartName>(r#""input\nchannel""#).is_err());
    }

    fn display_key() -> Result<DeviceKey, Box<dyn Error>> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: SchemeName::new("edid-serial")?,
                value:  ReportedId::new("DELL-U2723QE-9J4K2H3")?,
            },
        })
    }
}
