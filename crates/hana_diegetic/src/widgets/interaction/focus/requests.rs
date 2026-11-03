use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::prelude::Camera;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::ContainsEntity;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Event;
use bevy::prelude::On;
use bevy::prelude::PointerButton;
use bevy::prelude::PointerPress;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::PrimaryWindow;

use super::authority;
use super::authority::FocusIndicator;
use super::authority::PointerEditFocusCompletion;
use super::authority::WidgetFocusAuthority;
use super::authority::WidgetFocusChangeCause;
use super::authority::WidgetFocusTarget;
use super::authority::WidgetFocusable;
use super::authority::WindowFocusScope;
use crate::ImeApplied;
use crate::ImeCancelCause;
use crate::ImeCanceled;
use crate::ImeInputBlocker;
use crate::ImeStarted;
use crate::ImeTarget;
use crate::panel::DiegeticPanel;
use crate::widgets::PanelWidget;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;

/// Selects a live panel as one window's widget-focus scope.
///
/// This does not focus a widget. A subsequent traversal request starts from
/// the selected panel's first or last focusable widget, according to the
/// requested traversal direction.
#[derive(Clone, Copy, Debug, EntityEvent)]
pub struct RequestPanelFocus {
    /// Window whose focus scope should select the panel.
    pub window: Entity,
    /// Live panel selected for subsequent widget traversal.
    #[event_target]
    pub panel:  Entity,
}

/// Requests focus for a live widget in one window.
#[derive(Clone, Copy, Debug, EntityEvent)]
pub struct RequestWidgetFocus {
    /// Window whose focus scope should change.
    pub window: Entity,
    /// Live widget entity that should receive focus.
    #[event_target]
    pub widget: Entity,
}

/// Clears the focused widget in one window while retaining its active panel.
#[derive(Clone, Copy, Debug, Event)]
pub struct ClearWidgetFocus {
    /// Window whose focused widget should be cleared.
    pub window: Entity,
}

pub(in crate::widgets) fn request_panel_focus(
    request: On<RequestPanelFocus>,
    windows: Query<(), With<Window>>,
    panels: Query<(), With<DiegeticPanel>>,
    mut authority: ResMut<WidgetFocusAuthority>,
    mut commands: Commands,
) {
    let request = request.event();
    if windows.get(request.window).is_err() || panels.get(request.panel).is_err() {
        return;
    }
    let interaction_camera = authority
        .scopes
        .get(&request.window)
        .filter(|scope| scope.active_panel == request.panel)
        .and_then(|scope| scope.interaction_camera);
    authority::transition_focus(
        &mut authority,
        request.window,
        Some(WindowFocusScope {
            active_panel: request.panel,
            indicator: FocusIndicator::Hidden,
            widget: WidgetFocusTarget::Unfocused,
            interaction_camera,
        }),
        WidgetFocusChangeCause::Application,
        &mut commands,
    );
}

pub(in crate::widgets) fn request_widget_focus(
    request: On<RequestWidgetFocus>,
    windows: Query<(), With<Window>>,
    widgets: Query<&WidgetOf, (With<PanelWidget>, With<WidgetFocusable>)>,
    mut authority: ResMut<WidgetFocusAuthority>,
    mut commands: Commands,
) {
    let request = request.event();
    if windows.get(request.window).is_err() {
        return;
    }
    let Ok(widget_of) = widgets.get(request.widget) else {
        return;
    };
    let interaction_camera = authority
        .scopes
        .get(&request.window)
        .filter(|scope| scope.active_panel == widget_of.panel())
        .and_then(|scope| scope.interaction_camera);
    authority::transition_focus(
        &mut authority,
        request.window,
        Some(WindowFocusScope {
            active_panel: widget_of.panel(),
            indicator: FocusIndicator::Visible,
            widget: WidgetFocusTarget::Widget(request.widget),
            interaction_camera,
        }),
        WidgetFocusChangeCause::Application,
        &mut commands,
    );
}

pub(in crate::widgets) fn clear_widget_focus(
    request: On<ClearWidgetFocus>,
    mut authority: ResMut<WidgetFocusAuthority>,
    mut commands: Commands,
) {
    let request = request.event();
    let next = authority
        .scopes
        .get(&request.window)
        .map(|scope| WindowFocusScope {
            active_panel:       scope.active_panel,
            indicator:          FocusIndicator::Hidden,
            widget:             WidgetFocusTarget::Unfocused,
            interaction_camera: scope.interaction_camera,
        });
    authority::transition_focus(
        &mut authority,
        request.window,
        next,
        WidgetFocusChangeCause::ExplicitClear,
        &mut commands,
    );
}

pub(in crate::widgets) fn focus_from_pointer_press(
    press: On<PointerPress>,
    cameras: Query<&RenderTarget, With<Camera>>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    parents: Query<&ChildOf>,
    widgets: Query<(&WidgetOf, &WidgetKind), (With<PanelWidget>, With<WidgetFocusable>)>,
    windows: Query<(), With<Window>>,
    input_blocker: Option<Res<ImeInputBlocker>>,
    mut authority: ResMut<WidgetFocusAuthority>,
    mut commands: Commands,
) {
    if press.button != PointerButton::Primary
        || press.event_target() != press.original_event_target()
    {
        return;
    }
    let Some(window) = pointer_press_window(
        press.original_event_target(),
        press.hit.camera,
        &windows,
        &cameras,
        &primary_window,
    ) else {
        return;
    };

    install_ime_focus_release_observers(&mut authority, &mut commands);

    let Some((widget, widget_of)) =
        focusable_widget_in_hit_chain(press.original_event_target(), &parents, &widgets)
    else {
        if input_blocker
            .as_deref()
            .is_none_or(|blocker| !blocker.blocks_window(window))
        {
            clear_hidden_editable_focus(window, &widgets, &mut authority, &mut commands);
        }
        return;
    };
    authority::transition_focus(
        &mut authority,
        window,
        Some(WindowFocusScope {
            active_panel:       widget_of.panel(),
            indicator:          FocusIndicator::Hidden,
            widget:             WidgetFocusTarget::Widget(widget),
            interaction_camera: Some(press.hit.camera),
        }),
        WidgetFocusChangeCause::Pointer,
        &mut commands,
    );
}

fn focusable_widget_in_hit_chain<'a>(
    original_target: Entity,
    parents: &Query<&ChildOf>,
    widgets: &'a Query<(&WidgetOf, &WidgetKind), (With<PanelWidget>, With<WidgetFocusable>)>,
) -> Option<(Entity, &'a WidgetOf)> {
    let mut entity = original_target;
    loop {
        if let Ok((widget_of, _)) = widgets.get(entity) {
            return Some((entity, widget_of));
        }
        let Ok(parent) = parents.get(entity) else {
            return None;
        };
        entity = parent.parent();
    }
}

fn clear_hidden_editable_focus(
    window: Entity,
    widgets: &Query<(&WidgetOf, &WidgetKind), (With<PanelWidget>, With<WidgetFocusable>)>,
    authority: &mut WidgetFocusAuthority,
    commands: &mut Commands<'_, '_>,
) {
    let Some(scope) = authority.scopes.get(&window) else {
        return;
    };
    let WidgetFocusTarget::Widget(widget) = scope.widget else {
        return;
    };
    if scope.indicator != FocusIndicator::Hidden
        || !widgets
            .get(widget)
            .is_ok_and(|(_, kind)| kind.accepts_text_entry())
    {
        return;
    }
    authority.clear_pointer_focused_widget(window, commands);
}

fn install_ime_focus_release_observers(
    authority: &mut WidgetFocusAuthority,
    commands: &mut Commands<'_, '_>,
) {
    if !authority.reserve_ime_focus_release_observers() {
        return;
    }
    commands.add_observer(track_pointer_focus_for_ime_start);
    commands.add_observer(release_pointer_focus_after_ime_apply);
    commands.add_observer(release_pointer_focus_after_ime_cancel);
}

fn track_pointer_focus_for_ime_start(
    started: On<ImeStarted>,
    input_blocker: Res<ImeInputBlocker>,
    widgets: Query<(&WidgetOf, &PanelWidget, &WidgetKind), With<WidgetFocusable>>,
    mut authority: ResMut<WidgetFocusAuthority>,
) {
    if input_blocker.session_id() != Some(started.session_id) {
        return;
    }
    let Some(window) = input_blocker.window() else {
        return;
    };
    let Some(scope) = authority.scopes.get(&window) else {
        return;
    };
    let WidgetFocusTarget::Widget(widget) = scope.widget else {
        return;
    };
    let Ok((widget_of, panel_widget, kind)) = widgets.get(widget) else {
        return;
    };
    if scope.indicator != FocusIndicator::Hidden
        || !kind.accepts_text_entry()
        || !ime_target_matches_widget(&started.target, *widget_of, panel_widget)
    {
        return;
    }
    authority.track_pointer_edit_focus(started.session_id, window, started.target.clone());
}

#[derive(Clone, Copy)]
enum PointerFocusDisposition {
    Release,
    Retain,
}

fn release_pointer_focus_after_ime_apply(
    applied: On<ImeApplied>,
    widgets: Query<(&WidgetOf, &PanelWidget, &WidgetKind), With<WidgetFocusable>>,
    mut authority: ResMut<WidgetFocusAuthority>,
    mut commands: Commands,
) {
    release_pointer_focus_after_ime_end(
        applied.session_id,
        &applied.target,
        PointerFocusDisposition::Release,
        &widgets,
        &mut authority,
        &mut commands,
    );
}

fn release_pointer_focus_after_ime_cancel(
    canceled: On<ImeCanceled>,
    widgets: Query<(&WidgetOf, &PanelWidget, &WidgetKind), With<WidgetFocusable>>,
    mut authority: ResMut<WidgetFocusAuthority>,
    mut commands: Commands,
) {
    let focus_disposition = match canceled.cause {
        ImeCancelCause::Replaced => PointerFocusDisposition::Retain,
        ImeCancelCause::Escape
        | ImeCancelCause::Blur
        | ImeCancelCause::Request
        | ImeCancelCause::SessionMismatch
        | ImeCancelCause::WindowClosed
        | ImeCancelCause::FocusLost
        | ImeCancelCause::TargetStale
        | ImeCancelCause::LeaseLost => PointerFocusDisposition::Release,
    };
    release_pointer_focus_after_ime_end(
        canceled.session_id,
        &canceled.target,
        focus_disposition,
        &widgets,
        &mut authority,
        &mut commands,
    );
}

fn release_pointer_focus_after_ime_end(
    session_id: crate::ImeSessionId,
    target: &ImeTarget,
    focus_disposition: PointerFocusDisposition,
    widgets: &Query<(&WidgetOf, &PanelWidget, &WidgetKind), With<WidgetFocusable>>,
    authority: &mut WidgetFocusAuthority,
    commands: &mut Commands<'_, '_>,
) {
    let PointerEditFocusCompletion::Tracked(session) =
        authority.complete_pointer_edit_focus(session_id)
    else {
        return;
    };
    match focus_disposition {
        PointerFocusDisposition::Release if session.target == *target => {},
        PointerFocusDisposition::Release | PointerFocusDisposition::Retain => return,
    }
    let Some(scope) = authority.scopes.get(&session.window) else {
        return;
    };
    let WidgetFocusTarget::Widget(widget) = scope.widget else {
        return;
    };
    let Ok((widget_of, panel_widget, kind)) = widgets.get(widget) else {
        return;
    };
    if scope.indicator != FocusIndicator::Hidden
        || !kind.accepts_text_entry()
        || !ime_target_matches_widget(target, *widget_of, panel_widget)
    {
        return;
    }
    authority.clear_pointer_focused_widget(session.window, commands);
}

fn ime_target_matches_widget(
    target: &ImeTarget,
    widget_of: WidgetOf,
    panel_widget: &PanelWidget,
) -> bool {
    match target {
        ImeTarget::WorldPanelField { panel, field_id }
        | ImeTarget::ScreenPanelField { panel, field_id } => {
            *panel == widget_of.panel() && field_id == panel_widget.id()
        },
        ImeTarget::AppOwned { .. } => false,
    }
}

fn hit_camera_window(
    camera: Entity,
    cameras: &Query<&RenderTarget, With<Camera>>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<Entity> {
    match cameras
        .get(camera)
        .ok()?
        .normalize(primary_window.single().ok())?
    {
        NormalizedRenderTarget::Window(window_ref) => Some(window_ref.entity()),
        NormalizedRenderTarget::Image(_)
        | NormalizedRenderTarget::TextureView(_)
        | NormalizedRenderTarget::None { .. } => None,
    }
}

fn pointer_press_window(
    original_target: Entity,
    hit_camera: Entity,
    windows: &Query<(), With<Window>>,
    cameras: &Query<&RenderTarget, With<Camera>>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<Entity> {
    if windows.contains(original_target) {
        return Some(original_target);
    }
    hit_camera_window(hit_camera, cameras, primary_window)
        .filter(|window| windows.contains(*window))
}

#[cfg(test)]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::camera::RenderTarget;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::App;
    use bevy::prelude::Camera;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PointerPress;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::UVec2;
    use bevy::prelude::Vec2;
    use bevy::prelude::Window;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowRef;

    use super::*;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeCancelCause;
    use crate::ImeCanceled;
    use crate::ImeEditableFieldSpec;
    use crate::ImeOpenSession;
    use crate::ImeRequestCancel;
    use crate::ImeSessionId;
    use crate::ImeTarget;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::WidgetInput;
    use crate::ime::ImePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetFocusAuthority;
    use crate::widgets::WidgetFocusChangeCause;
    use crate::widgets::WidgetFocusChanged;
    use crate::widgets::WidgetFocusVisible;
    use crate::widgets::WidgetFocused;
    use crate::widgets::WidgetsPlugin;

    const PANEL_HEIGHT: f32 = 50.0;
    const PANEL_WIDTH: f32 = 100.0;

    #[derive(Default, Resource)]
    struct FocusChanges(Vec<WidgetFocusChanged>);

    fn record_focus_change(change: On<WidgetFocusChanged>, mut changes: ResMut<FocusChanges>) {
        changes.0.push(*change.event());
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

    fn editable_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
        builder.with(
            El::new().editable_field(
                "number",
                ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("number")),
                crate::TextStyle::new(10.0),
            ),
            |builder| {
                builder.text("5");
            },
        );
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

    fn spawn_editable_panel(app: &mut App) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
            .with_tree(editable_tree())
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

    fn request_panel_focus(app: &mut App, window: Entity, panel: Entity) {
        app.world_mut().trigger(RequestPanelFocus { window, panel });
        app.world_mut().flush();
    }

    fn press(app: &mut App, window: Entity, camera: Entity, target: Entity) {
        let Some(window_ref) = WindowRef::Entity(window).normalize(None) else {
            return;
        };
        app.world_mut().trigger(PointerPress {
            entity:  target,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::Window(window_ref),
                    position: Vec2::ZERO,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(camera, 0.0, None, None),
            count:   1,
        });
        app.world_mut().flush();
    }

    #[test]
    fn panel_focus_request_selects_traversal_scope_without_focusing_a_widget() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let first_panel = spawn_panel(&mut app, &["first"]);
        let selected_panel = spawn_panel(&mut app, &["selected"]);
        app.update();
        let first = widget(&mut app, first_panel, "first");
        let selected = widget(&mut app, selected_panel, "selected");
        request_focus(&mut app, window, first);
        app.world_mut().resource_mut::<FocusChanges>().0.clear();

        request_panel_focus(&mut app, window, selected_panel);

        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .active_panel(window),
            Some(selected_panel)
        );
        assert_eq!(focused(&app, window), None);
        assert!(app.world().get::<WidgetFocused>(first).is_none());
        assert!(app.world().get::<WidgetFocusVisible>(first).is_none());
        let changes = &app.world().resource::<FocusChanges>().0;
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].previous, WidgetFocusTarget::Widget(first));
        assert_eq!(changes[0].current, WidgetFocusTarget::Unfocused);
        assert_eq!(changes[0].cause, WidgetFocusChangeCause::Application);

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();

        assert_eq!(focused(&app, window), Some(selected));
        assert!(app.world().get::<WidgetFocused>(selected).is_some());
        assert!(app.world().get::<WidgetFocusVisible>(selected).is_some());
    }

    #[test]
    fn pointer_focus_uses_the_hit_camera_render_target() {
        let mut app = test_app();
        let primary_window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let pointer_window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["remembered", "target"]);
        app.update();
        let remembered = widget(&mut app, panel, "remembered");
        let target = widget(&mut app, panel, "target");
        request_focus(&mut app, primary_window, remembered);
        assert!(app.world().get::<WidgetFocusVisible>(remembered).is_some());
        let none_camera = app
            .world_mut()
            .spawn((Camera::default(), RenderTarget::None { size: UVec2::ONE }))
            .id();
        let primary_camera = app
            .world_mut()
            .spawn((Camera::default(), RenderTarget::Window(WindowRef::Primary)))
            .id();
        let Some(primary_window_ref) = WindowRef::Entity(primary_window).normalize(None) else {
            return;
        };
        let Some(pointer_window_ref) = WindowRef::Entity(pointer_window).normalize(None) else {
            return;
        };

        app.world_mut().trigger(PointerPress {
            entity:  target,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::Window(primary_window_ref),
                    position: Vec2::ZERO,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(none_camera, 0.0, None, None),
            count:   1,
        });
        app.world_mut().flush();
        assert_eq!(focused(&app, primary_window), Some(remembered));

        app.world_mut().trigger(PointerPress {
            entity:  target,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::Window(pointer_window_ref),
                    position: Vec2::ZERO,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(primary_camera, 0.0, None, None),
            count:   1,
        });
        app.world_mut().flush();
        assert_eq!(focused(&app, primary_window), Some(target));
        assert_eq!(focused(&app, pointer_window), None);
        assert!(app.world().get::<WidgetFocused>(target).is_some());
        assert!(app.world().get::<WidgetFocusVisible>(target).is_none());
        assert_eq!(
            app.world()
                .resource::<FocusChanges>()
                .0
                .last()
                .map(|change| change.cause),
            Some(WidgetFocusChangeCause::Pointer)
        );

        app.world_mut().resource_mut::<FocusChanges>().0.clear();
        app.world_mut().write_message(WidgetInput::FocusLast {
            window: primary_window,
        });
        app.update();

        assert_eq!(focused(&app, primary_window), Some(target));
        assert!(app.world().get::<WidgetFocused>(target).is_some());
        assert!(app.world().get::<WidgetFocusVisible>(target).is_some());
        assert!(
            app.world().resource::<FocusChanges>().0.is_empty(),
            "revealing the indicator must not report a semantic focus transition",
        );
    }

    #[test]
    fn number_box_empty_hit_clears_pointer_focus_but_child_hit_keeps_it() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "number");
        let editable_child = app.world_mut().spawn(ChildOf(editable)).id();

        press(&mut app, window, camera, editable_child);

        assert_eq!(focused(&app, window), Some(editable));
        assert!(app.world().get::<WidgetFocusVisible>(editable).is_none());

        let empty_hit = app.world_mut().spawn_empty().id();
        press(&mut app, window, camera, empty_hit);

        assert_eq!(focused(&app, window), None);
        assert!(app.world().get::<WidgetFocused>(editable).is_none());
    }

    #[test]
    fn number_box_active_edit_keeps_focus_until_cancel_finishes() {
        let mut app = test_app();
        app.add_plugins(ImePlugin);
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "number");
        press(&mut app, window, camera, editable);
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::WorldPanelField {
                panel,
                field_id: PanelElementId::named("number"),
            },
            window,
            initial_text: "5".to_owned(),
            field_spec: ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("number")),
            anchor: None,
        });
        app.world_mut().flush();

        let outside = app.world_mut().spawn_empty().id();
        press(&mut app, window, camera, outside);

        assert_eq!(focused(&app, window), Some(editable));
        let session_id = app
            .world()
            .resource::<ImeInputBlocker>()
            .session_id()
            .unwrap_or(ImeSessionId::new(0));
        app.world_mut().trigger(ImeRequestCancel {
            session_id,
            cause: ImeCancelCause::Escape,
        });
        app.world_mut().flush();

        assert_eq!(focused(&app, window), None);
        assert!(app.world().get::<WidgetFocused>(editable).is_none());
    }

    #[test]
    fn number_box_edit_end_releases_only_its_window_focus() {
        let mut app = test_app();
        app.add_plugins(ImePlugin);
        let first_window = app.world_mut().spawn(Window::default()).id();
        let second_window = app.world_mut().spawn(Window::default()).id();
        let first_camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Entity(first_window)),
            ))
            .id();
        let second_camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Entity(second_window)),
            ))
            .id();
        let panel = spawn_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "number");
        press(&mut app, first_window, first_camera, editable);
        press(&mut app, second_window, second_camera, editable);
        app.world_mut().trigger(ImeOpenSession {
            target:       ImeTarget::WorldPanelField {
                panel,
                field_id: PanelElementId::named("number"),
            },
            window:       first_window,
            initial_text: "5".to_owned(),
            field_spec:   ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("number")),
            anchor:       None,
        });
        app.world_mut().flush();
        let session_id = app
            .world()
            .resource::<ImeInputBlocker>()
            .session_id()
            .unwrap_or(ImeSessionId::new(0));

        app.world_mut().trigger(ImeRequestCancel {
            session_id,
            cause: ImeCancelCause::Escape,
        });
        app.world_mut().flush();

        assert_eq!(focused(&app, first_window), None);
        assert_eq!(focused(&app, second_window), Some(editable));
        assert!(app.world().get::<WidgetFocused>(editable).is_some());
    }

    #[test]
    fn number_box_replaced_edit_keeps_successor_pointer_focus() {
        let mut app = test_app();
        app.add_plugins(ImePlugin);
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "number");
        press(&mut app, window, camera, editable);
        let target = ImeTarget::WorldPanelField {
            panel,
            field_id: PanelElementId::named("number"),
        };
        for initial_text in ["5", "6"] {
            app.world_mut().trigger(ImeOpenSession {
                target: target.clone(),
                window,
                initial_text: initial_text.to_owned(),
                field_spec: ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("number")),
                anchor: None,
            });
            app.world_mut().flush();
        }

        assert_eq!(focused(&app, window), Some(editable));
        assert!(app.world().get::<WidgetFocused>(editable).is_some());
        assert_eq!(
            app.world().resource::<ImeInputBlocker>().window(),
            Some(window)
        );
    }

    #[test]
    fn number_box_keyboard_focus_survives_edit_terminal_event() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "number");
        press(&mut app, window, camera, editable);
        request_focus(&mut app, window, editable);

        app.world_mut().trigger(ImeCanceled {
            session_id: ImeSessionId::new(1),
            target:     ImeTarget::WorldPanelField {
                panel,
                field_id: PanelElementId::named("number"),
            },
            cause:      ImeCancelCause::Escape,
        });
        app.world_mut().flush();

        assert_eq!(focused(&app, window), Some(editable));
        assert!(app.world().get::<WidgetFocusVisible>(editable).is_some());
    }
}
