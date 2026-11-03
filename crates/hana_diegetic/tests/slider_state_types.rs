//! Public-client coverage for named slider state and proposal sources.
#![allow(clippy::panic, reason = "tests should panic on invalid fixtures")]

use bevy::camera::NormalizedRenderTarget;
use bevy::camera::PerspectiveProjection;
use bevy::camera::Projection;
use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::ecs::system::RunSystemOnce;
use bevy::picking::backend::HitData;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::PanelBuildError;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::RequestSliderAdjustment;
use hana_diegetic::Slider;
use hana_diegetic::SliderAdjustment;
use hana_diegetic::SliderChangeRequested;
use hana_diegetic::SliderConfigError;
use hana_diegetic::SliderProposalSource;
use hana_diegetic::SliderState;
use hana_diegetic::SliderStep;
use hana_diegetic::SliderStepping;

const VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);
const PRESSING_POINTER: PointerId = PointerId::Touch(11);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Proposal {
    entity:   Entity,
    is_final: bool,
    source:   SliderProposalSource,
}

#[derive(Default, Resource)]
struct RecordedProposals(Vec<Proposal>);

fn record_proposal(change: On<SliderChangeRequested>, mut recorded: ResMut<RecordedProposals>) {
    recorded.0.push(Proposal {
        entity:   change.event_target(),
        is_final: change.is_final,
        source:   change.source,
    });
}

fn slider_tree(slider: Slider) -> hana_diegetic::LayoutTree {
    let mut builder = LayoutBuilder::new(100.0, 50.0);
    builder.with(El::new().size(20.0, 10.0).widget("level", slider), |_| {});
    builder.build()
}

fn panel(slider: Slider) -> Result<DiegeticPanel, PanelBuildError> {
    DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .world_height(0.5)
        .with_tree(slider_tree(slider))
        .build()
}

fn resolve_widget(app: &mut App, panel: Entity) -> Entity {
    let result = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named("level"))
        });
    let Ok(Some(widget)) = result else {
        panic!("level slider should be reified");
    };
    widget
}

fn seeded_camera() -> Camera {
    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: VIEWPORT.as_uvec2(),
        scale_factor:  1.0,
    });
    let mut projection = Projection::Perspective(PerspectiveProjection::default());
    projection.update(VIEWPORT.x, VIEWPORT.y);
    camera.computed.clip_from_view = projection.get_clip_from_view();
    camera
}

fn seeded_camera_transform() -> Transform {
    Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y)
}

const fn pointer_hit(camera: Entity) -> HitData {
    HitData {
        camera,
        depth: 0.0,
        position: None,
        normal: None,
        extra: None,
    }
}

struct Fixture {
    app:    App,
    widget: Entity,
    camera: Entity,
    target: NormalizedRenderTarget,
}

impl Fixture {
    fn new(slider: Slider) -> Self {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<RecordedProposals>()
            .add_plugins(HeadlessDiegeticUiPlugin)
            .add_observer(record_proposal);
        app.finish();

        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                seeded_camera(),
                seeded_camera_transform(),
                GlobalTransform::from(seeded_camera_transform()),
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id();
        let Ok(panel) = panel(slider) else {
            panic!("slider panel should build");
        };
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = resolve_widget(&mut app, panel);
        let Some(target) = RenderTarget::Window(WindowRef::Primary).normalize(Some(window)) else {
            panic!("primary window target should normalize");
        };
        Self {
            app,
            widget,
            camera,
            target,
        }
    }

    fn proposals(&self) -> &[Proposal] { &self.app.world().resource::<RecordedProposals>().0 }

    fn location(&self, position: Vec2) -> Location {
        Location {
            target: self.target.clone(),
            position,
        }
    }

    fn press(&mut self) {
        let location = self.location(VIEWPORT * 0.5);
        self.app.world_mut().trigger(PointerPress {
            entity:  self.widget,
            pointer: Pointer::new(PRESSING_POINTER, location),
            button:  PointerButton::Primary,
            hit:     pointer_hit(self.camera),
            count:   1,
        });
        self.app.world_mut().flush();
    }

    fn drag(&mut self) {
        let location = self.location(VIEWPORT * 0.5 + Vec2::new(40.0, 0.0));
        self.app.world_mut().trigger(PointerDrag {
            entity:   self.widget,
            pointer:  Pointer::new(PRESSING_POINTER, location),
            button:   PointerButton::Primary,
            distance: Vec2::ZERO,
            delta:    Vec2::ZERO,
        });
        self.app.world_mut().flush();
    }

    fn release(&mut self) {
        let location = self.location(VIEWPORT * 0.5 + Vec2::new(40.0, 0.0));
        self.app.world_mut().trigger(PointerRelease {
            entity:  self.widget,
            pointer: Pointer::new(PRESSING_POINTER, location),
            button:  PointerButton::Primary,
            hit:     pointer_hit(self.camera),
        });
        self.app.world_mut().flush();
    }
}

#[test]
fn reified_slider_with_step_exposes_stepped_state() {
    let fixture = Fixture::new(Slider::new(0.0..=1.0).step(0.25));
    let Some(state) = fixture.app.world().get::<SliderState>(fixture.widget) else {
        panic!("slider should have runtime state");
    };
    let Ok(step) = SliderStep::new(0.25) else {
        panic!("positive step should validate");
    };
    assert_eq!(state.stepping(), SliderStepping::Stepped(step));
}

#[test]
fn reified_slider_without_step_exposes_stepless_state() {
    let fixture = Fixture::new(Slider::new(0.0..=1.0));
    let Some(state) = fixture.app.world().get::<SliderState>(fixture.widget) else {
        panic!("slider should have runtime state");
    };
    assert_eq!(state.stepping(), SliderStepping::Stepless);
}

#[test]
fn zero_step_declaration_fails_panel_build() {
    let result = panel(Slider::new(0.0..=1.0).step(0.0));
    let Err(PanelBuildError::SliderConfig(id, SliderConfigError::NonPositiveStep)) = result else {
        panic!("zero step should fail slider validation");
    };
    assert_eq!(id, PanelElementId::named("level"));
}

#[test]
fn pointer_press_proposal_names_pressing_pointer() {
    let mut fixture = Fixture::new(Slider::new(0.0..=1.0).value(0.5));
    fixture.press();
    assert_eq!(
        fixture.proposals(),
        [Proposal {
            entity:   fixture.widget,
            is_final: false,
            source:   SliderProposalSource::Pointer(PRESSING_POINTER),
        }]
    );
}

#[test]
fn pointer_drag_proposal_names_pressing_pointer() {
    let mut fixture = Fixture::new(Slider::new(0.0..=1.0).value(0.5));
    fixture.press();
    fixture.drag();
    assert_eq!(fixture.proposals().len(), 2);
    assert_eq!(
        fixture.proposals()[1],
        Proposal {
            entity:   fixture.widget,
            is_final: false,
            source:   SliderProposalSource::Pointer(PRESSING_POINTER),
        }
    );
}

#[test]
fn pointer_release_proposal_names_pressing_pointer() {
    let mut fixture = Fixture::new(Slider::new(0.0..=1.0).value(0.5));
    fixture.press();
    fixture.drag();
    fixture.release();
    assert_eq!(fixture.proposals().len(), 3);
    assert_eq!(
        fixture.proposals()[2],
        Proposal {
            entity:   fixture.widget,
            is_final: true,
            source:   SliderProposalSource::Pointer(PRESSING_POINTER),
        }
    );
}

#[test]
fn adjustment_request_proposal_names_request_source() {
    let mut fixture = Fixture::new(Slider::new(0.0..=1.0).value(0.5));
    fixture.app.world_mut().trigger(RequestSliderAdjustment {
        entity:     fixture.widget,
        adjustment: SliderAdjustment::Absolute(0.75),
    });
    fixture.app.world_mut().flush();
    assert_eq!(
        fixture.proposals(),
        [Proposal {
            entity:   fixture.widget,
            is_final: true,
            source:   SliderProposalSource::Request,
        }]
    );
}
