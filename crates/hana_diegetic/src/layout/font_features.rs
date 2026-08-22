//! OpenType feature configuration for layout text.

use bevy::prelude::Reflect;
use bitflags::bitflags;

use super::constants::CALT_TAG;
use super::constants::DLIG_TAG;
use super::constants::KERN_TAG;
use super::constants::LIGA_TAG;

bitflags! {
    /// Named flags for common OpenType features.
    ///
    /// Used inside [`FontFeatures`] to specify which features are
    /// explicitly enabled or disabled during text shaping.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct FontFeatureFlags: u16 {
        /// Standard ligatures (`liga`): fi, fl, ffi, ffl.
        const LIGA = 1 << 0;
        /// Contextual alternates (`calt`): ->, =>, ::, !=.
        const CALT = 1 << 1;
        /// Discretionary ligatures (`dlig`): decorative alternates.
        const DLIG = 1 << 2;
        /// Kerning (`kern`): inter-character spacing adjustments.
        const KERN = 1 << 3;
    }
}

/// OpenType feature overrides for text shaping.
///
/// Controls which OpenType features are explicitly enabled or disabled
/// during text shaping. Features not overridden use the shaper's defaults
/// (`HarfBuzz` enables `liga`, `calt`, `kern` by default; `dlig` is off).
///
/// ```ignore
/// // Disable contextual alternates (coding ligatures):
/// let features = FontFeatures::new()
///     .without(FontFeatureFlags::CALT);
///
/// // Enable discretionary ligatures:
/// let features = FontFeatures::new()
///     .with(FontFeatureFlags::DLIG);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
pub struct FontFeatures {
    /// Features explicitly forced on.
    #[reflect(ignore)]
    enabled:  FontFeatureFlags,
    /// Features explicitly forced off.
    #[reflect(ignore)]
    disabled: FontFeatureFlags,
}

impl FontFeatures {
    /// No overrides — all features use shaper defaults.
    pub const NONE: Self = Self {
        enabled:  FontFeatureFlags::empty(),
        disabled: FontFeatureFlags::empty(),
    };

    /// Creates a new `FontFeatures` with no overrides.
    #[must_use]
    pub const fn new() -> Self { Self::NONE }

    /// Returns `true` if no features are overridden.
    #[must_use]
    pub const fn is_default(&self) -> bool { self.enabled.is_empty() && self.disabled.is_empty() }

    /// Explicitly enables the given feature(s).
    #[must_use]
    pub const fn with(mut self, flags: FontFeatureFlags) -> Self {
        self.enabled = self.enabled.union(flags);
        self.disabled = self.disabled.difference(flags);
        self
    }

    /// Explicitly disables the given feature(s).
    #[must_use]
    pub const fn without(mut self, flags: FontFeatureFlags) -> Self {
        self.disabled = self.disabled.union(flags);
        self.enabled = self.enabled.difference(flags);
        self
    }

    /// Converts to parley font feature settings.
    ///
    /// Returns a `Vec` of `(tag_bytes, value)` pairs. Only features
    /// with explicit overrides are included — the shaper's defaults
    /// handle everything else.
    #[must_use]
    pub fn to_parley_settings(&self) -> Vec<([u8; 4], u16)> {
        let mut settings = Vec::new();

        let all_flags = [
            (FontFeatureFlags::LIGA, LIGA_TAG),
            (FontFeatureFlags::CALT, CALT_TAG),
            (FontFeatureFlags::DLIG, DLIG_TAG),
            (FontFeatureFlags::KERN, KERN_TAG),
        ];

        for (flag, tag) in all_flags {
            if self.enabled.contains(flag) {
                settings.push((tag, 1));
            } else if self.disabled.contains(flag) {
                settings.push((tag, 0));
            }
        }

        settings
    }
}
