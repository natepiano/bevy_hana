//! Held-binding primitives: the BEI-style value binding plus engagement binding pair.
//!
//! Types:
//! - [`HeldBinding`] — a value binding (motion) paired with an engagement binding (button/key that
//!   latches the motion). Stored on [`super::action_set::HeldActionBindingEntry`] after validation.
//! - [`InputBinding`] — a `bevy_enhanced_input` [`Binding`] plus the composite variants
//!   (`CardinalKeys`, `Vec3Keys`, `BidirectionalKeys`, `GamepadAxes2d`,
//!   `BidirectionalGamepadButtons`, `GamepadButtonAxis`, `GamepadVec3`) that expand into multiple
//!   BEI bindings.
//!
//! Helpers `route_for_sources` and `sources_for_binding` derive default routing and source
//! metadata from the underlying BEI binding.

use bevy::prelude::GamepadAxis;
use bevy::prelude::GamepadButton;
use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Reflect;
use bevy_enhanced_input::prelude::Binding;

use super::action_set::BindingRoutePolicy;
use super::descriptor::HeldBindingDescriptor;
use super::descriptor::InputAxisTransform;
use super::descriptor::InputBindingDescriptor;
use super::descriptor::InputBindingEntry;
use super::descriptor::InputBindingModifiers;
use super::descriptor::InputBindingOutputAxis;
use super::descriptor::InputBindingScale;
use super::descriptor::InputDeadZone;
use super::descriptor::InputGain;
use crate::input::ControlSpeed;
use crate::input::InteractionSources;

/// A held enhanced-input binding made from a value binding and an engagement binding.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct HeldBinding {
    pub(super) motion:     InputBinding,
    pub(super) engagement: InputBinding,
    pub(super) gates:      BindingGates,
    pub(super) sources:    InteractionSources,
    pub(super) route:      BindingRoutePolicy,
    pub(super) speed:      ControlSpeed,
}

impl HeldBinding {
    /// Creates a held binding from BEI-style value and engagement bindings.
    #[must_use]
    pub fn new(motion: impl Into<InputBinding>, engagement: impl Into<InputBinding>) -> Self {
        let motion = motion.into();
        let engagement = engagement.into();
        let sources = motion.sources().union(engagement.sources());
        let route = route_for_sources(sources);
        Self {
            motion,
            engagement,
            gates: BindingGates::default(),
            sources,
            route,
            speed: ControlSpeed::Normal,
        }
    }

    /// Creates a held binding whose value binding also engages the action.
    #[must_use]
    pub fn same(binding: impl Into<InputBinding>) -> Self {
        let binding = binding.into();
        Self::new(binding.clone(), binding)
    }

    /// Overrides source attribution for this binding.
    #[must_use]
    pub const fn with_sources(mut self, sources: InteractionSources) -> Self {
        self.sources = sources;
        self
    }

    /// Overrides routing for this binding.
    #[must_use]
    pub const fn with_route(mut self, route: BindingRoutePolicy) -> Self {
        self.route = route;
        self
    }

    /// Requires `input` while this held binding is active.
    #[must_use]
    pub fn with_required_gate(mut self, input: impl Into<GateInput>) -> Self {
        self.gates = self.gates.with_required(input);
        self
    }

    /// Suppresses this held binding while `input` is active.
    #[must_use]
    pub fn with_blocked_gate(mut self, input: impl Into<GateInput>) -> Self {
        self.gates = self.gates.with_blocked(input);
        self
    }

    /// Sets the authored input gain for the motion binding.
    #[must_use]
    pub fn with_input_gain(mut self, input_gain: f32) -> Self {
        self.motion = self.motion.with_input_gain(input_gain);
        self
    }

    /// Tags this binding as the normal or slow (precise) speed variant.
    #[must_use]
    pub const fn speed(mut self, speed: ControlSpeed) -> Self {
        self.speed = speed;
        self
    }
}

impl From<HeldBinding> for HeldBindingDescriptor {
    fn from(binding: HeldBinding) -> Self {
        Self {
            motion:             binding.motion.descriptor(),
            engagement:         Some(binding.engagement.descriptor()),
            gates:              binding.gates,
            sources:            binding.sources,
            engagement_sources: binding.sources,
            route:              binding.route,
            speed:              binding.speed,
        }
    }
}

/// Gate descriptors applied to both motion and engagement entries of a held binding.
#[derive(Clone, Debug, Default, PartialEq, Eq, Reflect)]
pub struct BindingGates {
    gates: Vec<BindingGate>,
}

impl BindingGates {
    /// Returns the gate descriptors in installation order.
    #[must_use]
    pub fn entries(&self) -> &[BindingGate] { &self.gates }

    fn with_required(mut self, input: impl Into<GateInput>) -> Self {
        self.gates.push(BindingGate {
            input:    input.into(),
            polarity: GatePolarity::Required,
        });
        self
    }

    fn with_blocked(mut self, input: impl Into<GateInput>) -> Self {
        self.gates.push(BindingGate {
            input:    input.into(),
            polarity: GatePolarity::Blocked,
        });
        self
    }
}

/// One symbolic gate attached to a held binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub struct BindingGate {
    /// Input checked by this gate.
    pub input:    GateInput,
    /// Whether the input is required or blocks the binding.
    pub polarity: GatePolarity,
}

/// Physical input used by a held-binding gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
#[non_exhaustive]
pub enum GateInput {
    /// Gamepad button gate.
    GamepadButton(GamepadButton),
    /// Keyboard key gate.
    Key(KeyCode),
}

impl From<GamepadButton> for GateInput {
    fn from(value: GamepadButton) -> Self { Self::GamepadButton(value) }
}

impl From<KeyCode> for GateInput {
    fn from(value: KeyCode) -> Self { Self::Key(value) }
}

/// Gate behavior for a held binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum GatePolarity {
    /// Gate input must be active.
    Required,
    /// Gate input suppresses the binding while active.
    Blocked,
}

/// A BEI-style input binding plus composite helpers.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
#[non_exhaustive]
pub enum InputBinding {
    /// A native `bevy_enhanced_input` binding.
    Binding(Binding),
    /// Four keyboard keys captured as positive Y, positive X, negative Y, negative X.
    CardinalKeys(KeyCode, KeyCode, KeyCode, KeyCode),
    /// Six keyboard keys captured as a 3D value: forward, backward, left, right, up, down.
    Vec3Keys(KeyCode, KeyCode, KeyCode, KeyCode, KeyCode, KeyCode),
    /// Two keyboard keys captured as positive and negative 1D values.
    BidirectionalKeys(KeyCode, KeyCode),
    /// Two gamepad axes captured as X and Y.
    GamepadAxes2d(GamepadAxis, GamepadAxis),
    /// Two analog gamepad buttons captured as positive and negative 1D values.
    BidirectionalGamepadButtons(GamepadButton, GamepadButton),
    /// A single analog gamepad button captured as a signed 1D value.
    GamepadButtonAxis(GamepadButton, f32),
    /// Two gamepad axes plus two buttons captured as a 3D value: `strafe` on X (right), `forward`
    /// on -Z (push the stick forward to move forward), and `up`/`down` buttons on Y.
    GamepadVec3 {
        /// Axis captured as X (positive right).
        strafe:  GamepadAxis,
        /// Axis captured as -Z (positive forward).
        forward: GamepadAxis,
        /// Button captured as +Y (up).
        up:      GamepadButton,
        /// Button captured as -Y (down).
        down:    GamepadButton,
    },
    /// A binding with descriptor modifiers applied after composite expansion.
    Modified {
        /// Inner binding being modified.
        binding:    Box<Self>,
        /// Modifiers applied to every expanded entry.
        modifiers:  InputBindingModifiers,
        /// Optional scale applied with composite-axis awareness.
        scale:      Option<InputBindingScale>,
        /// Authored input gain applied after signed scale lowering.
        input_gain: InputGain,
    },
}

impl InputBinding {
    /// Creates a four-key 2D binding from positive Y, positive X, negative Y, negative X.
    #[must_use]
    pub const fn cardinal_keys(
        north: KeyCode,
        east: KeyCode,
        south: KeyCode,
        west: KeyCode,
    ) -> Self {
        Self::CardinalKeys(north, east, south, west)
    }

    /// Creates a six-key 3D binding capturing forward/backward on Z, left/right
    /// on X, and up/down on Y.
    #[must_use]
    pub const fn vec3_keys(
        forward: KeyCode,
        backward: KeyCode,
        left: KeyCode,
        right: KeyCode,
        up: KeyCode,
        down: KeyCode,
    ) -> Self {
        Self::Vec3Keys(forward, backward, left, right, up, down)
    }

    /// Creates a two-key 1D binding from positive and negative keys.
    #[must_use]
    pub const fn bidirectional_keys(positive: KeyCode, negative: KeyCode) -> Self {
        Self::BidirectionalKeys(positive, negative)
    }

    /// Creates a two-axis gamepad binding from X and Y axes.
    #[must_use]
    pub const fn gamepad_axes_2d(x: GamepadAxis, y: GamepadAxis) -> Self {
        Self::GamepadAxes2d(x, y)
    }

    /// Creates a two-button gamepad binding from positive and negative buttons.
    #[must_use]
    pub const fn bidirectional_gamepad_buttons(
        positive: GamepadButton,
        negative: GamepadButton,
    ) -> Self {
        Self::BidirectionalGamepadButtons(positive, negative)
    }

    /// Creates a signed single-button gamepad axis binding.
    #[must_use]
    pub const fn gamepad_button_axis(button: GamepadButton, scale: f32) -> Self {
        Self::GamepadButtonAxis(button, scale)
    }

    /// Creates a gamepad 3D move binding: `strafe` axis on X (right), `forward` axis on -Z (push
    /// the stick forward to move forward), and `up`/`down` buttons on Y.
    #[must_use]
    pub const fn gamepad_vec3(
        strafe: GamepadAxis,
        forward: GamepadAxis,
        up: GamepadButton,
        down: GamepadButton,
    ) -> Self {
        Self::GamepadVec3 {
            strafe,
            forward,
            up,
            down,
        }
    }

    /// Applies a BEI scale modifier after dead-zone processing.
    #[must_use]
    pub fn with_scale(self, scale: impl Into<InputBindingScale>) -> Self {
        self.with_descriptor_update(|_, binding_scale, _| {
            *binding_scale = Some(scale.into());
        })
    }

    /// Applies an axial BEI dead-zone modifier.
    #[must_use]
    pub fn with_dead_zone(self, dead_zone: InputDeadZone) -> Self {
        self.with_descriptor_update(|modifiers, _, _| {
            *modifiers = modifiers.with_dead_zone(dead_zone);
        })
    }

    /// Scales held input by the current frame delta.
    #[must_use]
    pub fn with_delta_scale(self) -> Self {
        self.with_descriptor_update(|modifiers, _, _| {
            *modifiers = modifiers.with_delta_scale();
        })
    }

    /// Sets the authored input gain for this input binding.
    #[must_use]
    pub fn with_input_gain(self, input_gain_value: f32) -> Self {
        self.with_descriptor_update(|_, _, input_gain| {
            *input_gain = InputGain(input_gain_value);
        })
    }

    fn with_descriptor_update(
        self,
        update: impl FnOnce(&mut InputBindingModifiers, &mut Option<InputBindingScale>, &mut InputGain),
    ) -> Self {
        match self {
            Self::Modified {
                binding,
                mut modifiers,
                mut scale,
                mut input_gain,
            } => {
                update(&mut modifiers, &mut scale, &mut input_gain);
                Self::Modified {
                    binding,
                    modifiers,
                    scale,
                    input_gain,
                }
            },
            binding => {
                let mut modifiers = InputBindingModifiers::default();
                let mut scale = None;
                let mut input_gain = InputGain::DEFAULT;
                update(&mut modifiers, &mut scale, &mut input_gain);
                Self::Modified {
                    binding: Box::new(binding),
                    modifiers,
                    scale,
                    input_gain,
                }
            },
        }
    }

    /// Returns the flattened descriptor entries produced by this binding.
    #[must_use]
    pub fn descriptor(&self) -> InputBindingDescriptor {
        match self {
            Self::Binding(binding) => InputBindingDescriptor::single(*binding),
            Self::CardinalKeys(north, east, south, west) => InputBindingDescriptor::entries([
                InputBindingEntry::new(Binding::from(*east), InputAxisTransform::None),
                InputBindingEntry::new(Binding::from(*west), InputAxisTransform::Negate),
                InputBindingEntry::new(Binding::from(*north), InputAxisTransform::Swizzle)
                    .with_output_axis(InputBindingOutputAxis::Y),
                InputBindingEntry::new(Binding::from(*south), InputAxisTransform::SwizzleNegate)
                    .with_output_axis(InputBindingOutputAxis::Y),
            ]),
            Self::Vec3Keys(forward, backward, left, right, up, down) => {
                InputBindingDescriptor::entries([
                    InputBindingEntry::new(Binding::from(*right), InputAxisTransform::None),
                    InputBindingEntry::new(Binding::from(*left), InputAxisTransform::Negate),
                    InputBindingEntry::new(Binding::from(*up), InputAxisTransform::Swizzle)
                        .with_output_axis(InputBindingOutputAxis::Y),
                    InputBindingEntry::new(Binding::from(*down), InputAxisTransform::SwizzleNegate)
                        .with_output_axis(InputBindingOutputAxis::Y),
                    InputBindingEntry::new(Binding::from(*backward), InputAxisTransform::SwizzleZ)
                        .with_output_axis(InputBindingOutputAxis::Z),
                    InputBindingEntry::new(
                        Binding::from(*forward),
                        InputAxisTransform::SwizzleZNegate,
                    )
                    .with_output_axis(InputBindingOutputAxis::Z),
                ])
            },
            Self::BidirectionalKeys(positive, negative) => InputBindingDescriptor::entries([
                InputBindingEntry::new(Binding::from(*positive), InputAxisTransform::None),
                InputBindingEntry::new(Binding::from(*negative), InputAxisTransform::Negate),
            ]),
            Self::GamepadAxes2d(x, y) => InputBindingDescriptor::entries([
                InputBindingEntry::new(Binding::GamepadAxis(*x), InputAxisTransform::None),
                InputBindingEntry::new(Binding::GamepadAxis(*y), InputAxisTransform::Swizzle)
                    .with_output_axis(InputBindingOutputAxis::Y),
            ]),
            Self::BidirectionalGamepadButtons(positive, negative) => {
                InputBindingDescriptor::entries([
                    InputBindingEntry::new(
                        Binding::GamepadButton(*positive),
                        InputAxisTransform::None,
                    ),
                    InputBindingEntry::new(
                        Binding::GamepadButton(*negative),
                        InputAxisTransform::Negate,
                    ),
                ])
            },
            Self::GamepadButtonAxis(button, scale) => {
                InputBindingDescriptor::single(Binding::GamepadButton(*button))
                    .with_entry_modifiers(InputBindingModifiers::default(), Some((*scale).into()))
            },
            Self::GamepadVec3 {
                strafe,
                forward,
                up,
                down,
            } => InputBindingDescriptor::entries([
                InputBindingEntry::new(Binding::GamepadAxis(*strafe), InputAxisTransform::None),
                InputBindingEntry::new(
                    Binding::GamepadAxis(*forward),
                    InputAxisTransform::SwizzleZNegate,
                )
                .with_output_axis(InputBindingOutputAxis::Z),
                InputBindingEntry::new(Binding::GamepadButton(*up), InputAxisTransform::Swizzle)
                    .with_output_axis(InputBindingOutputAxis::Y),
                InputBindingEntry::new(
                    Binding::GamepadButton(*down),
                    InputAxisTransform::SwizzleNegate,
                )
                .with_output_axis(InputBindingOutputAxis::Y),
            ]),
            Self::Modified {
                binding,
                modifiers,
                scale,
                input_gain,
            } => binding
                .descriptor()
                .with_entry_modifiers(*modifiers, *scale)
                .with_entry_input_gain(*input_gain),
        }
    }

    fn sources(&self) -> InteractionSources {
        match self {
            Self::Binding(binding) => sources_for_binding(*binding),
            Self::CardinalKeys(..) | Self::Vec3Keys(..) | Self::BidirectionalKeys(..) => {
                InteractionSources::KEYBOARD
            },
            Self::GamepadAxes2d(..)
            | Self::BidirectionalGamepadButtons(..)
            | Self::GamepadButtonAxis(..)
            | Self::GamepadVec3 { .. } => InteractionSources::GAMEPAD,
            Self::Modified { binding, .. } => binding.sources(),
        }
    }
}

impl From<Binding> for InputBinding {
    fn from(value: Binding) -> Self { Self::Binding(value) }
}

impl From<KeyCode> for InputBinding {
    fn from(value: KeyCode) -> Self { Self::Binding(Binding::from(value)) }
}

impl From<MouseButton> for InputBinding {
    fn from(value: MouseButton) -> Self { Self::Binding(Binding::from(value)) }
}

impl From<GamepadButton> for InputBinding {
    fn from(value: GamepadButton) -> Self { Self::Binding(Binding::from(value)) }
}

impl From<GamepadAxis> for InputBinding {
    fn from(value: GamepadAxis) -> Self { Self::Binding(Binding::GamepadAxis(value)) }
}

const fn route_for_sources(sources: InteractionSources) -> BindingRoutePolicy {
    if sources.contains(InteractionSources::MOUSE) {
        BindingRoutePolicy::CursorPosition
    } else {
        BindingRoutePolicy::NoPosition
    }
}

pub(crate) const fn sources_for_binding(binding: Binding) -> InteractionSources {
    match binding {
        Binding::Keyboard { .. } => InteractionSources::KEYBOARD,
        Binding::MouseButton { .. } | Binding::MouseMotion { .. } => InteractionSources::MOUSE,
        Binding::MouseWheel { .. } => InteractionSources::WHEEL,
        Binding::GamepadButton(_) | Binding::GamepadAxis(_) => InteractionSources::GAMEPAD,
        Binding::Custom(_) | Binding::None => InteractionSources::NONE,
        Binding::AnyKey => InteractionSources::KEYBOARD
            .union(InteractionSources::MOUSE)
            .union(InteractionSources::GAMEPAD),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamepad_vec3_maps_stick_and_buttons_to_move_axes() {
        let binding = InputBinding::gamepad_vec3(
            GamepadAxis::LeftStickX,
            GamepadAxis::LeftStickY,
            GamepadButton::RightTrigger2,
            GamepadButton::LeftTrigger2,
        );
        let descriptor = binding.descriptor();
        let entries = descriptor.entries_slice();

        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries[0].binding(),
            Binding::GamepadAxis(GamepadAxis::LeftStickX)
        );
        assert_eq!(
            entries[0].modifiers().axis_transform(),
            InputAxisTransform::None
        );
        assert_eq!(
            entries[1].binding(),
            Binding::GamepadAxis(GamepadAxis::LeftStickY)
        );
        assert_eq!(
            entries[1].modifiers().axis_transform(),
            InputAxisTransform::SwizzleZNegate
        );
        assert_eq!(
            entries[2].binding(),
            Binding::GamepadButton(GamepadButton::RightTrigger2)
        );
        assert_eq!(
            entries[2].modifiers().axis_transform(),
            InputAxisTransform::Swizzle
        );
        assert_eq!(
            entries[3].binding(),
            Binding::GamepadButton(GamepadButton::LeftTrigger2)
        );
        assert_eq!(
            entries[3].modifiers().axis_transform(),
            InputAxisTransform::SwizzleNegate
        );
        assert!(binding.sources().contains(InteractionSources::GAMEPAD));
    }
}
