use std::time::Duration;

use bevy::reflect::Reflect;

/// Exact nonnegative sequence time with nanosecond resolution.
///
/// The private representation remains normalized so `nanoseconds` is always
/// less than one second. Unlike [`Duration`], the whole-seconds field can hold
/// the exact sum of every duration in an in-memory sequence.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct SequenceTime {
    whole_seconds: u128,
    nanoseconds:   u32,
}

impl SequenceTime {
    const NANOSECONDS_PER_SECOND: u32 = 1_000_000_000;
    const U32_RADIX_AS_F64: f64 = 4_294_967_296.0;

    /// Zero elapsed sequence time.
    pub const ZERO: Self = Self {
        whole_seconds: 0,
        nanoseconds:   0,
    };

    /// Sums authored durations without narrowing whole seconds or nanoseconds.
    #[must_use]
    pub fn sum(durations: &[Duration]) -> Self {
        durations
            .iter()
            .copied()
            .fold(Self::ZERO, Self::add_duration)
    }

    /// Returns the exact whole-second portion.
    #[must_use]
    pub const fn whole_seconds(self) -> u128 { self.whole_seconds }

    /// Returns the exact subsecond nanoseconds in `0..1_000_000_000`.
    #[must_use]
    pub const fn subsec_nanoseconds(self) -> u32 { self.nanoseconds }

    /// Converts this value to approximate fractional seconds for scalar math.
    ///
    /// Exact boundary comparison and event accounting should use the integer
    /// accessors. This conversion can round when `whole_seconds` exceeds the
    /// exact integer range of `f64`.
    #[must_use]
    pub fn as_seconds_f64(self) -> f64 {
        Self::whole_seconds_as_f64(self.whole_seconds)
            + f64::from(self.nanoseconds) / f64::from(Self::NANOSECONDS_PER_SECOND)
    }

    /// Returns this time advanced by one authored duration.
    #[must_use]
    pub fn advanced_by(self, duration: Duration) -> Self { Self::add_duration(self, duration) }

    /// Returns the fraction of `total` this time represents.
    ///
    /// A zero total has no elapsed extent, so every time maps to `0.0`. The
    /// division runs in `f64` and its result is rounded to `f32`.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the f64 quotient is rounded to the f32 this accessor returns"
    )]
    pub fn fraction_of(self, total: Self) -> f32 {
        if total.is_zero() {
            return 0.0;
        }
        (self.as_seconds_f64() / total.as_seconds_f64()) as f32
    }

    /// Returns whether both exact portions are zero.
    #[must_use]
    pub const fn is_zero(self) -> bool { self.whole_seconds == 0 && self.nanoseconds == 0 }

    fn add_duration(mut sequence_time: Self, duration: Duration) -> Self {
        let nanoseconds = sequence_time.nanoseconds + duration.subsec_nanos();
        sequence_time.whole_seconds +=
            u128::from(duration.as_secs()) + u128::from(nanoseconds / Self::NANOSECONDS_PER_SECOND);
        sequence_time.nanoseconds = nanoseconds % Self::NANOSECONDS_PER_SECOND;
        sequence_time
    }

    fn whole_seconds_as_f64(whole_seconds: u128) -> f64 {
        let bytes = whole_seconds.to_be_bytes();
        let words: [u32; 4] = std::array::from_fn(|word_index| {
            let byte_index = word_index * 4;
            u32::from_be_bytes([
                bytes[byte_index],
                bytes[byte_index + 1],
                bytes[byte_index + 2],
                bytes[byte_index + 3],
            ])
        });
        words.into_iter().fold(0.0, |seconds, word| {
            seconds.mul_add(Self::U32_RADIX_AS_F64, f64::from(word))
        })
    }
}

impl From<Duration> for SequenceTime {
    fn from(duration: Duration) -> Self {
        Self {
            whole_seconds: u128::from(duration.as_secs()),
            nanoseconds:   duration.subsec_nanos(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_time_sums_duration_maxima_exactly() {
        let durations = [Duration::MAX, Duration::MAX];
        let sequence_time = SequenceTime::sum(&durations);

        assert_eq!(sequence_time.whole_seconds(), u128::from(u64::MAX) * 2 + 1);
        assert_eq!(sequence_time.subsec_nanoseconds(), 999_999_998);
        assert!(sequence_time.as_seconds_f64().is_finite());
    }

    #[test]
    fn sequence_time_from_duration_preserves_both_parts() {
        let duration = Duration::new(17, 23);
        let sequence_time = SequenceTime::from(duration);

        assert_eq!(sequence_time.whole_seconds(), 17);
        assert_eq!(sequence_time.subsec_nanoseconds(), 23);
    }
}
