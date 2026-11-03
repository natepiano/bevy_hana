//! External-client coverage for long-word sizing through a headless panel.

use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::MinimalPlugins;
use hana_diegetic::BoundingBox;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Mm;
use hana_diegetic::PanelBuildError;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Unit;

const WORD: &str = "NotRequestedWithoutBreak";
const FONT_SIZE: f32 = 4.0;
const PANEL_WIDTH_MM: f32 = 20.0;
const TOLERANCE: f32 = 0.001;
const FIXED_COLUMN_INDEX: usize = 1;
const WORD_ELEMENT_INDEX: usize = 2;

fn word_tree(column_width: Sizing, panel_width_mm: f32) -> LayoutTree {
    let mut builder = LayoutBuilder::new(panel_width_mm, 200.0);
    builder.with(
        El::column().width(column_width).height(Sizing::FIT),
        |builder| {
            builder.text(Text::new(WORD, TextStyle::new(FONT_SIZE)).wrap(TextWrap::Words));
        },
    );
    builder.build()
}

fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(DiegeticTextMeasurer::default())
        .add_plugins(HeadlessDiegeticUiPlugin);
    app
}

fn word_dimensions(app: &App) -> (f32, f32) {
    let measure = &app.world().resource::<DiegeticTextMeasurer>().measure_fn;
    let style = TextStyle::new(FONT_SIZE);
    let text_measure = style.as_measure().scaled(Unit::Millimeters.to_points());
    let word = measure(WORD, &text_measure);
    (word.width, word.line_height)
}

fn measured_width(app: &App, text: &str) -> f32 {
    let measure = &app.world().resource::<DiegeticTextMeasurer>().measure_fn;
    let text_measure = TextStyle::new(FONT_SIZE)
        .as_measure()
        .scaled(Unit::Millimeters.to_points());
    measure(text, &text_measure).width
}

fn spawn_fixed_word_panel(app: &mut App, width_points: f32) -> Result<Entity, PanelBuildError> {
    let width_mm = width_points / Unit::Millimeters.to_points();
    let panel = DiegeticPanel::world()
        .size(Mm(width_mm), Fit)
        .font_unit(Unit::Millimeters)
        .with_tree(word_tree(Sizing::fixed(width_mm), width_mm))
        .build()?;
    let entity = app.world_mut().spawn(panel).id();
    app.update();
    Ok(entity)
}

fn laid_out_word_line(
    computed: &ComputedDiegeticPanel,
) -> Option<(BoundingBox, BoundingBox, String)> {
    let layout = computed.layout().solved()?;
    let fixed_box = layout.computed[FIXED_COLUMN_INDEX].bounds;
    let lines: Vec<_> = layout
        .commands
        .iter()
        .filter(|command| command.element_idx == WORD_ELEMENT_INDEX)
        .collect();
    assert_eq!(lines.len(), 1, "one unbroken word should occupy one line");
    // The command kind is private to the library, so its Debug view is the
    // external client's only way to read the laid-out text.
    let debug = format!("{:?}", lines[0].kind);
    let text = debug.strip_prefix("Text { text: \"")?.split('"').next()?;
    Some((fixed_box, lines[0].bounds, text.to_string()))
}

fn assert_line_inside_box(fixed_box: BoundingBox, line: BoundingBox) {
    assert!(
        line.x + TOLERANCE >= fixed_box.x,
        "line starts left of box: {line:?} in {fixed_box:?}"
    );
    assert!(
        line.x + line.width <= fixed_box.x + fixed_box.width + TOLERANCE,
        "line ends right of box: {line:?} in {fixed_box:?}"
    );
}

fn drawn_lines(computed: &ComputedDiegeticPanel) -> Vec<String> {
    let Some(layout) = computed.layout().solved() else {
        return Vec::new();
    };
    layout
        .commands
        .iter()
        .filter_map(|command| {
            // RenderCommandKind is intentionally private to the library. Its
            // derived Debug view still exposes the emitted text for a client test.
            let debug = format!("{:?}", command.kind);
            let text = debug.strip_prefix("Text { text: \"")?;
            Some(text.split('"').next()?.to_string())
        })
        .collect()
}

#[test]
fn oversized_unbroken_word_uses_one_ellipsis_line_inside_fixed_panel() {
    let panel = DiegeticPanel::world()
        .size(Mm(PANEL_WIDTH_MM), Fit)
        .font_unit(Unit::Millimeters)
        .with_tree(word_tree(Sizing::fixed(PANEL_WIDTH_MM), PANEL_WIDTH_MM))
        .build();
    assert!(panel.is_ok());
    let Ok(panel) = panel else { return };
    let mut app = headless_app();
    let (word_width, line_height) = word_dimensions(&app);
    let panel_width = PANEL_WIDTH_MM * Unit::Millimeters.to_points();
    assert!(word_width > panel_width);
    let panel = app.world_mut().spawn(panel).id();
    app.update();

    let computed = app.world().get::<ComputedDiegeticPanel>(panel);
    assert!(computed.is_some());
    let Some(computed) = computed else { return };
    let bounds = computed.content_bounds();
    assert!(bounds.is_some());
    let Some(bounds) = bounds else { return };
    assert!(bounds.width <= panel_width + TOLERANCE);
    assert!((bounds.height - line_height).abs() < TOLERANCE);
    let lines = drawn_lines(computed);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].ends_with('…'),
        "expected visible ellipsis: {lines:?}"
    );
}

#[test]
fn fit_panel_keeps_whole_unbroken_word_on_one_line() {
    let panel = DiegeticPanel::world()
        .size(Fit, Mm(200.0))
        .font_unit(Unit::Millimeters)
        .with_tree(word_tree(Sizing::FIT, PANEL_WIDTH_MM))
        .build();
    assert!(panel.is_ok());
    let Ok(panel) = panel else { return };
    let mut app = headless_app();
    let (word_width, line_height) = word_dimensions(&app);
    let panel = app.world_mut().spawn(panel).id();
    app.update();

    let computed = app.world().get::<ComputedDiegeticPanel>(panel);
    assert!(computed.is_some());
    let Some(computed) = computed else { return };
    let bounds = computed.content_bounds();
    assert!(bounds.is_some());
    let Some(bounds) = bounds else { return };
    assert!(bounds.width + TOLERANCE >= word_width);
    assert!((bounds.height - line_height).abs() < TOLERANCE);
    assert_eq!(drawn_lines(computed), [WORD]);
}

#[test]
fn box_narrower_than_ellipsis_lays_out_no_glyph_and_keeps_line_height() {
    let mut app = headless_app();
    let ellipsis_width = measured_width(&app, "…");
    let (_, line_height) = word_dimensions(&app);
    let box_width = ellipsis_width - 0.25;
    assert!(box_width > 0.0);
    let panel = spawn_fixed_word_panel(&mut app, box_width);
    assert!(panel.is_ok(), "fixed word panel failed to build: {panel:?}");
    let Ok(panel) = panel else { return };
    let computed = app.world().get::<ComputedDiegeticPanel>(panel);
    assert!(computed.is_some());
    let Some(computed) = computed else { return };
    let line = laid_out_word_line(computed);
    assert!(line.is_some());
    let Some((fixed_box, line, text)) = line else {
        return;
    };

    assert!(
        (fixed_box.width - box_width).abs() < TOLERANCE,
        "fixed box {fixed_box:?}, requested width {box_width}, line {line:?}"
    );
    assert!((fixed_box.height - line_height).abs() < TOLERANCE);
    assert_eq!(
        line.width, 0.0,
        "a narrower box must lay out no glyph: {line:?}"
    );
    assert!((line.height - line_height).abs() < TOLERANCE);
    assert_eq!(text, "", "a narrower box must emit no text");
    assert_line_inside_box(fixed_box, line);
}

#[test]
fn box_just_wide_enough_lays_out_ellipsis_inside_box() {
    let mut app = headless_app();
    let ellipsis_width = measured_width(&app, "…");
    let first_character_and_ellipsis_width = measured_width(&app, "N…");
    let box_width = ellipsis_width + 0.25;
    assert!(box_width < first_character_and_ellipsis_width);
    let panel = spawn_fixed_word_panel(&mut app, box_width);
    assert!(panel.is_ok(), "fixed word panel failed to build: {panel:?}");
    let Ok(panel) = panel else { return };
    let computed = app.world().get::<ComputedDiegeticPanel>(panel);
    assert!(computed.is_some());
    let Some(computed) = computed else { return };
    let line = laid_out_word_line(computed);
    assert!(line.is_some());
    let Some((fixed_box, line, text)) = line else {
        return;
    };

    assert!(
        (fixed_box.width - box_width).abs() < TOLERANCE,
        "fixed box {fixed_box:?}, requested width {box_width}, line {line:?}"
    );
    assert!(
        (line.width - ellipsis_width).abs() < TOLERANCE,
        "only the ellipsis should fit: {line:?}, ellipsis advance {ellipsis_width}"
    );
    assert_eq!(text, "…", "the laid-out line must contain the ellipsis");
    assert_line_inside_box(fixed_box, line);
}
