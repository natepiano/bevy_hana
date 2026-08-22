//! Descriptor and entry types plus the runtime binding-active predicates.
//!
//! Types:
//! - [`HeldBindingDescriptor`] / [`ActionBindingDescriptor`] — reflectable descriptor entries
//!   stored on a camera's binding descriptor (the editor/keymap-facing draft).
//! - [`InputBindingDescriptor`] / [`InputBindingEntry`] / [`InputBindingModifiers`] — flattened
//!   list of native `bevy_enhanced_input` bindings plus the per-entry modifiers applied by the
//!   adapter when it spawns enhanced-input binding entities.
//!
//! Functions:
//! - [`InputBindingDescriptor::enabled_is_active`] — runtime predicate evaluated against live input
//!   resources.

use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Reflect;
use bevy::prelude::Vec2;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ModKeys;

use super::action_set::BindingEngagement;
use super::action_set::BindingRoutePolicy;
use super::error::BindingsError;
use super::held_binding;
use super::held_binding::BindingGates;
use super::source_binding;
use super::source_binding::LiveInputs;
use crate::input::ControlSpeed;
use crate::input::InteractionSources;

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct HeldBindingDescriptor {
    /// Motion input descriptor.
    pub motion:             InputBindingDescriptor,
    /// Optional engagement descriptor that latches the held action.
    pub engagement:         Option<InputBindingDescriptor>,
    /// Additional gate inputs required or blocked for this binding.
    pub gates:              BindingGates,
    /// Source metadata reported for the motion binding.
    pub sources:            InteractionSources,
    /// Source metadata reported for the engagement binding.
    pub engagement_sources: InteractionSources,
    /// Routing policy for this held binding.
    pub route:              BindingRoutePolicy,
    /// Runtime speed tier for this held binding.
    pub speed:              ControlSpeed,
}

/// Reflectable descriptor for an impulse action binding.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct ActionBindingDescriptor {
    /// Native input descriptor for this action.
    pub binding:    InputBindingDescriptor,
    /// Source metadata reported for this action.
    pub sources:    InteractionSources,
    /// Routing policy for this action.
    pub route:      BindingRoutePolicy,
    /// Whether this action is impulse or held.
    pub engagement: BindingEngagement,
}

impl From<Binding> for ActionBindingDescriptor {
    fn from(binding: Binding) -> Self {
        let sources = held_binding::sources_for_binding(binding);
        Self {
            binding: InputBindingDescriptor::single(binding),
            sources,
            route: route_for_sources(sources),
            engagement: BindingEngagement::Impulse,
        }
    }
}

const fn route_for_sources(sources: InteractionSources) -> BindingRoutePolicy {
    if sources.contains(InteractionSources::MOUSE) {
        BindingRoutePolicy::CursorPosition
    } else {
        BindingRoutePolicy::NoPosition
    }
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct InputBindingDescriptor {
    entries: Vec<InputBindingEntry>,
}

impl InputBindingDescriptor {
    pub(super) fn single(binding: Binding) -> Self {
        Self {
            entries: vec![InputBindingEntry::new(binding, InputAxisTransform::None)],
        }
    }

    pub(super) fn entries<const N: usize>(entries: [InputBindingEntry; N]) -> Self {
        Self {
            entries: entries.into(),
        }
    }

    /// Returns the flattened binding entries.
    pub fn entries_slice(&self) -> &[InputBindingEntry] { &self.entries }

    /// Returns binding entries whose input gain allows runtime participation.
    pub fn enabled_entries(&self) -> impl Iterator<Item = &InputBindingEntry> {
        self.entries.iter().filter(|entry| entry.is_enabled())
    }

    /// Returns `true` when any entry participates in runtime input.
    #[must_use]
    pub fn has_enabled_entries(&self) -> bool { self.enabled_entries().next().is_some() }

    pub(super) const fn is_empty(&self) -> bool { self.entries.is_empty() }

    pub(super) fn with_entry_modifiers(
        mut self,
        modifiers: InputBindingModifiers,
        scale: Option<InputBindingScale>,
    ) -> Self {
        for entry in &mut self.entries {
            entry.modifiers = entry.modifiers.combine(modifiers, scale, entry.output_axis);
        }
        self
    }

    pub(super) fn with_entry_input_gain(mut self, input_gain: InputGain) -> Self {
        for entry in &mut self.entries {
            entry.input_gain = input_gain;
        }
        self
    }

    /// Returns `true` when any enabled entry's binding is currently pressed.
    pub fn enabled_is_active(&self, inputs: &LiveInputs<'_>) -> bool {
        self.enabled_entries()
            .any(|entry| source_binding::binding_is_active(entry.binding, inputs))
    }

    /// Returns the first mouse-button binding entry's button and modifier keys.
    pub fn mouse_button_engagement(&self) -> Option<(MouseButton, ModKeys)> {
        self.entries.iter().find_map(|entry| match entry.binding {
            Binding::MouseButton { button, mod_keys } => Some((button, mod_keys)),
            Binding::Keyboard { .. }
            | Binding::MouseMotion { .. }
            | Binding::MouseWheel { .. }
            | Binding::GamepadButton(_)
            | Binding::GamepadAxis(_)
            | Binding::AnyKey
            | Binding::Custom(_)
            | Binding::None => None,
        })
    }

    /// Returns the first enabled mouse-button binding entry's button and modifier keys.
    pub fn enabled_mouse_button_engagement(&self) -> Option<(MouseButton, ModKeys)> {
        self.enabled_entries()
            .find_map(|entry| match entry.binding {
                Binding::MouseButton { button, mod_keys } => Some((button, mod_keys)),
                Binding::Keyboard { .. }
                | Binding::MouseMotion { .. }
                | Binding::MouseWheel { .. }
                | Binding::GamepadButton(_)
                | Binding::GamepadAxis(_)
                | Binding::AnyKey
                | Binding::Custom(_)
                | Binding::None => None,
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct InputBindingEntry {
    binding:     Binding,
    modifiers:   InputBindingModifiers,
    input_gain:  InputGain,
    output_axis: InputBindingOutputAxis,
}

impl InputBindingEntry {
    pub(super) const fn new(binding: Binding, axis_transform: InputAxisTransform) -> Self {
        Self {
            binding,
            modifiers: InputBindingModifiers::new(axis_transform),
            input_gain: InputGain::DEFAULT,
            output_axis: InputBindingOutputAxis::X,
        }
    }

    pub(super) const fn with_output_axis(mut self, output_axis: InputBindingOutputAxis) -> Self {
        self.output_axis = output_axis;
        self
    }

    /// Returns the underlying enhanced-input binding.
    #[must_use]
    pub const fn binding(&self) -> Binding { self.binding }

    /// Returns the modifiers installed with this binding entry.
    #[must_use]
    pub const fn modifiers(&self) -> InputBindingModifiers { self.modifiers }

    /// Returns modifiers with authored input gain composed into the scale.
    #[must_use]
    pub fn install_modifiers(&self) -> InputBindingModifiers {
        self.modifiers.with_input_gain(self.input_gain)
    }

    /// Returns the authored per-entry input gain.
    #[must_use]
    pub const fn input_gain(&self) -> InputGain { self.input_gain }

    const fn is_enabled(&self) -> bool { self.input_gain.is_enabled() }
}

/// Authored multiplier on a binding's raw device input.
///
/// Composed into the binding modifier scale at capture before the input
/// becomes a semantic camera action.
/// Stored separately from the signed binding scale.
///
/// Distinct from [`Sensitivity`](crate::Sensitivity): input gain scales the
/// signal at the input binding; sensitivity scales a camera axis's response
/// during the orbit/pan/zoom operation, downstream of input gain.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct InputGain(
    /// Authored input multiplier.
    pub f32,
);

impl InputGain {
    /// Default enabled gain.
    pub const DEFAULT: Self = Self(1.0);
    /// Explicitly disabled gain.
    pub const DISABLED: Self = Self(0.0);

    /// Returns the stored multiplier.
    #[must_use]
    pub const fn value(self) -> f32 { self.0 }

    /// Returns whether this input gain participates in runtime input.
    #[must_use]
    pub const fn is_enabled(self) -> bool { self.0 != Self::DISABLED.0 }

    /// Validates that this gain is finite and non-negative.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError::InvalidScale`] when the gain is negative or non-finite.
    pub fn validate(self) -> Result<(), BindingsError> {
        if self.0.is_finite() && self.0 >= Self::DISABLED.0 {
            Ok(())
        } else {
            Err(BindingsError::InvalidScale)
        }
    }
}

impl Default for InputGain {
    fn default() -> Self { Self::DEFAULT }
}

/// Canonical modifier descriptor attached to a flattened binding entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct InputBindingModifiers {
    dead_zone:      Option<InputDeadZone>,
    scale:          Option<f32>,
    delta_scale:    InputDeltaScale,
    axis_transform: InputAxisTransform,
}

impl InputBindingModifiers {
    pub(super) const fn new(axis_transform: InputAxisTransform) -> Self {
        Self {
            dead_zone: None,
            scale: None,
            delta_scale: InputDeltaScale::Disabled,
            axis_transform,
        }
    }

    pub(super) fn combine(
        self,
        modifiers: Self,
        scale: Option<InputBindingScale>,
        output_axis: InputBindingOutputAxis,
    ) -> Self {
        Self {
            dead_zone:      modifiers.dead_zone.or(self.dead_zone),
            scale:          scale_component(scale, output_axis).or(self.scale),
            delta_scale:    modifiers.delta_scale,
            axis_transform: self.axis_transform,
        }
    }

    pub(super) const fn with_dead_zone(mut self, dead_zone: InputDeadZone) -> Self {
        self.dead_zone = Some(dead_zone);
        self
    }

    pub(super) const fn with_delta_scale(mut self) -> Self {
        self.delta_scale = InputDeltaScale::Auto;
        self
    }

    fn with_input_gain(mut self, input_gain: InputGain) -> Self {
        self.scale = combined_scale(self.scale, input_gain);
        self
    }

    /// Returns the axial dead-zone modifier.
    #[must_use]
    pub const fn dead_zone(self) -> Option<InputDeadZone> { self.dead_zone }

    /// Returns the scalar modifier.
    #[must_use]
    pub const fn scale(self) -> Option<f32> { self.scale }

    /// Returns the frame-delta scale mode.
    #[must_use]
    pub const fn delta_scale(self) -> InputDeltaScale { self.delta_scale }

    /// Returns the axis transform.
    #[must_use]
    pub const fn axis_transform(self) -> InputAxisTransform { self.axis_transform }
}

/// Scale descriptor that can apply uniformly or per logical 2D axis.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub enum InputBindingScale {
    /// Same scale for every expanded entry.
    Uniform(f32),
    /// Per-axis scale for 2D composite bindings.
    Axes2d(Vec2),
}

impl From<f32> for InputBindingScale {
    fn from(value: f32) -> Self { Self::Uniform(value) }
}

impl From<Vec2> for InputBindingScale {
    fn from(value: Vec2) -> Self { Self::Axes2d(value) }
}

/// Axial dead-zone thresholds for analog bindings.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct InputDeadZone {
    /// Threshold below which input is ignored.
    pub lower_threshold: f32,
    /// Threshold above which input is clamped to full strength.
    pub upper_threshold: f32,
}

impl InputDeadZone {
    /// Creates an axial dead-zone descriptor.
    #[must_use]
    pub const fn new(lower_threshold: f32, upper_threshold: f32) -> Self {
        Self {
            lower_threshold,
            upper_threshold,
        }
    }
}

/// Normal and slow scalar multipliers for per-camera slow mode.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct CameraInputScalePolicy {
    /// Scale used when slow mode is inactive.
    pub normal: f32,
    /// Scale used when slow mode is active.
    pub slow:   f32,
}

/// Key-driven slow-mode policy stored on validated camera bindings.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct CameraSlowMode {
    /// Key whose press edge toggles slow mode for the routed camera.
    pub toggle_key: KeyCode,
    /// Modifier keys held with `toggle_key` for the toggle to fire.
    pub mod_keys:   ModKeys,
    /// Scale policy applied while resolving camera input.
    pub scale:      CameraInputScalePolicy,
}

/// Whether a held binding is scaled by frame delta.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum InputDeltaScale {
    /// Leave the value in source units.
    #[default]
    Disabled,
    /// Multiply the value by the current frame delta.
    Auto,
}

/// Intrinsic axis transform applied after dead zone, scale, and delta scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum InputAxisTransform {
    /// Leave the input axis unchanged.
    #[default]
    None,
    /// Negate every axis.
    Negate,
    /// Swizzle X into Y.
    Swizzle,
    /// Swizzle X into Y and negate it.
    SwizzleNegate,
    /// Swizzle X into Z.
    SwizzleZ,
    /// Swizzle X into Z and negate it.
    SwizzleZNegate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub(super) enum InputBindingOutputAxis {
    #[default]
    X,
    Y,
    Z,
}

const fn scale_component(
    scale: Option<InputBindingScale>,
    output_axis: InputBindingOutputAxis,
) -> Option<f32> {
    match scale {
        Some(InputBindingScale::Uniform(value)) => Some(value),
        Some(InputBindingScale::Axes2d(value)) => match output_axis {
            InputBindingOutputAxis::X => Some(value.x),
            InputBindingOutputAxis::Y => Some(value.y),
            // A 2D per-axis scale defines no Z component, so a Z-axis entry keeps
            // no per-axis scale (its input gain still applies downstream).
            InputBindingOutputAxis::Z => None,
        },
        None => None,
    }
}

fn combined_scale(scale: Option<f32>, input_gain: InputGain) -> Option<f32> {
    match scale {
        Some(scale) => Some(scale * input_gain.value()),
        None if input_gain == InputGain::DEFAULT => None,
        None => Some(input_gain.value()),
    }
}
