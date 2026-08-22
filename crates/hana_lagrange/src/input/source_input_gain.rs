//! Source-level input gain setter traits shared by camera presets.

/// Sets mouse-backed source input gain on a camera preset.
pub trait MouseInputGain {
    /// `InputGain` value accepted by this preset.
    type Gain;

    /// Returns this preset with mouse-backed source input gain replaced.
    #[must_use]
    fn mouse_input_gain(self, input_gain: Self::Gain) -> Self;
}

/// Sets Bevy pixel-scroll source input gain on a camera preset.
pub trait SmoothScrollInputGain {
    /// `InputGain` value accepted by this preset.
    type Gain;

    /// Returns this preset with smooth-scroll source input gain replaced.
    #[must_use]
    fn smooth_scroll_input_gain(self, input_gain: Self::Gain) -> Self;
}

/// Sets gamepad-backed source input gain on a camera preset.
pub trait GamepadInputGain {
    /// `InputGain` value accepted by this preset.
    type Gain;

    /// Returns this preset with gamepad-backed source input gain replaced.
    #[must_use]
    fn gamepad_input_gain(self, input_gain: Self::Gain) -> Self;
}
