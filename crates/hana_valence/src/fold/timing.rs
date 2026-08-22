use std::time::Duration;

use bevy_kana::Easing;
use bevy_kana::SequenceTime;

/// One authored movement's delay, extent, and curve inside its stage.
///
/// A [`FoldSequence`](super::FoldSequence) requires one of these as its default;
/// a [`FoldStage`](super::FoldStage) or a single stage member may override the
/// whole value. Authored durations are trusted, so a zero `duration` is an
/// instantaneous snap to the member's target rather than a rejected value.
///
/// This value is cloneable rather than `Copy` because `easing` may own a
/// lookup curve.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoldTiming {
    /// Delay after the owning stage begins before this movement starts.
    pub start_offset: Duration,
    /// How long the movement runs once it starts.
    pub duration:     Duration,
    /// Curve applied to normalized progress across `duration`.
    pub easing:       Easing,
}

impl FoldTiming {
    /// Creates timing that starts with its stage and runs for `duration`.
    ///
    /// `easing` accepts a stock `EaseFunction` or an owned `EasingCurve`.
    pub fn new(duration: Duration, easing: impl Into<Easing>) -> Self {
        Self {
            start_offset: Duration::ZERO,
            duration,
            easing: easing.into(),
        }
    }

    /// Creates instantaneous timing that snaps to its target with no travel.
    #[must_use]
    pub fn snap() -> Self { Self::default() }

    /// Delays this movement's start within its stage.
    #[must_use]
    pub const fn with_start_offset(mut self, start_offset: Duration) -> Self {
        self.start_offset = start_offset;
        self
    }

    /// Replaces how long this movement runs.
    #[must_use]
    pub const fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Replaces the curve applied across `duration`.
    #[must_use]
    pub fn with_easing(mut self, easing: impl Into<Easing>) -> Self {
        self.easing = easing.into();
        self
    }

    /// Returns the exact stage-relative time where this movement finishes.
    ///
    /// The sum runs in [`SequenceTime`], so two `Duration::MAX` parts stay
    /// exact instead of saturating.
    #[must_use]
    pub fn end(&self) -> SequenceTime { SequenceTime::sum(&[self.start_offset, self.duration]) }

    /// Returns the stage duration this movement alone would require.
    ///
    /// This is the value handed to
    /// [`SequenceStages::new`](bevy_kana::SequenceStages::new) for the stage
    /// whose members this movement outlasts. Unlike [`Self::end`] it saturates,
    /// because a `Duration` cannot hold every exact sum. A
    /// [`FoldSegment`](super::FoldSegment) ends on this same saturating sum, so
    /// no member's segment can end after the stage extent built from it.
    #[must_use]
    pub const fn stage_duration(&self) -> Duration {
        self.start_offset.saturating_add(self.duration)
    }
}

#[cfg(test)]
mod tests {
    use bevy_math::curve::EaseFunction;

    use super::*;

    const OFFSET: Duration = Duration::from_millis(250);
    const TRAVEL: Duration = Duration::from_millis(750);

    #[test]
    fn authored_timing_records_offset_extent_and_curve() {
        let fold_timing = FoldTiming::new(TRAVEL, EaseFunction::QuadraticIn)
            .with_start_offset(OFFSET)
            .with_easing(EaseFunction::Linear);

        assert_eq!(fold_timing.start_offset, OFFSET);
        assert_eq!(fold_timing.duration, TRAVEL);
        assert_eq!(fold_timing.easing, Easing::Bevy(EaseFunction::Linear));
        assert_eq!(fold_timing.stage_duration(), OFFSET + TRAVEL);
        assert_eq!(fold_timing.end(), SequenceTime::from(OFFSET + TRAVEL));
    }

    #[test]
    fn snap_timing_has_no_extent_and_ends_where_it_starts() {
        let fold_timing = FoldTiming::snap().with_start_offset(OFFSET);

        assert_eq!(fold_timing.duration, Duration::ZERO);
        assert_eq!(fold_timing.end(), SequenceTime::from(OFFSET));
        assert_eq!(fold_timing.stage_duration(), OFFSET);
    }

    #[test]
    fn maximum_durations_sum_exactly_and_saturate_only_for_the_stage_duration() {
        let fold_timing =
            FoldTiming::new(Duration::MAX, EaseFunction::Linear).with_start_offset(Duration::MAX);

        assert_eq!(
            fold_timing.end().whole_seconds(),
            u128::from(u64::MAX) * 2 + 1
        );
        assert_eq!(fold_timing.end().subsec_nanoseconds(), 999_999_998);
        assert_eq!(fold_timing.stage_duration(), Duration::MAX);
    }
}
