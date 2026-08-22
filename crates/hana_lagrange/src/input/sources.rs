use bevy::prelude::Reflect;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct CameraInteractionSourceBits: u16 {
        const MOUSE = 1 << 0;
        const WHEEL = 1 << 1;
        const SMOOTH_SCROLL = 1 << 2;
        const PINCH = 1 << 3;
        const TOUCH = 1 << 4;
        const KEYBOARD = 1 << 5;
        const GAMEPAD = 1 << 6;
        const MANUAL = 1 << 7;
    }
}

/// Source-attribution flags for camera interaction lifecycle events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct InteractionSources {
    bits: u16,
}

impl InteractionSources {
    /// Empty source set.
    pub const NONE: Self = Self::from_source_bits(CameraInteractionSourceBits::empty());
    /// Mouse button or mouse-motion source.
    pub const MOUSE: Self = Self::from_source_bits(CameraInteractionSourceBits::MOUSE);
    /// Mouse wheel line-step source.
    pub const WHEEL: Self = Self::from_source_bits(CameraInteractionSourceBits::WHEEL);
    /// Smooth pixel-scroll source.
    pub const SMOOTH_SCROLL: Self =
        Self::from_source_bits(CameraInteractionSourceBits::SMOOTH_SCROLL);
    /// Pinch gesture source.
    pub const PINCH: Self = Self::from_source_bits(CameraInteractionSourceBits::PINCH);
    /// Touch gesture source.
    pub const TOUCH: Self = Self::from_source_bits(CameraInteractionSourceBits::TOUCH);
    /// Keyboard source.
    pub const KEYBOARD: Self = Self::from_source_bits(CameraInteractionSourceBits::KEYBOARD);
    /// Gamepad source.
    pub const GAMEPAD: Self = Self::from_source_bits(CameraInteractionSourceBits::GAMEPAD);
    /// App-authored manual source.
    pub const MANUAL: Self = Self::from_source_bits(CameraInteractionSourceBits::MANUAL);

    const fn from_source_bits(source_bits: CameraInteractionSourceBits) -> Self {
        Self {
            bits: source_bits.bits(),
        }
    }

    /// Returns `true` when this set has no sources.
    #[must_use]
    pub const fn is_empty(self) -> bool { self.bits == Self::NONE.bits }

    /// Returns `true` when `other` is fully contained in this set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool { self.bits & other.bits == other.bits }

    /// Returns `true` when this set shares at least one source with `other`.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool { self.bits & other.bits != Self::NONE.bits }

    /// Returns a set containing sources from both sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    /// Returns this set without any sources from `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self {
            bits: self.bits & !other.bits,
        }
    }
}

/// Source token for app-authored manual camera input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManualInputSource {
    sources: InteractionSources,
}

impl Default for ManualInputSource {
    fn default() -> Self { Self::manual() }
}

impl ManualInputSource {
    /// Creates the default manual source token.
    #[must_use]
    pub const fn manual() -> Self {
        Self {
            sources: InteractionSources::MANUAL,
        }
    }

    /// Creates a manual source token with additional attribution.
    #[must_use]
    pub const fn with_sources(sources: InteractionSources) -> Self {
        Self {
            sources: sources.union(InteractionSources::MANUAL),
        }
    }

    /// Creates a manual source token for keyboard-driven app code.
    #[must_use]
    pub const fn observed_keyboard() -> Self { Self::with_sources(InteractionSources::KEYBOARD) }

    /// Creates a manual source token for gamepad-driven app code.
    #[must_use]
    pub const fn observed_gamepad() -> Self { Self::with_sources(InteractionSources::GAMEPAD) }

    /// Returns the source set, always including [`InteractionSources::MANUAL`].
    #[must_use]
    pub const fn sources(self) -> InteractionSources { self.sources }
}
