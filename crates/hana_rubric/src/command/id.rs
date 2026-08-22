use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use bevy::prelude::Reflect;

/// A validated, namespaced keymap command identifier.
///
/// Command IDs contain exactly one `::` namespace separator. Both segments
/// use ASCII snake case, such as `selection::zoom_to` or `camera::home`.
/// Hana's built-in command namespaces are `camera`, `selection`,
/// `dimension_lock`, `transport`, `scene`, `voice`, `spawn`, and `inspect`.
/// Third-party command namespaces remain valid.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct CommandId(String);

impl CommandId {
    const NAMESPACE_SEPARATOR_BYTE: u8 = b':';

    /// The id of a command declared through [`KeymapCommand`](crate::KeymapCommand).
    ///
    /// No parse can fail here: the `command!` declaration already asserted
    /// [`Self::is_valid`] on `C::ID` at compile time.
    #[must_use]
    pub fn declared<C: crate::KeymapCommand>() -> Self { Self(C::ID.to_owned()) }

    /// Returns whether `value` is a valid namespaced snake-case command ID.
    ///
    /// This `const fn` is the validation path used by both compile-time command
    /// declarations and runtime keymap parsing.
    #[must_use]
    pub const fn is_valid(value: &str) -> bool {
        let bytes = value.as_bytes();
        let mut separator_index = None;
        let mut index = 0;

        while index < bytes.len() {
            if bytes[index] == Self::NAMESPACE_SEPARATOR_BYTE {
                if index + 1 == bytes.len()
                    || bytes[index + 1] != Self::NAMESPACE_SEPARATOR_BYTE
                    || separator_index.is_some()
                {
                    return false;
                }

                separator_index = Some(index);
                index += 2;
            } else {
                index += 1;
            }
        }

        match separator_index {
            Some(separator_index) => {
                is_snake_case_segment(bytes, 0, separator_index)
                    && is_snake_case_segment(bytes, separator_index + 2, bytes.len())
            },
            None => false,
        }
    }

    /// Borrows the original validated command ID text.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl AsRef<str> for CommandId {
    fn as_ref(&self) -> &str { self.as_str() }
}

impl Display for CommandId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Error returned when text does not satisfy [`CommandId::is_valid`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandIdParseError;

impl Display for CommandIdParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("command IDs require one :: separator and snake-case segments")
    }
}

impl Error for CommandIdParseError {}

impl TryFrom<&str> for CommandId {
    type Error = CommandIdParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if Self::is_valid(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(CommandIdParseError)
        }
    }
}

impl TryFrom<String> for CommandId {
    type Error = CommandIdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if Self::is_valid(&value) {
            Ok(Self(value))
        } else {
            Err(CommandIdParseError)
        }
    }
}

impl FromStr for CommandId {
    type Err = CommandIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> { Self::try_from(value) }
}

const fn is_snake_case_segment(bytes: &[u8], start: usize, end: usize) -> bool {
    if start == end || !bytes[start].is_ascii_lowercase() {
        return false;
    }

    let mut index = start + 1;
    while index < end {
        let byte = bytes[index];
        if is_snake_case_character(byte) {
            index += 1;
            continue;
        }

        if byte != b'_' || index + 1 == end || !is_snake_case_character(bytes[index + 1]) {
            return false;
        }

        index += 1;
    }

    true
}

const fn is_snake_case_character(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use bevy::reflect::PartialReflect;
    use bevy::reflect::ReflectRef;

    use super::CommandId;

    struct ValidationCase {
        value:          &'static str,
        expected_valid: bool,
    }

    const VALIDATION_CASES: [ValidationCase; 10] = [
        ValidationCase {
            value:          "camera::home",
            expected_valid: true,
        },
        ValidationCase {
            value:          "selection::zoom_to",
            expected_valid: true,
        },
        ValidationCase {
            value:          "camera",
            expected_valid: false,
        },
        ValidationCase {
            value:          "camera::home::reset",
            expected_valid: false,
        },
        ValidationCase {
            value:          "Camera::home",
            expected_valid: false,
        },
        ValidationCase {
            value:          "camera::Home",
            expected_valid: false,
        },
        ValidationCase {
            value:          "camera-home::reset",
            expected_valid: false,
        },
        ValidationCase {
            value:          "camera_::home",
            expected_valid: false,
        },
        ValidationCase {
            value:          "::home",
            expected_valid: false,
        },
        ValidationCase {
            value:          "camera::",
            expected_valid: false,
        },
    ];

    const CONST_VALIDATION_RESULTS: [bool; VALIDATION_CASES.len()] =
        const_validation_results(&VALIDATION_CASES);

    #[test]
    fn command_id_accepts_camera_home() {
        assert!(CommandId::try_from("camera::home").is_ok());
    }

    #[test]
    fn command_id_rejects_zero_and_two_namespace_separators() {
        assert!(CommandId::try_from("camera").is_err());
        assert!(CommandId::try_from("camera::home::reset").is_err());
    }

    #[test]
    fn command_id_rejects_non_snake_case_segments() {
        assert!(CommandId::try_from("Camera::home").is_err());
        assert!(CommandId::try_from("camera::Home").is_err());
        assert!(CommandId::try_from("camera-home::reset").is_err());
    }

    #[test]
    fn command_id_rejects_empty_segments() {
        assert!(CommandId::try_from("::home").is_err());
        assert!(CommandId::try_from("camera::").is_err());
    }

    #[test]
    fn command_id_reflection_is_opaque() {
        let command_id = CommandId::try_from("camera::home");

        assert!(matches!(
            command_id.as_ref().map(PartialReflect::reflect_ref),
            Ok(ReflectRef::Opaque(_))
        ));
    }

    #[test]
    fn command_id_const_and_runtime_validation_agree() {
        for (validation_case, const_validity) in
            VALIDATION_CASES.iter().zip(CONST_VALIDATION_RESULTS.iter())
        {
            assert_eq!(*const_validity, validation_case.expected_valid);
            assert_eq!(
                CommandId::try_from(validation_case.value).is_ok(),
                *const_validity
            );
        }
    }

    const fn const_validation_results<const LENGTH: usize>(
        validation_cases: &[ValidationCase; LENGTH],
    ) -> [bool; LENGTH] {
        let mut results = [false; LENGTH];
        let mut index = 0;

        while index < LENGTH {
            results[index] = CommandId::is_valid(validation_cases[index].value);
            index += 1;
        }

        results
    }
}
