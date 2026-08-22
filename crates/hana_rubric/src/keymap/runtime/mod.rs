mod dispatch;
mod held;
mod key_edge;

use bevy::ecs::world::World;
use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy_enhanced_input::prelude::CustomInput;
use bevy_enhanced_input::prelude::CustomInputs;
pub(crate) use dispatch::cancel_pending_sequences;
pub(crate) use dispatch::route_input;
pub(crate) use held::KeymapRuntime;
use held::PhysicalSourceReleaseProgress;

/// Ordered observations of a context-loss reset for tests that exercise the public schedule.
#[cfg(test)]
#[derive(Default, Resource)]
pub(crate) struct RoutingResetTrace(pub(crate) Vec<RoutingResetStep>);

/// A completed stage of the shared context-loss reset transaction.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoutingResetStep {
    PendingSequenceCancelled,
    PhysicalSourcesReleased,
    RoutingStateRecorded,
    PressedKeysInhibited,
}

/// Which pending sequence state a routing reset discards before it releases physical sources.
enum PendingSequenceCancellation {
    None,
    EffectiveMatcher,
}

/// Resets physical routing in the order every routing-state boundary requires.
///
/// The caller records its new [`KeymapRuntime`] routing state after physical held sources release
/// and before keys currently held down become inhibited.
fn reset_routing(
    world: &mut World,
    pending_sequence_cancellation: PendingSequenceCancellation,
    record_routing_state: impl FnOnce(&mut KeymapRuntime),
) {
    match pending_sequence_cancellation {
        PendingSequenceCancellation::None => {},
        PendingSequenceCancellation::EffectiveMatcher => {
            dispatch::cancel_pending_effective_sequence(world);
        },
    }
    #[cfg(test)]
    record_reset_step(world, RoutingResetStep::PendingSequenceCancelled);
    world.init_resource::<KeymapRuntime>();
    world.init_resource::<CustomInputs>();

    let pressed = world
        .get_resource::<ButtonInput<KeyCode>>()
        .map(|keys| keys.get_pressed().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    release_all_physical_sources(world);
    #[cfg(test)]
    record_reset_step(world, RoutingResetStep::PhysicalSourcesReleased);
    record_routing_state(&mut world.resource_mut::<KeymapRuntime>());
    #[cfg(test)]
    record_reset_step(world, RoutingResetStep::RoutingStateRecorded);
    world
        .resource_mut::<KeymapRuntime>()
        .inhibit(pressed.into_iter());
    #[cfg(test)]
    record_reset_step(world, RoutingResetStep::PressedKeysInhibited);
}

#[cfg(test)]
fn record_reset_step(world: &mut World, step: RoutingResetStep) {
    let Some(mut routing_reset_trace) = world.get_resource_mut::<RoutingResetTrace>() else {
        return;
    };

    routing_reset_trace.0.push(step);
}

/// Resets physical keymap input after a focus or input-suppression transition.
pub(crate) fn reset_physical_input(world: &mut World) {
    reset_routing(world, PendingSequenceCancellation::EffectiveMatcher, |_| {});
}

fn release_all_physical_sources(world: &mut World) {
    loop {
        let physical_source_release_progress = world
            .resource_mut::<KeymapRuntime>()
            .release_one_physical_source();
        match physical_source_release_progress {
            PhysicalSourceReleaseProgress::ReleasedOne(custom_input_transition) => {
                custom_input_transition.write_to(&mut world.resource_mut::<CustomInputs>());
            },
            PhysicalSourceReleaseProgress::Complete => break,
        }
    }
}

pub(super) fn set_event_source(
    keymap_runtime: &mut KeymapRuntime,
    custom_input: CustomInput,
    is_active: bool,
    custom_inputs: &mut CustomInputs,
) {
    keymap_runtime.set_event_source(custom_input, is_active, custom_inputs);
}
