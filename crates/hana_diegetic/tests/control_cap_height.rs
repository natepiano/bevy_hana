//! Font-backed layout invariants alongside the drawn control-label proof.

use bevy::ecs::system::RunSystemOnce as _;
use bevy::prelude::App;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::MinimalPlugins;
use hana_diegetic::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn solve(tree: LayoutTree) -> TestResult<(App, Entity)> {
    let fonts = FontRegistry::new()?;
    let context = fonts.font_context();
    let measure_fn = std::sync::Arc::new(move |text: &str, measure: &TextMeasure| {
        let mut context = context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut layout = parley::Layout::<()>::new();
        let mut layout_context = parley::LayoutContext::default();
        let mut builder = layout_context.ranged_builder(&mut context, text, 1.0, true);
        builder.push_default(parley::style::StyleProperty::FontFamily(
            parley::style::FontFamily::named("JetBrains Mono"),
        ));
        builder.push_default(parley::style::StyleProperty::FontSize(measure.size));
        builder.build_into(&mut layout, text);
        drop(context);
        layout.break_all_lines(None);
        TextDimensions {
            width:       layout.full_width(),
            height:      layout.height(),
            line_height: layout
                .lines()
                .next()
                .map_or(0.0, |line| line.metrics().line_height),
        }
    });
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(DiegeticTextMeasurer { measure_fn })
        .insert_resource(fonts)
        .add_plugins(HeadlessDiegeticUiPlugin);
    let panel = DiegeticPanel::world()
        .size(Pt(240.0), Pt(120.0))
        .font_unit(Unit::Points)
        .with_tree(tree)
        .build()?;
    let panel = app.world_mut().spawn(panel).id();
    app.update();
    Ok((app, panel))
}

fn text_command(app: &App, panel: Entity, label: &str) -> TestResult<BoundingBox> {
    app.world()
        .get::<ComputedDiegeticPanel>(panel)
        .ok_or("missing computed panel")?
        .layout()
        .solved()
        .ok_or("missing solved panel")?
        .commands
        .iter()
        .find_map(|command| match &command.kind {
            RenderCommandKind::Text { text, .. } if text == label => Some(command.bounds),
            RenderCommandKind::RichText { paragraph, .. } if paragraph.text == label => {
                Some(command.bounds)
            },
            _ => None,
        })
        .ok_or_else(|| format!("missing text command for {label}").into())
}

fn flat_cap(app: &App, style: &TextStyle) -> TestResult<TextInk> {
    match app
        .world()
        .resource::<FontRegistry>()
        .measure_ink("H", &style.as_measure())?
    {
        TextInkMeasurement::Visible(ink) => Ok(ink),
        TextInkMeasurement::NoVisibleInk => Err("H must have visible ink".into()),
    }
}

#[test]
fn control_cap_height_centres_nested_fit_labels_and_ignores_descenders() -> TestResult {
    let style = TextStyle::new(Pt(11.0));
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(240.0), Pt(120.0)));
    for label in ["Open", "Jump"] {
        builder.with(
            El::row().size(Pt(100.0), Pt(30.0)).align_y(AlignY::Center),
            |builder| {
                builder.with(
                    El::new().width(Sizing::FIT).height(Sizing::FIT),
                    |builder| {
                        builder.text(Text::new(label, style.clone()));
                    },
                );
            },
        );
    }
    let (app, panel) = solve(builder.build())?;
    let cap = flat_cap(&app, &style)?;
    let open = text_command(&app, panel, "Open")?;
    let jump = text_command(&app, panel, "Jump")?;
    let cap_mid = f32::midpoint(cap.top, cap.bottom);
    assert!((open.y + cap_mid - 15.0).abs() < 0.001);
    assert!((jump.y + cap_mid - 45.0).abs() < 0.001);
    assert!((jump.y - open.y - 30.0).abs() < 0.001);
    Ok(())
}

#[test]
fn control_cap_height_preserves_top_bottom_and_multiline_rules() -> TestResult {
    let style = TextStyle::new(Pt(11.0));
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(240.0), Pt(120.0)));
    for (label, align) in [
        ("Top", AlignY::Top),
        ("Bottom", AlignY::Bottom),
        ("Two\nLines", AlignY::Center),
    ] {
        builder.text(
            Text::new(label, style.clone())
                .wrap(TextWrap::Newlines)
                .layout(El::new().size(Pt(100.0), Pt(40.0)).align_y(align)),
        );
    }
    let (app, panel) = solve(builder.build())?;
    let measure = &app.world().resource::<DiegeticTextMeasurer>().measure_fn;
    let line = measure("Top", &style.as_measure()).height;
    let top = text_command(&app, panel, "Top")?;
    let bottom = text_command(&app, panel, "Bottom")?;
    let two = text_command(&app, panel, "Two")?;
    let last = text_command(&app, panel, "Lines")?;
    assert!(top.y.abs() < 0.001);
    assert!((bottom.y + line - 80.0).abs() < 0.001);
    assert!((two.y - f32::mul_add(2.0f32.mul_add(-line, 40.0), 0.5, 80.0)).abs() < 0.001);
    assert!((last.y - two.y - line).abs() < 0.001);
    Ok(())
}

fn no_wrap_hard_break_tree(separator: &str, color: Color) -> LayoutTree {
    let style = TextStyle::new(Pt(11.0)).with_color(color);
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(240.0), Pt(120.0)));
    builder.text(
        Text::new(format!("H{separator}H"), style.clone())
            .wrap(TextWrap::None)
            .layout(El::new().size(Pt(100.0), Pt(60.0)).align_y(AlignY::Center)),
    );
    builder.with(
        El::row().size(Pt(100.0), Pt(60.0)).align_y(AlignY::Center),
        |builder| {
            builder.text(Text::new(format!("I{separator}I"), style).wrap(TextWrap::None));
        },
    );
    builder.build()
}

fn check_no_wrap_hard_break(separator: &str) -> TestResult {
    let original = no_wrap_hard_break_tree(separator, Color::WHITE);
    let recolored = no_wrap_hard_break_tree(separator, Color::BLACK);
    assert_eq!(
        original.classify_change(&recolored),
        LayoutTreeChange::VisualOnly
    );
    let (mut app, panel) = solve(original)?;
    let style = TextStyle::new(Pt(11.0));
    let labels = [
        (format!("H{separator}H"), 0.0),
        (format!("I{separator}I"), 60.0),
    ];
    for (label, row_top) in &labels {
        let dimensions =
            (app.world().resource::<DiegeticTextMeasurer>().measure_fn)(label, &style.as_measure());
        assert!(
            dimensions.height > 1.5 * dimensions.line_height,
            "{label:?} must shape into multiple lines"
        );
        assert!(dimensions.height < 60.0, "{label:?} must fit in its row");
        let expected = f32::mul_add(60.0 - dimensions.height, 0.5, *row_top);
        let actual = text_command(&app, panel, label)?;
        assert!(
            (actual.y - expected).abs() < 0.001,
            "{label:?}: no-wrap multiline block starts at {}, expected {expected}",
            actual.y
        );
    }
    app.world_mut()
        .run_system_once(move |mut commands: Commands| commands.set_tree(panel, recolored.clone()))
        .map_err(|error| format!("cannot run tree replacement: {error}"))?
        .map_err(|error| format!("cannot replace tree: {error}"))?;
    app.update();
    for (label, row_top) in &labels {
        let dimensions =
            (app.world().resource::<DiegeticTextMeasurer>().measure_fn)(label, &style.as_measure());
        let expected = f32::mul_add(60.0 - dimensions.height, 0.5, *row_top);
        assert!(
            (text_command(&app, panel, label)?.y - expected).abs() < 0.001,
            "{label:?}: visual refresh must keep multiline placement"
        );
    }
    Ok(())
}

#[test]
fn control_cap_height_preserves_unicode_line_separator_without_wrapping() -> TestResult {
    check_no_wrap_hard_break("\u{2028}")
}

#[test]
fn control_cap_height_preserves_unicode_paragraph_separator_without_wrapping() -> TestResult {
    check_no_wrap_hard_break("\u{2029}")
}

#[test]
fn control_cap_height_preserves_ascii_hard_breaks_without_wrapping() -> TestResult {
    for separator in ["\n", "\r\n"] {
        check_no_wrap_hard_break(separator)?;
    }
    Ok(())
}

#[test]
fn control_cap_height_does_not_double_trim_palette_labels() -> TestResult {
    let style = TextStyle::new(Pt(11.0));
    let fonts = FontRegistry::new()?;
    let trims = fonts
        .font(FontId::MONOSPACE)?
        .metrics(style.size())
        .shaped_line_cap_box_trims();
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(240.0), Pt(120.0)));
    builder.with(
        El::row().size(Pt(100.0), Pt(30.0)).align_y(AlignY::Center),
        |builder| {
            builder.text(
                Text::new("Open", style.clone()).trim_text_box(Pt(trims.top()), Pt(trims.bottom())),
            );
        },
    );
    let (app, panel) = solve(builder.build())?;
    let cap = flat_cap(&app, &style)?;
    let open = text_command(&app, panel, "Open")?;
    assert!((open.y + f32::midpoint(cap.top, cap.bottom) - 15.0).abs() < 0.001);
    Ok(())
}

fn centred_plain_and_rich_tree(color: Color) -> LayoutTree {
    let style = TextStyle::new(Pt(11.0)).with_color(color);
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(240.0), Pt(120.0)));
    builder.with(
        El::row().size(Pt(100.0), Pt(30.0)).align_y(AlignY::Center),
        |builder| {
            builder.text(Text::new("Open", style.clone()));
        },
    );
    builder.with(
        El::row().size(Pt(100.0), Pt(30.0)).align_y(AlignY::Center),
        |builder| {
            builder.rich_text(RichText::new(style).wrap(TextWrap::None), |rich| {
                rich.span("Jump");
            });
        },
    );
    builder.build()
}

#[test]
fn control_cap_height_survives_visual_refresh_for_plain_and_rich_labels() -> TestResult {
    let original = centred_plain_and_rich_tree(Color::WHITE);
    let recolored = centred_plain_and_rich_tree(Color::BLACK);
    assert_eq!(
        original.classify_change(&recolored),
        LayoutTreeChange::VisualOnly
    );
    let (mut app, panel) = solve(original)?;
    let cap = flat_cap(&app, &TextStyle::new(Pt(11.0)))?;
    for (label, centre) in [("Open", 15.0), ("Jump", 45.0)] {
        let before = text_command(&app, panel, label)?;
        assert!((before.y + f32::midpoint(cap.top, cap.bottom) - centre).abs() < 0.001);
    }
    app.world_mut()
        .run_system_once(move |mut commands: Commands| commands.set_tree(panel, recolored.clone()))
        .map_err(|error| format!("cannot run tree replacement: {error}"))?
        .map_err(|error| format!("cannot replace tree: {error}"))?;
    app.update();
    for (label, centre) in [("Open", 15.0), ("Jump", 45.0)] {
        let after = text_command(&app, panel, label)?;
        assert!((after.y + f32::midpoint(cap.top, cap.bottom) - centre).abs() < 0.001);
    }
    Ok(())
}
