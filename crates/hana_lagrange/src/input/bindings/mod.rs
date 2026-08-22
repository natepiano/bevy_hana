//! Camera-agnostic binding vocabulary shared by every camera kind's binding model.
//!
//! Submodules:
//! - [`held_binding`] — [`HeldBinding`] / [`InputBinding`] primitives and their gates.
//! - [`descriptor`] — reflectable descriptor and entry types plus the runtime binding-active
//!   predicates.
//! - `binding_forwards` — macro-generated forwarding impls for per-camera binding-set newtypes.
//! - [`source_binding`] — live input-state attribution for binding source metadata.
//! - [`action_set`] — generic per-action binding-set storage written by each camera's validator.
//! - [`error`] — [`BindingsError`].
//! - [`validate`] — camera-agnostic descriptor validation and lowering into the generic storage.

mod action_set;
#[macro_use]
mod binding_forwards;
mod descriptor;
mod error;
mod held_binding;
mod source_binding;
mod validate;

pub use action_set::BindingEngagement;
pub use action_set::BindingRoutePolicy;
pub use action_set::HeldActionBindingEntry;
pub use action_set::HeldActionBindingSet;
pub use action_set::ImpulseActionBindingEntry;
pub use action_set::ImpulseActionBindingSet;
pub use descriptor::ActionBindingDescriptor;
pub use descriptor::CameraInputScalePolicy;
pub use descriptor::CameraSlowMode;
pub use descriptor::HeldBindingDescriptor;
pub use descriptor::InputAxisTransform;
pub use descriptor::InputBindingDescriptor;
pub use descriptor::InputBindingEntry;
pub use descriptor::InputBindingModifiers;
pub use descriptor::InputBindingScale;
pub use descriptor::InputDeadZone;
pub use descriptor::InputDeltaScale;
pub use descriptor::InputGain;
pub use error::BindingsError;
pub use held_binding::BindingGate;
pub use held_binding::BindingGates;
pub use held_binding::GateInput;
pub use held_binding::GatePolarity;
pub use held_binding::HeldBinding;
pub use held_binding::InputBinding;
pub(crate) use held_binding::sources_for_binding;
pub use source_binding::LiveInputs;
pub use source_binding::attributed_sources;
pub use source_binding::mod_keys_pressed;
pub use validate::action_descriptor_to_entry;
pub use validate::held_descriptors_to_set;
pub use validate::validate_held_entries;
pub use validate::validate_impulse_entries;
pub use validate::validate_slow_mode;
