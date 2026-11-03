//! Public-client coverage for focused slider arrow stepping.
#![allow(clippy::panic, reason = "tests should panic on invalid fixtures")]

use std::time::Duration;

use bevy::camera::NormalizedRenderTarget;
use bevy::camera::PerspectiveProjection;
use bevy::camera::Projection;
use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
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
use bevy::prelude::PointerPress;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::window::Ime;
use bevy::window::PrimaryWindow;
use bevy::window::WindowClosed;
use bevy::window::WindowFocused;
use bevy::window::WindowRef;
use bevy_enhanced_input::prelude::*;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeOpenSession;
use hana_diegetic::ImeTarget;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::PanelWidgetWriter;
use hana_diegetic::RequestSliderAdjustment;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::Slider;
use hana_diegetic::SliderAdjustment;
use hana_diegetic::SliderChangeRequested;
use hana_diegetic::SliderDirection;
use hana_diegetic::SliderProposalSource;
use hana_diegetic::SliderState;
use hana_diegetic::WidgetControlSummary;
use hana_diegetic::WidgetInput;
use hana_diegetic::WidgetInputBindings;
use hana_diegetic::WidgetInputBindingsError;
use hana_diegetic::WidgetInputMode;
use hana_diegetic::WidgetInputPlugin;
use hana_diegetic::WidgetInteractivity;
use hana_diegetic::WidgetStepDirection;
use hana_diegetic::slider_self_update;

const VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Proposal {
    value:  f32,
    source: SliderProposalSource,
}

#[derive(Default, Resource)]
struct Observed {
    proposals:          Vec<Proposal>,
    application_arrows: usize,
}

fn record_proposal(event: On<SliderChangeRequested>, mut observed: ResMut<Observed>) {
    observed.proposals.push(Proposal {
        value:  event.value.get(),
        source: event.source,
    });
}

#[derive(Component)]
struct ApplicationContext;

hana_rubric::action!(
    /// Lower-priority application action bound to the right arrow.
    ApplicationArrow
);

fn record_application_arrow(_: On<Start<ApplicationArrow>>, mut observed: ResMut<Observed>) {
    observed.application_arrows += 1;
}

fn spawn_application_arrow(spawner: &mut ActionSpawner<ApplicationContext>) {
    spawner.spawn((
        Action::<ApplicationArrow>::new(),
        related!(Bindings[Binding::from(KeyCode::ArrowRight)]),
    ));
}

struct Fixture {
    app:    App,
    window: Entity,
    slider: Entity,
    camera: Entity,
    target: NormalizedRenderTarget,
    cursor: MessageCursor<WidgetInput>,
    steps:  Vec<WidgetStepDirection>,
}

impl Fixture {
    fn new(slider: Slider) -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .add_message::<Ime>()
            .add_message::<WindowClosed>()
            .add_message::<WindowFocused>()
            .insert_resource(DiegeticTextMeasurer::default())
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                60,
            )))
            .init_resource::<Observed>()
            .add_plugins((HeadlessDiegeticUiPlugin, WidgetInputPlugin))
            .add_input_context::<ApplicationContext>()
            .add_observer(record_proposal)
            .add_observer(slider_self_update)
            .add_observer(record_application_arrow);
        app.finish();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        app.update();
        app.world_mut().spawn((
            ApplicationContext,
            ContextPriority::<ApplicationContext>::new(0),
            Actions::<ApplicationContext>::spawn(SpawnWith(spawn_application_arrow)),
        ));
        let camera_transform = Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: VIEWPORT.as_uvec2(),
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(VIEWPORT.x, VIEWPORT.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        let camera = app
            .world_mut()
            .spawn((
                camera,
                camera_transform,
                GlobalTransform::from(camera_transform),
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id();
        let target = RenderTarget::Window(WindowRef::Primary).normalize(Some(window));
        let Some(target) = target else {
            panic!("window target normalizes")
        };
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(20.0, 10.0).widget("amount", slider), |_| {});
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .world_height(0.5)
            .with_tree(builder.build())
            .build();
        let Ok(panel) = panel else {
            panic!("valid slider panel")
        };
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let slider = resolve_widget(&mut app, panel);
        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: slider,
        });
        app.update();
        Self {
            app,
            window,
            slider,
            camera,
            target,
            cursor: MessageCursor::default(),
            steps: Vec::new(),
        }
    }

    fn update(&mut self) {
        self.app.update();
        let messages = self.app.world().resource::<Messages<WidgetInput>>();
        self.steps
            .extend(self.cursor.read(messages).filter_map(|input| match input {
                WidgetInput::Step { window, direction } if *window == self.window => {
                    Some(*direction)
                },
                _ => None,
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

    fn press(&mut self, key_code: KeyCode) { self.key(key_code, ButtonState::Pressed); }
    fn release(&mut self, key_code: KeyCode) { self.key(key_code, ButtonState::Released); }
    fn step(&mut self, direction: WidgetStepDirection) {
        self.app.world_mut().write_message(WidgetInput::Step {
            window: self.window,
            direction,
        });
        self.update();
    }
    fn proposals(&self) -> &[Proposal] { &self.app.world().resource::<Observed>().proposals }
    fn value(&self) -> f32 {
        let Some(state) = self.app.world().get::<SliderState>(self.slider) else {
            panic!("reified slider state")
        };
        state.value()
    }
    fn application_arrows(&self) -> usize {
        self.app.world().resource::<Observed>().application_arrows
    }
    fn pointer_press(&mut self) {
        self.pointer_press_without_flush();
        self.app.world_mut().flush();
    }
    fn pointer_press_without_flush(&mut self) {
        self.app.world_mut().trigger(PointerPress {
            entity:  self.slider,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   self.target.clone(),
                    position: VIEWPORT * 0.5,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(self.camera, 0.0, None, None),
            count:   1,
        });
    }
}

fn resolve_widget(app: &mut App, panel: Entity) -> Entity {
    let result = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named("amount"))
        });
    let Ok(Some(widget)) = result else {
        panic!("slider reifies")
    };
    widget
}

fn unit_slider() -> Slider { Slider::new(0.0..=1.0).value(0.5) }

#[test]
fn arrow_press_emits_one_widget_step_and_one_semantic_proposal() {
    let mut fixture = Fixture::new(unit_slider());
    fixture.press(KeyCode::ArrowRight);
    assert_eq!(fixture.steps, [WidgetStepDirection::Right]);
    assert_eq!(
        fixture.proposals(),
        [Proposal {
            value:  0.55,
            source: SliderProposalSource::Semantic,
        }]
    );
    fixture.release(KeyCode::ArrowRight);
}

#[test]
fn each_slider_direction_maps_all_arrow_keys_to_one_step() {
    let rows: [(SliderDirection, [f32; 4]); 4] = [
        (SliderDirection::LeftToRight, [1.0, -1.0, 1.0, -1.0]),
        (SliderDirection::RightToLeft, [-1.0, 1.0, 1.0, -1.0]),
        (SliderDirection::BottomToTop, [1.0, -1.0, 1.0, -1.0]),
        (SliderDirection::TopToBottom, [1.0, -1.0, -1.0, 1.0]),
    ];
    let directions = [
        WidgetStepDirection::Right,
        WidgetStepDirection::Left,
        WidgetStepDirection::Up,
        WidgetStepDirection::Down,
    ];
    for (axis, signs) in rows {
        for (direction, sign) in directions.into_iter().zip(signs) {
            let mut fixture = Fixture::new(unit_slider().step(0.1).direction(axis));
            fixture.step(direction);
            let [proposal] = fixture.proposals() else {
                panic!("one stepped proposal")
            };
            assert!(
                (proposal.value - sign.mul_add(0.1, 0.5)).abs() < 1e-6,
                "{axis:?} {direction:?}"
            );
            assert_eq!(proposal.source, SliderProposalSource::Semantic);
        }
    }
}

#[test]
fn stepless_slider_moves_by_one_twentieth_of_its_span() {
    let mut fixture = Fixture::new(Slider::new(10.0..=30.0).value(15.0));
    fixture.step(WidgetStepDirection::Right);
    assert_eq!(
        fixture.proposals(),
        [Proposal {
            value:  16.0,
            source: SliderProposalSource::Semantic,
        }]
    );
}

#[test]
fn full_f32_range_stepless_steps_remain_finite_and_clamped() {
    for (initial, direction, increases) in [
        (f32::MIN, WidgetStepDirection::Right, true),
        (f32::MAX, WidgetStepDirection::Left, false),
    ] {
        let mut fixture = Fixture::new(Slider::new(f32::MIN..=f32::MAX).value(initial));
        for _ in 0..25 {
            let count = fixture.proposals().len();
            fixture.step(direction);
            assert_eq!(fixture.proposals().len(), count + 1);
            let Some(proposal) = fixture.proposals().last() else {
                panic!("step proposal")
            };
            assert!(proposal.value.is_finite());
            assert!((f32::MIN..=f32::MAX).contains(&proposal.value));
            assert_eq!(proposal.source, SliderProposalSource::Semantic);
        }
        if increases {
            assert!(fixture.value() > initial, "{direction:?}");
        } else {
            assert!(fixture.value() < initial, "{direction:?}");
        }
    }
}

#[test]
fn request_adjustment_uses_request_source() {
    let mut fixture = Fixture::new(unit_slider());
    fixture.app.world_mut().trigger(RequestSliderAdjustment {
        entity:     fixture.slider,
        adjustment: SliderAdjustment::Absolute(0.75),
    });
    fixture.app.world_mut().flush();
    assert_eq!(
        fixture.proposals(),
        [Proposal {
            value:  0.75,
            source: SliderProposalSource::Request,
        }]
    );
}

#[test]
fn disabled_slider_ignores_step() {
    let mut fixture = Fixture::new(unit_slider());
    let slider = fixture.slider;
    let changed = fixture
        .app
        .world_mut()
        .run_system_once(move |mut writer: PanelWidgetWriter| {
            writer.override_interactivity(slider, WidgetInteractivity::Disabled)
        });
    let Ok(changed) = changed else {
        panic!("writer runs")
    };
    assert!(changed);
    fixture.update();
    fixture.step(WidgetStepDirection::Right);
    assert_eq!(fixture.proposals(), []);
}

#[test]
fn pointer_drag_ignores_step() {
    let mut fixture = Fixture::new(unit_slider());
    fixture.pointer_press();
    fixture.update();
    let count = fixture.proposals().len();
    fixture.step(WidgetStepDirection::Right);
    assert_eq!(fixture.proposals().len(), count);
}

#[test]
fn pointer_drag_wins_over_step_in_the_next_update() {
    let mut fixture = Fixture::new(unit_slider());
    fixture.pointer_press_without_flush();
    // `step` runs an update, which can apply the deferred `SliderDrag` marker.
    // The inline slider test checks the capture record before that flush.
    fixture.step(WidgetStepDirection::Right);
    assert_eq!(fixture.proposals().len(), 1);
    assert!(
        fixture
            .proposals()
            .iter()
            .all(|proposal| { proposal.source == SliderProposalSource::Pointer(PointerId::Mouse) })
    );
}

#[test]
fn ime_lease_consumes_arrow_without_step_or_application_action() {
    let mut fixture = Fixture::new(unit_slider());
    let owner = fixture.app.world_mut().spawn_empty().id();
    fixture.app.world_mut().trigger(ImeOpenSession {
        target:       ImeTarget::AppOwned {
            owner,
            field_id: PanelElementId::named("field"),
        },
        window:       fixture.window,
        initial_text: String::new(),
        field_spec:   ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("field")),
        anchor:       None,
    });
    fixture.app.world_mut().flush();
    fixture.press(KeyCode::ArrowRight);
    assert_eq!(fixture.proposals(), []);
    assert_eq!(fixture.application_arrows(), 0);
    fixture.release(KeyCode::ArrowRight);
}

#[test]
fn default_bindings_label_the_four_arrow_keys() {
    let WidgetControlSummary {
        step_left,
        step_right,
        step_up,
        step_down,
        ..
    } = WidgetInputMode::Default.control_summary();
    assert_eq!(
        step_left,
        vec![Binding::from(KeyCode::ArrowLeft).to_string()]
    );
    assert_eq!(
        step_right,
        vec![Binding::from(KeyCode::ArrowRight).to_string()]
    );
    assert_eq!(step_up, vec![Binding::from(KeyCode::ArrowUp).to_string()]);
    assert_eq!(
        step_down,
        vec![Binding::from(KeyCode::ArrowDown).to_string()]
    );
    let summary = WidgetInputMode::Bindings(WidgetInputBindings::default()).control_summary();
    assert_eq!(
        summary.step_right,
        vec![Binding::from(KeyCode::ArrowRight).to_string()]
    );
}

#[test]
fn custom_step_bindings_route_each_direction_and_leave_arrow_for_application() {
    let bindings = WidgetInputBindings::builder()
        .step_left(KeyCode::KeyA)
        .step_right(KeyCode::KeyD)
        .step_up(KeyCode::KeyW)
        .step_down(KeyCode::KeyS)
        .build();
    let Ok(bindings) = bindings else {
        panic!("valid custom step bindings")
    };
    let mode = WidgetInputMode::Bindings(bindings);
    let summary = mode.control_summary();
    assert_eq!(
        summary.step_left,
        vec![Binding::from(KeyCode::KeyA).to_string()]
    );
    assert_eq!(
        summary.step_right,
        vec![Binding::from(KeyCode::KeyD).to_string()]
    );
    assert_eq!(
        summary.step_up,
        vec![Binding::from(KeyCode::KeyW).to_string()]
    );
    assert_eq!(
        summary.step_down,
        vec![Binding::from(KeyCode::KeyS).to_string()]
    );

    let mut fixture = Fixture::new(unit_slider());
    fixture
        .app
        .world_mut()
        .entity_mut(fixture.window)
        .insert(mode);
    fixture.update();
    fixture.update();
    for key in [KeyCode::KeyA, KeyCode::KeyD, KeyCode::KeyW, KeyCode::KeyS] {
        fixture.press(key);
        fixture.release(key);
    }
    assert_eq!(
        fixture.steps,
        [
            WidgetStepDirection::Left,
            WidgetStepDirection::Right,
            WidgetStepDirection::Up,
            WidgetStepDirection::Down,
        ]
    );
    assert_eq!(fixture.proposals().len(), 4);
    assert!(
        fixture
            .proposals()
            .iter()
            .all(|proposal| proposal.source == SliderProposalSource::Semantic)
    );
    fixture.press(KeyCode::ArrowRight);
    assert_eq!(fixture.steps.len(), 4);
    assert_eq!(fixture.application_arrows(), 1);
    fixture.release(KeyCode::ArrowRight);
}

#[test]
fn duplicate_step_bindings_are_rejected_by_the_builder() {
    let result = WidgetInputBindings::builder()
        .step_left(KeyCode::KeyA)
        .step_right(KeyCode::KeyA)
        .build();
    assert_eq!(
        result,
        Err(WidgetInputBindingsError::ConflictingBinding(
            KeyCode::KeyA.into()
        ))
    );
}

#[test]
fn held_arrow_repeats_after_initial_delay_and_each_pulse() {
    let mut fixture = Fixture::new(unit_slider());
    fixture.press(KeyCode::ArrowRight);
    assert_eq!(fixture.steps.len(), 1);
    assert_eq!(fixture.proposals().len(), 1);
    for _ in 0..5 {
        fixture.update();
        assert_eq!(fixture.steps.len(), 1);
        assert_eq!(fixture.proposals().len(), 1);
    }
    fixture.update();
    assert_eq!(fixture.steps.len(), 2);
    assert_eq!(fixture.proposals().len(), 2);
    for _ in 0..2 {
        fixture.update();
        assert_eq!(fixture.steps.len(), 2);
    }
    fixture.update();
    assert_eq!(fixture.steps.len(), 3);
    assert_eq!(fixture.proposals().len(), 3);
    assert!(
        fixture
            .proposals()
            .iter()
            .all(|proposal| proposal.source == SliderProposalSource::Semantic)
    );
    assert_eq!(fixture.application_arrows(), 0);
    fixture.release(KeyCode::ArrowRight);
    assert_eq!(fixture.steps.len(), 3);
    assert_eq!(fixture.proposals().len(), 3);
}

#[test]
fn focus_change_mid_arrow_hold_ends_steps_and_frees_arrow_until_fresh_press() {
    let mut fixture = Fixture::new(unit_slider());
    let mut builder = LayoutBuilder::new(100.0, 50.0);
    builder.with(
        El::new().size(20.0, 10.0).widget("amount", unit_slider()),
        |_| {},
    );
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .world_height(0.5)
        .with_tree(builder.build())
        .build();
    let Ok(panel) = panel else {
        panic!("valid second slider panel")
    };
    let panel = fixture.app.world_mut().spawn(panel).id();
    fixture.update();
    let second_slider = resolve_widget(&mut fixture.app, panel);

    fixture.press(KeyCode::ArrowRight);
    for _ in 0..6 {
        fixture.update();
    }
    assert!(fixture.proposals().len() >= 2);
    let count = fixture.proposals().len();
    fixture.app.world_mut().trigger(RequestWidgetFocus {
        window: fixture.window,
        widget: second_slider,
    });
    for _ in 0..9 {
        fixture.update();
    }
    assert_eq!(fixture.proposals().len(), count);
    let Some(second_state) = fixture.app.world().get::<SliderState>(second_slider) else {
        panic!("second slider state")
    };
    assert!((second_state.value() - 0.5).abs() < 1e-6);
    assert!(fixture.application_arrows() > 0);

    fixture.release(KeyCode::ArrowRight);
    fixture.press(KeyCode::ArrowRight);
    assert_eq!(fixture.proposals().len(), count + 1);
    let Some(second_state) = fixture.app.world().get::<SliderState>(second_slider) else {
        panic!("second slider state after fresh press")
    };
    assert!(second_state.value() > 0.5);
    fixture.release(KeyCode::ArrowRight);
}

#[test]
fn releasing_arrow_between_pulses_adds_no_step_or_proposal() {
    let mut fixture = Fixture::new(unit_slider());
    fixture.press(KeyCode::ArrowRight);
    for _ in 0..5 {
        fixture.update();
    }
    assert_eq!(fixture.steps.len(), 1);
    assert_eq!(fixture.proposals().len(), 1);
    fixture.release(KeyCode::ArrowRight);
    fixture.update();
    assert_eq!(fixture.steps.len(), 1);
    assert_eq!(fixture.proposals().len(), 1);
}

#[test]
fn pointer_drag_mid_hold_ends_step_and_frees_arrow_for_application() {
    let mut fixture = Fixture::new(unit_slider());
    fixture.press(KeyCode::ArrowRight);
    assert_eq!(fixture.proposals().len(), 1);
    fixture.pointer_press();
    let count = fixture.proposals().len();
    for _ in 0..8 {
        fixture.update();
    }
    assert_eq!(fixture.proposals().len(), count);
    assert!(fixture.application_arrows() > 0);
    fixture.release(KeyCode::ArrowRight);
}
