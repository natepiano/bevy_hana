//! JSONC keymap foundations for Bevy applications.
//!
//! `hana_rubric` exposes validated [`CommandId`] values and durable
//! [`KeymapLoadFailures`] data for callers that load and inspect keymaps.

#[cfg(test)]
mod allocation_test_support;
mod command;
mod condition;
mod diagnostic;
mod disk;
mod keymap;
mod keymap_plugin;
mod keystroke;
mod palette;
mod platform_shortcut_mode;
/// Convenience re-exports for common keymap callers.
pub mod prelude;

/// Cancels every partially matched multi-keystroke sequence in the active keymap.
///
/// Applications call this after a recovery action or focus change should discard the next
/// keystroke instead of completing an earlier sequence.
pub fn cancel_pending_sequences(world: &mut World) { keymap::cancel_pending_sequences(world); }

/// Resets physical keymap input after a focus or input-suppression transition.
///
/// This cancels partially matched sequences, releases physical held sources while preserving
/// semantic-event sources, publishes resulting held-value changes, and inhibits every key that
/// remains down until its release. Calling it on the next transition refreshes that inhibition
/// from the keys still down.
pub fn reset_physical_input(world: &mut World) { keymap::reset_physical_input(world); }

#[cfg(test)]
pub(crate) use allocation_test_support::TEST_ALLOCATOR;
use bevy::prelude::World;
pub use command::Capability;
pub use command::CommandId;
pub use command::CommandIdParseError;
pub use command::CommandInfo;
pub use command::CommandInvocationOutcome;
pub use command::CommandLookup;
pub use command::CommandRegistry;
pub use command::HeldCommandLookupOutcome;
pub use command::HoldPhase;
pub use command::Keybindings;
pub use command::KeymapCommand;
pub use command::PaletteInvocableCommand;
pub use command::ReflectKeymapCommand;
pub use condition::ActiveKeymapContext;
pub use condition::ActiveKeymapContextState;
pub use condition::ActiveKeymapContextTransition;
pub use condition::ContextDimensionName;
pub use condition::ContextSnapshot;
pub use condition::ContextValueName;
pub use condition::KeymapStateDimension;
pub use diagnostic::Diagnostic;
pub use diagnostic::DiagnosticKind;
pub use diagnostic::DiagnosticOrigin;
pub use diagnostic::DiagnosticSeverity;
pub use diagnostic::KeymapLoadFailures;
pub use disk::KeymapConfigurationDirectory;
pub use disk::KeymapPathAvailability;
pub use disk::KeymapPathFailure;
pub use disk::KeymapPaths;
pub use keymap::AuthoredKeymapBindings;
pub use keymap::CommandKeystroke;
pub use keymap::EffectiveKeymapPublication;
pub use keymap::EffectiveKeymapSnapshot;
pub use keymap::EffectiveKeymapStatus;
pub use keymap::KeyboardClaim;
pub use keymap::KeyboardOwner;
pub use keymap::KeyboardRelease;
pub use keymap::KeymapBindingUnavailability;
pub use keymap::KeymapBindings;
pub use keymap::KeymapGeneration;
pub use keymap::KeystrokeRouting;
pub use keymap::LoadedKeymapBindings;
pub use keymap::MatchedPredicateLayer;
pub use keymap::StateDimensionPredicateIdentity;
pub use keymap_plugin::DefaultKeymapSource;
pub use keymap_plugin::KeymapPlugin;
pub use keymap_plugin::KeymapSystems;
pub use keystroke::DeferredMatch;
pub use keystroke::EmptyKeystrokeSequenceError;
pub use keystroke::InvalidOrdinaryKeyCode;
pub use keystroke::Keystroke;
pub use keystroke::KeystrokeParseError;
pub use keystroke::KeystrokeSequence;
pub use keystroke::KeystrokeSequenceParseError;
pub use keystroke::MatchOutcome;
pub use keystroke::ModifierFamily;
pub use keystroke::Modifiers;
pub use keystroke::OrdinaryKey;
pub use keystroke::PrimaryTrigger;
pub use keystroke::SequenceMatcher;
pub use keystroke::TimeoutOutcome;
pub use palette::CommandPaletteQueryResult;
pub use palette::CommandPaletteRow;
pub use palette::PaletteBinding;
pub use palette::PaletteSelectionOutcome;
pub use palette::query_command_palette;
