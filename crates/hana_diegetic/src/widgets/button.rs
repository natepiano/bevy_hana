use std::collections::HashMap;
use std::fmt;
use std::fmt::Formatter;
use std::sync::Arc;

use bevy::ecs::error::BevyError;
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::system::SystemHandle;
use bevy::ecs::system::SystemHandleTemplate;
use bevy::ecs::system::SystemId;
use bevy::ecs::template::SceneEntityReferences;
use bevy::ecs::template::Template;
use bevy::ecs::template::TemplateContext;
use bevy::ecs::world::DeferredWorld;
use bevy::ecs::world::EntityWorldMut;
use bevy::picking::PickingSettings;
use bevy::picking::events::PointerState;
use bevy::picking::hover::HoverMap;
use bevy::picking::hover::PickingInteraction;
use bevy::picking::pointer::PointerId;
use bevy::prelude::Add;
use bevy::prelude::Cancel;
use bevy::prelude::Changed;
use bevy::prelude::Click;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Despawn;
use bevy::prelude::DragEnd;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Has;
use bevy::prelude::In;
use bevy::prelude::IntoSystem;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::Pointer;
use bevy::prelude::PointerButton;
use bevy::prelude::Press;
use bevy::prelude::Query;
use bevy::prelude::Release;
use bevy::prelude::Remove;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::warn;

use super::PanelWidget;
use super::SemanticWidgetIntent;
use super::WidgetDisabled;
use super::WidgetFocusVisible;
use super::WidgetKind;
use super::WidgetOf;
use super::WidgetVisualOverrides;
use super::WidgetVisualSlots;
use super::appearance::WidgetState;
use super::capture;
use super::capture::WidgetCaptures;
use super::visual;
use crate::DiegeticPanel;
use crate::PanelElementId;
use crate::cascade::Resolved;
use crate::ime;
use crate::ime::ImeBlurIntent;
use crate::ime::ImeEditorState;

/// Cloneable authored click callback, compared by [`Arc`] identity so the
/// enclosing authored `Button` (and its `WidgetSpec`) stays `PartialEq`.
#[derive(Clone)]
pub(super) struct ButtonCallback(Arc<SystemHandleTemplate<In<ButtonClicked>, ()>>);

impl ButtonCallback {
    fn new<M>(system: impl IntoSystem<In<ButtonClicked>, (), M>) -> Self {
        Self(Arc::new(SystemHandleTemplate::value(system)))
    }

    /// Returns the tracked [`SystemHandle`] for this callback.
    ///
    /// The first build registers the callback as a tracked system;
    /// later builds of the same callback return the cached handle without
    /// registering again.
    pub(super) fn build_handle(
        &self,
        widget: &mut EntityWorldMut<'_>,
    ) -> Result<SystemHandle<In<ButtonClicked>, ()>, BevyError> {
        let mut entity_references = SceneEntityReferences::default();
        self.0
            .as_ref()
            .build_template(&mut TemplateContext::new(widget, &mut entity_references))
    }
}

impl PartialEq for ButtonCallback {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}

impl Eq for ButtonCallback {}

impl fmt::Debug for ButtonCallback {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ButtonCallback")
            .finish_non_exhaustive()
    }
}

/// Authored behavior for a panel button.
///
/// Declare one inline with [`El::button`](crate::El::button), or build one here
/// and attach it with [`El::widget`](crate::El::widget). A button carries
/// behavior only: its resting look stays on the element's ordinary
/// [`El::background`](crate::El::background),
/// [`El::border`](crate::El::border), and
/// [`El::material`](crate::El::material) declarations, and its per-state look
/// on the element's state builders.
#[must_use]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Button {
    callback: Option<ButtonCallback>,
}

impl Button {
    /// Creates a button declaration with default behavior.
    pub const fn new() -> Self { Self { callback: None } }

    /// Runs `system` with each completed [`ButtonClicked`] for this button.
    ///
    /// The callback is an ordinary Bevy system taking `In<ButtonClicked>`
    /// plus any other system parameters. Reify registers it once as a tracked
    /// system; authoring the same widget id with a different
    /// callback releases the prior tracked handle, and dropping the final
    /// handle unregisters the system. This sugar complements direct
    /// [`ButtonClicked`] observation — a global or entity-scoped observer
    /// reads the widget entity from the event target.
    pub fn on_click<M>(mut self, system: impl IntoSystem<In<ButtonClicked>, (), M>) -> Self {
        self.set_callback(system);
        self
    }

    pub(crate) fn set_callback<M>(&mut self, system: impl IntoSystem<In<ButtonClicked>, (), M>) {
        self.callback = Some(ButtonCallback::new(system));
    }

    pub(super) const fn callback(&self) -> Option<&ButtonCallback> { self.callback.as_ref() }
}

/// Maps each changed button's live state onto its retained visual overrides.
///
/// Runs after `WidgetSystems::FocusCommandsApplied`, so application and
/// keyboard-traversal indicator commands, as well as pointer-driven indicator
/// removal, are visible in the same frame. Each `Changed` query and
/// [`RemovedComponents`] stream is consumed here, so a quiet frame never walks
/// the live buttons.
///
/// Hover reads the all-pointer [`PickingInteraction`] aggregate and pressed
/// reads the private [`ButtonPress`] marker; [`WidgetCaptures`] and
/// [`ButtonCaptures`] stay lifecycle authority and are never read for
/// presentation. Writes go through [`visual::write_widget_overrides`], which
/// compares immutably first, so an unchanged state never marks
/// [`WidgetVisualOverrides`] changed.
pub(super) fn present_button_state(
    changed: Query<
        (Entity, &WidgetKind),
        (
            With<WidgetOf>,
            Or<(
                Changed<Resolved<super::WidgetHoveredAppearance>>,
                Changed<Resolved<super::WidgetPressedAppearance>>,
                Changed<Resolved<super::WidgetFocusedAppearance>>,
                Changed<Resolved<super::WidgetDisabledAppearance>>,
                Changed<WidgetVisualSlots>,
                Changed<PickingInteraction>,
                Changed<WidgetFocusVisible>,
                Changed<WidgetDisabled>,
                Changed<ButtonPress>,
            )>,
        ),
    >,
    buttons: Query<
        (
            Entity,
            &Resolved<super::WidgetHoveredAppearance>,
            &Resolved<super::WidgetPressedAppearance>,
            &Resolved<super::WidgetFocusedAppearance>,
            &Resolved<super::WidgetDisabledAppearance>,
            &WidgetOf,
            &WidgetVisualSlots,
            Option<&PickingInteraction>,
            Has<WidgetDisabled>,
            Has<WidgetFocusVisible>,
            Has<ButtonPress>,
        ),
        With<WidgetOf>,
    >,
    kinds: Query<&WidgetKind, With<WidgetOf>>,
    panels: Query<&DiegeticPanel>,
    mut removed_interactions: RemovedComponents<PickingInteraction>,
    mut removed_focus: RemovedComponents<WidgetFocusVisible>,
    mut removed_disabled: RemovedComponents<WidgetDisabled>,
    mut removed_presses: RemovedComponents<ButtonPress>,
    mut overrides: Query<&mut WidgetVisualOverrides>,
    mut commands: Commands,
) {
    let dirty = visual::dirty_widgets(
        changed.iter().map(|(entity, kind)| (entity, *kind)),
        removed_interactions
            .read()
            .chain(removed_focus.read())
            .chain(removed_disabled.read())
            .chain(removed_presses.read()),
        &kinds,
        WidgetKind::Button,
    );
    for entity in dirty {
        let Ok((
            entity,
            hovered,
            pressed_appearance,
            focused_appearance,
            disabled_appearance,
            widget_of,
            slots,
            interaction,
            disabled,
            focused,
            pressed,
        )) = buttons.get(entity)
        else {
            continue;
        };
        let active = [
            focused.then_some(WidgetState::Focused),
            matches!(
                interaction,
                Some(PickingInteraction::Hovered | PickingInteraction::Pressed)
            )
            .then_some(WidgetState::Hovered),
            pressed.then_some(WidgetState::Pressed),
            disabled.then_some(WidgetState::Disabled),
        ];
        let panel = panels.get(widget_of.panel()).ok();
        let mut desired = WidgetVisualOverrides::default();
        visual::resolve_part_overrides(
            &mut desired,
            slots,
            hovered,
            pressed_appearance,
            focused_appearance,
            disabled_appearance,
            &active,
            panel,
        );
        visual::write_widget_overrides(entity, desired, &mut overrides, &mut commands);
    }
}

/// Reports the beginning of a pointer-driven button press.
#[derive(Clone, Debug, EntityEvent)]
pub struct ButtonPressed {
    /// Live button entity receiving the press.
    #[event_target]
    pub entity:     Entity,
    /// Panel-local button id.
    pub id:         PanelElementId,
    /// Pointer that began the press.
    pub pointer_id: PointerId,
}

/// Reports the valid release of a pointer-driven button press.
#[derive(Clone, Debug, EntityEvent)]
pub struct ButtonReleased {
    /// Live button entity receiving the release.
    #[event_target]
    pub entity:     Entity,
    /// Panel-local button id.
    pub id:         PanelElementId,
    /// Pointer that completed the press.
    pub pointer_id: PointerId,
}

/// Reports pointer or semantic activation of a button.
#[derive(Clone, Debug, EntityEvent)]
pub struct ButtonClicked {
    /// Live button entity receiving activation.
    #[event_target]
    pub entity:     Entity,
    /// Panel-local button id.
    pub id:         PanelElementId,
    /// Activating pointer, or `None` for semantic activation.
    pub pointer_id: Option<PointerId>,
}

/// Reports a pointer-driven button press that did not reach release.
#[derive(Clone, Debug, EntityEvent)]
pub struct ButtonCanceled {
    /// Live button entity whose press was canceled.
    #[event_target]
    pub entity:     Entity,
    /// Panel-local button id.
    pub id:         PanelElementId,
    /// Pointer whose press was canceled.
    pub pointer_id: PointerId,
    /// Reason the press ended without release.
    pub cause:      ButtonCancelCause,
}

/// Reason a pointer-driven button press was canceled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonCancelCause {
    /// The pointer input stream reported cancellation.
    PointerCanceled,
    /// The captured pointer entity or its [`PointerId`] was removed.
    PointerRemoved,
    /// The captured pointer released without a valid button release.
    CaptureLost,
    /// The button became disabled while pressed.
    Disabled,
    /// The widget was removed or its owning panel role ended.
    WidgetRemoved,
    /// The same widget id changed to another widget kind.
    WidgetKindChanged,
    /// Semantic input explicitly canceled the press.
    Explicit,
}

#[derive(Component)]
#[component(
    on_remove = emit_button_terminal,
    on_despawn = emit_button_terminal
)]
pub(crate) struct ButtonPress;

/// Tracked handle to a widget's registered click-callback system.
///
/// Reify installs and replaces this component; dropping it releases the
/// widget's strong handle, and Bevy unregisters the system once the final
/// handle is gone.
#[derive(Component)]
pub(super) struct ButtonCallbackHandle(SystemHandle<In<ButtonClicked>, ()>);

impl ButtonCallbackHandle {
    pub(super) const fn new(handle: SystemHandle<In<ButtonClicked>, ()>) -> Self { Self(handle) }

    fn system_id(&self) -> SystemId<In<ButtonClicked>> { SystemId::from(&self.0) }

    #[cfg(test)]
    pub(super) fn system_entity(&self) -> Entity { self.0.entity() }
}

struct PendingButtonPress {
    id:       PanelElementId,
    terminal: ButtonTerminal,
}

impl PendingButtonPress {
    const fn new(id: PanelElementId) -> Self {
        Self {
            id,
            terminal: ButtonTerminal::Pending,
        }
    }

    fn release(&mut self, outcome: ButtonReleaseOutcome) -> bool {
        match self.terminal {
            ButtonTerminal::Pending => {
                self.terminal = ButtonTerminal::Release(outcome);
                true
            },
            ButtonTerminal::Release(ButtonReleaseOutcome::WithoutClick)
                if outcome == ButtonReleaseOutcome::Clicked =>
            {
                self.terminal = ButtonTerminal::Release(outcome);
                false
            },
            ButtonTerminal::Release(_) | ButtonTerminal::Cancel(_) => false,
        }
    }

    const fn cancel(&mut self, cause: ButtonCancelCause) -> bool {
        match self.terminal {
            ButtonTerminal::Pending => {
                self.terminal = ButtonTerminal::Cancel(cause);
                true
            },
            ButtonTerminal::Release(_) | ButtonTerminal::Cancel(_) => false,
        }
    }
}

#[derive(Clone, Copy)]
enum ButtonTerminal {
    Pending,
    Release(ButtonReleaseOutcome),
    Cancel(ButtonCancelCause),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ButtonReleaseOutcome {
    WithoutClick,
    Clicked,
}

/// Pending button-press payloads keyed by the captured widget entity.
///
/// Pointer/widget occupancy and raw-action ordering live in the shared
/// [`WidgetCaptures`]; this resource holds only the button facts event
/// emission needs — the panel-local id and the terminal outcome. Entries are
/// inserted and removed together with their [`WidgetCaptures`] occupancy, so
/// membership here always has a live owning pointer.
#[derive(Default, Resource)]
pub(crate) struct ButtonCaptures {
    presses: HashMap<Entity, PendingButtonPress>,
}

impl ButtonCaptures {
    fn begin_press(&mut self, entity: Entity, id: PanelElementId) {
        self.presses.insert(entity, PendingButtonPress::new(id));
    }

    fn press_mut(&mut self, entity: Entity) -> Option<&mut PendingButtonPress> {
        self.presses.get_mut(&entity)
    }

    fn cancel(&mut self, entity: Entity, cause: ButtonCancelCause) -> bool {
        self.presses
            .get_mut(&entity)
            .is_some_and(|press| press.cancel(cause))
    }

    fn take(&mut self, entity: Entity) -> Option<PendingButtonPress> {
        self.presses.remove(&entity)
    }
}

pub(super) fn press_from_pointer(
    mut press: On<Pointer<Press>>,
    widgets: Query<
        (
            &PanelWidget,
            &WidgetKind,
            Has<WidgetDisabled>,
            Has<ButtonPress>,
        ),
        With<WidgetOf>,
    >,
    mut captures: ResMut<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    if press.button != PointerButton::Primary {
        return;
    }
    let entity = press.event_target();
    let Ok((widget, kind, disabled, pressed)) = widgets.get(entity) else {
        return;
    };
    if *kind != WidgetKind::Button {
        return;
    }
    press.propagate(false);
    let Some(sequence) = captures.observe_press(press.pointer_id, entity) else {
        return;
    };
    if disabled || pressed || !captures.try_capture(press.pointer_id, entity, sequence) {
        return;
    }

    button_captures.begin_press(entity, widget.id().clone());
    commands.entity(entity).insert(ButtonPress);
    commands.trigger(ButtonPressed {
        entity,
        id: widget.id().clone(),
        pointer_id: press.pointer_id,
    });
}

pub(super) fn click_from_pointer(
    mut click: On<Pointer<Click>>,
    mut widgets: Query<(&PanelWidget, &WidgetKind, &WidgetOf, Has<WidgetDisabled>)>,
    captures: Res<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    editor_state: Option<Res<ImeEditorState>>,
    mut blur_intent: Option<ResMut<ImeBlurIntent>>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    let entity = click.event_target();
    let Ok((_, kind, widget_of, disabled)) = widgets.get_mut(entity) else {
        return;
    };
    if *kind != WidgetKind::Button {
        return;
    }

    if let (Some(editor_state), Some(blur_intent)) =
        (editor_state.as_deref(), blur_intent.as_deref_mut())
    {
        ime::classify_widget_click(widget_of.panel(), editor_state, blur_intent);
    }
    click.propagate(false);
    if disabled || !captures.captures(click.pointer_id, entity) {
        return;
    }
    if let Some(button_press) = button_captures.press_mut(entity) {
        button_press.release(ButtonReleaseOutcome::Clicked);
    }
}

pub(super) fn release_from_pointer(
    mut release: On<Pointer<Release>>,
    widgets: Query<&WidgetKind>,
    captures: Res<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    if release.button != PointerButton::Primary {
        return;
    }
    let entity = release.event_target();
    let Ok(kind) = widgets.get(entity) else {
        return;
    };
    if *kind != WidgetKind::Button {
        return;
    }
    release.propagate(false);
    if !captures.captures(release.pointer_id, entity) {
        return;
    }
    let Some(button_press) = button_captures.press_mut(entity) else {
        return;
    };
    button_press.release(ButtonReleaseOutcome::WithoutClick);
    commands.entity(entity).remove::<ButtonPress>();
}

pub(super) fn cancel_from_pointer(
    mut cancel: On<Pointer<Cancel>>,
    captures: Res<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    let entity = cancel.event_target();
    if captures.captures(cancel.pointer_id, entity) {
        cancel.propagate(false);
        cancel_button_press(
            entity,
            ButtonCancelCause::PointerCanceled,
            &mut button_captures,
            &mut commands,
        );
    }
}

pub(super) fn cancel_from_drag_end(
    mut drag_end: On<Pointer<DragEnd>>,
    captures: Res<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    let entity = drag_end.event_target();
    if drag_end.button == PointerButton::Primary && captures.captures(drag_end.pointer_id, entity) {
        drag_end.propagate(false);
        cancel_button_press(
            entity,
            ButtonCancelCause::CaptureLost,
            &mut button_captures,
            &mut commands,
        );
    }
}

/// Resolves a raw primary release for a captured button the shared dispatcher
/// left unresolved.
///
/// When hover processing is enabled and the pointer is still over the captured
/// button, this completes the press with a click; otherwise it cancels the
/// capture as lost. `final_is_later` reports that a later accepted press by the
/// same pointer already targets a different widget, so a stale still-pressing
/// entry must not keep this release over the original capture.
///
/// Returns whether a terminal was recorded, so the shared dispatcher marks the
/// pointer freed only when typed terminal processing will release shared
/// occupancy. When the release lands over the capture but the typed press
/// payload is absent, this removes no marker and returns false rather than
/// claiming a freed pointer whose occupancy no hook will release.
pub(super) fn apply_raw_release(
    entity: Entity,
    pointer_id: PointerId,
    final_is_later: bool,
    hover_map: &HoverMap,
    pointer_state: &PointerState,
    picking_settings: &PickingSettings,
    button_captures: &mut ButtonCaptures,
    commands: &mut Commands<'_, '_>,
) -> bool {
    let hover_is_current = picking_settings.is_enabled
        && picking_settings.is_hover_enabled
        && pointer_state
            .get(pointer_id, PointerButton::Primary)
            .is_none_or(|state| !state.pressing.contains_key(&entity) || final_is_later);
    let released_over_capture = hover_is_current
        && hover_map
            .get(&pointer_id)
            .is_some_and(|hovered| hovered.contains_key(&entity));
    if released_over_capture {
        let Some(button_press) = button_captures.press_mut(entity) else {
            return false;
        };
        button_press.release(ButtonReleaseOutcome::Clicked);
        commands.entity(entity).remove::<ButtonPress>();
        true
    } else {
        cancel_button_press(
            entity,
            ButtonCancelCause::CaptureLost,
            button_captures,
            commands,
        )
    }
}

pub(super) fn capture_reconciled_press(
    world: &mut World,
    entity: Entity,
    pointer_id: PointerId,
    sequence: u64,
) {
    let is_button = world
        .get::<WidgetKind>(entity)
        .is_some_and(|kind| *kind == WidgetKind::Button);
    if !is_button
        || world.get::<WidgetOf>(entity).is_none()
        || world.get::<WidgetDisabled>(entity).is_some()
        || world.get::<ButtonPress>(entity).is_some()
    {
        return;
    }
    let Some(id) = world
        .get::<PanelWidget>(entity)
        .map(|widget| widget.id().clone())
    else {
        return;
    };
    if !world
        .resource_mut::<WidgetCaptures>()
        .try_capture(pointer_id, entity, sequence)
    {
        return;
    }

    world
        .resource_mut::<ButtonCaptures>()
        .begin_press(entity, id.clone());
    world.entity_mut(entity).insert(ButtonPress);
    world.trigger(ButtonPressed {
        entity,
        id,
        pointer_id,
    });
}

pub(super) fn cancel_from_pointer_removal(
    removed: On<Remove, PointerId>,
    pointers: Query<&PointerId>,
    captures: Res<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    let Ok(&pointer_id) = pointers.get(removed.entity) else {
        return;
    };
    let Some(widget) = captures.widget(pointer_id) else {
        return;
    };
    cancel_button_press(
        widget,
        ButtonCancelCause::PointerRemoved,
        &mut button_captures,
        &mut commands,
    );
}

pub(super) fn cancel_from_disabled(
    disabled: On<Add, WidgetDisabled>,
    mut captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    cancel_button_press(
        disabled.entity,
        ButtonCancelCause::Disabled,
        &mut captures,
        &mut commands,
    );
}

pub(super) fn cancel_from_widget_removal(
    removed: On<Remove, PanelWidget>,
    mut captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    cancel_button_press(
        removed.entity,
        ButtonCancelCause::WidgetRemoved,
        &mut captures,
        &mut commands,
    );
}

pub(super) fn cancel_before_widget_despawn(
    despawn: On<Despawn, PanelWidget>,
    mut captures: ResMut<ButtonCaptures>,
) {
    captures.cancel(despawn.entity, ButtonCancelCause::WidgetRemoved);
}

pub(super) fn handle_semantic_intent(
    intent: On<SemanticWidgetIntent>,
    widgets: Query<(&PanelWidget, &WidgetKind)>,
    mut captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    let entity = intent.event_target();
    let Ok((widget, kind)) = widgets.get(entity) else {
        return;
    };
    if *kind != WidgetKind::Button {
        return;
    }
    match intent.event() {
        SemanticWidgetIntent::Activate { .. } => {
            commands.trigger(ButtonClicked {
                entity,
                id: widget.id().clone(),
                pointer_id: None,
            });
        },
        SemanticWidgetIntent::Cancel { .. } => {
            cancel_button_press(
                entity,
                ButtonCancelCause::Explicit,
                &mut captures,
                &mut commands,
            );
        },
    }
}

/// Runs the clicked widget's authored callback with the completed event.
///
/// `WidgetsPlugin` installs this single global [`ButtonClicked`] observer;
/// reify never installs a per-widget observer. The observer reads only the
/// target's [`ButtonCallbackHandle`] and forwards the finished event — it
/// never touches [`ButtonPress`] or [`ButtonCaptures`], whose pointer
/// lifecycle and semantic activation are resolved before dispatch.
pub(super) fn dispatch_click_callback(
    click: On<ButtonClicked>,
    handles: Query<&ButtonCallbackHandle>,
    mut commands: Commands,
) {
    let Ok(handle) = handles.get(click.event_target()) else {
        return;
    };
    commands.run_system_with(handle.system_id(), click.event().clone());
}

fn emit_button_terminal(mut world: DeferredWorld, context: HookContext) {
    let entity = context.entity;
    let Some(button_press) = world
        .get_resource_mut::<ButtonCaptures>()
        .and_then(|mut captures| captures.take(entity))
    else {
        return;
    };
    let Some(pointer_id) = world
        .get_resource_mut::<WidgetCaptures>()
        .and_then(|mut captures| captures.release_widget(entity))
    else {
        warn!(
            "button {:?} ({entity}) has a pending press but no capture owner; skipping terminal \
             events",
            button_press.id
        );
        return;
    };
    let (id, terminal) = (button_press.id, button_press.terminal);

    match terminal {
        ButtonTerminal::Release(outcome) => {
            capture::trigger_immediate(
                &mut world,
                ButtonReleased {
                    entity,
                    id: id.clone(),
                    pointer_id,
                },
            );
            if outcome == ButtonReleaseOutcome::Clicked {
                capture::trigger_immediate(
                    &mut world,
                    ButtonClicked {
                        entity,
                        id,
                        pointer_id: Some(pointer_id),
                    },
                );
            }
        },
        ButtonTerminal::Pending | ButtonTerminal::Cancel(_) => {
            let cause = match terminal {
                ButtonTerminal::Cancel(cause) => cause,
                ButtonTerminal::Pending => ButtonCancelCause::CaptureLost,
                ButtonTerminal::Release(_) => return,
            };
            capture::trigger_immediate(
                &mut world,
                ButtonCanceled {
                    entity,
                    id,
                    pointer_id,
                    cause,
                },
            );
        },
    }
}

/// Records a button cancellation and, when newly recorded, removes
/// [`ButtonPress`] so its hook emits the terminal. Returns whether a terminal
/// was recorded, so the shared dispatcher marks the pointer freed only when
/// shared occupancy will be released.
pub(super) fn cancel_button_press(
    entity: Entity,
    cause: ButtonCancelCause,
    captures: &mut ButtonCaptures,
    commands: &mut Commands<'_, '_>,
) -> bool {
    let canceled = captures.cancel(entity, cause);
    if canceled {
        commands.entity(entity).remove::<ButtonPress>();
    }
    canceled
}

pub(crate) fn finalize_panel_buttons(
    panel: Entity,
    button_presses: &Query<'_, '_, (Entity, &WidgetOf), With<ButtonPress>>,
    captures: &mut ButtonCaptures,
    commands: &mut Commands<'_, '_>,
) {
    for (entity, widget_of) in button_presses {
        if widget_of.panel() == panel && captures.cancel(entity, ButtonCancelCause::WidgetRemoved) {
            commands.entity(entity).remove::<ButtonPress>();
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::change_detection::Tick;
    use bevy::ecs::observer::ObservedBy;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::picking::InteractionPlugin;
    use bevy::picking::PickingPlugin;
    use bevy::picking::PickingSettings;
    use bevy::picking::PickingSystems;
    use bevy::picking::backend::HitData;
    use bevy::picking::backend::PointerHits;
    use bevy::picking::events::PointerState;
    use bevy::picking::events::pointer_events;
    use bevy::picking::hover::HoverMap;
    use bevy::picking::hover::PickingInteraction;
    use bevy::picking::hover::PreviousHoverMap;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerAction;
    use bevy::picking::pointer::PointerId;
    use bevy::picking::pointer::PointerInput;
    use bevy::picking::pointer::PointerLocation;
    use bevy::picking::pointer::PointerMap;
    use bevy::picking::pointer::update_pointer_map;
    use bevy::prelude::App;
    use bevy::prelude::Camera;
    use bevy::prelude::Cancel;
    use bevy::prelude::Click;
    use bevy::prelude::Color;
    use bevy::prelude::Commands;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::DragEnd;
    use bevy::prelude::Entity;
    use bevy::prelude::EntityEvent;
    use bevy::prelude::First;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::Handle;
    use bevy::prelude::In;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::Press;
    use bevy::prelude::Query;
    use bevy::prelude::Release;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::StandardMaterial;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::prelude::default;
    use bevy::window::Ime;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowClosed;
    use bevy::window::WindowFocused;
    use bevy::window::WindowRef;
    use hana_valence::AnchorSite;
    use hana_valence::AnchoredHere;
    use hana_valence::AnchoredTo;

    use super::ButtonCancelCause;
    use super::ButtonCanceled;
    use super::ButtonCaptures;
    use super::ButtonClicked;
    use super::ButtonPress;
    use super::ButtonPressed;
    use super::ButtonReleased;
    use super::ButtonTerminal;
    use super::WidgetCaptures;
    use super::cancel_before_widget_despawn;
    use super::cancel_from_disabled;
    use super::cancel_from_drag_end;
    use super::cancel_from_pointer;
    use super::cancel_from_pointer_removal;
    use super::cancel_from_widget_removal;
    use super::click_from_pointer;
    use super::handle_semantic_intent;
    use super::press_from_pointer;
    use super::release_from_pointer;
    use crate::Appearance;
    use crate::Border;
    use crate::Button;
    use crate::CascadeEntityCommandsExt;
    use crate::ClearWidgetFocus;
    use crate::ComputedDiegeticPanel;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeCommitCause;
    use crate::ImeCommitRequested;
    use crate::ImeEditableFieldSpec;
    use crate::ImeOpenSession;
    use crate::ImePlugin;
    use crate::ImeStarted;
    use crate::ImeTarget;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::PanelWidgetWriter;
    use crate::Px;
    use crate::RequestWidgetFocus;
    use crate::Sizing;
    use crate::Slider;
    use crate::WidgetInput;
    use crate::WidgetInputPlugin;
    use crate::WidgetInteractivity;
    use crate::ime;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::PanelWidget;
    use crate::widgets::ScreenWidgetAnchoredHere;
    use crate::widgets::ScreenWidgetAnchoredTo;
    use crate::widgets::SemanticWidgetIntent;
    use crate::widgets::VisualOverrideIndex;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetDisabled;
    use crate::widgets::WidgetFocusVisible;
    use crate::widgets::WidgetKind;
    use crate::widgets::WidgetOf;
    use crate::widgets::WidgetSystems;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::capture;
    use crate::widgets::capture::CapturedWidget;
    use crate::widgets::slider::SliderCaptures;

    const BUTTON_ID: &str = "action";
    const FIELD_ID: &str = "field";

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedButtonEvent {
        Pressed(PointerId),
        Released(PointerId),
        Clicked(Option<PointerId>),
        Canceled(PointerId, ButtonCancelCause),
    }

    #[derive(Default, Resource)]
    struct RecordedButtonEvents(Vec<RecordedButtonEvent>);

    #[derive(Default, Resource)]
    struct TeardownObservation {
        cancellations:  usize,
        relations_seen: usize,
    }

    #[derive(Default, Resource)]
    struct PanelClicks(usize);

    #[derive(Default, Resource)]
    struct CallbackClicks(Vec<(Entity, Option<PointerId>)>);

    #[derive(Default, Resource)]
    struct ScopedClicks(Vec<Entity>);

    #[derive(Default, Resource)]
    struct ImeObservation {
        starts:        usize,
        opens:         usize,
        commit_causes: Vec<ImeCommitCause>,
    }

    fn record_pressed(event: On<ButtonPressed>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Pressed(event.pointer_id));
    }

    fn record_released(event: On<ButtonReleased>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Released(event.pointer_id));
    }

    fn record_clicked(event: On<ButtonClicked>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Clicked(event.pointer_id));
    }

    fn record_canceled(event: On<ButtonCanceled>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Canceled(event.pointer_id, event.cause));
    }

    fn observe_teardown_cancellation(
        event: On<ButtonCanceled>,
        widgets: Query<(&WidgetOf, &AnchoredHere, &ScreenWidgetAnchoredHere), With<PanelWidget>>,
        mut observation: ResMut<TeardownObservation>,
    ) {
        observation.cancellations += 1;
        if widgets.get(event.entity).is_ok() {
            observation.relations_seen += 1;
        }
    }

    fn record_panel_click(_event: On<Pointer<Click>>, mut clicks: ResMut<PanelClicks>) {
        clicks.0 += 1;
    }

    fn record_callback_click(click: In<ButtonClicked>, mut clicks: ResMut<CallbackClicks>) {
        clicks.0.push((click.entity, click.pointer_id));
    }

    fn record_scoped_click(click: On<ButtonClicked>, mut clicks: ResMut<ScopedClicks>) {
        clicks.0.push(click.event_target());
    }

    fn record_ime_started(_event: On<ImeStarted>, mut observation: ResMut<ImeObservation>) {
        observation.starts += 1;
    }

    fn record_ime_open(_event: On<ImeOpenSession>, mut observation: ResMut<ImeObservation>) {
        observation.opens += 1;
    }

    fn record_ime_commit(event: On<ImeCommitRequested>, mut observation: ResMut<ImeObservation>) {
        observation.commit_causes.push(event.cause);
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.init_resource::<WidgetCaptures>()
            .init_resource::<ButtonCaptures>()
            .init_resource::<SliderCaptures>()
            .init_resource::<RecordedButtonEvents>()
            .init_resource::<HoverMap>()
            .init_resource::<PickingSettings>()
            .init_resource::<PointerState>()
            .add_message::<PointerInput>()
            .add_message::<Pointer<Press>>()
            .add_observer(press_from_pointer)
            .add_observer(click_from_pointer)
            .add_observer(release_from_pointer)
            .add_observer(cancel_from_pointer)
            .add_observer(cancel_from_drag_end)
            .add_observer(cancel_from_pointer_removal)
            .add_observer(cancel_from_disabled)
            .add_observer(cancel_from_widget_removal)
            .add_observer(cancel_before_widget_despawn)
            .add_observer(handle_semantic_intent)
            .add_observer(record_pressed)
            .add_observer(record_released)
            .add_observer(record_clicked)
            .add_observer(record_canceled)
            .add_systems(
                PreUpdate,
                capture::reconcile_pointer_input.in_set(PickingSystems::Last),
            );
        app
    }

    fn spawn_button(app: &mut App) -> Entity {
        let panel = app.world_mut().spawn_empty().id();
        app.world_mut()
            .spawn((
                PanelWidget::new(PanelElementId::named(BUTTON_ID)),
                WidgetKind::Button,
                WidgetOf::new(panel),
            ))
            .id()
    }

    fn pointer_events_test_app(pointer_id: PointerId) -> App {
        let mut app = test_app();
        app.add_plugins(InteractionPlugin)
            .configure_sets(
                PreUpdate,
                PickingSystems::Hover.run_if(PickingSettings::hover_should_run),
            )
            .init_resource::<PointerMap>();
        add_pointer(&mut app, pointer_id);
        app
    }

    fn add_pointer(app: &mut App, pointer_id: PointerId) {
        app.world_mut()
            .spawn((pointer_id, PointerLocation::new(location())));
        let result = app.world_mut().run_system_cached(update_pointer_map);
        assert!(result.is_ok());
    }

    fn set_hover_maps(
        app: &mut App,
        pointer_id: PointerId,
        previous: &[Entity],
        current: &[Entity],
    ) {
        let hover_entries = |entities: &[Entity]| {
            entities
                .iter()
                .copied()
                .map(|entity| (entity, hit()))
                .collect()
        };
        app.world_mut()
            .resource_mut::<PreviousHoverMap>()
            .insert(pointer_id, hover_entries(previous));
        app.world_mut()
            .resource_mut::<HoverMap>()
            .insert(pointer_id, hover_entries(current));
    }

    fn run_pointer_actions(
        app: &mut App,
        pointer_id: PointerId,
        actions: impl IntoIterator<Item = PointerAction>,
    ) {
        run_pointer_inputs(app, actions.into_iter().map(|action| (pointer_id, action)));
    }

    fn run_pointer_inputs(
        app: &mut App,
        inputs: impl IntoIterator<Item = (PointerId, PointerAction)>,
    ) {
        for (pointer_id, action) in inputs {
            app.world_mut()
                .write_message(PointerInput::new(pointer_id, location(), action));
        }
        let result = app.world_mut().run_system_cached(pointer_events);
        assert!(result.is_ok());
        let result = app
            .world_mut()
            .run_system_cached(capture::reconcile_pointer_input);
        assert!(result.is_ok());
    }

    fn spawn_child_button(app: &mut App) -> Entity {
        let panel = app
            .world_mut()
            .spawn_empty()
            .observe(record_panel_click)
            .id();
        let mut widget = Entity::PLACEHOLDER;
        app.world_mut().entity_mut(panel).with_children(|children| {
            widget = children
                .spawn((
                    PanelWidget::new(PanelElementId::named(BUTTON_ID)),
                    WidgetKind::Button,
                    WidgetOf::new(panel),
                ))
                .id();
        });
        widget
    }

    fn integrated_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<RecordedButtonEvents>()
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .add_observer(record_pressed)
            .add_observer(record_released)
            .add_observer(record_clicked)
            .add_observer(record_canceled);
        app
    }

    fn integrated_ime_test_app() -> App {
        let mut app = integrated_test_app();
        app.add_plugins(InputPlugin)
            .add_message::<Ime>()
            .add_message::<WindowClosed>()
            .add_message::<WindowFocused>()
            .add_plugins(ImePlugin)
            .init_resource::<ImeObservation>()
            .add_observer(record_ime_open)
            .add_observer(record_ime_started)
            .add_observer(record_ime_commit);
        app
    }

    fn button_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button(BUTTON_ID), |_| {});
        builder.build()
    }

    fn callback_button_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().button(BUTTON_ID).on_click(record_callback_click),
            |_| {},
        );
        builder.build()
    }

    fn field_spec() -> ImeEditableFieldSpec {
        ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"))
    }

    fn button_and_field_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().editable_field(FIELD_ID, field_spec()), |_| {});
        builder.with(El::new().button(BUTTON_ID), |_| {});
        builder.build()
    }

    fn button_over_field_tree() -> LayoutTree {
        let mut builder =
            LayoutBuilder::with_root(El::overlay().width(Sizing::GROW).height(Sizing::GROW));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .editable_field(FIELD_ID, field_spec()),
            |_| {},
        );
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .button(BUTTON_ID),
            |_| {},
        );
        builder.build()
    }

    fn slider_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().widget(BUTTON_ID, Slider::new(0.0..=1.0).value(0.5)),
            |_| {},
        );
        builder.build()
    }

    fn empty_tree() -> LayoutTree { LayoutBuilder::new(100.0, 50.0).build() }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        app.world_mut().spawn(panel).id()
    }

    fn resolve_widget(app: &mut App, panel: Entity) -> Entity {
        let id = PanelElementId::named(BUTTON_ID);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let Ok(widget) = result else {
            return Entity::PLACEHOLDER;
        };
        assert!(widget.is_some());
        widget.unwrap_or(Entity::PLACEHOLDER)
    }

    fn location() -> Location {
        Location {
            target:   NormalizedRenderTarget::None {
                width:  1,
                height: 1,
            },
            position: Vec2::ZERO,
        }
    }

    fn hit() -> HitData { HitData::new(Entity::PLACEHOLDER, 0.0, None, None) }

    fn press(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            location(),
            Press {
                button: PointerButton::Primary,
                hit:    hit(),
                count:  1,
            },
            widget,
        ));
        app.world_mut().flush();
    }

    fn click(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            location(),
            Click {
                button:   PointerButton::Primary,
                hit:      hit(),
                duration: std::time::Duration::ZERO,
                count:    1,
            },
            widget,
        ));
        app.world_mut().flush();
    }

    fn double_click(
        app: &mut App,
        target: Entity,
        pointer_id: PointerId,
        location: Location,
        hit: HitData,
    ) {
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            location,
            Click {
                button: PointerButton::Primary,
                hit,
                duration: std::time::Duration::ZERO,
                count: 2,
            },
            target,
        ));
        app.world_mut().flush();
    }

    fn open_panel_ime(app: &mut App, window: Entity, panel: Entity) {
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::WorldPanelField {
                panel,
                field_id: PanelElementId::named(FIELD_ID),
            },
            window,
            initial_text: String::new(),
            field_spec: field_spec(),
            anchor: None,
        });
        app.world_mut().flush();
    }

    fn handle_ime_blur(app: &mut App) {
        let result = app.world_mut().run_system_once(ime::handle_blur_intent);
        assert!(result.is_ok());
        app.world_mut().flush();
    }

    fn field_hit_position(app: &App, panel: Entity) -> Option<Vec3> {
        let panel_data = app.world().get::<DiegeticPanel>(panel)?;
        let computed = app.world().get::<crate::ComputedDiegeticPanel>(panel)?;
        let transform = app.world().get::<GlobalTransform>(panel)?;
        let record = computed.field_records().first()?;
        let panel_local = Vec2::new(
            record.bounds.width.mul_add(0.5, record.bounds.x),
            record.bounds.height.mul_add(0.5, record.bounds.y),
        );
        let points_to_world = panel_data.points_to_world();
        let (anchor_x, anchor_y) = panel_data.anchor_offsets();
        let local = Vec3::new(
            panel_local.x.mul_add(points_to_world, -anchor_x),
            (-panel_local.y).mul_add(points_to_world, anchor_y),
            0.0,
        );
        Some(transform.transform_point(local))
    }

    fn release(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            location(),
            Release {
                button: PointerButton::Primary,
                hit:    hit(),
            },
            widget,
        ));
        app.world_mut().flush();
    }

    fn cancel_pointer(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            location(),
            Cancel { hit: hit() },
            widget,
        ));
        app.world_mut().flush();
    }

    fn assert_pending_capture(app: &App, widget: Entity, pointer_id: PointerId) {
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        let captures = app.world().resource::<WidgetCaptures>();
        assert_eq!(captures.widget(pointer_id), Some(widget));
        assert_eq!(captures.pointer(widget), Some(pointer_id));
        assert!(matches!(
            app.world()
                .resource::<ButtonCaptures>()
                .presses
                .get(&widget)
                .map(|press| press.terminal),
            Some(ButtonTerminal::Pending)
        ));
    }

    fn assert_no_captures(app: &App) {
        assert!(app.world().resource::<WidgetCaptures>().is_empty());
        assert!(app.world().resource::<ButtonCaptures>().presses.is_empty());
    }

    fn capture_sequence(app: &App, pointer_id: PointerId) -> Option<u64> {
        app.world()
            .resource::<WidgetCaptures>()
            .owner(pointer_id)
            .map(CapturedWidget::sequence)
    }

    fn click_count(app: &App, widget: Entity, pointer_id: PointerId) -> Option<u8> {
        app.world()
            .resource::<PointerState>()
            .get(pointer_id, PointerButton::Primary)?
            .clicking
            .get(&widget)
            .map(|(_, count)| *count)
    }

    fn events(app: &App) -> &[RecordedButtonEvent] {
        &app.world().resource::<RecordedButtonEvents>().0
    }

    fn clear_events(app: &mut App) {
        app.world_mut()
            .resource_mut::<RecordedButtonEvents>()
            .0
            .clear();
    }

    fn send_key(app: &mut App, window: Entity, key_code: KeyCode, state: ButtonState) {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state,
            text: None,
            repeat: false,
            window,
        });
    }

    #[test]
    fn pointer_click_releases_before_clicking_with_the_captured_pointer() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(7);

        press(&mut app, widget, pointer_id);
        click(&mut app, widget, pointer_id);
        release(&mut app, widget, pointer_id);

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(pointer_id),
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Clicked(Some(pointer_id)),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn ordered_press_click_release_observes_the_pending_press() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(9);

        {
            let mut commands = app.world_mut().commands();
            commands.trigger(Pointer::new(
                pointer_id,
                location(),
                Press {
                    button: PointerButton::Primary,
                    hit:    hit(),
                    count:  1,
                },
                widget,
            ));
            commands.trigger(Pointer::new(
                pointer_id,
                location(),
                Click {
                    button:   PointerButton::Primary,
                    hit:      hit(),
                    duration: std::time::Duration::ZERO,
                    count:    1,
                },
                widget,
            ));
            commands.trigger(Pointer::new(
                pointer_id,
                location(),
                Release {
                    button: PointerButton::Primary,
                    hit:    hit(),
                },
                widget,
            ));
        }
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(pointer_id),
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Clicked(Some(pointer_id)),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn pointer_events_reconciles_same_frame_press_release_on_new_hover() {
        let pointer_id = PointerId::Touch(41);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[], &[widget]);

        run_pointer_actions(
            &mut app,
            pointer_id,
            [
                PointerAction::Press(PointerButton::Primary),
                PointerAction::Release(PointerButton::Primary),
            ],
        );

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(pointer_id),
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Clicked(Some(pointer_id)),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn pointer_events_reconciles_release_after_previous_hover_disappears() {
        let pointer_id = PointerId::Touch(42);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);
        set_hover_maps(&mut app, pointer_id, &[], &[]);

        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Release(PointerButton::Primary)],
        );

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::CaptureLost,
            )]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn pointer_events_targeted_release_flushes_before_later_press() {
        let pointer_id = PointerId::Touch(43);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);

        run_pointer_actions(
            &mut app,
            pointer_id,
            [
                PointerAction::Release(PointerButton::Primary),
                PointerAction::Press(PointerButton::Primary),
            ],
        );

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Clicked(Some(pointer_id)),
                RecordedButtonEvent::Pressed(pointer_id),
            ]
        );
        assert_pending_capture(&app, widget, pointer_id);
    }

    #[test]
    fn pointer_events_recaptures_press_after_unresolved_release() {
        let pointer_id = PointerId::Touch(44);
        let mut app = pointer_events_test_app(pointer_id);
        let first_widget = spawn_button(&mut app);
        let second_widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[first_widget], &[first_widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);
        set_hover_maps(&mut app, pointer_id, &[], &[second_widget]);

        run_pointer_actions(
            &mut app,
            pointer_id,
            [
                PointerAction::Release(PointerButton::Primary),
                PointerAction::Press(PointerButton::Primary),
            ],
        );

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Canceled(pointer_id, ButtonCancelCause::CaptureLost),
                RecordedButtonEvent::Pressed(pointer_id),
            ]
        );
        assert!(app.world().get::<ButtonPress>(first_widget).is_none());
        assert_pending_capture(&app, second_widget, pointer_id);
    }

    #[test]
    fn pointer_events_recaptures_same_button_after_unresolved_release() {
        let pointer_id = PointerId::Touch(46);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);
        set_hover_maps(&mut app, pointer_id, &[], &[widget]);

        run_pointer_actions(
            &mut app,
            pointer_id,
            [
                PointerAction::Release(PointerButton::Primary),
                PointerAction::Press(PointerButton::Primary),
            ],
        );

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Clicked(Some(pointer_id)),
                RecordedButtonEvent::Pressed(pointer_id),
            ]
        );
        assert_pending_capture(&app, widget, pointer_id);
    }

    #[test]
    fn reset_click_count_does_not_alias_same_button_recapture() {
        let pointer_id = PointerId::Touch(47);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        assert_eq!(click_count(&app, widget, pointer_id), Some(1));
        let Some(first_sequence) = capture_sequence(&app, pointer_id) else {
            return;
        };
        clear_events(&mut app);
        app.world_mut()
            .resource_mut::<PointerState>()
            .get_mut(pointer_id, PointerButton::Primary)
            .clicking
            .clear();
        set_hover_maps(&mut app, pointer_id, &[], &[widget]);

        run_pointer_actions(
            &mut app,
            pointer_id,
            [
                PointerAction::Release(PointerButton::Primary),
                PointerAction::Press(PointerButton::Primary),
            ],
        );

        assert_eq!(click_count(&app, widget, pointer_id), Some(1));
        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Clicked(Some(pointer_id)),
                RecordedButtonEvent::Pressed(pointer_id),
            ]
        );
        assert_pending_capture(&app, widget, pointer_id);
        assert!(
            capture_sequence(&app, pointer_id).is_some_and(|sequence| sequence > first_sequence)
        );
    }

    #[test]
    fn saturated_click_count_does_not_alias_same_button_recapture() {
        let pointer_id = PointerId::Touch(48);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        app.world_mut()
            .resource_mut::<PointerState>()
            .get_mut(pointer_id, PointerButton::Primary)
            .clicking
            .insert(widget, (std::time::Instant::now(), u8::MAX));
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        assert_eq!(click_count(&app, widget, pointer_id), Some(u8::MAX));
        let Some(first_sequence) = capture_sequence(&app, pointer_id) else {
            return;
        };
        clear_events(&mut app);
        set_hover_maps(&mut app, pointer_id, &[], &[widget]);

        run_pointer_actions(
            &mut app,
            pointer_id,
            [
                PointerAction::Release(PointerButton::Primary),
                PointerAction::Press(PointerButton::Primary),
            ],
        );

        assert_eq!(click_count(&app, widget, pointer_id), Some(u8::MAX));
        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Clicked(Some(pointer_id)),
                RecordedButtonEvent::Pressed(pointer_id),
            ]
        );
        assert_pending_capture(&app, widget, pointer_id);
        assert!(
            capture_sequence(&app, pointer_id).is_some_and(|sequence| sequence > first_sequence)
        );
    }

    #[test]
    fn release_then_other_pointer_press_hands_off_same_button() {
        let first_pointer = PointerId::Touch(49);
        let second_pointer = PointerId::Touch(50);
        let mut app = pointer_events_test_app(first_pointer);
        add_pointer(&mut app, second_pointer);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, first_pointer, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            first_pointer,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);
        set_hover_maps(&mut app, first_pointer, &[], &[widget]);
        set_hover_maps(&mut app, second_pointer, &[], &[widget]);

        run_pointer_inputs(
            &mut app,
            [
                (
                    first_pointer,
                    PointerAction::Release(PointerButton::Primary),
                ),
                (second_pointer, PointerAction::Press(PointerButton::Primary)),
            ],
        );

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(first_pointer),
                RecordedButtonEvent::Clicked(Some(first_pointer)),
                RecordedButtonEvent::Pressed(second_pointer),
            ]
        );
        assert_pending_capture(&app, widget, second_pointer);
        assert_eq!(capture_sequence(&app, first_pointer), None);
    }

    #[test]
    fn other_pointer_press_before_release_stays_rejected() {
        let first_pointer = PointerId::Touch(51);
        let second_pointer = PointerId::Touch(52);
        let mut app = pointer_events_test_app(first_pointer);
        add_pointer(&mut app, second_pointer);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, first_pointer, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            first_pointer,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);
        set_hover_maps(&mut app, first_pointer, &[], &[widget]);
        set_hover_maps(&mut app, second_pointer, &[], &[widget]);

        run_pointer_inputs(
            &mut app,
            [
                (second_pointer, PointerAction::Press(PointerButton::Primary)),
                (
                    first_pointer,
                    PointerAction::Release(PointerButton::Primary),
                ),
            ],
        );

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(first_pointer),
                RecordedButtonEvent::Clicked(Some(first_pointer)),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn disabled_hover_processing_cancels_raw_release_once() {
        let pointer_id = PointerId::Touch(53);
        let mut app = pointer_events_test_app(pointer_id);
        app.add_message::<PointerHits>();
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);
        app.world_mut()
            .resource_mut::<PickingSettings>()
            .is_hover_enabled = false;
        app.world_mut().run_schedule(First);
        app.world_mut().write_message(PointerInput::new(
            pointer_id,
            location(),
            PointerAction::Release(PointerButton::Primary),
        ));

        app.world_mut().run_schedule(PreUpdate);

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::CaptureLost,
            )]
        );
        assert!(
            app.world()
                .resource::<PointerState>()
                .get(pointer_id, PointerButton::Primary)
                .is_some_and(|state| state.pressing.contains_key(&widget))
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);

        {
            let mut settings = app.world_mut().resource_mut::<PickingSettings>();
            settings.is_enabled = true;
            settings.is_hover_enabled = true;
        }
        app.update();

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::CaptureLost,
            )]
        );
        assert!(
            app.world()
                .resource::<PointerState>()
                .get(pointer_id, PointerButton::Primary)
                .is_some_and(|state| !state.pressing.contains_key(&widget))
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);

        app.update();

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::CaptureLost,
            )]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn picking_without_interaction_skips_pointer_reconciliation() {
        let mut app = integrated_test_app();
        app.set_error_handler(bevy::ecs::error::panic)
            .insert_resource(PickingSettings {
                is_enabled: false,
                ..default()
            })
            .add_plugins(PickingPlugin);

        app.update();

        assert!(!app.world().contains_resource::<PointerState>());
        assert!(!app.world().contains_resource::<HoverMap>());
    }

    #[test]
    fn pointer_events_raw_cancel_over_empty_is_exact_once() {
        let pointer_id = PointerId::Touch(45);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);
        set_hover_maps(&mut app, pointer_id, &[], &[]);

        run_pointer_actions(&mut app, pointer_id, [PointerAction::Cancel]);
        let result = app
            .world_mut()
            .run_system_cached(capture::reconcile_pointer_input);
        assert!(result.is_ok());

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::PointerCanceled,
            )]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn raw_cancel_finalizes_lost_widget_with_pointer_canceled_cause() {
        let pointer_id = PointerId::Touch(60);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);

        // Lose the lookup components while the shared occupancy and typed button
        // payload survive, so the reconciler's `WidgetKind` lookup returns
        // `None`.
        app.world_mut().entity_mut(widget).remove::<WidgetOf>();
        app.world_mut().flush();

        // A raw cancel the reconciler owns (cleared hover suppresses the targeted
        // `Pointer<Cancel>`). The lost-widget fallback must keep the concrete
        // `PointerCanceled` cause rather than degrading a cancel to `CaptureLost`.
        set_hover_maps(&mut app, pointer_id, &[], &[]);
        run_pointer_actions(&mut app, pointer_id, [PointerAction::Cancel]);

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::PointerCanceled,
            )]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn raw_release_finalizes_lost_widget_with_capture_lost_cause() {
        let pointer_id = PointerId::Touch(61);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);

        app.world_mut().entity_mut(widget).remove::<WidgetOf>();
        app.world_mut().flush();

        // A raw release the reconciler owns (cleared previous hover suppresses the
        // targeted `Pointer<Release>`). A kind-lost release keeps `CaptureLost`.
        set_hover_maps(&mut app, pointer_id, &[], &[]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Release(PointerButton::Primary)],
        );

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::CaptureLost,
            )]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn raw_release_with_present_kind_and_missing_payload_frees_nothing() {
        let pointer_id = PointerId::Touch(62);
        let mut app = pointer_events_test_app(pointer_id);
        let widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[widget], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);

        // Drop the typed press payload while the `WidgetKind` lookup and shared
        // occupancy remain: the raw release lands over the capture but finds no
        // payload to complete.
        assert!(
            app.world_mut()
                .resource_mut::<ButtonCaptures>()
                .take(widget)
                .is_some()
        );

        // A raw release the reconciler owns (empty previous hover) with the widget
        // still under the current hover, then a later press. Because the release
        // records no terminal, the reconciler must not remove the marker or mark
        // the pointer freed, so the later press cannot claim the widget.
        set_hover_maps(&mut app, pointer_id, &[], &[widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [
                PointerAction::Release(PointerButton::Primary),
                PointerAction::Press(PointerButton::Primary),
            ],
        );

        assert!(events(&app).is_empty());
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        let captures = app.world().resource::<WidgetCaptures>();
        assert_eq!(captures.widget(pointer_id), Some(widget));
        assert_eq!(captures.pointer(widget), Some(pointer_id));
    }

    #[test]
    fn pointer_events_ignores_press_after_raw_cancel() {
        let pointer_id = PointerId::Touch(54);
        let mut app = pointer_events_test_app(pointer_id);
        let captured_widget = spawn_button(&mut app);
        let hovered_widget = spawn_button(&mut app);
        set_hover_maps(&mut app, pointer_id, &[captured_widget], &[captured_widget]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Press(PointerButton::Primary)],
        );
        clear_events(&mut app);
        set_hover_maps(&mut app, pointer_id, &[], &[hovered_widget]);

        run_pointer_actions(
            &mut app,
            pointer_id,
            [
                PointerAction::Cancel,
                PointerAction::Press(PointerButton::Primary),
            ],
        );
        let result = app
            .world_mut()
            .run_system_cached(capture::reconcile_pointer_input);
        assert!(result.is_ok());

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::PointerCanceled,
            )]
        );
        assert!(app.world().get::<ButtonPress>(captured_widget).is_none());
        assert!(app.world().get::<ButtonPress>(hovered_widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn ordered_click_release_drag_end_preserves_committed_release() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(8);

        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        {
            let mut commands = app.world_mut().commands();
            commands.trigger(Pointer::new(
                pointer_id,
                location(),
                Click {
                    button:   PointerButton::Primary,
                    hit:      hit(),
                    duration: std::time::Duration::ZERO,
                    count:    1,
                },
                widget,
            ));
            commands.trigger(Pointer::new(
                pointer_id,
                location(),
                Release {
                    button: PointerButton::Primary,
                    hit:    hit(),
                },
                widget,
            ));
            commands.trigger(Pointer::new(
                pointer_id,
                location(),
                DragEnd {
                    button:   PointerButton::Primary,
                    distance: Vec2::ONE,
                },
                widget,
            ));
        }
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Clicked(Some(pointer_id)),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn targeted_release_over_captured_widget_completes_without_activation() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Mouse;

        press(&mut app, widget, pointer_id);
        release(&mut app, widget, pointer_id);

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(pointer_id),
                RecordedButtonEvent::Released(pointer_id),
            ]
        );
    }

    #[test]
    fn another_pointer_cannot_terminate_or_replace_capture() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let captured = PointerId::Touch(1);
        let other = PointerId::Touch(2);

        press(&mut app, widget, captured);
        press(&mut app, widget, other);
        release(&mut app, widget, other);
        assert_eq!(events(&app), [RecordedButtonEvent::Pressed(captured)]);

        release(&mut app, widget, captured);
        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(captured),
                RecordedButtonEvent::Released(captured),
            ]
        );
    }

    #[test]
    fn captured_pointer_cannot_capture_another_button() {
        let mut app = test_app();
        let captured_widget = spawn_button(&mut app);
        let other_widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(2);

        press(&mut app, captured_widget, pointer_id);
        press(&mut app, other_widget, pointer_id);

        assert!(app.world().get::<ButtonPress>(captured_widget).is_some());
        assert!(app.world().get::<ButtonPress>(other_widget).is_none());
        assert_eq!(events(&app), [RecordedButtonEvent::Pressed(pointer_id)]);

        release(&mut app, captured_widget, pointer_id);
        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(pointer_id),
                RecordedButtonEvent::Released(pointer_id),
            ]
        );
    }

    #[test]
    fn exhausted_press_sequence_rejects_new_press_and_preserves_the_owner() {
        let mut app = test_app();
        let captured_widget = spawn_button(&mut app);
        let other_widget = spawn_button(&mut app);
        let captured = PointerId::Touch(61);
        let other = PointerId::Touch(62);

        press(&mut app, captured_widget, captured);
        clear_events(&mut app);
        app.world_mut()
            .resource_mut::<WidgetCaptures>()
            .saturate_sequence();

        press(&mut app, other_widget, other);
        assert!(events(&app).is_empty());
        assert!(app.world().get::<ButtonPress>(other_widget).is_none());
        assert_pending_capture(&app, captured_widget, captured);

        release(&mut app, captured_widget, captured);
        assert_eq!(events(&app), [RecordedButtonEvent::Released(captured)]);
        assert_no_captures(&app);
    }

    #[test]
    fn raw_cancel_over_empty_space_cancels_exactly_once() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(3);

        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        app.world_mut().write_message(PointerInput::new(
            pointer_id,
            location(),
            PointerAction::Cancel,
        ));
        app.update();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::PointerCanceled,
            )]
        );
        app.update();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::PointerCanceled,
            )]
        );
    }

    #[test]
    fn targeted_release_then_new_press_preserves_the_new_capture() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(31);

        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        release(&mut app, widget, pointer_id);
        press(&mut app, widget, pointer_id);

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(pointer_id),
                RecordedButtonEvent::Pressed(pointer_id),
            ]
        );
        assert_pending_capture(&app, widget, pointer_id);
    }

    #[test]
    fn cancel_then_press_is_an_invalid_stream_that_does_not_replay_capture() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(32);

        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        app.world_mut().write_message(PointerInput::new(
            pointer_id,
            location(),
            PointerAction::Cancel,
        ));
        cancel_pointer(&mut app, widget, pointer_id);
        app.world_mut().write_message(PointerInput::new(
            pointer_id,
            location(),
            PointerAction::Press(PointerButton::Primary),
        ));
        press(&mut app, widget, pointer_id);

        app.update();

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Canceled(pointer_id, ButtonCancelCause::PointerCanceled,),
                RecordedButtonEvent::Pressed(pointer_id),
                RecordedButtonEvent::Canceled(pointer_id, ButtonCancelCause::PointerCanceled,),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn targeted_drag_end_after_dragging_away_cancels_capture_once() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Mouse;

        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            location(),
            DragEnd {
                button:   PointerButton::Primary,
                distance: Vec2::ONE,
            },
            widget,
        ));
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::CaptureLost,
            )]
        );
    }

    #[test]
    fn disable_and_widget_removal_cancel_capture_once() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let pointer_id = PointerId::Mouse;

        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        let result = app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.override_interactivity(widget, WidgetInteractivity::Disabled)
            });
        assert_eq!(result.ok(), Some(true));
        app.update();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::Disabled,
            )]
        );

        let result = app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.override_interactivity(widget, WidgetInteractivity::Enabled)
            });
        assert_eq!(result.ok(), Some(true));
        app.update();
        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        app.init_resource::<TeardownObservation>();
        install_attachment_relations(&mut app, widget);
        app.world_mut()
            .entity_mut(widget)
            .observe(observe_teardown_cancellation);
        app.world_mut().entity_mut(widget).despawn();
        app.world_mut().flush();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::WidgetRemoved,
            )]
        );
        let observation = app.world().resource::<TeardownObservation>();
        assert_eq!(observation.cancellations, 1);
        assert_eq!(observation.relations_seen, 1);
    }

    #[test]
    fn pointer_removal_and_semantic_cancel_preserve_captured_pointer() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(11);
        let pointer = app.world_mut().spawn(pointer_id).id();

        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        app.world_mut().entity_mut(pointer).despawn();
        app.world_mut().flush();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::PointerRemoved,
            )]
        );

        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        app.world_mut()
            .trigger(SemanticWidgetIntent::Cancel { entity: widget });
        app.world_mut().flush();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::Explicit,
            )]
        );
        app.world_mut()
            .trigger(SemanticWidgetIntent::Cancel { entity: widget });
        app.world_mut().flush();
        assert_eq!(events(&app).len(), 1);
    }

    #[test]
    fn semantic_activation_emits_only_a_pointerless_click() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        });
        app.world_mut().flush();

        assert_eq!(events(&app), [RecordedButtonEvent::Clicked(None)]);
        assert!(app.world().get::<ButtonPress>(widget).is_none());
    }

    #[test]
    fn on_click_receives_pointer_and_semantic_clicks() {
        let mut app = integrated_test_app();
        app.init_resource::<CallbackClicks>();
        let panel = spawn_panel(&mut app, callback_button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let pointer_id = PointerId::Mouse;

        press(&mut app, widget, pointer_id);
        click(&mut app, widget, pointer_id);
        release(&mut app, widget, pointer_id);
        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        });
        app.world_mut().flush();

        assert_eq!(
            app.world().resource::<CallbackClicks>().0,
            [(widget, Some(pointer_id)), (widget, None)]
        );
    }

    #[test]
    fn one_plugin_observer_dispatches_and_the_widget_owns_no_observer() {
        let mut app = integrated_test_app();
        app.init_resource::<CallbackClicks>();
        let panel = spawn_panel(&mut app, callback_button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert!(
            app.world().get::<ObservedBy>(widget).is_none(),
            "reify must not install a per-widget observer",
        );

        let pointer_id = PointerId::Mouse;
        press(&mut app, widget, pointer_id);
        click(&mut app, widget, pointer_id);
        release(&mut app, widget, pointer_id);

        assert_eq!(
            app.world().resource::<CallbackClicks>().0,
            [(widget, Some(pointer_id))],
            "exactly one observer dispatches the callback per click",
        );
    }

    #[test]
    fn entity_scoped_observer_on_reader_resolved_widget_receives_the_click() {
        let mut app = integrated_test_app();
        app.init_resource::<ScopedClicks>();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .entity_mut(widget)
            .observe(record_scoped_click);

        let pointer_id = PointerId::Mouse;
        press(&mut app, widget, pointer_id);
        click(&mut app, widget, pointer_id);
        release(&mut app, widget, pointer_id);

        assert_eq!(app.world().resource::<ScopedClicks>().0, [widget]);
    }

    #[test]
    fn public_focused_activation_emits_one_pointerless_click_without_a_press_lifecycle() {
        let mut app = integrated_test_app();
        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        assert!(app.world().get::<crate::WidgetFocused>(widget).is_some());
        app.world_mut()
            .write_message(WidgetInput::Activate { window });
        app.update();
        app.world_mut().flush();

        assert_eq!(events(&app), [RecordedButtonEvent::Clicked(None)]);
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn resolved_semantic_activation_ignores_later_derived_disabled_marker() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let result = app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.override_interactivity(widget, WidgetInteractivity::Disabled)
            });
        assert_eq!(result.ok(), Some(true));
        app.update();
        assert!(app.world().get::<crate::WidgetDisabled>(widget).is_some());

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        });
        app.world_mut().flush();

        assert_eq!(events(&app), [RecordedButtonEvent::Clicked(None)]);
    }

    #[test]
    fn button_click_stops_propagation_to_its_owner_panel() {
        let mut app = test_app();
        app.init_resource::<PanelClicks>();
        let widget = spawn_child_button(&mut app);

        click(&mut app, widget, PointerId::Mouse);

        assert_eq!(app.world().resource::<PanelClicks>().0, 0);
    }

    #[test]
    fn button_click_ends_panel_field_editing_on_same_or_other_panel() {
        let mut app = integrated_ime_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let source_panel = spawn_panel(&mut app, button_and_field_tree());
        let other_panel = spawn_panel(&mut app, button_tree());
        app.update();
        let source_button = resolve_widget(&mut app, source_panel);
        let other_button = resolve_widget(&mut app, other_panel);

        open_panel_ime(&mut app, window, source_panel);
        click(&mut app, source_button, PointerId::Mouse);
        handle_ime_blur(&mut app);
        assert_eq!(
            app.world().resource::<ImeObservation>().commit_causes,
            [ImeCommitCause::Blur]
        );
        app.world_mut()
            .resource_mut::<ImeObservation>()
            .commit_causes
            .clear();

        open_panel_ime(&mut app, window, source_panel);
        click(&mut app, other_button, PointerId::Mouse);
        handle_ime_blur(&mut app);
        assert_eq!(
            app.world().resource::<ImeObservation>().commit_causes,
            [ImeCommitCause::Blur]
        );
    }

    #[test]
    fn button_over_field_blocks_the_panel_double_click_activator() {
        let mut app = integrated_ime_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app.world_mut().spawn(Camera::default()).id();
        let panel = spawn_panel(&mut app, button_over_field_tree());
        app.update();
        app.world_mut().flush();
        let widget = resolve_widget(&mut app, panel);
        let position = field_hit_position(&app, panel);
        assert!(position.is_some());
        let Some(position) = position else {
            return;
        };
        let panel_data = app.world().get::<DiegeticPanel>(panel);
        let computed = app.world().get::<crate::ComputedDiegeticPanel>(panel);
        let transform = app.world().get::<GlobalTransform>(panel);
        assert!(panel_data.is_some());
        assert!(computed.is_some());
        assert!(transform.is_some());
        let (Some(panel_data), Some(computed), Some(transform)) = (panel_data, computed, transform)
        else {
            return;
        };
        let projected = crate::render::project_flat_panel_hit(position, panel_data, transform);
        assert!(projected.is_some());
        let Some(projected) = projected else {
            return;
        };
        assert!(computed.field_at_local_position(projected).is_some());
        let window_ref = WindowRef::Entity(window).normalize(None);
        assert!(window_ref.is_some());
        let Some(window_ref) = window_ref else {
            return;
        };
        let location = Location {
            target:   NormalizedRenderTarget::Window(window_ref),
            position: Vec2::ZERO,
        };
        let hit = HitData::new(camera, 0.0, Some(position), None);

        double_click(
            &mut app,
            panel,
            PointerId::Mouse,
            location.clone(),
            hit.clone(),
        );
        assert!(app.world().resource::<ImeObservation>().opens > 0);
        assert!(app.world().resource::<ImeObservation>().starts > 0);
        app.world_mut().resource_mut::<ImeObservation>().starts = 0;
        app.world_mut().resource_mut::<ImeObservation>().opens = 0;

        double_click(&mut app, widget, PointerId::Mouse, location, hit);
        assert_eq!(app.world().resource::<ImeObservation>().opens, 0);
        assert_eq!(app.world().resource::<ImeObservation>().starts, 0);
    }

    #[test]
    fn same_kind_tree_and_enabled_interactivity_refreshes_preserve_capture() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let pointer_id = PointerId::Mouse;
        press(&mut app, widget, pointer_id);
        clear_events(&mut app);

        let result = app.world_mut().commands().set_tree(panel, button_tree());
        assert!(result.is_ok());
        app.update();
        assert_eq!(resolve_widget(&mut app, panel), widget);
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        assert!(events(&app).is_empty());

        let result = app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.override_interactivity(widget, WidgetInteractivity::Enabled)
            });
        assert_eq!(result.ok(), Some(true));
        app.update();
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        assert!(events(&app).is_empty());

        release(&mut app, widget, pointer_id);
        assert_eq!(events(&app), [RecordedButtonEvent::Released(pointer_id)]);
    }

    #[test]
    fn kind_change_and_tree_removal_cancel_before_reuse_or_despawn() {
        let mut app = integrated_test_app();
        app.init_resource::<TeardownObservation>();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let pointer_id = PointerId::Touch(19);
        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        install_attachment_relations(&mut app, widget);
        app.world_mut()
            .entity_mut(widget)
            .observe(observe_teardown_cancellation);

        let result = app.world_mut().commands().set_tree(panel, slider_tree());
        assert!(result.is_ok());
        app.update();
        assert_eq!(resolve_widget(&mut app, panel), widget);
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::WidgetKindChanged,
            )]
        );
        let observation = app.world().resource::<TeardownObservation>();
        assert_eq!(observation.cancellations, 1);
        assert_eq!(observation.relations_seen, 1);
        assert_no_captures(&app);

        let result = app.world_mut().commands().set_tree(panel, button_tree());
        assert!(result.is_ok());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        let result = app.world_mut().commands().set_tree(panel, empty_tree());
        assert!(result.is_ok());
        app.update();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::WidgetRemoved,
            )]
        );
        let observation = app.world().resource::<TeardownObservation>();
        assert_eq!(observation.cancellations, 2);
        assert_eq!(observation.relations_seen, 2);
        assert!(app.world().get_entity(widget).is_err());
        assert_no_captures(&app);
    }

    fn install_attachment_relations(app: &mut App, widget: Entity) {
        app.world_mut().spawn(AnchoredTo::new(
            widget,
            AnchorSite::Center,
            AnchorSite::Center,
        ));
        app.world_mut().spawn(ScreenWidgetAnchoredTo::new(widget));
        assert!(app.world().get::<AnchoredHere>(widget).is_some());
        assert!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(widget)
                .is_some()
        );
    }

    #[test]
    fn panel_role_removal_cancels_before_attachment_cleanup() {
        let mut app = integrated_test_app();
        app.init_resource::<TeardownObservation>();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let pointer_id = PointerId::Mouse;
        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        install_attachment_relations(&mut app, widget);
        app.world_mut()
            .entity_mut(widget)
            .observe(observe_teardown_cancellation);

        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
        app.world_mut().flush();

        let observation = app.world().resource::<TeardownObservation>();
        assert_eq!(observation.cancellations, 1);
        assert_eq!(observation.relations_seen, 1);
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::WidgetRemoved,
            )]
        );
    }

    #[test]
    fn full_panel_despawn_cancels_once_before_linked_widget_despawn() {
        let mut app = integrated_test_app();
        app.init_resource::<TeardownObservation>();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let pointer_id = PointerId::Mouse;
        press(&mut app, widget, pointer_id);
        clear_events(&mut app);
        install_attachment_relations(&mut app, widget);
        app.world_mut()
            .entity_mut(widget)
            .observe(observe_teardown_cancellation);

        assert!(app.world_mut().despawn(panel));
        app.world_mut().flush();

        let observation = app.world().resource::<TeardownObservation>();
        assert_eq!(observation.cancellations, 1);
        assert_eq!(observation.relations_seen, 1);
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::WidgetRemoved,
            )]
        );
    }

    #[test]
    fn built_in_escape_uses_explicit_cancel_and_is_idle_without_capture() {
        let mut app = integrated_test_app();
        app.add_plugins((InputPlugin, WidgetInputPlugin));
        app.finish();
        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let pointer_id = PointerId::Touch(23);
        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        press(&mut app, widget, pointer_id);
        clear_events(&mut app);

        send_key(&mut app, window, KeyCode::Escape, ButtonState::Pressed);
        app.update();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                pointer_id,
                ButtonCancelCause::Explicit,
            )]
        );

        clear_events(&mut app);
        send_key(&mut app, window, KeyCode::Escape, ButtonState::Released);
        app.update();
        send_key(&mut app, window, KeyCode::Escape, ButtonState::Pressed);
        app.update();
        assert!(events(&app).is_empty());
    }

    // ── Direct state presentation builders ─────────────────────────────────

    const NORMAL_FILL: Color = Color::srgb(0.05, 0.10, 0.20);
    const NORMAL_BORDER: Color = Color::srgb(0.30, 0.30, 0.30);
    const HOVER_FILL: Color = Color::srgb(0.15, 0.30, 0.60);
    const PRESS_FILL: Color = Color::srgb(0.60, 0.30, 0.10);
    const FOCUS_FILL: Color = Color::srgb(0.20, 0.20, 0.45);
    const FOCUS_BORDER: Color = Color::srgb(0.95, 0.85, 0.25);
    const DISABLED_BORDER: Color = Color::srgb(0.35, 0.35, 0.40);
    const DISABLED_FILL: Color = Color::srgb(0.12, 0.12, 0.14);
    const PEER_HOVER_FILL: Color = Color::srgb(0.10, 0.55, 0.25);

    /// A button element in the default row layout.
    type ButtonElement = El<crate::Row, crate::WidgetElement<Button>>;

    fn button_tree_with(
        id: &'static str,
        configure: impl FnOnce(ButtonElement) -> ButtonElement,
    ) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            configure(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .background(NORMAL_FILL)
                    .border(Border::all(1.0, NORMAL_BORDER))
                    .button(id),
            ),
            |_| {},
        );
        builder.build()
    }

    fn two_styled_buttons_tree(first_hover: Color, second_hover: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        for (id, hover) in [("first", first_hover), ("second", second_hover)] {
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .background(NORMAL_FILL)
                    .border(Border::all(1.0, NORMAL_BORDER))
                    .button(id)
                    .hovered(Appearance::new().background(hover)),
                |_| {},
            );
        }
        builder.build()
    }

    fn widget_by_id(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let widget = result.ok().flatten();
        assert!(widget.is_some());
        widget.unwrap_or(Entity::PLACEHOLDER)
    }

    fn root_override(app: &App, widget: Entity) -> Option<VisualSlotOverride> {
        let slots = app.world().get::<WidgetVisualSlots>(widget)?;
        let element_index = slots.element_index(VisualSlotId::BUTTON_ROOT)?;
        app.world()
            .get::<WidgetVisualOverrides>(widget)
            .and_then(|overrides| {
                overrides
                    .element_overrides()
                    .iter()
                    .find(|(index, _)| *index == element_index)
                    .map(|(_, value)| value.clone())
            })
    }

    fn root_element_index(app: &App, widget: Entity) -> Option<usize> {
        app.world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| slots.element_index(VisualSlotId::BUTTON_ROOT))
    }

    fn indexed_root_override(
        app: &App,
        panel: Entity,
        widget: Entity,
    ) -> Option<VisualSlotOverride> {
        let element_index = root_element_index(app, widget)?;
        app.world()
            .resource::<VisualOverrideIndex>()
            .get(panel, element_index)
            .cloned()
    }

    fn computed_tick(app: &App, panel: Entity) -> Option<Tick> {
        app.world()
            .entity(panel)
            .get_ref::<ComputedDiegeticPanel>()
            .map(|computed| computed.last_changed())
    }

    fn override_tick(app: &App, widget: Entity) -> Option<Tick> {
        app.world()
            .entity(widget)
            .get_ref::<WidgetVisualOverrides>()
            .map(|overrides| overrides.last_changed())
    }

    fn assert_override_quiet(app: &mut App, widget: Entity, tick: Option<Tick>) {
        app.update();
        assert_eq!(
            override_tick(app, widget),
            tick,
            "a quiet frame must not write the button override component",
        );
    }

    #[test]
    fn hovered_background_patches_only_the_fill_and_reaches_dispatch_same_frame() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert_eq!(root_override(&app, widget), None);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        let expected = VisualSlotOverride {
            fill_color: Some(HOVER_FILL),
            ..VisualSlotOverride::default()
        };
        assert_eq!(root_override(&app, widget), Some(expected.clone()));
        assert_eq!(
            indexed_root_override(&app, panel, widget),
            Some(expected),
            "the first insertion must reach dispatch in the same frame",
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::None);
        app.update();
        assert_eq!(root_override(&app, widget), None);
        assert_eq!(indexed_root_override(&app, panel, widget), None);
    }

    #[test]
    fn border_color_and_material_builders_patch_their_own_properties() {
        let mut app = integrated_test_app();
        let material: Handle<StandardMaterial> = Handle::default();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .hovered(Appearance::new().border_color(FOCUS_BORDER))
                    .pressed(Appearance::new().material(material.clone()))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "a border-color builder must not touch fill or material",
        );

        app.world_mut().entity_mut(widget).insert(ButtonPress);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                border_color: Some(FOCUS_BORDER),
                material: Some(material),
                ..VisualSlotOverride::default()
            }),
            "a state material applies without touching state colors",
        );
    }

    #[test]
    fn zero_width_border_lets_a_borderless_button_grow_a_focus_border() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let mut tree = LayoutBuilder::new(100.0, 50.0);
        tree.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(NORMAL_FILL)
                .border(Border::all(Px(0.0), FOCUS_BORDER))
                .button(BUTTON_ID)
                .focused(Appearance::new().border_width(Px(3.0))),
            |_| {},
        );
        let panel = spawn_panel(&mut app, tree.build());
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();
        assert!(
            root_override(&app, widget)
                .and_then(|value| value.border_widths)
                .is_some_and(|widths| widths.iter().all(|width| *width > 0.0)),
            "an authored zero-width border is a real record a focus width can widen",
        );
    }

    #[test]
    fn focused_border_width_resolves_against_the_owning_panel() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.focused(Appearance::new().border_width(Px(3.0)))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert_eq!(root_override(&app, widget), None);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();

        let points_to_world = app
            .world()
            .get::<DiegeticPanel>(panel)
            .map_or(0.0, DiegeticPanel::points_to_world);
        // Three logical pixels resolve to points through the fixed 96 DPI
        // convention, then scale to the owning panel's world units.
        let expected = 3.0 * 0.75 * points_to_world;
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                border_widths: Some([expected; 4]),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut().trigger(ClearWidgetFocus { window });
        app.world_mut().flush();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "leaving focus restores the authored border width",
        );
    }

    #[test]
    fn state_precedence_layers_independently_per_property() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .focused(
                        Appearance::new()
                            .background(FOCUS_FILL)
                            .border_color(FOCUS_BORDER),
                    )
                    .hovered(Appearance::new().background(HOVER_FILL))
                    .pressed(Appearance::new().background(PRESS_FILL))
                    .disabled(Appearance::new().border_color(DISABLED_BORDER))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(FOCUS_FILL),
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(HOVER_FILL),
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "hover replaces the focused fill while the border falls through to focus",
        );

        app.world_mut().entity_mut(widget).insert(ButtonPress);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(PRESS_FILL),
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "press replaces the hovered fill while the border still falls through",
        );

        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.world_mut().flush();
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(PRESS_FILL),
                border_color: Some(DISABLED_BORDER),
                ..VisualSlotOverride::default()
            }),
            "disabled replaces the border while the fill falls through to press",
        );
    }

    #[test]
    fn set_tree_accepts_state_targets_without_ordinary_declarations() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(&mut app, button_tree_with(BUTTON_ID, |element| element));
        app.update();

        let mut background_only = LayoutBuilder::new(100.0, 50.0);
        background_only.with(
            El::new()
                .button(BUTTON_ID)
                .hovered(Appearance::new().background(HOVER_FILL)),
            |_| {},
        );
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, background_only.build());
        assert!(result.is_ok());

        let mut border_only = LayoutBuilder::new(100.0, 50.0);
        border_only.with(
            El::new()
                .background(NORMAL_FILL)
                .button(BUTTON_ID)
                .hovered(Appearance::new().border_color(FOCUS_BORDER)),
            |_| {},
        );
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, border_only.build());
        assert!(result.is_ok());

        let mut border_width_only = LayoutBuilder::new(100.0, 50.0);
        border_width_only.with(
            El::new()
                .background(NORMAL_FILL)
                .button(BUTTON_ID)
                .focused(Appearance::new().border_width(Px(2.0))),
            |_| {},
        );
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, border_width_only.build());
        assert!(result.is_ok());

        let mut material_only = LayoutBuilder::new(100.0, 50.0);
        material_only.with(
            El::new()
                .button(BUTTON_ID)
                .disabled(Appearance::new().material(Handle::default())),
            |_| {},
        );
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, material_only.build());
        assert!(result.is_ok());

        app.update();
    }

    #[test]
    fn buttons_retain_distinct_state_values_through_computed_output_and_reify() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            two_styled_buttons_tree(HOVER_FILL, PEER_HOVER_FILL),
        );
        app.update();
        let first = widget_by_id(&mut app, panel, "first");
        let second = widget_by_id(&mut app, panel, "second");
        app.world_mut()
            .entity_mut(first)
            .insert(PickingInteraction::Hovered);
        app.world_mut()
            .entity_mut(second)
            .insert(PickingInteraction::Hovered);
        app.update();

        let fill =
            |app: &App, widget| root_override(app, widget).and_then(|value| value.fill_color);
        assert_eq!(fill(&app, first), Some(HOVER_FILL));
        assert_eq!(fill(&app, second), Some(PEER_HOVER_FILL));

        // Same-id replacement swaps the authored hover colors; the live
        // widgets keep their entities and adopt the new authored values.
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, two_styled_buttons_tree(PEER_HOVER_FILL, HOVER_FILL));
        assert!(result.is_ok());
        app.update();
        assert_eq!(widget_by_id(&mut app, panel, "first"), first);
        assert_eq!(widget_by_id(&mut app, panel, "second"), second);
        app.update();
        assert_eq!(fill(&app, first), Some(PEER_HOVER_FILL));
        assert_eq!(fill(&app, second), Some(HOVER_FILL));
    }

    #[test]
    fn removing_state_builders_on_reify_clears_the_stale_override() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(root_override(&app, widget).is_some());

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, button_tree_with(BUTTON_ID, |element| element));
        assert!(result.is_ok());
        app.update();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "a same-id button without state builders must drop the old override",
        );
    }

    #[test]
    fn pressed_presentation_reads_button_press_and_ignores_captures() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.pressed(Appearance::new().background(PRESS_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        // A capture without the `ButtonPress` marker shows no pressed state.
        let captured = app
            .world_mut()
            .resource_mut::<WidgetCaptures>()
            .try_capture(PointerId::Mouse, widget, 1);
        assert!(captured);
        app.world_mut()
            .resource_mut::<ButtonCaptures>()
            .begin_press(widget, PanelElementId::named(BUTTON_ID));
        app.update();
        assert_eq!(root_override(&app, widget), None);

        // The marker alone drives pressed presentation.
        app.world_mut()
            .resource_mut::<WidgetCaptures>()
            .release_widget(widget);
        app.world_mut()
            .resource_mut::<ButtonCaptures>()
            .presses
            .clear();
        app.world_mut().entity_mut(widget).insert(ButtonPress);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(PRESS_FILL),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn traversal_focus_is_visible_to_presentation_in_the_same_frame() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let mut tree = LayoutBuilder::new(100.0, 50.0);
        for (id, focus_border) in [("first", None), ("second", Some(FOCUS_BORDER))] {
            let element = El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(NORMAL_FILL)
                .border(Border::all(1.0, NORMAL_BORDER))
                .button(id);
            let element = match focus_border {
                Some(color) => element.focused(Appearance::new().border_color(color)),
                None => element,
            };
            tree.with(element, |_| {});
        }
        let panel = spawn_panel(&mut app, tree.build());
        app.update();
        let first = widget_by_id(&mut app, panel, "first");
        let widget = widget_by_id(&mut app, panel, "second");
        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: first,
        });
        app.world_mut().flush();
        app.update();
        assert_eq!(root_override(&app, widget), None);

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();

        let expected = VisualSlotOverride {
            border_color: Some(FOCUS_BORDER),
            ..VisualSlotOverride::default()
        };
        assert_eq!(
            root_override(&app, widget),
            Some(expected.clone()),
            "semantic traversal focus must restyle in the frame it lands",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), Some(expected));
    }

    fn pointer_presentation_app(pointer_id: PointerId) -> App {
        let mut app = integrated_test_app();
        app.add_plugins(InteractionPlugin)
            .add_message::<PointerInput>()
            .add_message::<PointerHits>()
            .init_resource::<PointerMap>();
        add_pointer(&mut app, pointer_id);
        app
    }

    fn write_widget_hit(app: &mut App, pointer_id: PointerId, camera: Entity, widget: Entity) {
        let hit = HitData::new(camera, 0.0, None, None);
        app.world_mut()
            .write_message(PointerHits::new(pointer_id, vec![(widget, hit)], 0.0));
    }

    #[test]
    fn pointer_input_retains_focus_without_showing_its_indicator() {
        let pointer_id = PointerId::Mouse;
        let mut app = pointer_presentation_app(pointer_id);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = app.world_mut().spawn(Camera::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .hovered(Appearance::new().background(HOVER_FILL))
                    .pressed(Appearance::new().background(PRESS_FILL))
                    .focused(Appearance::new().border_color(FOCUS_BORDER))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert_eq!(root_override(&app, widget), None);

        // A backend hit alone: `generate_hovermap` and `update_interactions`
        // produce the hovered `PickingInteraction`, and presentation reaches
        // the override in the same `app.update()`.
        write_widget_hit(&mut app, pointer_id, camera, widget);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(HOVER_FILL),
                ..VisualSlotOverride::default()
            }),
            "a real backend hover hit must restyle in the update it lands",
        );

        // A raw primary press through Bevy's real `pointer_events` dispatcher:
        // the press observers insert `ButtonPress` and retain pointer focus,
        // but pointer focus does not draw the keyboard focus indicator.
        write_widget_hit(&mut app, pointer_id, camera, widget);
        app.world_mut().write_message(PointerInput::new(
            pointer_id,
            location(),
            PointerAction::Press(PointerButton::Primary),
        ));
        app.update();
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        assert!(app.world().get::<crate::WidgetFocused>(widget).is_some());
        assert!(app.world().get::<WidgetFocusVisible>(widget).is_none());
        let expected = VisualSlotOverride {
            fill_color: Some(PRESS_FILL),
            ..VisualSlotOverride::default()
        };
        assert_eq!(
            root_override(&app, widget),
            Some(expected.clone()),
            "a pointer press must show pressed state without the keyboard focus border",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), Some(expected));

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();

        assert!(app.world().get::<WidgetFocusVisible>(widget).is_some());
        let expected = VisualSlotOverride {
            fill_color: Some(PRESS_FILL),
            border_color: Some(FOCUS_BORDER),
            ..VisualSlotOverride::default()
        };
        assert_eq!(
            root_override(&app, widget),
            Some(expected.clone()),
            "keyboard traversal must reveal the indicator without changing the focused widget",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), Some(expected));
    }

    /// One application focus operation held until [`trigger_queued_focus_change`]
    /// takes it inside `Update`.
    #[derive(Clone, Copy)]
    enum FocusChange {
        Request { window: Entity, widget: Entity },
        Clear { window: Entity },
    }

    #[derive(Default, Resource)]
    struct QueuedFocusChange(Option<FocusChange>);

    /// Triggers the queued [`FocusChange`] from inside `Update`. Scheduled after
    /// `WidgetSystems::SemanticInput` and before
    /// `WidgetSystems::FocusCommandsApplied`, so the `RequestWidgetFocus` /
    /// `ClearWidgetFocus` observer commands are applied by the production focus
    /// fence rather than a test-side `World::flush`.
    fn trigger_queued_focus_change(
        mut queued_focus_change: ResMut<QueuedFocusChange>,
        mut commands: Commands,
    ) {
        match queued_focus_change.0.take() {
            Some(FocusChange::Request { window, widget }) => {
                commands.trigger(RequestWidgetFocus { window, widget });
            },
            Some(FocusChange::Clear { window }) => {
                commands.trigger(ClearWidgetFocus { window });
            },
            None => {},
        }
    }

    fn queue_focus_change(app: &mut App, focus_change: FocusChange) {
        app.world_mut().resource_mut::<QueuedFocusChange>().0 = Some(focus_change);
    }

    #[test]
    fn application_focus_request_is_visible_to_presentation_in_the_same_frame() {
        let mut app = integrated_test_app();
        app.init_resource::<QueuedFocusChange>().add_systems(
            Update,
            trigger_queued_focus_change
                .after(WidgetSystems::SemanticInput)
                .before(WidgetSystems::FocusCommandsApplied),
        );
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.focused(Appearance::new().border_color(FOCUS_BORDER))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert_eq!(root_override(&app, widget), None);

        queue_focus_change(&mut app, FocusChange::Request { window, widget });
        app.update();

        let expected = VisualSlotOverride {
            border_color: Some(FOCUS_BORDER),
            ..VisualSlotOverride::default()
        };
        assert_eq!(
            root_override(&app, widget),
            Some(expected.clone()),
            "application focus must restyle in the same update that triggers the request",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), Some(expected));

        queue_focus_change(&mut app, FocusChange::Clear { window });
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "application focus clear must restyle in the same update that triggers it",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), None);
    }

    #[test]
    fn disabled_presentation_adds_no_second_panel_or_computed_change() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.disabled(
                    Appearance::new()
                        .background(DISABLED_FILL)
                        .border_color(DISABLED_BORDER),
                )
            }),
        );
        app.update();
        app.update();
        let widget = resolve_widget(&mut app, panel);

        // A panel-level cascade edge restyles without touching the panel or
        // its computed output at all.
        let before = computed_tick(&app, panel);
        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.world_mut().flush();
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert_eq!(
            computed_tick(&app, panel),
            before,
            "a cascade disabled edge must not refresh the computed panel",
        );
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(DISABLED_FILL),
                border_color: Some(DISABLED_BORDER),
                ..VisualSlotOverride::default()
            }),
        );

        // A widget-local authored edit performs its one visual-only computed
        // refresh; presentation then adds no second change.
        app.world_mut()
            .commands()
            .entity(panel)
            .inherit_widget_interactivity();
        app.world_mut().flush();
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_none());
        let before = computed_tick(&app, panel);
        let result =
            app.world_mut()
                .run_system_once(move |mut writer: crate::PanelWidgetWriter| {
                    writer.override_interactivity(widget, WidgetInteractivity::Disabled)
                });
        assert_eq!(result.ok(), Some(true));
        app.update();
        let after_edit = computed_tick(&app, panel);
        assert_ne!(
            after_edit, before,
            "the authored edit performs its Phase 2 visual-only refresh",
        );
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert_eq!(
            root_override(&app, widget).and_then(|value| value.fill_color),
            Some(DISABLED_FILL),
        );
        assert_eq!(
            computed_tick(&app, panel),
            after_edit,
            "presentation must not add another panel/computed change",
        );
    }

    #[test]
    fn repeated_identical_state_leaves_the_override_tick_unchanged() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        let tick = app
            .world()
            .entity(widget)
            .get_ref::<WidgetVisualOverrides>()
            .map(|overrides| overrides.last_changed());
        assert!(tick.is_some());

        app.update();
        app.update();
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<WidgetVisualOverrides>()
                .map(|overrides| overrides.last_changed()),
            tick,
            "an unchanged state must not mark the override component changed",
        );
    }

    #[test]
    fn button_state_edges_update_overrides_once_then_leave_quiet_frames_unchanged() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .hovered(Appearance::new().background(HOVER_FILL))
                    .pressed(Appearance::new().background(PRESS_FILL))
                    .focused(Appearance::new().background(FOCUS_FILL))
                    .disabled(Appearance::new().background(DISABLED_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        // Hover insertion writes the authored hovered layer, then the next
        // frame remains quiet.
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        let hovered_tick = override_tick(&app, widget);
        assert!(hovered_tick.is_some());
        assert_override_quiet(&mut app, widget, hovered_tick);

        // Both aggregate values mean hovered in the state resolver, so this
        // input-value edge legitimately preserves the resolved override.
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Pressed);
        app.update();
        assert_eq!(override_tick(&app, widget), hovered_tick);
        assert_override_quiet(&mut app, widget, hovered_tick);

        // The private press marker supplies the distinct pressed appearance.
        app.world_mut().entity_mut(widget).insert(ButtonPress);
        app.update();
        let pressed_tick = override_tick(&app, widget);
        assert!(pressed_tick.is_some());
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(PRESS_FILL),
        );
        assert_ne!(pressed_tick, hovered_tick);
        assert_override_quiet(&mut app, widget, pressed_tick);

        app.world_mut().entity_mut(widget).remove::<ButtonPress>();
        app.update();
        let restored_hover_tick = override_tick(&app, widget);
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(HOVER_FILL),
        );
        assert_ne!(restored_hover_tick, pressed_tick);
        assert_override_quiet(&mut app, widget, restored_hover_tick);

        app.world_mut()
            .entity_mut(widget)
            .remove::<PickingInteraction>();
        app.update();
        assert_eq!(override_tick(&app, widget), None);
        assert_override_quiet(&mut app, widget, None);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();
        let focused_tick = override_tick(&app, widget);
        assert!(focused_tick.is_some());
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(FOCUS_FILL),
        );
        assert_override_quiet(&mut app, widget, focused_tick);

        app.world_mut().trigger(ClearWidgetFocus { window });
        app.world_mut().flush();
        app.update();
        assert_eq!(override_tick(&app, widget), None);
        assert_override_quiet(&mut app, widget, None);

        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.world_mut().flush();
        app.update();
        app.update();
        let disabled_tick = override_tick(&app, widget);
        assert!(disabled_tick.is_some());
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(DISABLED_FILL),
        );
        assert_override_quiet(&mut app, widget, disabled_tick);

        app.world_mut()
            .commands()
            .entity(panel)
            .inherit_widget_interactivity();
        app.world_mut().flush();
        app.update();
        app.update();
        assert_eq!(override_tick(&app, widget), None);
        assert_override_quiet(&mut app, widget, None);
    }

    #[test]
    fn interaction_removal_presents_the_edge_back_to_normal() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(root_override(&app, widget).is_some());

        app.world_mut()
            .entity_mut(widget)
            .remove::<PickingInteraction>();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "an aggregate removal must clear the stale hover override",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), None);
    }

    #[test]
    fn state_overrides_leave_unrelated_slots_and_widgets_untouched() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            two_styled_buttons_tree(HOVER_FILL, PEER_HOVER_FILL),
        );
        app.update();
        let first = widget_by_id(&mut app, panel, "first");
        let second = widget_by_id(&mut app, panel, "second");

        app.world_mut()
            .entity_mut(first)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert!(root_override(&app, first).is_some());
        assert_eq!(
            root_override(&app, second),
            None,
            "an unhovered peer button must stay unstyled",
        );
        assert_eq!(indexed_root_override(&app, panel, second), None);
        let first_index = root_element_index(&app, first);
        let second_index = root_element_index(&app, second);
        assert_ne!(first_index, second_index);
    }

    #[test]
    fn state_edges_do_not_rebuild_same_kind_peer_overrides() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            two_styled_buttons_tree(HOVER_FILL, PEER_HOVER_FILL),
        );
        app.update();
        let first = widget_by_id(&mut app, panel, "first");
        let second = widget_by_id(&mut app, panel, "second");

        app.world_mut()
            .entity_mut(first)
            .insert(PickingInteraction::Hovered);
        app.world_mut()
            .entity_mut(second)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(app.world().get::<WidgetVisualOverrides>(first).is_some());
        assert!(app.world().get::<WidgetVisualOverrides>(second).is_some());

        app.world_mut()
            .entity_mut(second)
            .remove::<WidgetVisualOverrides>();
        app.world_mut()
            .entity_mut(first)
            .insert(PickingInteraction::Pressed);
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(second).is_none(),
            "a changed interaction on one button must not rebuild its peer override",
        );

        app.world_mut()
            .entity_mut(first)
            .remove::<PickingInteraction>();
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(second).is_none(),
            "an interaction removal on one button must not rebuild its peer override",
        );
    }

    #[test]
    fn on_click_fires_once_and_keeps_its_system_through_state_changes() {
        let mut app = integrated_test_app();
        app.init_resource::<CallbackClicks>();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .on_click(record_callback_click)
                    .hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let system_entity = app
            .world()
            .get::<super::ButtonCallbackHandle>(widget)
            .map(super::ButtonCallbackHandle::system_entity);
        assert!(system_entity.is_some());

        app.world_mut().trigger(ButtonClicked {
            entity:     widget,
            id:         PanelElementId::named(BUTTON_ID),
            pointer_id: None,
        });
        app.update();
        assert_eq!(app.world().resource::<CallbackClicks>().0.len(), 1);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(root_override(&app, widget).is_some());
        app.world_mut().trigger(ButtonClicked {
            entity:     widget,
            id:         PanelElementId::named(BUTTON_ID),
            pointer_id: None,
        });
        app.update();
        assert_eq!(
            app.world().resource::<CallbackClicks>().0.len(),
            2,
            "each click runs the callback exactly once through state changes",
        );
        assert_eq!(
            app.world()
                .get::<super::ButtonCallbackHandle>(widget)
                .map(super::ButtonCallbackHandle::system_entity),
            system_entity,
            "presentation state changes must not replace the tracked system",
        );
    }
}
