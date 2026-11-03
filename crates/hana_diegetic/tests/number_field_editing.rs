//! Number fields use text insertion, selection, and their declared alignment.

#![allow(
    clippy::expect_used,
    reason = "fixtures require solved fields and emitted edit snapshots"
)]

use bevy::camera::ComputedCameraValues;
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::ecs::system::RunSystemOnce;
use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::Key;
use bevy::input::keyboard::KeyboardInput;
use bevy::picking::backend::HitData;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::window::WindowRef;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::BoundingBox;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeBufferSnapshot;
use hana_diegetic::ImeCursorState;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeInputBlocker;
use hana_diegetic::ImeTextChanged;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::Pt;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;

const VALUE: &str = "-12.34";
const FONT_SIZE: f32 = 16.0;
const WIDTH: f32 = 200.0;
const HEIGHT: f32 = 40.0;
const INSET: f32 = 7.0;

#[derive(Resource, Default)]
struct LatestEdit(Option<ImeBufferSnapshot>);

struct Field {
    app:       App,
    window:    Entity,
    panel:     Entity,
    widget:    Entity,
    camera:    Entity,
    alignment: AlignX,
}

impl Field {
    fn new(alignment: AlignX) -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, InputPlugin))
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin)
            .init_resource::<LatestEdit>()
            .add_observer(
                |changed: On<ImeTextChanged>, mut latest: ResMut<LatestEdit>| {
                    latest.0 = Some(changed.snapshot.clone());
                },
            );
        let window = app.world_mut().spawn(Window::default()).id();
        let style = TextStyle::new(FONT_SIZE);
        let mut builder = LayoutBuilder::with_widget_root(
            El::row()
                .size(WIDTH, HEIGHT)
                .alignment(alignment, AlignY::Center)
                .padding(Padding::xy(INSET - 1.0, 2.0))
                .border(Border::all(1.0, Color::WHITE))
                .editable_field(
                    "value",
                    ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("value")),
                    style.clone(),
                ),
        );
        builder.text((VALUE, style));
        let panel = DiegeticPanel::world()
            .size(Pt(WIDTH), Pt(HEIGHT))
            .world_height(HEIGHT)
            .anchor(Anchor::TopLeft)
            .font_unit(Unit::Points)
            .with_tree(builder.build())
            .build()
            .expect("number field builds");
        let panel = app.world_mut().spawn((panel, Transform::default())).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera {
                    computed: ComputedCameraValues {
                        clip_from_view: bevy::math::proj::orthographic(
                            0.0, WIDTH, -HEIGHT, 0.0, 0.1, 100.0,
                        ),
                        target_info: Some(RenderTargetInfo {
                            physical_size: UVec2::new(200, 40),
                            scale_factor:  1.0,
                        }),
                        ..default()
                    },
                    ..default()
                },
                RenderTarget::Window(WindowRef::Entity(window)),
                Transform::from_xyz(0.0, 0.0, 1.0),
            ))
            .id();
        app.update();
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("value"))
            })
            .expect("widget lookup runs")
            .expect("number field widget");
        Self {
            app,
            window,
            panel,
            widget,
            camera,
            alignment,
        }
    }

    fn text_width(&self, text: &str) -> f32 {
        (self
            .app
            .world()
            .resource::<DiegeticTextMeasurer>()
            .measure_fn)(text, &TextStyle::new(FONT_SIZE).as_measure())
        .width
    }

    fn origin(&self) -> f32 {
        let remaining = INSET.mul_add(-2.0, WIDTH) - FONT_SIZE / 8.0 - self.text_width(VALUE);
        INSET
            + match self.alignment {
                AlignX::Left => 0.0,
                AlignX::Center => remaining * 0.5,
                AlignX::Right => remaining,
            }
    }

    fn x_at(&self, prefix: &str) -> f32 { self.origin() + self.text_width(prefix) }

    fn location(&self, x: f32) -> Location {
        Location {
            target:   NormalizedRenderTarget::Window(
                WindowRef::Entity(self.window)
                    .normalize(None)
                    .expect("window target"),
            ),
            position: Vec2::new(x, HEIGHT * 0.5),
        }
    }

    fn hit(&self, x: f32) -> HitData {
        HitData::new(
            self.camera,
            1.0,
            Some(Vec3::new(x, -HEIGHT * 0.5, 0.0)),
            None,
        )
    }

    fn click(&mut self, x: f32, count: u8) {
        let press = PointerPress {
            entity: self.widget,
            pointer: Pointer::new(PointerId::Mouse, self.location(x)),
            button: PointerButton::Primary,
            hit: self.hit(x),
            count,
        };
        self.app.world_mut().trigger(press);
        self.app.update();
        let click = PointerClick {
            entity: self.widget,
            pointer: Pointer::new(PointerId::Mouse, self.location(x)),
            button: PointerButton::Primary,
            hit: self.hit(x),
            duration: std::time::Duration::ZERO,
            count,
        };
        self.app.world_mut().trigger(click);
        self.app.update();
    }

    fn snapshot(&self) -> &ImeBufferSnapshot {
        self.app
            .world()
            .resource::<LatestEdit>()
            .0
            .as_ref()
            .expect("click emitted an edit snapshot")
    }

    fn assert_insertion(&self, index: usize) {
        assert!(
            matches!(self.snapshot().cursor, ImeCursorState::Insertion(boundary) if boundary.as_usize() == index),
            "expected insertion at {index}: {:?}",
            self.snapshot()
        );
    }

    fn assert_selection(&self, anchor: usize, focus: usize) {
        assert!(
            matches!(&self.snapshot().cursor, ImeCursorState::Selection(selection) if selection.anchor.as_usize() == anchor && selection.focus.as_usize() == focus),
            "expected selection {anchor}..{focus}: {:?}",
            self.snapshot()
        );
    }

    fn caret(&self) -> BoundingBox {
        self.app
            .world()
            .get::<ComputedDiegeticPanel>(self.panel)
            .expect("computed field")
            .layout()
            .solved()
            .expect("solved field")
            .commands
            .iter()
            .find_map(|command| {
                (matches!(command.kind, RenderCommandKind::Rectangle { .. })
                    && command.bounds.width > 0.0
                    && command.bounds.width < 2.0
                    && command.bounds.height > 10.0)
                    .then_some(command.bounds)
            })
            .expect("editing draws a visible caret")
    }

    fn type_text(&mut self, text: &str) {
        self.app.world_mut().write_message(KeyboardInput {
            key_code:    KeyCode::Digit9,
            logical_key: Key::Character(text.into()),
            state:       ButtonState::Pressed,
            text:        Some(text.into()),
            repeat:      false,
            window:      self.window,
        });
        self.app.update();
    }
}

#[test]
fn single_click_places_a_visible_caret_at_the_point_without_selection() {
    let mut field = Field::new(AlignX::Left);
    let x = field.x_at("-12");
    field.click(x, 1);
    field.assert_insertion(3);
    assert!((field.caret().x - x).abs() < 0.5);
    field.click(field.x_at("-"), 1);
    field.assert_insertion(1);
}

#[test]
fn typing_inserts_at_the_clicked_caret() {
    let mut field = Field::new(AlignX::Left);
    field.click(field.x_at("-12"), 1);
    field.type_text("9");
    assert_eq!(field.snapshot().committed_text, "-129.34");
    field.assert_insertion(4);
}

#[test]
fn double_click_selects_the_whole_number_and_draws_the_caret_at_its_end() {
    let mut field = Field::new(AlignX::Left);
    let end = field.x_at(VALUE);
    field.click(field.x_at("-1"), 1);
    let session = field.app.world().resource::<ImeInputBlocker>().session_id();
    field.click(field.x_at("-1"), 2);
    assert_eq!(
        field.app.world().resource::<ImeInputBlocker>().session_id(),
        session
    );
    field.assert_selection(0, VALUE.len());
    assert!((field.caret().x - end).abs() < 0.5);
    field.type_text("9");
    assert_eq!(field.snapshot().committed_text, "9");
}

#[test]
fn dragging_selects_a_range_from_the_pressed_character() {
    let mut field = Field::new(AlignX::Right);
    let start = field.x_at("-1");
    let end = field.x_at("-12.3");
    let drag_start = PointerDragStart {
        entity:  field.widget,
        pointer: Pointer::new(PointerId::Mouse, field.location(start)),
        button:  PointerButton::Primary,
        hit:     field.hit(start),
    };
    field.app.world_mut().trigger(drag_start);
    field.app.update();
    let drag = PointerDrag {
        entity:   field.widget,
        pointer:  Pointer::new(PointerId::Mouse, field.location(end)),
        button:   PointerButton::Primary,
        distance: Vec2::new(end - start, 0.0),
        delta:    Vec2::new(end - start, 0.0),
    };
    field.app.world_mut().trigger(drag);
    field.app.update();
    field.assert_selection(2, 5);
    assert!((field.caret().x - end).abs() < 0.5);
    // Bevy emits Click before DragEnd when releasing over the original field.
    field.click(end, 1);
    field.assert_selection(2, 5);
    let drag_end = PointerDragEnd {
        entity:   field.widget,
        pointer:  Pointer::new(PointerId::Mouse, field.location(end)),
        button:   PointerButton::Primary,
        distance: Vec2::new(end - start, 0.0),
    };
    field.app.world_mut().trigger(drag_end);
    field.app.update();
    field.click(start, 1);
    field.assert_insertion(2);
}

#[test]
fn right_aligned_click_uses_the_aligned_text_origin_and_keeps_the_caret_aligned() {
    let mut field = Field::new(AlignX::Right);
    let x = field.x_at("-12.");
    field.click(x, 1);
    field.assert_insertion(4);
    assert!((field.caret().x - x).abs() < 0.5);
    field.click(WIDTH - INSET - 0.1, 1);
    field.assert_insertion(VALUE.len());
    assert!(field.caret().x > WIDTH - INSET - 3.0);
    field.click(INSET, 1);
    field.assert_insertion(0);
    assert!((field.caret().x - field.origin()).abs() < 0.5);
}
