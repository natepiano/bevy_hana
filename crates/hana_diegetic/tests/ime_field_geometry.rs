//! Editing and validating a value must keep its authored field frame.

#![allow(
    clippy::expect_used,
    reason = "regression fixtures fail immediately when required field state is absent"
)]

use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::Key;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::keyboard::NativeKey;
use bevy::prelude::App;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::window::Ime;
use bevy::window::WindowFocused;
use bevy::window::WindowRef;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Border;
use hana_diegetic::BoundingBox;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::ImeAcceptCommit;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeApplied;
use hana_diegetic::ImeAppliedResult;
use hana_diegetic::ImeBuiltInFieldKind;
use hana_diegetic::ImeBuiltInFieldSpec;
use hana_diegetic::ImeCommitRequested;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeInputBlocker;
use hana_diegetic::ImeOpenSession;
use hana_diegetic::ImeRejectCommit;
use hana_diegetic::ImeRejection;
use hana_diegetic::ImeTarget;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::Px;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::Text;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;

mod drawn_support;

#[test]
fn ime_field_scrolling_clips_to_the_padded_content_viewport() {
    let mut scene = FieldScene::new(FieldContract::Integer);
    scene.open_and_type("12345678901234567890");
    let computed = scene
        .app
        .world()
        .get::<ComputedDiegeticPanel>(scene.panel)
        .expect("field layout");
    let field = &computed.field_records()[0];
    let clip = computed
        .layout()
        .solved()
        .expect("field solves")
        .commands
        .iter()
        .find(|command| {
            command.element_idx == field.element_index
                && matches!(command.kind, RenderCommandKind::ScissorStart)
        })
        .expect("editable content clip")
        .bounds;
    assert!(
        clip.x > field.bounds.x + 0.75,
        "scrolling text stays inside the field padding"
    );
    assert!(clip.x + clip.width < field.bounds.x + field.bounds.width - 0.75);
}

#[test]
fn ime_field_caret_keeps_the_right_inner_padding_for_short_and_scrolled_text() {
    for text in ["999", "12345678901234567890"] {
        let mut scene = FieldScene::with_text_style(
            FieldContract::Integer,
            TextStyle::new(13.0).with_align(TextAlign::Right),
        );
        scene.open_and_type(text);
        let computed = scene.app.world().get::<ComputedDiegeticPanel>(scene.panel);
        assert!(computed.is_some(), "editing field has a computed layout");
        let Some(computed) = computed else { return };
        let field = &computed.field_records()[0];
        let solved = computed.layout().solved();
        assert!(solved.is_some(), "editing field layout solves");
        let Some(solved) = solved else { return };
        let caret = solved.commands.iter().find_map(|command| {
            (matches!(command.kind, RenderCommandKind::Rectangle { .. })
                && command.element_idx != field.element_index
                && command.bounds.width > 0.0
                && command.bounds.width < command.bounds.height)
                .then_some(command.bounds)
        });
        assert!(caret.is_some(), "editing field draws its caret");
        let Some(caret) = caret else { return };
        let right_inset = field.bounds.x + field.bounds.width - caret.x - caret.width;
        let padded_inset = Unit::Pixels.to_points() * 7.0;
        assert!(
            right_inset >= padded_inset - 1e-4,
            "{text}: caret {caret:?} must keep the 6px padding and 1px border; inset {right_inset}"
        );
    }
}

#[test]
fn ime_field_candidate_anchor_keeps_the_caret_inside_the_right_inner_padding() -> Result<(), String>
{
    use bevy::ecs::system::RunSystemOnce;
    use hana_diegetic::PanelAnchorGeometryParam;
    use hana_diegetic::PanelAnchorPoints;

    for text in ["999", "12345678901234567890"] {
        let mut scene = FieldScene::with_text_style(
            FieldContract::Integer,
            TextStyle::new(13.0).with_align(TextAlign::Right),
        );
        scene.open_and_type(text);
        let caret_width = editing_caret_bounds(&scene)?.width / Unit::Pixels.to_points();
        let panel = scene.panel;
        let geometry = scene
            .app
            .world_mut()
            .run_system_once(move |geometry: PanelAnchorGeometryParam| geometry.get(panel))
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let PanelAnchorPoints::Screen { bounds, .. } = *geometry.points() else {
            return Err("numeric field must have screen geometry".to_owned());
        };
        let window = scene.app.world().get::<Window>(scene.window);
        assert!(window.is_some(), "editing window remains present");
        let Some(window) = window else { return Ok(()) };
        let right_limit = bounds.top_left().x + bounds.size().x - 7.0;
        assert!(
            window.ime_position.x >= bounds.top_left().x + 7.0,
            "candidate anchor must belong to the editing field"
        );
        assert!(
            window.ime_position.x + caret_width <= right_limit + 1e-4,
            "{text}: candidate anchor {} must reserve caret width {caret_width} before {right_limit}",
            window.ime_position.x,
        );
    }
    Ok(())
}

fn editing_caret_bounds(scene: &FieldScene) -> Result<BoundingBox, String> {
    let computed = scene
        .app
        .world()
        .get::<ComputedDiegeticPanel>(scene.panel)
        .ok_or_else(|| "editing field has no computed layout".to_owned())?;
    let solved = computed
        .layout()
        .solved()
        .ok_or_else(|| "editing field layout has not solved".to_owned())?;
    let field_index = computed.field_records()[0].element_index;
    solved
        .commands
        .iter()
        .find_map(|command| {
            (matches!(command.kind, RenderCommandKind::Rectangle { .. })
                && command.element_idx != field_index
                && command.bounds.width > 0.0
                && command.bounds.width < command.bounds.height)
                .then_some(command.bounds)
        })
        .ok_or_else(|| "editing field draws no caret".to_owned())
}

#[derive(Clone, Copy, Debug)]
enum FieldContract {
    Text,
    Float,
    Integer,
    AppOwned,
}

impl FieldContract {
    fn spec(self) -> ImeEditableFieldSpec {
        match self {
            Self::Text => {
                ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text))
            },
            Self::Float => ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Float {
                    min: Some(0.0),
                    max: Some(10.0),
                },
            )),
            Self::Integer => ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Integer {
                    min: Some(0),
                    max: Some(10),
                },
            )),
            Self::AppOwned => ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("field")),
        }
    }
}

#[derive(Default, Resource)]
struct AppliedValues(Vec<ImeAppliedResult>);

fn apply_app_value(request: On<ImeCommitRequested>, mut commands: Commands) {
    if !matches!(request.field_spec, ImeEditableFieldSpec::AppOwned(_)) {
        return;
    }
    if request.text == "231" {
        commands.trigger(ImeRejectCommit {
            session_id: request.session_id,
            attempt_id: request.attempt_id,
            reason:     ImeRejection::OutOfRange("expected 0 to 10".to_owned()),
        });
    } else {
        commands.trigger(ImeAcceptCommit {
            session_id: request.session_id,
            attempt_id: request.attempt_id,
            result:     ImeAppliedResult::AppOwned {
                display_text:   Some(request.text.clone()),
                value_revision: None,
            },
        });
    }
}

#[derive(Debug, PartialEq)]
struct FieldFrame {
    bounds:  BoundingBox,
    borders: Vec<(BoundingBox, Border)>,
}

struct FieldScene {
    app:      App,
    window:   Entity,
    panel:    Entity,
    contract: FieldContract,
}

impl FieldScene {
    fn new(contract: FieldContract) -> Self {
        Self::with_text_style(contract, TextStyle::new(13.0))
    }

    fn with_text_style(contract: FieldContract, style: TextStyle) -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin)
            .init_resource::<AppliedValues>()
            .add_observer(apply_app_value)
            .add_observer(
                |applied: On<ImeApplied>, mut values: ResMut<AppliedValues>| {
                    values.0.push(applied.result.clone());
                },
            );
        let window = app.world_mut().spawn(Window::default()).id();
        let mut builder = LayoutBuilder::with_widget_root(
            El::row()
                .size(100.0, 24.0)
                .padding(Padding::xy(6.0, 1.0))
                .border(Border::all(1.0, Color::WHITE))
                .background(Color::BLACK)
                .alignment(AlignX::Right, AlignY::Center)
                .editable_field("field", contract.spec(), style.clone()),
        );
        builder.text(Text::new("1", style));
        let panel = DiegeticPanel::screen()
            .size(Px(100.0), Px(24.0))
            .window(WindowRef::Entity(window))
            .font_unit(Unit::Pixels)
            .with_tree(builder.build())
            .build()
            .expect("field panel builds");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        Self {
            app,
            window,
            panel,
            contract,
        }
    }

    fn frame(&self) -> FieldFrame {
        let computed = self
            .app
            .world()
            .get::<ComputedDiegeticPanel>(self.panel)
            .expect("field layout remains");
        let field = &computed.field_records()[0];
        let borders = computed
            .layout()
            .solved()
            .expect("field solves")
            .commands
            .iter()
            .filter_map(|command| match command.kind {
                RenderCommandKind::Border { border }
                    if command.element_idx == field.element_index =>
                {
                    Some((command.bounds, border))
                },
                _ => None,
            })
            .collect();
        FieldFrame {
            bounds: field.bounds,
            borders,
        }
    }

    fn open_and_type(&mut self, text: &str) {
        self.app.world_mut().trigger(ImeOpenSession {
            target:       ImeTarget::ScreenPanelField {
                panel:    self.panel,
                field_id: "field".into(),
            },
            window:       self.window,
            initial_text: "1".to_owned(),
            field_spec:   self.contract.spec(),
            anchor:       None,
        });
        self.app.update();
        self.app.world_mut().write_message(Ime::Commit {
            window: self.window,
            value:  text.to_owned(),
        });
        self.app.update();
    }

    fn press(&mut self, key_code: KeyCode) {
        for state in [ButtonState::Pressed, ButtonState::Released] {
            self.app.world_mut().write_message(KeyboardInput {
                key_code,
                logical_key: Key::Unidentified(NativeKey::Unidentified),
                state,
                text: None,
                repeat: false,
                window: self.window,
            });
            self.app.update();
        }
    }

    fn text(&self) -> &str {
        &self
            .app
            .world()
            .get::<ComputedDiegeticPanel>(self.panel)
            .expect("field layout remains")
            .field_records()[0]
            .display_text
    }

    fn validation_count(&mut self) -> usize {
        let world = self.app.world_mut();
        let mut panels = world.query::<(Entity, &ComputedDiegeticPanel)>();
        panels
            .iter(world)
            .filter(|(entity, computed)| {
                *entity != self.panel
                    && computed.layout().solved().is_some_and(|layout| {
                        layout.commands.iter().any(|command| matches!(
                    &command.kind, RenderCommandKind::Text { text, .. } if text.contains("10")
                ))
                    })
            })
            .count()
    }
}

#[test]
fn ime_field_enter_commits_every_field_kind_without_changing_its_frame() {
    for contract in [
        FieldContract::Text,
        FieldContract::Float,
        FieldContract::Integer,
        FieldContract::AppOwned,
    ] {
        let mut scene = FieldScene::new(contract);
        let before = scene.frame();
        scene.open_and_type("5");
        assert_eq!(scene.frame(), before, "{contract:?}: typed frame");
        scene.press(KeyCode::Enter);
        assert_eq!(scene.text(), "5", "{contract:?}: committed display");
        assert_eq!(scene.app.world().resource::<AppliedValues>().0.len(), 1);
        assert_eq!(
            scene.app.world().resource::<ImeInputBlocker>().session_id(),
            None
        );
        assert_eq!(scene.frame(), before, "{contract:?}: accepted frame");
    }
}

#[test]
fn ime_field_rejected_enter_and_tab_keep_every_numeric_field_frame() {
    for key in [KeyCode::Enter, KeyCode::Tab] {
        for contract in [
            FieldContract::Float,
            FieldContract::Integer,
            FieldContract::AppOwned,
        ] {
            let mut scene = FieldScene::new(contract);
            let before = scene.frame();
            scene.open_and_type("231");
            scene.press(key);
            assert_eq!(scene.app.world().resource::<AppliedValues>().0, Vec::new());
            assert!(
                scene
                    .app
                    .world()
                    .resource::<ImeInputBlocker>()
                    .session_id()
                    .is_some()
            );
            assert_eq!(
                scene.frame(),
                before,
                "{contract:?}: rejection must keep all four edges"
            );
            assert_eq!(
                scene.validation_count(),
                1,
                "rejection remains visible beside the field"
            );
            scene.press(KeyCode::Escape);
            assert_eq!(scene.validation_count(), 0, "Escape removes the feedback");
            assert_eq!(scene.text(), "1");
            assert_eq!(scene.frame(), before);
        }
    }
}

#[test]
fn ime_field_escape_restores_every_field_kind_without_changing_its_frame() {
    for contract in [
        FieldContract::Text,
        FieldContract::Float,
        FieldContract::Integer,
        FieldContract::AppOwned,
    ] {
        let mut scene = FieldScene::new(contract);
        let before = scene.frame();
        scene.open_and_type("5");
        scene.press(KeyCode::Escape);
        assert_eq!(scene.text(), "1");
        assert_eq!(scene.frame(), before);
        assert_eq!(
            scene.app.world().resource::<ImeInputBlocker>().session_id(),
            None
        );
    }
}

#[test]
fn ime_field_enter_after_session_replacement_restores_the_authored_frame() {
    let mut scene = FieldScene::new(FieldContract::AppOwned);
    let before = scene.frame();
    scene.open_and_type("231");
    scene.press(KeyCode::Enter);
    scene.open_and_type("5");
    scene.press(KeyCode::Enter);
    assert_eq!(scene.text(), "5");
    assert_eq!(
        scene.app.world().resource::<ImeInputBlocker>().session_id(),
        None
    );
    assert_eq!(scene.frame(), before);
    let rectangles = scene
        .app
        .world()
        .get::<ComputedDiegeticPanel>(scene.panel)
        .expect("field layout remains")
        .layout()
        .solved()
        .expect("field solves")
        .commands
        .iter()
        .filter(|command| matches!(command.kind, RenderCommandKind::Rectangle { .. }))
        .count();
    assert_eq!(
        rectangles, 1,
        "an accepted replacement must remove the old editor's caret"
    );
}

#[test]
fn ime_field_tab_commits_every_field_kind_without_changing_its_frame() {
    for contract in [
        FieldContract::Text,
        FieldContract::Float,
        FieldContract::Integer,
        FieldContract::AppOwned,
    ] {
        let mut scene = FieldScene::new(contract);
        let before = scene.frame();
        scene.open_and_type("5");
        scene.press(KeyCode::Tab);
        assert_eq!(scene.text(), "5");
        assert_eq!(scene.app.world().resource::<AppliedValues>().0.len(), 1);
        assert_eq!(scene.frame(), before);
    }
}

#[test]
fn ime_field_tab_commits_a_cancel_on_blur_field_before_moving_focus() {
    let spec = ImeEditableFieldSpec::AppOwned(
        ImeAppOwnedFieldSpec::new("field")
            .cancel_on_blur()
            .commit_on_tab(),
    );
    assert_tab_commit_and_focus_traversal(FieldContract::AppOwned, spec);
}

#[test]
fn ime_field_tab_moves_pointer_focus_after_committing_every_field_kind() {
    for contract in [
        FieldContract::Text,
        FieldContract::Float,
        FieldContract::Integer,
        FieldContract::AppOwned,
    ] {
        assert_tab_commit_and_focus_traversal(contract, contract.spec());
    }
}

fn assert_tab_commit_and_focus_traversal(contract: FieldContract, spec: ImeEditableFieldSpec) {
    let mut scene = FieldScene::new(contract);
    scene
        .app
        .world_mut()
        .get_mut::<DiegeticPanel>(scene.panel)
        .expect("field panel")
        .set_size((Px(200.0), Px(24.0)))
        .expect("two-widget panel size");
    let mut builder = LayoutBuilder::with_root(El::row().size(200.0, 24.0));
    builder.with(
        El::row()
            .size(100.0, 24.0)
            .border(Border::all(1.0, Color::WHITE))
            .editable_field("field", spec.clone(), TextStyle::new(13.0)),
        |field| {
            field.text(Text::new("1", TextStyle::new(13.0)));
        },
    );
    builder.with(El::row().size(100.0, 24.0).button("next"), |button| {
        button.text(Text::new("Next", TextStyle::new(13.0)));
    });
    scene
        .app
        .world_mut()
        .trigger(hana_diegetic::ImeReplacePanelTree {
            panel: scene.panel,
            tree:  builder.build(),
        });
    scene.app.update();
    let field = named_widget(&scene.app, "field");
    pointer_focus_field(&mut scene, field);
    scene.app.update();
    scene.app.world_mut().trigger(ImeOpenSession {
        target:       ImeTarget::ScreenPanelField {
            panel:    scene.panel,
            field_id: "field".into(),
        },
        window:       scene.window,
        initial_text: "1".to_owned(),
        field_spec:   spec,
        anchor:       None,
    });
    scene.app.update();
    scene.app.world_mut().write_message(Ime::Commit {
        window: scene.window,
        value:  "5".to_owned(),
    });
    scene.app.update();
    let before = scene.frame();
    scene.press(KeyCode::Tab);
    scene.app.update();
    assert_eq!(scene.text(), "5");
    assert_eq!(scene.app.world().resource::<AppliedValues>().0.len(), 1);
    assert_eq!(scene.frame(), before);
    assert_eq!(
        scene.app.world().resource::<ImeInputBlocker>().session_id(),
        None
    );
    let next = named_widget(&scene.app, "next");
    assert!(
        scene
            .app
            .world()
            .get::<hana_diegetic::WidgetFocused>(field)
            .is_none()
    );
    assert!(
        scene
            .app
            .world()
            .get::<hana_diegetic::WidgetFocused>(next)
            .is_some()
    );
}

fn named_widget(app: &App, id: &str) -> Entity {
    app.world()
        .iter_entities()
        .find_map(|entity| {
            let widget = entity.get::<hana_diegetic::PanelWidget>()?;
            (widget.id().as_str() == Some(id)).then_some(entity.id())
        })
        .expect("fixture widget")
}

fn pointer_focus_field(scene: &mut FieldScene, field: Entity) {
    let camera = scene
        .app
        .world_mut()
        .spawn((
            bevy::prelude::Camera::default(),
            bevy::camera::RenderTarget::Window(WindowRef::Entity(scene.window)),
        ))
        .id();
    let window = WindowRef::Entity(scene.window)
        .normalize(None)
        .expect("fixture window");
    scene.app.world_mut().trigger(bevy::prelude::PointerPress {
        entity:  field,
        pointer: bevy::prelude::Pointer::new(
            bevy::picking::pointer::PointerId::Mouse,
            bevy::picking::pointer::Location {
                target:   bevy::camera::NormalizedRenderTarget::Window(window),
                position: bevy::prelude::Vec2::ZERO,
            },
        ),
        button:  bevy::prelude::PointerButton::Primary,
        hit:     bevy::picking::backend::HitData::new(camera, 1.0, None, None),
        count:   1,
    });
}

#[test]
fn ime_field_focus_loss_restores_every_field_kind_without_changing_its_frame() {
    for contract in [
        FieldContract::Text,
        FieldContract::Float,
        FieldContract::Integer,
        FieldContract::AppOwned,
    ] {
        let mut scene = FieldScene::new(contract);
        let before = scene.frame();
        scene.open_and_type("5");
        scene.app.world_mut().write_message(WindowFocused {
            window:  scene.window,
            focused: false,
        });
        scene.app.update();
        scene.app.update();
        assert_eq!(scene.text(), "1");
        assert_eq!(scene.frame(), before);
        assert_eq!(
            scene.app.world().resource::<ImeInputBlocker>().session_id(),
            None
        );
    }
}

#[test]
fn ime_field_enter_keeps_the_drawn_bottom_border() -> Result<(), String> {
    drawn_support::run_cases(&[drawn_support::DrawnCase {
        name: "enter_keeps_the_drawn_bottom_border",
        run:  || drawn_number_field_keeps_the_bottom_border(NumberFieldEditingState::Accepted),
    }])
}

#[test]
fn ime_field_rejected_enter_keeps_the_drawn_bottom_border() -> Result<(), String> {
    drawn_support::run_cases(&[drawn_support::DrawnCase {
        name: "rejected_enter_keeps_the_drawn_bottom_border",
        run:  || drawn_number_field_keeps_the_bottom_border(NumberFieldEditingState::Rejected),
    }])
}

#[test]
fn ime_field_long_typed_value_keeps_all_four_drawn_borders() -> Result<(), String> {
    drawn_support::run_cases(&[drawn_support::DrawnCase {
        name: "long_typed_value_keeps_all_four_drawn_borders",
        run:  || drawn_number_field_keeps_the_bottom_border(NumberFieldEditingState::EditingLong),
    }])
}

enum NumberFieldEditingState {
    Accepted,
    Rejected,
    EditingLong,
}

struct DrawnNumberField {
    app:                drawn_support::CaseApp,
    window:             Entity,
    interaction_camera: Entity,
    panel:              Entity,
    region:             drawn_support::CaptureRegion,
    target:             bevy::prelude::Handle<bevy::prelude::Image>,
}

fn drawn_number_field_scene() -> Result<DrawnNumberField, String> {
    drawn_field_scene(drawn_number_field_panel()?)
}

fn drawn_field_scene(field_panel: DiegeticPanel) -> Result<DrawnNumberField, String> {
    use bevy::camera::Hdr;
    use bevy::camera::RenderTarget;
    use bevy::prelude::Camera;
    use bevy::prelude::Camera3d;
    use bevy::prelude::Msaa;
    use bevy::prelude::Transform;
    use bevy::prelude::UVec2;
    use bevy::prelude::Vec3;
    use bevy::render::view::DebandDither;
    use bevy::render::view::Tonemapping;

    let mut app = drawn_support::CaseApp::new();
    let size = UVec2::new(420, 180);
    let projection = bevy::camera::Projection::Perspective(bevy::camera::PerspectiveProjection {
        near: 0.001,
        ..Default::default()
    });
    let camera_pose = Transform::from_xyz(0.0, 0.0, 0.030 / std::f32::consts::FRAC_PI_8.tan())
        .looking_at(Vec3::ZERO, Vec3::Y);
    let window = app
        .world_mut()
        .spawn((
            Window {
                resolution: (size.x, size.y).into(),
                ..Window::default()
            },
            bevy::window::PrimaryWindow,
        ))
        .id();
    let interaction_camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            projection.clone(),
            RenderTarget::Window(WindowRef::Entity(window)),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            DebandDither::Disabled,
            Msaa::Off,
            camera_pose,
        ))
        .id();
    let region = drawn_support::CaptureRegion::new(size, UVec2::ZERO, size);
    let target = app.target(region);
    let mut camera_data = Camera::default();
    region.apply_to_camera(&mut camera_data);
    app.world_mut().spawn((
        Camera3d::default(),
        projection,
        camera_data,
        RenderTarget::Image(target.clone().into()),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        camera_pose,
    ));
    let panel = app
        .world_mut()
        .spawn((field_panel, Transform::IDENTITY))
        .id();
    app.settle()?;
    Ok(DrawnNumberField {
        app,
        window,
        interaction_camera,
        panel,
        region,
        target,
    })
}

fn drawn_number_field_keeps_the_bottom_border(
    state: NumberFieldEditingState,
) -> Result<(), String> {
    let DrawnNumberField {
        mut app,
        window,
        interaction_camera,
        panel,
        region,
        target,
    } = drawn_number_field_scene()?;
    let before = app.capture(target.clone())?;
    open_drawn_field(&mut app, window, interaction_camera, panel);
    app.world_mut().flush();
    app.update();
    assert!(
        app.world()
            .resource::<ImeInputBlocker>()
            .session_id()
            .is_some()
    );
    let (value, session_open) = match state {
        NumberFieldEditingState::Accepted => ("5", false),
        NumberFieldEditingState::Rejected => ("231", true),
        NumberFieldEditingState::EditingLong => ("12345678901234567890", true),
    };
    app.world_mut().write_message(Ime::Commit {
        window,
        value: value.to_owned(),
    });
    app.update();
    if !matches!(state, NumberFieldEditingState::EditingLong) {
        press_drawn_enter(&mut app, window);
    }
    assert_eq!(
        app.world()
            .resource::<ImeInputBlocker>()
            .session_id()
            .is_some(),
        session_open
    );
    assert_eq!(
        app.world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("field retains layout")
            .field_records()[0]
            .display_text,
        value
    );
    let after = app.capture(target)?;
    drawn_support::dump_pixels::<false>(
        "ime_field_geometry",
        "",
        "after Enter",
        region,
        region,
        &after,
    )?;
    // The whole field projects to x=60..360, y=54..126. Its green outline is
    // isolated from white digits, fill and backdrop; compare every border pixel.
    let original = drawn_border_pixels(&before, region.size.x);
    assert!(
        original.iter().filter(|&&(_, y)| y >= 123).count() >= 200,
        "idle field must draw its bottom border: {} border pixels",
        original.len()
    );
    let actual = drawn_border_pixels(&after, region.size.x);
    let missing = original
        .iter()
        .filter(|pixel| !actual.contains(pixel))
        .collect::<Vec<_>>();
    let added = actual
        .iter()
        .filter(|pixel| !original.contains(pixel))
        .collect::<Vec<_>>();
    assert!(
        actual == original,
        "Enter must preserve every drawn border pixel: missing {missing:?}, added {added:?}"
    );
    Ok(())
}

fn drawn_number_field_panel() -> Result<DiegeticPanel, String> {
    use hana_diegetic::Anchor;
    use hana_diegetic::Mm;
    use hana_diegetic::PanelPicking;

    let mut builder = LayoutBuilder::with_widget_root(
        El::row()
            .size(100.0, 24.0)
            .padding(Padding::xy(6.0, 1.0))
            .border(Border::all(Px(1.0), Color::srgb(0.0, 1.0, 0.0)))
            .background(Color::srgb(0.08, 0.08, 0.08))
            .alignment(AlignX::Right, AlignY::Center)
            .editable_field("field", FieldContract::Integer.spec(), TextStyle::new(13.0)),
    );
    builder.text(Text::new("1", TextStyle::new(13.0)));
    DiegeticPanel::world()
        .size(Mm(100.0), Mm(24.0))
        .font_unit(Unit::Millimeters)
        .anchor(Anchor::Center)
        .picking(PanelPicking::INTERACTIVE)
        .with_tree(builder.build())
        .build()
        .map_err(|error| error.to_string())
}

fn open_drawn_field(app: &mut App, window: Entity, camera: Entity, panel: Entity) {
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PointerClick;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;

    let field = app
        .world()
        .iter_entities()
        .find_map(|entity| {
            let widget = entity.get::<hana_diegetic::PanelWidget>()?;
            let owner = entity.get::<hana_diegetic::WidgetOf>()?;
            (owner.panel() == panel && widget.id().as_str() == Some("field")).then_some(entity.id())
        })
        .expect("reified editable field");
    let window_ref = WindowRef::Entity(window)
        .normalize(None)
        .expect("window target normalizes");
    app.world_mut().trigger(PointerClick {
        entity:   field,
        pointer:  Pointer::new(
            PointerId::Mouse,
            Location {
                target:   bevy::camera::NormalizedRenderTarget::Window(window_ref),
                position: Vec2::new(210.0, 90.0),
            },
        ),
        button:   PointerButton::Primary,
        hit:      HitData::new(camera, 0.0, Some(Vec3::ZERO), None),
        duration: std::time::Duration::ZERO,
        count:    2,
    });
}

fn press_drawn_enter(app: &mut App, window: Entity) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Enter,
            logical_key: Key::Enter,
            state,
            text: None,
            repeat: false,
            window,
        });
        app.update();
    }
}

fn drawn_border_pixels(pixels: &[u8], image_width: u32) -> Vec<(u32, u32)> {
    (50..140)
        .flat_map(|y| (50..370).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let offset = ((y * image_width + x) * 4) as usize;
            pixels[offset + 1] > pixels[offset].saturating_add(40)
                && pixels[offset + 1] > pixels[offset + 2].saturating_add(40)
        })
        .collect()
}

#[test]
fn ime_field_text_value_draws_shared_border_and_left_aligned_text() -> Result<(), String> {
    drawn_support::run_cases(&[drawn_support::DrawnCase {
        name: "text_value_draws_shared_border_and_left_aligned_text",
        run:  drawn_text_value_field,
    }])
}

fn drawn_text_value_field() -> Result<(), String> {
    use hana_diegetic::Anchor;
    use hana_diegetic::ControlMetrics;
    use hana_diegetic::ControlPalette;
    use hana_diegetic::Mm;
    use hana_diegetic::PanelPicking;
    use hana_diegetic::Sizing;
    let metrics = ControlMetrics::from_label_size(13.0_f32);
    let mut builder = LayoutBuilder::with_widget_root(
        El::row()
            .text_value_field(
                "field",
                ImeEditableFieldSpec::AppOwned(
                    ImeAppOwnedFieldSpec::new("field")
                        .commit_on_tab()
                        .cancel_on_blur(),
                ),
                &metrics,
            )
            .width(Sizing::fixed(100.0)),
    );
    builder.text(Text::new(
        "hana",
        TextStyle::new(metrics.label_size).with_color(ControlPalette::system_dark().field_text),
    ));
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(metrics.field.height))
        .font_unit(Unit::Millimeters)
        .anchor(Anchor::Center)
        .picking(PanelPicking::INTERACTIVE)
        .with_tree(builder.build())
        .build()
        .map_err(|error| error.to_string())?;
    let mut scene = drawn_field_scene(panel)?;
    let before = scene.app.capture(scene.target.clone())?;
    assert_text_field_pixels(&before, scene.region.size.x, &metrics)?;
    open_drawn_field(
        &mut scene.app,
        scene.window,
        scene.interaction_camera,
        scene.panel,
    );
    scene.app.world_mut().flush();
    scene.app.update();
    scene.app.world_mut().write_message(Ime::Commit {
        window: scene.window,
        value:  "hana".to_owned(),
    });
    scene.app.update();
    let editing = scene.app.capture(scene.target)?;
    assert_text_field_pixels(&editing, scene.region.size.x, &metrics)
}

fn assert_text_field_pixels(
    pixels: &[u8],
    image_width: u32,
    metrics: &hana_diegetic::ControlMetrics<f32>,
) -> Result<(), String> {
    let (rgba, remainder) = pixels.as_chunks::<4>();
    assert_eq!(remainder, &[] as &[u8]);
    let sample = |x: u32, y: u32| rgba[(y * image_width + x) as usize];
    // The field projects at 3px/mm around (210, 90). Derive its vertical edges
    // from the shared height; mid-edge samples avoid both glyphs and corners.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the field's edges project to small positive pixel rows"
    )]
    let (top, bottom) = (
        metrics.field.height.mul_add(-1.5, 90.0).round() as u32,
        metrics.field.height.mul_add(1.5, 90.0).round() as u32,
    );
    for (x, y) in [(60, 90), (359, 90), (210, top), (210, bottom - 1)] {
        let [r, g, b, _] = sample(x, y);
        assert!(
            r > 80 && g > 80 && b > 80,
            "shared outline is visible at {x},{y}: {r},{g},{b}"
        );
    }
    let left = (67..180)
        .find(|&x| {
            (72..110).any(|y| {
                let [r, g, b, _] = sample(x, y);
                r > 170 && g > 170 && b > 170
            })
        })
        .ok_or("left-aligned text has no visible ink")?;
    let left = f32::from(u16::try_from(left).map_err(|error| error.to_string())?);
    let expected = (metrics.field.side_padding + metrics.field.border_width).mul_add(3.0, 60.0);
    assert!(
        left >= expected - 1.0 && (left - expected).abs() <= 4.0,
        "text ink starts at {left}px inside shared left padding {expected}px"
    );
    Ok(())
}
