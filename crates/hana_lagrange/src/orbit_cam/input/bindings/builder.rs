//! Programmatic construction of [`super::OrbitCamBindings`].
//!
//! Types:
//! - [`OrbitCamBindingsBuilder`] — fluent builder that accumulates binding descriptors and runs
//!   [`super::validate::validate_bindings`] on `.build()`.
//! - Dispatch enums consumed by `.orbit()` / `.pan()` / `.zoom()` builder methods:
//!   [`OrbitCamOrbitBinding`] / [`OrbitCamPanBinding`] / [`OrbitCamZoomBinding`].

use bevy::prelude::Reflect;
use bevy_enhanced_input::prelude::Binding;

use super::OrbitCamBindings;
use super::binding_kinds::CameraInputGamepadSelectionPolicy;
use super::binding_kinds::OrbitCamBindingWithInputGain;
use super::binding_kinds::OrbitCamButtonDragZoom;
use super::binding_kinds::OrbitCamLineScroll;
use super::binding_kinds::OrbitCamMouseDrag;
use super::binding_kinds::OrbitCamMouseWheelZoom;
use super::binding_kinds::OrbitCamPinchZoom;
use super::binding_kinds::OrbitCamTouchBinding;
use super::binding_kinds::OrbitCamTouchBindingConfig;
use super::binding_kinds::OrbitCamTrackpadScroll;
use super::binding_kinds::ZoomInversion;
use super::validate;
use crate::input::ActionBindingDescriptor;
use crate::input::BindingsError;
use crate::input::CameraSlowMode;
use crate::input::HeldBinding;
use crate::input::HeldBindingDescriptor;
use crate::input::InputBinding;

/// Builder-internal accumulator of authored bindings, lowered to
/// [`OrbitCamBindings`] on `.build()`.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub(super) struct OrbitCamBindingsDescriptor {
    pub(super) orbit:            Vec<HeldBindingDescriptor>,
    pub(super) pan:              Vec<HeldBindingDescriptor>,
    pub(super) zoom_smooth:      Vec<HeldBindingDescriptor>,
    pub(super) zoom_coarse:      Vec<ActionBindingDescriptor>,
    pub(super) trackpad_orbit:   Vec<OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>>,
    pub(super) line_orbit:       Vec<OrbitCamBindingWithInputGain<OrbitCamLineScroll>>,
    pub(super) trackpad_pan:     Vec<OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>>,
    pub(super) line_pan:         Vec<OrbitCamBindingWithInputGain<OrbitCamLineScroll>>,
    pub(super) trackpad_zoom:    Vec<OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>>,
    pub(super) line_zoom:        Vec<OrbitCamBindingWithInputGain<OrbitCamLineScroll>>,
    pub(super) mouse_wheel_zoom: Option<OrbitCamBindingWithInputGain<OrbitCamMouseWheelZoom>>,
    pub(super) pinch_zoom:       Option<OrbitCamBindingWithInputGain<OrbitCamPinchZoom>>,
    pub(super) touch:            Option<OrbitCamTouchBindingConfig>,
    pub(super) gamepad:          CameraInputGamepadSelectionPolicy,
    pub(super) zoom_inversion:   ZoomInversion,
    pub(super) button_drag_zoom: Option<OrbitCamBindingWithInputGain<OrbitCamButtonDragZoom>>,
    pub(super) slow_mode:        Option<CameraSlowMode>,
    pub(super) home:             Vec<ActionBindingDescriptor>,
}

/// Builder for `OrbitCamBindings`.
#[derive(Clone, Debug, Default)]
pub struct OrbitCamBindingsBuilder {
    descriptor: OrbitCamBindingsDescriptor,
}

impl OrbitCamBindingsBuilder {
    /// Adds a binding that produces orbit intent.
    #[must_use]
    pub fn orbit(mut self, binding: impl Into<OrbitCamOrbitBinding>) -> Self {
        match binding.into() {
            OrbitCamOrbitBinding::Held(binding) => self.descriptor.orbit.push(binding.into()),
            OrbitCamOrbitBinding::Trackpad(binding) => self.descriptor.trackpad_orbit.push(binding),
            OrbitCamOrbitBinding::LineScroll(binding) => self.descriptor.line_orbit.push(binding),
        }
        self
    }

    /// Adds a binding that produces pan intent.
    #[must_use]
    pub fn pan(mut self, binding: impl Into<OrbitCamPanBinding>) -> Self {
        match binding.into() {
            OrbitCamPanBinding::Held(binding) => self.descriptor.pan.push(binding.into()),
            OrbitCamPanBinding::Trackpad(binding) => self.descriptor.trackpad_pan.push(binding),
            OrbitCamPanBinding::LineScroll(binding) => self.descriptor.line_pan.push(binding),
        }
        self
    }

    /// Adds a binding that produces zoom intent.
    ///
    /// Held and trackpad bindings are appended. Singleton adapter sources
    /// (mouse wheel, pinch, and button-drag zoom) use last-write-wins when the
    /// builder receives repeated calls for the same source.
    #[must_use]
    pub fn zoom(mut self, binding: impl Into<OrbitCamZoomBinding>) -> Self {
        match binding.into() {
            OrbitCamZoomBinding::Held(binding) => {
                self.descriptor.zoom_smooth.push(binding.into());
            },
            OrbitCamZoomBinding::Trackpad(binding) => self.descriptor.trackpad_zoom.push(binding),
            OrbitCamZoomBinding::LineScroll(binding) => self.descriptor.line_zoom.push(binding),
            OrbitCamZoomBinding::MouseWheel(binding) => {
                self.descriptor.mouse_wheel_zoom = Some(binding);
            },
            OrbitCamZoomBinding::Pinch(binding) => {
                self.descriptor.pinch_zoom = Some(binding);
            },
            OrbitCamZoomBinding::ButtonDrag(binding) => {
                self.descriptor.button_drag_zoom = Some(binding);
            },
        }
        self
    }

    /// Sets the touch policy.
    ///
    /// Repeated calls use last-write-wins.
    #[must_use]
    pub const fn touch(mut self, touch: Option<OrbitCamTouchBinding>) -> Self {
        self.descriptor.touch = match touch {
            Some(touch) => Some(OrbitCamTouchBindingConfig::new(touch)),
            None => None,
        };
        self
    }

    /// Sets the touch policy with explicit per-action input gain.
    ///
    /// Repeated calls use last-write-wins.
    #[must_use]
    pub const fn touch_config(mut self, touch: Option<OrbitCamTouchBindingConfig>) -> Self {
        self.descriptor.touch = touch;
        self
    }

    /// Sets the gamepad selection policy.
    #[must_use]
    pub const fn gamepad(mut self, gamepad: CameraInputGamepadSelectionPolicy) -> Self {
        self.descriptor.gamepad = gamepad;
        self
    }

    /// Adds a source (key or gamepad button) that resets the camera to its home pose.
    #[must_use]
    pub fn home(mut self, home: impl Into<Binding>) -> Self {
        self.descriptor
            .home
            .push(ActionBindingDescriptor::from(home.into()));
        self
    }

    /// Sets the zoom inversion policy.
    #[must_use]
    pub const fn zoom_inversion(mut self, zoom_inversion: ZoomInversion) -> Self {
        self.descriptor.zoom_inversion = zoom_inversion;
        self
    }

    /// Sets the slow-mode policy.
    #[must_use]
    pub const fn slow_mode(mut self, slow_mode: CameraSlowMode) -> Self {
        self.descriptor.slow_mode = Some(slow_mode);
        self
    }

    /// Builds validated `OrbitCamBindings`.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when the descriptor violates a binding
    /// invariant.
    pub fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        validate::validate_bindings(&self.descriptor)
    }
}

/// Binding that can produce orbit intent.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[non_exhaustive]
pub enum OrbitCamOrbitBinding {
    /// Held enhanced-input binding.
    Held(HeldBinding),
    /// Trackpad smooth-scroll binding.
    Trackpad(OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>),
    /// Line-scroll binding.
    LineScroll(OrbitCamBindingWithInputGain<OrbitCamLineScroll>),
}

impl From<HeldBinding> for OrbitCamOrbitBinding {
    fn from(value: HeldBinding) -> Self { Self::Held(value) }
}

impl From<OrbitCamMouseDrag> for OrbitCamOrbitBinding {
    fn from(value: OrbitCamMouseDrag) -> Self { Self::Held(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamMouseDrag>> for OrbitCamOrbitBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamMouseDrag>) -> Self {
        Self::Held(value.into())
    }
}

impl From<InputBinding> for OrbitCamOrbitBinding {
    fn from(value: InputBinding) -> Self { Self::Held(HeldBinding::same(value)) }
}

impl From<OrbitCamTrackpadScroll> for OrbitCamOrbitBinding {
    fn from(value: OrbitCamTrackpadScroll) -> Self { Self::Trackpad(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>> for OrbitCamOrbitBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>) -> Self {
        Self::Trackpad(value)
    }
}

/// Binding that can produce pan intent.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[non_exhaustive]
pub enum OrbitCamPanBinding {
    /// Held enhanced-input binding.
    Held(HeldBinding),
    /// Trackpad smooth-scroll binding.
    Trackpad(OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>),
    /// Line-scroll binding.
    LineScroll(OrbitCamBindingWithInputGain<OrbitCamLineScroll>),
}

impl From<HeldBinding> for OrbitCamPanBinding {
    fn from(value: HeldBinding) -> Self { Self::Held(value) }
}

impl From<OrbitCamMouseDrag> for OrbitCamPanBinding {
    fn from(value: OrbitCamMouseDrag) -> Self { Self::Held(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamMouseDrag>> for OrbitCamPanBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamMouseDrag>) -> Self {
        Self::Held(value.into())
    }
}

impl From<InputBinding> for OrbitCamPanBinding {
    fn from(value: InputBinding) -> Self { Self::Held(HeldBinding::same(value)) }
}

impl From<OrbitCamTrackpadScroll> for OrbitCamPanBinding {
    fn from(value: OrbitCamTrackpadScroll) -> Self { Self::Trackpad(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>> for OrbitCamPanBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>) -> Self {
        Self::Trackpad(value)
    }
}

/// Binding that can produce zoom intent.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[non_exhaustive]
pub enum OrbitCamZoomBinding {
    /// Held enhanced-input binding.
    Held(HeldBinding),
    /// Trackpad smooth-scroll binding.
    Trackpad(OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>),
    /// Line-scroll binding.
    LineScroll(OrbitCamBindingWithInputGain<OrbitCamLineScroll>),
    /// Mouse wheel zoom binding.
    MouseWheel(OrbitCamBindingWithInputGain<OrbitCamMouseWheelZoom>),
    /// Pinch gesture zoom binding.
    Pinch(OrbitCamBindingWithInputGain<OrbitCamPinchZoom>),
    /// Button-drag zoom binding.
    ButtonDrag(OrbitCamBindingWithInputGain<OrbitCamButtonDragZoom>),
}

impl From<HeldBinding> for OrbitCamZoomBinding {
    fn from(value: HeldBinding) -> Self { Self::Held(value) }
}

impl From<InputBinding> for OrbitCamZoomBinding {
    fn from(value: InputBinding) -> Self { Self::Held(HeldBinding::same(value)) }
}

impl From<OrbitCamTrackpadScroll> for OrbitCamZoomBinding {
    fn from(value: OrbitCamTrackpadScroll) -> Self { Self::Trackpad(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>> for OrbitCamZoomBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>) -> Self {
        Self::Trackpad(value)
    }
}

impl From<OrbitCamMouseWheelZoom> for OrbitCamZoomBinding {
    fn from(value: OrbitCamMouseWheelZoom) -> Self { Self::MouseWheel(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamMouseWheelZoom>> for OrbitCamZoomBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamMouseWheelZoom>) -> Self {
        Self::MouseWheel(value)
    }
}

impl From<OrbitCamPinchZoom> for OrbitCamZoomBinding {
    fn from(value: OrbitCamPinchZoom) -> Self { Self::Pinch(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamPinchZoom>> for OrbitCamZoomBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamPinchZoom>) -> Self { Self::Pinch(value) }
}

impl From<OrbitCamButtonDragZoom> for OrbitCamZoomBinding {
    fn from(value: OrbitCamButtonDragZoom) -> Self { Self::ButtonDrag(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamButtonDragZoom>> for OrbitCamZoomBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamButtonDragZoom>) -> Self {
        Self::ButtonDrag(value)
    }
}

impl From<OrbitCamLineScroll> for OrbitCamOrbitBinding {
    fn from(value: OrbitCamLineScroll) -> Self { Self::LineScroll(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamLineScroll>> for OrbitCamOrbitBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamLineScroll>) -> Self {
        Self::LineScroll(value)
    }
}

impl From<OrbitCamLineScroll> for OrbitCamPanBinding {
    fn from(value: OrbitCamLineScroll) -> Self { Self::LineScroll(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamLineScroll>> for OrbitCamPanBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamLineScroll>) -> Self {
        Self::LineScroll(value)
    }
}

impl From<OrbitCamLineScroll> for OrbitCamZoomBinding {
    fn from(value: OrbitCamLineScroll) -> Self { Self::LineScroll(value.into()) }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamLineScroll>> for OrbitCamZoomBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamLineScroll>) -> Self {
        Self::LineScroll(value)
    }
}
