use std::collections::HashMap;
use std::collections::HashSet;

use bevy::ecs::change_detection::MaybeLocation;
use bevy::ecs::world::DeferredWorld;
use bevy::picking::PickingSettings;
use bevy::picking::events::PointerState;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerAction;
use bevy::picking::pointer::PointerId;
use bevy::picking::pointer::PointerInput;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::Has;
use bevy::prelude::MessageReader;
use bevy::prelude::PointerButton;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::error;
use bevy::prelude::warn;

use super::PanelWidget;
use super::WidgetDisabled;
use super::WidgetKind;
use super::WidgetOf;
use super::button;
use super::button::ButtonCancelCause;
use super::button::ButtonCaptures;
use super::slider;
use super::slider::SliderCancelCause;
use super::slider::SliderCaptures;
use super::slider::SliderProjection;
use super::slider::SliderWidget;

/// One occupancy entry: the widget a pointer owns and the checked
/// attempted-press sequence number that accepted it.
#[derive(Clone, Copy)]
pub(super) struct CapturedWidget {
    entity:   Entity,
    sequence: u64,
}

impl CapturedWidget {
    const fn entity(self) -> Entity { self.entity }

    pub(super) const fn sequence(self) -> u64 { self.sequence }
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
pub(super) struct WidgetCaptures {
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
    pub(super) fn observe_press(&mut self, pointer_id: PointerId, entity: Entity) -> Option<u64> {
        let Some(sequence) = self.sequence.checked_add(1) else {
            error!("Hana widget press sequence exhausted; ignoring press for {pointer_id:?}");
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
    pub(super) fn captures(&self, pointer_id: PointerId, widget: Entity) -> bool {
        self.owners
            .get(&pointer_id)
            .is_some_and(|owner| owner.entity == widget)
    }

    /// Returns the widget owned by `pointer_id`, if any.
    pub(super) fn widget(&self, pointer_id: PointerId) -> Option<Entity> {
        self.owner(pointer_id).map(CapturedWidget::entity)
    }

    /// Returns the occupancy entry for `pointer_id`, if any.
    pub(super) fn owner(&self, pointer_id: PointerId) -> Option<CapturedWidget> {
        self.owners.get(&pointer_id).copied()
    }

    /// Returns the pointer owning `widget`, if any.
    pub(super) fn pointer(&self, widget: Entity) -> Option<PointerId> {
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
    pub(super) fn try_capture(
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
    pub(super) fn release_widget(&mut self, widget: Entity) -> Option<PointerId> {
        let pointer_id = self.pointer(widget)?;
        self.owners.remove(&pointer_id);
        Some(pointer_id)
    }

    /// Removes and returns the attempted-press observations accumulated since
    /// the last raw reconciliation.
    fn take_latest_observed(&mut self) -> HashMap<PointerId, (Entity, u64)> {
        std::mem::take(&mut self.latest_observed)
    }

    /// Removes and returns the per-pointer observed-press counts accumulated
    /// since the last raw reconciliation.
    fn take_observed_counts(&mut self) -> HashMap<PointerId, u32> {
        std::mem::take(&mut self.observed_counts)
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool { self.owners.is_empty() }

    #[cfg(test)]
    pub(super) const fn saturate_sequence(&mut self) { self.sequence = u64::MAX; }
}

/// Triggers `event` immediately inside a component hook's [`DeferredWorld`].
///
/// Widget terminal hooks run in a [`DeferredWorld`], which has no `trigger`.
/// This resolves the event key and drives Bevy's raw trigger path so button
/// and slider terminals fire their [`EntityEvent`]s while the marker is being
/// removed.
pub(super) fn trigger_immediate<'a, E>(world: &mut DeferredWorld<'_>, mut event: E)
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

/// Which kind of terminal raw action ended a captured pointer's interaction.
#[derive(Clone, Copy)]
enum RawTerminalKind {
    Release,
    Cancel,
}

/// Reconciles captures left unresolved by Bevy's targeted pointer events for
/// every captured widget, regardless of kind.
///
/// Bevy targets [`Pointer<Click>`], [`Pointer<Release>`], and
/// [`Pointer<DragEnd>`] from its previous hover or dragging state; those
/// observers remain authoritative and free the capture before this system
/// runs. This system reads the raw primary [`PointerInput`] stream in order,
/// removes surviving terminal captures — dispatching to the button or slider
/// terminal helper for the captured widget's current [`WidgetKind`] — and then
/// establishes only final presses that occurred after the terminal action
/// which freed their pointer and widget. Raw [`PointerAction::Cancel`] and
/// pointer removal remain separate terminal fallbacks; Bevy documents `Cancel`
/// as terminal, so later raw actions for that pointer are warned about and
/// ignored. A private sequence distinguishes an accepted press from a later
/// press initially rejected while its pointer or widget was still captured.
/// `WidgetsPlugin` runs this system only when [`PointerInput`] messages,
/// [`PointerState`], [`HoverMap`], and [`bevy::picking::PickingSettings`] are
/// all installed.
pub(super) fn reconcile_pointer_input(
    mut inputs: MessageReader<PointerInput>,
    pointer_state: Res<PointerState>,
    hover_map: Res<HoverMap>,
    picking_settings: Res<PickingSettings>,
    widgets: Query<(&WidgetKind, Has<WidgetDisabled>), (With<PanelWidget>, With<WidgetOf>)>,
    sliders: Query<SliderWidget>,
    slider_projection: SliderProjection,
    mut captures: ResMut<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    let (primary_presses, raw_press_counts, terminals) = read_primary_actions(&mut inputs);
    let latest_observed = captures.take_latest_observed();
    let observed_counts = captures.take_observed_counts();
    let mut removed_at = HashMap::new();
    for (order, pointer_id, terminal, position) in terminals {
        let Some(owner) = captures.owner(pointer_id) else {
            continue;
        };
        if removed_at.contains_key(&pointer_id) {
            continue;
        }
        // Defer a release only when a later press re-captured this pointer
        // within the same batch — the owner is the latest observed press and
        // its raw press sits after this release. A disabled or projection-
        // failing slider press is deliberately not observed, so it never enters
        // the widget capture-order path: when the observed-press count differs
        // from the raw primary-press count the latest raw press was one of
        // those, and it must not suppress the accepted interaction's real
        // release.
        if matches!(terminal, RawTerminalKind::Release)
            && should_defer_release(
                pointer_id,
                owner,
                order,
                &primary_presses,
                &latest_observed,
                &observed_counts,
                &raw_press_counts,
            )
        {
            continue;
        }

        let entity = owner.entity();
        let final_is_later =
            later_press_is_final(pointer_id, owner, &latest_observed, &pointer_state);
        // `removed_at` records the pointer as freed only when the terminal
        // actually released shared occupancy, so a later press cannot treat an
        // unfreed pointer or widget as available.
        let kind = widgets.get(entity).map(|(kind, _)| *kind).ok();
        let freed = match terminal {
            RawTerminalKind::Cancel => match kind {
                Some(WidgetKind::Button) => button::cancel_button_press(
                    entity,
                    ButtonCancelCause::PointerCanceled,
                    &mut button_captures,
                    &mut commands,
                ),
                Some(WidgetKind::EditableField) => false,
                Some(WidgetKind::Slider) => slider::cancel_slider_drag(
                    entity,
                    SliderCancelCause::PointerCanceled,
                    &mut slider_captures,
                    &mut commands,
                ),
                None => cancel_lost_widget(
                    entity,
                    ButtonCancelCause::PointerCanceled,
                    SliderCancelCause::PointerCanceled,
                    &mut button_captures,
                    &mut slider_captures,
                    &mut commands,
                ),
            },
            RawTerminalKind::Release => match kind {
                Some(WidgetKind::Button) => button::apply_raw_release(
                    entity,
                    pointer_id,
                    final_is_later,
                    &hover_map,
                    &pointer_state,
                    &picking_settings,
                    &mut button_captures,
                    &mut commands,
                ),
                Some(WidgetKind::EditableField) => false,
                Some(WidgetKind::Slider) => slider::resolve_release(
                    entity,
                    position,
                    &sliders,
                    &slider_projection,
                    &mut slider_captures,
                    &mut commands,
                ),
                None => cancel_lost_widget(
                    entity,
                    ButtonCancelCause::CaptureLost,
                    SliderCancelCause::CaptureLost,
                    &mut button_captures,
                    &mut slider_captures,
                    &mut commands,
                ),
            },
        };
        if freed {
            removed_at.insert(pointer_id, order);
        }
    }

    queue_final_presses(
        latest_observed,
        &primary_presses,
        &removed_at,
        &pointer_state,
        &widgets,
        &captures,
        &mut commands,
    );

    if slider_captures.has_pending() {
        commands.queue(|world: &mut World| {
            world.resource_mut::<SliderCaptures>().clear_pending();
        });
    }
}

/// Finalizes a captured widget whose current [`WidgetKind`] is unavailable
/// because its identity components (`PanelWidget` / `WidgetOf`) were removed
/// while the typed capture payload survived.
///
/// Without the kind the terminal loop cannot pick a typed helper, so this
/// cancels through whichever payload still holds the entity — at most one does,
/// and each call is a no-op otherwise — so the marker's remove hook frees shared
/// occupancy exactly once. The concrete button and slider causes are passed in
/// so a lost-widget cancel keeps the terminal action's real cause
/// (`PointerCanceled` for a raw cancel, `CaptureLost` for a raw release) without
/// a public generic terminal API. It reports whether a terminal was recorded so
/// the caller marks the pointer freed only when shared occupancy was actually
/// released.
fn cancel_lost_widget(
    entity: Entity,
    button_cause: ButtonCancelCause,
    slider_cause: SliderCancelCause,
    button_captures: &mut ButtonCaptures,
    slider_captures: &mut SliderCaptures,
    commands: &mut Commands<'_, '_>,
) -> bool {
    let slider = slider::cancel_slider_drag(entity, slider_cause, slider_captures, commands);
    let button = button::cancel_button_press(entity, button_cause, button_captures, commands);
    slider || button
}

fn should_defer_release(
    pointer_id: PointerId,
    owner: CapturedWidget,
    release_order: usize,
    primary_presses: &HashMap<PointerId, usize>,
    latest_observed: &HashMap<PointerId, (Entity, u64)>,
    observed_counts: &HashMap<PointerId, u32>,
    raw_press_counts: &HashMap<PointerId, u32>,
) -> bool {
    let accepted_is_latest = latest_observed
        .get(&pointer_id)
        .is_some_and(|(_, sequence)| *sequence == owner.sequence());
    let latest_press_observed = observed_counts.get(&pointer_id).copied().unwrap_or(0)
        == raw_press_counts.get(&pointer_id).copied().unwrap_or(0);
    accepted_is_latest
        && latest_press_observed
        && primary_presses
            .get(&pointer_id)
            .is_some_and(|press_order| *press_order > release_order)
}

fn later_press_is_final(
    pointer_id: PointerId,
    owner: CapturedWidget,
    latest_observed: &HashMap<PointerId, (Entity, u64)>,
    pointer_state: &PointerState,
) -> bool {
    latest_observed
        .get(&pointer_id)
        .is_some_and(|(latest_entity, sequence)| {
            *sequence > owner.sequence()
                && pointer_state
                    .get(pointer_id, PointerButton::Primary)
                    .is_some_and(|state| state.pressing.contains_key(latest_entity))
        })
}

fn read_primary_actions(
    inputs: &mut MessageReader<'_, '_, PointerInput>,
) -> (
    HashMap<PointerId, usize>,
    HashMap<PointerId, u32>,
    Vec<(usize, PointerId, RawTerminalKind, Vec2)>,
) {
    let mut primary_presses = HashMap::new();
    let mut raw_press_counts = HashMap::new();
    let mut terminals = Vec::new();
    let mut canceled_pointers = HashSet::new();
    for (order, input) in inputs.read().enumerate() {
        if canceled_pointers.contains(&input.pointer_id) {
            warn!(
                "received {:?} after terminal pointer cancel for {:?}",
                input.action, input.pointer_id
            );
            continue;
        }
        match input.action {
            PointerAction::Press(PointerButton::Primary) => {
                primary_presses.insert(input.pointer_id, order);
                *raw_press_counts.entry(input.pointer_id).or_default() += 1;
            },
            PointerAction::Release(PointerButton::Primary) => {
                terminals.push((
                    order,
                    input.pointer_id,
                    RawTerminalKind::Release,
                    input.location.position,
                ));
            },
            PointerAction::Cancel => {
                canceled_pointers.insert(input.pointer_id);
                terminals.push((
                    order,
                    input.pointer_id,
                    RawTerminalKind::Cancel,
                    input.location.position,
                ));
            },
            PointerAction::Press(_)
            | PointerAction::Release(_)
            | PointerAction::Move { .. }
            | PointerAction::Scroll { .. } => {},
        }
    }
    (primary_presses, raw_press_counts, terminals)
}

fn queue_final_presses(
    latest_observed: HashMap<PointerId, (Entity, u64)>,
    primary_presses: &HashMap<PointerId, usize>,
    removed_at: &HashMap<PointerId, usize>,
    pointer_state: &PointerState,
    widgets: &Query<
        '_,
        '_,
        (&WidgetKind, Has<WidgetDisabled>),
        (With<PanelWidget>, With<WidgetOf>),
    >,
    captures: &WidgetCaptures,
    commands: &mut Commands<'_, '_>,
) {
    let mut final_presses = latest_observed
        .into_iter()
        .filter_map(|(pointer_id, (entity, sequence))| {
            let order = primary_presses.get(&pointer_id).copied()?;
            pointer_state
                .get(pointer_id, PointerButton::Primary)
                .is_some_and(|state| state.pressing.contains_key(&entity))
                .then_some((order, pointer_id, entity, sequence))
        })
        .collect::<Vec<_>>();
    final_presses.sort_unstable_by_key(|(order, ..)| *order);

    for (order, pointer_id, entity, sequence) in final_presses {
        if captures
            .owner(pointer_id)
            .is_some_and(|owner| owner.sequence() == sequence)
        {
            continue;
        }
        let pointer_is_freed = captures.owner(pointer_id).is_none_or(|_| {
            removed_at
                .get(&pointer_id)
                .is_some_and(|removed_order| *removed_order < order)
        });
        let widget_is_freed = captures.pointer(entity).is_none_or(|captured_pointer| {
            removed_at
                .get(&captured_pointer)
                .is_some_and(|removed_order| *removed_order < order)
        });
        if pointer_is_freed
            && widget_is_freed
            && let Ok((kind, disabled)) = widgets.get(entity)
            && !disabled
        {
            match kind {
                WidgetKind::Button => commands.queue(move |world: &mut World| {
                    button::capture_reconciled_press(world, entity, pointer_id, sequence);
                }),
                WidgetKind::EditableField => {},
                WidgetKind::Slider => commands.queue(move |world: &mut World| {
                    slider::capture_reconciled_press(world, entity, pointer_id, sequence);
                }),
            }
        }
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
