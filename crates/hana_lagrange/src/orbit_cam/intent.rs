//! `OrbitCam` input-intent vocabulary and channel access.

use core::ops::AddAssign;

use bevy::prelude::Reflect;
use bevy::prelude::Vec2;

use super::OrbitCamKind;
use crate::input::CameraInputKind;
use crate::input::ControlSpeed;
use crate::input::InputIntent;
use crate::input::IntentChannel;
use crate::input::IntentChannels;
use crate::input::InteractionSources;
use crate::input::ManualInputSource;
use crate::input::OrbitCamInputContext;

/// Orbit motion expressed in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct OrbitDelta(Vec2);

impl AddAssign for OrbitDelta {
    fn add_assign(&mut self, delta: Self) { self.0 += delta.0; }
}

impl From<Vec2> for OrbitDelta {
    fn from(value: Vec2) -> Self { Self(value) }
}

impl OrbitDelta {
    /// Returns the logical-pixel delta.
    #[must_use]
    pub const fn pixels(self) -> Vec2 { self.0 }
}

/// Pan motion expressed in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct PanDelta(Vec2);

impl AddAssign for PanDelta {
    fn add_assign(&mut self, delta: Self) { self.0 += delta.0; }
}

impl From<Vec2> for PanDelta {
    fn from(value: Vec2) -> Self { Self(value) }
}

impl PanDelta {
    /// Returns the logical-pixel delta.
    #[must_use]
    pub const fn pixels(self) -> Vec2 { self.0 }
}

/// Coarse zoom amount from stepped sources such as mouse wheels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct CoarseZoomDelta(f32);

impl AddAssign for CoarseZoomDelta {
    fn add_assign(&mut self, delta: Self) { self.0 += delta.0; }
}

impl From<f32> for CoarseZoomDelta {
    fn from(value: f32) -> Self { Self(value) }
}

impl CoarseZoomDelta {
    /// Returns the zoom amount.
    #[must_use]
    pub const fn amount(self) -> f32 { self.0 }
}

/// Smooth zoom amount from continuous sources such as pixel scroll or pinch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct SmoothZoomDelta(f32);

impl AddAssign for SmoothZoomDelta {
    fn add_assign(&mut self, delta: Self) { self.0 += delta.0; }
}

impl From<f32> for SmoothZoomDelta {
    fn from(value: f32) -> Self { Self(value) }
}

impl SmoothZoomDelta {
    /// Returns the zoom amount.
    #[must_use]
    pub const fn amount(self) -> f32 { self.0 }
}

/// Zoom intent with separate stepped and continuous deltas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct ZoomDelta {
    coarse: CoarseZoomDelta,
    smooth: SmoothZoomDelta,
}

impl AddAssign for ZoomDelta {
    fn add_assign(&mut self, delta: Self) {
        self.coarse += delta.coarse;
        self.smooth += delta.smooth;
    }
}

impl From<CoarseZoomDelta> for ZoomDelta {
    fn from(coarse: CoarseZoomDelta) -> Self {
        Self {
            coarse,
            smooth: SmoothZoomDelta::default(),
        }
    }
}

impl From<SmoothZoomDelta> for ZoomDelta {
    fn from(smooth: SmoothZoomDelta) -> Self {
        Self {
            coarse: CoarseZoomDelta::default(),
            smooth,
        }
    }
}

impl ZoomDelta {
    /// Returns the stepped zoom delta.
    #[must_use]
    pub const fn coarse(self) -> CoarseZoomDelta { self.coarse }

    /// Returns the continuous zoom delta.
    #[must_use]
    pub const fn smooth(self) -> SmoothZoomDelta { self.smooth }
}

impl CameraInputKind for OrbitCamKind {
    type Context = OrbitCamInputContext;
    type Input = OrbitCamInput;
    type Channels = OrbitCamChannels;
}

/// Named input channels consumed by the `OrbitCam` controller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct OrbitCamChannels {
    orbit: IntentChannel<OrbitDelta>,
    pan:   IntentChannel<PanDelta>,
    zoom:  IntentChannel<ZoomDelta>,
}

impl OrbitCamChannels {
    const fn has_input(self) -> bool {
        self.orbit.is_active() || self.pan.is_active() || self.zoom.is_active()
    }

    const fn sources(self) -> InteractionSources {
        self.orbit
            .sources()
            .union(self.pan.sources())
            .union(self.zoom.sources())
    }
}

impl IntentChannels for OrbitCamChannels {
    fn clear(&mut self) { *self = Self::default(); }

    fn has_input(&self) -> bool { (*self).has_input() }

    fn sources(&self) -> InteractionSources { (*self).sources() }
}

/// Semantic per-frame camera input consumed by the `OrbitCam` controller.
pub type OrbitCamInput = InputIntent<OrbitCamKind>;

impl InputIntent<OrbitCamKind> {
    /// Returns the orbit delta.
    #[must_use]
    pub const fn orbit(&self) -> OrbitDelta { self.channels().orbit.delta() }

    /// Returns the pan delta.
    #[must_use]
    pub const fn pan(&self) -> PanDelta { self.channels().pan.delta() }

    /// Returns the coarse zoom delta.
    #[must_use]
    pub const fn zoom_coarse(&self) -> CoarseZoomDelta { self.channels().zoom.delta().coarse() }

    /// Returns the smooth zoom delta.
    #[must_use]
    pub const fn zoom_smooth(&self) -> SmoothZoomDelta { self.channels().zoom.delta().smooth() }

    /// Returns active sources for orbit input this frame.
    #[must_use]
    pub const fn orbit_sources(&self) -> InteractionSources { self.channels().orbit.sources() }

    /// Returns active sources for pan input this frame.
    #[must_use]
    pub const fn pan_sources(&self) -> InteractionSources { self.channels().pan.sources() }

    /// Returns active sources for zoom input this frame.
    #[must_use]
    pub const fn zoom_sources(&self) -> InteractionSources { self.channels().zoom.sources() }

    /// Returns the speed variant of the orbit input this frame.
    #[must_use]
    pub const fn orbit_speed(&self) -> ControlSpeed { self.channels().orbit.speed() }

    /// Returns the speed variant of the pan input this frame.
    #[must_use]
    pub const fn pan_speed(&self) -> ControlSpeed { self.channels().pan.speed() }

    /// Returns the speed variant of the zoom input this frame.
    #[must_use]
    pub const fn zoom_speed(&self) -> ControlSpeed { self.channels().zoom.speed() }

    /// Returns `true` when the frame carries orbit intent.
    #[must_use]
    pub const fn has_orbit(&self) -> bool { self.channels().orbit.is_active() }

    /// Returns `true` when the frame carries pan intent.
    #[must_use]
    pub const fn has_pan(&self) -> bool { self.channels().pan.is_active() }

    /// Returns `true` when the frame carries zoom intent.
    #[must_use]
    pub const fn has_zoom(&self) -> bool { self.channels().zoom.is_active() }
}

impl InputIntent<OrbitCamKind> {
    pub(super) const fn set_orbit_speed(&mut self, speed: ControlSpeed) {
        self.channels_mut().orbit.set_speed(speed);
    }

    pub(super) const fn set_pan_speed(&mut self, speed: ControlSpeed) {
        self.channels_mut().pan.set_speed(speed);
    }

    pub(super) const fn set_zoom_speed(&mut self, speed: ControlSpeed) {
        self.channels_mut().zoom.set_speed(speed);
    }

    pub(crate) fn add_orbit_from_source(
        &mut self,
        delta: impl Into<OrbitDelta>,
        source: ManualInputSource,
    ) -> &mut Self {
        self.add_orbit_with_sources(delta, source.sources())
    }

    pub(crate) fn add_orbit_with_sources(
        &mut self,
        delta: impl Into<OrbitDelta>,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().orbit.add_delta(delta, sources);
        self
    }

    pub(crate) const fn mark_orbit_active_with_sources(
        &mut self,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().orbit.mark_active_with_sources(sources);
        self
    }

    pub(crate) fn add_pan_from_source(
        &mut self,
        delta: impl Into<PanDelta>,
        source: ManualInputSource,
    ) -> &mut Self {
        self.add_pan_with_sources(delta, source.sources())
    }

    pub(super) fn add_pan_with_sources(
        &mut self,
        delta: impl Into<PanDelta>,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().pan.add_delta(delta, sources);
        self
    }

    pub(crate) const fn mark_pan_active_with_sources(
        &mut self,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().pan.mark_active_with_sources(sources);
        self
    }

    pub(crate) fn add_zoom_coarse_from_source(
        &mut self,
        delta: impl Into<CoarseZoomDelta>,
        source: ManualInputSource,
    ) -> &mut Self {
        self.add_zoom_coarse_with_sources(delta, source.sources())
    }

    pub(crate) fn add_zoom_coarse_with_sources(
        &mut self,
        delta: impl Into<CoarseZoomDelta>,
        sources: InteractionSources,
    ) -> &mut Self {
        let delta = delta.into();
        self.channels_mut()
            .zoom
            .add_delta(ZoomDelta::from(delta), sources);
        self
    }

    pub(crate) fn add_zoom_smooth_from_source(
        &mut self,
        delta: impl Into<SmoothZoomDelta>,
        source: ManualInputSource,
    ) -> &mut Self {
        self.add_zoom_smooth_with_sources(delta, source.sources())
    }

    pub(super) fn add_zoom_smooth_with_sources(
        &mut self,
        delta: impl Into<SmoothZoomDelta>,
        sources: InteractionSources,
    ) -> &mut Self {
        let delta = delta.into();
        self.channels_mut()
            .zoom
            .add_delta(ZoomDelta::from(delta), sources);
        self
    }

    pub(crate) const fn mark_zoom_active_with_sources(
        &mut self,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().zoom.mark_active_with_sources(sources);
        self
    }

    pub(crate) fn clear_orbit(&mut self) -> &mut Self {
        self.channels_mut().orbit.clear();
        self
    }

    pub(crate) fn clear_pan(&mut self) -> &mut Self {
        self.channels_mut().pan.clear();
        self
    }
}
