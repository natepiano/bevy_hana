use std::time::Instant;

use bevy::ecs::world::World;
use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;
use bevy_enhanced_input::prelude::CustomInput;
use bevy_enhanced_input::prelude::CustomInputs;

use super::PendingSequenceCancellation;
use super::held::ActiveRoutingTransition;
use super::held::CustomInputTransition;
use super::held::HeldChordPhysicalOwnership;
use super::held::InactiveRoutingTransition;
use super::held::KeyboardHandover;
use super::held::KeymapRuntime;
use super::held::PhysicalSourceReleaseProgress;
use super::key_edge;
use super::key_edge::OrdinaryKeyRoutingState;
use super::key_edge::PhysicalKeyRole;
use super::key_edge::PrimaryTriggerOwnership;
use super::reset_routing;
use crate::ActiveKeymapContext;
use crate::ActiveKeymapContextState;
use crate::DeferredMatch;
use crate::MatchOutcome;
use crate::Modifiers;
use crate::TimeoutOutcome;
use crate::command::Invocation;
use crate::keymap::CommandHandle;
use crate::keymap::CompiledKeymap;
use crate::keymap::EffectiveKeymapSnapshot;
use crate::keymap::EffectiveKeymapStatus;
use crate::keymap::KeymapGeneration;
use crate::keymap::KeystrokeRouting;
use crate::keymap::ModifierFamilyHeldBinding;
use crate::keymap::constants::SEQUENCE_TIMEOUT;

pub(crate) fn cancel_pending_sequences(world: &mut World) {
    let Some(mut compiled_keymap) = world.get_resource_mut::<CompiledKeymap>() else {
        return;
    };

    compiled_keymap.matcher.cancel_pending();
}

pub(super) fn cancel_pending_effective_sequence(world: &mut World) {
    let Some(mut compiled_keymap) = world.get_resource_mut::<CompiledKeymap>() else {
        return;
    };
    compiled_keymap.matcher.cancel_pending();
}

pub(crate) fn route_input(world: &mut World) {
    if !world.contains_resource::<CompiledKeymap>() {
        synchronize_without_effective_matcher(world);
        return;
    }
    world.init_resource::<KeymapRuntime>();
    match effective_routing_selection(world) {
        EffectiveRoutingSelection::Inactive => {
            synchronize_unavailable_routing(world);
            return;
        },
        EffectiveRoutingSelection::Active => {},
    }
    world.init_resource::<CustomInputs>();
    let keystroke_routing = world
        .get_resource::<KeystrokeRouting>()
        .cloned()
        .unwrap_or_default();

    let routed_commands = synchronize_and_resolve_timeout(world, &keystroke_routing);
    dispatch_all(world, routed_commands);
    route_releases(world);
    route_presses(world, &keystroke_routing);
}

fn synchronize_without_effective_matcher(world: &mut World) {
    let Some(active_context) = world.get_resource::<ActiveKeymapContext>() else {
        return;
    };
    let Some(effective_status) = world.get_resource::<EffectiveKeymapStatus>() else {
        return;
    };
    let inactive = matches!(
        active_context.state(),
        ActiveKeymapContextState::AwaitingStateDimensions
            | ActiveKeymapContextState::StateDimensionsUnavailable { .. }
    ) || !matches!(effective_status, EffectiveKeymapStatus::Loaded(_));
    if inactive {
        world.init_resource::<KeymapRuntime>();
        synchronize_unavailable_routing_generation(
            world,
            crate::keymap::KeymapGeneration::initial(),
        );
    }
}

enum EffectiveRoutingSelection {
    Active,
    Inactive,
}

fn effective_routing_selection(world: &World) -> EffectiveRoutingSelection {
    let active_context = world.resource::<ActiveKeymapContext>().state();
    let effective_status = world.resource::<EffectiveKeymapStatus>();
    let generation = world.resource::<CompiledKeymap>().generation;
    let coherent = match (active_context, effective_status) {
        (ActiveKeymapContextState::GlobalRouting, EffectiveKeymapStatus::Loaded(publication)) => {
            publication.generation == generation
                && matches!(publication.snapshot, EffectiveKeymapSnapshot::Global)
        },
        (
            ActiveKeymapContextState::Resolved(snapshot),
            EffectiveKeymapStatus::Loaded(publication),
        ) => {
            publication.generation == generation
                && matches!(
                    &publication.snapshot,
                    EffectiveKeymapSnapshot::Resolved(published) if published == snapshot
                )
        },
        (
            ActiveKeymapContextState::AwaitingStateDimensions
            | ActiveKeymapContextState::StateDimensionsUnavailable { .. },
            EffectiveKeymapStatus::AwaitingAcceptedDocument
            | EffectiveKeymapStatus::RejectedInitialDocument
            | EffectiveKeymapStatus::AwaitingStateDimensions
            | EffectiveKeymapStatus::StateDimensionsUnavailable { .. }
            | EffectiveKeymapStatus::UnmaterializableStateDimensions
            | EffectiveKeymapStatus::Loaded(_),
        )
        | (
            ActiveKeymapContextState::GlobalRouting | ActiveKeymapContextState::Resolved(_),
            EffectiveKeymapStatus::AwaitingAcceptedDocument
            | EffectiveKeymapStatus::RejectedInitialDocument
            | EffectiveKeymapStatus::AwaitingStateDimensions
            | EffectiveKeymapStatus::StateDimensionsUnavailable { .. }
            | EffectiveKeymapStatus::UnmaterializableStateDimensions,
        ) => false,
    };
    if coherent {
        EffectiveRoutingSelection::Active
    } else {
        EffectiveRoutingSelection::Inactive
    }
}

fn synchronize_unavailable_routing(world: &mut World) {
    let generation = world.resource::<CompiledKeymap>().generation;
    synchronize_unavailable_routing_generation(world, generation);
}

fn synchronize_unavailable_routing_generation(world: &mut World, generation: KeymapGeneration) {
    match world
        .resource::<KeymapRuntime>()
        .inactive_routing_transition(generation)
    {
        InactiveRoutingTransition::Initial => {
            reset_routing(world, PendingSequenceCancellation::None, |keymap_runtime| {
                keymap_runtime.record_unavailable_routing(generation);
            });
        },
        InactiveRoutingTransition::Entered => {
            reset_routing(
                world,
                PendingSequenceCancellation::EffectiveMatcher,
                |keymap_runtime| keymap_runtime.record_unavailable_routing(generation),
            );
        },
        InactiveRoutingTransition::Reloaded => world
            .resource_mut::<KeymapRuntime>()
            .record_unavailable_routing(generation),
        InactiveRoutingTransition::Unchanged => {},
    }
}

fn synchronize_and_resolve_timeout(
    world: &mut World,
    keystroke_routing: &KeystrokeRouting,
) -> RoutedCommands {
    synchronize_active_routing(world, keystroke_routing);

    if !world.resource::<CompiledKeymap>().matcher.is_pending() {
        return RoutedCommands::default();
    }

    world.resource_scope::<CompiledKeymap, _>(|world, mut compiled_keymap| {
        let now = world.resource::<KeymapRuntime>().now();
        let timeout_outcome = compiled_keymap
            .matcher
            .resolve_timeout(now, SEQUENCE_TIMEOUT);

        match timeout_outcome {
            TimeoutOutcome::Resolved(command_handle) => RoutedCommands::from(routed_command(
                &compiled_keymap,
                command_handle,
                keystroke_routing,
            )),
            TimeoutOutcome::DiscardedPartialPrefix
            | TimeoutOutcome::AwaitingKeystroke
            | TimeoutOutcome::NoPendingSequence => RoutedCommands::default(),
        }
    })
}

fn synchronize_active_routing(world: &mut World, keystroke_routing: &KeystrokeRouting) {
    let generation = world.resource::<CompiledKeymap>().generation;
    let active_routing_transition = world
        .resource::<KeymapRuntime>()
        .active_routing_transition(generation);
    let keyboard_handover = world
        .resource_mut::<KeymapRuntime>()
        .observe_routing(keystroke_routing);
    let pending_sequence_cancellation = match keyboard_handover {
        KeyboardHandover::Crossed => PendingSequenceCancellation::EffectiveMatcher,
        KeyboardHandover::Unchanged => match active_routing_transition {
            ActiveRoutingTransition::Initial
            | ActiveRoutingTransition::Recovered
            | ActiveRoutingTransition::GenerationChanged
            | ActiveRoutingTransition::Unchanged => PendingSequenceCancellation::None,
        },
    };
    match (keyboard_handover, active_routing_transition) {
        (KeyboardHandover::Crossed, ActiveRoutingTransition::Unchanged) => {
            reset_routing(world, pending_sequence_cancellation, |_| {});
        },
        (
            KeyboardHandover::Crossed,
            ActiveRoutingTransition::Initial
            | ActiveRoutingTransition::Recovered
            | ActiveRoutingTransition::GenerationChanged,
        )
        | (
            KeyboardHandover::Unchanged,
            ActiveRoutingTransition::Recovered | ActiveRoutingTransition::GenerationChanged,
        ) => reset_routing(world, pending_sequence_cancellation, |keymap_runtime| {
            keymap_runtime.record_active_routing(generation);
        }),
        (KeyboardHandover::Unchanged, ActiveRoutingTransition::Initial) => world
            .resource_mut::<KeymapRuntime>()
            .record_active_routing(generation),
        (KeyboardHandover::Unchanged, ActiveRoutingTransition::Unchanged) => {},
    }
}

fn route_releases(world: &mut World) {
    clear_processed_keycodes(world);
    while let Some(key) = next_released_key(world) {
        let pressed_modifiers = world
            .get_resource::<ButtonInput<KeyCode>>()
            .map(Modifiers::from_pressed)
            .unwrap_or_default();
        let custom_input_transition = {
            let mut keymap_runtime = world.resource_mut::<KeymapRuntime>();
            keymap_runtime.mark_processed(key);
            keymap_runtime.release_inhibition(key);
            keymap_runtime.release_key(key)
        };
        write_custom_input_transition(world, custom_input_transition);
        release_chords_missing_modifiers(world, pressed_modifiers);
    }
}

fn route_presses(world: &mut World, keystroke_routing: &KeystrokeRouting) {
    clear_processed_keycodes(world);
    let primary_trigger_ownership = world.get_resource::<ButtonInput<KeyCode>>().map_or(
        PrimaryTriggerOwnership::Unclaimed,
        PrimaryTriggerOwnership::from,
    );

    match primary_trigger_ownership {
        PrimaryTriggerOwnership::Unclaimed => {},
        PrimaryTriggerOwnership::ModifierFamilies => {
            activate_modifier_family_held_bindings(world, keystroke_routing);
        },
        PrimaryTriggerOwnership::OrdinaryKeys(ordinary_key_routing_state) => {
            suspend_modifier_family_held_bindings(world);
            route_ordinary_key_presses(world, keystroke_routing);
            match ordinary_key_routing_state {
                OrdinaryKeyRoutingState::Held => {},
                OrdinaryKeyRoutingState::PressEdgesOnly => {
                    activate_modifier_family_held_bindings(world, keystroke_routing);
                },
            }
        },
    }
}

fn route_ordinary_key_presses(world: &mut World, keystroke_routing: &KeystrokeRouting) {
    while let Some(key) = next_pressed_key(world) {
        world.resource_mut::<KeymapRuntime>().mark_processed(key);
        if world.resource::<KeymapRuntime>().is_inhibited(key) {
            continue;
        }
        let PhysicalKeyRole::OrdinaryKey(ordinary_key) = PhysicalKeyRole::from(key) else {
            continue;
        };

        let Some(keystroke) = world
            .get_resource::<ButtonInput<KeyCode>>()
            .map(|pressed| key_edge::keystroke(pressed, ordinary_key))
        else {
            continue;
        };
        let held_chord_physical_ownership =
            HeldChordPhysicalOwnership::new(key, keystroke.modifiers());
        let routed_commands = route_keystroke(world, keystroke, keystroke_routing);
        claim_held_chords(world, routed_commands, held_chord_physical_ownership);
        release_physical_if_no_longer_pressed(world, key);
        dispatch_all(world, routed_commands);
    }
}

/// Activates the held custom input of every hold-to-act command the pressed chord matched.
fn claim_held_chords(
    world: &mut World,
    routed_commands: RoutedCommands,
    held_chord_physical_ownership: HeldChordPhysicalOwnership,
) {
    claim_held_chord(world, routed_commands.first, held_chord_physical_ownership);
    claim_held_chord(world, routed_commands.second, held_chord_physical_ownership);
}

fn claim_held_chord(
    world: &mut World,
    routed_command: RoutedCommand,
    held_chord_physical_ownership: HeldChordPhysicalOwnership,
) {
    let RoutedCommand::HoldChord(custom_input) = routed_command else {
        return;
    };
    let custom_input_transition = world
        .resource_mut::<KeymapRuntime>()
        .activate_ordinary_chord(held_chord_physical_ownership, custom_input);
    write_custom_input_transition(world, custom_input_transition);
}

fn release_physical_if_no_longer_pressed(world: &mut World, key: KeyCode) {
    let key_remains_pressed = world
        .get_resource::<ButtonInput<KeyCode>>()
        .is_some_and(|key_input| key_input.pressed(key));
    let pressed_modifiers = world
        .get_resource::<ButtonInput<KeyCode>>()
        .map(Modifiers::from_pressed)
        .unwrap_or_default();
    if !key_remains_pressed {
        let custom_input_transition = world.resource_mut::<KeymapRuntime>().release_key(key);
        write_custom_input_transition(world, custom_input_transition);
    }
    release_chords_missing_modifiers(world, pressed_modifiers);
}

/// Activates the bare-modifier held bindings of the keymap, unless a text field
/// owns the keyboard — a held modifier is never the command that closes a field,
/// so no exemption reaches this path.
fn activate_modifier_family_held_bindings(world: &mut World, keystroke_routing: &KeystrokeRouting) {
    if matches!(keystroke_routing, KeystrokeRouting::TextEntry { .. }) {
        return;
    }
    for key in key_edge::PHYSICAL_MODIFIER_KEYS {
        let is_pressed = world
            .get_resource::<ButtonInput<KeyCode>>()
            .is_some_and(|pressed| pressed.pressed(key));
        if !is_pressed || world.resource::<KeymapRuntime>().is_inhibited(key) {
            continue;
        }
        let PhysicalKeyRole::ModifierFamily(modifier_family) = PhysicalKeyRole::from(key) else {
            continue;
        };
        let modifier_family_held_binding = world
            .resource::<CompiledKeymap>()
            .modifier_family_held_binding(modifier_family);
        let ModifierFamilyHeldBinding::Bound(custom_input) = modifier_family_held_binding else {
            continue;
        };

        let custom_input_transition = world
            .resource_mut::<KeymapRuntime>()
            .activate_modifier_family(key, custom_input);
        write_custom_input_transition(world, custom_input_transition);
    }
}

fn suspend_modifier_family_held_bindings(world: &mut World) {
    for key in key_edge::PHYSICAL_MODIFIER_KEYS {
        let custom_input_transition = world.resource_mut::<KeymapRuntime>().release_key(key);
        write_custom_input_transition(world, custom_input_transition);
    }
}

fn clear_processed_keycodes(world: &mut World) {
    world
        .resource_mut::<KeymapRuntime>()
        .clear_processed_keycodes();
}

fn next_pressed_key(world: &World) -> Option<KeyCode> {
    let keymap_runtime = world.get_resource::<KeymapRuntime>()?;
    world
        .get_resource::<ButtonInput<KeyCode>>()?
        .get_just_pressed()
        .copied()
        .find(|key| !keymap_runtime.is_processed(*key))
}

fn next_released_key(world: &World) -> Option<KeyCode> {
    let keymap_runtime = world.get_resource::<KeymapRuntime>()?;
    world
        .get_resource::<ButtonInput<KeyCode>>()?
        .get_just_released()
        .copied()
        .find(|key| !keymap_runtime.is_processed(*key))
}

fn route_keystroke(
    world: &mut World,
    keystroke: crate::Keystroke,
    keystroke_routing: &KeystrokeRouting,
) -> RoutedCommands {
    world.resource_scope::<CompiledKeymap, _>(|world, mut compiled_keymap| {
        let now = world.resource::<KeymapRuntime>().now();
        let match_outcome =
            compiled_keymap
                .matcher
                .match_keystroke(keystroke, now, SEQUENCE_TIMEOUT);
        let mut routed_commands = RoutedCommands::default();
        route_match_outcome(
            &mut compiled_keymap,
            now,
            match_outcome,
            keystroke_routing,
            &mut routed_commands,
        );

        routed_commands
    })
}

fn route_match_outcome(
    compiled_keymap: &mut CompiledKeymap,
    now: Instant,
    match_outcome: MatchOutcome<CommandHandle>,
    keystroke_routing: &KeystrokeRouting,
    routed_commands: &mut RoutedCommands,
) {
    match match_outcome {
        MatchOutcome::Matched(command_handle) => {
            routed_commands.push(routed_command(
                compiled_keymap,
                command_handle,
                keystroke_routing,
            ));
        },
        MatchOutcome::Reprocess {
            deferred_match,
            keystroke,
        } => {
            if let DeferredMatch::Fire(command_handle) = deferred_match {
                routed_commands.push(routed_command(
                    compiled_keymap,
                    command_handle,
                    keystroke_routing,
                ));
            }
            if let MatchOutcome::Matched(command_handle) =
                compiled_keymap
                    .matcher
                    .match_keystroke(keystroke, now, SEQUENCE_TIMEOUT)
            {
                routed_commands.push(routed_command(
                    compiled_keymap,
                    command_handle,
                    keystroke_routing,
                ));
            }
        },
        MatchOutcome::Deferred(_) | MatchOutcome::NoMatch | MatchOutcome::Pending => {},
    }
}

/// Resolves what a matched keystroke does, which is nothing at all while a text
/// field owns the keyboard and the matched command is not one it exempts.
fn routed_command(
    compiled_keymap: &CompiledKeymap,
    command_handle: CommandHandle,
    keystroke_routing: &KeystrokeRouting,
) -> RoutedCommand {
    if !compiled_keymap
        .command_id(command_handle)
        .is_some_and(|command_id| keystroke_routing.routes(command_id))
    {
        return RoutedCommand::Nothing;
    }
    match compiled_keymap.invocation(command_handle) {
        Some(Invocation::Held(custom_input)) => RoutedCommand::HoldChord(custom_input),
        Some(Invocation::OneShot | Invocation::Unremappable) => compiled_keymap
            .dispatch(command_handle)
            .map_or(RoutedCommand::Nothing, RoutedCommand::Dispatch),
        None => RoutedCommand::Nothing,
    }
}

fn write_custom_input_transition(
    world: &mut World,
    custom_input_transition: CustomInputTransition,
) {
    custom_input_transition.write_to(&mut world.resource_mut::<CustomInputs>());
}

fn release_chords_missing_modifiers(world: &mut World, pressed_modifiers: Modifiers) {
    loop {
        let physical_source_release_progress = world
            .resource_mut::<KeymapRuntime>()
            .release_one_chord_missing_modifiers(pressed_modifiers);
        match physical_source_release_progress {
            PhysicalSourceReleaseProgress::ReleasedOne(custom_input_transition) => {
                write_custom_input_transition(world, custom_input_transition);
            },
            PhysicalSourceReleaseProgress::Complete => break,
        }
    }
}

fn dispatch_all(world: &mut World, routed_commands: RoutedCommands) {
    dispatch_one(world, routed_commands.first);
    dispatch_one(world, routed_commands.second);
}

fn dispatch_one(world: &mut World, routed_command: RoutedCommand) {
    match routed_command {
        RoutedCommand::Dispatch(dispatch) => dispatch(world),
        RoutedCommand::HoldChord(_) | RoutedCommand::Nothing => {},
    }
}

/// What a matched keystroke resolves to before the runtime acts on it.
///
/// `routed_command` builds this from the compiled keymap and [`KeystrokeRouting`] alone; each
/// caller then acts only on the variants it handles, so the sequence-timeout path, which calls
/// `dispatch_all` and not `claim_held_chords`, drops a [`RoutedCommand::HoldChord`] that has no
/// physical key to own it.
#[derive(Clone, Copy, Default)]
enum RoutedCommand {
    #[default]
    Nothing,
    Dispatch(fn(&mut World)),
    HoldChord(CustomInput),
}

/// The commands one keystroke can resolve to.
///
/// A keystroke yields at most two: a sequence prefix that a longer sequence just abandoned, plus
/// the command the reprocessed keystroke matches on its own.
#[derive(Clone, Copy, Default)]
struct RoutedCommands {
    first:  RoutedCommand,
    second: RoutedCommand,
}

impl RoutedCommands {
    const fn push(&mut self, routed_command: RoutedCommand) {
        if matches!(routed_command, RoutedCommand::Nothing) {
            return;
        }
        if matches!(self.first, RoutedCommand::Nothing) {
            self.first = routed_command;
        } else {
            self.second = routed_command;
        }
    }
}

impl From<RoutedCommand> for RoutedCommands {
    fn from(routed_command: RoutedCommand) -> Self {
        let mut routed_commands = Self::default();
        routed_commands.push(routed_command);
        routed_commands
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "runtime command declarations generate action marker types used through the registry"
)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use bevy::ecs::spawn::SpawnRelated;
    use bevy::ecs::spawn::SpawnWith;
    use bevy::input::ButtonInput;
    use bevy::input::keyboard::KeyCode;
    use bevy::prelude::App;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::Event;
    use bevy::prelude::On;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::With;
    use bevy::reflect::TypeRegistry;
    use bevy::time::TimePlugin;
    use bevy_enhanced_input::bindings;
    use bevy_enhanced_input::prelude::Action;
    use bevy_enhanced_input::prelude::ActionSpawner;
    use bevy_enhanced_input::prelude::ActionValue;
    use bevy_enhanced_input::prelude::Actions;
    use bevy_enhanced_input::prelude::Binding;
    use bevy_enhanced_input::prelude::Complete;
    use bevy_enhanced_input::prelude::CustomInput;
    use bevy_enhanced_input::prelude::CustomInputs;
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use bevy_enhanced_input::prelude::InputAction;
    use bevy_enhanced_input::prelude::InputContextAppExt;
    use bevy_enhanced_input::prelude::Start;

    use super::KeymapRuntime;
    use super::route_input;
    use crate::ActiveKeymapContext;
    use crate::CommandId;
    use crate::CommandRegistry;
    use crate::DiagnosticOrigin;
    use crate::EffectiveKeymapPublication;
    use crate::EffectiveKeymapSnapshot;
    use crate::EffectiveKeymapStatus;
    use crate::HoldPhase;
    use crate::KeymapCommand;
    use crate::KeymapPlugin;
    use crate::KeystrokeSequence;
    use crate::ReflectKeymapCommand;
    use crate::SequenceMatcher;
    use crate::command::Invocation;
    use crate::condition::StateDimensionRegistry;
    use crate::keymap::AcceptedKeymapDocument;
    use crate::keymap::CommandHandle;
    use crate::keymap::CompiledKeymap;
    use crate::keymap::KeyboardOwner;
    use crate::keymap::KeymapGeneration;
    use crate::keymap::KeystrokeRouting;
    use crate::keymap::MergedKeymap;
    use crate::keymap::merged::UserKeymap;

    const DEFAULTS_PATH: &str = "runtime-defaults.jsonc";
    const FIRST_GENERATION: KeymapGeneration = KeymapGeneration::initial().next();
    const SECOND_GENERATION: KeymapGeneration = FIRST_GENERATION.next();

    fn defaults_keymap_file() -> DiagnosticOrigin {
        DiagnosticOrigin::KeymapFile(PathBuf::from(DEFAULTS_PATH))
    }

    crate::command! {
        action:      RuntimeOneShotAction,
        event:       RuntimeOneShot,
        id:          "runtime::one_shot",
        title:       "Runtime One Shot",
        description: "Dispatches one event through the keymap runtime.",
    }

    #[derive(Component)]
    struct RuntimeInputContext;

    crate::command! {
        action:      RuntimeTwoStrokeAction,
        event:       RuntimeTwoStroke,
        id:          "runtime::two_stroke",
        title:       "Runtime Two Stroke",
        description: "Dispatches after the second keymap stroke.",
    }

    crate::command! {
        held,
        action:      RuntimeHeldAction,
        event:       RuntimeHeld,
        id:          "runtime::held",
        title:       "Runtime Held",
        description: "Writes a custom input while the matched key is pressed.",
    }

    crate::command! {
        held,
        action:      RuntimeShiftHeldAction,
        event:       RuntimeShiftHeld,
        id:          "runtime::shift_held",
        title:       "Runtime Shift Held",
        description: "Writes a second custom input while its modifier family is pressed.",
    }

    crate::command! {
        held,
        action:      RuntimeAltHeldAction,
        event:       RuntimeAltHeld,
        id:          "runtime::alt_held",
        title:       "Runtime Alt Held",
        description: "Writes a third custom input while its modifier family is pressed.",
    }

    crate::command! {
        action:      RuntimeUnremappableAction,
        event:       RuntimeUnremappable,
        id:          "runtime::unremappable",
        title:       "Runtime Unremappable",
        description: "Dispatches through an opaque compiled command handle.",
        capability:  Unremappable,
    }

    #[derive(Debug, Default, Eq, PartialEq, Resource)]
    struct DispatchCounts {
        one_shot:     usize,
        two_stroke:   usize,
        unremappable: usize,
    }

    #[derive(Debug, Default, Eq, PartialEq, Resource)]
    struct HeldTransitionCounts {
        started:   usize,
        completed: usize,
    }

    #[test]
    fn single_stroke_dispatches_its_semantic_event() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID)]),
            FIRST_GENERATION,
        )?;

        press(&mut app, KeyCode::KeyG);

        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 1);
        Ok(())
    }

    #[test]
    fn same_frame_press_and_release_dispatches_one_shot() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID)]),
            FIRST_GENERATION,
        )?;
        {
            let mut key_input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            key_input.press(KeyCode::KeyG);
            key_input.release(KeyCode::KeyG);
        }

        route_input(app.world_mut());

        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 1);
        Ok(())
    }

    #[test]
    fn missing_compiled_keymap_returns_without_routing() {
        let mut app = runtime_app();

        route_input(app.world_mut());

        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 0);
    }

    #[test]
    fn two_stroke_sequence_dispatches_on_its_second_stroke() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g h", RuntimeTwoStroke::ID)]),
            FIRST_GENERATION,
        )?;

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        assert_eq!(app.world().resource::<DispatchCounts>().two_stroke, 0);
        press(&mut app, KeyCode::KeyH);

        assert_eq!(app.world().resource::<DispatchCounts>().two_stroke, 1);
        Ok(())
    }

    #[test]
    fn deferred_short_binding_dispatches_when_the_runtime_clock_reaches_the_timeout()
    -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID), ("g h", RuntimeTwoStroke::ID)]),
            FIRST_GENERATION,
        )?;
        let now = Instant::now();
        app.world_mut()
            .resource_mut::<KeymapRuntime>()
            .set_test_clock(now);

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        assert!(
            app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );

        app.world_mut()
            .resource_mut::<KeymapRuntime>()
            .set_test_clock(now + crate::keymap::constants::SEQUENCE_TIMEOUT);
        route_input(app.world_mut());

        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 1);
        assert!(
            !app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );
        Ok(())
    }

    #[test]
    fn held_binding_writes_its_custom_input_on_press_and_release() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        release(&mut app, KeyCode::KeyG);

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn a_key_released_while_a_text_field_owns_the_keyboard_does_not_stay_held() -> Result<(), String>
    {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        app.world_mut()
            .insert_resource(KeystrokeRouting::text_entry(query_field(), []));
        route_input(app.world_mut());
        release(&mut app, KeyCode::KeyG);
        app.world_mut()
            .insert_resource(KeystrokeRouting::EveryBinding);
        route_input(app.world_mut());

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );

        press(&mut app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        release(&mut app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn a_held_command_goes_false_at_the_handover_to_a_text_field() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        app.world_mut()
            .insert_resource(KeystrokeRouting::text_entry(query_field(), []));
        route_input(app.world_mut());

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    /// The other direction of the same handover: an exempt hold-to-act command
    /// is the one held input a text field can leave active, so handing the
    /// keyboard back is where it goes false.
    #[test]
    fn a_held_command_goes_false_at_the_handover_back_to_the_keymap() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;
        app.world_mut()
            .insert_resource(KeystrokeRouting::text_entry(
                query_field(),
                [CommandId::declared::<RuntimeHeld>()],
            ));
        route_input(app.world_mut());

        press(&mut app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        app.world_mut()
            .insert_resource(KeystrokeRouting::EveryBinding);
        route_input(app.world_mut());

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn a_pending_sequence_is_cancelled_at_the_handover_to_a_text_field() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g h", RuntimeTwoStroke::ID)]),
            FIRST_GENERATION,
        )?;

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        assert_eq!(app.world().resource::<DispatchCounts>().two_stroke, 0);

        app.world_mut()
            .insert_resource(KeystrokeRouting::text_entry(
                query_field(),
                [CommandId::declared::<RuntimeTwoStroke>()],
            ));
        route_input(app.world_mut());
        press(&mut app, KeyCode::KeyH);
        release(&mut app, KeyCode::KeyH);

        assert_eq!(app.world().resource::<DispatchCounts>().two_stroke, 0);
        Ok(())
    }

    /// The other direction of the same handover: an exempt multi-stroke command
    /// can leave a sequence pending while the field owns the keyboard, and
    /// handing the keyboard back cancels it.
    #[test]
    fn a_pending_sequence_is_cancelled_at_the_handover_back_to_the_keymap() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g h", RuntimeTwoStroke::ID)]),
            FIRST_GENERATION,
        )?;
        app.world_mut()
            .insert_resource(KeystrokeRouting::text_entry(
                query_field(),
                [CommandId::declared::<RuntimeTwoStroke>()],
            ));
        route_input(app.world_mut());

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        assert!(
            app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );

        app.world_mut()
            .insert_resource(KeystrokeRouting::EveryBinding);
        route_input(app.world_mut());
        press(&mut app, KeyCode::KeyH);
        release(&mut app, KeyCode::KeyH);

        assert!(
            !app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );
        assert_eq!(app.world().resource::<DispatchCounts>().two_stroke, 0);
        Ok(())
    }

    /// The other direction of the same handover. The inhibition is asserted on
    /// [`KeymapRuntime`] directly because a text field suppresses the
    /// bare-modifier held bindings outright, so nothing downstream of it can
    /// tell an inhibited key from a suppressed one.
    #[test]
    fn a_key_down_at_the_handover_to_a_text_field_is_inhibited() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("ctrl", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::ControlLeft);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        app.world_mut()
            .insert_resource(KeystrokeRouting::text_entry(query_field(), []));
        route_input(app.world_mut());

        assert!(
            app.world()
                .resource::<KeymapRuntime>()
                .is_inhibited(KeyCode::ControlLeft)
        );
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn a_key_down_at_the_handover_back_to_the_keymap_stays_inhibited_until_released()
    -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("ctrl", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;
        app.world_mut()
            .insert_resource(KeystrokeRouting::text_entry(query_field(), []));
        route_input(app.world_mut());

        press(&mut app, KeyCode::ControlLeft);
        assert_ne!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        app.world_mut()
            .insert_resource(KeystrokeRouting::EveryBinding);
        route_input(app.world_mut());
        route_input(app.world_mut());
        assert_ne!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        release(&mut app, KeyCode::ControlLeft);
        press(&mut app, KeyCode::ControlLeft);

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        Ok(())
    }

    #[test]
    fn text_entry_routes_the_commands_it_exempts_and_no_others() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID), ("h", RuntimeTwoStroke::ID)]),
            FIRST_GENERATION,
        )?;
        app.world_mut()
            .insert_resource(KeystrokeRouting::text_entry(
                query_field(),
                [CommandId::declared::<RuntimeOneShot>()],
            ));
        route_input(app.world_mut());

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        press(&mut app, KeyCode::KeyH);
        release(&mut app, KeyCode::KeyH);

        let dispatch_counts = app.world().resource::<DispatchCounts>();
        assert_eq!(dispatch_counts.one_shot, 1);
        assert_eq!(dispatch_counts.two_stroke, 0);
        Ok(())
    }

    #[test]
    fn same_frame_press_and_release_leaves_held_input_inactive() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;
        {
            let mut key_input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            key_input.press(KeyCode::KeyG);
            key_input.release(KeyCode::KeyG);
        }

        route_input(app.world_mut());

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn modifier_family_binding_counts_left_and_right_shift_as_one_hold() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("shift", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;
        spawn_held_action(&mut app, custom_input)?;

        press(&mut app, KeyCode::ShiftLeft);
        app.update();
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        press(&mut app, KeyCode::ShiftRight);
        release(&mut app, KeyCode::ShiftLeft);
        app.update();
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        release(&mut app, KeyCode::ShiftRight);
        app.update();

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        assert_eq!(
            *app.world().resource::<HeldTransitionCounts>(),
            HeldTransitionCounts {
                started:   1,
                completed: 1,
            }
        );
        Ok(())
    }

    #[test]
    fn shifted_key_suspends_its_bare_hold_until_the_key_is_released() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("shift", RuntimeHeld::ID), ("shift-f", RuntimeOneShot::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::ShiftLeft);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        press(&mut app, KeyCode::KeyF);

        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 1);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        release(&mut app, KeyCode::KeyF);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        release(&mut app, KeyCode::ShiftLeft);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn unroutable_key_press_leaves_a_bare_modifier_hold_active() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("shift", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;
        spawn_held_action(&mut app, custom_input)?;

        press(&mut app, KeyCode::ShiftLeft);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        press(&mut app, KeyCode::AudioVolumeUp);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        app.update();

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        assert_eq!(
            *app.world().resource::<HeldTransitionCounts>(),
            HeldTransitionCounts {
                started:   1,
                completed: 0,
            }
        );
        Ok(())
    }

    #[test]
    fn same_frame_shift_and_key_press_never_activates_the_bare_hold() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("shift", RuntimeHeld::ID), ("shift-f", RuntimeOneShot::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;
        spawn_held_action(&mut app, custom_input)?;
        {
            let mut pressed = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            pressed.press(KeyCode::ShiftLeft);
            pressed.press(KeyCode::KeyF);
        }

        route_input(app.world_mut());
        {
            let mut pressed = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            pressed.clear_just_pressed(KeyCode::ShiftLeft);
            pressed.clear_just_pressed(KeyCode::KeyF);
        }
        app.update();

        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 1);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            None
        );
        assert_eq!(
            *app.world().resource::<HeldTransitionCounts>(),
            HeldTransitionCounts::default()
        );
        Ok(())
    }

    #[test]
    fn modified_held_chord_ends_when_primary_key_is_released_first() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("shift-f", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::ShiftLeft);
        press(&mut app, KeyCode::KeyF);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        release(&mut app, KeyCode::KeyF);

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::ShiftLeft)
        );
        Ok(())
    }

    #[test]
    fn modified_held_chord_ends_when_required_modifier_is_released_first() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("shift-f", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::ShiftLeft);
        press(&mut app, KeyCode::KeyF);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        release(&mut app, KeyCode::ShiftLeft);
        route_input(app.world_mut());

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::KeyF)
        );
        Ok(())
    }

    #[test]
    fn event_source_release_keeps_a_physical_held_binding_active() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::KeyG);
        app.world_mut().trigger(RuntimeHeld {
            phase: HoldPhase::Begin,
        });
        app.world_mut().trigger(RuntimeHeld {
            phase: HoldPhase::End,
        });

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        release(&mut app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn event_source_remains_active_across_a_generation_change() -> Result<(), String> {
        let mut app = runtime_app();
        let command_registry = command_registry(&mut app)?;
        let first = compile(
            bindings(&[("g", RuntimeHeld::ID)]),
            &command_registry,
            FIRST_GENERATION,
        )?;
        let second = compile(
            bindings(&[("h", RuntimeHeld::ID)]),
            &command_registry,
            SECOND_GENERATION,
        )?;
        replace_compiled(&mut app, first);
        app.world_mut().init_resource::<KeymapRuntime>();
        let custom_input = held_custom_input(&app)?;
        route_input(app.world_mut());

        app.world_mut().trigger(RuntimeHeld {
            phase: HoldPhase::Begin,
        });
        replace_compiled(&mut app, second);
        route_input(app.world_mut());

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        Ok(())
    }

    #[test]
    fn held_binding_drives_one_start_and_complete_through_enhanced_input() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;
        spawn_held_action(&mut app, custom_input)?;

        press(&mut app, KeyCode::KeyG);
        app.update();
        release(&mut app, KeyCode::KeyG);
        app.update();

        assert_eq!(
            *app.world().resource::<HeldTransitionCounts>(),
            HeldTransitionCounts {
                started:   1,
                completed: 1,
            }
        );
        Ok(())
    }

    #[test]
    fn remapping_an_unpressed_held_binding_changes_its_source_without_replacing_its_action()
    -> Result<(), String> {
        let mut app = runtime_app();
        let command_registry = command_registry(&mut app)?;
        let first = compile(
            bindings(&[("g", RuntimeHeld::ID)]),
            &command_registry,
            FIRST_GENERATION,
        )?;
        let second = compile(
            bindings(&[("h", RuntimeHeld::ID)]),
            &command_registry,
            SECOND_GENERATION,
        )?;
        replace_compiled(&mut app, first);
        app.world_mut().init_resource::<KeymapRuntime>();
        let custom_input = held_custom_input(&app)?;
        let action_entity = spawn_held_action(&mut app, custom_input)?;

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        replace_compiled(&mut app, second);
        press(&mut app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        release(&mut app, KeyCode::KeyG);
        press(&mut app, KeyCode::KeyH);

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        assert!(app.world().get_entity(action_entity).is_ok());
        Ok(())
    }

    #[test]
    fn generation_change_inhibits_already_pressed_keys_until_release() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID)]),
            FIRST_GENERATION,
        )?;

        route_input(app.world_mut());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        let command_registry = command_registry(&mut app)?;
        let replacement = compile(
            bindings(&[("g", RuntimeOneShot::ID)]),
            &command_registry,
            SECOND_GENERATION,
        )?;
        replace_compiled(&mut app, replacement);
        route_input(app.world_mut());
        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 0);
        assert!(
            app.world()
                .resource::<KeymapRuntime>()
                .is_inhibited(KeyCode::KeyG)
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::KeyG);

        release(&mut app, KeyCode::KeyG);
        press(&mut app, KeyCode::KeyG);
        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 1);
        Ok(())
    }

    #[test]
    fn recovery_cancellation_clears_a_pending_global_sequence() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID), ("g h", RuntimeTwoStroke::ID)]),
            FIRST_GENERATION,
        )?;

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        assert!(
            app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );

        crate::cancel_pending_sequences(app.world_mut());

        assert!(
            !app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );
        press(&mut app, KeyCode::KeyH);
        assert_eq!(app.world().resource::<DispatchCounts>().two_stroke, 0);
        Ok(())
    }

    #[test]
    fn physical_input_reset_cancels_sequences_and_refreshes_held_state() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID), ("h j", RuntimeTwoStroke::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::KeyG);
        press(&mut app, KeyCode::KeyH);
        release(&mut app, KeyCode::KeyH);
        assert!(
            app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        crate::reset_physical_input(app.world_mut());

        assert!(
            !app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        assert!(
            app.world()
                .resource::<KeymapRuntime>()
                .is_inhibited(KeyCode::KeyG)
        );
        press(&mut app, KeyCode::KeyJ);
        assert_eq!(app.world().resource::<DispatchCounts>().two_stroke, 0);

        app.world_mut().trigger(RuntimeHeld {
            phase: HoldPhase::Begin,
        });
        crate::reset_physical_input(app.world_mut());
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyG);
        crate::reset_physical_input(app.world_mut());
        assert!(
            !app.world()
                .resource::<KeymapRuntime>()
                .is_inhibited(KeyCode::KeyG)
        );
        app.world_mut().trigger(RuntimeHeld {
            phase: HoldPhase::End,
        });
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_released(KeyCode::KeyG);
        press(&mut app, KeyCode::KeyG);

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        Ok(())
    }

    #[test]
    fn keymap_plugin_routes_global_bindings_without_a_context_plugin() -> Result<(), String> {
        let mut app = runtime_app();
        app.add_plugins(KeymapPlugin::new());
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID)]),
            FIRST_GENERATION,
        )?;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);

        app.update();

        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 1);
        Ok(())
    }

    #[test]
    fn modifier_edges_leave_pending_sequences_armed() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID), ("g h", RuntimeTwoStroke::ID)]),
            FIRST_GENERATION,
        )?;

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        press(&mut app, KeyCode::ShiftLeft);

        assert!(
            app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );
        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 0);
        Ok(())
    }

    #[test]
    fn physical_super_prevents_a_bare_binding_from_dispatching() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID)]),
            FIRST_GENERATION,
        )?;

        press(&mut app, KeyCode::SuperLeft);
        press(&mut app, KeyCode::KeyG);

        assert_eq!(app.world().resource::<DispatchCounts>().one_shot, 0);
        Ok(())
    }

    #[test]
    fn unremappable_entries_dispatch_through_the_compiled_function_pointer() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeOneShot::ID)]),
            FIRST_GENERATION,
        )?;
        let command_handle = app
            .world()
            .resource::<CompiledKeymap>()
            .commands
            .iter()
            .position(|(_, command_entry)| {
                matches!(command_entry.invocation(), Invocation::Unremappable)
            })
            .map(CommandHandle::from_index)
            .ok_or_else(|| "runtime registry did not compile its unremappable entry".to_owned())?;
        let sequence = "u"
            .parse::<KeystrokeSequence>()
            .map_err(|error| format!("invalid unremappable test sequence: {error}"))?;
        app.world_mut().resource_mut::<CompiledKeymap>().matcher =
            SequenceMatcher::new([(sequence, command_handle)]);

        press(&mut app, KeyCode::KeyU);

        assert_eq!(app.world().resource::<DispatchCounts>().unremappable, 1);
        Ok(())
    }

    #[test]
    fn steady_state_held_routing_does_not_allocate() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        let allocations_before = crate::TEST_ALLOCATOR.allocation_count();
        route_input(app.world_mut());
        let allocations_after = crate::TEST_ALLOCATOR.allocation_count();

        assert_eq!(allocations_after - allocations_before, 0);
        Ok(())
    }

    /// The warm-up presses and releases the same four keys together so every routing structure is
    /// already populated, and takes one empty [`World::resource_scope`] because Bevy's own
    /// first-touch cost lands there; the measured pass then reports only what routing allocates
    /// while it activates all four keys at once.
    #[test]
    fn simultaneous_modifier_family_held_routing_does_not_allocate() -> Result<(), String> {
        const MODIFIER_KEYS: [KeyCode; 4] = [
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::ShiftLeft,
            KeyCode::AltLeft,
        ];

        let mut app = runtime_app();
        insert_compiled_for_modifier_family_holds(
            &mut app,
            bindings(&[
                ("ctrl", RuntimeHeld::ID),
                ("shift", RuntimeShiftHeld::ID),
                ("alt", RuntimeAltHeld::ID),
            ]),
            FIRST_GENERATION,
        )?;

        press_together(&mut app, MODIFIER_KEYS);
        release_together(&mut app, MODIFIER_KEYS);
        for key in MODIFIER_KEYS {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
        }
        app.world_mut()
            .resource_scope::<CompiledKeymap, _>(|_, _| {});

        let allocations_before = crate::TEST_ALLOCATOR.allocation_count();
        route_input(app.world_mut());
        let allocations_after = crate::TEST_ALLOCATOR.allocation_count();

        assert_eq!(allocations_after - allocations_before, 0);
        Ok(())
    }

    #[test]
    fn same_frame_held_tap_does_not_allocate() -> Result<(), String> {
        let mut app = runtime_app();
        insert_compiled(
            &mut app,
            bindings(&[("g", RuntimeHeld::ID)]),
            FIRST_GENERATION,
        )?;
        let custom_input = held_custom_input(&app)?;

        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        {
            let mut key_input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            key_input.press(KeyCode::KeyG);
            key_input.release(KeyCode::KeyG);
        }
        let allocations_before = crate::TEST_ALLOCATOR.allocation_count();
        route_input(app.world_mut());
        let allocations_after = crate::TEST_ALLOCATOR.allocation_count();

        assert_eq!(allocations_after - allocations_before, 0);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    /// The text field the handover tests hand the keyboard to.
    struct QueryField;

    fn query_field() -> KeyboardOwner { KeyboardOwner::of::<QueryField>() }

    fn runtime_app() -> App {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<CustomInputs>()
            .init_resource::<DispatchCounts>()
            .init_resource::<HeldTransitionCounts>();
        app.world_mut().add_observer(
            |_: On<RuntimeOneShot>, mut dispatch_counts: ResMut<DispatchCounts>| {
                dispatch_counts.one_shot += 1;
            },
        );
        app.world_mut().add_observer(
            |_: On<RuntimeTwoStroke>, mut dispatch_counts: ResMut<DispatchCounts>| {
                dispatch_counts.two_stroke += 1;
            },
        );
        app.world_mut().add_observer(
            |_: On<RuntimeUnremappable>, mut dispatch_counts: ResMut<DispatchCounts>| {
                dispatch_counts.unremappable += 1;
            },
        );
        app.world_mut().add_observer(
            |_: On<Start<RuntimeHeldAction>>,
             mut transition_counts: ResMut<HeldTransitionCounts>| {
                transition_counts.started += 1;
            },
        );
        app.world_mut().add_observer(
            |_: On<Complete<RuntimeHeldAction>>,
             mut transition_counts: ResMut<HeldTransitionCounts>| {
                transition_counts.completed += 1;
            },
        );
        app
    }

    fn insert_compiled(
        app: &mut App,
        source: String,
        generation: KeymapGeneration,
    ) -> Result<(), String> {
        let command_registry = command_registry(app)?;

        insert_compiled_for_registry(app, command_registry, source, generation)
    }

    /// Compiles against a registry holding one held command per modifier family, so several
    /// distinct custom inputs can be active at once.
    fn insert_compiled_for_modifier_family_holds(
        app: &mut App,
        source: String,
        generation: KeymapGeneration,
    ) -> Result<(), String> {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RuntimeAltHeld>();
        type_registry.register::<RuntimeHeld>();
        type_registry.register::<RuntimeShiftHeld>();
        let command_registry = built_command_registry(app, &type_registry)?;

        insert_compiled_for_registry(app, command_registry, source, generation)
    }

    fn insert_compiled_for_registry(
        app: &mut App,
        command_registry: CommandRegistry,
        source: String,
        generation: KeymapGeneration,
    ) -> Result<(), String> {
        let compiled_keymap = compile(source, &command_registry, generation)?;
        replace_compiled(app, compiled_keymap);
        app.world_mut().init_resource::<KeymapRuntime>();
        Ok(())
    }

    fn replace_compiled(app: &mut App, compiled_keymap: CompiledKeymap) {
        let generation = compiled_keymap.generation;
        app.world_mut().insert_resource(compiled_keymap);
        app.world_mut().init_resource::<ActiveKeymapContext>();
        app.world_mut()
            .insert_resource(EffectiveKeymapStatus::Loaded(EffectiveKeymapPublication {
                generation,
                snapshot: EffectiveKeymapSnapshot::Global,
                matched_layers: Vec::new(),
            }));
    }

    fn command_registry(app: &mut App) -> Result<CommandRegistry, String> {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RuntimeHeld>();
        type_registry.register::<RuntimeOneShot>();
        type_registry.register::<RuntimeTwoStroke>();
        type_registry.register::<RuntimeUnremappable>();

        built_command_registry(app, &type_registry)
    }

    fn command_registry_with_shift_held(app: &mut App) -> Result<CommandRegistry, String> {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RuntimeHeld>();
        type_registry.register::<RuntimeOneShot>();
        type_registry.register::<RuntimeShiftHeld>();
        type_registry.register::<RuntimeTwoStroke>();
        type_registry.register::<RuntimeUnremappable>();

        built_command_registry(app, &type_registry)
    }

    fn built_command_registry(
        app: &mut App,
        type_registry: &TypeRegistry,
    ) -> Result<CommandRegistry, String> {
        let command_registry = {
            let mut custom_inputs = app.world_mut().resource_mut::<CustomInputs>();
            CommandRegistry::build(type_registry, &mut custom_inputs).map_err(|diagnostics| {
                format!("runtime command registry errors: {diagnostics:?}")
            })?
        };
        command_registry.register_held_observers(app.world_mut());

        Ok(command_registry)
    }

    fn compile(
        source: String,
        command_registry: &CommandRegistry,
        generation: KeymapGeneration,
    ) -> Result<CompiledKeymap, String> {
        let state_dimensions = StateDimensionRegistry::default();
        let (accepted_document, diagnostics) = AcceptedKeymapDocument::from_sources(
            &defaults_keymap_file(),
            &source,
            &UserKeymap::DefaultsOnly,
            command_registry,
            &state_dimensions,
            &[],
        )
        .map_err(|diagnostics| format!("runtime keymap errors: {diagnostics:?}"))?;
        if !diagnostics.is_empty() {
            return Err(format!("runtime keymap diagnostics: {diagnostics:?}"));
        }

        let materialization = accepted_document.materialize(EffectiveKeymapSnapshot::Global);
        let (merged_keymap, materialization_diagnostics) =
            MergedKeymap::from_effective_state_bindings(materialization.bindings, command_registry);
        if !materialization_diagnostics.is_empty() {
            return Err(format!(
                "runtime materialization diagnostics: {materialization_diagnostics:?}"
            ));
        }

        Ok(merged_keymap.compile(generation, command_registry))
    }

    fn bindings(entries: &[(&str, &str)]) -> String {
        let bindings = entries
            .iter()
            .map(|(keystroke, command_id)| format!(r#""{keystroke}": "{command_id}""#))
            .collect::<Vec<_>>()
            .join(", ");

        format!(r#"{{ "bindings": [{{ "bindings": {{ {bindings} }} }}] }}"#)
    }

    fn held_custom_input(app: &App) -> Result<CustomInput, String> {
        held_custom_input_from_compiled(app.world().resource::<CompiledKeymap>())
    }

    fn held_custom_input_from_compiled(
        compiled_keymap: &CompiledKeymap,
    ) -> Result<CustomInput, String> {
        compiled_keymap
            .commands
            .iter()
            .find_map(|(_, command_entry)| match command_entry.invocation() {
                Invocation::Held(custom_input) => Some(custom_input),
                Invocation::OneShot | Invocation::Unremappable => None,
            })
            .ok_or_else(|| "runtime keymap has no held custom input".to_owned())
    }

    fn held_custom_input_for_command(
        compiled_keymap: &CompiledKeymap,
        command_id: &str,
    ) -> Result<CustomInput, String> {
        compiled_keymap
            .commands
            .iter()
            .find_map(|(declared_command_id, command_entry)| {
                if declared_command_id.as_str() != command_id {
                    return None;
                }
                match command_entry.invocation() {
                    Invocation::Held(custom_input) => Some(custom_input),
                    Invocation::OneShot | Invocation::Unremappable => None,
                }
            })
            .ok_or_else(|| format!("runtime keymap has no held custom input for {command_id}"))
    }

    fn spawn_held_action(app: &mut App, custom_input: CustomInput) -> Result<Entity, String> {
        app.add_plugins(TimePlugin);
        app.add_plugins(EnhancedInputPlugin);
        app.add_input_context::<RuntimeInputContext>();
        app.finish();
        app.world_mut().spawn((
            RuntimeInputContext,
            Actions::<RuntimeInputContext>::spawn(SpawnWith(
                move |action_spawner: &mut ActionSpawner<RuntimeInputContext>| {
                    action_spawner.spawn((
                        Action::<RuntimeHeldAction>::new(),
                        bindings![Binding::Custom(custom_input)],
                    ));
                },
            )),
        ));
        app.world_mut().flush();
        app.update();

        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, With<Action<RuntimeHeldAction>>>();
        query
            .single(world)
            .map_err(|_| "runtime held action was not spawned".to_owned())
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        route_input(app.world_mut());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(key);
    }

    fn release(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(key);
        route_input(app.world_mut());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_released(key);
    }

    fn press_together<const COUNT: usize>(app: &mut App, keys: [KeyCode; COUNT]) {
        for key in keys {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
        }
        route_input(app.world_mut());
        for key in keys {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .clear_just_pressed(key);
        }
    }

    fn release_together<const COUNT: usize>(app: &mut App, keys: [KeyCode; COUNT]) {
        for key in keys {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(key);
        }
        route_input(app.world_mut());
        for key in keys {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .clear_just_released(key);
        }
    }
}
