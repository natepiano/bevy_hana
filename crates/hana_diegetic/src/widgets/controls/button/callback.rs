use std::fmt;
use std::fmt::Formatter;
use std::sync::Arc;

use bevy::ecs::error::BevyError;
use bevy::ecs::system::SystemHandle;
use bevy::ecs::system::SystemHandleTemplate;
use bevy::ecs::system::SystemId;
use bevy::ecs::template::SceneEntityReferences;
use bevy::ecs::template::Template;
use bevy::ecs::template::TemplateContext;
use bevy::ecs::world::EntityWorldMut;
use bevy::prelude::Commands;
use bevy::prelude::Component;
#[cfg(test)]
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::In;
use bevy::prelude::IntoSystem;
use bevy::prelude::On;
use bevy::prelude::Query;

use super::events::ButtonClicked;

/// Cloneable authored click callback, compared by [`Arc`] identity so the
/// enclosing authored `Button` (and its `WidgetSpec`) stays `PartialEq`.
#[derive(Clone)]
pub(in crate::widgets) struct ButtonCallback(Arc<SystemHandleTemplate<In<ButtonClicked>, ()>>);

impl ButtonCallback {
    pub(super) fn new<M>(system: impl IntoSystem<In<ButtonClicked>, (), M>) -> Self {
        Self(Arc::new(SystemHandleTemplate::value(system)))
    }

    /// Returns the tracked [`SystemHandle`] for this callback.
    ///
    /// The first build registers the callback as a tracked system;
    /// later builds of the same callback return the cached handle without
    /// registering again.
    pub(in crate::widgets) fn build_handle(
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::widgets) enum ButtonCallbackPolicy {
    #[default]
    NotifyObserversOnly,
    AlsoRun(ButtonCallback),
}

/// Tracked handle to a widget's registered click-callback system.
///
/// Reify installs and replaces this component; dropping it releases the
/// widget's strong handle, and Bevy unregisters the system once the final
/// handle is gone.
#[derive(Component)]
pub(in crate::widgets) struct ButtonCallbackHandle(SystemHandle<In<ButtonClicked>, ()>);

impl ButtonCallbackHandle {
    pub(in crate::widgets) const fn new(handle: SystemHandle<In<ButtonClicked>, ()>) -> Self {
        Self(handle)
    }

    fn system_id(&self) -> SystemId<In<ButtonClicked>> { SystemId::from(&self.0) }

    #[cfg(test)]
    pub(in crate::widgets) fn system_entity(&self) -> Entity { self.0.entity() }
}

/// Runs the clicked widget's authored callback with the completed event.
///
/// `WidgetsPlugin` installs this single global [`ButtonClicked`] observer;
/// reify never installs a per-widget observer. The observer reads only the
/// target's [`ButtonCallbackHandle`] and forwards the finished event — it
/// never touches [`ButtonPress`] or [`ButtonCaptures`], whose pointer
/// lifecycle and semantic activation are resolved before dispatch.
pub(in crate::widgets) fn dispatch_click_callback(
    click: On<ButtonClicked>,
    handles: Query<&ButtonCallbackHandle>,
    mut commands: Commands,
) {
    let Ok(handle) = handles.get(click.event_target()) else {
        return;
    };
    commands.run_system_with(handle.system_id(), click.event().clone());
}

#[cfg(test)]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::observer::ObservedBy;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::backend::HitData;
    use bevy::picking::hover::PickingInteraction;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::Entity;
    use bevy::prelude::In;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PointerClick;
    use bevy::prelude::PointerPress;
    use bevy::prelude::PointerRelease;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Vec2;

    use crate::Appearance;
    use crate::Border;
    use crate::Button;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Sizing;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::controls::button::ButtonActivationSource;
    use crate::widgets::controls::button::ButtonCancelCause;
    use crate::widgets::controls::button::ButtonCanceled;
    use crate::widgets::controls::button::ButtonClicked;
    use crate::widgets::controls::button::ButtonPressSource;
    use crate::widgets::controls::button::ButtonPressed;
    use crate::widgets::controls::button::ButtonReleased;

    const BUTTON_ID: &str = "action";
    const NORMAL_FILL: Color = Color::srgb(0.05, 0.10, 0.20);
    const NORMAL_BORDER: Color = Color::srgb(0.30, 0.30, 0.30);
    const HOVER_FILL: Color = Color::srgb(0.15, 0.30, 0.60);

    /// A button element in the default row layout.
    type ButtonElement = El<crate::Row, crate::WidgetElement<Button>>;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedButtonEvent {
        Pressed(PointerId),
        Released(PointerId),
        Clicked(ButtonActivationSource),
        Canceled(PointerId, ButtonCancelCause),
    }

    #[derive(Default, Resource)]
    struct RecordedButtonEvents(Vec<RecordedButtonEvent>);

    #[derive(Default, Resource)]
    struct CallbackClicks(Vec<(Entity, ButtonActivationSource)>);

    fn record_pressed(event: On<ButtonPressed>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Pressed(match event.source {
                ButtonPressSource::Pointer(pointer_id) => pointer_id,
                ButtonPressSource::Semantic => return,
            }));
    }

    fn record_released(event: On<ButtonReleased>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Released(match event.source {
                ButtonPressSource::Pointer(pointer_id) => pointer_id,
                ButtonPressSource::Semantic => return,
            }));
    }

    fn record_clicked(event: On<ButtonClicked>, mut events: ResMut<RecordedButtonEvents>) {
        events.0.push(RecordedButtonEvent::Clicked(event.source));
    }

    fn record_canceled(event: On<ButtonCanceled>, mut events: ResMut<RecordedButtonEvents>) {
        events.0.push(RecordedButtonEvent::Canceled(
            match event.source {
                ButtonPressSource::Pointer(pointer_id) => pointer_id,
                ButtonPressSource::Semantic => return,
            },
            event.cause,
        ));
    }

    fn record_callback_click(click: In<ButtonClicked>, mut clicks: ResMut<CallbackClicks>) {
        clicks.0.push((click.entity, click.source));
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

    fn callback_button_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().button(BUTTON_ID).on_click(record_callback_click),
            |_| {},
        );
        builder.build()
    }

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

    fn release(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(PointerRelease {
            entity:  widget,
            pointer: Pointer::new(pointer_id, location()),
            button:  PointerButton::Primary,
            hit:     hit(),
        });
        app.world_mut().flush();
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
            [(widget, ButtonActivationSource::Pointer(pointer_id))],
            "exactly one observer dispatches the callback per click",
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
            entity: widget,
            id:     PanelElementId::named(BUTTON_ID),
            source: ButtonActivationSource::Semantic,
        });
        app.update();
        assert_eq!(app.world().resource::<CallbackClicks>().0.len(), 1);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(root_override(&app, widget).is_some());
        app.world_mut().trigger(ButtonClicked {
            entity: widget,
            id:     PanelElementId::named(BUTTON_ID),
            source: ButtonActivationSource::Semantic,
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
