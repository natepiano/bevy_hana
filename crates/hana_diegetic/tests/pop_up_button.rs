//! External-client coverage for the shared pop-up chooser presentation.

use std::f32::consts::FRAC_PI_8;

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::ecs::system::RunSystemOnce;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::App;
use bevy::prelude::Camera;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Handle;
use bevy::prelude::Image;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::Msaa;
use bevy::prelude::Transform;
use bevy::prelude::UVec2;
use bevy::prelude::Vec3;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Appearance;
use hana_diegetic::Button;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ControlMetrics;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::FontRegistry;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::Sizing;
use hana_diegetic::TextInkMeasurement;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_diegetic::WidgetDisabledAppearance;
use hana_diegetic::WidgetFocusedAppearance;
use hana_diegetic::WidgetHoveredAppearance;
use hana_diegetic::WidgetPressedAppearance;
use hana_kana::Cascade;
use hana_kana::ToU16;

mod drawn_support;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

/// Captured live-window dimensions.
const IMAGE_WIDTH: u16 = 1280;
const IMAGE_HEIGHT: u16 = 720;
/// Hana's authored face-label size in millimetres.
const HANA_FACE_LABEL_SIZE: f32 = 5.5;
/// The pop-up's left edge, on a whole physical pixel for the width proof.
const POP_UP_LEFT_PIXELS: f32 = 16.0;
/// The live Type pop-up's outer top edge at close framing.
const POP_UP_TOP_PIXELS: f32 = 316.0;
/// The widest choice in the Shape tool's Type pop-up.
const WIDEST_CHOICE: &str = "Rectangle";
/// Hana's Shape value at close framing, in physical pixels per em.
const DEFAULT_FRAMING_LABEL_SIZE: f32 = 20.1;
/// A larger label that makes alignment differences clear despite pixel rounding.
const LARGE_LABEL_SIZE: f32 = 26.0;
/// Least channel difference that counts as a different colour: above fill dithering, below ink.
const COLOUR_THRESHOLD: u8 = 12;

#[test]
fn pop_up_uses_control_metrics_and_distinct_button_states() {
    let metrics = ControlMetrics::from_label_size(13.0_f32);
    let mut builder = LayoutBuilder::new(80.0, 30.0);
    builder.text(Button::new().pop_up("choice", "Opaque", "Opaque", &metrics));
    let tree = builder.build();
    assert!(
        (tree.element_corner_radius(1).top_left.value - metrics.button.corner_radius).abs() < 1e-4,
    );

    let panel = DiegeticPanel::world()
        .size(Mm(80.0), Mm(30.0))
        .with_tree(tree)
        .build();
    assert!(panel.is_ok());
    let Ok(panel) = panel else { return };

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(DiegeticTextMeasurer::default())
        .add_plugins(HeadlessDiegeticUiPlugin);
    let panel = app.world_mut().spawn(panel).id();
    app.update();

    let computed = app.world().get::<ComputedDiegeticPanel>(panel);
    assert!(computed.is_some());
    let Some(computed) = computed else { return };
    let bounds = computed.content_bounds();
    assert!(bounds.is_some());
    let Some(bounds) = bounds else { return };
    assert!(
        metrics
            .button
            .height
            .mul_add(-Unit::Millimeters.to_points(), bounds.height)
            .abs()
            < 1e-4,
    );

    let widget = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named("choice"))
        });
    assert!(widget.is_ok());
    let Ok(Some(widget)) = widget else { return };
    let hovered = app.world().get::<Cascade<WidgetHoveredAppearance>>(widget);
    let pressed = app.world().get::<Cascade<WidgetPressedAppearance>>(widget);
    let focused = app.world().get::<Cascade<WidgetFocusedAppearance>>(widget);
    let disabled = app.world().get::<Cascade<WidgetDisabledAppearance>>(widget);

    assert_eq!(
        hovered.and_then(Cascade::as_override),
        Some(&WidgetHoveredAppearance::new(
            Appearance::new()
                .background(Color::srgb(0.27, 0.30, 0.35))
                .border_color(Color::srgb(0.75, 0.78, 0.83)),
        )),
    );
    assert_eq!(
        pressed.and_then(Cascade::as_override),
        Some(&WidgetPressedAppearance::new(
            Appearance::new()
                .background(Color::srgb(0.16, 0.18, 0.21))
                .border_color(Color::srgb(0.60, 0.62, 0.66)),
        )),
    );
    assert!(focused.and_then(Cascade::as_override).is_some());
    assert_eq!(
        disabled.and_then(Cascade::as_override),
        Some(&WidgetDisabledAppearance::new(
            Appearance::new()
                .background(Color::srgb(0.11, 0.12, 0.14))
                .border_color(Color::srgb(0.26, 0.28, 0.31))
                .text_color(Color::srgb(0.52, 0.54, 0.57))
                .path_color(Color::srgb(0.52, 0.54, 0.57)),
        )),
    );
}

fn pixels_per_millimetre(label_pixels: f32) -> f32 { label_pixels / HANA_FACE_LABEL_SIZE }

fn fixture_panel_size(label_pixels: f32) -> (f32, f32) {
    let pixels_per_millimetre = pixels_per_millimetre(label_pixels);
    (
        f32::from(IMAGE_WIDTH) / pixels_per_millimetre,
        f32::from(IMAGE_HEIGHT) / pixels_per_millimetre,
    )
}

fn spawn_pop_up(app: &mut App, label_pixels: f32, value: &str) -> Result<(), String> {
    let metrics = ControlMetrics::from_label_size(HANA_FACE_LABEL_SIZE);
    let pixels_per_millimetre = pixels_per_millimetre(label_pixels);
    let (width, height) = fixture_panel_size(label_pixels);
    let mut layout = LayoutBuilder::new(width, height);
    layout.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::new(
                POP_UP_LEFT_PIXELS / pixels_per_millimetre,
                POP_UP_LEFT_PIXELS / pixels_per_millimetre,
                POP_UP_TOP_PIXELS / pixels_per_millimetre,
                POP_UP_LEFT_PIXELS / pixels_per_millimetre,
            ))
            .alignment(AlignX::Left, AlignY::Top),
        |builder| {
            builder.text(Button::new().pop_up("shape", value, WIDEST_CHOICE, &metrics));
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(width), Mm(height))
        .font_unit(Unit::Millimeters)
        .hdr_text_coverage_bias(2.0)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid pop-up panel: {error}"))?;
    let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    Ok(())
}

fn spawn_hana_camera(
    app: &mut App,
    target: Handle<Image>,
    region: CaptureRegion,
    panel_height: f32,
) {
    let bloom = Bloom {
        prefilter: BloomPrefilter {
            threshold:          1.0,
            threshold_softness: 0.1,
        },
        ..Bloom::default()
    };
    app.world_mut().spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            ..Camera::default()
        },
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom.clone(),
    ));
    let mut image_camera = Camera::default();
    region.apply_to_camera(&mut image_camera);
    // The default 45° view spans the fixture panel's height at this distance.
    let distance = panel_height * 0.001 * 0.5 / FRAC_PI_8.tan();
    app.world_mut().spawn((
        Camera3d::default(),
        image_camera,
        RenderTarget::Image(target.into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, distance).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// First and last image rows of each drawn part of the pop-up.
struct DrawnPopUp {
    /// Columns the button paints, border included.
    columns:       (u16, u16),
    /// Rows the button paints, border included.
    button:        (u16, u16),
    /// Rows inside the border, where the button's plain column shows only fill.
    inside_border: (u16, u16),
    /// Rows with value ink.
    ink:           (u16, u16),
    /// Last ink row of the first capital, which stands on the baseline.
    baseline:      u16,
}

fn rgb(pixels: &[u8], x: u16, y: u16) -> Result<[u8; 3], String> {
    let offset = (usize::from(y) * usize::from(IMAGE_WIDTH) + usize::from(x)) * 4;
    pixels
        .get(offset..offset + 3)
        .and_then(|rgb| rgb.try_into().ok())
        .ok_or_else(|| format!("screenshot has no pixel at {x},{y}"))
}

fn differs(first: [u8; 3], second: [u8; 3]) -> bool {
    first
        .iter()
        .zip(second)
        .any(|(first, second)| first.abs_diff(second) > COLOUR_THRESHOLD)
}

fn row_span(mut rows: impl Iterator<Item = u16>) -> Option<(u16, u16)> {
    let first = rows.next()?;
    Some(rows.fold((first, first), |(top, bottom), row| {
        (top.min(row), bottom.max(row))
    }))
}

fn drawn_pop_up(
    pixels: &[u8],
    metrics: &ControlMetrics<f32>,
    value: &str,
) -> Result<DrawnPopUp, String> {
    let left = POP_UP_LEFT_PIXELS;
    let text_start = left + metrics.button.side_padding;
    let centre_row = metrics
        .button
        .height
        .mul_add(0.5, POP_UP_TOP_PIXELS)
        .round()
        .to_u16();
    let mut painted_columns = Vec::new();
    for x in 0..IMAGE_WIDTH {
        if differs(rgb(pixels, x, centre_row)?, [0; 3]) {
            painted_columns.push(x);
        }
    }
    let columns = row_span(painted_columns.into_iter()).ok_or("pop-up button was not drawn")?;
    let chevron_start = f32::from(columns.1 + 1)
        - metrics.pop_up.chevron_left_inset
        - metrics.pop_up.chevron_stroke;
    // Past the rounded corner and before the value, a column shows only border and fill.
    let plain_column = (0..IMAGE_WIDTH)
        .find(|&x| {
            f32::from(x) >= left + metrics.button.corner_radius && f32::from(x) + 1.0 <= text_start
        })
        .ok_or("no plain column between the corner and the value")?;
    let text_columns: Vec<u16> = (0..IMAGE_WIDTH)
        .filter(|&x| f32::from(x) + 1.0 > text_start && f32::from(x) + 1.0 <= chevron_start)
        .collect();
    let registry = FontRegistry::new().map_err(|error| error.to_string())?;
    let first_letter = value
        .chars()
        .next()
        .ok_or("pop-up value is empty")?
        .to_string();
    let first_cap = registry
        .measure_ink(
            &first_letter,
            &TextStyle::new(metrics.label_size).as_measure(),
        )
        .map_err(|error| error.to_string())?;
    let TextInkMeasurement::Visible(first_cap) = first_cap else {
        return Err("the first capital has no measured ink".to_string());
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a glyph's ink width is a few positive pixels"
    )]
    let first_cap_width = (first_cap.right - first_cap.left).ceil() as u16;
    let plain: Vec<[u8; 3]> = (0..IMAGE_HEIGHT)
        .map(|y| rgb(pixels, plain_column, y))
        .collect::<Result<_, _>>()?;
    let button = row_span((0..IMAGE_HEIGHT).filter(|&y| differs(plain[usize::from(y)], [0; 3])))
        .ok_or("pop-up button was not drawn")?;
    let fill = plain[usize::from(button.0.midpoint(button.1))];
    let inside_border =
        row_span((button.0..=button.1).filter(|&y| !differs(plain[usize::from(y)], fill)))
            .ok_or("pop-up fill was not drawn")?;
    let mut ink_rows = Vec::new();
    let mut first_cap_rows = Vec::new();
    let mut first_ink_column = None;
    for &x in &text_columns {
        let mut column_rows = Vec::new();
        for y in 0..IMAGE_HEIGHT {
            if differs(rgb(pixels, x, y)?, plain[usize::from(y)]) {
                column_rows.push(y);
            }
        }
        if !column_rows.is_empty() {
            first_ink_column.get_or_insert(x);
        }
        if first_ink_column.is_some_and(|start| x <= start + first_cap_width) {
            first_cap_rows.extend(&column_rows);
        }
        ink_rows.extend(column_rows);
    }
    let ink = row_span(ink_rows.into_iter()).ok_or("pop-up value text was not drawn")?;
    let baseline = row_span(first_cap_rows.into_iter())
        .ok_or("pop-up first capital was not drawn")?
        .1;
    Ok(DrawnPopUp {
        columns,
        button,
        inside_border,
        ink,
        baseline,
    })
}

fn check_value_cap_and_baseline_centred(label_size: f32, value: &str) -> Result<(), String> {
    let metrics = ControlMetrics::from_label_size(label_size);
    let mut app = CaseApp::new();
    let size = UVec2::new(u32::from(IMAGE_WIDTH), u32::from(IMAGE_HEIGHT));
    let region = CaptureRegion::new(size, UVec2::ZERO, size);
    let target = app.target(region);
    let (_, panel_height) = fixture_panel_size(label_size);
    spawn_hana_camera(&mut app, target.clone(), region, panel_height);
    spawn_pop_up(&mut app, label_size, value)?;
    app.settle()?;
    let pixels = app.capture(target)?;
    if let Some(root) = std::env::var_os("HANA_CAP_SHOTS") {
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        image::save_buffer(
            std::path::Path::new(&root).join(format!("chooser_{value}_{label_size}.png")),
            &pixels,
            u32::from(IMAGE_WIDTH),
            u32::from(IMAGE_HEIGHT),
            image::ColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;
    }
    dump_pixels::<true>(
        "pop_up_button",
        "",
        &format!("value={value} px_per_em={label_size:.9}"),
        region,
        region,
        &pixels,
    )?;
    let drawn = drawn_pop_up(&pixels, &metrics, value)?;
    let button_height = drawn.button.1 - drawn.button.0 + 1;
    let gap_above = i32::from(drawn.ink.0) - i32::from(drawn.inside_border.0);
    let gap_below = i32::from(drawn.inside_border.1) - i32::from(drawn.baseline);
    let measured = format!(
        "{value} at label size {label_size} px: button rows {}..={} ({button_height} px), inside border {}..={}, ink {}..={}, baseline {}, cap-top gap {gap_above}, baseline gap {gap_below}",
        drawn.button.0,
        drawn.button.1,
        drawn.inside_border.0,
        drawn.inside_border.1,
        drawn.ink.0,
        drawn.ink.1,
        drawn.baseline,
    );
    eprintln!("{measured}");
    // Each edge's soft row outside the box counts as button.
    assert!(
        (f32::from(button_height) - metrics.button.height).abs() <= 2.0,
        "{measured}; expected a {} px button",
        metrics.button.height
    );
    assert!(
        gap_above >= 0 && gap_below >= 0 && drawn.ink.1 <= drawn.inside_border.1,
        "ink reaches the border: {measured}"
    );
    if value == "Polygon" {
        assert!(
            drawn.ink.1 > drawn.baseline,
            "descender was not drawn: {measured}"
        );
    }
    assert!(
        (gap_above - gap_below).abs() <= 1,
        "cap and baseline are not centred: {measured}"
    );
    Ok(())
}

#[test]
fn pop_up_box_fits_the_widest_choice_plus_its_own_chrome() -> Result<(), String> {
    let metrics = ControlMetrics::from_label_size(DEFAULT_FRAMING_LABEL_SIZE);
    let mut app = CaseApp::new();
    let size = UVec2::new(u32::from(IMAGE_WIDTH), u32::from(IMAGE_HEIGHT));
    let region = CaptureRegion::new(size, UVec2::ZERO, size);
    let target = app.target(region);
    let (_, panel_height) = fixture_panel_size(DEFAULT_FRAMING_LABEL_SIZE);
    spawn_hana_camera(&mut app, target.clone(), region, panel_height);
    spawn_pop_up(&mut app, DEFAULT_FRAMING_LABEL_SIZE, "Line")?;
    app.settle()?;
    let expected = {
        let measurer = app.world().resource::<DiegeticTextMeasurer>();
        let text_and_padding = (measurer.measure_fn)(
            WIDEST_CHOICE,
            &TextStyle::new(metrics.label_size).as_measure(),
        )
        .width
            + metrics.button.side_padding
            + metrics.pop_up.text_trailing_padding;
        2.0_f32.mul_add(metrics.border_width, text_and_padding)
    };
    let pixels = app.capture(target)?;
    let drawn = drawn_pop_up(&pixels, &metrics, "Line")?;
    let centre_row = metrics
        .button
        .height
        .mul_add(0.5, POP_UP_TOP_PIXELS)
        .round()
        .to_u16();
    let brightest_column = |columns: std::ops::RangeInclusive<u16>| -> Result<u16, String> {
        columns
            .map(|x| rgb(&pixels, x, centre_row).map(|rgb| (x, rgb)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max_by(|(_, left), (_, right)| {
                left.iter()
                    .map(|&channel| u16::from(channel))
                    .sum::<u16>()
                    .cmp(&right.iter().map(|&channel| u16::from(channel)).sum::<u16>())
            })
            .map(|(x, _)| x)
            .ok_or_else(|| "pop-up edge profile is empty".to_string())
    };
    let left_edge = brightest_column(drawn.columns.0..=drawn.columns.0 + 5)?;
    let right_edge = brightest_column(drawn.columns.1 - 5..=drawn.columns.1)?;
    let width = f32::from(right_edge - left_edge + 1);
    eprintln!(
        "pop-up painted columns {}..={}, full border columns {left_edge}..={right_edge}, width {width}, widest-choice box {expected:.3}",
        drawn.columns.0, drawn.columns.1,
    );
    assert!(
        (width - expected).abs() <= 1.0,
        "drawn pop-up width {width} differs from the widest choice plus chrome {expected:.3}"
    );
    Ok(())
}

#[test]
fn pop_up_value_cap_and_baseline_have_even_inner_gaps() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "default_framing_rectangle",
            run:  || check_value_cap_and_baseline_centred(DEFAULT_FRAMING_LABEL_SIZE, "Rectangle"),
        },
        DrawnCase {
            name: "default_framing_polygon",
            run:  || check_value_cap_and_baseline_centred(DEFAULT_FRAMING_LABEL_SIZE, "Polygon"),
        },
        DrawnCase {
            name: "large_rectangle",
            run:  || check_value_cap_and_baseline_centred(LARGE_LABEL_SIZE, "Rectangle"),
        },
        DrawnCase {
            name: "large_polygon",
            run:  || check_value_cap_and_baseline_centred(LARGE_LABEL_SIZE, "Polygon"),
        },
    ])
}
