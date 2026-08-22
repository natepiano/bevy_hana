use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

use bevy::ecs::prelude::Resource;
use bevy::input::keyboard::KeyCode;
use bevy_enhanced_input::prelude::ActionValue;
use bevy_enhanced_input::prelude::CustomInput;
use bevy_enhanced_input::prelude::CustomInputs;

use super::key_edge::PHYSICAL_MODIFIER_KEYS;
use crate::Modifiers;
use crate::OrdinaryKey;
use crate::keymap::KeymapGeneration;
use crate::keymap::KeystrokeRouting;

/// How many physical keys can own an entry in `KeymapRuntime::physical_sources` at once.
///
/// A key reaches that map as either the primary key of an ordinary chord — necessarily an
/// [`OrdinaryKey`] — or one side of a modifier family, so the whole routable domain is every
/// `OrdinaryKey` plus `PHYSICAL_MODIFIER_KEYS`. Reserving that much at setup keeps routing free
/// of reallocation even when all four modifier families and both sides of each are held at once.
const ROUTABLE_PHYSICAL_KEYS: usize = OrdinaryKey::COUNT + PHYSICAL_MODIFIER_KEYS.len();

/// How many custom inputs `KeymapRuntime::held_sources` can carry while routing drives them.
///
/// Routing reaches that map only through `KeymapRuntime::activate_physical_source`, which records
/// at most one physical source per key, so it can never make more distinct [`CustomInput`]s active
/// than there are routable physical keys. `KeymapRuntime::set_event_source` adds further entries
/// from `HoldPhase` events, which arrive from observers rather than from the input path.
const ROUTING_HELD_CUSTOM_INPUTS: usize = ROUTABLE_PHYSICAL_KEYS;

/// How many key codes `KeymapRuntime::inhibited` reserves room for.
///
/// `KeymapRuntime::inhibit` refills the set from every key `ButtonInput<KeyCode>` reports pressed
/// at a keymap swap, including key codes no keystroke can spell, so the routable domain is a
/// reservation rather than a ceiling — a swap that exceeds it grows the set on the swap path,
/// which already collects those keys into a `Vec`. Routing only ever removes from the set, through
/// `KeymapRuntime::release_inhibition`.
const INHIBITED_KEYS_AT_A_KEYMAP_SWAP: usize = ROUTABLE_PHYSICAL_KEYS;

/// How many key codes `KeymapRuntime::processed_keycodes` can gather in one routing pass.
///
/// The set records the keys `route_releases` and `route_ordinary_key_presses` already consumed
/// this frame, including key codes routing cannot spell as an [`OrdinaryKey`], so it is bounded by
/// how many keys change state in a single frame. Reserving the routable domain leaves that count
/// far below capacity — no keyboard reports every ordinary key and both sides of all four modifier
/// families changing state at once.
const KEYS_PROCESSED_IN_ONE_FRAME: usize = ROUTABLE_PHYSICAL_KEYS;

#[derive(Resource)]
pub(crate) struct KeymapRuntime {
    clock:                Clock,
    keymap_routing_state: KeymapRoutingState,
    keystroke_routing:    KeystrokeRouting,
    held_sources:         HashMap<CustomInput, HeldSources>,
    physical_sources:     HashMap<KeyCode, HeldPhysicalSource>,
    inhibited:            HashSet<KeyCode>,
    processed_keycodes:   HashSet<KeyCode>,
}

impl Default for KeymapRuntime {
    fn default() -> Self {
        Self {
            clock:                Clock::System,
            keymap_routing_state: KeymapRoutingState::Uninitialized,
            keystroke_routing:    KeystrokeRouting::default(),
            held_sources:         HashMap::with_capacity(ROUTING_HELD_CUSTOM_INPUTS),
            physical_sources:     HashMap::with_capacity(ROUTABLE_PHYSICAL_KEYS),
            inhibited:            HashSet::with_capacity(INHIBITED_KEYS_AT_A_KEYMAP_SWAP),
            processed_keycodes:   HashSet::with_capacity(KEYS_PROCESSED_IN_ONE_FRAME),
        }
    }
}

impl KeymapRuntime {
    pub(super) fn now(&self) -> Instant { self.clock.now() }

    pub(super) fn inactive_routing_transition(
        &self,
        generation: KeymapGeneration,
    ) -> InactiveRoutingTransition {
        match self.keymap_routing_state {
            KeymapRoutingState::Uninitialized => InactiveRoutingTransition::Initial,
            KeymapRoutingState::Unavailable {
                generation: committed_generation,
            } if committed_generation == generation => InactiveRoutingTransition::Unchanged,
            KeymapRoutingState::Unavailable { .. } => InactiveRoutingTransition::Reloaded,
            KeymapRoutingState::Active { .. } => InactiveRoutingTransition::Entered,
        }
    }

    pub(super) const fn record_unavailable_routing(&mut self, generation: KeymapGeneration) {
        self.keymap_routing_state = KeymapRoutingState::Unavailable { generation };
    }

    pub(super) fn active_routing_transition(
        &self,
        generation: KeymapGeneration,
    ) -> ActiveRoutingTransition {
        match self.keymap_routing_state {
            KeymapRoutingState::Uninitialized => ActiveRoutingTransition::Initial,
            KeymapRoutingState::Unavailable { .. } => ActiveRoutingTransition::Recovered,
            KeymapRoutingState::Active {
                generation: committed_generation,
            } if committed_generation != generation => ActiveRoutingTransition::GenerationChanged,
            KeymapRoutingState::Active { .. } => ActiveRoutingTransition::Unchanged,
        }
    }

    pub(super) const fn record_active_routing(&mut self, generation: KeymapGeneration) {
        self.keymap_routing_state = KeymapRoutingState::Active { generation };
    }

    /// Records the routing this pass runs under and reports whether the
    /// keyboard changed hands since the previous pass.
    pub(super) fn observe_routing(
        &mut self,
        keystroke_routing: &KeystrokeRouting,
    ) -> KeyboardHandover {
        if self.keystroke_routing == *keystroke_routing {
            return KeyboardHandover::Unchanged;
        }
        self.keystroke_routing = keystroke_routing.clone();

        KeyboardHandover::Crossed
    }

    pub(super) fn inhibit(&mut self, pressed: impl Iterator<Item = KeyCode>) {
        self.inhibited.clear();
        self.inhibited.extend(pressed);
    }

    pub(super) fn is_inhibited(&self, key: KeyCode) -> bool { self.inhibited.contains(&key) }

    pub(super) fn release_inhibition(&mut self, key: KeyCode) { self.inhibited.remove(&key); }

    pub(super) fn clear_processed_keycodes(&mut self) { self.processed_keycodes.clear(); }

    pub(super) fn is_processed(&self, key: KeyCode) -> bool {
        self.processed_keycodes.contains(&key)
    }

    pub(super) fn mark_processed(&mut self, key: KeyCode) { self.processed_keycodes.insert(key); }

    pub(super) fn activate_modifier_family(
        &mut self,
        key: KeyCode,
        custom_input: CustomInput,
    ) -> CustomInputTransition {
        self.activate_physical_source(key, HeldPhysicalSource::ModifierFamily { custom_input })
    }

    pub(super) fn activate_ordinary_chord(
        &mut self,
        ownership: HeldChordPhysicalOwnership,
        custom_input: CustomInput,
    ) -> CustomInputTransition {
        self.activate_physical_source(
            ownership.primary_key,
            HeldPhysicalSource::OrdinaryChord {
                custom_input,
                ownership,
            },
        )
    }

    pub(super) fn release_key(&mut self, key: KeyCode) -> CustomInputTransition {
        let Some(held_physical_source) = self.physical_sources.remove(&key) else {
            return CustomInputTransition::Unchanged;
        };
        let custom_input = held_physical_source.custom_input();
        let Some((before, after)) = self
            .held_sources
            .get_mut(&custom_input)
            .map(|held_sources| {
                let before = held_sources.activity();
                held_sources.physical -= 1;
                (before, held_sources.activity())
            })
        else {
            return CustomInputTransition::Unchanged;
        };
        if matches!(after, HeldInputActivity::Inactive) {
            self.held_sources.remove(&custom_input);
        }

        CustomInputTransition::between(custom_input, before, after)
    }

    pub(super) fn release_one_chord_missing_modifiers(
        &mut self,
        pressed_modifiers: Modifiers,
    ) -> PhysicalSourceReleaseProgress {
        let key = self
            .physical_sources
            .iter()
            .find_map(|(key, held_physical_source)| match held_physical_source {
                HeldPhysicalSource::ModifierFamily { .. } => None,
                HeldPhysicalSource::OrdinaryChord { ownership, .. } => {
                    (!ownership.modifiers_are_held(pressed_modifiers)).then_some(*key)
                },
            });
        let Some(key) = key else {
            return PhysicalSourceReleaseProgress::Complete;
        };

        PhysicalSourceReleaseProgress::ReleasedOne(self.release_key(key))
    }

    pub(super) fn release_one_physical_source(&mut self) -> PhysicalSourceReleaseProgress {
        let Some(key) = self.physical_sources.keys().next().copied() else {
            return PhysicalSourceReleaseProgress::Complete;
        };

        PhysicalSourceReleaseProgress::ReleasedOne(self.release_key(key))
    }

    fn activate_physical_source(
        &mut self,
        key: KeyCode,
        held_physical_source: HeldPhysicalSource,
    ) -> CustomInputTransition {
        if self.physical_sources.contains_key(&key) {
            return CustomInputTransition::Unchanged;
        }

        let custom_input = held_physical_source.custom_input();
        let (before, after) = {
            let held_sources = self.held_sources.entry(custom_input).or_default();
            let before = held_sources.activity();
            held_sources.physical += 1;
            (before, held_sources.activity())
        };
        self.physical_sources.insert(key, held_physical_source);

        CustomInputTransition::between(custom_input, before, after)
    }

    pub(super) fn set_event_source(
        &mut self,
        custom_input: CustomInput,
        is_active: bool,
        custom_inputs: &mut CustomInputs,
    ) {
        if is_active {
            let held_sources = self.held_sources.entry(custom_input).or_default();
            let before = held_sources.activity();
            held_sources.event = 1;
            CustomInputTransition::between(custom_input, before, held_sources.activity())
                .write_to(custom_inputs);
            return;
        }

        let Some(held_sources) = self.held_sources.get_mut(&custom_input) else {
            custom_inputs.insert(custom_input, ActionValue::Bool(false));
            return;
        };
        let before = held_sources.activity();
        held_sources.event = 0;
        let after = held_sources.activity();
        CustomInputTransition::between(custom_input, before, after).write_to(custom_inputs);
        if matches!(after, HeldInputActivity::Inactive) {
            self.held_sources.remove(&custom_input);
        }
    }

    #[cfg(test)]
    pub(super) const fn set_test_clock(&mut self, now: Instant) { self.clock = Clock::Test(now); }
}

/// Whether [`KeystrokeRouting`] moved the keyboard between the keymap and a text
/// field since the previous routing pass.
pub(super) enum KeyboardHandover {
    Crossed,
    Unchanged,
}

pub(super) enum PhysicalSourceReleaseProgress {
    ReleasedOne(CustomInputTransition),
    Complete,
}

/// The one held-input write a single [`KeymapRuntime`] mutation can produce.
///
/// Each mutation hands its transition straight back to its caller, which writes it into
/// [`CustomInputs`] before it calls the next mutation, so two unwritten transitions never coexist.
#[derive(Clone, Copy)]
#[must_use]
pub(super) enum CustomInputTransition {
    Unchanged,
    Changed {
        custom_input: CustomInput,
        activity:     HeldInputActivity,
    },
}

impl CustomInputTransition {
    const fn between(
        custom_input: CustomInput,
        before: HeldInputActivity,
        after: HeldInputActivity,
    ) -> Self {
        match (before, after) {
            (HeldInputActivity::Active, HeldInputActivity::Active)
            | (HeldInputActivity::Inactive, HeldInputActivity::Inactive) => Self::Unchanged,
            (HeldInputActivity::Inactive, HeldInputActivity::Active)
            | (HeldInputActivity::Active, HeldInputActivity::Inactive) => Self::Changed {
                custom_input,
                activity: after,
            },
        }
    }

    pub(super) fn write_to(self, custom_inputs: &mut CustomInputs) {
        if let Self::Changed {
            custom_input,
            activity,
        } = self
        {
            custom_inputs.insert(custom_input, activity.action_value());
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum HeldInputActivity {
    Active,
    Inactive,
}

impl HeldInputActivity {
    const fn action_value(self) -> ActionValue { ActionValue::Bool(matches!(self, Self::Active)) }
}

#[derive(Clone, Copy)]
pub(super) struct HeldChordPhysicalOwnership {
    primary_key:        KeyCode,
    required_modifiers: Modifiers,
}

impl HeldChordPhysicalOwnership {
    pub(super) const fn new(primary_key: KeyCode, required_modifiers: Modifiers) -> Self {
        Self {
            primary_key,
            required_modifiers,
        }
    }

    const fn modifiers_are_held(self, pressed_modifiers: Modifiers) -> bool {
        (!self.required_modifiers.has_control() || pressed_modifiers.has_control())
            && (!self.required_modifiers.has_alt() || pressed_modifiers.has_alt())
            && (!self.required_modifiers.has_shift() || pressed_modifiers.has_shift())
            && (!self.required_modifiers.has_platform() || pressed_modifiers.has_platform())
    }
}

#[derive(Clone, Copy)]
enum HeldPhysicalSource {
    ModifierFamily {
        custom_input: CustomInput,
    },
    OrdinaryChord {
        custom_input: CustomInput,
        ownership:    HeldChordPhysicalOwnership,
    },
}

impl HeldPhysicalSource {
    const fn custom_input(self) -> CustomInput {
        match self {
            Self::ModifierFamily { custom_input } | Self::OrdinaryChord { custom_input, .. } => {
                custom_input
            },
        }
    }
}

/// The complete routing state held by [`KeymapRuntime`].
///
/// A generation and matcher only coexist while routing is active. The unavailable state retains
/// the last committed generation without manufacturing a matcher for a public no-matcher state.
enum KeymapRoutingState {
    Uninitialized,
    Unavailable { generation: KeymapGeneration },
    Active { generation: KeymapGeneration },
}

/// The work `route_input` must perform before it returns for a public no-matcher state.
pub(super) enum InactiveRoutingTransition {
    Initial,
    Entered,
    Reloaded,
    Unchanged,
}

/// The work `route_input` must perform before it uses the selected matcher.
#[derive(Clone, Copy)]
pub(super) enum ActiveRoutingTransition {
    Initial,
    Recovered,
    GenerationChanged,
    Unchanged,
}

#[derive(Clone, Copy)]
enum Clock {
    System,
    #[cfg(test)]
    Test(Instant),
}

impl Clock {
    fn now(self) -> Instant {
        match self {
            Self::System => Instant::now(),
            #[cfg(test)]
            Self::Test(now) => now,
        }
    }
}

#[derive(Default)]
struct HeldSources {
    event:    usize,
    physical: usize,
}

impl HeldSources {
    const fn activity(&self) -> HeldInputActivity {
        if self.event + self.physical == 0 {
            HeldInputActivity::Inactive
        } else {
            HeldInputActivity::Active
        }
    }
}
