//! Keymap data and runtime support.

use bevy_enhanced_input::prelude::CustomInput;
use bevy_enhanced_input::prelude::CustomInputs;

mod bindings;
mod compiled;
mod constants;
mod document;
mod effective;
mod merged;
mod reload;
mod routing;
mod runtime;
mod schema;

pub(crate) use bindings::ApplicationRecoveryAssociation;
pub use bindings::AuthoredKeymapBindings;
pub use bindings::CommandKeystroke;
pub use bindings::KeymapBindingUnavailability;
pub use bindings::KeymapBindings;
pub use bindings::LoadedKeymapBindings;
pub(crate) use bindings::ProtectedCommandBinding;
pub(super) use compiled::CommandHandle;
pub(crate) use compiled::CompiledKeymap;
pub use compiled::KeymapGeneration;
pub(super) use compiled::ModifierFamilyHeldBinding;
#[cfg(test)]
pub(crate) use document::predicate_match_count;
#[cfg(test)]
pub(crate) use document::reset_predicate_match_count;
pub(crate) use effective::AcceptedKeymapDocument;
pub use effective::EffectiveKeymapPublication;
pub use effective::EffectiveKeymapSnapshot;
pub use effective::EffectiveKeymapStatus;
pub use effective::MatchedPredicateLayer;
pub use effective::StateDimensionPredicateIdentity;
pub(crate) use merged::MergedKeymap;
pub(crate) use merged::UserKeymap;
pub(crate) use reload::PendingReload;
pub(crate) use reload::ReloadConfiguration;
pub(crate) use reload::ReloadRequest;
pub(crate) use reload::UserKeymapContents;
pub(crate) use reload::commit_defaults;
pub(crate) use reload::commit_effective_keymap;
pub(crate) use reload::commit_reload;
pub use routing::KeyboardClaim;
pub use routing::KeyboardOwner;
pub use routing::KeyboardRelease;
pub use routing::KeystrokeRouting;
pub(super) use runtime::KeymapRuntime;
#[cfg(test)]
pub(crate) use runtime::RoutingResetStep;
#[cfg(test)]
pub(crate) use runtime::RoutingResetTrace;
pub(super) use runtime::cancel_pending_sequences;
pub(super) use runtime::reset_physical_input;
pub(super) use runtime::route_input;
pub(crate) use schema::state_dimension_reference_default_bytes;
pub(crate) use schema::state_dimension_schema_bytes;

pub(super) fn set_event_source(
    keymap_runtime: &mut KeymapRuntime,
    custom_input: CustomInput,
    is_active: bool,
    custom_inputs: &mut CustomInputs,
) {
    runtime::set_event_source(keymap_runtime, custom_input, is_active, custom_inputs);
}
