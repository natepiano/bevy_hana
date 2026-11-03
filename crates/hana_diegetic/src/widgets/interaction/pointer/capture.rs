use std::collections::HashMap;

use bevy::ecs::change_detection::MaybeLocation;
use bevy::ecs::world::DeferredWorld;
use bevy::picking::pointer::PointerId;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::Resource;
use bevy::prelude::error;

use crate::constants::DIAGNOSTIC_LOG_TARGET;

/// One occupancy entry: the widget a pointer owns and the checked
/// attempted-press sequence number that accepted it.
#[derive(Clone, Copy)]
pub(in crate::widgets) struct CapturedWidget {
    entity:   Entity,
    sequence: u64,
}

impl CapturedWidget {
    pub(super) const fn entity(self) -> Entity { self.entity }

    pub(in crate::widgets) const fn sequence(self) -> u64 { self.sequence }
}

/// Shared pointer/widget occupancy and raw-action ordering authority.
///
/// One pointer owns at most one widget and one widget is owned by at most one
/// pointer; [`Self::try_capture`] claims both directions and
/// [`Self::release_widget`] frees both. Widget behavior modules keep their own
/// per-press payloads (button ids, terminal outcomes) keyed by widget entity;
/// this resource holds only the cross-widget facts they share: both occupancy
/// directions, the attempted-press observations raw reconciliation orders by,
/// and checked sequence exhaustion.
#[derive(Default, Resource)]
pub(in crate::widgets) struct WidgetCaptures {
    owners:          HashMap<PointerId, CapturedWidget>,
    latest_observed: HashMap<PointerId, (Entity, u64)>,
    /// Count of presses per pointer that reached [`Self::observe_press`] since
    /// the last raw reconciliation. Raw reconciliation compares it against the
    /// count of raw primary presses to tell whether the latest raw press
    /// actually entered the widget capture-order path.
    observed_counts: HashMap<PointerId, u32>,
    sequence:        u64,
}

impl WidgetCaptures {
    /// Records an attempted press and returns its sequence number, or `None`
    /// when the checked sequence is exhausted. An exhausted sequence leaves
    /// every existing occupancy untouched.
    pub(in crate::widgets) fn observe_press(
        &mut self,
        pointer_id: PointerId,
        entity: Entity,
    ) -> Option<u64> {
        let Some(sequence) = self.sequence.checked_add(1) else {
            error!(target: DIAGNOSTIC_LOG_TARGET, "Hana widget press sequence exhausted; ignoring press for {pointer_id:?}");
            return None;
        };
        self.sequence = sequence;
        self.latest_observed.insert(pointer_id, (entity, sequence));
        *self.observed_counts.entry(pointer_id).or_default() += 1;
        Some(sequence)
    }

    /// Whether both occupancy directions are free for this pointer/widget
    /// pair.
    fn can_capture(&self, pointer_id: PointerId, widget: Entity) -> bool {
        !self.owners.contains_key(&pointer_id)
            && !self.owners.values().any(|owner| owner.entity == widget)
    }

    /// Whether `pointer_id` currently owns `widget`.
    pub(in crate::widgets) fn captures(&self, pointer_id: PointerId, widget: Entity) -> bool {
        self.owners
            .get(&pointer_id)
            .is_some_and(|owner| owner.entity == widget)
    }

    /// Returns the widget owned by `pointer_id`, if any.
    pub(in crate::widgets) fn widget(&self, pointer_id: PointerId) -> Option<Entity> {
        self.owner(pointer_id).map(CapturedWidget::entity)
    }

    /// Returns the occupancy entry for `pointer_id`, if any.
    pub(in crate::widgets) fn owner(&self, pointer_id: PointerId) -> Option<CapturedWidget> {
        self.owners.get(&pointer_id).copied()
    }

    /// Returns the pointer owning `widget`, if any.
    pub(in crate::widgets) fn pointer(&self, widget: Entity) -> Option<PointerId> {
        self.owners
            .iter()
            .find_map(|(&pointer_id, owner)| (owner.entity == widget).then_some(pointer_id))
    }

    /// Claims both occupancy directions for the pair and reports whether the
    /// claim was accepted.
    ///
    /// The claim is rejected when the pointer already owns a widget or the
    /// widget is already owned by a pointer; a rejected claim leaves every
    /// existing occupancy unchanged.
    pub(in crate::widgets) fn try_capture(
        &mut self,
        pointer_id: PointerId,
        entity: Entity,
        sequence: u64,
    ) -> bool {
        if !self.can_capture(pointer_id, entity) {
            return false;
        }
        self.owners
            .insert(pointer_id, CapturedWidget { entity, sequence });
        true
    }

    /// Frees both occupancy directions for the pointer owning `widget` and
    /// returns that pointer.
    pub(in crate::widgets) fn release_widget(&mut self, widget: Entity) -> Option<PointerId> {
        let pointer_id = self.pointer(widget)?;
        self.owners.remove(&pointer_id);
        Some(pointer_id)
    }

    /// Removes and returns the attempted-press observations accumulated since
    /// the last raw reconciliation.
    pub(super) fn take_latest_observed(&mut self) -> HashMap<PointerId, (Entity, u64)> {
        std::mem::take(&mut self.latest_observed)
    }

    /// Removes and returns the per-pointer observed-press counts accumulated
    /// since the last raw reconciliation.
    pub(super) fn take_observed_counts(&mut self) -> HashMap<PointerId, u32> {
        std::mem::take(&mut self.observed_counts)
    }

    #[cfg(test)]
    pub(in crate::widgets) fn is_empty(&self) -> bool { self.owners.is_empty() }

    #[cfg(test)]
    pub(in crate::widgets) const fn saturate_sequence(&mut self) { self.sequence = u64::MAX; }
}

/// Triggers `event` immediately inside a component hook's [`DeferredWorld`].
///
/// Widget terminal hooks run in a [`DeferredWorld`], which has no `trigger`.
/// This resolves the event key and drives Bevy's raw trigger path so button
/// and slider terminals fire their [`EntityEvent`]s while the marker is being
/// removed.
pub(in crate::widgets) fn trigger_immediate<'a, E>(world: &mut DeferredWorld<'_>, mut event: E)
where
    E: Event<Trigger<'a>: Default>,
{
    let Some(event_key) = world.event_key::<E>() else {
        return;
    };
    let mut trigger = <E::Trigger<'a> as Default>::default();
    // SAFETY: `event_key` was fetched for `E` from this `DeferredWorld`, and
    // `trigger` is the `Event::Trigger` associated with `E`.
    unsafe {
        world.trigger_raw(event_key, &mut event, &mut trigger, MaybeLocation::caller());
    }
}

#[cfg(test)]
mod tests {
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::Entity;
    use bevy::prelude::World;

    use super::WidgetCaptures;

    const FIRST_POINTER: PointerId = PointerId::Touch(1);
    const SECOND_POINTER: PointerId = PointerId::Touch(2);

    fn widgets() -> (Entity, Entity) {
        let mut world = World::new();
        (world.spawn_empty().id(), world.spawn_empty().id())
    }

    #[test]
    fn one_pointer_owns_one_widget_and_blocks_both_directions() {
        let (first_widget, second_widget) = widgets();
        let mut captures = WidgetCaptures::default();
        let sequence = captures.observe_press(FIRST_POINTER, first_widget);
        assert_eq!(sequence, Some(1));
        assert!(captures.try_capture(FIRST_POINTER, first_widget, 1));

        assert!(captures.captures(FIRST_POINTER, first_widget));
        assert_eq!(captures.widget(FIRST_POINTER), Some(first_widget));
        assert_eq!(captures.pointer(first_widget), Some(FIRST_POINTER));
        assert!(!captures.can_capture(FIRST_POINTER, second_widget));
        assert!(!captures.can_capture(SECOND_POINTER, first_widget));
        assert!(captures.can_capture(SECOND_POINTER, second_widget));
    }

    #[test]
    fn occupied_pointer_cannot_claim_another_widget() {
        let (first_widget, second_widget) = widgets();
        let mut captures = WidgetCaptures::default();
        assert!(captures.try_capture(FIRST_POINTER, first_widget, 1));

        assert!(!captures.try_capture(FIRST_POINTER, second_widget, 2));
        assert!(captures.captures(FIRST_POINTER, first_widget));
        assert_eq!(captures.pointer(first_widget), Some(FIRST_POINTER));
        assert_eq!(captures.pointer(second_widget), None);
    }

    #[test]
    fn occupied_widget_cannot_be_claimed_by_another_pointer() {
        let (first_widget, _) = widgets();
        let mut captures = WidgetCaptures::default();
        assert!(captures.try_capture(FIRST_POINTER, first_widget, 1));

        assert!(!captures.try_capture(SECOND_POINTER, first_widget, 2));
        assert!(captures.captures(FIRST_POINTER, first_widget));
        assert_eq!(captures.pointer(first_widget), Some(FIRST_POINTER));
        assert_eq!(captures.widget(SECOND_POINTER), None);
    }

    #[test]
    fn release_widget_frees_both_directions() {
        let (first_widget, second_widget) = widgets();
        let mut captures = WidgetCaptures::default();
        assert!(captures.try_capture(FIRST_POINTER, first_widget, 1));

        assert_eq!(captures.release_widget(first_widget), Some(FIRST_POINTER));
        assert!(captures.is_empty());
        assert!(captures.can_capture(FIRST_POINTER, second_widget));
        assert!(captures.can_capture(SECOND_POINTER, first_widget));
        assert_eq!(captures.release_widget(first_widget), None);
    }

    #[test]
    fn observe_press_orders_attempts_and_drains_on_take() {
        let (first_widget, second_widget) = widgets();
        let mut captures = WidgetCaptures::default();
        let first = captures.observe_press(FIRST_POINTER, first_widget);
        let second = captures.observe_press(FIRST_POINTER, second_widget);
        assert!(first < second);

        let observed = captures.take_latest_observed();
        assert_eq!(
            observed.get(&FIRST_POINTER).copied(),
            second.map(|sequence| (second_widget, sequence)),
        );
        assert!(captures.take_latest_observed().is_empty());
    }

    #[test]
    fn exhausted_sequence_rejects_observation_and_preserves_the_owner() {
        let (first_widget, second_widget) = widgets();
        let mut captures = WidgetCaptures::default();
        assert!(captures.try_capture(FIRST_POINTER, first_widget, 1));
        captures.saturate_sequence();

        assert_eq!(captures.observe_press(SECOND_POINTER, second_widget), None);
        assert!(captures.take_latest_observed().is_empty());
        assert!(captures.captures(FIRST_POINTER, first_widget));
        assert_eq!(captures.pointer(first_widget), Some(FIRST_POINTER));
    }
}
