/// Error when setting the size of a panel.
///
/// [`DiegeticPanelBuilder::build`](crate::DiegeticPanelBuilder::build) checks dimensions when
/// both axes have a fixed size.
/// Dynamic axes start at zero and get their dimensions during layout.
/// Units such as [`Px`](crate::Px) allow zero for sizing constraints.
#[derive(thiserror::Error, Debug)]
pub enum SizeError {
    /// At least one panel dimension is zero or negative.
    #[error("panel dimensions must be positive, got {width}×{height}")]
    NonPositive {
        /// The requested width.
        width:  f32,
        /// The requested height.
        height: f32,
    },
}

#[cfg(test)]
mod tests {
    use super::SizeError;

    #[test]
    fn invalid_size_message_is_stable() {
        let error = SizeError::NonPositive {
            width:  12.5,
            height: -4.0,
        };

        assert_eq!(
            error.to_string(),
            "panel dimensions must be positive, got 12.5×-4"
        );
    }
}
