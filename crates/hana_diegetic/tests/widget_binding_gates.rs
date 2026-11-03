//! Public-client coverage for widget binding consumption and fall-through.

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
use bevy_enhanced_input::prelude::*;
use hana_diegetic::ButtonActivationSource;
use hana_diegetic::ButtonCancelCause;
use hana_diegetic::ButtonCanceled;
use hana_diegetic::ButtonClicked;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeInputBlocker;
use hana_diegetic::ImeOpenSession;
use hana_diegetic::ImeTarget;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::RequestPanelFocus;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::Slider;
use hana_diegetic::WidgetInputPlugin;
use hana_rubric::Keybindings;

#[derive(Component)]
struct ApplicationInputContext;

hana_rubric::action!(
    /// Application binding on Space.
    ApplicationSpace
);
hana_rubric::action!(
    /// Application binding on Enter.
    ApplicationEnter
);
hana_rubric::action!(
    /// Application binding on Escape.
    ApplicationEscape
);
hana_rubric::action!(
    /// Modifier action for exact application bindings.
    ApplicationShift
);

#[derive(Default, Resource)]
struct ObservedInput {
    app_space:      usize,
    app_enter:      usize,
    app_escape:     usize,
    button_clicks:  usize,
    button_cancels: usize,
}

#[derive(Resource)]
struct FocusAfterInput {
    window: Entity,
    widget: Entity,
}

fn focus_after_input(world: &mut World) {
    if let Some(focus) = world.remove_resource::<FocusAfterInput>() {
        world.trigger(RequestWidgetFocus {
            window: focus.window,
            widget: focus.widget,
        });
    }
}

fn record_space(_: On<Start<ApplicationSpace>>, mut observed: ResMut<ObservedInput>) {
    observed.app_space += 1;
}

fn record_enter(_: On<Start<ApplicationEnter>>, mut observed: ResMut<ObservedInput>) {
    observed.app_enter += 1;
}

fn record_escape(_: On<Start<ApplicationEscape>>, mut observed: ResMut<ObservedInput>) {
    observed.app_escape += 1;
}

fn record_click(event: On<ButtonClicked>, mut observed: ResMut<ObservedInput>) {
    if event.source == ButtonActivationSource::Semantic {
        observed.button_clicks += 1;
    }
}

fn record_cancel(event: On<ButtonCanceled>, mut observed: ResMut<ObservedInput>) {
    if event.cause == ButtonCancelCause::Explicit {
        observed.button_cancels += 1;
    }
}

fn spawn_application_actions(spawner: &mut ActionSpawner<ApplicationInputContext>) {
    let bindings = Keybindings::new::<ApplicationShift>(
        spawner,
        ActionSettings {
            consume_input: true,
            require_reset: true,
            ..default()
        },
    );
    bindings.spawn_key::<ApplicationSpace>(spawner, KeyCode::Space);
    bindings.spawn_key::<ApplicationEnter>(spawner, KeyCode::Enter);
    bindings.spawn_key::<ApplicationEscape>(spawner, KeyCode::Escape);
}

fn test_app() -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, InputPlugin))
        .add_message::<Ime>()
        .add_message::<WindowClosed>()
        .add_message::<WindowFocused>()
        .insert_resource(DiegeticTextMeasurer::default())
        .init_resource::<ObservedInput>()
        .add_plugins((HeadlessDiegeticUiPlugin, WidgetInputPlugin))
        .add_input_context::<ApplicationInputContext>()
        .add_observer(record_space)
        .add_observer(record_enter)
        .add_observer(record_escape)
        .add_observer(record_click)
        .add_observer(record_cancel);
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
        Actions::<ApplicationInputContext>::spawn(SpawnWith(spawn_application_actions)),
    ));
    app.update();
    (app, window)
}

fn spawn_panel(app: &mut App) -> Entity {
    let mut builder = LayoutBuilder::new(100.0, 50.0);
    builder.with(El::new().button("action"), |_| {});
    builder.with(
        El::new().widget("amount", Slider::new(0.0..=1.0).value(0.5)),
        |_| {},
    );
    let result = DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .with_tree(builder.build())
        .build();
    assert!(result.is_ok());
    let Ok(panel) = result else {
        return Entity::PLACEHOLDER;
    };
    let panel = app.world_mut().spawn(panel).id();
    app.update();
    panel
}

fn widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
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
    let Some(widget) = widget else {
        return Entity::PLACEHOLDER;
    };
    widget
}

fn press_key(app: &mut App, window: Entity, key_code: KeyCode) {
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window,
    });
}

fn release_key(app: &mut App, window: Entity, key_code: KeyCode) {
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: ButtonState::Released,
        text: None,
        repeat: false,
        window,
    });
}

fn pointer_press(app: &mut App, widget: Entity) {
    app.world_mut().trigger(bevy::prelude::PointerPress {
        entity:  widget,
        pointer: Pointer::new(
            PointerId::Mouse,
            Location {
                target:   bevy::camera::NormalizedRenderTarget::None {
                    width:  1,
                    height: 1,
                },
                position: Vec2::ZERO,
            },
        ),
        button:  PointerButton::Primary,
        hit:     HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
        count:   1,
    });
    app.world_mut().flush();
}

#[test]
fn space_falls_through_without_focus_and_is_consumed_by_a_focused_button() {
    let (mut app, window) = test_app();
    let panel = spawn_panel(&mut app);
    let button = widget(&mut app, panel, "action");
    app.world_mut().trigger(RequestPanelFocus { window, panel });
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    app.update();
    assert_eq!(app.world().resource::<ObservedInput>().app_space, 1);
    assert_eq!(app.world().resource::<ObservedInput>().button_clicks, 0);

    release_key(&mut app, window, KeyCode::Space);
    app.update();
    app.world_mut().trigger(RequestWidgetFocus {
        window,
        widget: button,
    });
    app.update();
    press_key(&mut app, window, KeyCode::Space);
    app.update();
    assert_eq!(app.world().resource::<ObservedInput>().button_clicks, 0);
    release_key(&mut app, window, KeyCode::Space);
    app.update();
    let observed = app.world().resource::<ObservedInput>();
    assert_eq!(observed.app_space, 1);
    assert_eq!(observed.button_clicks, 1);
}

#[test]
fn escape_falls_through_without_a_press_and_cancels_a_pointer_pressed_button() {
    let (mut app, window) = test_app();
    let panel = spawn_panel(&mut app);
    let button = widget(&mut app, panel, "action");
    app.world_mut().trigger(RequestWidgetFocus {
        window,
        widget: button,
    });
    app.update();

    press_key(&mut app, window, KeyCode::Escape);
    app.update();
    assert_eq!(app.world().resource::<ObservedInput>().app_escape, 1);
    assert_eq!(app.world().resource::<ObservedInput>().button_cancels, 0);

    release_key(&mut app, window, KeyCode::Escape);
    app.update();
    pointer_press(&mut app, button);
    press_key(&mut app, window, KeyCode::Escape);
    app.update();
    let observed = app.world().resource::<ObservedInput>();
    assert_eq!(observed.app_escape, 1);
    assert_eq!(observed.button_cancels, 1);
}

#[test]
fn focused_slider_allows_space_and_enter_application_bindings() {
    let (mut app, window) = test_app();
    let panel = spawn_panel(&mut app);
    let slider = widget(&mut app, panel, "amount");
    app.world_mut().trigger(RequestWidgetFocus {
        window,
        widget: slider,
    });
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    press_key(&mut app, window, KeyCode::Enter);
    app.update();
    let observed = app.world().resource::<ObservedInput>();
    assert_eq!(observed.app_space, 1);
    assert_eq!(observed.app_enter, 1);
    assert_eq!(observed.button_clicks, 0);
}

#[test]
fn focus_change_and_space_press_reach_at_most_one_consumer() {
    let (mut app, window) = test_app();
    let panel = spawn_panel(&mut app);
    let button = widget(&mut app, panel, "action");
    app.world_mut().trigger(RequestPanelFocus { window, panel });
    app.update();

    app.world_mut().insert_resource(FocusAfterInput {
        window,
        widget: button,
    });
    app.add_systems(
        PreUpdate,
        focus_after_input.after(EnhancedInputSystems::Update),
    );
    press_key(&mut app, window, KeyCode::Space);
    app.update();
    let observed = app.world().resource::<ObservedInput>();
    assert!(observed.app_space + observed.button_clicks <= 1);
    assert_eq!(observed.app_space, 1);
    assert_eq!(observed.button_clicks, 0);
}

#[test]
fn ime_edit_start_and_space_press_reach_at_most_one_consumer() {
    let (mut app, window) = test_app();
    let panel = spawn_panel(&mut app);
    let button = widget(&mut app, panel, "action");
    app.world_mut().trigger(RequestWidgetFocus {
        window,
        widget: button,
    });
    app.update();
    let owner = app.world_mut().spawn_empty().id();

    app.world_mut().trigger(ImeOpenSession {
        target: ImeTarget::AppOwned {
            owner,
            field_id: PanelElementId::named("editor"),
        },
        window,
        initial_text: String::new(),
        field_spec: ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("editor")),
        anchor: None,
    });
    press_key(&mut app, window, KeyCode::Space);
    app.update();
    let observed = app.world().resource::<ObservedInput>();
    assert!(observed.app_space + observed.button_clicks <= 1);
    assert!(
        app.world()
            .resource::<ImeInputBlocker>()
            .blocks_window(window)
    );
}

#[test]
fn simultaneous_activate_and_escape_each_reach_at_most_one_consumer() {
    let (mut app, window) = test_app();
    let panel = spawn_panel(&mut app);
    let button = widget(&mut app, panel, "action");
    app.world_mut().trigger(RequestWidgetFocus {
        window,
        widget: button,
    });
    app.update();

    press_key(&mut app, window, KeyCode::Space);
    press_key(&mut app, window, KeyCode::Escape);
    app.update();
    let observed = app.world().resource::<ObservedInput>();
    assert_eq!(observed.button_clicks, 0);
    assert_eq!(observed.app_escape, 1);

    release_key(&mut app, window, KeyCode::Space);
    app.update();
    let observed = app.world().resource::<ObservedInput>();
    assert!(observed.app_space + observed.button_clicks <= 1);
    assert!(observed.app_escape + observed.button_cancels <= 1);
    assert_eq!(observed.button_clicks, 1);
    assert_eq!(observed.app_escape, 1);
}
