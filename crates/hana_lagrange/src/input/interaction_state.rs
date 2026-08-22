use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;

use super::ControlSpeed;
use super::FreeCamInteractionKind;
use super::InteractionSources;
use super::OrbitCamInteractionKind;
use super::ZoomDirection;

/// A decomposed `FreeCam` movement affordance that a control-summary row can
/// represent.
///
/// A gamepad move binding fuses the stick, boost gate, and vertical triggers
/// into one action; tagging each decomposed row with its direction lets a panel
/// light only the affordance currently engaged instead of every row that shares
/// the `Translate` or `Roll` action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum FreeCamControlDirection {
    /// Horizontal move-stick input at normal speed.
    Stick,
    /// Horizontal move-stick input while the boost gate is held.
    Boost,
    /// Upward vertical input.
    Up,
    /// Downward vertical input.
    Down,
    /// Roll toward the left.
    RollLeft,
    /// Roll toward the right.
    RollRight,
}

impl FreeCamControlDirection {
    const fn bit(self) -> u8 {
        match self {
            Self::Stick => 1 << 0,
            Self::Boost => 1 << 1,
            Self::Up => 1 << 2,
            Self::Down => 1 << 3,
            Self::RollLeft => 1 << 4,
            Self::RollRight => 1 << 5,
        }
    }
}

/// The set of [`FreeCamControlDirection`]s engaged in a reported `FreeCam`
/// interaction.
///
/// Populated per frame from the resolved move vector, boost gate, and roll
/// sign so a control panel lights only the rows whose direction is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct FreeCamActiveDirections {
    bits: u8,
}

impl FreeCamActiveDirections {
    /// No direction engaged.
    pub const NONE: Self = Self { bits: 0 };

    /// Returns `true` when no direction is engaged.
    #[must_use]
    pub const fn is_empty(self) -> bool { self.bits == 0 }

    /// Returns `true` when `direction` is engaged.
    #[must_use]
    pub const fn contains(self, direction: FreeCamControlDirection) -> bool {
        self.bits & direction.bit() != 0
    }

    /// Returns this set with `direction` added.
    #[must_use]
    pub const fn with(self, direction: FreeCamControlDirection) -> Self {
        Self {
            bits: self.bits | direction.bit(),
        }
    }
}

/// Read-only state describing the active interaction for an `OrbitCam`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Component, Default)]
pub struct OrbitCamInteractionState {
    orbit:          InteractionSources,
    pan:            InteractionSources,
    zoom:           InteractionSources,
    orbit_speed:    Option<ControlSpeed>,
    pan_speed:      Option<ControlSpeed>,
    zoom_speed:     Option<ControlSpeed>,
    zoom_direction: Option<ZoomDirection>,
}

/// Read-only state describing the active interaction for a `FreeCam`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Component, Default)]
pub struct FreeCamInteractionState {
    translate:       InteractionSources,
    look:            InteractionSources,
    roll:            InteractionSources,
    translate_speed: Option<ControlSpeed>,
    look_speed:      Option<ControlSpeed>,
    roll_speed:      Option<ControlSpeed>,
    directions:      FreeCamActiveDirections,
}

impl FreeCamInteractionState {
    /// Returns `true` when any interaction is active.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !self.translate.is_empty() || !self.look.is_empty() || !self.roll.is_empty()
    }

    /// Returns `true` when `kind` is active.
    #[must_use]
    pub const fn is_kind_active(&self, kind: FreeCamInteractionKind) -> bool {
        !self.sources(kind).is_empty()
    }

    /// Returns the sources currently contributing to `kind`.
    #[must_use]
    pub const fn sources(&self, kind: FreeCamInteractionKind) -> InteractionSources {
        match kind {
            FreeCamInteractionKind::Translate => self.translate,
            FreeCamInteractionKind::Look => self.look,
            FreeCamInteractionKind::Roll => self.roll,
        }
    }

    /// Returns the sources currently contributing to translation input.
    #[must_use]
    pub const fn translate_sources(&self) -> InteractionSources { self.translate }

    /// Returns the sources currently contributing to look input.
    #[must_use]
    pub const fn look_sources(&self) -> InteractionSources { self.look }

    /// Returns the sources currently contributing to roll input.
    #[must_use]
    pub const fn roll_sources(&self) -> InteractionSources { self.roll }

    /// Returns the reported speed variant for `kind`.
    #[must_use]
    pub const fn speed(&self, kind: FreeCamInteractionKind) -> Option<ControlSpeed> {
        match kind {
            FreeCamInteractionKind::Translate => self.translate_speed,
            FreeCamInteractionKind::Look => self.look_speed,
            FreeCamInteractionKind::Roll => self.roll_speed,
        }
    }

    /// Returns the decomposed move directions currently engaged.
    #[must_use]
    pub const fn directions(&self) -> FreeCamActiveDirections { self.directions }

    /// Returns `true` when `direction` is engaged.
    #[must_use]
    pub const fn is_direction_active(&self, direction: FreeCamControlDirection) -> bool {
        self.directions.contains(direction)
    }

    pub(super) const fn set_directions(&mut self, directions: FreeCamActiveDirections) {
        self.directions = directions;
    }

    pub(super) const fn set_sources(
        &mut self,
        kind: FreeCamInteractionKind,
        sources: InteractionSources,
    ) {
        match kind {
            FreeCamInteractionKind::Translate => self.translate = sources,
            FreeCamInteractionKind::Look => self.look = sources,
            FreeCamInteractionKind::Roll => self.roll = sources,
        }
    }

    pub(super) const fn set_speed(
        &mut self,
        kind: FreeCamInteractionKind,
        speed: Option<ControlSpeed>,
    ) {
        match kind {
            FreeCamInteractionKind::Translate => self.translate_speed = speed,
            FreeCamInteractionKind::Look => self.look_speed = speed,
            FreeCamInteractionKind::Roll => self.roll_speed = speed,
        }
    }
}

impl OrbitCamInteractionState {
    /// Returns `true` when any interaction is active.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !self.orbit.is_empty() || !self.pan.is_empty() || !self.zoom.is_empty()
    }

    /// Returns `true` when `kind` is active.
    #[must_use]
    pub const fn is_kind_active(&self, kind: OrbitCamInteractionKind) -> bool {
        !self.sources(kind).is_empty()
    }

    /// Returns the sources currently contributing to `kind`.
    #[must_use]
    pub const fn sources(&self, kind: OrbitCamInteractionKind) -> InteractionSources {
        match kind {
            OrbitCamInteractionKind::Orbit => self.orbit,
            OrbitCamInteractionKind::Pan => self.pan,
            OrbitCamInteractionKind::Zoom => self.zoom,
        }
    }

    /// Returns the sources currently contributing to orbit input.
    #[must_use]
    pub const fn orbit_sources(&self) -> InteractionSources { self.orbit }

    /// Returns the sources currently contributing to pan input.
    #[must_use]
    pub const fn pan_sources(&self) -> InteractionSources { self.pan }

    /// Returns the sources currently contributing to zoom input.
    #[must_use]
    pub const fn zoom_sources(&self) -> InteractionSources { self.zoom }

    /// Returns the reported speed variant for `kind`, or `None` while a fresh
    /// gamepad interaction has not yet settled under the reporting-speed
    /// debounce. `Slow` is reported immediately; only the return to `Normal`
    /// waits out the settle window.
    #[must_use]
    pub const fn speed(&self, kind: OrbitCamInteractionKind) -> Option<ControlSpeed> {
        match kind {
            OrbitCamInteractionKind::Orbit => self.orbit_speed,
            OrbitCamInteractionKind::Pan => self.pan_speed,
            OrbitCamInteractionKind::Zoom => self.zoom_speed,
        }
    }

    /// Returns the direction of the active zoom interaction, or `None` when no
    /// zoom is active. Held through the reporting-debounce window so a control
    /// panel can light the engaged direction's row, and reversing direction
    /// updates it immediately without waiting on the debounce.
    #[must_use]
    pub const fn zoom_direction(&self) -> Option<ZoomDirection> { self.zoom_direction }

    pub(super) const fn set_sources(
        &mut self,
        kind: OrbitCamInteractionKind,
        sources: InteractionSources,
    ) {
        match kind {
            OrbitCamInteractionKind::Orbit => self.orbit = sources,
            OrbitCamInteractionKind::Pan => self.pan = sources,
            OrbitCamInteractionKind::Zoom => self.zoom = sources,
        }
    }

    pub(super) const fn set_speed(
        &mut self,
        kind: OrbitCamInteractionKind,
        speed: Option<ControlSpeed>,
    ) {
        match kind {
            OrbitCamInteractionKind::Orbit => self.orbit_speed = speed,
            OrbitCamInteractionKind::Pan => self.pan_speed = speed,
            OrbitCamInteractionKind::Zoom => self.zoom_speed = speed,
        }
    }

    pub(super) const fn set_zoom_direction(&mut self, zoom_direction: Option<ZoomDirection>) {
        self.zoom_direction = zoom_direction;
    }
}
