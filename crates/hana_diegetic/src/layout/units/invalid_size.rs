/// Error returned when panel dimensions are zero or negative.
///
/// Emitted by
/// [`DiegeticPanelBuilder::build`](crate::DiegeticPanelBuilder) when
/// both axes are fixed-size and at least one is `<= 0.0`. Dynamic axes
/// ([`Fit`](crate::Fit) / [`Percent`](crate::Percent) /
/// [`Grow`](crate::Grow)) start at `0.0` and resolve later; they do not
/// trip this check.
///
/// # Why a runtime check rather than a compile-time one?
///
/// The rest of the sizing API catches invalid states at compile time via
/// [`PanelSizing`](crate::PanelSizing) and
/// [`CompatibleUnits`](crate::CompatibleUnits). Zero and negative sizes
/// are the single remaining runtime check, and that's a deliberate
/// tradeoff:
///
/// - **Floats can't carry type-level non-zero proofs.** Rust has no const generics for `f32`, so
///   there is no `NonZero<f32>` to build a `NonZeroDimension` wrapper on top of. Any "no zero"
///   enforcement requires a fallible constructor like `Px::new(600.0) -> Option<Px>` — that's still
///   a runtime check, just relocated earlier in the pipeline.
/// - **`Px(0.0)` is useful.** It's a legitimate "no minimum" sentinel in
///   [`GrowRange`](crate::GrowRange) / [`FitRange`](crate::FitRange) and a "no cap" sentinel in
///   [`GrowMax`](crate::GrowMax) / [`FitMax`](crate::FitMax). Banning `Px(0.0)` at construction
///   forces a parallel type for "zero-ok" arguments or awkward `Unlimited` enum variants, which is
///   worse ergonomics than one `Result` at `.build()`.
/// - **`const fn` panic-on-zero** would catch only literal zero in source; any runtime-computed
///   value (config file, user input, arithmetic) still needs the same check, so it doesn't
///   eliminate the runtime path.
///
/// If you want the check to fail earlier (e.g., at the point you read a
/// user-supplied value), validate the float yourself before handing it
/// to [`Px`](crate::Px) / [`Mm`](crate::Mm) / [`Pt`](crate::Pt) /
/// [`In`](crate::layout::In).
#[derive(thiserror::Error, Debug)]
#[error("panel dimensions must be positive, got {width}×{height}")]
pub struct InvalidSize {
    /// The invalid width value.
    pub width:  f32,
    /// The invalid height value.
    pub height: f32,
}

#[cfg(test)]
mod tests {
    use super::InvalidSize;

    #[test]
    fn invalid_size_message_is_stable() {
        let error = InvalidSize {
            width:  12.5,
            height: -4.0,
        };

        assert_eq!(
            error.to_string(),
            "panel dimensions must be positive, got 12.5×-4"
        );
    }
}
