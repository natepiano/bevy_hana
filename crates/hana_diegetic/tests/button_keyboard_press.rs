//! Public-client coverage for the button keyboard press lifecycle.

use bevy::ecs::message::MessageCursor;
use bevy::ecs::system::RunSystemOnce;
use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::Key;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::keyboard::NativeKey;
use bevy::picking::backend::HitData;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::window::Ime;
use bevy::window::WindowClosed;
use bevy::window::WindowFocused;
use hana_diegetic::Button;
use hana_diegetic::ButtonActivationSource;
use hana_diegetic::ButtonCancelCause;
use hana_diegetic::ButtonCanceled;
use hana_diegetic::ButtonClicked;
use hana_diegetic::ButtonLatch;
use hana_diegetic::ButtonPressSource;
use hana_diegetic::ButtonPressed;
use hana_diegetic::ButtonReleased;
use hana_diegetic::ClearWidgetFocus;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidget;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::PanelWidgetWriter;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::Slider;
use hana_diegetic::SwitchSide;
use hana_diegetic::SwitchState;
use hana_diegetic::SwitchToggleRequested;
use hana_diegetic::WidgetInput;
use hana_diegetic::WidgetInputDisabled;
use hana_diegetic::WidgetInputMode;
use hana_diegetic::WidgetInputPlugin;
use hana_diegetic::WidgetInteractivity;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Event {
    Pressed(Entity, ButtonPressSource),
    Released(Entity, ButtonPressSource),
    Clicked(Entity, ButtonActivationSource),
    Canceled(Entity, ButtonPressSource, ButtonCancelCause),
    SwitchRequested(Entity, SwitchSide),
}

#[derive(Default, Resource)]
struct ObservedEvents(Vec<Event>);

fn record_pressed(event: On<ButtonPressed>, mut observed: ResMut<ObservedEvents>) {
    observed.0.push(Event::Pressed(event.entity, event.source));
}

fn record_released(event: On<ButtonReleased>, mut observed: ResMut<ObservedEvents>) {
    observed.0.push(Event::Released(event.entity, event.source));
}

fn record_clicked(event: On<ButtonClicked>, mut observed: ResMut<ObservedEvents>) {
    observed.0.push(Event::Clicked(event.entity, event.source));
}

fn record_canceled(event: On<ButtonCanceled>, mut observed: ResMut<ObservedEvents>) {
    observed
        .0
        .push(Event::Canceled(event.entity, event.source, event.cause));
}

fn record_switch_request(event: On<SwitchToggleRequested>, mut observed: ResMut<ObservedEvents>) {
    observed
        .0
        .push(Event::SwitchRequested(event.entity, event.requested));
}

struct Fixture {
    app:    App,
    window: Entity,
    button: Entity,
    toggle: Entity,
    switch: Entity,
    slider: Entity,
}

impl Fixture {
    fn new() -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .add_message::<Ime>()
            .add_message::<WindowClosed>()
            .add_message::<WindowFocused>()
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<ObservedEvents>()
            .add_plugins((HeadlessDiegeticUiPlugin, WidgetInputPlugin))
            .add_observer(record_pressed)
            .add_observer(record_released)
            .add_observer(record_clicked)
            .add_observer(record_canceled)
            .add_observer(record_switch_request);
        app.finish();

        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        app.update();

        let mut builder = LayoutBuilder::new(100.0, 60.0);
        builder.with(El::new().button("button"), |_| {});
        builder.with(El::new().widget("toggle", Button::new().toggle()), |_| {});
        builder.with(El::new().size(24.0, 12.0).switch("switch"), |builder| {
            builder.with(El::new().size(10.0, 10.0).switch_knob(), |_| {});
        });
        builder.with(El::new().widget("slider", Slider::new(0.0..=1.0)), |_| {});
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(60.0))
            .with_tree(builder.build())
            .build();
        assert!(panel.is_ok());
        let Ok(panel) = panel else {
            return Self {
                app,
                window,
                button: Entity::PLACEHOLDER,
                toggle: Entity::PLACEHOLDER,
                switch: Entity::PLACEHOLDER,
                slider: Entity::PLACEHOLDER,
            };
        };
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let button = resolve_widget(&mut app, panel, "button");
        let toggle = resolve_widget(&mut app, panel, "toggle");
        let switch = resolve_widget(&mut app, panel, "switch");
        let slider = resolve_widget(&mut app, panel, "slider");
        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: button,
        });
        app.update();
        Self {
            app,
            window,
            button,
            toggle,
            switch,
            slider,
        }
    }

    fn events(&self) -> &[Event] { &self.app.world().resource::<ObservedEvents>().0 }

    fn focus(&mut self, widget: Entity) {
        self.app.world_mut().trigger(RequestWidgetFocus {
            window: self.window,
            widget,
        });
        self.app.update();
    }

    fn key(&mut self, key_code: KeyCode, state: ButtonState, repeat: bool) {
        self.queue_key(key_code, state, repeat);
        self.app.update();
    }

    fn queue_key(&mut self, key_code: KeyCode, state: ButtonState, repeat: bool) {
        self.app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state,
            text: None,
            repeat,
            window: self.window,
        });
    }

    fn press(&mut self, key: KeyCode) { self.key(key, ButtonState::Pressed, false); }

    fn release(&mut self, key: KeyCode) { self.key(key, ButtonState::Released, false); }

    fn pointer_press(&mut self, widget: Entity) {
        self.app.world_mut().trigger(PointerPress {
            entity:  widget,
            pointer: Pointer::new(PointerId::Mouse, pointer_location()),
            button:  PointerButton::Primary,
            hit:     pointer_hit(),
            count:   1,
        });
        self.app.world_mut().flush();
    }

    fn pointer_cancel(&mut self, widget: Entity) {
        self.app.world_mut().trigger(PointerCancel {
            entity:  widget,
            pointer: Pointer::new(PointerId::Mouse, pointer_location()),
            hit:     pointer_hit(),
        });
        self.app.world_mut().flush();
    }
}

fn resolve_widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
    let result = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named(id))
        });
    assert!(result.is_ok());
    let Ok(widget) = result else {
        return Entity::PLACEHOLDER;
    };
    assert!(widget.is_some());
    widget.unwrap_or(Entity::PLACEHOLDER)
}

const fn pointer_location() -> Location {
    Location {
        target:   bevy::camera::NormalizedRenderTarget::None {
            width:  1,
            height: 1,
        },
        position: Vec2::ZERO,
    }
}

const fn pointer_hit() -> HitData {
    HitData {
        camera:   Entity::PLACEHOLDER,
        depth:    0.0,
        position: None,
        normal:   None,
        extra:    None,
    }
}

const fn semantic_press(widget: Entity) -> Event {
    Event::Pressed(widget, ButtonPressSource::Semantic)
}

const fn semantic_release(widget: Entity) -> Event {
    Event::Released(widget, ButtonPressSource::Semantic)
}

const fn semantic_click(widget: Entity) -> Event {
    Event::Clicked(widget, ButtonActivationSource::Semantic)
}

const fn semantic_cancel(widget: Entity, cause: ButtonCancelCause) -> Event {
    Event::Canceled(widget, ButtonPressSource::Semantic, cause)
}

#[test]
fn space_press_reaches_activate_and_button_lifecycles_in_the_same_frame() {
    let mut fixture = Fixture::new();
    let mut cursor = MessageCursor::<WidgetInput>::default();

    fixture.queue_key(KeyCode::Space, ButtonState::Pressed, false);
    fixture.app.update();

    let inputs = cursor
        .read(fixture.app.world().resource::<Messages<WidgetInput>>())
        .collect::<Vec<_>>();
    assert!(matches!(
        inputs.as_slice(),
        [WidgetInput::ActivatePress { window }] if *window == fixture.window
    ));
    assert_eq!(fixture.events(), [semantic_press(fixture.button)]);
}

#[test]
fn momentary_button_stays_pressed_until_space_release_and_clicks_once() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    assert_eq!(fixture.events(), [semantic_press(fixture.button)]);
    fixture.app.update();
    assert_eq!(fixture.events(), [semantic_press(fixture.button)]);
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_release(fixture.button),
            semantic_click(fixture.button),
        ]
    );
    fixture.app.update();
    assert_eq!(fixture.events().len(), 3);
}

#[test]
fn toggle_latch_changes_once_on_release_and_not_during_hold() {
    let mut fixture = Fixture::new();
    fixture.focus(fixture.toggle);
    assert_eq!(
        fixture.app.world().get::<ButtonLatch>(fixture.toggle),
        Some(&ButtonLatch::Released)
    );
    fixture.press(KeyCode::Enter);
    assert_eq!(fixture.events(), [semantic_press(fixture.toggle)]);
    assert_eq!(
        fixture.app.world().get::<ButtonLatch>(fixture.toggle),
        Some(&ButtonLatch::Released)
    );
    fixture.release(KeyCode::Enter);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.toggle),
            semantic_release(fixture.toggle),
            semantic_click(fixture.toggle),
        ]
    );
    assert_eq!(
        fixture.app.world().get::<ButtonLatch>(fixture.toggle),
        Some(&ButtonLatch::Pressed)
    );
}

#[test]
fn switch_requests_one_toggle_only_after_release() {
    let mut fixture = Fixture::new();
    fixture.focus(fixture.switch);
    assert_eq!(
        fixture
            .app
            .world()
            .get::<SwitchState>(fixture.switch)
            .map(|state| state.side),
        Some(SwitchSide::On)
    );
    fixture.press(KeyCode::Space);
    assert_eq!(fixture.events(), [semantic_press(fixture.switch)]);
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events()[..2],
        [
            semantic_press(fixture.switch),
            semantic_release(fixture.switch)
        ]
    );
    assert_eq!(
        fixture
            .events()
            .iter()
            .filter(|event| **event == semantic_click(fixture.switch))
            .count(),
        1
    );
    assert_eq!(
        fixture
            .events()
            .iter()
            .filter(|event| **event == Event::SwitchRequested(fixture.switch, SwitchSide::Off))
            .count(),
        1
    );
    assert_eq!(fixture.events().len(), 4);
}

#[test]
fn escape_cancels_held_button_without_click() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.press(KeyCode::Escape);
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_cancel(fixture.button, ButtonCancelCause::Explicit)
        ]
    );
}

#[test]
fn focus_move_cancels_held_button_without_click() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.focus(fixture.slider);
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_cancel(fixture.button, ButtonCancelCause::FocusLost)
        ]
    );
}

#[test]
fn other_window_focus_loss_and_release_leave_held_press_intact() {
    let mut fixture = Fixture::new();
    let other_window = fixture.app.world_mut().spawn(Window::default()).id();
    fixture.app.world_mut().trigger(RequestWidgetFocus {
        window: other_window,
        widget: fixture.button,
    });
    fixture.app.update();

    fixture
        .app
        .world_mut()
        .write_message(WidgetInput::ActivatePress {
            window: fixture.window,
        });
    fixture.app.update();
    assert_eq!(fixture.events(), [semantic_press(fixture.button)]);
    fixture
        .app
        .world_mut()
        .write_message(WidgetInput::ActivateRelease {
            window: other_window,
        });
    fixture.app.update();
    fixture
        .app
        .world_mut()
        .write_message(WidgetInput::ActivateInterrupted {
            window: other_window,
        });
    fixture.app.update();
    fixture.app.world_mut().trigger(ClearWidgetFocus {
        window: other_window,
    });
    fixture.app.update();
    assert_eq!(fixture.events(), [semantic_press(fixture.button)]);

    fixture
        .app
        .world_mut()
        .write_message(WidgetInput::ActivateRelease {
            window: fixture.window,
        });
    fixture.app.update();
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_release(fixture.button),
            semantic_click(fixture.button),
        ]
    );
}

#[test]
fn window_blur_cancels_held_button_once_as_focus_lost() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    let window = fixture.app.world_mut().get_mut::<Window>(fixture.window);
    assert!(window.is_some());
    if let Some(mut window) = window {
        window.focused = false;
    }
    fixture.app.world_mut().write_message(WindowFocused {
        window:  fixture.window,
        focused: false,
    });
    fixture.app.update();
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_cancel(fixture.button, ButtonCancelCause::FocusLost)
        ]
    );
}

#[test]
fn disabling_button_cancels_held_press_without_click() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    let changed = fixture
        .app
        .world_mut()
        .run_system_once(move |mut writer: PanelWidgetWriter| {
            writer.override_interactivity(fixture.button, WidgetInteractivity::Disabled)
        });
    assert!(changed.is_ok());
    assert!(changed.unwrap_or(false));
    fixture.app.update();
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_cancel(fixture.button, ButtonCancelCause::Disabled)
        ]
    );
}

#[test]
fn removing_button_cancels_held_press_without_click() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.button)
        .remove::<PanelWidget>();
    fixture.app.update();
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_cancel(fixture.button, ButtonCancelCause::WidgetRemoved)
        ]
    );
}

#[test]
fn input_mode_change_cancels_held_press_as_interrupted() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.window)
        .insert(WidgetInputMode::Manual);
    fixture.app.update();
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_cancel(fixture.button, ButtonCancelCause::InputInterrupted)
        ]
    );
}

#[test]
fn adapter_teardown_cancels_held_press_as_interrupted() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.window)
        .insert(WidgetInputDisabled);
    fixture.app.update();
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_cancel(fixture.button, ButtonCancelCause::InputInterrupted)
        ]
    );
}

#[test]
fn pointer_press_is_refused_during_keyboard_hold() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.pointer_press(fixture.button);
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_release(fixture.button),
            semantic_click(fixture.button)
        ]
    );
}

#[test]
fn keyboard_press_is_refused_during_pointer_hold() {
    let mut fixture = Fixture::new();
    fixture.pointer_press(fixture.button);
    assert_eq!(
        fixture.events(),
        [Event::Pressed(
            fixture.button,
            ButtonPressSource::Pointer(PointerId::Mouse)
        )]
    );
    fixture.press(KeyCode::Space);
    fixture.release(KeyCode::Space);
    fixture.pointer_cancel(fixture.button);
    assert_eq!(
        fixture.events(),
        [
            Event::Pressed(fixture.button, ButtonPressSource::Pointer(PointerId::Mouse)),
            Event::Canceled(
                fixture.button,
                ButtonPressSource::Pointer(PointerId::Mouse),
                ButtonCancelCause::PointerCanceled
            ),
        ]
    );
}

#[test]
fn key_repeat_never_starts_another_press() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.key(KeyCode::Space, ButtonState::Pressed, true);
    fixture.app.update();
    assert_eq!(fixture.events(), [semantic_press(fixture.button)]);
    fixture.release(KeyCode::Space);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_release(fixture.button),
            semantic_click(fixture.button)
        ]
    );
}

#[test]
fn same_frame_release_then_press_completes_old_press_and_starts_new_hold() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.queue_key(KeyCode::Space, ButtonState::Released, false);
    fixture.queue_key(KeyCode::Enter, ButtonState::Pressed, false);
    fixture.app.update();
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_release(fixture.button),
            semantic_click(fixture.button),
            semantic_press(fixture.button),
        ]
    );
    fixture.app.update();
    assert_eq!(fixture.events().len(), 4, "the second hold remains pending");
    fixture.release(KeyCode::Enter);
    assert_eq!(
        fixture.events(),
        [
            semantic_press(fixture.button),
            semantic_release(fixture.button),
            semantic_click(fixture.button),
            semantic_press(fixture.button),
            semantic_release(fixture.button),
            semantic_click(fixture.button),
        ]
    );
}
