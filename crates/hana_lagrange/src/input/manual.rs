use bevy::ecs::query::QueryEntityError;
use bevy::ecs::system::SystemParam;
use bevy::prelude::Entity;
use bevy::prelude::Mut;
use bevy::prelude::Query;
use bevy::prelude::With;

use super::CameraManual;
use super::CoarseZoomDelta;
use super::FreeCamInput;
use super::LookDelta;
use super::ManualInputSource;
use super::OrbitCamInput;
use super::OrbitDelta;
use super::PanDelta;
use super::RollDelta;
use super::SmoothZoomDelta;
use super::TranslateDelta;
use crate::FreeCamKind;
use crate::OrbitCamKind;

/// System parameter for writing app-authored manual `OrbitCamInput`.
#[derive(SystemParam)]
pub struct OrbitCamManualInputWriter<'w, 's> {
    inputs: Query<'w, 's, &'static mut OrbitCamInput, With<CameraManual<OrbitCamKind>>>,
}

impl OrbitCamManualInputWriter<'_, '_> {
    /// Returns a manual input writer for `camera`.
    ///
    /// # Errors
    ///
    /// Returns [`QueryEntityError`] when `camera` is not in manual input mode, does
    /// not have an `OrbitCamInput` component, or cannot be queried.
    pub fn get_mut(
        &mut self,
        camera: Entity,
        source: ManualInputSource,
    ) -> Result<OrbitCamManualInput<'_>, QueryEntityError> {
        self.inputs
            .get_mut(camera)
            .map(|input| OrbitCamManualInput { input, source })
    }
}

/// Mutable manual input handle for a single camera.
pub struct OrbitCamManualInput<'input> {
    input:  Mut<'input, OrbitCamInput>,
    source: ManualInputSource,
}

impl OrbitCamManualInput<'_> {
    /// Clears all camera intent for the frame.
    pub fn clear(&mut self) -> &mut Self {
        self.input.clear();
        self
    }

    /// Adds orbit intent in logical pixels.
    pub fn orbit(&mut self, delta: impl Into<OrbitDelta>) -> &mut Self {
        self.input.add_orbit_from_source(delta, self.source);
        self
    }

    /// Marks orbit input active for the frame without adding motion.
    pub fn mark_orbit_active(&mut self) -> &mut Self {
        self.input
            .mark_orbit_active_with_sources(self.source.sources());
        self
    }

    /// Adds pan intent in logical pixels.
    pub fn pan(&mut self, delta: impl Into<PanDelta>) -> &mut Self {
        self.input.add_pan_from_source(delta, self.source);
        self
    }

    /// Marks pan input active for the frame without adding motion.
    pub fn mark_pan_active(&mut self) -> &mut Self {
        self.input
            .mark_pan_active_with_sources(self.source.sources());
        self
    }

    /// Adds coarse zoom intent.
    pub fn zoom_coarse(&mut self, delta: impl Into<CoarseZoomDelta>) -> &mut Self {
        self.input.add_zoom_coarse_from_source(delta, self.source);
        self
    }

    /// Adds smooth zoom intent.
    pub fn zoom_smooth(&mut self, delta: impl Into<SmoothZoomDelta>) -> &mut Self {
        self.input.add_zoom_smooth_from_source(delta, self.source);
        self
    }

    /// Marks zoom input active for the frame without adding zoom.
    pub fn mark_zoom_active(&mut self) -> &mut Self {
        self.input
            .mark_zoom_active_with_sources(self.source.sources());
        self
    }
}

/// System parameter for writing app-authored manual `FreeCamInput`.
#[derive(SystemParam)]
pub struct FreeCamManualInputWriter<'w, 's> {
    inputs: Query<'w, 's, &'static mut FreeCamInput, With<CameraManual<FreeCamKind>>>,
}

impl FreeCamManualInputWriter<'_, '_> {
    /// Returns a manual input writer for `camera`.
    ///
    /// # Errors
    ///
    /// Returns [`QueryEntityError`] when `camera` is not in manual input mode, does
    /// not have a `FreeCamInput` component, or cannot be queried.
    pub fn get_mut(
        &mut self,
        camera: Entity,
        source: ManualInputSource,
    ) -> Result<FreeCamManualInput<'_>, QueryEntityError> {
        self.inputs
            .get_mut(camera)
            .map(|input| FreeCamManualInput { input, source })
    }
}

/// Mutable manual input handle for a single `FreeCam`.
pub struct FreeCamManualInput<'input> {
    input:  Mut<'input, FreeCamInput>,
    source: ManualInputSource,
}

impl FreeCamManualInput<'_> {
    /// Clears all camera intent for the frame.
    pub fn clear(&mut self) -> &mut Self {
        self.input.clear();
        self
    }

    /// Adds translation intent in controller-local axes.
    pub fn translate(&mut self, delta: impl Into<TranslateDelta>) -> &mut Self {
        self.input.add_translate_from_source(delta, self.source);
        self
    }

    /// Marks translation input active for the frame without adding motion.
    pub fn mark_translate_active(&mut self) -> &mut Self {
        self.input
            .mark_translate_active_with_sources(self.source.sources());
        self
    }

    /// Adds look intent in logical pixels.
    pub fn look(&mut self, delta: impl Into<LookDelta>) -> &mut Self {
        self.input.add_look_from_source(delta, self.source);
        self
    }

    /// Marks look input active for the frame without adding motion.
    pub fn mark_look_active(&mut self) -> &mut Self {
        self.input
            .mark_look_active_with_sources(self.source.sources());
        self
    }

    /// Adds roll intent.
    pub fn roll(&mut self, delta: impl Into<RollDelta>) -> &mut Self {
        self.input.add_roll_from_source(delta, self.source);
        self
    }

    /// Marks roll input active for the frame without adding roll.
    pub fn mark_roll_active(&mut self) -> &mut Self {
        self.input
            .mark_roll_active_with_sources(self.source.sources());
        self
    }
}
