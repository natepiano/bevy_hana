//! Public-client coverage for held widget activation edges.
#![allow(clippy::panic, reason = "tests should panic on invalid fixtures")]

use bevy::ecs::message::MessageCursor;
use bevy::ecs::system::RunSystemOnce;
use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::Key;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::keyboard::NativeKey;
use bevy::prelude::*;
use bevy::window::Ime;
use bevy::window::WindowClosed;
use bevy::window::WindowFocused;
use bevy_enhanced_input::prelude::*;
use hana_diegetic::ButtonActivationSource;
use hana_diegetic::ButtonClicked;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::PanelWidgetWriter;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::Slider;
use hana_diegetic::WidgetInput;
use hana_diegetic::WidgetInputDisabled;
use hana_diegetic::WidgetInputMode;
use hana_diegetic::WidgetInputPlugin;
use hana_diegetic::WidgetInteractivity;
use hana_rubric::HoldLatch;

#[derive(Component)]
struct ApplicationInputContext;

hana_rubric::action!(
    /// Lower-priority application action bound to Space.
    ApplicationSpace
);

#[derive(Default, Resource)]
struct ObservedActions {
    app_space: usize,
    clicks:    usize,
}

fn record_app_space(_: On<Start<ApplicationSpace>>, mut observed: ResMut<ObservedActions>) {
    observed.app_space += 1;
}

fn record_button_click(event: On<ButtonClicked>, mut observed: ResMut<ObservedActions>) {
    if event.source == ButtonActivationSource::Semantic {
        observed.clicks += 1;
    }
}

fn spawn_application_space(spawner: &mut ActionSpawner<ApplicationInputContext>) {
    spawner.spawn((
        Action::<ApplicationSpace>::new(),
        ActionSettings {
            consume_input: true,
            ..default()
        },
        related!(Bindings[Binding::from(KeyCode::Space)]),
    ));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Edge {
    Press,
    Release,
    Interrupted,
}

struct Fixture {
    app:    App,
    window: Entity,
    button: Entity,
    slider: Entity,
    cursor: MessageCursor<WidgetInput>,
    edges:  Vec<Edge>,
}

impl Fixture {
    fn new() -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .add_message::<Ime>()
            .add_message::<WindowClosed>()
            .add_message::<WindowFocused>()
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<ObservedActions>()
            .add_plugins((HeadlessDiegeticUiPlugin, WidgetInputPlugin))
            .add_input_context::<ApplicationInputContext>()
            .add_observer(record_app_space)
            .add_observer(record_button_click);
        app.finish();

        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        app.update();
        app.world_mut().spawn((
            ApplicationInputContext,
            ContextPriority::<ApplicationInputContext>::new(0),
            Actions::<ApplicationInputContext>::spawn(SpawnWith(spawn_application_space)),
        ));

        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("action"), |_| {});
        builder.with(
            El::new().widget("amount", Slider::new(0.0..=1.0).value(0.5)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build();
        let Ok(panel) = panel else {
            panic!("valid panel");
        };
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let button = resolve_widget(&mut app, panel, "action");
        let slider = resolve_widget(&mut app, panel, "amount");
        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: button,
        });
        app.update();

        Self {
            app,
            window,
            button,
            slider,
            cursor: MessageCursor::default(),
            edges: Vec::new(),
        }
    }

    fn update(&mut self) {
        self.app.update();
        let messages = self.app.world().resource::<Messages<WidgetInput>>();
        self.edges
            .extend(self.cursor.read(messages).filter_map(|input| {
                assert_eq!(input.window(), self.window);
                match input {
                    WidgetInput::ActivatePress { .. } => Some(Edge::Press),
                    WidgetInput::ActivateRelease { .. } => Some(Edge::Release),
                    WidgetInput::ActivateInterrupted { .. } => Some(Edge::Interrupted),
                    _ => None,
                }
            }));
    }

    fn key(&mut self, key_code: KeyCode, state: ButtonState) {
        self.app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state,
            text: None,
            repeat: false,
            window: self.window,
        });
        self.update();
    }

    fn press(&mut self, key: KeyCode) { self.key(key, ButtonState::Pressed); }

    fn release(&mut self, key: KeyCode) { self.key(key, ButtonState::Released); }

    fn app_space(&self) -> usize { self.app.world().resource::<ObservedActions>().app_space }

    fn clicks(&self) -> usize { self.app.world().resource::<ObservedActions>().clicks }

    fn held_actions(&mut self) -> Vec<Entity> {
        let mut query = self
            .app
            .world_mut()
            .query_filtered::<Entity, With<HoldLatch>>();
        query.iter(self.app.world()).collect()
    }

    fn active_hold(&mut self) -> Entity {
        let mut query = self
            .app
            .world_mut()
            .query_filtered::<(Entity, &TriggerState), With<HoldLatch>>();
        let active = query
            .iter(self.app.world())
            .find_map(|(entity, state)| (*state == TriggerState::Fired).then_some(entity));
        let Some(active) = active else {
            panic!("one active held shortcut");
        };
        active
    }
}

fn resolve_widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
    let widget = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named(id))
        });
    let Ok(Some(widget)) = widget else {
        panic!("widget exists");
    };
    widget
}

#[test]
fn space_hold_emits_one_press_and_one_release() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press]);
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Release]);
}

#[test]
fn overlapping_space_and_enter_emit_one_press_and_one_terminal_after_both_release() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.press(KeyCode::Enter);
    fixture.release(KeyCode::Space);
    assert_eq!(fixture.edges, [Edge::Press]);
    fixture.release(KeyCode::Enter);
    assert_eq!(fixture.edges, [Edge::Press, Edge::Release]);
}

#[test]
fn releasing_space_and_pressing_enter_in_one_frame_emits_both_edges_and_clicks_twice() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    assert_eq!(fixture.clicks(), 0);

    fixture.app.world_mut().write_message(KeyboardInput {
        key_code:    KeyCode::Space,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state:       ButtonState::Released,
        text:        None,
        repeat:      false,
        window:      fixture.window,
    });
    fixture.app.world_mut().write_message(KeyboardInput {
        key_code:    KeyCode::Enter,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state:       ButtonState::Pressed,
        text:        None,
        repeat:      false,
        window:      fixture.window,
    });
    fixture.update();

    assert_eq!(fixture.edges, [Edge::Press, Edge::Release, Edge::Press]);
    assert_eq!(fixture.clicks(), 1);
    fixture.release(KeyCode::Enter);
    assert_eq!(
        fixture.edges,
        [Edge::Press, Edge::Release, Edge::Press, Edge::Release]
    );
    assert_eq!(fixture.clicks(), 2);
}

#[test]
fn input_disabled_mid_hold_emits_one_interruption() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.window)
        .insert(WidgetInputDisabled);
    fixture.update();
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
}

#[test]
fn focused_button_clicks_once_on_the_release_edge() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.clicks(), 0);
    fixture.release(KeyCode::Space);
    assert_eq!(fixture.clicks(), 1);
    assert_eq!(fixture.edges, [Edge::Press, Edge::Release]);
}

#[test]
fn input_mode_change_mid_hold_emits_one_interruption() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.window)
        .insert(WidgetInputMode::Manual);
    fixture.update();
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
}

#[test]
fn window_blur_mid_hold_emits_one_interruption() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    {
        let mut window_entity = fixture.app.world_mut().entity_mut(fixture.window);
        let Some(mut window) = window_entity.get_mut::<Window>() else {
            panic!("window exists");
        };
        window.focused = false;
    }
    fixture.app.world_mut().write_message(WindowFocused {
        window:  fixture.window,
        focused: false,
    });
    fixture.update();
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
}

#[test]
fn absent_hold_action_entities_emit_one_interruption() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    for action in fixture.held_actions() {
        fixture.app.world_mut().despawn(action);
    }
    fixture.update();
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
}

#[test]
fn removing_widget_input_mode_tears_down_a_hold_with_one_interruption() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.window)
        .remove::<WidgetInputMode>();
    fixture.update();
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
}

#[test]
fn externally_despawned_active_hold_interrupts_and_never_releases() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    let held_action = fixture.active_hold();
    fixture.app.world_mut().despawn(held_action);
    fixture.update();
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
}

#[test]
fn focus_change_to_nonactivating_widget_interrupts_and_frees_held_space() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    fixture.app.world_mut().trigger(RequestWidgetFocus {
        window: fixture.window,
        widget: fixture.slider,
    });
    fixture.update();
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
    assert_eq!(fixture.app_space(), 1);
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
}

#[test]
fn disabling_focused_button_interrupts_and_frees_held_space() {
    let mut fixture = Fixture::new();
    fixture.press(KeyCode::Space);
    let button = fixture.button;
    let changed = fixture
        .app
        .world_mut()
        .run_system_once(move |mut writer: PanelWidgetWriter| {
            writer.override_interactivity(button, WidgetInteractivity::Disabled)
        });
    let Ok(changed) = changed else {
        panic!("writer runs");
    };
    assert!(changed);
    fixture.update();
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
    assert_eq!(fixture.app_space(), 1);
    fixture.release(KeyCode::Space);
    fixture.update();
    assert_eq!(fixture.edges, [Edge::Press, Edge::Interrupted]);
}
