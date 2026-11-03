use std::collections::HashMap;

use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::world::DeferredWorld;
use bevy::picking::PickingSettings;
use bevy::picking::events::PointerState;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::Add;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Despawn;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Has;
use bevy::prelude::On;
use bevy::prelude::PointerButton;
use bevy::prelude::PointerCancel;
use bevy::prelude::PointerClick;
use bevy::prelude::PointerDragEnd;
use bevy::prelude::PointerOut;
use bevy::prelude::PointerOver;
use bevy::prelude::PointerPress;
use bevy::prelude::PointerRelease;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::warn;

use super::events::ButtonCancelCause;
use super::events::ButtonCanceled;
use super::events::ButtonClicked;
use super::events::ButtonPressSource;
use super::events::ButtonPressed;
use super::events::ButtonReleased;
use crate::PanelElementId;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::ime;
use crate::ime::ImeBlurIntent;
use crate::ime::ImeEditorState;
use crate::widgets::PanelWidget;
use crate::widgets::SemanticWidgetIntent;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetFocusChanged;
use crate::widgets::WidgetFocusTarget;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;
use crate::widgets::interaction as capture;
use crate::widgets::interaction::WidgetCaptures;

/// The retained state of a toggle button.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub enum ButtonLatch {
    /// The toggle is at rest.
    Released,
    /// The toggle remains pressed until activated or released from code.
    Pressed,
}

/// Releases a toggle button without emitting [`ButtonClicked`].
pub fn release_button(commands: &mut Commands<'_, '_>, entity: Entity) {
    commands.queue(move |world: &mut World| {
        if let Some(mut latch) = world.get_mut::<ButtonLatch>(entity)
            && *latch != ButtonLatch::Released
        {
            *latch = ButtonLatch::Released;
        }
    });
}

#[derive(Component)]
#[component(
    on_remove = emit_button_terminal,
    on_despawn = emit_button_terminal
)]
pub(crate) struct ButtonPress;

/// Whether the pointer owning a temporary button press remains over its widget.
#[derive(Component)]
pub(in crate::widgets) struct ButtonPointerInside(pub bool);

pub(in crate::widgets::controls::button) struct PendingButtonPress {
    id:       PanelElementId,
    holder:   ButtonPressHolder,
    terminal: ButtonTerminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::widgets::controls::button) enum ButtonPressHolder {
    Pointer(PointerId),
    Semantic { window: Entity },
}

impl ButtonPressHolder {
    const fn source(self) -> ButtonPressSource {
        match self {
            Self::Pointer(pointer_id) => ButtonPressSource::Pointer(pointer_id),
            Self::Semantic { .. } => ButtonPressSource::Semantic,
        }
    }
}

impl PendingButtonPress {
    const fn new(id: PanelElementId, holder: ButtonPressHolder) -> Self {
        Self {
            id,
            holder,
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

/// Pending button-press payloads keyed by widget entity.
///
/// Pointer/widget occupancy and raw-action ordering live in the shared
/// [`WidgetCaptures`] for pointer presses. Semantic presses do not occupy a
/// pointer; the pending entry arbitrates between both input paths before the
/// deferred [`ButtonPress`] marker is inserted.
#[derive(Default, Resource)]
pub(crate) struct ButtonCaptures {
    presses: HashMap<Entity, PendingButtonPress>,
}

impl ButtonCaptures {
    /// The pointer owning this button's temporary press, excluding semantic holds.
    pub(super) fn pointer(&self, entity: Entity) -> Option<PointerId> {
        match self.presses.get(&entity)?.holder {
            ButtonPressHolder::Pointer(pointer) => Some(pointer),
            ButtonPressHolder::Semantic { .. } => None,
        }
    }

    pub(in crate::widgets::controls::button) fn begin_press(
        &mut self,
        entity: Entity,
        id: PanelElementId,
        holder: ButtonPressHolder,
    ) {
        self.presses
            .insert(entity, PendingButtonPress::new(id, holder));
    }

    fn press_mut(&mut self, entity: Entity) -> Option<&mut PendingButtonPress> {
        self.presses.get_mut(&entity)
    }

    fn cancel(&mut self, entity: Entity, cause: ButtonCancelCause) -> bool {
        self.presses
            .get_mut(&entity)
            .is_some_and(|press| press.cancel(cause))
    }

    pub(in crate::widgets::controls::button) fn take(
        &mut self,
        entity: Entity,
    ) -> Option<PendingButtonPress> {
        self.presses.remove(&entity)
    }
}

pub(in crate::widgets) fn press_from_pointer(
    mut press: On<PointerPress>,
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
    let Some(sequence) = captures.observe_press(press.pointer.id, entity) else {
        return;
    };
    if disabled
        || pressed
        || button_captures.presses.contains_key(&entity)
        || !captures.try_capture(press.pointer.id, entity, sequence)
    {
        return;
    }

    button_captures.begin_press(
        entity,
        widget.id().clone(),
        ButtonPressHolder::Pointer(press.pointer.id),
    );
    commands
        .entity(entity)
        .insert((ButtonPress, ButtonPointerInside(true)));
    commands.trigger(ButtonPressed {
        entity,
        id: widget.id().clone(),
        source: ButtonPressSource::Pointer(press.pointer.id),
    });
}

pub(in crate::widgets) fn pointer_over_button(
    over: On<PointerOver>,
    captures: Res<ButtonCaptures>,
    mut inside: Query<&mut ButtonPointerInside>,
) {
    set_pointer_inside(
        over.event_target(),
        over.pointer.id,
        true,
        &captures,
        &mut inside,
    );
}

pub(in crate::widgets) fn pointer_out_of_button(
    out: On<PointerOut>,
    captures: Res<ButtonCaptures>,
    mut inside: Query<&mut ButtonPointerInside>,
) {
    set_pointer_inside(
        out.event_target(),
        out.pointer.id,
        false,
        &captures,
        &mut inside,
    );
}

fn set_pointer_inside(
    entity: Entity,
    pointer: PointerId,
    is_inside: bool,
    captures: &ButtonCaptures,
    inside: &mut Query<&mut ButtonPointerInside>,
) {
    if captures.pointer(entity) == Some(pointer)
        && let Ok(mut state) = inside.get_mut(entity)
        && state.0 != is_inside
    {
        state.0 = is_inside;
    }
}

pub(in crate::widgets) fn click_from_pointer(
    mut click: On<PointerClick>,
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
    if disabled || !captures.captures(click.pointer.id, entity) {
        return;
    }
    if let Some(button_press) = button_captures.press_mut(entity) {
        button_press.release(ButtonReleaseOutcome::Clicked);
    }
}

pub(in crate::widgets) fn release_from_pointer(
    mut release: On<PointerRelease>,
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
    if !captures.captures(release.pointer.id, entity) {
        return;
    }
    let Some(button_press) = button_captures.press_mut(entity) else {
        return;
    };
    button_press.release(ButtonReleaseOutcome::WithoutClick);
    commands.entity(entity).remove::<ButtonPress>();
}

pub(in crate::widgets) fn cancel_from_pointer(
    mut cancel: On<PointerCancel>,
    captures: Res<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    let entity = cancel.event_target();
    if captures.captures(cancel.pointer.id, entity) {
        cancel.propagate(false);
        cancel_button_press(
            entity,
            ButtonCancelCause::PointerCanceled,
            &mut button_captures,
            &mut commands,
        );
    }
}

pub(in crate::widgets) fn cancel_from_drag_end(
    mut drag_end: On<PointerDragEnd>,
    captures: Res<WidgetCaptures>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    let entity = drag_end.event_target();
    if drag_end.button == PointerButton::Primary && captures.captures(drag_end.pointer.id, entity) {
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
pub(in crate::widgets) fn apply_raw_release(
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

pub(in crate::widgets) fn capture_reconciled_button_press(
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
        || world
            .resource::<ButtonCaptures>()
            .presses
            .contains_key(&entity)
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

    world.resource_mut::<ButtonCaptures>().begin_press(
        entity,
        id.clone(),
        ButtonPressHolder::Pointer(pointer_id),
    );
    world
        .entity_mut(entity)
        .insert((ButtonPress, ButtonPointerInside(true)));
    world.trigger(ButtonPressed {
        entity,
        id,
        source: ButtonPressSource::Pointer(pointer_id),
    });
}

pub(in crate::widgets) fn cancel_from_pointer_removal(
    removed: On<Remove<PointerId>>,
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

pub(in crate::widgets) fn cancel_from_disabled(
    disabled: On<Add<WidgetDisabled>>,
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

pub(in crate::widgets) fn cancel_from_widget_removal(
    removed: On<Remove<PanelWidget>>,
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

pub(in crate::widgets) fn cancel_before_widget_despawn(
    despawn: On<Despawn<PanelWidget>>,
    mut captures: ResMut<ButtonCaptures>,
) {
    captures.cancel(despawn.entity, ButtonCancelCause::WidgetRemoved);
}

pub(in crate::widgets) fn handle_semantic_intent(
    intent: On<SemanticWidgetIntent>,
    widgets: Query<(
        &PanelWidget,
        &WidgetKind,
        Has<WidgetDisabled>,
        Has<ButtonPress>,
    )>,
    mut captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    let entity = intent.event_target();
    let Ok((widget, kind, disabled, pressed)) = widgets.get(entity) else {
        return;
    };
    if *kind != WidgetKind::Button {
        return;
    }
    match intent.event() {
        SemanticWidgetIntent::Activate { window, .. } => {
            if disabled || pressed || captures.presses.contains_key(&entity) {
                return;
            }
            captures.begin_press(
                entity,
                widget.id().clone(),
                ButtonPressHolder::Semantic { window: *window },
            );
            commands.entity(entity).insert(ButtonPress);
            commands.trigger(ButtonPressed {
                entity,
                id: widget.id().clone(),
                source: ButtonPressSource::Semantic,
            });
        },
        SemanticWidgetIntent::ActivateRelease { window, .. } => {
            if let Some(press) = captures.press_mut(entity)
                && press.holder == (ButtonPressHolder::Semantic { window: *window })
                && press.release(ButtonReleaseOutcome::Clicked)
            {
                commands.entity(entity).remove::<ButtonPress>();
            }
        },
        SemanticWidgetIntent::ActivateInterrupted { window, .. } => {
            if captures.presses.get(&entity).is_some_and(|press| {
                press.holder == (ButtonPressHolder::Semantic { window: *window })
            }) {
                cancel_button_press(
                    entity,
                    ButtonCancelCause::InputInterrupted,
                    &mut captures,
                    &mut commands,
                );
            }
        },
        SemanticWidgetIntent::Cancel { .. } => {
            cancel_button_press(
                entity,
                ButtonCancelCause::Explicit,
                &mut captures,
                &mut commands,
            );
        },
        SemanticWidgetIntent::Step { .. } => {},
    }
}

pub(in crate::widgets) fn cancel_from_focus_loss(
    change: On<WidgetFocusChanged>,
    mut captures: ResMut<ButtonCaptures>,
    mut commands: Commands,
) {
    let WidgetFocusTarget::Widget(entity) = change.previous else {
        return;
    };
    if captures.presses.get(&entity).is_some_and(|press| {
        press.holder
            == (ButtonPressHolder::Semantic {
                window: change.window,
            })
    }) {
        cancel_button_press(
            entity,
            ButtonCancelCause::FocusLost,
            &mut captures,
            &mut commands,
        );
    }
}

fn emit_button_terminal(mut world: DeferredWorld, context: HookContext) {
    let entity = context.entity;
    let Some(button_press) = world
        .get_resource_mut::<ButtonCaptures>()
        .and_then(|mut captures| captures.take(entity))
    else {
        return;
    };
    let (id, source, terminal) = (
        button_press.id,
        button_press.holder.source(),
        button_press.terminal,
    );
    if let ButtonPressSource::Pointer(pointer_id) = source {
        let released = world
            .get_resource_mut::<WidgetCaptures>()
            .and_then(|mut captures| captures.release_widget(entity));
        if released != Some(pointer_id) {
            warn!(
                target: DIAGNOSTIC_LOG_TARGET, "button {:?} ({entity}) has a pending pointer press but no matching capture owner; skipping terminal events",
                id
            );
            return;
        }
    }

    match terminal {
        ButtonTerminal::Release(outcome) => {
            capture::trigger_immediate(
                &mut world,
                ButtonReleased {
                    entity,
                    id: id.clone(),
                    source,
                },
            );
            if outcome == ButtonReleaseOutcome::Clicked {
                if let Some(mut latch) = world.get_mut::<ButtonLatch>(entity) {
                    *latch = next_latch(*latch);
                }
                capture::trigger_immediate(
                    &mut world,
                    ButtonClicked {
                        entity,
                        id,
                        source: source.into(),
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
                    source,
                    cause,
                },
            );
        },
    }
}

const fn next_latch(latch: ButtonLatch) -> ButtonLatch {
    match latch {
        ButtonLatch::Released => ButtonLatch::Pressed,
        ButtonLatch::Pressed => ButtonLatch::Released,
    }
}

/// Records a button cancellation and, when newly recorded, removes
/// [`ButtonPress`] so its hook emits the terminal. Returns whether a terminal
/// was recorded, so the shared dispatcher marks the pointer freed only when
/// shared occupancy will be released.
pub(in crate::widgets) fn cancel_button_press(
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
    use bevy::picking::hover::PreviousHoverMap;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerAction;
    use bevy::picking::pointer::PointerId;
    use bevy::picking::pointer::PointerInput;
    use bevy::picking::pointer::PointerLocation;
    use bevy::picking::pointer::PointerMap;
    use bevy::prelude::App;
    use bevy::prelude::Camera;
    use bevy::prelude::Commands;
    use bevy::prelude::Entity;
    use bevy::prelude::EntityEvent;
    use bevy::prelude::First;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::In;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PointerCancel;
    use bevy::prelude::PointerClick;
    use bevy::prelude::PointerDragEnd;
    use bevy::prelude::PointerPress;
    use bevy::prelude::PointerRelease;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::Query;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::prelude::default;
    use bevy::window::Ime;
    use bevy::window::WindowClosed;
    use bevy::window::WindowFocused;
    use bevy::window::WindowRef;
    use hana_valence::AnchorSite;
    use hana_valence::AnchoredHere;
    use hana_valence::AnchoredTo;

    use super::ButtonPressHolder;
    use super::ButtonTerminal;
    use super::cancel_before_widget_despawn;
    use super::cancel_from_disabled;
    use super::cancel_from_drag_end;
    use super::cancel_from_focus_loss;
    use super::cancel_from_pointer;
    use super::cancel_from_pointer_removal;
    use super::cancel_from_widget_removal;
    use super::click_from_pointer;
    use super::handle_semantic_intent;
    use super::press_from_pointer;
    use super::release_button;
    use super::release_from_pointer;
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
    use crate::widgets::WidgetFocusChangeCause;
    use crate::widgets::WidgetFocusChanged;
    use crate::widgets::WidgetFocusTarget;
    use crate::widgets::WidgetKind;
    use crate::widgets::WidgetOf;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::controls::SliderCaptures;
    use crate::widgets::controls::button::ButtonActivationSource;
    use crate::widgets::controls::button::ButtonCancelCause;
    use crate::widgets::controls::button::ButtonCanceled;
    use crate::widgets::controls::button::ButtonCaptures;
    use crate::widgets::controls::button::ButtonClicked;
    use crate::widgets::controls::button::ButtonLatch;
    use crate::widgets::controls::button::ButtonPress;
    use crate::widgets::controls::button::ButtonPressSource;
    use crate::widgets::controls::button::ButtonPressed;
    use crate::widgets::controls::button::ButtonReleased;
    use crate::widgets::interaction::CapturedWidget;
    use crate::widgets::interaction::WidgetCaptures;

    const BUTTON_ID: &str = "action";
    const FIELD_ID: &str = "field";

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedButtonEvent {
        Pressed(ButtonPressSource),
        Released(ButtonPressSource),
        Clicked(ButtonActivationSource),
        Canceled(ButtonPressSource, ButtonCancelCause),
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
    struct CallbackClicks(Vec<(Entity, ButtonActivationSource)>);

    #[derive(Default, Resource)]
    struct ScopedClicks(Vec<Entity>);

    #[derive(Default, Resource)]
    struct ObservedToggleClicks(Vec<(ButtonActivationSource, Option<ButtonLatch>)>);

    fn observe_toggle_click(
        click: On<ButtonClicked>,
        latches: Query<&ButtonLatch>,
        mut observed: ResMut<ObservedToggleClicks>,
    ) {
        observed
            .0
            .push((click.source, latches.get(click.entity).ok().copied()));
    }

    #[derive(Default, Resource)]
    struct ImeObservation {
        starts:        usize,
        opens:         usize,
        commit_causes: Vec<ImeCommitCause>,
    }

    fn record_pressed(event: On<ButtonPressed>, mut events: ResMut<RecordedButtonEvents>) {
        events.0.push(RecordedButtonEvent::Pressed(event.source));
    }

    fn record_released(event: On<ButtonReleased>, mut events: ResMut<RecordedButtonEvents>) {
        events.0.push(RecordedButtonEvent::Released(event.source));
    }

    fn record_clicked(event: On<ButtonClicked>, mut events: ResMut<RecordedButtonEvents>) {
        events.0.push(RecordedButtonEvent::Clicked(event.source));
    }

    fn record_canceled(event: On<ButtonCanceled>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Canceled(event.source, event.cause));
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

    fn record_panel_click(_event: On<PointerClick>, mut clicks: ResMut<PanelClicks>) {
        clicks.0 += 1;
    }

    fn record_callback_click(click: In<ButtonClicked>, mut clicks: ResMut<CallbackClicks>) {
        clicks.0.push((click.entity, click.source));
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
            .init_resource::<crate::FineAdjustment>()
            .init_resource::<RecordedButtonEvents>()
            .init_resource::<HoverMap>()
            .init_resource::<PickingSettings>()
            .init_resource::<PointerState>()
            .init_resource::<PointerMap>()
            .add_message::<PointerInput>()
            .add_message::<PointerPress>()
            .add_observer(press_from_pointer)
            .add_observer(click_from_pointer)
            .add_observer(release_from_pointer)
            .add_observer(cancel_from_pointer)
            .add_observer(cancel_from_drag_end)
            .add_observer(cancel_from_pointer_removal)
            .add_observer(cancel_from_disabled)
            .add_observer(cancel_from_focus_loss)
            .add_observer(cancel_from_widget_removal)
            .add_observer(cancel_before_widget_despawn)
            .add_observer(handle_semantic_intent)
            .add_observer(record_pressed)
            .add_observer(record_released)
            .add_observer(record_clicked)
            .add_observer(record_canceled)
            .add_systems(
                PreUpdate,
                crate::widgets::interaction::reconcile_pointer_input.in_set(PickingSystems::Last),
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
        crate::test_support::configure_click_interval(&mut app, 2);
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
            .run_system_cached(crate::widgets::interaction::reconcile_pointer_input);
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
        builder.with(
            El::new().editable_field(FIELD_ID, field_spec(), crate::TextStyle::new(10.0)),
            |_| {},
        );
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
                .editable_field(FIELD_ID, field_spec(), crate::TextStyle::new(10.0)),
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
        app.world_mut().trigger(PointerPress {
            entity:  widget,
            pointer: Pointer::new(pointer_id, location()),
            button:  PointerButton::Primary,
            hit:     hit(),
            count:   1,
        });
        app.world_mut().flush();
    }

    fn click(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(PointerClick {
            entity:   widget,
            pointer:  Pointer::new(pointer_id, location()),
            button:   PointerButton::Primary,
            hit:      hit(),
            duration: std::time::Duration::ZERO,
            count:    1,
        });
        app.world_mut().flush();
    }

    fn double_click(
        app: &mut App,
        target: Entity,
        pointer_id: PointerId,
        location: Location,
        hit: HitData,
    ) {
        app.world_mut().trigger(PointerClick {
            entity: target,
            pointer: Pointer::new(pointer_id, location),
            button: PointerButton::Primary,
            hit,
            duration: std::time::Duration::ZERO,
            count: 2,
        });
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
        app.world_mut().trigger(PointerRelease {
            entity:  widget,
            pointer: Pointer::new(pointer_id, location()),
            button:  PointerButton::Primary,
            hit:     hit(),
        });
        app.world_mut().flush();
    }

    fn cancel_pointer(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(PointerCancel {
            entity:  widget,
            pointer: Pointer::new(pointer_id, location()),
            hit:     hit(),
        });
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

    fn assert_semantic_capture(app: &App, widget: Entity, window: Entity) {
        assert!(app.world().resource::<WidgetCaptures>().is_empty());
        assert_eq!(
            app.world().resource::<ButtonCaptures>().presses[&widget].holder,
            ButtonPressHolder::Semantic { window },
        );
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
    fn a_momentary_button_is_unchanged() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(7);

        press(&mut app, widget, pointer_id);
        click(&mut app, widget, pointer_id);
        release(&mut app, widget, pointer_id);

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(pointer_id)),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert!(app.world().get::<ButtonLatch>(widget).is_none());
        assert_no_captures(&app);
    }

    #[test]
    fn ordered_press_click_release_observes_the_pending_press() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let pointer_id = PointerId::Touch(9);

        {
            let mut commands = app.world_mut().commands();
            commands.trigger(PointerPress {
                entity:  widget,
                pointer: Pointer::new(pointer_id, location()),
                button:  PointerButton::Primary,
                hit:     hit(),
                count:   1,
            });
            commands.trigger(PointerClick {
                entity:   widget,
                pointer:  Pointer::new(pointer_id, location()),
                button:   PointerButton::Primary,
                hit:      hit(),
                duration: std::time::Duration::ZERO,
                count:    1,
            });
            commands.trigger(PointerRelease {
                entity:  widget,
                pointer: Pointer::new(pointer_id, location()),
                button:  PointerButton::Primary,
                hit:     hit(),
            });
        }
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(pointer_id)),
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
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(pointer_id)),
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
                ButtonPressSource::Pointer(pointer_id),
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
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(pointer_id)),
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
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
                RecordedButtonEvent::Canceled(
                    ButtonPressSource::Pointer(pointer_id),
                    ButtonCancelCause::CaptureLost
                ),
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
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
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(pointer_id)),
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
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
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(pointer_id)),
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
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
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(pointer_id)),
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
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
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(first_pointer)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(first_pointer)),
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(second_pointer)),
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
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(first_pointer)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(first_pointer)),
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
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
            .run_system_cached(crate::widgets::interaction::reconcile_pointer_input);
        assert!(result.is_ok());

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                ButtonPressSource::Pointer(pointer_id),
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
        // `PointerCancel`). The lost-widget fallback must keep the concrete
        // `PointerCanceled` cause rather than degrading a cancel to `CaptureLost`.
        set_hover_maps(&mut app, pointer_id, &[], &[]);
        run_pointer_actions(&mut app, pointer_id, [PointerAction::Cancel]);

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                ButtonPressSource::Pointer(pointer_id),
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
        // targeted `PointerRelease`). A kind-lost release keeps `CaptureLost`.
        set_hover_maps(&mut app, pointer_id, &[], &[]);
        run_pointer_actions(
            &mut app,
            pointer_id,
            [PointerAction::Release(PointerButton::Primary)],
        );

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                ButtonPressSource::Pointer(pointer_id),
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

        assert_eq!(events(&app), []);
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
            .run_system_cached(crate::widgets::interaction::reconcile_pointer_input);
        assert!(result.is_ok());

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                ButtonPressSource::Pointer(pointer_id),
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
            commands.trigger(PointerClick {
                entity:   widget,
                pointer:  Pointer::new(pointer_id, location()),
                button:   PointerButton::Primary,
                hit:      hit(),
                duration: std::time::Duration::ZERO,
                count:    1,
            });
            commands.trigger(PointerRelease {
                entity:  widget,
                pointer: Pointer::new(pointer_id, location()),
                button:  PointerButton::Primary,
                hit:     hit(),
            });
            commands.trigger(PointerDragEnd {
                entity:   widget,
                pointer:  Pointer::new(pointer_id, location()),
                button:   PointerButton::Primary,
                distance: Vec2::ONE,
            });
        }
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Pointer(pointer_id)),
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
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
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
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(
                captured
            ))]
        );

        release(&mut app, widget, captured);
        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(captured)),
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(captured)),
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
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(
                pointer_id
            ))]
        );

        release(&mut app, captured_widget, pointer_id);
        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
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
        assert_eq!(events(&app), []);
        assert!(app.world().get::<ButtonPress>(other_widget).is_none());
        assert_pending_capture(&app, captured_widget, captured);

        release(&mut app, captured_widget, captured);
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Released(ButtonPressSource::Pointer(
                captured
            ))]
        );
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
                ButtonPressSource::Pointer(pointer_id),
                ButtonCancelCause::PointerCanceled,
            )]
        );
        app.update();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                ButtonPressSource::Pointer(pointer_id),
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
                RecordedButtonEvent::Released(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
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
                RecordedButtonEvent::Canceled(
                    ButtonPressSource::Pointer(pointer_id),
                    ButtonCancelCause::PointerCanceled,
                ),
                RecordedButtonEvent::Pressed(ButtonPressSource::Pointer(pointer_id)),
                RecordedButtonEvent::Canceled(
                    ButtonPressSource::Pointer(pointer_id),
                    ButtonCancelCause::PointerCanceled,
                ),
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
        app.world_mut().trigger(PointerDragEnd {
            entity:   widget,
            pointer:  Pointer::new(pointer_id, location()),
            button:   PointerButton::Primary,
            distance: Vec2::ONE,
        });
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Canceled(
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
                ButtonCancelCause::Explicit,
            )]
        );
        app.world_mut()
            .trigger(SemanticWidgetIntent::Cancel { entity: widget });
        app.world_mut().flush();
        assert_eq!(events(&app).len(), 1);
    }

    #[test]
    fn semantic_activation_uses_press_release_and_pointerless_click() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        });
        app.world_mut().flush();
        app.world_mut()
            .trigger(SemanticWidgetIntent::ActivateRelease {
                entity: widget,
                window: Entity::PLACEHOLDER,
            });
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Semantic),
                RecordedButtonEvent::Released(ButtonPressSource::Semantic),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Semantic),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
    }

    #[test]
    fn pointer_press_before_semantic_commands_flush_preserves_semantic_holder() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let window = app.world_mut().spawn_empty().id();

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window,
        });
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        app.world_mut().trigger(PointerPress {
            entity:  widget,
            pointer: Pointer::new(PointerId::Mouse, location()),
            button:  PointerButton::Primary,
            hit:     hit(),
            count:   1,
        });
        assert_eq!(
            app.world().resource::<ButtonCaptures>().presses[&widget].holder,
            ButtonPressHolder::Semantic { window },
        );
        app.world_mut().flush();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Pressed(ButtonPressSource::Semantic)],
        );
        assert_semantic_capture(&app, widget, window);
    }

    #[test]
    fn semantic_press_rejects_pointer_and_repeat_then_interrupts_once() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        let activate = SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        };

        app.world_mut().trigger(activate);
        app.world_mut().flush();
        app.world_mut().trigger(activate);
        press(&mut app, widget, PointerId::Mouse);
        app.world_mut().flush();
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Pressed(ButtonPressSource::Semantic)]
        );
        assert_semantic_capture(&app, widget, Entity::PLACEHOLDER);

        app.world_mut()
            .trigger(SemanticWidgetIntent::ActivateInterrupted {
                entity: widget,
                window: Entity::PLACEHOLDER,
            });
        app.world_mut().flush();
        app.world_mut()
            .trigger(SemanticWidgetIntent::ActivateRelease {
                entity: widget,
                window: Entity::PLACEHOLDER,
            });
        app.world_mut().flush();
        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Semantic),
                RecordedButtonEvent::Canceled(
                    ButtonPressSource::Semantic,
                    ButtonCancelCause::InputInterrupted,
                ),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
    }

    #[test]
    fn focus_loss_cancels_a_semantic_press_once() {
        let mut app = test_app();
        let widget = spawn_button(&mut app);
        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        });
        app.world_mut().flush();

        app.world_mut().trigger(WidgetFocusChanged {
            window:   Entity::PLACEHOLDER,
            previous: WidgetFocusTarget::Widget(widget),
            current:  WidgetFocusTarget::Unfocused,
            cause:    WidgetFocusChangeCause::ScopeLost,
        });
        app.world_mut().flush();
        app.world_mut()
            .trigger(SemanticWidgetIntent::ActivateInterrupted {
                entity: widget,
                window: Entity::PLACEHOLDER,
            });
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Semantic),
                RecordedButtonEvent::Canceled(
                    ButtonPressSource::Semantic,
                    ButtonCancelCause::FocusLost,
                ),
            ]
        );
    }

    #[test]
    fn a_pointer_click_reports_its_pointer_and_a_semantic_click_reports_semantic() {
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
        app.world_mut()
            .trigger(SemanticWidgetIntent::ActivateRelease {
                entity: widget,
                window: Entity::PLACEHOLDER,
            });
        app.world_mut().flush();

        assert_eq!(
            app.world().resource::<CallbackClicks>().0,
            [
                (widget, ButtonActivationSource::Pointer(pointer_id)),
                (widget, ButtonActivationSource::Semantic),
            ]
        );
    }

    #[test]
    fn toggle_clicks_update_latch_before_observers_and_release_emits_no_click() {
        let mut app = test_app();
        app.init_resource::<ObservedToggleClicks>()
            .add_observer(observe_toggle_click);
        let widget = spawn_button(&mut app);
        app.world_mut()
            .entity_mut(widget)
            .insert(ButtonLatch::Released);
        let pointer = PointerId::Mouse;

        press(&mut app, widget, pointer);
        click(&mut app, widget, pointer);
        release(&mut app, widget, pointer);
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Pressed)
        );

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        });
        app.world_mut().flush();
        app.world_mut()
            .trigger(SemanticWidgetIntent::ActivateRelease {
                entity: widget,
                window: Entity::PLACEHOLDER,
            });
        app.world_mut().flush();
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Released)
        );

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        });
        app.world_mut().flush();
        app.world_mut()
            .trigger(SemanticWidgetIntent::ActivateRelease {
                entity: widget,
                window: Entity::PLACEHOLDER,
            });
        app.world_mut().flush();
        assert!(
            app.world_mut()
                .run_system_once(move |mut commands: Commands| release_button(
                    &mut commands,
                    widget
                ))
                .is_ok()
        );
        app.world_mut().flush();
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Released)
        );
        assert_eq!(
            app.world().resource::<ObservedToggleClicks>().0,
            [
                (
                    ButtonActivationSource::Pointer(pointer),
                    Some(ButtonLatch::Pressed)
                ),
                (
                    ButtonActivationSource::Semantic,
                    Some(ButtonLatch::Released)
                ),
                (ButtonActivationSource::Semantic, Some(ButtonLatch::Pressed)),
            ]
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
    fn public_focused_activation_holds_then_clicks_on_release() {
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
            .write_message(WidgetInput::ActivatePress { window });
        app.update();
        app.world_mut().flush();

        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Pressed(ButtonPressSource::Semantic)]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        assert_semantic_capture(&app, widget, window);
        app.world_mut()
            .write_message(WidgetInput::ActivateRelease { window });
        app.update();
        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Semantic),
                RecordedButtonEvent::Released(ButtonPressSource::Semantic),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Semantic),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_none());
    }

    #[test]
    fn release_then_press_in_one_frame_starts_the_next_hold() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, button_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();

        app.world_mut()
            .write_message(WidgetInput::ActivatePress { window });
        app.update();
        app.world_mut()
            .write_message(WidgetInput::ActivateRelease { window });
        app.world_mut()
            .write_message(WidgetInput::ActivatePress { window });
        app.update();

        assert_eq!(
            events(&app),
            [
                RecordedButtonEvent::Pressed(ButtonPressSource::Semantic),
                RecordedButtonEvent::Released(ButtonPressSource::Semantic),
                RecordedButtonEvent::Clicked(ButtonActivationSource::Semantic),
                RecordedButtonEvent::Pressed(ButtonPressSource::Semantic),
            ]
        );
        assert!(app.world().get::<ButtonPress>(widget).is_some());
    }

    #[test]
    fn resolved_semantic_activation_rejects_disabled_marker() {
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

        assert_eq!(events(&app), []);
        assert!(app.world().get::<ButtonPress>(widget).is_none());
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
        assert_eq!(events(&app), []);

        let result = app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.override_interactivity(widget, WidgetInteractivity::Enabled)
            });
        assert_eq!(result.ok(), Some(true));
        app.update();
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        assert_eq!(events(&app), []);

        release(&mut app, widget, pointer_id);
        assert_eq!(
            events(&app),
            [RecordedButtonEvent::Released(ButtonPressSource::Pointer(
                pointer_id
            ))]
        );
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
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
                ButtonPressSource::Pointer(pointer_id),
                ButtonCancelCause::Explicit,
            )]
        );

        clear_events(&mut app);
        send_key(&mut app, window, KeyCode::Escape, ButtonState::Released);
        app.update();
        send_key(&mut app, window, KeyCode::Escape, ButtonState::Pressed);
        app.update();
        assert_eq!(events(&app), []);
    }
}
