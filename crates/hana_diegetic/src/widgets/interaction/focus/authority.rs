use std::collections::HashMap;

use bevy::platform::collections::HashSet;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::Has;
use bevy::prelude::MessageReader;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::RemovedComponents;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::WindowFocused;

use crate::ImeSessionId;
use crate::ImeTarget;
use crate::widgets::PanelWidget;

/// Marks a widget as eligible for window-scoped keyboard focus.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq, Reflect)]
#[reflect(Component)]
pub struct WidgetFocusable;

/// Marks a widget that currently owns focus in at least one window.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq, Reflect)]
#[component(immutable)]
#[reflect(Component)]
pub struct WidgetFocused;

/// Marks a widget whose retained focus should currently be drawn.
///
/// [`WidgetFocused`] remains the keyboard-input target after a pointer press,
/// while this private marker is removed. Keyboard traversal restores this
/// marker without changing the focused widget.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
#[component(immutable)]
pub(crate) struct WidgetFocusVisible(());

/// One end of a widget focus transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetFocusTarget {
    /// No widget holds focus in the window.
    Unfocused,
    /// This widget holds focus.
    Widget(Entity),
}

/// Reports a window-scoped widget focus transition.
#[derive(Clone, Copy, Debug, Event)]
pub struct WidgetFocusChanged {
    /// Window whose focus changed.
    pub window:   Entity,
    /// Widget that previously owned focus.
    pub previous: WidgetFocusTarget,
    /// Widget that now owns focus.
    pub current:  WidgetFocusTarget,
    /// Reason for the transition.
    pub cause:    WidgetFocusChangeCause,
}

/// Reason for a widget focus transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidgetFocusChangeCause {
    /// A pointer press selected the widget.
    Pointer,
    /// A next, previous, first, or last request selected the widget.
    Traversal,
    /// Another semantic widget operation selected the widget.
    Semantic,
    /// Application code requested the widget directly.
    Application,
    /// Application code explicitly cleared the window's focus.
    ExplicitClear,
    /// The focused widget or its owning panel role was removed.
    WidgetRemoved,
    /// The focused widget lost [`WidgetFocusable`].
    FocusabilityRemoved,
    /// The window input scope was removed.
    ScopeLost,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum FocusIndicator {
    Hidden,
    Visible,
}

#[derive(Clone, Copy)]
pub(super) struct WindowFocusScope {
    pub(super) active_panel:       Entity,
    pub(super) indicator:          FocusIndicator,
    pub(super) widget:             WidgetFocusTarget,
    pub(super) interaction_camera: Option<Entity>,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum ImeFocusReleaseObserverState {
    #[default]
    Uninstalled,
    Installed,
}

#[derive(Clone)]
pub(super) struct PointerEditFocusSession {
    pub(super) window: Entity,
    pub(super) target: ImeTarget,
}

pub(super) enum PointerEditFocusCompletion {
    Untracked,
    Tracked(PointerEditFocusSession),
}

#[derive(Default, Resource)]
pub(crate) struct WidgetFocusAuthority {
    pub(super) scopes:                HashMap<Entity, WindowFocusScope>,
    ime_focus_release_observers:      ImeFocusReleaseObserverState,
    pointer_edit_focus_by_session_id: HashMap<ImeSessionId, PointerEditFocusSession>,
}

impl WidgetFocusAuthority {
    pub(in crate::widgets::interaction) fn active_panel(&self, window: Entity) -> Option<Entity> {
        self.scopes.get(&window).map(|scope| scope.active_panel)
    }

    pub(crate) fn focused_widget(&self, window: Entity) -> WidgetFocusTarget {
        self.scopes
            .get(&window)
            .map_or(WidgetFocusTarget::Unfocused, |scope| scope.widget)
    }

    pub(crate) fn interaction_camera(&self, window: Entity, widget: Entity) -> Option<Entity> {
        self.scopes
            .get(&window)
            .filter(|scope| scope.widget == WidgetFocusTarget::Widget(widget))
            .and_then(|scope| scope.interaction_camera)
    }

    pub(crate) fn clear_pointer_focused_widget(
        &mut self,
        window: Entity,
        commands: &mut Commands<'_, '_>,
    ) {
        let Some(scope) = self.scopes.get(&window).copied() else {
            return;
        };
        if scope.indicator != FocusIndicator::Hidden || scope.widget == WidgetFocusTarget::Unfocused
        {
            return;
        }
        transition_focus(
            self,
            window,
            Some(WindowFocusScope {
                widget: WidgetFocusTarget::Unfocused,
                ..scope
            }),
            WidgetFocusChangeCause::Semantic,
            commands,
        );
    }

    pub(super) fn reserve_ime_focus_release_observers(&mut self) -> bool {
        if self.ime_focus_release_observers == ImeFocusReleaseObserverState::Installed {
            return false;
        }
        self.ime_focus_release_observers = ImeFocusReleaseObserverState::Installed;
        true
    }

    pub(super) fn track_pointer_edit_focus(
        &mut self,
        session_id: ImeSessionId,
        window: Entity,
        target: ImeTarget,
    ) {
        self.pointer_edit_focus_by_session_id
            .insert(session_id, PointerEditFocusSession { window, target });
    }

    pub(super) fn complete_pointer_edit_focus(
        &mut self,
        session_id: ImeSessionId,
    ) -> PointerEditFocusCompletion {
        match self.pointer_edit_focus_by_session_id.remove(&session_id) {
            Some(session) => PointerEditFocusCompletion::Tracked(session),
            None => PointerEditFocusCompletion::Untracked,
        }
    }

    pub(crate) fn tooltip_focus_context(
        &self,
        widget: Entity,
    ) -> Option<(Entity, Entity, Option<Entity>)> {
        self.scopes
            .iter()
            .filter_map(|(&window, scope)| {
                (scope.widget == WidgetFocusTarget::Widget(widget)
                    && scope.indicator == FocusIndicator::Visible)
                    .then_some((window, scope.active_panel, scope.interaction_camera))
            })
            .min_by_key(|(window, _, _)| window.to_bits())
    }

    fn focuses(&self, widget: Entity) -> bool {
        self.scopes
            .values()
            .any(|scope| scope.widget == WidgetFocusTarget::Widget(widget))
    }

    fn shows_focus(&self, widget: Entity) -> bool {
        self.scopes.values().any(|scope| {
            scope.widget == WidgetFocusTarget::Widget(widget)
                && scope.indicator == FocusIndicator::Visible
        })
    }
}

pub(in crate::widgets) fn cleanup_removed_focus_participants(
    mut removed_focusable: RemovedComponents<WidgetFocusable>,
    mut removed_widgets: RemovedComponents<PanelWidget>,
    mut removed_windows: RemovedComponents<Window>,
    mut window_focus_events: MessageReader<WindowFocused>,
    participants: Query<
        (Has<PanelWidget>, Has<WidgetFocusable>),
        Or<(With<PanelWidget>, With<WidgetFocusable>)>,
    >,
    windows: Query<(), With<Window>>,
    mut authority: ResMut<WidgetFocusAuthority>,
    mut commands: Commands,
) {
    let removed_participants = removed_focusable
        .read()
        .chain(removed_widgets.read())
        .collect::<HashSet<_>>();
    for widget in removed_participants {
        let state = participants.get(widget).ok();
        if state.is_some_and(|(is_widget, is_focusable)| is_widget && is_focusable) {
            continue;
        }
        let cause = if state.is_some_and(|(is_widget, _)| is_widget) {
            WidgetFocusChangeCause::FocusabilityRemoved
        } else {
            WidgetFocusChangeCause::WidgetRemoved
        };
        clear_removed_widget_focus(widget, cause, &mut authority, &mut commands);
    }

    let removed_scopes = removed_windows.read().collect::<Vec<_>>();
    for window in removed_scopes {
        if windows.get(window).is_ok() {
            continue;
        }
        transition_focus(
            &mut authority,
            window,
            None,
            WidgetFocusChangeCause::ScopeLost,
            &mut commands,
        );
    }

    for event in window_focus_events.read() {
        if event.focused {
            continue;
        }
        transition_focus(
            &mut authority,
            event.window,
            None,
            WidgetFocusChangeCause::ScopeLost,
            &mut commands,
        );
    }
}

pub(crate) fn finalize_panel_focus(
    panel: Entity,
    authority: &mut WidgetFocusAuthority,
    commands: &mut Commands<'_, '_>,
) {
    let affected_windows = authority
        .scopes
        .iter()
        .filter_map(|(&window, scope)| (scope.active_panel == panel).then_some(window))
        .collect::<Vec<_>>();
    for window in affected_windows {
        transition_focus(
            authority,
            window,
            None,
            WidgetFocusChangeCause::WidgetRemoved,
            commands,
        );
    }
}

fn clear_removed_widget_focus(
    widget: Entity,
    cause: WidgetFocusChangeCause,
    authority: &mut WidgetFocusAuthority,
    commands: &mut Commands<'_, '_>,
) {
    let affected = authority
        .scopes
        .iter()
        .filter_map(|(&window, scope)| {
            (scope.widget == WidgetFocusTarget::Widget(widget))
                .then_some((window, scope.active_panel))
        })
        .collect::<Vec<_>>();
    for (window, active_panel) in affected {
        transition_focus(
            authority,
            window,
            Some(WindowFocusScope {
                active_panel,
                indicator: FocusIndicator::Hidden,
                widget: WidgetFocusTarget::Unfocused,
                interaction_camera: authority
                    .scopes
                    .get(&window)
                    .and_then(|scope| scope.interaction_camera),
            }),
            cause,
            commands,
        );
    }
}

pub(super) fn transition_focus(
    authority: &mut WidgetFocusAuthority,
    window: Entity,
    next: Option<WindowFocusScope>,
    cause: WidgetFocusChangeCause,
    commands: &mut Commands<'_, '_>,
) {
    let previous = authority.focused_widget(window);
    let next_widget = next.map_or(WidgetFocusTarget::Unfocused, |scope| scope.widget);
    let affected_widgets = [previous, next_widget]
        .into_iter()
        .filter_map(|target| match target {
            WidgetFocusTarget::Widget(widget) => Some(widget),
            WidgetFocusTarget::Unfocused => None,
        })
        .collect::<HashSet<_>>();
    let previous_markers = affected_widgets
        .iter()
        .map(|&widget| {
            (
                widget,
                authority.focuses(widget),
                authority.shows_focus(widget),
            )
        })
        .collect::<Vec<_>>();
    match next {
        Some(scope) => {
            authority.scopes.insert(window, scope);
        },
        None => {
            authority.scopes.remove(&window);
        },
    }
    let current = authority.focused_widget(window);
    for (widget, was_focused, was_visible) in previous_markers {
        let is_focused = authority.focuses(widget);
        let is_visible = authority.shows_focus(widget);
        if was_focused != is_focused {
            if is_focused {
                commands.entity(widget).insert(WidgetFocused);
            } else {
                commands.entity(widget).try_remove::<WidgetFocused>();
            }
        }
        if was_visible != is_visible {
            if is_visible {
                commands.entity(widget).insert(WidgetFocusVisible(()));
            } else {
                commands.entity(widget).try_remove::<WidgetFocusVisible>();
            }
        }
    }
    if previous == current {
        return;
    }
    commands.trigger(WidgetFocusChanged {
        window,
        previous,
        current,
        cause,
    });
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Query;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::window::WindowFocused;
    use hana_valence::AnchorSite;
    use hana_valence::AnchoredHere;
    use hana_valence::AnchoredTo;

    use super::*;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::WidgetOf;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ClearWidgetFocus;
    use crate::widgets::PanelWidget;
    use crate::widgets::RequestWidgetFocus;
    use crate::widgets::ScreenWidgetAnchoredHere;
    use crate::widgets::ScreenWidgetAnchoredTo;
    use crate::widgets::WidgetsPlugin;

    const PANEL_HEIGHT: f32 = 50.0;
    const PANEL_WIDTH: f32 = 100.0;

    #[derive(Default, Resource)]
    struct FocusChanges(Vec<WidgetFocusChanged>);

    #[derive(Default, Resource)]
    struct TeardownObservation {
        changes:   usize,
        relations: Option<TeardownRelations>,
    }

    struct TeardownRelations;

    fn record_focus_change(change: On<WidgetFocusChanged>, mut changes: ResMut<FocusChanges>) {
        changes.0.push(*change.event());
    }

    fn observe_teardown_change(
        change: On<WidgetFocusChanged>,
        widgets: Query<(&WidgetOf, &AnchoredHere, &ScreenWidgetAnchoredHere), With<PanelWidget>>,
        mut observation: ResMut<TeardownObservation>,
    ) {
        if change.cause != WidgetFocusChangeCause::WidgetRemoved {
            return;
        }
        observation.changes += 1;
        let WidgetFocusTarget::Widget(previous) = change.previous else {
            return;
        };
        if widgets.get(previous).is_ok() {
            observation.relations = Some(TeardownRelations);
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<FocusChanges>()
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .add_observer(record_focus_change);
        app
    }

    fn widget_tree(ids: &[&str]) -> LayoutTree {
        let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
        for id in ids {
            builder.with(El::new().button(*id), |_| {});
        }
        builder.build()
    }

    fn spawn_panel(app: &mut App, ids: &[&str]) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
            .with_tree(widget_tree(ids))
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        app.world_mut().spawn(panel).id()
    }

    fn widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
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

    fn focused(app: &App, window: Entity) -> Option<Entity> {
        match app
            .world()
            .resource::<WidgetFocusAuthority>()
            .focused_widget(window)
        {
            WidgetFocusTarget::Widget(widget) => Some(widget),
            WidgetFocusTarget::Unfocused => None,
        }
    }

    fn request_focus(app: &mut App, window: Entity, widget: Entity) {
        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
    }

    #[test]
    fn windows_keep_independent_focus_and_shared_marker_state() {
        let mut app = test_app();
        let first_window = app.world_mut().spawn(Window::default()).id();
        let second_window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["first", "second"]);
        app.update();
        let first = widget(&mut app, panel, "first");
        let second = widget(&mut app, panel, "second");

        request_focus(&mut app, first_window, first);
        request_focus(&mut app, second_window, second);
        assert_eq!(focused(&app, first_window), Some(first));
        assert_eq!(focused(&app, second_window), Some(second));
        assert!(app.world().get::<WidgetFocused>(first).is_some());
        assert!(app.world().get::<WidgetFocused>(second).is_some());

        request_focus(&mut app, second_window, first);
        app.world_mut().trigger(ClearWidgetFocus {
            window: first_window,
        });
        app.world_mut().flush();
        assert_eq!(focused(&app, first_window), None);
        assert_eq!(focused(&app, second_window), Some(first));
        assert!(app.world().get::<WidgetFocused>(first).is_some());

        let changes = &app.world().resource::<FocusChanges>().0;
        assert_eq!(
            changes.last().map(|change| change.cause),
            Some(WidgetFocusChangeCause::ExplicitClear)
        );
    }

    #[test]
    fn removed_focusable_stays_removed_across_reify_and_clears_focus() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["first", "second"]);
        app.update();
        let first = widget(&mut app, panel, "first");
        request_focus(&mut app, window, first);

        app.world_mut()
            .entity_mut(first)
            .remove::<WidgetFocusable>();
        app.update();
        assert_eq!(focused(&app, window), None);
        assert_eq!(
            app.world()
                .resource::<FocusChanges>()
                .0
                .last()
                .map(|change| change.cause),
            Some(WidgetFocusChangeCause::FocusabilityRemoved)
        );

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, widget_tree(&["second", "first"]));
        assert!(result.is_ok());
        app.update();
        assert_eq!(widget(&mut app, panel, "first"), first);
        assert!(app.world().get::<WidgetFocusable>(first).is_none());
    }

    #[test]
    fn window_input_scope_loss_clears_only_that_windows_focus() {
        let mut app = test_app();
        let lost_window = app.world_mut().spawn(Window::default()).id();
        let retained_window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["target"]);
        app.update();
        let target = widget(&mut app, panel, "target");
        request_focus(&mut app, lost_window, target);
        request_focus(&mut app, retained_window, target);

        app.world_mut().write_message(WindowFocused {
            window:  lost_window,
            focused: false,
        });
        app.update();

        assert_eq!(focused(&app, lost_window), None);
        assert_eq!(focused(&app, retained_window), Some(target));
        assert!(app.world().get::<WidgetFocused>(target).is_some());
        assert_eq!(
            app.world()
                .resource::<FocusChanges>()
                .0
                .last()
                .map(|change| change.cause),
            Some(WidgetFocusChangeCause::ScopeLost)
        );
    }

    #[test]
    fn direct_focused_widget_despawn_clears_authority_once() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["target"]);
        app.update();
        let target = widget(&mut app, panel, "target");
        request_focus(&mut app, window, target);
        app.world_mut().resource_mut::<FocusChanges>().0.clear();

        assert!(app.world_mut().despawn(target));
        app.update();
        app.update();

        assert_eq!(focused(&app, window), None);
        let changes = &app.world().resource::<FocusChanges>().0;
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].window, window);
        assert_eq!(changes[0].previous, WidgetFocusTarget::Widget(target));
        assert_eq!(changes[0].current, WidgetFocusTarget::Unfocused);
        assert_eq!(changes[0].cause, WidgetFocusChangeCause::WidgetRemoved);
    }

    #[test]
    fn panel_role_teardown_reports_focus_before_widget_relations_are_removed() {
        let mut app = test_app();
        app.init_resource::<TeardownObservation>()
            .add_observer(observe_teardown_change);
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["target"]);
        app.update();
        let target = widget(&mut app, panel, "target");
        request_focus(&mut app, window, target);

        app.world_mut().spawn(AnchoredTo::new(
            target,
            AnchorSite::Center,
            AnchorSite::Center,
        ));
        app.world_mut().spawn(ScreenWidgetAnchoredTo::new(target));
        assert!(app.world().get::<AnchoredHere>(target).is_some());
        assert!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(target)
                .is_some()
        );

        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();

        let observation = app.world().resource::<TeardownObservation>();
        assert_eq!(observation.changes, 1);
        assert!(observation.relations.is_some());
        assert_eq!(focused(&app, window), None);
        assert!(app.world().get::<WidgetFocused>(target).is_none());
    }

    #[test]
    fn full_panel_despawn_reports_focus_before_widget_relations_are_removed() {
        let mut app = test_app();
        app.init_resource::<TeardownObservation>()
            .add_observer(observe_teardown_change);
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["target"]);
        app.update();
        let target = widget(&mut app, panel, "target");
        request_focus(&mut app, window, target);

        app.world_mut().spawn(AnchoredTo::new(
            target,
            AnchorSite::Center,
            AnchorSite::Center,
        ));
        app.world_mut().spawn(ScreenWidgetAnchoredTo::new(target));
        assert!(app.world().get::<AnchoredHere>(target).is_some());
        assert!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(target)
                .is_some()
        );
        app.world_mut().resource_mut::<FocusChanges>().0.clear();

        assert!(app.world_mut().despawn(panel));

        let observation = app.world().resource::<TeardownObservation>();
        assert_eq!(observation.changes, 1);
        assert!(observation.relations.is_some());
        assert_eq!(focused(&app, window), None);
        let changes = &app.world().resource::<FocusChanges>().0;
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].window, window);
        assert_eq!(changes[0].previous, WidgetFocusTarget::Widget(target));
        assert_eq!(changes[0].current, WidgetFocusTarget::Unfocused);
        assert_eq!(changes[0].cause, WidgetFocusChangeCause::WidgetRemoved);
    }
}
