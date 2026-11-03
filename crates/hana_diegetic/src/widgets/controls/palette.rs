use bevy::prelude::Color;

/// Colors for the common control roles in a dark System Settings style.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlPalette {
    /// Accent that carries no text: slider value and enabled switch track.
    pub accent:                Color,
    /// Deeper accent for a fill that carries a white label, such as a default button.
    pub accent_behind_text:    Color,
    /// Non-default button face.
    pub control_fill:          Color,
    /// Inset text field face.
    pub field_fill:            Color,
    /// Inset text field hairline.
    pub field_border:          Color,
    /// Text inside an inset value field.
    pub field_text:            Color,
    /// Tooltip outline against a dark surface.
    pub tooltip_border:        Color,
    /// Unfilled slider track.
    pub track:                 Color,
    /// Darker blue live fill that clears the light reach capsule at small sizes.
    pub slider_cabled_fill:    Color,
    /// Light neutral-grey reach capsule behind the whole track and live accent fill.
    pub slider_reachable_band: Color,
    /// Switch outline, held separate from the faint slider track.
    pub switch_border:         Color,
    /// Unfilled switch track, bright enough for its edge and dark enough for its knob.
    pub switch_off_track:      Color,
    /// Switch knob and bright control text.
    pub knob:                  Color,
    /// Near-white slider thumb face.
    pub slider_knob:           Color,
    /// Bright hairline around the slider thumb.
    pub slider_knob_rim:       Color,
    /// Marker for the value pushed into a slider.
    pub pushed_value_marker:   Color,
}

impl ControlPalette {
    /// Returns the standard dark control colors.
    #[must_use]
    pub const fn system_dark() -> Self {
        Self {
            accent:                Color::srgb(0.039, 0.518, 1.0),
            accent_behind_text:    Color::srgb(37.0 / 255.0, 99.0 / 255.0, 235.0 / 255.0),
            control_fill:          Color::srgb(0.36, 0.36, 0.37),
            field_fill:            Color::srgb(0.11, 0.11, 0.12),
            field_border:          Color::srgb(0.62, 0.62, 0.62),
            field_text:            Color::srgb(0.78, 0.82, 0.88),
            tooltip_border:        Color::srgba(0.58, 0.59, 0.61, 0.95),
            track:                 Color::srgba(1.0, 1.0, 1.0, 0.27),
            slider_cabled_fill:    Color::srgb(0.0, 0.22, 0.58),
            slider_reachable_band: Color::srgb(1.0, 1.0, 1.0),
            switch_border:         Color::srgb(0.62, 0.65, 0.70),
            switch_off_track:      Color::srgb(0.40, 0.43, 0.47),
            knob:                  Color::srgb(1.0, 1.0, 1.0),
            slider_knob:           Color::srgb(229.0 / 255.0, 232.0 / 255.0, 234.0 / 255.0),
            slider_knob_rim:       Color::srgb(1.0, 1.0, 1.0),
            pushed_value_marker:   Color::srgb(0.28, 0.85, 0.98),
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::Color;

    use super::ControlPalette;

    /// Minimum contrast for a visible control boundary against its host surface.
    const CONTROL_CONTRAST: f32 = 3.0;
    /// WCAG 2 AA minimum contrast for body text.
    const AA_TEXT_CONTRAST: f32 = 4.5;

    /// Alpha-composites `foreground` over `background` in the renderer's linear space.
    fn composite(foreground: Color, background: Color) -> Color {
        let foreground = foreground.to_linear();
        let background = background.to_linear();
        let remaining = 1.0 - foreground.alpha;
        let alpha = background.alpha.mul_add(remaining, foreground.alpha);
        Color::linear_rgba(
            foreground.red.mul_add(
                foreground.alpha,
                background.red * background.alpha * remaining,
            ) / alpha,
            foreground.green.mul_add(
                foreground.alpha,
                background.green * background.alpha * remaining,
            ) / alpha,
            foreground.blue.mul_add(
                foreground.alpha,
                background.blue * background.alpha * remaining,
            ) / alpha,
            alpha,
        )
    }

    /// WCAG relative luminance of an sRGB color.
    fn relative_luminance(color: Color) -> f32 {
        let srgba = color.to_srgba();
        let linear = |channel: f32| {
            if channel <= 0.040_45 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126_f32.mul_add(
            linear(srgba.red),
            0.7152_f32.mul_add(linear(srgba.green), 0.0722 * linear(srgba.blue)),
        )
    }

    /// WCAG contrast ratio between two opaque colors.
    fn contrast_ratio(first: Color, second: Color) -> f32 {
        let first = relative_luminance(first);
        let second = relative_luminance(second);
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    #[test]
    fn white_label_on_the_accent_behind_text_reads_at_aa_contrast() {
        let palette = ControlPalette::system_dark();
        let contrast = contrast_ratio(Color::WHITE, palette.accent_behind_text);

        assert!(
            contrast >= AA_TEXT_CONTRAST,
            "white on the accent behind text measures {contrast:.2}:1"
        );
    }

    #[test]
    fn slider_track_reaches_control_contrast_on_every_host_panel() {
        let palette = ControlPalette::system_dark();
        for (name, host) in [
            ("jack panel group", Color::srgb(0.19, 0.215, 0.26)),
            (
                "info panel",
                Color::srgb(46.0 / 255.0, 52.0 / 255.0, 64.0 / 255.0),
            ),
            ("widgets example", Color::srgba(0.02, 0.03, 0.07, 0.92)),
        ] {
            let opaque_host = composite(host, Color::BLACK);
            let drawn_track = composite(palette.track, opaque_host);
            let contrast = contrast_ratio(drawn_track, opaque_host);
            assert!(
                contrast >= CONTROL_CONTRAST,
                "slider track on {name} measures {contrast:.2}:1"
            );
        }
    }
}
