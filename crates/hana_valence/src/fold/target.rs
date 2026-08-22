use crate::FoldAuthorError;

/// A validated normalized fold destination.
///
/// [`Self::BASE`] is the resting endpoint a member starts from and
/// [`Self::FOLDED`] is its fully folded endpoint. Interior values are legal
/// authored destinations, so a stage can stop a member part way. Easing may
/// still carry evaluated output past an endpoint; that overshoot is an
/// evaluation result, not an authored target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldTarget(f32);

impl FoldTarget {
    /// The unfolded resting endpoint every member starts from.
    pub const BASE: Self = Self(0.0);
    /// The fully folded endpoint.
    pub const FOLDED: Self = Self(1.0);

    /// Creates a target from a finite fraction within `0..=1`.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::NonFiniteFoldTarget`] for a NaN or infinite
    /// value and [`FoldAuthorError::FoldTargetOutOfRange`] for a finite value
    /// outside the normalized range.
    pub fn try_new(fraction: f32) -> Result<Self, FoldAuthorError> {
        if !fraction.is_finite() {
            return Err(FoldAuthorError::NonFiniteFoldTarget);
        }
        if !(Self::BASE.0..=Self::FOLDED.0).contains(&fraction) {
            return Err(FoldAuthorError::FoldTargetOutOfRange);
        }
        Ok(Self(fraction))
    }

    /// Returns the normalized fraction of the folded endpoint.
    #[must_use]
    pub const fn fraction(self) -> f32 { self.0 }
}

impl Default for FoldTarget {
    fn default() -> Self { Self::BASE }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exactly representable authored fractions"
)]
mod tests {
    use super::*;

    const HALF_FOLDED: f32 = 0.5;

    #[test]
    fn endpoints_and_interior_targets_keep_their_authored_fraction() {
        assert_eq!(FoldTarget::BASE.fraction(), 0.0);
        assert_eq!(FoldTarget::FOLDED.fraction(), 1.0);
        assert_eq!(FoldTarget::default(), FoldTarget::BASE);
        assert_eq!(
            FoldTarget::try_new(HALF_FOLDED).map(FoldTarget::fraction),
            Ok(HALF_FOLDED)
        );
    }

    #[test]
    fn target_construction_rejects_non_finite_and_out_of_range_fractions() {
        assert_eq!(
            FoldTarget::try_new(f32::NAN),
            Err(FoldAuthorError::NonFiniteFoldTarget)
        );
        assert_eq!(
            FoldTarget::try_new(f32::INFINITY),
            Err(FoldAuthorError::NonFiniteFoldTarget)
        );
        assert_eq!(
            FoldTarget::try_new(-HALF_FOLDED),
            Err(FoldAuthorError::FoldTargetOutOfRange)
        );
        assert_eq!(
            FoldTarget::try_new(FoldTarget::FOLDED.fraction() + HALF_FOLDED),
            Err(FoldAuthorError::FoldTargetOutOfRange)
        );
    }
}
