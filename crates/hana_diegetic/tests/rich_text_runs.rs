//! External-client contracts for a single paragraph with several paint runs.

use std::sync::Arc;
use std::sync::Mutex;

use bevy::asset::AssetApp;
use bevy::asset::AssetPlugin;
use bevy::ecs::system::RunSystemOnce;
use bevy::gizmos::GizmoAsset;
use bevy::prelude::App;
use bevy::prelude::ChildOf;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Image;
use bevy::prelude::Mesh;
use bevy::prelude::MinimalPlugins;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::Shader;
use hana_diegetic::BoundingBox;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands as _;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::FontFeatureFlags;
use hana_diegetic::FontFeatures;
use hana_diegetic::FontSlant;
use hana_diegetic::FontWeight;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutEngine;
use hana_diegetic::LayoutResult;
use hana_diegetic::LayoutTree;
use hana_diegetic::LayoutTreeChange;
use hana_diegetic::MeasureTextFn;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelTextLayout;
use hana_diegetic::PanelTextRuns;
use hana_diegetic::Pt;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::RichParagraph;
use hana_diegetic::RichText;
use hana_diegetic::ShapedTextCache;
use hana_diegetic::SpanStyle;
use hana_diegetic::Text;
use hana_diegetic::TextContent;
use hana_diegetic::TextDimensions;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Unit;

const TOLERANCE: f32 = 0.001;
const BASE_COLOR: Color = Color::srgb_u8(148, 161, 169);
const FIRST_COLOR: Color = Color::srgb_u8(230, 70, 90);
const SECOND_COLOR: Color = Color::srgb_u8(60, 210, 120);
const THIRD_COLOR: Color = Color::srgb_u8(70, 130, 240);
const RICH_ID: &str = "paragraph";
const PLAIN_ID: &str = "plain-sentinel";

#[test]
fn rich_colour_leaf_matches_plain_box_lines_and_ink_at_fractional_positions() -> Result<(), String>
{
    let mut app = rendering_app();
    for top in [241.0, 241.75] {
        for width in [300.0, 70.0] {
            let tree = colour_twin_tree(&app, width, top)?;
            let panel = spawn_twin_panel(&mut app, tree)?;
            assert_twin_geometry(&app, panel)?;
        }
    }
    Ok(())
}

#[test]
fn rich_text_rejects_wrap_modes_deferred_to_typography() {
    for wrap in [TextWrap::Words, TextWrap::Newlines, TextWrap::TruncateTail] {
        assert!(
            std::panic::catch_unwind(|| RichText::new(base_style()).wrap(wrap)).is_err(),
            "rich text must reject {wrap:?} until the typography phase"
        );
    }
}

#[test]
fn rich_authored_ellipsis_after_a_hard_newline_keeps_its_own_colour() -> Result<(), String> {
    // The former Newlines reflow misclassified an authored ellipsis as synthetic.
    // Reject that deferred mode and preserve paint on the supported hard line.
    assert!(
        std::panic::catch_unwind(|| RichText::new(base_style()).wrap(TextWrap::Newlines)).is_err()
    );
    let mut app = rendering_app();
    let tree = rich_tree(
        base_style(),
        &[
            ("A\n", SpanStyle::new().with_color(FIRST_COLOR)),
            ("…B", SpanStyle::new().with_color(THIRD_COLOR)),
        ],
    );
    let panel = spawn_panel(&mut app, tree)?;
    let runs = paint_runs(&app, panel, RICH_ID)?;
    let ellipsis = runs
        .iter()
        .find(|run| run.line == 1 && run.text.contains('…'))
        .ok_or("authored ellipsis was not reified on the second line")?;
    assert_eq!(ellipsis.text, "…B");
    assert_eq!(ellipsis.style.color(), THIRD_COLOR);
    assert_eq!(
        runs.iter()
            .find(|run| run.line == 0 && run.text.trim_end_matches('\n') == "A")
            .ok_or("missing first line")?
            .style
            .color(),
        FIRST_COLOR
    );
    Ok(())
}

fn colour_twin_tree(app: &App, width: f32, top: f32) -> Result<LayoutTree, String> {
    let registry = app.world().resource::<hana_diegetic::FontRegistry>();
    let trims = registry
        .font(hana_diegetic::FontId::MONOSPACE)
        .map_err(|error| error.to_string())?
        .metrics(12.0)
        .shaped_line_cap_box_trims();
    let style = TextStyle::new(Pt(12.0)).with_shadow_mode(GlyphShadowMode::None);
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(width), Pt(537.75)).padding(
        hana_diegetic::Padding::new(Pt(0.0), Pt(0.0), Pt(top), Pt(0.0)),
    ));
    builder.text(
        Text::new("Align horizontal centers", style.clone())
            .trim_text_box(Pt(trims.top()), Pt(trims.bottom()))
            .wrap(TextWrap::None)
            .id(PLAIN_ID),
    );
    builder.rich_text(
        RichText::new(style)
            .trim_text_box(Pt(trims.top()), Pt(trims.bottom()))
            .wrap(TextWrap::None)
            .id(RICH_ID),
        |rich| {
            rich.span(("Align", SpanStyle::new().with_color(FIRST_COLOR)))
                .span(" horizontal centers");
        },
    );
    Ok(builder.build())
}

fn spawn_twin_panel(app: &mut App, tree: LayoutTree) -> Result<Entity, String> {
    let panel = app
        .world_mut()
        .spawn(
            DiegeticPanel::world()
                .size(Pt(800.0), Pt(537.75))
                .font_unit(Unit::Points)
                .with_tree(tree)
                .build()
                .map_err(|error| error.to_string())?,
        )
        .id();
    app.update();
    Ok(panel)
}

fn assert_twin_geometry(app: &App, panel: Entity) -> Result<usize, String> {
    let plain = paint_runs(app, panel, PLAIN_ID)?;
    let rich = paint_runs(app, panel, RICH_ID)?;
    let layout = solved_panel(app, panel)?;
    let plain_leaf = layout.computed[plain.first().ok_or("no plain run")?.element].bounds;
    let rich_leaf = layout.computed[rich.first().ok_or("no rich run")?.element].bounds;
    assert_close(rich_leaf.width, plain_leaf.width, "twin leaf width");
    assert_close(rich_leaf.height, plain_leaf.height, "twin leaf height");
    assert_eq!(
        plain.len(),
        rich.iter()
            .map(|run| run.line)
            .max()
            .map_or(0, |line| line + 1)
    );
    for run in &plain {
        let line = rich
            .iter()
            .filter(|rich| rich.line == run.line)
            .collect::<Vec<_>>();
        assert_eq!(
            line.iter()
                .map(|rich| rich.text.as_str())
                .collect::<String>(),
            run.text
        );
        assert_close(
            line.iter()
                .map(|r| r.bounds.x + r.bounds.width)
                .fold(0.0, f32::max)
                - rich_leaf.x,
            run.bounds.x + run.bounds.width - plain_leaf.x,
            "twin line right",
        );
        for rich in &line {
            assert_close(
                rich.bounds.y - rich_leaf.y,
                run.bounds.y - plain_leaf.y,
                "twin line top",
            );
            assert_close(rich.bounds.height, run.bounds.height, "twin line pitch");
        }
        // The actual emitted quad union pins baseline and ink placement, including
        // Parley's negative leading, rather than only comparing command boxes.
        let drawn = drawn_line_bounds(app, &[run], plain_leaf)?;
        let rich_drawn = drawn_line_bounds(app, &line, rich_leaf)?;
        assert_close(rich_drawn.x, drawn.x, "twin drawn left");
        assert_close(rich_drawn.y, drawn.y, "twin drawn top/baseline");
        assert_close(rich_drawn.width, drawn.width, "twin drawn width");
        assert_close(
            rich_drawn.height,
            drawn.height,
            "twin drawn height/baseline",
        );
    }
    Ok(plain.len())
}

fn drawn_line_bounds(
    app: &App,
    runs: &[&ReifiedPaintRun],
    leaf: BoundingBox,
) -> Result<BoundingBox, String> {
    let mut left = f32::INFINITY;
    let mut top = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for run in runs {
        let bounds = app
            .world()
            .get::<hana_diegetic::DrawnTextBounds>(run.entity)
            .ok_or_else(|| format!("run {:?} has no drawn bounds", run.text))?
            .local_aabb();
        let placement = app
            .world()
            .get::<PanelTextLayout>(run.entity)
            .ok_or("no placement")?;
        left = left.min((bounds.min().x + placement.anchor_x) / placement.scale_x - leaf.x);
        right = right.max((bounds.max().x + placement.anchor_x) / placement.scale_x - leaf.x);
        top = top.min((placement.anchor_y - bounds.max().y) / placement.scale_y - leaf.y);
        bottom = bottom.max((placement.anchor_y - bounds.min().y) / placement.scale_y - leaf.y);
    }
    Ok(BoundingBox {
        x:      left,
        y:      top,
        width:  right - left,
        height: bottom - top,
    })
}

fn base_style() -> TextStyle {
    TextStyle::new(Pt(16.0))
        .with_color(BASE_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn rich_tree(base: TextStyle, spans: &[(&str, SpanStyle)]) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(800.0), Pt(200.0)));
    builder.rich_text(
        RichText::new(base).wrap(TextWrap::None).id(RICH_ID),
        |rich| {
            for &(text, style) in spans {
                rich.span((text, style));
            }
        },
    );
    builder.build()
}

fn approximate_measure() -> MeasureTextFn {
    Arc::new(|text, measure| TextDimensions {
        width:       text.chars().map(|_| measure.size * 0.6).sum::<f32>()
            + text
                .chars()
                .filter(|&character| character == ' ')
                .map(|_| measure.word_spacing)
                .sum::<f32>(),
        height:      measure.size,
        line_height: measure.size,
    })
}

fn solve(tree: &LayoutTree) -> LayoutResult {
    LayoutEngine::new(approximate_measure()).compute(tree, 800.0, 200.0, 1.0)
}

fn paragraph(layout: &LayoutResult) -> Result<&RichParagraph, String> {
    let paragraphs = layout
        .commands
        .iter()
        .filter_map(|command| match &command.kind {
            RenderCommandKind::RichText {
                paragraph,
                line_index,
            } => Some((paragraph.as_ref(), *line_index)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [(paragraph, line)] = paragraphs.as_slice() else {
        return Err(format!(
            "one rich leaf must emit one paragraph command, found {}",
            paragraphs.len()
        ));
    };
    assert_eq!(*line, 0);
    Ok(*paragraph)
}

fn rich_bounds(layout: &LayoutResult) -> Result<BoundingBox, String> {
    layout
        .commands
        .iter()
        .find_map(|command| {
            matches!(&command.kind, RenderCommandKind::RichText { .. }).then_some(command.bounds)
        })
        .ok_or_else(|| "layout emitted no rich paragraph".to_owned())
}

fn rendering_app() -> App {
    // Execute the production measure/reify/shape/batch systems without a GPU or window.
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Shader>()
        .init_asset::<Mesh>()
        .init_asset::<GizmoAsset>()
        .init_asset::<Image>()
        .init_asset::<ShaderBuffer>()
        .add_plugins(DiegeticUiPlugin);
    app
}

fn spawn_panel(app: &mut App, tree: LayoutTree) -> Result<Entity, String> {
    let panel = DiegeticPanel::world()
        .size(Pt(800.0), Pt(200.0))
        .font_unit(Unit::Points)
        .with_tree(tree)
        .build()
        .map_err(|error| format!("cannot build rich test panel: {error}"))?;
    let entity = app.world_mut().spawn(panel).id();
    app.update();
    Ok(entity)
}

fn solved_panel(app: &App, panel: Entity) -> Result<&LayoutResult, String> {
    app.world()
        .get::<ComputedDiegeticPanel>(panel)
        .and_then(|computed| computed.layout().solved())
        .ok_or_else(|| format!("panel {panel} has no solved layout"))
}

fn replace_tree(app: &mut App, panel: Entity, tree: LayoutTree) -> Result<(), String> {
    app.world_mut()
        .run_system_once(move |mut commands: Commands| commands.set_tree(panel, tree.clone()))
        .map_err(|error| format!("cannot run tree replacement: {error}"))?
        .map_err(|error| format!("cannot replace test tree: {error}"))?;
    app.update();
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
struct ReifiedPaintRun {
    entity:  Entity,
    text:    String,
    style:   TextStyle,
    id:      PanelElementId,
    line:    usize,
    element: usize,
    bounds:  BoundingBox,
}

fn paint_runs(app: &App, panel: Entity, id: &str) -> Result<Vec<ReifiedPaintRun>, String> {
    let runs = app
        .world()
        .get::<PanelTextRuns>(panel)
        .ok_or_else(|| format!("panel {panel} has no text children"))?;
    let id = PanelElementId::named(id);
    let mut result = Vec::new();
    for entity in runs.iter().copied() {
        let layout = app
            .world()
            .get::<PanelTextLayout>(entity)
            .ok_or_else(|| format!("run {entity} has no layout"))?;
        if layout.id != id {
            continue;
        }
        let text = app
            .world()
            .get::<TextContent>(entity)
            .ok_or_else(|| format!("run {entity} has no source text"))?;
        let style = app
            .world()
            .get::<TextStyle>(entity)
            .ok_or_else(|| format!("run {entity} has no resolved style"))?;
        result.push(ReifiedPaintRun {
            entity,
            text: text.text().to_owned(),
            style: style.clone(),
            id: layout.id.clone(),
            line: layout.line_index,
            element: layout.element_idx,
            bounds: layout.bounds,
        });
    }
    result.sort_by(|left, right| left.bounds.x.total_cmp(&right.bounds.x));
    Ok(result)
}

fn three_run_tree(colors: [Color; 3]) -> LayoutTree {
    rich_tree(
        base_style(),
        &[
            ("AV", SpanStyle::new().with_color(colors[0])),
            ("ATAR", SpanStyle::new().with_color(colors[1])),
            (" café", SpanStyle::new().with_color(colors[2])),
        ],
    )
}

fn assert_close(actual: f32, expected: f32, context: &str) {
    assert!(
        (actual - expected).abs() <= TOLERANCE,
        "{context}: got {actual}, expected {expected}"
    );
}

#[test]
fn rich_text_concatenates_spans_and_records_char_boundary_ranges() -> Result<(), String> {
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(800.0), Pt(200.0)));
    builder.rich_text(RichText::new(base_style()).id(RICH_ID), |rich| {
        rich.span(("é", SpanStyle::new().with_color(FIRST_COLOR)))
            .span("中")
            .span(("🙂e\u{301}", SpanStyle::new().with_color(THIRD_COLOR)));
    });
    let tree = builder.build();
    assert_eq!(
        tree.len(),
        2,
        "spans must remain one leaf, not child leaves"
    );
    let solved = solve(&tree);
    let paragraph = paragraph(&solved)?;
    assert_eq!(paragraph.text, "é中🙂e\u{301}");
    assert_eq!(
        paragraph
            .runs
            .iter()
            .map(|run| run.range.clone())
            .collect::<Vec<_>>(),
        [0..2, 2..5, 5..12]
    );
    for run in &paragraph.runs {
        assert!(paragraph.text.is_char_boundary(run.range.start));
        assert!(paragraph.text.is_char_boundary(run.range.end));
    }
    Ok(())
}

#[test]
fn rich_text_ignores_empty_spans_and_coalesces_equal_neighbours() -> Result<(), String> {
    let base = base_style().with_word_spacing(2.0);
    let tree = rich_tree(
        base,
        &[
            ("", SpanStyle::new().with_color(THIRD_COLOR)),
            ("a", SpanStyle::new()),
            ("", SpanStyle::new().with_color(FIRST_COLOR)),
            (
                "b",
                SpanStyle::new()
                    .with_color(BASE_COLOR)
                    .with_word_spacing(2.0),
            ),
            ("c", SpanStyle::new().with_color(FIRST_COLOR)),
            (
                "d",
                SpanStyle::new()
                    .with_color(FIRST_COLOR)
                    .with_word_spacing(2.0),
            ),
            ("", SpanStyle::new()),
        ],
    );
    let layout = solve(&tree);
    let paragraph = paragraph(&layout)?;
    assert_eq!(paragraph.text, "abcd");
    assert_eq!(paragraph.runs.len(), 2);
    assert_eq!(paragraph.runs[0].range, 0..2);
    assert_eq!(paragraph.runs[1].range, 2..4);
    assert_eq!(paragraph.runs[0].color, BASE_COLOR);
    assert_eq!(paragraph.runs[1].color, FIRST_COLOR);
    Ok(())
}

#[test]
fn span_style_inherits_unset_fields_from_the_box() -> Result<(), String> {
    let base = base_style()
        .with_font(7)
        .with_weight(FontWeight::BOLD)
        .with_slant(FontSlant::Italic)
        .with_line_height(1.4)
        .with_letter_spacing(0.75)
        .with_word_spacing(2.25)
        .with_font_features(FontFeatures::new().without(FontFeatureFlags::CALT));
    let tree = rich_tree(
        base.clone(),
        &[
            ("base", SpanStyle::new()),
            ("paint", SpanStyle::new().with_color(FIRST_COLOR)),
            ("space", SpanStyle::new().with_word_spacing(6.0)),
        ],
    );
    let layout = solve(&tree);
    let paragraph = paragraph(&layout)?;
    assert_eq!(paragraph.runs.len(), 3);
    assert_eq!(paragraph.runs[0].measure, base.as_measure());
    assert_eq!(paragraph.runs[0].color, base.color());
    assert_eq!(paragraph.runs[1].measure, base.as_measure());
    assert_eq!(paragraph.runs[1].color, FIRST_COLOR);
    assert_eq!(
        paragraph.runs[2].measure,
        base.with_word_spacing(6.0).as_measure()
    );
    assert_eq!(paragraph.runs[2].color, BASE_COLOR);
    Ok(())
}

#[test]
fn rich_leaf_measures_once_per_layout_request() -> Result<(), String> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let measure: MeasureTextFn = Arc::new(move |text, _| {
        observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(text.to_owned());
        TextDimensions {
            width:       137.0,
            height:      23.0,
            line_height: 23.0,
        }
    });
    let engine = LayoutEngine::new(measure);
    let tree = three_run_tree([FIRST_COLOR, SECOND_COLOR, THIRD_COLOR]);
    for width in [800.0, 500.0] {
        let layout = engine.compute(&tree, width, 200.0, 1.0);
        assert_close(rich_bounds(&layout)?.width, 137.0, "whole paragraph width");
    }
    let calls = calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        *calls,
        ["AVATAR café", "AVATAR café"],
        "one request must measure the concatenation exactly once"
    );
    Ok(())
}

#[test]
fn rich_leaf_width_matches_the_same_string_as_plain_text_when_only_colour_differs()
-> Result<(), String> {
    let mut app = rendering_app();
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(800.0), Pt(200.0)));
    builder.text(Text::new("AVATAR café", base_style()).id(PLAIN_ID));
    builder.rich_text(RichText::new(base_style()).id(RICH_ID), |rich| {
        rich.span(("AV", SpanStyle::new().with_color(FIRST_COLOR)))
            .span(("ATAR", SpanStyle::new().with_color(SECOND_COLOR)))
            .span((" café", SpanStyle::new().with_color(THIRD_COLOR)));
    });
    let panel = spawn_panel(&mut app, builder.build())?;
    let layout = solved_panel(&app, panel)?;
    let plain = layout
        .commands
        .iter()
        .find_map(|command| match &command.kind {
            RenderCommandKind::Text { text, .. } if text == "AVATAR café" => Some(command.bounds),
            _ => None,
        })
        .ok_or_else(|| "plain sentinel command is absent".to_owned())?;
    let rich = rich_bounds(layout)?;
    assert_close(
        rich.width,
        plain.width,
        "colour boundaries must preserve whole-string shaping",
    );
    assert_close(
        rich.height,
        plain.height,
        "colour boundaries must preserve the line box",
    );
    Ok(())
}

#[test]
fn plain_and_rich_measurements_keep_run_spacing_separate() -> Result<(), String> {
    let mut app = rendering_app();
    let base = base_style().with_word_spacing(1.0);
    let plain_tree = || {
        let mut builder = LayoutBuilder::with_root(El::column().size(Pt(800.0), Pt(200.0)));
        builder.text(Text::new("A B C", base.clone()).id(PLAIN_ID));
        builder.build()
    };
    let plain_panel = spawn_panel(&mut app, plain_tree())?;
    let plain_bounds = solved_panel(&app, plain_panel)?
        .commands
        .iter()
        .find_map(|command| {
            matches!(&command.kind, RenderCommandKind::Text { .. }).then_some(command.bounds)
        })
        .ok_or_else(|| "plain command is absent".to_owned())?;
    let ordinary = rich_tree(base.clone(), &[("A B C", SpanStyle::new())]);
    let spaced = rich_tree(
        base.clone(),
        &[("A B C", SpanStyle::new().with_word_spacing(5.0))],
    );
    for (tree, expected_extra) in [
        (ordinary.clone(), 0.0),
        (spaced.clone(), 8.0),
        (spaced, 8.0),
        (ordinary, 0.0),
    ] {
        let panel = spawn_panel(&mut app, tree)?;
        assert_close(
            rich_bounds(solved_panel(&app, panel)?)?.width,
            plain_bounds.width + expected_extra,
            "same text and base style retain each paragraph's run spacing",
        );
    }
    let repeated_plain = spawn_panel(&mut app, plain_tree())?;
    let repeated_bounds = solved_panel(&app, repeated_plain)?
        .commands
        .iter()
        .find_map(|command| {
            matches!(&command.kind, RenderCommandKind::Text { .. }).then_some(command.bounds)
        })
        .ok_or_else(|| "repeated plain command is absent".to_owned())?;
    assert_eq!(plain_bounds, repeated_bounds);
    Ok(())
}

#[test]
fn word_spacing_override_widens_only_its_spaces() -> Result<(), String> {
    let mut app = rendering_app();
    let base = base_style().with_word_spacing(1.0);
    let original = rich_tree(
        base.clone(),
        &[
            ("A ", SpanStyle::new().with_color(FIRST_COLOR)),
            ("B C ", SpanStyle::new().with_color(SECOND_COLOR)),
            ("D E", SpanStyle::new().with_color(THIRD_COLOR)),
        ],
    );
    let changed = rich_tree(
        base.clone(),
        &[
            ("A ", SpanStyle::new().with_color(FIRST_COLOR)),
            (
                "B C ",
                SpanStyle::new()
                    .with_color(SECOND_COLOR)
                    .with_word_spacing(5.0),
            ),
            ("D E", SpanStyle::new().with_color(THIRD_COLOR)),
        ],
    );
    assert_eq!(
        original.classify_change(&changed),
        LayoutTreeChange::LayoutAffecting
    );
    let baseline_panel = spawn_panel(&mut app, original)?;
    let changed_panel = spawn_panel(&mut app, changed)?;
    let baseline = rich_bounds(solved_panel(&app, baseline_panel)?)?;
    let wider = rich_bounds(solved_panel(&app, changed_panel)?)?;
    assert_close(
        wider.width - baseline.width,
        8.0,
        "only two overridden spaces widen by four points each",
    );
    assert_close(
        wider.height,
        baseline.height,
        "word spacing must not change height",
    );
    let before_runs = paint_runs(&app, baseline_panel, RICH_ID)?;
    let after_runs = paint_runs(&app, changed_panel, RICH_ID)?;
    assert_eq!(before_runs.len(), 3);
    assert_eq!(after_runs.len(), 3);
    assert_close(
        after_runs[0].bounds.x,
        before_runs[0].bounds.x,
        "preceding run stays still",
    );
    assert_close(
        after_runs[1].bounds.x,
        before_runs[1].bounds.x,
        "override begins after the preceding spaces",
    );
    assert_close(
        after_runs[2].bounds.x - before_runs[2].bounds.x,
        8.0,
        "following run moves by only the overridden advances",
    );
    let no_spaces = rich_tree(base, &[("ABCDE", SpanStyle::new().with_word_spacing(5.0))]);
    let ordinary = rich_tree(
        base_style().with_word_spacing(1.0),
        &[("ABCDE", SpanStyle::new())],
    );
    let ordinary_panel = spawn_panel(&mut app, ordinary)?;
    let no_spaces_panel = spawn_panel(&mut app, no_spaces)?;
    assert_close(
        rich_bounds(solved_panel(&app, no_spaces_panel)?)?.width,
        rich_bounds(solved_panel(&app, ordinary_panel)?)?.width,
        "an override without spaces changes no advance",
    );
    Ok(())
}

#[test]
fn colour_only_change_reuses_the_shaped_paragraph() -> Result<(), String> {
    let mut app = rendering_app();
    let cache = app.world().resource::<ShapedTextCache>().clone();
    let initial_shapes = cache.rich_shape_count();
    let original = three_run_tree([FIRST_COLOR, SECOND_COLOR, THIRD_COLOR]);
    let recolored = three_run_tree([SECOND_COLOR, THIRD_COLOR, FIRST_COLOR]);
    assert_eq!(
        original.classify_change(&recolored),
        LayoutTreeChange::VisualOnly
    );
    let panel = spawn_panel(&mut app, original)?;
    let cold_shapes = cache.rich_shape_count();
    assert!(
        cold_shapes > initial_shapes,
        "the cold paragraph must exercise the actual shaping observer"
    );
    let before = paint_runs(&app, panel, RICH_ID)?;
    assert_eq!(
        before.len(),
        3,
        "the control paragraph must already be reified"
    );
    let bounds = rich_bounds(solved_panel(&app, panel)?)?;
    let material_rows = app
        .world()
        .resource::<DiegeticPerfStats>()
        .material_table
        .rows
        .get();
    replace_tree(&mut app, panel, recolored)?;
    let after = paint_runs(&app, panel, RICH_ID)?;
    assert_eq!(
        before.iter().map(|run| run.entity).collect::<Vec<_>>(),
        after.iter().map(|run| run.entity).collect::<Vec<_>>()
    );
    assert_eq!(
        after
            .iter()
            .map(|run| run.style.color())
            .collect::<Vec<_>>(),
        [SECOND_COLOR, THIRD_COLOR, FIRST_COLOR]
    );
    assert_eq!(rich_bounds(solved_panel(&app, panel)?)?, bounds);
    let perf = app.world().resource::<DiegeticPerfStats>();
    assert_eq!(
        perf.panel_text.parley_ms, 0.0,
        "paint-only edits must perform no Parley shaping"
    );
    assert_eq!(
        perf.material_table.rows.get(),
        material_rows,
        "paint updates reuse material slots"
    );
    assert_eq!(
        cache.rich_shape_count(),
        cold_shapes,
        "recolouring reuses the actual cached paragraph"
    );
    assert_coalescing_reuses_geometry(&mut app, panel, &cache, cold_shapes, bounds)
}

fn assert_coalescing_reuses_geometry(
    app: &mut App,
    panel: Entity,
    cache: &ShapedTextCache,
    cold_shapes: usize,
    bounds: BoundingBox,
) -> Result<(), String> {
    let coalesced = three_run_tree([SECOND_COLOR; 3]);
    assert_eq!(
        three_run_tree([SECOND_COLOR, THIRD_COLOR, FIRST_COLOR]).classify_change(&coalesced),
        LayoutTreeChange::VisualOnly
    );
    replace_tree(app, panel, coalesced)?;
    let coalesced_runs = paint_runs(app, panel, RICH_ID)?;
    assert_eq!(
        coalesced_runs.len(),
        1,
        "equal paint neighbours merge without changing the paragraph's geometry"
    );
    assert_eq!(coalesced_runs[0].text, "AVATAR café");
    assert_eq!(coalesced_runs[0].style.color(), SECOND_COLOR);
    assert_eq!(rich_bounds(solved_panel(app, panel)?)?, bounds);
    assert_eq!(
        app.world()
            .resource::<DiegeticPerfStats>()
            .panel_text
            .parley_ms,
        0.0
    );
    assert_eq!(
        cache.rich_shape_count(),
        cold_shapes,
        "paint-run coalescing must not invalidate cached geometry"
    );
    let size_changed = rich_tree(
        TextStyle::new(Pt(19.0))
            .with_color(BASE_COLOR)
            .with_shadow_mode(GlyphShadowMode::None),
        &[("AVATAR café", SpanStyle::new().with_color(SECOND_COLOR))],
    );
    replace_tree(app, panel, size_changed)?;
    assert!(
        cache.rich_shape_count() > cold_shapes,
        "a genuine geometry edit exercises the miss counter"
    );
    Ok(())
}

#[test]
fn each_paint_run_reifies_one_child_with_its_colour() -> Result<(), String> {
    let mut app = rendering_app();
    let panel = spawn_panel(
        &mut app,
        three_run_tree([FIRST_COLOR, SECOND_COLOR, THIRD_COLOR]),
    )?;
    let layout = solved_panel(&app, panel)?;
    assert_eq!(paragraph(layout)?.runs.len(), 3);
    let runs = paint_runs(&app, panel, RICH_ID)?;
    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>(),
        ["AV", "ATAR", " café"]
    );
    assert_eq!(
        runs.iter().map(|run| run.style.color()).collect::<Vec<_>>(),
        [FIRST_COLOR, SECOND_COLOR, THIRD_COLOR]
    );
    assert!(
        runs.iter()
            .all(|run| run.line == 0 && run.element == runs[0].element)
    );
    assert!(
        runs.iter()
            .all(|run| run.bounds.width > 0.0 && run.bounds.height > 0.0)
    );
    assert!(
        runs.windows(2)
            .all(|pair| pair[0].bounds.x < pair[1].bounds.x)
    );
    for run in &runs {
        let parent = app
            .world()
            .get::<ChildOf>(run.entity)
            .ok_or_else(|| format!("paint run {} is not a panel child", run.entity))?;
        assert_eq!(parent.parent(), panel);
    }
    assert_eq!(
        app.world()
            .get::<PanelTextRuns>(panel)
            .map(|runs| runs.len()),
        Some(3)
    );
    Ok(())
}

#[test]
fn rich_runs_keep_identity_across_an_unchanged_rebuild() -> Result<(), String> {
    let mut app = rendering_app();
    let tree = three_run_tree([FIRST_COLOR, SECOND_COLOR, THIRD_COLOR]);
    let rebuilt = three_run_tree([FIRST_COLOR, SECOND_COLOR, THIRD_COLOR]);
    assert_eq!(tree.classify_change(&rebuilt), LayoutTreeChange::Identical);
    let panel = spawn_panel(&mut app, tree)?;
    let before = paint_runs(&app, panel, RICH_ID)?;
    assert_eq!(before.len(), 3);
    replace_tree(&mut app, panel, rebuilt)?;
    assert_eq!(paint_runs(&app, panel, RICH_ID)?, before);
    Ok(())
}

fn sentinel_tree(with_rich: bool) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(800.0), Pt(200.0)));
    builder.text(Text::new("Plain AV café", base_style()).id(PLAIN_ID));
    if with_rich {
        builder.rich_text(RichText::new(base_style()).id(RICH_ID), |rich| {
            rich.span(("Rich ", SpanStyle::new().with_color(FIRST_COLOR)))
                .span(("neighbour", SpanStyle::new().with_color(SECOND_COLOR)));
        });
    }
    builder.build()
}

#[test]
fn plain_text_leaf_is_unchanged_beside_a_rich_leaf() -> Result<(), String> {
    let mut app = rendering_app();
    let panel = spawn_panel(&mut app, sentinel_tree(false))?;
    let before = paint_runs(&app, panel, PLAIN_ID)?;
    assert_eq!(before.len(), 1);
    replace_tree(&mut app, panel, sentinel_tree(true))?;
    assert_eq!(paint_runs(&app, panel, PLAIN_ID)?, before);
    assert_eq!(paint_runs(&app, panel, RICH_ID)?.len(), 2);
    let solved = solved_panel(&app, panel)?;
    let plain_commands = solved
        .commands
        .iter()
        .filter_map(|command| match &command.kind {
            RenderCommandKind::Text { text, config } => Some((text.as_str(), config)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(plain_commands.len(), 1);
    assert_eq!(plain_commands[0].0, "Plain AV café");
    assert_eq!(plain_commands[0].1.as_measure(), base_style().as_measure());
    assert_eq!(plain_commands[0].1.color(), BASE_COLOR);
    Ok(())
}
