//! A panel field keeps one visible line while its IME editor follows the caret.

#![allow(
    clippy::expect_used,
    reason = "integration fixtures fail explicitly when required panel state is absent"
)]

use bevy::input::ButtonState;
use bevy::input::InputPlugin;
use bevy::input::keyboard::Key;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::keyboard::NativeKey;
use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::Window;
use bevy::window::Ime;
use bevy::window::WindowClosed;
use bevy::window::WindowFocused;
use bevy::window::WindowRef;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::ImeBuiltInFieldKind;
use hana_diegetic::ImeBuiltInFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeOpenSession;
use hana_diegetic::ImeTarget;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::PanelElementId;
use hana_diegetic::Px;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;

const FIELD_WIDTH: f32 = 72.0;
const FIELD_HEIGHT: f32 = 18.0;
const TOLERANCE: f32 = 0.5;
const LONG_VALUE: &str = "macOS field scrolling across a very long single line";

struct FieldFixture {
    app:    App,
    window: Entity,
    panel:  Entity,
}

fn field_tree(value: &str) -> LayoutTree {
    let mut builder = LayoutBuilder::new(140.0, 40.0);
    builder.with(
        El::new().size(FIELD_WIDTH, FIELD_HEIGHT).editable_field(
            "field",
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text)),
            TextStyle::new(12.0),
        ),
        |builder| {
            builder.text((value, TextStyle::new(12.0)));
        },
    );
    builder.build()
}

impl FieldFixture {
    fn new(value: &str) -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .add_message::<Ime>()
            .add_message::<WindowClosed>()
            .add_message::<WindowFocused>()
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = DiegeticPanel::screen()
            .size(Px(140.0), Px(40.0))
            .window(WindowRef::Entity(window))
            .font_unit(Unit::Pixels)
            .with_tree(field_tree(value))
            .build()
            .expect("field panel builds");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        Self { app, window, panel }
    }

    fn open(&mut self, value: &str) {
        self.app.world_mut().trigger(ImeOpenSession {
            target:       ImeTarget::ScreenPanelField {
                panel:    self.panel,
                field_id: PanelElementId::named("field"),
            },
            window:       self.window,
            initial_text: value.to_owned(),
            field_spec:   ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Text,
            )),
            anchor:       None,
        });
        self.app.update();
    }

    fn type_ime(&mut self, text: &str) {
        self.app.world_mut().write_message(Ime::Commit {
            window: self.window,
            value:  text.to_owned(),
        });
        self.app.update();
    }

    fn press(&mut self, key_code: KeyCode) {
        self.app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: self.window,
        });
        self.app.update();
    }

    fn solved(&self) -> &ComputedDiegeticPanel {
        let world = self.app.world();
        let computed = world
            .get::<ComputedDiegeticPanel>(self.panel)
            .expect("panel layout remains");
        assert!(computed.layout().solved().is_some(), "panel layout solves");
        computed
    }

    fn scroll_x(&self) -> f32 {
        let computed = self.solved();
        let field_index = computed
            .field_records()
            .iter()
            .find(|record| record.field_id == PanelElementId::named("field"))
            .expect("field has solved bounds")
            .element_index;
        let scroll = computed
            .layout()
            .solved()
            .expect("layout solved")
            .scrolls
            .iter()
            .find(|scroll| scroll.element_index == field_index)
            .expect("the field clips and scrolls its content");
        assert!(scroll.viewport_points.x <= FIELD_WIDTH + TOLERANCE);
        assert!(
            scroll.offset_points.y.abs() <= TOLERANCE,
            "field scrolls only sideways"
        );
        scroll.offset_points.x
    }

    fn assert_one_line(&self) {
        let computed = self.solved();
        let field = computed
            .field_records()
            .iter()
            .find(|record| record.field_id == PanelElementId::named("field"))
            .expect("field has solved bounds");
        let solved = computed.layout().solved().expect("layout solved");
        // This fixture has one field and no sibling elements, so every solved
        // element after its root belongs to the field's content.
        let descendants = solved
            .computed
            .get(field.element_index + 1..)
            .expect("field content has solved geometry");
        for (index, descendant) in descendants.iter().enumerate() {
            let bounds = descendant.bounds;
            assert!(
                bounds.height <= field.bounds.height + TOLERANCE,
                "field descendant {index} wrapped beyond the frame: {bounds:?} vs {:?}",
                field.bounds,
            );
        }
        assert!(
            !descendants.is_empty(),
            "field has display or editing content"
        );
    }

    fn assert_display_starts_inside_frame(&self) {
        let computed = self.solved();
        let field = &computed.field_records()[0];
        let display_x = computed
            .layout()
            .solved()
            .expect("layout solved")
            .computed
            .get(field.element_index + 1)
            .expect("display has solved geometry")
            .bounds
            .x;
        assert!(
            (display_x - field.bounds.x).abs() <= TOLERANCE,
            "idle display starts at the field's left edge",
        );
    }

    fn candidate_x(&self) -> f32 {
        self.app
            .world()
            .get::<Window>(self.window)
            .expect("window remains")
            .ime_position
            .x
    }
}

#[test]
fn idle_and_committed_long_value_stay_on_one_line() {
    let mut fixture = FieldFixture::new(LONG_VALUE);
    fixture.assert_one_line();
    fixture.assert_display_starts_inside_frame();

    fixture.open(LONG_VALUE);
    fixture.assert_one_line();
    fixture.press(KeyCode::Enter);
    fixture.app.update();

    fixture.assert_one_line();
    fixture.assert_display_starts_inside_frame();
    let computed = fixture.solved();
    assert_eq!(computed.field_records()[0].display_text, LONG_VALUE);
}

#[test]
fn caret_scrolls_on_each_ime_update_and_returns_to_the_start() {
    let mut fixture = FieldFixture::new("");
    fixture.open("");
    fixture.assert_one_line();
    let start_candidate_x = fixture.candidate_x();
    assert!(fixture.scroll_x().abs() <= TOLERANCE);

    let mut previous_offset = 0.0;
    let mut typed = String::new();
    for (index, character) in LONG_VALUE.chars().enumerate() {
        fixture.type_ime(&character.to_string());
        typed.push(character);
        assert_eq!(
            fixture.solved().field_records()[0].display_text,
            typed,
            "the layout must contain this character on its typed frame",
        );
        fixture.assert_one_line();
        let offset = fixture.scroll_x();
        if index < 2 {
            assert!(offset.abs() <= TOLERANCE, "short text starts unscrolled");
        }
        assert!(
            offset + TOLERANCE >= previous_offset,
            "offset reset across an IME rebuild"
        );
        let candidate_x = fixture.candidate_x();
        assert!(candidate_x >= start_candidate_x - TOLERANCE);
        assert!(
            candidate_x <= start_candidate_x + FIELD_WIDTH + TOLERANCE,
            "IME candidate passed the visible field on the typed frame: {candidate_x}",
        );
        previous_offset = offset;
    }
    assert!(
        previous_offset > 0.0,
        "long text must scroll within the field"
    );

    fixture.press(KeyCode::Home);
    fixture.assert_one_line();
    assert!(
        fixture.scroll_x().abs() <= TOLERANCE,
        "Home reveals the text start"
    );
    assert!(
        (fixture.candidate_x() - start_candidate_x).abs() <= 1.0,
        "IME candidate follows the caret back to the start",
    );
}

#[test]
fn shift_home_follows_selection_focus_to_visible_start() {
    let mut fixture = FieldFixture::new("");
    fixture.open("");
    fixture.type_ime(LONG_VALUE);
    assert!(fixture.scroll_x() > 0.0);

    fixture.press(KeyCode::ShiftLeft);
    fixture.press(KeyCode::Home);

    assert!(fixture.scroll_x().abs() <= TOLERANCE);
    fixture.assert_one_line();
    let field_x = fixture.solved().field_records()[0].bounds.x;
    let candidate_x = fixture.candidate_x();
    assert!(candidate_x >= field_x - TOLERANCE);
    assert!(candidate_x <= field_x + FIELD_WIDTH + TOLERANCE);
}
