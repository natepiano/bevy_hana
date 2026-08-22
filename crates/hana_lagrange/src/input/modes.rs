use std::fmt;
use std::marker::PhantomData;

use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::reflect::FromReflect;
use bevy::reflect::TypePath;

use super::CameraControlSummary;
use super::CameraInputKind;
use super::FreeCamBindings;
use super::FreeCamPreset;
use super::OrbitCamBindings;
use super::OrbitCamPreset;
use super::control_summary;
use crate::FreeCam;
use crate::FreeCamKind;
use crate::OrbitCamKind;

mod sealed {
    pub trait Sealed {}
}

use sealed::Sealed;

/// Camera-kind input mode type family.
///
/// This trait is sealed; implementers are the crate-defined [`OrbitCamKind`]
/// and [`FreeCamKind`]. Camera kinds are defined by this crate.
///
/// [`FreeCamKind`]: crate::FreeCamKind
/// [`OrbitCamKind`]: crate::OrbitCamKind
pub trait CameraInputModeKind: CameraInputKind + Sealed {
    /// Built-in preset payload for this camera kind.
    type Preset: Clone + fmt::Debug + PartialEq + FromReflect + Reflect + TypePath;
    /// App-authored binding payload for this camera kind.
    type Bindings: Clone + fmt::Debug + PartialEq + FromReflect + Reflect + TypePath;

    /// Returns the default input mode for this camera kind.
    fn default_mode() -> InputMode<Self>;

    /// Describes the effective controls for this input mode.
    fn describe_controls(mode: &InputMode<Self>) -> CameraControlSummary;

    /// Describes the effective controls for this camera and input mode.
    fn describe_controls_for(_: &Self::Camera, mode: &InputMode<Self>) -> CameraControlSummary {
        Self::describe_controls(mode)
    }
}

/// Selected input mode for one camera kind.
///
/// Camera components require a concrete alias such as [`OrbitCamInputMode`] or
/// [`FreeCamInputMode`]. Use `Preset` for a built-in keymap, `Bindings` for
/// app-owned validated bindings, or `Manual` when app code writes camera intent.
#[derive(Component, Clone, Debug, PartialEq, Reflect)]
#[reflect(
    Component,
    Default,
    where K::Preset: FromReflect + TypePath,
          K::Bindings: FromReflect + TypePath
)]
#[non_exhaustive]
pub enum InputMode<K: CameraInputModeKind> {
    /// Built-in preset mode.
    Preset(K::Preset),
    /// Custom validated bindings mode.
    Bindings(K::Bindings),
    /// Manual mode where app code writes camera intent.
    Manual,
}

impl<K: CameraInputModeKind> InputMode<K> {
    /// Builds preset input mode from a built-in camera preset.
    #[must_use]
    pub fn with_preset(preset: impl Into<K::Preset>) -> Self { Self::Preset(preset.into()) }
}

impl<K: CameraInputModeKind> Default for InputMode<K> {
    fn default() -> Self { K::default_mode() }
}

impl CameraInputModeKind for OrbitCamKind {
    type Preset = OrbitCamPreset;
    type Bindings = OrbitCamBindings;

    fn default_mode() -> InputMode<Self> { InputMode::with_preset(OrbitCamPreset::default()) }

    fn describe_controls(mode: &InputMode<Self>) -> CameraControlSummary {
        control_summary::describe_orbit_camera_controls(mode)
    }
}

impl From<OrbitCamPreset> for OrbitCamInputMode {
    fn from(preset: OrbitCamPreset) -> Self { Self::with_preset(preset) }
}

impl From<OrbitCamBindings> for OrbitCamInputMode {
    fn from(bindings: OrbitCamBindings) -> Self { Self::Bindings(bindings) }
}

/// Selected input mode for an [`crate::OrbitCam`].
///
/// `OrbitCam` requires this component and defaults to simple mouse preset input.
/// Use `Preset` for a built-in keymap, `Bindings` for app-owned validated
/// bindings, or `Manual` when app code writes camera intent through
/// [`OrbitCamManualInputWriter`].
///
/// [`OrbitCamManualInputWriter`]: super::OrbitCamManualInputWriter
pub type OrbitCamInputMode = InputMode<OrbitCamKind>;

impl CameraInputModeKind for FreeCamKind {
    type Preset = FreeCamPreset;
    type Bindings = FreeCamBindings;

    fn default_mode() -> InputMode<Self> { InputMode::with_preset(FreeCamPreset::default()) }

    fn describe_controls(mode: &InputMode<Self>) -> CameraControlSummary {
        control_summary::describe_free_cam_controls(mode)
    }

    fn describe_controls_for(camera: &FreeCam, mode: &InputMode<Self>) -> CameraControlSummary {
        control_summary::describe_free_cam_controls_for(camera, mode)
    }
}

/// Selected input mode for a [`FreeCam`](crate::FreeCam).
///
/// `FreeCam` requires this component and defaults to mouse-and-keyboard preset
/// input. Use `Preset` for a built-in keymap, `Bindings` for app-owned
/// bindings, or `Manual` when app code writes camera intent through
/// [`FreeCamManualInputWriter`](super::FreeCamManualInputWriter).
pub type FreeCamInputMode = InputMode<FreeCamKind>;

/// Runtime marker for cameras using manual app-authored input.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CameraManual<K: CameraInputModeKind> {
    marker: PhantomData<fn() -> K>,
}

impl<K: CameraInputModeKind> Default for CameraManual<K> {
    fn default() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

impl Sealed for OrbitCamKind {}
impl Sealed for FreeCamKind {}
