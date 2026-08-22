//! Ordered non-empty keystroke sequences.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::slice::Iter;
use std::str::FromStr;

use super::Keystroke;
use super::KeystrokeParseError;

/// An ordered, non-empty collection of keystrokes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeystrokeSequence {
    keystrokes: Vec<Keystroke>,
}

impl KeystrokeSequence {
    /// Creates a sequence when at least one keystroke is present.
    ///
    /// # Errors
    ///
    /// Returns [`EmptyKeystrokeSequenceError`] when `keystrokes` is empty.
    pub fn new(keystrokes: Vec<Keystroke>) -> Result<Self, EmptyKeystrokeSequenceError> {
        if keystrokes.is_empty() {
            return Err(EmptyKeystrokeSequenceError);
        }

        Ok(Self { keystrokes })
    }

    /// Returns the keystrokes in source order.
    #[must_use]
    pub fn as_slice(&self) -> &[Keystroke] { &self.keystrokes }

    /// Returns the first keystroke in source order.
    ///
    /// `KeystrokeSequence` construction rejects empty input.
    #[must_use]
    pub fn first(&self) -> Keystroke { self.keystrokes[0] }

    /// Returns the number of keystrokes.
    #[must_use]
    #[expect(
        clippy::len_without_is_empty,
        reason = "KeystrokeSequence is non-empty by construction"
    )]
    pub const fn len(&self) -> usize { self.keystrokes.len() }

    /// Iterates over the keystrokes in source order.
    pub fn iter(&self) -> Iter<'_, Keystroke> { self.keystrokes.iter() }

    /// Compares sequences by the deterministic representative-binding order.
    ///
    /// Shorter sequences win. Equal-length sequences compare each keystroke in source order by
    /// its structural representation rather than its platform-formatted text.
    pub(crate) fn structural_cmp(&self, other: &Self) -> Ordering {
        self.len().cmp(&other.len()).then_with(|| {
            self.iter()
                .zip(other)
                .map(|(keystroke, other_keystroke)| keystroke.structural_cmp(*other_keystroke))
                .find(|ordering| !ordering.is_eq())
                .unwrap_or(Ordering::Equal)
        })
    }
}

impl<'a> IntoIterator for &'a KeystrokeSequence {
    type IntoIter = Iter<'a, Keystroke>;
    type Item = &'a Keystroke;

    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

impl TryFrom<Vec<Keystroke>> for KeystrokeSequence {
    type Error = EmptyKeystrokeSequenceError;

    fn try_from(keystrokes: Vec<Keystroke>) -> Result<Self, Self::Error> { Self::new(keystrokes) }
}

impl Display for KeystrokeSequence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        for (index, keystroke) in self.keystrokes.iter().enumerate() {
            if index != 0 {
                formatter.write_str(" ")?;
            }
            keystroke.fmt(formatter)?;
        }

        Ok(())
    }
}

impl FromStr for KeystrokeSequence {
    type Err = KeystrokeSequenceParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut keystrokes = Vec::new();
        let mut offset = 0;

        for token in input.split(' ') {
            let token_offset = offset;
            offset += token.len() + 1;

            if token.is_empty() {
                continue;
            }

            let keystroke = token.parse().map_err(|error: KeystrokeParseError| {
                KeystrokeSequenceParseError::Keystroke(error.at_offset(token_offset))
            })?;
            keystrokes.push(keystroke);
        }

        Self::new(keystrokes).map_err(KeystrokeSequenceParseError::Empty)
    }
}

/// An error returned when constructing an empty [`KeystrokeSequence`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyKeystrokeSequenceError;

impl Display for EmptyKeystrokeSequenceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a keystroke sequence must contain at least one keystroke")
    }
}

impl Error for EmptyKeystrokeSequenceError {}

/// A sequence parse failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeystrokeSequenceParseError {
    /// The sequence contains no keystrokes.
    Empty(EmptyKeystrokeSequenceError),
    /// One source keystroke failed to parse.
    Keystroke(KeystrokeParseError),
}

impl Display for KeystrokeSequenceParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(error) => error.fmt(formatter),
            Self::Keystroke(error) => error.fmt(formatter),
        }
    }
}

impl Error for KeystrokeSequenceParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Empty(error) => Some(error),
            Self::Keystroke(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EmptyKeystrokeSequenceError;
    use super::Keystroke;
    use super::KeystrokeSequence;
    use super::KeystrokeSequenceParseError;

    #[test]
    fn sequence_display_round_trips() -> Result<(), KeystrokeSequenceParseError> {
        let sequence: KeystrokeSequence = "cmd-k l".parse()?;

        assert_eq!(sequence.to_string().parse::<KeystrokeSequence>()?, sequence);

        Ok(())
    }

    #[test]
    fn empty_sequences_are_rejected() {
        assert_eq!(
            KeystrokeSequence::new(Vec::new()),
            Err(EmptyKeystrokeSequenceError)
        );
    }

    #[test]
    fn accessors_follow_the_nonempty_sequence_invariant() -> Result<(), KeystrokeSequenceParseError>
    {
        let sequence: KeystrokeSequence = "g h".parse()?;
        let first = "g"
            .parse::<Keystroke>()
            .map_err(KeystrokeSequenceParseError::Keystroke)?;

        assert_eq!(sequence.first(), first);
        assert_eq!(sequence.len(), 2);
        assert_eq!(
            sequence.iter().copied().collect::<Vec<_>>(),
            sequence.as_slice()
        );

        Ok(())
    }

    #[test]
    fn sequence_errors_use_the_full_source_offset() {
        let result = "cmd-k unknown-key".parse::<KeystrokeSequence>();

        assert!(matches!(
            result,
            Err(KeystrokeSequenceParseError::Keystroke(error))
                if error.token() == "unknown" && error.offset() == 6
        ));
    }

    #[test]
    fn a_bare_modifier_is_one_sequence_keystroke() -> Result<(), KeystrokeSequenceParseError> {
        let sequence: KeystrokeSequence = "shift".parse()?;

        assert_eq!(sequence.len(), 1);
        assert_eq!(sequence.to_string(), "shift");
        Ok(())
    }
}
