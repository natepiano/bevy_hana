//! A world tooltip must draw its words, not only lay out a text command.

use std::collections::HashMap;
use std::time::Duration;

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::RunSystemOnce;
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::render::view::DebandDither;
use bevy::render::view::Tonemapping;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::ClearWidgetFocus;
use hana_diegetic::Column;
use hana_diegetic::ControlPalette;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelPlane;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::Tooltip;
use hana_diegetic::TooltipCardOccupiedRegions;
use hana_diegetic::Tooltips;
use hana_diegetic::Unit;
use hana_kana::Position;
use hana_valence::AnchorFrame;
use hana_valence::AnchorSite;
use hana_valence::ResolvedAnchorGeometry;

mod drawn_support;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

const WIDTH: u32 = 512;
const HEIGHT: u32 = 192;
const CAPTURE_WIDTH: u32 = 448;
const CAPTURE_HEIGHT: u32 = 132;
const WORDS: &str = "Tooltip words";
/// World height the panel captures' orthographic view spans.
const PANEL_VIEW_HEIGHT: f32 = 0.105;
/// Keeps legacy text-only captures outside the 0.9999 parallel-frame path; their image camera
/// independently faces the tooltip plane and measures its ink at a fixed scale.
const NONPARALLEL_PRESENTATION_ANGLE: f32 = 0.02;

/// Trims the right and bottom space the text-area checks never read. The
/// top-left origin keeps the full view's screen-space dither phase.
const fn text_area_region() -> CaptureRegion {
    CaptureRegion::new(
        UVec2::new(WIDTH, HEIGHT),
        UVec2::ZERO,
        UVec2::new(CAPTURE_WIDTH, CAPTURE_HEIGHT),
    )
}

/// The whole view, for checks that measure the drawn panel box.
const fn full_region() -> CaptureRegion {
    CaptureRegion::new(
        UVec2::new(WIDTH, HEIGHT),
        UVec2::ZERO,
        UVec2::new(WIDTH, HEIGHT),
    )
}

#[derive(Clone, Copy, Debug)]
enum DrawnPanelCase {
    PlainWords,
    PlainCapLabel,
    TooltipWords,
    FittedCapHelp,
    FittedSmallHelp,
    FittedLargeHelp,
    FittedShadowHelp,
    StackedOutsideHelp,
    StackedOutsideHelpScaled,
    TitleRowOutsideHelp,
    FittedDescenderHelp,
    UndersizedCapHelp,
    EmptyTooltip,
}

#[derive(Clone, Copy)]
enum SceneLighting {
    Unlit,
    Bright,
    Dark,
}

#[derive(Clone, Copy)]
enum CameraZoom {
    Initial,
    AfterMaterialization,
    DuringReveal,
}

#[derive(Clone, Copy)]
enum ControlAnchorDepth {
    PanelPlane,
    InFront,
}

#[derive(Clone, Copy)]
enum HostOrientation {
    ViewAligned,
    Tilted,
}

#[derive(Clone, Copy)]
enum SourceGridAlignment {
    OffGrid,
    Unchecked,
}

fn tooltip(contents: DrawnPanelCase) -> Tooltip {
    let fitted = matches!(
        contents,
        DrawnPanelCase::FittedCapHelp
            | DrawnPanelCase::FittedSmallHelp
            | DrawnPanelCase::FittedLargeHelp
            | DrawnPanelCase::FittedShadowHelp
            | DrawnPanelCase::StackedOutsideHelp
            | DrawnPanelCase::StackedOutsideHelpScaled
            | DrawnPanelCase::TitleRowOutsideHelp
            | DrawnPanelCase::FittedDescenderHelp
            | DrawnPanelCase::UndersizedCapHelp
    );
    let root = if fitted {
        El::column()
    } else {
        El::column().size(0.22, 0.06)
    };
    let scale = if matches!(contents, DrawnPanelCase::StackedOutsideHelpScaled) {
        1.9
    } else {
        1.0
    };
    let mut tooltip = Tooltip::new(root)
        .with_label_size(Mm(
            if matches!(
                contents,
                DrawnPanelCase::StackedOutsideHelp
                    | DrawnPanelCase::StackedOutsideHelpScaled
                    | DrawnPanelCase::TitleRowOutsideHelp
            ) {
                6.0 * scale
            } else {
                18.0
            },
        ))
        .show_after(Duration::ZERO);
    if fitted {
        tooltip = tooltip.font_unit(Unit::Meters);
    }
    match contents {
        DrawnPanelCase::TooltipWords => {
            tooltip.text(WORDS);
        },
        DrawnPanelCase::FittedCapHelp | DrawnPanelCase::FittedShadowHelp => {
            tooltip.text(("HELP", TextStyle::new(0.0153)));
        },
        DrawnPanelCase::FittedSmallHelp => {
            tooltip.text(("Sets width", TextStyle::new(0.006)));
        },
        DrawnPanelCase::FittedLargeHelp => {
            tooltip.text(("HELP", TextStyle::new(0.0165)));
        },
        DrawnPanelCase::StackedOutsideHelp
        | DrawnPanelCase::StackedOutsideHelpScaled
        | DrawnPanelCase::TitleRowOutsideHelp => {
            tooltip.text(("HELP", TextStyle::new(0.0051 * scale)));
        },
        DrawnPanelCase::FittedDescenderHelp => {
            tooltip.text(("H gyp", TextStyle::new(0.0153)));
        },
        DrawnPanelCase::UndersizedCapHelp => {
            tooltip.text(("HELP", TextStyle::new(0.0135)));
        },
        _ => {},
    }
    tooltip
}

fn plain_world_panel() -> Result<DiegeticPanel, String> {
    let palette = ControlPalette::system_dark();
    let mut layout = LayoutBuilder::with_root(
        El::column()
            .size(0.22, 0.06)
            .padding(Padding::all(Mm(10.0)))
            .background(palette.field_fill),
    );
    layout.text(Text::new(
        WORDS,
        TextStyle::new(0.0153).with_color(palette.knob),
    ));
    DiegeticPanel::world()
        .font_unit(Unit::Meters)
        .size(0.22, 0.06)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid plain text control panel: {error}"))
}

fn plain_cap_label() -> Result<DiegeticPanel, String> {
    let palette = ControlPalette::system_dark();
    let mut layout = LayoutBuilder::with_root(
        El::column()
            .size(0.22, 0.06)
            .padding(Padding::all(Mm(10.0)))
            .background(palette.field_fill),
    );
    layout.text(Text::new(
        "HELP",
        TextStyle::new(0.018).with_color(palette.knob),
    ));
    DiegeticPanel::world()
        .font_unit(Unit::Meters)
        .size(0.22, 0.06)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid row label panel: {error}"))
}

fn configure_scene_lighting(app: &mut App, lighting: SceneLighting) {
    app.world_mut().insert_resource(match lighting {
        SceneLighting::Unlit => Lighting::Unlit,
        SceneLighting::Bright | SceneLighting::Dark => Lighting::Lit,
    });
    if matches!(lighting, SceneLighting::Dark) {
        app.insert_resource(GlobalAmbientLight {
            brightness: 0.0,
            ..default()
        });
    }
    if matches!(lighting, SceneLighting::Bright) {
        app.world_mut().spawn((
            DirectionalLight {
                illuminance: 20_000.0,
                ..default()
            },
            Transform::from_rotation(Quat::from_rotation_x(-0.2)),
        ));
    }
}

fn shown_panel_app(
    contents: DrawnPanelCase,
    lighting: SceneLighting,
) -> Result<(CaseApp, Entity), String> {
    let mut app = CaseApp::new();
    configure_scene_lighting(&mut app, lighting);
    shown_panel_in_app(app, contents)
}

fn tooltip_host(app: &mut CaseApp, contents: DrawnPanelCase) -> Result<(Entity, Vec2), String> {
    let stacked = matches!(
        contents,
        DrawnPanelCase::StackedOutsideHelp | DrawnPanelCase::StackedOutsideHelpScaled
    );
    let title_row = matches!(contents, DrawnPanelCase::TitleRowOutsideHelp);
    let card_like = stacked || title_row;
    let card_scale = if matches!(contents, DrawnPanelCase::StackedOutsideHelpScaled) {
        1.9
    } else {
        1.0
    };
    let card_size = if title_row {
        Vec2::new(0.26, 0.03)
    } else {
        Vec2::new(0.07, 0.045)
    };
    let mut layout = if card_like {
        LayoutBuilder::with_root(
            El::column()
                .size(card_size.x, card_size.y)
                .background(Color::srgb_u8(45, 82, 134)),
        )
    } else {
        LayoutBuilder::new(0.08, 0.04)
    };
    if title_row {
        layout.with(
            El::new()
                .size(0.014, 0.01)
                .background(Color::srgb_u8(20, 220, 220))
                .button("control")
                .tooltip(tooltip(contents)),
            |_| {},
        );
        for (id, color) in [
            ("middle", Color::srgb_u8(220, 30, 30)),
            ("bottom", Color::srgb_u8(30, 220, 30)),
        ] {
            layout.with(
                El::new().size(0.014, 0.01).background(color).button(id),
                |_| {},
            );
        }
    } else if stacked {
        layout.with(
            El::new()
                .size(0.014, 0.015)
                .background(Color::srgb_u8(220, 30, 30))
                .button("top"),
            |_| {},
        );
        layout.with(
            El::new()
                .size(0.014, 0.015)
                .background(Color::srgb_u8(20, 220, 220))
                .button("control")
                .tooltip(tooltip(contents)),
            |_| {},
        );
        layout.with(
            El::new()
                .size(0.014, 0.015)
                .background(Color::srgb_u8(30, 220, 30))
                .button("bottom"),
            |_| {},
        );
    } else {
        layout.with(
            El::new()
                .size(0.08, 0.04)
                .button("control")
                .tooltip(tooltip(contents)),
            |_| {},
        );
    }
    let host_size = if card_like {
        card_size
    } else {
        Vec2::new(0.08, 0.04)
    };
    let host = DiegeticPanel::world()
        .size(host_size.x, host_size.y)
        .world_width(host_size.x)
        .anchor(if card_like {
            Anchor::Center
        } else {
            Anchor::TopLeft
        })
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid tooltip host panel: {error}"))?;
    let host = app
        .world_mut()
        .spawn((
            host,
            Transform::from_xyz(if card_like { 0.0 } else { -2.2 }, 0.0, 0.0)
                .with_scale(Vec3::splat(card_scale)),
        ))
        .id();
    Ok((host, host_size))
}

fn shown_panel_in_app(
    mut app: CaseApp,
    contents: DrawnPanelCase,
) -> Result<(CaseApp, Entity), String> {
    let mut window = Window::default();
    window.resolution.set(512.0, 192.0);
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    let projection = if matches!(
        contents,
        DrawnPanelCase::StackedOutsideHelp
            | DrawnPanelCase::StackedOutsideHelpScaled
            | DrawnPanelCase::TitleRowOutsideHelp
    ) {
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: PANEL_VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        })
    } else {
        Projection::Perspective(PerspectiveProjection::default())
    };
    let camera_rotation = if matches!(projection, Projection::Perspective(_)) {
        Quat::from_rotation_y(NONPARALLEL_PRESENTATION_ANGLE)
    } else {
        Quat::IDENTITY
    };
    app.world_mut().spawn((
        Camera3d::default(),
        projection,
        RenderTarget::Window(WindowRef::Entity(window)),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        DebandDither::Disabled,
        Transform::from_xyz(0.0, 0.0, 5.0).with_rotation(camera_rotation),
    ));
    let (host, host_size) = tooltip_host(&mut app, contents)?;
    let card_like = matches!(
        contents,
        DrawnPanelCase::StackedOutsideHelp
            | DrawnPanelCase::StackedOutsideHelpScaled
            | DrawnPanelCase::TitleRowOutsideHelp
    );

    app.settle()?;
    let panel_entity = if matches!(
        contents,
        DrawnPanelCase::PlainWords | DrawnPanelCase::PlainCapLabel
    ) {
        let panel = if matches!(contents, DrawnPanelCase::PlainCapLabel) {
            plain_cap_label()?
        } else {
            plain_world_panel()?
        };
        app.world_mut().spawn((panel, Transform::IDENTITY)).id()
    } else {
        let control = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(host, &PanelElementId::named("control"))
            })
            .map_err(|error| format!("widget query failed: {error}"))?
            .ok_or_else(|| "host has no tooltip control".to_string())?;
        if card_like {
            let card = Rect::from_center_size(Vec2::ZERO, host_size);
            app.world_mut()
                .entity_mut(control)
                .insert(TooltipCardOccupiedRegions::new(host, card, vec![card]));
        }
        let tip = app
            .world()
            .get::<Tooltips>(control)
            .and_then(|controllers| controllers.iter().next())
            .ok_or_else(|| "control has no tooltip controller".to_string())?;
        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: control,
        });
        tip
    };
    for _ in 0..30 {
        app.update();
    }
    if app.world().get::<Visibility>(panel_entity) == Some(&Visibility::Hidden) {
        return Err("world panel remained hidden".to_string());
    }
    Ok((app, panel_entity))
}

/// A panel capture and the scale its panel was drawn at.
struct PanelCapture {
    image:         Vec<u8>,
    /// Image pixels per layout pixel of the captured panel.
    pixels_per_px: f32,
}

fn capture_panel(
    contents: DrawnPanelCase,
    lighting: SceneLighting,
    region: CaptureRegion,
) -> Result<Vec<u8>, String> {
    capture_panel_with_help(contents, lighting, region, true).map(|capture| capture.image)
}

fn capture_panel_with_help(
    contents: DrawnPanelCase,
    lighting: SceneLighting,
    region: CaptureRegion,
    show_help: bool,
) -> Result<PanelCapture, String> {
    let (mut app, panel_entity) = shown_panel_app(contents, lighting)?;
    let transform = app
        .world()
        .get::<GlobalTransform>(panel_entity)
        .ok_or_else(|| "visible panel has no world transform".to_string())?;
    let normal = transform.affine().transform_vector3(Vec3::Z).normalize();
    let up = transform.affine().transform_vector3(Vec3::Y).normalize();
    let panel = app
        .world()
        .get::<DiegeticPanel>(panel_entity)
        .ok_or_else(|| "visible world panel has no panel component".to_string())?;
    let center = PanelPlane::from_panel(panel, transform)
        .map_err(|error| format!("visible world panel has no panel plane: {error}"))?
        .point(Anchor::Center);
    let world_per_px = POINTS_PER_PX
        * panel.points_to_world()
        * transform.affine().transform_vector3(Vec3::Y).length();
    let pixels_per_px = world_per_px * region.full_size.as_vec2().y / PANEL_VIEW_HEIGHT;

    if !show_help {
        app.world_mut()
            .entity_mut(panel_entity)
            .insert(Visibility::Hidden);
    }

    if matches!(contents, DrawnPanelCase::FittedShadowHelp) {
        let background = LayoutBuilder::with_root(
            El::new()
                .size(0.22, 0.08)
                .background(Color::srgb_u8(45, 82, 134)),
        );
        let background = DiegeticPanel::world()
            .font_unit(Unit::Meters)
            .size(0.22, 0.08)
            .anchor(Anchor::Center)
            .with_tree(background.build())
            .build()
            .map_err(|error| format!("invalid shadow receiving panel: {error}"))?;
        app.world_mut().spawn((
            background,
            Transform::from_translation(center - normal * 0.004),
        ));
    }

    let target = app.target(region);
    let mut camera = Camera::default();
    region.apply_to_camera(&mut camera);
    app.world_mut().spawn((
        Camera3d::default(),
        camera,
        RenderTarget::Image(target.clone().into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        DebandDither::Disabled,
        Msaa::Off,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: PANEL_VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(center + normal * 0.25).looking_at(center, up),
    ));
    for _ in 0..4 {
        app.update();
    }

    let image = app.capture(target)?;
    if image.len() != (region.size.x * region.size.y * 4) as usize {
        return Err(format!("screenshot has {} bytes", image.len()));
    }
    Ok(PanelCapture {
        image,
        pixels_per_px,
    })
}

fn luminance(color: [u8; 3]) -> f64 {
    let linear = color.map(|channel| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    linear[2].mul_add(0.0722, linear[1].mul_add(0.7152, linear[0] * 0.2126))
}

/// Byte offsets of the text area in a text-area capture.
fn text_area_offsets() -> impl Iterator<Item = usize> + Clone {
    // The fixed 0.22 x 0.06 m root occupies the middle of the 0.28 x
    // 0.105 m camera view. This crop excludes the border and rounded corners.
    (60..132).flat_map(|y| (64..448).map(move |x| ((y * CAPTURE_WIDTH + x) * 4) as usize))
}

/// The most common colour in the text area and how many pixels hold it.
fn text_area_fill(image: &[u8]) -> Result<([u8; 3], usize), String> {
    let mut colors = HashMap::<[u8; 3], usize>::new();
    for offset in text_area_offsets() {
        let color = [image[offset], image[offset + 1], image[offset + 2]];
        *colors.entry(color).or_default() += 1;
    }
    colors
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .ok_or_else(|| "tooltip text area has no pixels".to_string())
}

fn high_contrast_in_text_area(image: &[u8]) -> Result<usize, String> {
    let (fill, fill_pixels) = text_area_fill(image)?;
    if fill == [0, 0, 0] || fill_pixels < 1000 {
        return Err(format!(
            "tooltip box was not drawn in text area: fill={fill:?}, pixels={fill_pixels}"
        ));
    }
    let fill_luminance = luminance(fill);
    let high_contrast = text_area_offsets()
        .filter(|&offset| {
            let color = [image[offset], image[offset + 1], image[offset + 2]];
            let pixel_luminance = luminance(color);
            let ratio = (fill_luminance.max(pixel_luminance) + 0.05)
                / (fill_luminance.min(pixel_luminance) + 0.05);
            ratio >= 4.5
        })
        .count();
    eprintln!(
        "panel fill={fill:?}, fill pixels={fill_pixels}, high-contrast pixels={high_contrast}"
    );
    Ok(high_contrast)
}

fn dump_text_area(case: &str, image: &[u8]) -> Result<(), String> {
    dump_pixels::<true>(
        "tooltip_drawn_words",
        case,
        &format!(
            "px_per_em={:.9}",
            f64::from(HEIGHT) / f64::from(PANEL_VIEW_HEIGHT)
        ),
        text_area_region(),
        CaptureRegion::new(
            UVec2::new(WIDTH, HEIGHT),
            UVec2::new(64, 60),
            UVec2::new(384, 72),
        ),
        image,
    )
}

fn world_tooltip_words_reach_drawn_pixels() -> Result<(), String> {
    let image = capture_panel(
        DrawnPanelCase::TooltipWords,
        SceneLighting::Unlit,
        text_area_region(),
    )?;
    dump_text_area("world_tooltip_words_reach_drawn_pixels", &image)?;
    let count = high_contrast_in_text_area(&image)?;
    assert!(count >= 300, "only {count} readable tooltip-word pixels");
    Ok(())
}

fn empty_world_tooltip_has_no_drawn_word_pixels() -> Result<(), String> {
    let image = capture_panel(
        DrawnPanelCase::EmptyTooltip,
        SceneLighting::Unlit,
        text_area_region(),
    )?;
    dump_text_area("empty_world_tooltip_has_no_drawn_word_pixels", &image)?;
    let count = high_contrast_in_text_area(&image)?;
    assert_eq!(count, 0, "empty tooltip has {count} word-like pixels");
    Ok(())
}

fn plain_world_panel_words_reach_drawn_pixels() -> Result<(), String> {
    let image = capture_panel(
        DrawnPanelCase::PlainWords,
        SceneLighting::Unlit,
        text_area_region(),
    )?;
    dump_text_area("plain_world_panel_words_reach_drawn_pixels", &image)?;
    let count = high_contrast_in_text_area(&image)?;
    assert!(
        count >= 300,
        "only {count} readable plain-panel word pixels"
    );
    Ok(())
}

fn repeated_world_panel_words_upload_cold_glyph_atlas() -> Result<(), String> {
    let app = CaseApp::new();
    let before = app.world().resource::<Assets<ShaderBuffer>>().len();
    let (app, _) = shown_panel_in_app(app, DrawnPanelCase::PlainWords)?;
    let after = app.world().resource::<Assets<ShaderBuffer>>().len();
    assert!(
        after >= before + 6,
        "repeated panel words did not upload a fresh three-buffer glyph atlas: {before} -> {after}"
    );
    Ok(())
}

#[derive(Debug)]
struct PixelBounds {
    left:   u32,
    right:  u32,
    top:    u32,
    bottom: u32,
}

impl PixelBounds {
    const fn width(&self) -> u32 { self.right - self.left + 1 }
    const fn height(&self) -> u32 { self.bottom - self.top + 1 }
}

/// Bounds of the pixels that match `pred` in a full-size capture.
fn pixel_bounds(image: &[u8], pred: impl Fn(u32, u32, [u8; 3]) -> bool) -> Option<PixelBounds> {
    let mut bounds = None::<PixelBounds>;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let offset = ((y * WIDTH + x) * 4) as usize;
            let rgb = [image[offset], image[offset + 1], image[offset + 2]];
            if pred(x, y, rgb) {
                let box_ = bounds.get_or_insert(PixelBounds {
                    left:   x,
                    right:  x,
                    top:    y,
                    bottom: y,
                });
                box_.left = box_.left.min(x);
                box_.right = box_.right.max(x);
                box_.top = box_.top.min(y);
                box_.bottom = box_.bottom.max(y);
            }
        }
    }
    bounds
}

fn solid_panel_bounds(image: &[u8]) -> Result<PixelBounds, String> {
    solid_fill(image).map(|(_, bounds)| bounds)
}

/// The commonest drawn colour and the bounds of its pixels.
fn solid_fill(image: &[u8]) -> Result<([u8; 3], PixelBounds), String> {
    let mut colors = HashMap::<[u8; 3], usize>::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let offset = ((y * WIDTH + x) * 4) as usize;
            let rgb = [image[offset], image[offset + 1], image[offset + 2]];
            if rgb != [0, 0, 0] {
                *colors.entry(rgb).or_default() += 1;
            }
        }
    }
    let (fill, count) = colors
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .ok_or_else(|| "tooltip drew no solid fill".to_string())?;
    if count < 30 {
        return Err(format!("tooltip has only {count} solid-fill pixels"));
    }
    pixel_bounds(image, |_, _, rgb| rgb == fill)
        .map(|bounds| (fill, bounds))
        .ok_or_else(|| "tooltip drew no solid panel pixels".to_string())
}

/// A solid-ink pixel is at least halfway from the panel fill to the brightest
/// glyph ink in linear luminance, in a row and column with at least four such
/// pixels. Sparse anti-aliased edge rows and columns do not set the bounds.
fn cap_bounds(image: &[u8]) -> Result<PixelBounds, String> {
    const MIN_SOLID_INK_SUPPORT: u32 = 4;
    let box_ = pixel_bounds(image, |_, _, rgb| rgb != [0, 0, 0])
        .ok_or_else(|| "no drawn panel pixels".to_string())?;
    let border_exclusion = 8.min(box_.height() / 5);
    let mut colors = HashMap::<[u8; 3], usize>::new();
    for y in (box_.top + border_exclusion)..=(box_.bottom - border_exclusion) {
        for x in (box_.left + border_exclusion)..=(box_.right - border_exclusion) {
            let offset = ((y * WIDTH + x) * 4) as usize;
            // Software rendering dithers the dark fill across neighbouring byte values.
            // Count those shades together so an exact white glyph byte cannot become the mode.
            *colors
                .entry([
                    image[offset] >> 3,
                    image[offset + 1] >> 3,
                    image[offset + 2] >> 3,
                ])
                .or_default() += 1;
        }
    }
    let fill_bucket = colors
        .iter()
        .max_by_key(|(_, count)| *count)
        .ok_or_else(|| "panel interior has no fill".to_string())?
        .0;
    let fill = fill_bucket.map(|channel| channel * 8 + 4);
    let fill_luminance = luminance(fill);
    let glyph_luminance = colors
        .keys()
        .map(|color| luminance(color.map(|channel| channel * 8 + 4)))
        .fold(fill_luminance, f64::max);
    if (glyph_luminance + 0.05) / (fill_luminance + 0.05) < 4.5 {
        return Err(format!(
            "panel has no readable cap ink: box {box_:?}, fill {fill:?} ({fill_luminance:.3}), brightest {glyph_luminance:.3}"
        ));
    }
    let solid_ink_luminance = f64::midpoint(fill_luminance, glyph_luminance);
    let mut row_support = [0u32; HEIGHT as usize];
    let mut column_support = [0u32; WIDTH as usize];
    for y in (box_.top + border_exclusion + 1)..(box_.bottom - border_exclusion) {
        for x in (box_.left + border_exclusion + 1)..(box_.right - border_exclusion) {
            let offset = ((y * WIDTH + x) * 4) as usize;
            let rgb = [image[offset], image[offset + 1], image[offset + 2]];
            if luminance(rgb) >= solid_ink_luminance {
                row_support[y as usize] += 1;
                column_support[x as usize] += 1;
            }
        }
    }
    pixel_bounds(image, |x, y, rgb| {
        if x <= box_.left + border_exclusion
            || x >= box_.right - border_exclusion
            || y <= box_.top + border_exclusion
            || y >= box_.bottom - border_exclusion
        {
            return false;
        }
        row_support[y as usize] >= MIN_SOLID_INK_SUPPORT
            && column_support[x as usize] >= MIN_SOLID_INK_SUPPORT
            && luminance(rgb) >= solid_ink_luminance
    })
    .ok_or_else(|| format!(
        "no readable cap pixels: box {box_:?}, fill {fill:?}, row support {}, column support {}",
        row_support.into_iter().max().unwrap_or(0),
        column_support.into_iter().max().unwrap_or(0),
    ))
}

fn help_cap_height_matches_row(cap: &PixelBounds, row_cap: &PixelBounds) -> Result<(), String> {
    let row_height = u64::from(row_cap.height());
    let cap_height = u64::from(cap.height());
    let minimum = (row_height * 85).div_ceil(100).saturating_sub(1);
    let maximum = (row_height * 90).div_ceil(100) + 1;
    if !(minimum..=maximum).contains(&cap_height) {
        return Err(format!(
            "help cap is {cap_height} px against row's {row_height} px after pixel rounding; help {cap:?}, row {row_cap:?}"
        ));
    }
    Ok(())
}

fn fitted_world_help_has_even_drawn_padding_and_a_readable_cap() -> Result<(), String> {
    let help = capture_panel(
        DrawnPanelCase::FittedCapHelp,
        SceneLighting::Unlit,
        full_region(),
    )?;
    let row = capture_panel(
        DrawnPanelCase::PlainCapLabel,
        SceneLighting::Unlit,
        full_region(),
    )?;
    let box_ = solid_panel_bounds(&help)?;
    let cap = cap_bounds(&help)?;
    let row_cap = cap_bounds(&row)?;
    let padding = [
        cap.left - box_.left,
        box_.right - cap.right,
        cap.top - box_.top,
        box_.bottom - cap.bottom,
    ];
    help_cap_height_matches_row(&cap, &row_cap)?;
    assert!(
        cap.height() >= 9,
        "help cap draws only {} pixels",
        cap.height()
    );
    assert!(
        padding.iter().all(|&side| side >= 1),
        "help padding positive control found no clear space: {padding:?}"
    );
    assert!(
        padding[0].abs_diff(padding[1]) <= 1 && padding[2].abs_diff(padding[3]) <= 1,
        "uneven tooltip padding {padding:?}, box {box_:?}, cap {cap:?}"
    );
    assert!(box_.width() > cap.width(), "help box clips its words");
    eprintln!(
        "Fitted help: padding L/R/T/B {padding:?} px, cap {} px, row {} px, ratio {:.1}%",
        cap.height(),
        row_cap.height(),
        f64::from(cap.height()) * 100.0 / f64::from(row_cap.height()),
    );
    Ok(())
}

fn fitted_descender_help_has_even_drawn_ink_padding() -> Result<(), String> {
    let help = capture_panel(
        DrawnPanelCase::FittedDescenderHelp,
        SceneLighting::Unlit,
        full_region(),
    )?;
    let empty = capture_panel(
        DrawnPanelCase::EmptyTooltip,
        SceneLighting::Unlit,
        full_region(),
    )?;
    let box_ = solid_panel_bounds(&help)?;
    let ink = cap_bounds(&help)?;
    assert!(
        cap_bounds(&empty).is_err(),
        "empty-tooltip control unexpectedly has measured glyph ink"
    );
    let horizontal = [ink.left - box_.left, box_.right - ink.right];
    let cap_baseline = pixel_bounds(&help, |x, y, rgb| {
        (ink.left..=ink.left + 7).contains(&x)
            && (ink.top..=ink.bottom).contains(&y)
            && rgb.into_iter().all(|channel| channel >= 150)
    })
    .ok_or_else(|| "descender help drew no H cap".to_string())?
    .bottom;
    let vertical = [ink.top - box_.top, box_.bottom - ink.bottom];
    assert!(
        horizontal[0].abs_diff(horizontal[1]) <= 1 && vertical[0].abs_diff(vertical[1]) <= 1,
        "descender help padding horizontal={horizontal:?}, vertical={vertical:?}, box={box_:?}, ink={ink:?}"
    );
    assert!(
        ink.width() >= 10 && ink.height() >= 9,
        "descender help drew too little ink: {ink:?}"
    );
    assert!(
        ink.bottom > cap_baseline,
        "descender control did not draw below the H baseline"
    );
    eprintln!(
        "Descender help: padding horizontal {horizontal:?} px, ink top/bottom {vertical:?} px, descent {} px",
        ink.bottom - cap_baseline,
    );
    Ok(())
}

fn fitted_help_padding_tracks_drawn_cap_at_two_sizes() -> Result<(), String> {
    for contents in [
        DrawnPanelCase::FittedSmallHelp,
        DrawnPanelCase::FittedLargeHelp,
    ] {
        let image = capture_panel(contents, SceneLighting::Unlit, full_region())?;
        let box_ = solid_panel_bounds(&image)?;
        let cap = cap_bounds(&image)?;
        let padding = [
            cap.left - box_.left,
            box_.right - cap.right,
            cap.top - box_.top,
            box_.bottom - cap.bottom,
        ];
        let cap_height = UVec2::splat(cap.height()).as_vec2().x;
        let wanted = Vec2::splat((0.4 * cap_height).ceil()).as_uvec2().x;
        assert!(
            padding
                .iter()
                .all(|side| side.abs_diff(wanted) <= 1 && *side > 0),
            "{contents:?}: drawn padding {padding:?}, cap height {}, wanted {wanted}",
            cap.height()
        );
        eprintln!(
            "{contents:?}: cap {} px, padding L/R/T/B {padding:?} px, wanted {wanted} px",
            cap.height()
        );
        if matches!(contents, DrawnPanelCase::FittedSmallHelp) {
            assert_eq!(
                cap.height(),
                9,
                "small help must exercise the 9 px cap floor, not a roomier size"
            );
        }
    }
    Ok(())
}

const fn color_at(image: &[u8], x: u32, y: u32) -> [u8; 3] {
    let index = ((y * WIDTH + x) * 4) as usize;
    [image[index], image[index + 1], image[index + 2]]
}

fn world_help_softly_darkens_the_surface_below_it() -> Result<(), String> {
    let plain = capture_panel(
        DrawnPanelCase::FittedCapHelp,
        SceneLighting::Unlit,
        full_region(),
    )?;
    let box_ = pixel_bounds(&plain, |_, _, rgb| rgb != [0, 0, 0])
        .ok_or_else(|| "help drew no outer box".to_string())?;
    let with_shadow = capture_panel(
        DrawnPanelCase::FittedShadowHelp,
        SceneLighting::Unlit,
        full_region(),
    )?;
    let x = box_.left.midpoint(box_.right);
    let far_x = box_.right + 35;
    let far = color_at(&with_shadow, far_x, box_.bottom + 2);
    let below: Vec<_> = (1..=6)
        .map(|step| color_at(&with_shadow, x, box_.bottom + step))
        .collect();
    assert!(
        far[2].saturating_sub(below[1][2]) >= 3,
        "no visible help shadow below box {box_:?}: far {far:?}, below {below:?}"
    );
    assert!(
        below
            .windows(2)
            .all(|pair| pair[0][2].abs_diff(pair[1][2]) <= 12),
        "help shadow has a hard edge: {below:?}"
    );
    Ok(())
}

fn stacked_card_help_draws_outside_on_its_jack_row_at_scaled_spacing() -> Result<(), String> {
    let mut painted_gaps = Vec::new();
    for (contents, scale) in [
        (DrawnPanelCase::StackedOutsideHelp, 1.0),
        (DrawnPanelCase::StackedOutsideHelpScaled, 1.9),
    ] {
        let PanelCapture {
            image,
            pixels_per_px,
        } = capture_panel_with_help(contents, SceneLighting::Unlit, full_region(), true)?;
        let card_only =
            capture_panel_with_help(contents, SceneLighting::Unlit, full_region(), false)?.image;
        let card = pixel_bounds(&card_only, |_, _, rgb| rgb != [0, 0, 0])
            .ok_or_else(|| format!("{contents:?}: no drawn card"))?;
        // The host draws three equal rows, with the hovered jack in the middle.
        let own_row_top = card.top + card.height() / 3;
        let own_row_bottom = card.top + card.height() * 2 / 3;
        let own_jack = pixel_bounds(&card_only, |_, y, rgb| {
            (own_row_top..own_row_bottom).contains(&y)
                && rgb[1] > 100
                && rgb[2] > 100
                && u16::from(rgb[1]) > 2 * u16::from(rgb[0])
                && u16::from(rgb[2]) > 2 * u16::from(rgb[0])
        })
        .ok_or_else(|| format!("{contents:?}: no drawn jack row"))?;
        let help = pixel_bounds(&image, |x, _, rgb| {
            (x < card.left || x > card.right) && rgb != [0, 0, 0]
        })
        .ok_or_else(|| format!("{contents:?}: no drawn outside help"))?;
        let cap = pixel_bounds(&image, |x, _, rgb| {
            (x < card.left || x > card.right) && rgb.into_iter().all(|channel| channel >= 150)
        })
        .ok_or_else(|| format!("{contents:?}: no readable outside-help cap"))?;
        let body_gap = if help.right < card.left {
            card.left - help.right - 1
        } else if help.left > card.right {
            help.left - card.right - 1
        } else {
            return Err(format!(
                "{contents:?}: help {help:?} is not wholly beside card {card:?}"
            ));
        };
        let help_row = help.top.midpoint(help.bottom);
        let own_column = own_jack.left.midpoint(own_jack.right);
        let nearest_left = own_column - card.left < card.right - own_column;
        assert!(
            (own_row_top..own_row_bottom).contains(&help_row),
            "{contents:?}: help {help:?} is outside its drawn middle jack row {own_row_top}..{own_row_bottom}"
        );
        assert_eq!(
            help.right < card.left,
            nearest_left,
            "{contents:?}: help {help:?} is on the far side of jack {own_jack:?} in card {card:?}"
        );
        // The help's painted bounds, its body plus the shadow's side blur, stand the gap from
        // the card. The black background hides that blur, so the drawn body stands further off.
        let side_reach = f64::from(SHADOW_BLUR_PX * pixels_per_px);
        let painted_gap = f64::from(body_gap) - side_reach;
        assert!(
            painted_gap > 0.0,
            "{contents:?}: help paint touches the card edge: body gap {body_gap} px, side shadow reach {side_reach:.2} px"
        );
        let cap_height = f64::from(cap.height());
        assert!(
            (painted_gap - cap_height).abs() <= cap_height.mul_add(0.25, 0.5),
            "{contents:?}: help-to-card gap {painted_gap:.2} px is not about its {cap_height} px cap"
        );
        eprintln!(
            "{contents:?} at {scale:.1}x: gap {painted_gap:.2} px, cap {cap_height} px, card {} px",
            card.width()
        );
        painted_gaps.push((painted_gap, card.width(), cap_height, scale));
    }
    let [(_, _, first_cap, _), (_, _, second_cap, _)] = painted_gaps[..] else {
        return Err(format!("expected two painted gaps, found {painted_gaps:?}"));
    };
    let expected_second_cap = (first_cap * painted_gaps[1].3 / painted_gaps[0].3).round();
    assert!(
        (second_cap - expected_second_cap).abs() <= 1.0,
        "outside-help cap does not scale 1.9x with the card: {painted_gaps:?}, expected {expected_second_cap:.2} px"
    );
    Ok(())
}

fn title_row_help_draws_above_when_horizontal_edges_do_not_fit() -> Result<(), String> {
    let contents = DrawnPanelCase::TitleRowOutsideHelp;
    let image = capture_panel_with_help(contents, SceneLighting::Unlit, full_region(), true)?.image;
    let card_only =
        capture_panel_with_help(contents, SceneLighting::Unlit, full_region(), false)?.image;
    let card = pixel_bounds(&card_only, |_, _, rgb| rgb != [0, 0, 0])
        .ok_or_else(|| "title-row fixture drew no card".to_string())?;
    let help = pixel_bounds(&image, |_, y, rgb| {
        (y < card.top || y > card.bottom) && rgb != [0, 0, 0]
    })
    .ok_or_else(|| "title-row fixture drew no outside help".to_string())?;
    let projected_card_width = 0.26 * f64::from(HEIGHT) / f64::from(PANEL_VIEW_HEIGHT);
    let side_room = ((f64::from(WIDTH) - projected_card_width) * 0.5).max(0.0);
    assert!(
        side_room < f64::from(help.width()),
        "title-row fixture leaves {side_room} px beside the card, enough for its {} px help",
        help.width()
    );
    assert!(
        help.bottom < card.top,
        "title-row help must take the nearer top edge when neither horizontal edge fits: help {help:?}, card {card:?}"
    );
    eprintln!("Title-row outside help: help {help:?}, card {card:?}, side room {side_room} px");
    Ok(())
}

fn undersized_fitted_help_fails_readable_cap_ratio() -> Result<(), String> {
    let help = capture_panel(
        DrawnPanelCase::UndersizedCapHelp,
        SceneLighting::Unlit,
        full_region(),
    )?;
    let row = capture_panel(
        DrawnPanelCase::PlainCapLabel,
        SceneLighting::Unlit,
        full_region(),
    )?;
    let cap = cap_bounds(&help)?;
    let row_cap = cap_bounds(&row)?;
    assert!(
        help_cap_height_matches_row(&cap, &row_cap).is_err(),
        "undersized help cap of {} px passed against row's {} px",
        cap.height(),
        row_cap.height()
    );
    Ok(())
}

fn world_tooltip_keeps_its_fill_and_readable_words_in_lit_scenes() -> Result<(), String> {
    let reference = capture_panel(
        DrawnPanelCase::TooltipWords,
        SceneLighting::Unlit,
        text_area_region(),
    )?;
    let (authored_fill, _) = text_area_fill(&reference)?;
    for lighting in [SceneLighting::Bright, SceneLighting::Dark] {
        let image = capture_panel(DrawnPanelCase::TooltipWords, lighting, text_area_region())?;
        let (fill, _) = text_area_fill(&image)?;
        assert!(
            fill.into_iter()
                .zip(authored_fill)
                .all(|(drawn, authored)| drawn.abs_diff(authored) <= 3),
            "scene lighting changed tooltip fill from {authored_fill:?} to {fill:?}"
        );
        let count = high_contrast_in_text_area(&image)?;
        assert!(count >= 300, "only {count} tooltip-word pixels meet 4.5:1");
    }
    Ok(())
}

const SHADOW_HOST_FILL: Color = Color::srgb(0.85, 0.85, 0.85);
const SHADOW_HOST_INK: Color = Color::srgb(0.4, 0.4, 0.4);
/// Image pixels per tooltip layout pixel in a shadow capture.
const SHADOW_SCALE: f32 = 3.0;
/// The tooltip shadow's blur support and downward offset, in tooltip layout pixels.
const SHADOW_BLUR_PX: f32 = 8.0;
const SHADOW_OFFSET_PX: f32 = 3.0;
/// How far below the body the shadow reaches, in tooltip layout pixels.
const SHADOW_REACH_BELOW_PX: f32 = SHADOW_OFFSET_PX + SHADOW_BLUR_PX;
const POINTS_PER_PX: f32 = 0.75;
/// Largest shown-to-hidden luminance ratio that reads as a drawn shadow.
const SHADOW_DARKENING: f64 = 0.95;
/// Largest channel difference between two pixels of one even fill.
const FILL_TOLERANCE: u8 = 3;
/// Smallest channel change only the opaque body paints: the 30% shadow moves a host channel
/// by about 35 at most.
const BODY_PAINT_CHANGE: u8 = 40;

/// Logical size of the window the shadow scenes place their tooltips in.
const SHADOW_WINDOW: Vec2 = Vec2::new(640.0, 400.0);
/// How far in front of a screen tooltip its capture camera sits, in logical pixels.
const SCREEN_CAPTURE_DISTANCE: f32 = 1000.0;

/// Whether the tooltip's widget sits on a world panel or a screen-space panel.
#[derive(Clone, Copy, Debug)]
enum TooltipSpace {
    World,
    Screen,
}

/// How a shadow capture frames the shown tooltip.
#[derive(Clone, Copy)]
enum ShadowFraming {
    /// The body with its whole shadow and a margin around both.
    WholeTooltip,
    /// The view moved down until its top edge sits one image pixel below the drawn body, so
    /// only the shadow is in view.
    BelowBody,
}

/// Whether the capture camera composites with order-independent transparency.
#[derive(Clone, Copy)]
enum CaptureTransparency {
    Standard,
    Stable,
}

/// One view of a tooltip over its host, captured shown and then hidden.
struct ShadowCaptures {
    shown:  Vec<u8>,
    hidden: Vec<u8>,
    /// Drawn body edges, border included, in image pixels, y down.
    body:   Rect,
    /// Edges of the even fill inside the body's border.
    fill:   Rect,
}

fn shadow_host_panel() -> Result<DiegeticPanel, String> {
    let tooltip = Tooltip::new(El::column().size(0.03, 0.008))
        .with_label_size(Mm(18.0))
        .show_after(Duration::ZERO);
    let mut layout = LayoutBuilder::with_root(
        El::column()
            .size(0.10, 0.06)
            .padding(Padding::all(Mm(10.0)))
            .background(SHADOW_HOST_FILL),
    );
    layout.with(
        El::new()
            .size(0.03, 0.008)
            .button("control")
            .tooltip(tooltip),
        |_| {},
    );
    layout.text(Text::new(
        "HHHH",
        TextStyle::new(0.024).with_color(SHADOW_HOST_INK),
    ));
    DiegeticPanel::world()
        .font_unit(Unit::Meters)
        .size(0.10, 0.06)
        .world_width(0.10)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid shadow host panel: {error}"))
}

fn screen_shadow_host_panel(window: Entity) -> Result<DiegeticPanel, String> {
    let tooltip = Tooltip::new(El::column().size(Px(90.0), Px(24.0))).show_after(Duration::ZERO);
    let mut layout = LayoutBuilder::with_root(
        El::column()
            .size(Px(240.0), Px(160.0))
            .padding(Padding::all(Px(20.0)))
            .background(SHADOW_HOST_FILL),
    );
    layout.with(
        El::new()
            .size(Px(90.0), Px(24.0))
            .button("control")
            .tooltip(tooltip),
        |_| {},
    );
    layout.text(Text::new(
        "HHHH",
        TextStyle::new(64.0).with_color(SHADOW_HOST_INK),
    ));
    DiegeticPanel::screen()
        .size(Px(240.0), Px(160.0))
        .anchor(Anchor::Center)
        .screen_position(0.5 * SHADOW_WINDOW.x, 0.5 * SHADOW_WINDOW.y)
        .window_entity(window)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid screen shadow host panel: {error}"))
}

/// A tooltip shown over its host, in the window that places it.
struct ShownTooltip {
    app:     CaseApp,
    window:  Entity,
    control: Entity,
    tip:     Entity,
}

/// Focuses or clears the control, then waits for its tooltip to show or hide.
fn set_tooltip_shown(tooltip: &mut ShownTooltip, shown: bool) -> Result<(), String> {
    let window = tooltip.window;
    if shown {
        tooltip.app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: tooltip.control,
        });
    } else {
        tooltip.app.world_mut().trigger(ClearWidgetFocus { window });
    }
    for _ in 0..30 {
        tooltip.app.update();
    }
    let hidden = tooltip.app.world().get::<Visibility>(tooltip.tip) == Some(&Visibility::Hidden);
    if hidden == shown {
        return Err(format!(
            "tooltip hidden={hidden} after its control's focus changed to shown={shown}"
        ));
    }
    Ok(())
}

/// The shadow scenes' host: a light panel with dark words under its control's tooltip.
fn shadow_host(space: TooltipSpace, window: Entity) -> Result<DiegeticPanel, String> {
    match space {
        TooltipSpace::World => shadow_host_panel(),
        TooltipSpace::Screen => screen_shadow_host_panel(window),
    }
}

fn show_tooltip_over_host(
    space: TooltipSpace,
    host: impl FnOnce(Entity) -> Result<DiegeticPanel, String>,
) -> Result<ShownTooltip, String> {
    let mut app = CaseApp::new();
    let mut window = Window::default();
    window.resolution.set(SHADOW_WINDOW.x, SHADOW_WINDOW.y);
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Entity(window)),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        Transform::from_xyz(0.0, 0.0, 5.0)
            .with_rotation(Quat::from_rotation_y(NONPARALLEL_PRESENTATION_ANGLE)),
    ));
    let host_panel = host(window)?;
    let host = match space {
        TooltipSpace::World => app
            .world_mut()
            .spawn((host_panel, Transform::IDENTITY))
            .id(),
        TooltipSpace::Screen => app.world_mut().spawn(host_panel).id(),
    };
    app.settle()?;
    let control = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(host, &PanelElementId::named("control"))
        })
        .map_err(|error| format!("widget query failed: {error}"))?
        .ok_or_else(|| "host has no tooltip control".to_string())?;
    let tip = app
        .world()
        .get::<Tooltips>(control)
        .and_then(|controllers| controllers.iter().next())
        .ok_or_else(|| "control has no tooltip controller".to_string())?;
    let mut tooltip = ShownTooltip {
        app,
        window,
        control,
        tip,
    };
    set_tooltip_shown(&mut tooltip, true)?;
    Ok(tooltip)
}

/// The capture image's size in pixels.
fn image_size() -> Vec2 { UVec2::new(WIDTH, HEIGHT).as_vec2() }

/// Where a capture camera looks, and where the tooltip's panel plane lands in its image.
struct ShadowView {
    transform:        Transform,
    viewport_height:  f32,
    layers:           RenderLayers,
    /// World offset that moves the view one image pixel down.
    image_pixel_down: Vec3,
    /// Panel plane edges in image pixels, y down.
    plane:            Rect,
}

/// Image coordinates, y down, of a point in a view through `focus` with the given axes.
fn image_point(point: Vec3, focus: Vec3, right: Vec3, up: Vec3, image_per_world: f32) -> Vec2 {
    let offset = point - focus;
    0.5 * image_size() + image_per_world * Vec2::new(offset.dot(right), -offset.dot(up))
}

/// A view centred on the tooltip and its shadow, square to the tooltip's plane.
fn world_shadow_view(app: &CaseApp, tip: Entity) -> Result<ShadowView, String> {
    let transform = *app
        .world()
        .get::<GlobalTransform>(tip)
        .ok_or_else(|| "shown tooltip has no world transform".to_string())?;
    let panel = app
        .world()
        .get::<DiegeticPanel>(tip)
        .ok_or_else(|| "shown tooltip has no panel".to_string())?;
    let world_per_px = POINTS_PER_PX
        * panel.points_to_world()
        * transform.affine().transform_vector3(Vec3::Y).length();
    let plane = PanelPlane::from_panel(panel, &transform)
        .map_err(|error| format!("shown tooltip has no panel plane: {error}"))?;
    let normal = transform.affine().transform_vector3(Vec3::Z).normalize();
    let up = transform.affine().transform_vector3(Vec3::Y).normalize();
    let image_per_world = SHADOW_SCALE / world_per_px;
    let focus = plane.point(Anchor::Center) - up * (0.5 * SHADOW_REACH_BELOW_PX * world_per_px);
    let view = Transform::from_translation(focus + normal * 0.25).looking_at(focus, up);
    let corner = |anchor| {
        image_point(
            plane.point(anchor),
            focus,
            *view.right(),
            *view.up(),
            image_per_world,
        )
    };
    Ok(ShadowView {
        transform:        view,
        viewport_height:  image_size().y / image_per_world,
        layers:           RenderLayers::default(),
        image_pixel_down: -up / image_per_world,
        plane:            Rect::from_corners(corner(Anchor::TopLeft), corner(Anchor::BottomRight)),
    })
}

/// A screen tooltip lives on its overlay's private layers, one world unit per logical pixel.
fn screen_shadow_view(app: &CaseApp, tip: Entity) -> Result<ShadowView, String> {
    let translation = app
        .world()
        .get::<GlobalTransform>(tip)
        .ok_or_else(|| "shown screen tooltip has no transform".to_string())?
        .translation();
    let panel = app
        .world()
        .get::<DiegeticPanel>(tip)
        .ok_or_else(|| "shown screen tooltip has no panel".to_string())?;
    let layers = app
        .world()
        .get::<RenderLayers>(tip)
        .cloned()
        .ok_or_else(|| "shown screen tooltip has no overlay layers".to_string())?;
    let (offset_x, offset_y) = panel.anchor_offsets();
    let top_left = Vec3::new(translation.x - offset_x, translation.y + offset_y, 0.0);
    let bottom_right = top_left + Vec3::new(panel.width(), -panel.height(), 0.0);
    let focus = top_left.midpoint(bottom_right) - Vec3::Y * (0.5 * SHADOW_REACH_BELOW_PX);
    let corner = |point| image_point(point, focus, Vec3::X, Vec3::Y, SHADOW_SCALE);
    Ok(ShadowView {
        transform: Transform::from_translation(focus + Vec3::Z * SCREEN_CAPTURE_DISTANCE)
            .looking_at(focus, Vec3::Y),
        viewport_height: image_size().y / SHADOW_SCALE,
        layers,
        image_pixel_down: -Vec3::Y / SHADOW_SCALE,
        plane: Rect::from_corners(corner(top_left), corner(bottom_right)),
    })
}

fn tooltip_view(tooltip: &ShownTooltip, space: TooltipSpace) -> Result<ShadowView, String> {
    match space {
        TooltipSpace::World => world_shadow_view(&tooltip.app, tooltip.tip),
        TooltipSpace::Screen => screen_shadow_view(&tooltip.app, tooltip.tip),
    }
}

fn spawn_shadow_camera(
    app: &mut CaseApp,
    view: &ShadowView,
    space: TooltipSpace,
    transparency: CaptureTransparency,
    target: Handle<Image>,
) -> Entity {
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            RenderTarget::Image(target.into()),
            Msaa::Off,
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: view.viewport_height,
                },
                far: 2.0 * SCREEN_CAPTURE_DISTANCE,
                ..OrthographicProjection::default_3d()
            }),
            view.transform,
            view.layers.clone(),
        ))
        .id();
    match space {
        TooltipSpace::World => {
            app.world_mut()
                .entity_mut(camera)
                .insert((Hdr, Tonemapping::KhronosPbrNeutral));
        },
        TooltipSpace::Screen => {
            app.world_mut()
                .entity_mut(camera)
                .insert((Tonemapping::None, DebandDither::Disabled));
        },
    }
    if matches!(transparency, CaptureTransparency::Stable) {
        app.world_mut()
            .entity_mut(camera)
            .insert(StableTransparency);
    }
    camera
}

fn capture_after_frames(app: &mut CaseApp, target: Handle<Image>) -> Result<Vec<u8>, String> {
    for _ in 0..4 {
        app.update();
    }
    let size = app
        .world()
        .resource::<Assets<Image>>()
        .get(&target)
        .map(Image::size)
        .ok_or_else(|| "shadow capture target is gone".to_string())?;
    let image = app.capture(target)?;
    let expected = (size.x * size.y * 4) as usize;
    if image.len() != expected {
        return Err(format!(
            "shadow capture holds {} bytes, not {expected}",
            image.len()
        ));
    }
    Ok(image)
}

/// Captures the shown tooltip once its body paints the plane's centre.
///
/// A revealed tooltip can take frames to draw, so this waits on that drawn pixel, and
/// nextest's slow timeout bounds the wait. The first capture without the body says what the
/// wait is for, so a timed-out run names the case and the colours it saw.
fn capture_drawn_tooltip(
    app: &mut CaseApp,
    target: &Handle<Image>,
    hidden: &[u8],
    plane: Rect,
) -> Result<Vec<u8>, String> {
    let centre = plane.center().as_uvec2();
    if centre.cmpge(UVec2::new(WIDTH, HEIGHT)).any() {
        return Err(format!(
            "tooltip plane {plane:?} centres outside the capture"
        ));
    }
    let mut reported = false;
    loop {
        let shown = capture_after_frames(app, target.clone())?;
        if body_paints(&shown, hidden, centre) {
            return Ok(shown);
        }
        if !reported {
            eprintln!(
                "{}: waiting for the tooltip body at {centre}: drawn {:?}, host {:?}",
                drawn_support::current_case_name(),
                color_at(&shown, centre.x, centre.y),
                color_at(hidden, centre.x, centre.y)
            );
            reported = true;
        }
    }
}

fn tooltip_translation(tooltip: &ShownTooltip) -> Result<Vec3, String> {
    tooltip
        .app
        .world()
        .get::<GlobalTransform>(tooltip.tip)
        .map(GlobalTransform::translation)
        .ok_or_else(|| "shown tooltip has no transform".to_string())
}

/// Captures the host with the tooltip hidden, then the tooltip shown again once its body draws
/// where the view was built around it.
fn capture_hidden_then_shown(
    tooltip: &mut ShownTooltip,
    target: &Handle<Image>,
    plane: Rect,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let viewed = tooltip_translation(tooltip)?;
    set_tooltip_shown(tooltip, false)?;
    let hidden = capture_after_frames(&mut tooltip.app, target.clone())?;
    set_tooltip_shown(tooltip, true)?;
    let placed = tooltip_translation(tooltip)?;
    if placed != viewed {
        return Err(format!(
            "tooltip shown again at {placed}, not at {viewed} where the view was built"
        ));
    }
    let shown = capture_drawn_tooltip(&mut tooltip.app, target, &hidden, plane)?;
    Ok((hidden, shown))
}

/// Whether the opaque body paint changed this pixel from the host.
fn body_paints(shown: &[u8], hidden: &[u8], pixel: UVec2) -> bool {
    color_at(shown, pixel.x, pixel.y)
        .into_iter()
        .zip(color_at(hidden, pixel.x, pixel.y))
        .any(|(paint, host)| paint.abs_diff(host) > BODY_PAINT_CHANGE)
}

fn same_fill(colour: [u8; 3], fill: [u8; 3]) -> bool {
    colour
        .iter()
        .zip(fill)
        .all(|(channel, reference)| channel.abs_diff(reference) <= FILL_TOLERANCE)
}

/// The run of pixels through `centre` along its row and column that `belongs` accepts.
fn run_through(centre: UVec2, belongs: impl Fn(u32, u32) -> bool) -> Rect {
    let left = (0..centre.x)
        .rev()
        .take_while(|&x| belongs(x, centre.y))
        .last();
    let right = (centre.x + 1..WIDTH)
        .take_while(|&x| belongs(x, centre.y))
        .last();
    let top = (0..centre.y)
        .rev()
        .take_while(|&y| belongs(centre.x, y))
        .last();
    let bottom = (centre.y + 1..HEIGHT)
        .take_while(|&y| belongs(centre.x, y))
        .last();
    Rect::from_corners(
        UVec2::new(left.unwrap_or(centre.x), top.unwrap_or(centre.y)).as_vec2(),
        (UVec2::new(right.unwrap_or(centre.x), bottom.unwrap_or(centre.y)) + 1).as_vec2(),
    )
}

/// The tooltip's drawn body and fill in image pixels, y down, from the panel plane's centre:
/// the body is every pixel the opaque paint changed, the fill the even colour at the centre.
fn drawn_body(shown: &[u8], hidden: &[u8], plane: Rect) -> Result<(Rect, Rect), String> {
    let centre = plane.center().as_uvec2();
    if centre.cmpge(UVec2::new(WIDTH, HEIGHT)).any() {
        return Err(format!(
            "tooltip plane {plane:?} centres outside the capture"
        ));
    }
    let painted = |x, y| body_paints(shown, hidden, UVec2::new(x, y));
    let fill_colour = color_at(shown, centre.x, centre.y);
    let body = run_through(centre, painted);
    let fill = run_through(centre, |x, y| same_fill(color_at(shown, x, y), fill_colour));
    let inside_plane =
        plane.inflate(1.0).contains(body.min) && plane.inflate(1.0).contains(body.max);
    if !painted(centre.x, centre.y) || !inside_plane || body.width() < 0.5 * plane.width() {
        return Err(format!(
            "paint at the tooltip plane's centre spans {body:?}, not a body inside plane {plane:?}"
        ));
    }
    Ok((body, fill))
}

fn capture_tooltip_shadow(
    space: TooltipSpace,
    framing: ShadowFraming,
    transparency: CaptureTransparency,
) -> Result<ShadowCaptures, String> {
    let mut tooltip = show_tooltip_over_host(space, |window| shadow_host(space, window))?;
    let view = tooltip_view(&tooltip, space)?;
    let target = tooltip.app.target(full_region());
    let camera = spawn_shadow_camera(&mut tooltip.app, &view, space, transparency, target.clone());
    let (hidden, shown) = capture_hidden_then_shown(&mut tooltip, &target, view.plane)?;
    let (body, fill) = drawn_body(&shown, &hidden, view.plane)?;
    match framing {
        ShadowFraming::WholeTooltip => Ok(ShadowCaptures {
            shown,
            hidden,
            body,
            fill,
        }),
        ShadowFraming::BelowBody => {
            let drop = body.max.y + 1.0;
            tooltip
                .app
                .world_mut()
                .get_mut::<Transform>(camera)
                .ok_or_else(|| "shadow camera has no transform".to_string())?
                .translation += view.image_pixel_down * drop;
            let shown = capture_after_frames(&mut tooltip.app, target.clone())?;
            set_tooltip_shown(&mut tooltip, false)?;
            let hidden = capture_after_frames(&mut tooltip.app, target.clone())?;
            let lift =
                |rect: Rect| Rect::from_center_size(rect.center() - Vec2::Y * drop, rect.size());
            Ok(ShadowCaptures {
                shown,
                hidden,
                body: lift(body),
                fill: lift(fill),
            })
        },
    }
}

/// Image pixels whose centres lie inside `area`.
fn pixels_in(area: Rect) -> impl Iterator<Item = (u32, u32)> {
    let first = (area.min - 0.5).ceil().max(Vec2::ZERO).as_uvec2();
    let end = ((area.max - 0.5).floor() + 1.0)
        .clamp(Vec2::ZERO, image_size())
        .as_uvec2();
    (first.y..end.y).flat_map(move |y| (first.x..end.x).map(move |x| (x, y)))
}

/// The band from 1 to 2.5 tooltip pixels under the body, clear of its rounded corners,
/// darkens host fill and host ink alike.
fn shadow_band_darkens_host(captures: &ShadowCaptures) -> Result<(), String> {
    let body = captures.body;
    // No corner radius exceeds half the body's height.
    let corner_inset = 0.5 * body.height();
    let band_bottom = 2.5 * SHADOW_SCALE;
    let band = Rect::new(
        body.min.x + corner_inset,
        body.max.y + SHADOW_SCALE,
        body.max.x - corner_inset,
        body.max.y + band_bottom,
    );
    let pixels: Vec<(f64, f64)> = pixels_in(band)
        .map(|(x, y)| {
            (
                luminance(color_at(&captures.shown, x, y)),
                luminance(color_at(&captures.hidden, x, y)),
            )
        })
        .collect();
    let (darkest, lightest) = pixels
        .iter()
        .fold((f64::MAX, 0.0_f64), |(low, high), (_, host)| {
            (low.min(*host), high.max(*host))
        });
    if pixels.is_empty() || lightest < 2.0 * darkest {
        return Err(format!(
            "shadow band {band:?} crosses no host ink (host luminance {darkest:.3}..{lightest:.3})"
        ));
    }
    let middle = f64::midpoint(darkest, lightest);
    for (label, over_ink) in [("fill", false), ("ink", true)] {
        let class: Vec<&(f64, f64)> = pixels
            .iter()
            .filter(|(_, host)| (*host < middle) == over_ink)
            .collect();
        let shown: f64 = class.iter().map(|(shown, _)| shown).sum();
        let host: f64 = class.iter().map(|(_, host)| host).sum();
        if let Some((shown, host)) = class.iter().find(|(shown, host)| shown >= host) {
            return Err(format!(
                "host {label} under the shadow band is not darker: luminance {shown:.4} over {host:.4}"
            ));
        }
        let ratio = shown / host;
        if ratio > SHADOW_DARKENING {
            return Err(format!(
                "shadow band darkens host {label} only to {ratio:.3} of its luminance"
            ));
        }
    }
    Ok(())
}

/// Nothing changes past the shadow's reach: below the offset blur, or beside the side blur.
fn nothing_drawn_past_shadow_reach(captures: &ShadowCaptures) -> Result<(), String> {
    let body = captures.body;
    let below = (SHADOW_REACH_BELOW_PX + 1.0) * SHADOW_SCALE;
    let side = (SHADOW_BLUR_PX + 1.0) * SHADOW_SCALE;
    let reach_below = body.max.y + below;
    let image = image_size();
    let beside_top = body.min.y.max(0.0);
    let untouched = [
        Rect::new(0.0, reach_below, image.x, image.y),
        Rect::new(0.0, beside_top, body.min.x - side, reach_below),
        Rect::new(body.max.x + side, beside_top, image.x, reach_below),
    ];
    for area in untouched {
        if let Some((x, y)) = pixels_in(area).find(|&(x, y)| {
            color_at(&captures.shown, x, y)
                .into_iter()
                .zip(color_at(&captures.hidden, x, y))
                .any(|(shown, host)| shown.abs_diff(host) > 2)
        }) {
            return Err(format!(
                "pixel ({x}, {y}) past the shadow's reach changed from {:?} to {:?}",
                color_at(&captures.hidden, x, y),
                color_at(&captures.shown, x, y)
            ));
        }
    }
    Ok(())
}

/// The body's fill stays even: no shadow paints over it.
fn body_draws_over_its_shadow(captures: &ShadowCaptures) -> Result<(), String> {
    let fill = captures.fill;
    // Clear of the rounded corners: a world tooltip's corners reach further in from its sides
    // than half its height, but never as far as its full height.
    let corner_inset = fill.height();
    let interior = Rect::new(
        fill.min.x + corner_inset,
        fill.min.y + SHADOW_SCALE,
        fill.max.x - corner_inset,
        fill.max.y - SHADOW_SCALE,
    );
    let mut colours = pixels_in(interior).map(|(x, y)| color_at(&captures.shown, x, y));
    let first = colours
        .next()
        .ok_or_else(|| format!("tooltip body {:?} is outside the capture", captures.body))?;
    if let Some(colour) = colours.find(|&colour| !same_fill(colour, first)) {
        return Err(format!(
            "tooltip body is uneven: {colour:?} beside {first:?}"
        ));
    }
    Ok(())
}

fn dump_shadow_captures(captures: &ShadowCaptures) -> Result<(), String> {
    for (label, image) in [("shown", &captures.shown), ("hidden", &captures.hidden)] {
        dump_pixels::<false>(
            "tooltip_drawn_words",
            &format!("{}_{label}", drawn_support::current_case_name()),
            &format!("tooltip body {:?} fill {:?}", captures.body, captures.fill),
            full_region(),
            full_region(),
            image,
        )?;
    }
    Ok(())
}

fn tooltip_shadow_case(
    space: TooltipSpace,
    transparency: CaptureTransparency,
) -> Result<(), String> {
    let captures = capture_tooltip_shadow(space, ShadowFraming::WholeTooltip, transparency)?;
    let checks = body_draws_over_its_shadow(&captures)
        .and_then(|()| shadow_band_darkens_host(&captures))
        .and_then(|()| nothing_drawn_past_shadow_reach(&captures));
    if checks.is_err() {
        dump_shadow_captures(&captures)?;
    }
    checks
}

fn world_tooltip_shadow_draws_between_body_and_host() -> Result<(), String> {
    tooltip_shadow_case(TooltipSpace::World, CaptureTransparency::Standard)
}

fn world_tooltip_shadow_draws_between_body_and_host_under_oit() -> Result<(), String> {
    tooltip_shadow_case(TooltipSpace::World, CaptureTransparency::Stable)
}

fn screen_tooltip_shadow_draws_between_body_and_host() -> Result<(), String> {
    tooltip_shadow_case(TooltipSpace::Screen, CaptureTransparency::Standard)
}

fn screen_tooltip_shadow_draws_between_body_and_host_under_oit() -> Result<(), String> {
    tooltip_shadow_case(TooltipSpace::Screen, CaptureTransparency::Stable)
}

/// A body just past the view's edge still draws the shadow that reaches into view.
fn world_tooltip_shadow_draws_when_only_the_shadow_is_in_view() -> Result<(), String> {
    let captures = capture_tooltip_shadow(
        TooltipSpace::World,
        ShadowFraming::BelowBody,
        CaptureTransparency::Standard,
    )?;
    let checks = shadow_band_darkens_host(&captures)
        .and_then(|()| nothing_drawn_past_shadow_reach(&captures));
    if checks.is_err() {
        dump_shadow_captures(&captures)?;
    }
    checks
}

/// A blended surface nearer the camera than a world tooltip, over the band its shadow darkens,
/// keeps its own colour: the shadow composites before it, not over it.
fn world_tooltip_shadow_leaves_a_nearer_blended_surface_its_colour() -> Result<(), String> {
    const NEARER: f32 = 2.0;
    const NEARER_SURFACE: Color = Color::srgb(0.1, 0.3, 0.9);
    let mut tooltip = show_tooltip_over_host(TooltipSpace::World, |window| {
        shadow_host(TooltipSpace::World, window)
    })?;
    let mut view = world_shadow_view(&tooltip.app, tooltip.tip)?;
    let transform = *tooltip
        .app
        .world()
        .get::<GlobalTransform>(tooltip.tip)
        .ok_or_else(|| "shown tooltip has no world transform".to_string())?;
    let panel = tooltip
        .app
        .world()
        .get::<DiegeticPanel>(tooltip.tip)
        .ok_or_else(|| "shown tooltip has no panel".to_string())?;
    let plane = PanelPlane::from_panel(panel, &transform)
        .map_err(|error| format!("shown tooltip has no panel plane: {error}"))?;
    let normal = transform.affine().transform_vector3(Vec3::Z).normalize();
    let up = transform.affine().transform_vector3(Vec3::Y).normalize();
    let bottom_left = plane.point(Anchor::BottomLeft);
    let bottom_right = plane.point(Anchor::BottomRight);
    let width = bottom_left.distance(bottom_right);
    let world_per_image_px = width / view.plane.width();
    let gap = SHADOW_SCALE * world_per_image_px;
    let height = SHADOW_REACH_BELOW_PX * SHADOW_SCALE * world_per_image_px;
    let centre =
        bottom_left.midpoint(bottom_right) - up * height.mul_add(0.5, gap) + normal * NEARER;
    let mesh = tooltip
        .app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(width, height));
    let material = tooltip
        .app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: NEARER_SURFACE,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        });
    tooltip.app.world_mut().spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_translation(centre).with_rotation(transform.compute_transform().rotation),
    ));
    view.transform.translation += normal * (2.0 * NEARER);
    let target = tooltip.app.target(full_region());
    spawn_shadow_camera(
        &mut tooltip.app,
        &view,
        TooltipSpace::World,
        CaptureTransparency::Standard,
        target.clone(),
    );
    let (hidden, shown) = capture_hidden_then_shown(&mut tooltip, &target, view.plane)?;
    // Inset one tooltip pixel from every edge of the surface, clear of its antialiasing.
    let surface = Rect::new(
        view.plane.min.x + SHADOW_SCALE,
        SHADOW_SCALE.mul_add(2.0, view.plane.max.y),
        view.plane.max.x - SHADOW_SCALE,
        SHADOW_REACH_BELOW_PX.mul_add(SHADOW_SCALE, view.plane.max.y),
    );
    let middle = surface.center().as_uvec2();
    let drawn = color_at(&hidden, middle.x, middle.y);
    if drawn[2] <= drawn[0].saturating_add(40) {
        return Err(format!(
            "the nearer surface does not draw over the host at {middle}: {drawn:?}"
        ));
    }
    if let Some((x, y)) = pixels_in(surface).find(|&(x, y)| {
        color_at(&shown, x, y)
            .into_iter()
            .zip(color_at(&hidden, x, y))
            .any(|(shown, hidden)| shown.abs_diff(hidden) > 2)
    }) {
        return Err(format!(
            "the tooltip shadow changed the nearer surface at ({x}, {y}) from {:?} to {:?}",
            color_at(&hidden, x, y),
            color_at(&shown, x, y)
        ));
    }
    Ok(())
}

/// Words whose ink rises above the cap height and drops below the baseline. The first glyph
/// stands on the baseline, so its foot marks it.
const PADDED_WORDS: &str = "Plug a jack";

/// Drawn gaps between a tooltip's words and the inside of its border, in tooltip layout pixels.
#[derive(Debug)]
struct InkPadding {
    left:            f64,
    right:           f64,
    /// Below the top of the tallest ink.
    top:             f64,
    /// Below the lowest drawn ink, including descenders.
    bottom:          f64,
    /// Ink below the first glyph's baseline.
    descender_depth: f64,
}

/// A host that draws nothing, so its control's word tooltip is the only colour over the black
/// clear colour. The world tooltip's label is sized in points.
fn bare_words_host(
    space: TooltipSpace,
    window: Entity,
    words: &'static str,
) -> Result<DiegeticPanel, String> {
    let mut tooltip = Tooltip::new(El::column()).show_after(Duration::ZERO);
    if matches!(space, TooltipSpace::World) {
        tooltip = tooltip.with_label_size(Pt(11.0));
    }
    tooltip.text(words);
    match space {
        TooltipSpace::World => {
            let mut layout = LayoutBuilder::new(0.08, 0.04);
            layout.with(
                El::new()
                    .size(0.08, 0.04)
                    .button("control")
                    .tooltip(tooltip),
                |_| {},
            );
            DiegeticPanel::world()
                .size(0.08, 0.04)
                .world_width(0.08)
                .with_tree(layout.build())
                .build()
                .map_err(|error| format!("invalid world words host: {error}"))
        },
        TooltipSpace::Screen => {
            let mut layout = LayoutBuilder::with_root(El::column().size(Px(240.0), Px(160.0)));
            layout.with(
                El::new()
                    .size(Px(90.0), Px(24.0))
                    .button("control")
                    .tooltip(tooltip),
                |_| {},
            );
            DiegeticPanel::screen()
                .size(Px(240.0), Px(160.0))
                .anchor(Anchor::Center)
                .screen_position(0.5 * SHADOW_WINDOW.x, 0.5 * SHADOW_WINDOW.y)
                .window_entity(window)
                .with_tree(layout.build())
                .build()
                .map_err(|error| format!("invalid screen words host: {error}"))
        },
    }
}

/// Solid word ink inside the tooltip's fill: pixels at least halfway from the fill to the
/// brightest pixel there, in linear luminance. Returns the ink's bounds and the foot of its
/// first glyph, the columns from the ink's left edge to the first column without ink.
fn words_ink(
    image: &[u8],
    fill_colour: [u8; 3],
    fill: &PixelBounds,
) -> Result<(PixelBounds, u32), String> {
    let inside =
        |x: u32, y: u32| fill.left < x && x < fill.right && fill.top < y && y < fill.bottom;
    let fill_luminance = luminance(fill_colour);
    let brightest = (fill.top + 1..fill.bottom)
        .flat_map(|y| (fill.left + 1..fill.right).map(move |x| luminance(color_at(image, x, y))))
        .fold(fill_luminance, f64::max);
    let solid_ink = f64::midpoint(fill_luminance, brightest);
    let is_ink = |x, y| inside(x, y) && luminance(color_at(image, x, y)) >= solid_ink;
    let ink = pixel_bounds(image, |x, y, _| is_ink(x, y))
        .ok_or_else(|| format!("tooltip drew no word ink inside its fill {fill:?}"))?;
    let first_glyph_end = (ink.left..=ink.right)
        .find(|&x| !(ink.top..=ink.bottom).any(|y| is_ink(x, y)))
        .unwrap_or(ink.right + 1);
    let foot = (ink.top..=ink.bottom)
        .rev()
        .find(|&y| (ink.left..first_glyph_end).any(|x| is_ink(x, y)))
        .ok_or_else(|| format!("words drew no first glyph: ink {ink:?}"))?;
    Ok((ink, foot))
}

/// Shows words in a tooltip and measures its drawn padding at `SHADOW_SCALE`.
fn drawn_words_padding(space: TooltipSpace, words: &'static str) -> Result<InkPadding, String> {
    let mut tooltip =
        show_tooltip_over_host(space, |window| bare_words_host(space, window, words))?;
    let view = tooltip_view(&tooltip, space)?;
    let target = tooltip.app.target(full_region());
    spawn_shadow_camera(
        &mut tooltip.app,
        &view,
        space,
        CaptureTransparency::Standard,
        target.clone(),
    );
    let image = capture_after_frames(&mut tooltip.app, target)?;
    let (fill_colour, fill) = solid_fill(&image)?;
    let (ink, foot) = words_ink(&image, fill_colour, &fill)?;
    let layout_pixels = |image_pixels: u32| f64::from(image_pixels) / f64::from(SHADOW_SCALE);
    Ok(InkPadding {
        left:            layout_pixels(ink.left - fill.left),
        right:           layout_pixels(fill.right - ink.right),
        top:             layout_pixels(ink.top - fill.top),
        bottom:          layout_pixels(fill.bottom - ink.bottom),
        descender_depth: layout_pixels(ink.bottom - foot),
    })
}

/// Left and right gaps match within a tooltip pixel, as do the gaps above and below the full
/// drawn ink, and no gap is under a pixel.
fn words_have_even_drawn_padding(space: TooltipSpace) -> Result<(), String> {
    let padding = drawn_words_padding(space, PADDED_WORDS)?;
    eprintln!("{space:?} tooltip: drawn ink padding {padding:?} px");
    assert!(
        padding.descender_depth > 0.0,
        "words drew no descender: {padding:?}"
    );
    assert!(
        (padding.left - padding.right).abs() <= 1.0 && (padding.top - padding.bottom).abs() <= 1.0,
        "uneven drawn ink padding {padding:?}"
    );
    assert!(
        [padding.left, padding.right, padding.top, padding.bottom]
            .into_iter()
            .all(|gap| gap >= 1.0),
        "words touch the border: {padding:?}"
    );
    Ok(())
}

/// Descenders and dotted glyphs must sit visually centred inside both a screen tooltip and a
/// world tooltip, measured against the full drawn ink rather than the typeface baseline.
#[test]
fn screen_and_world_tooltips_balance_descender_ink_inside_the_frame() -> Result<(), String> {
    let mut failures = Vec::new();
    for (space, words) in [
        (TooltipSpace::Screen, "Processed image."),
        (TooltipSpace::World, "Processed image."),
    ] {
        let padding = drawn_words_padding(space, words)?;
        eprintln!("{space:?} {words:?}: drawn padding {padding:?} layout px");
        if padding.descender_depth <= 0.0 {
            failures.push(format!(
                "{space:?} {words:?} drew no descender: {padding:?}"
            ));
        }
        if (padding.top - padding.bottom).abs() > 1.0 {
            failures.push(format!(
                "{space:?} {words:?} ink sits low in its frame: top/bottom gaps {:.2}/{:.2} layout px",
                padding.top, padding.bottom,
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[test]
fn screen_tooltip_pads_its_words_evenly() -> Result<(), String> {
    words_have_even_drawn_padding(TooltipSpace::Screen)
}

#[test]
fn point_sized_world_tooltip_pads_its_words_evenly() -> Result<(), String> {
    words_have_even_drawn_padding(TooltipSpace::World)
}

/// The bordered host's border: no fill, word or shadow in its scene is this red.
const HOST_BORDER_COLOR: Color = Color::srgb(0.9, 0.1, 0.1);
const HOST_BORDER_PX: f32 = 2.0;
/// Light, so the tooltip's shadow darkens every pixel it reaches, inside the host or out.
const BACKDROP: Color = Color::srgb(0.95, 0.95, 0.95);
/// `TOOLTIP_VIEWPORT_MARGIN` (`widgets/constants.rs`): the gap a screen tooltip keeps from an
/// edge, in logical pixels.
const TOOLTIP_MARGIN_PX: u32 = 8;
/// Largest channel change that still reads as the host's own pixel.
const UNPAINTED_TOLERANCE: u8 = 2;

/// The side of its control a tooltip is placed on.
#[derive(Clone, Copy, Debug)]
enum Placement {
    Left,
    Right,
    Above,
    Below,
}

impl Placement {
    /// The tooltip's anchor, and the control's anchor it attaches to.
    const fn anchors(self) -> (Anchor, Anchor) {
        match self {
            Self::Left => (Anchor::CenterRight, Anchor::CenterLeft),
            Self::Right => (Anchor::CenterLeft, Anchor::CenterRight),
            Self::Above => (Anchor::BottomCenter, Anchor::TopCenter),
            Self::Below => (Anchor::TopCenter, Anchor::BottomCenter),
        }
    }
}

/// A light screen host with a red border, centred in the window. A marked card narrower than
/// the tooltip fills its top-left, with the control near the card's bottom-left corner, so the
/// tooltip placed Left or Below it would cross the host's border.
fn bordered_host(placement: Placement, window: Entity) -> Result<DiegeticPanel, String> {
    let (source, target) = placement.anchors();
    let tooltip = Tooltip::new(El::column().size(Px(140.0), Px(28.0)))
        .show_after(Duration::ZERO)
        .source_anchor(source)
        .target_anchor(target);
    let mut layout = LayoutBuilder::with_root(
        El::row()
            .size(Px(240.0), Px(150.0))
            .padding(Padding::all(Px(10.0)))
            .border(Border::all(Px(HOST_BORDER_PX), HOST_BORDER_COLOR))
            .background(SHADOW_HOST_FILL),
    );
    layout.with(
        El::column().size(Px(100.0), Px(126.0)).tooltip_boundary(),
        |card| {
            card.with(El::new().size(Px(100.0), Px(100.0)), |_| {});
            card.with(El::row().size(Px(100.0), Px(20.0)), |button_row| {
                button_row.with(El::new().size(Px(6.0), Px(20.0)), |_| {});
                button_row.with(
                    El::new()
                        .size(Px(30.0), Px(20.0))
                        .button("control")
                        .tooltip(tooltip),
                    |_| {},
                );
            });
        },
    );
    DiegeticPanel::screen()
        .size(Px(240.0), Px(150.0))
        .anchor(Anchor::Center)
        .screen_position(0.5 * SHADOW_WINDOW.x, 0.5 * SHADOW_WINDOW.y)
        .window_entity(window)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid bordered host: {error}"))
}

/// The window in logical pixels; a border capture spends one image pixel on each.
fn window_size() -> UVec2 { SHADOW_WINDOW.as_uvec2() }

fn window_pixels() -> impl Iterator<Item = UVec2> {
    let size = window_size();
    (0..size.y).flat_map(move |y| (0..size.x).map(move |x| UVec2::new(x, y)))
}

fn window_color(image: &[u8], pixel: UVec2) -> [u8; 3] {
    let index = ((pixel.y * window_size().x + pixel.x) * 4) as usize;
    [image[index], image[index + 1], image[index + 2]]
}

/// The largest channel change from the host's pixel to the shown one.
fn channel_change(shown: &[u8], hidden: &[u8], pixel: UVec2) -> u8 {
    window_color(shown, pixel)
        .into_iter()
        .zip(window_color(hidden, pixel))
        .map(|(paint, host)| paint.abs_diff(host))
        .max()
        .unwrap_or(0)
}

/// Whether a pixel shows the border's red, alone or a third or more blended with its
/// neighbours.
fn is_border(colour: [u8; 3]) -> bool {
    let [red, green, blue] = colour.map(i16::from);
    red - green.max(blue) > 60
}

fn window_bounds(pixels: impl Iterator<Item = UVec2>) -> Option<PixelBounds> {
    pixels.fold(None, |bounds, pixel| {
        Some(bounds.map_or(
            PixelBounds {
                left:   pixel.x,
                right:  pixel.x,
                top:    pixel.y,
                bottom: pixel.y,
            },
            |bounds: PixelBounds| PixelBounds {
                left:   bounds.left.min(pixel.x),
                right:  bounds.right.max(pixel.x),
                top:    bounds.top.min(pixel.y),
                bottom: bounds.bottom.max(pixel.y),
            },
        ))
    })
}

/// A shown screen tooltip's body in window pixels, y down: its overlay spends one world unit
/// on each logical pixel, with the origin at the window's centre.
fn screen_tooltip_body(tooltip: &ShownTooltip) -> Result<Rect, String> {
    let panel = tooltip
        .app
        .world()
        .get::<DiegeticPanel>(tooltip.tip)
        .ok_or_else(|| "shown screen tooltip has no panel".to_string())?;
    let translation = tooltip_translation(tooltip)?;
    let (offset_x, offset_y) = panel.anchor_offsets();
    let min = 0.5 * SHADOW_WINDOW + Vec2::new(translation.x - offset_x, -translation.y - offset_y);
    Ok(Rect::from_corners(
        min,
        min + Vec2::new(panel.width(), panel.height()),
    ))
}

/// How a window capture camera draws the screen tooltip's overlay layers.
#[derive(Clone, Copy)]
enum WindowCamera {
    /// Over the light backdrop, one sample per pixel.
    Backdrop,
    /// As the overlay camera draws a window: four samples per pixel, over a black scene.
    Overlay,
}

/// A camera on the screen tooltip's overlay layers that sees the whole window as `camera`
/// draws it.
fn spawn_window_camera_by(
    tooltip: &mut ShownTooltip,
    target: Handle<Image>,
    camera: WindowCamera,
) -> Result<(), String> {
    let (clear, msaa) = match camera {
        WindowCamera::Backdrop => (BACKDROP, Msaa::Off),
        WindowCamera::Overlay => (Color::BLACK, Msaa::Sample4),
    };
    let layers = tooltip
        .app
        .world()
        .get::<RenderLayers>(tooltip.tip)
        .cloned()
        .ok_or_else(|| "shown screen tooltip has no overlay layers".to_string())?;
    tooltip.app.world_mut().spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(clear),
            ..default()
        },
        RenderTarget::Image(target.into()),
        msaa,
        Tonemapping::None,
        DebandDither::Disabled,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: SHADOW_WINDOW.y,
            },
            far: 2.0 * SCREEN_CAPTURE_DISTANCE,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 0.0, SCREEN_CAPTURE_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y),
        layers,
    ));
    Ok(())
}

/// Captures the window once the shown body paints its centre; nextest's slow timeout bounds
/// the wait.
fn capture_window_when_drawn(
    app: &mut CaseApp,
    target: &Handle<Image>,
    hidden: &[u8],
    body: Rect,
) -> Result<Vec<u8>, String> {
    let centre = body.center().as_uvec2();
    if centre.cmpge(window_size()).any() {
        return Err(format!("tooltip body {body:?} centres outside the window"));
    }
    loop {
        let shown = capture_after_frames(app, target.clone())?;
        if channel_change(&shown, hidden, centre) > BODY_PAINT_CHANGE {
            return Ok(shown);
        }
    }
}

/// The window around a screen tooltip, captured with it hidden and then shown.
struct WindowCaptures {
    hidden: Vec<u8>,
    shown:  Vec<u8>,
    /// The placed body in window pixels, y down.
    body:   Rect,
}

/// Renders `host`'s control with its tooltip hidden, then shown.
fn capture_window_hidden_then_shown(
    host: impl FnOnce(Entity) -> Result<DiegeticPanel, String>,
) -> Result<WindowCaptures, String> {
    capture_window_hidden_then_shown_by(host, WindowCamera::Backdrop)
}

/// Renders `host`'s control with its tooltip hidden, then shown, as `camera` draws the window.
fn capture_window_hidden_then_shown_by(
    host: impl FnOnce(Entity) -> Result<DiegeticPanel, String>,
    camera: WindowCamera,
) -> Result<WindowCaptures, String> {
    let mut tooltip = show_tooltip_over_host(TooltipSpace::Screen, host)?;
    let size = window_size();
    let target = tooltip
        .app
        .target(CaptureRegion::new(size, UVec2::ZERO, size));
    spawn_window_camera_by(&mut tooltip, target.clone(), camera)?;
    set_tooltip_shown(&mut tooltip, false)?;
    let hidden = capture_after_frames(&mut tooltip.app, target.clone())?;
    set_tooltip_shown(&mut tooltip, true)?;
    let body = screen_tooltip_body(&tooltip)?;
    let shown = capture_window_when_drawn(&mut tooltip.app, &target, &hidden, body)?;
    Ok(WindowCaptures {
        hidden,
        shown,
        body,
    })
}

/// Renders the bordered host with its control's tooltip hidden, then shown on `placement`'s
/// side. No pixel the tooltip paints, body or shadow, lies on a border pixel or within
/// `TOOLTIP_MARGIN_PX` of one along either axis.
fn paints_clear_of_its_host_border(placement: Placement) -> Result<(), String> {
    let WindowCaptures {
        hidden,
        shown,
        body,
    } = capture_window_hidden_then_shown(|window| bordered_host(placement, window))?;
    paint_keeps_clear_of_border(
        &format!("{placement:?} tooltip, body {body:?}"),
        &hidden,
        &shown,
    )
}

/// No pixel `shown` paints over `hidden`, body or shadow, lies on a red border pixel or within
/// `TOOLTIP_MARGIN_PX` of one along either axis.
fn paint_keeps_clear_of_border(case: &str, hidden: &[u8], shown: &[u8]) -> Result<(), String> {
    let size = window_size();
    let border: Vec<bool> = window_pixels()
        .map(|pixel| is_border(window_color(hidden, pixel)))
        .collect();
    let on_border = |pixel: UVec2| border[(pixel.y * size.x + pixel.x) as usize];
    let border_bounds = window_bounds(window_pixels().filter(|&pixel| on_border(pixel)))
        .ok_or_else(|| "the host drew no border pixels".to_string())?;
    let painted: Vec<UVec2> = window_pixels()
        .filter(|&pixel| channel_change(shown, hidden, pixel) > UNPAINTED_TOLERANCE)
        .collect();
    let painted_bounds = window_bounds(painted.iter().copied())
        .ok_or_else(|| "the shown tooltip painted no pixels".to_string())?;
    eprintln!("{case}: painted {painted_bounds:?}, border {border_bounds:?}");
    let reach = UVec2::splat(TOOLTIP_MARGIN_PX);
    let nearest = painted
        .iter()
        .flat_map(|&paint| {
            let first = paint.saturating_sub(reach);
            let last = (paint + reach).min(size - 1);
            (first.y..=last.y)
                .flat_map(move |y| (first.x..=last.x).map(move |x| UVec2::new(x, y)))
                .filter(move |&pixel| on_border(pixel))
                .map(move |pixel| (paint, pixel))
        })
        .min_by_key(|&(paint, pixel)| (paint.as_ivec2() - pixel.as_ivec2()).abs().max_element());
    let Some((paint, pixel)) = nearest else {
        return Ok(());
    };
    let gap = match (paint.as_ivec2() - pixel.as_ivec2()).abs().max_element() {
        0 => "covers".to_string(),
        distance => format!("leaves {} px to", distance - 1),
    };
    Err(format!(
        "{case}: paint at {paint} {gap} border pixel {pixel}, inside the {TOOLTIP_MARGIN_PX} px \
         margin: painted {painted_bounds:?}, border {border_bounds:?}"
    ))
}

fn screen_tooltip_placed_left_paints_clear_of_its_host_border() -> Result<(), String> {
    paints_clear_of_its_host_border(Placement::Left)
}

fn screen_tooltip_placed_below_paints_clear_of_its_host_border() -> Result<(), String> {
    paints_clear_of_its_host_border(Placement::Below)
}

#[test]
fn screen_tooltip_paints_clear_of_its_host_border() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "screen_tooltip_placed_left_paints_clear_of_its_host_border",
            run:  screen_tooltip_placed_left_paints_clear_of_its_host_border,
        },
        DrawnCase {
            name: "screen_tooltip_placed_below_paints_clear_of_its_host_border",
            run:  screen_tooltip_placed_below_paints_clear_of_its_host_border,
        },
    ])
}

/// A grey world host panel, centred on the origin, padded around a red-bordered marked card, as
/// the widgets example pads its marked card inside its face panel. The blue control sits at the
/// card's top-right, so its worded tooltip, drawn at the host's scale, would cross the card's
/// border and the panel's edge on its preferred right side; below it, slid inside the card over
/// the green neighbour row, there is room.
fn world_card_host() -> Result<DiegeticPanel, String> {
    let mut tooltip = Tooltip::new(El::column().width(Sizing::FIT).height(Sizing::FIT))
        .show_after(Duration::ZERO)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    tooltip.text("Applies");
    let mut layout = LayoutBuilder::with_root(
        El::column()
            .size(Px(240.0), Px(150.0))
            .padding(Padding::all(Px(10.0)))
            .background(WORLD_CARD_PANEL_FILL),
    );
    layout.with(
        El::column()
            .size(Px(200.0), Px(110.0))
            .border(Border::all(Px(HOST_BORDER_PX), HOST_BORDER_COLOR))
            .tooltip_boundary(),
        |card| {
            card.with(El::new().size(Px(196.0), Px(10.0)), |_| {});
            card.with(El::row().size(Px(196.0), Px(20.0)), |button_row| {
                button_row.with(El::new().size(Px(160.0), Px(20.0)), |_| {});
                button_row.with(
                    El::new()
                        .size(Px(30.0), Px(20.0))
                        .background(WORLD_CARD_CONTROL_FILL)
                        .button("control")
                        .tooltip(tooltip),
                    |_| {},
                );
            });
            card.with(El::new().size(Px(196.0), Px(10.0)), |_| {});
            card.with(
                El::new()
                    .size(Px(196.0), Px(20.0))
                    .background(NEIGHBOUR_ROW_FILL),
                |_| {},
            );
        },
    );
    DiegeticPanel::world()
        .size(Px(240.0), Px(150.0))
        .anchor(Anchor::Center)
        .world_width(2.4)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid world card host: {error}"))
}

/// Renders a world tooltip whose control sits at the edge of a bordered marked card. No pixel it
/// paints, body or shadow, lies on the card's border or within `TOOLTIP_MARGIN_PX` of it.
#[test]
fn world_tooltip_in_a_marked_card_paints_clear_of_the_card_border() -> Result<(), String> {
    let (hidden, shown) = capture_world_card_hidden_then_shown()?;
    paint_keeps_clear_of_border("world tooltip in a marked card", &hidden, &shown)
}

/// Renders the world card host with its control's tooltip hidden, then shown, through the
/// presentation camera's view.
fn capture_world_card_hidden_then_shown() -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut tooltip = show_tooltip_over_host(TooltipSpace::World, |_| world_card_host())?;
    let size = window_size();
    let target = tooltip
        .app
        .target(CaptureRegion::new(size, UVec2::ZERO, size));
    // The presentation camera's view, drawn into the capture image the window's size.
    tooltip.app.world_mut().spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(BACKDROP),
            ..default()
        },
        RenderTarget::Image(target.clone().into()),
        Msaa::Off,
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    set_tooltip_shown(&mut tooltip, false)?;
    let hidden = capture_after_frames(&mut tooltip.app, target.clone())?;
    set_tooltip_shown(&mut tooltip, true)?;
    let mut reported = false;
    let shown = loop {
        let shown = capture_after_frames(&mut tooltip.app, target.clone())?;
        let change = window_pixels()
            .map(|pixel| channel_change(&shown, &hidden, pixel))
            .max()
            .unwrap_or(0);
        if change > BODY_PAINT_CHANGE {
            break shown;
        }
        if !reported {
            let centre = window_size() / 2;
            eprintln!(
                "{}: waiting for the tooltip to paint: largest change {change}, tooltip at {:?} \
                 {:?}, window centre drawn {:?}",
                drawn_support::current_case_name(),
                tooltip_translation(&tooltip),
                tooltip.app.world().get::<InheritedVisibility>(tooltip.tip),
                window_color(&shown, centre)
            );
            reported = true;
        }
    };
    Ok((hidden, shown))
}

/// The fill of the row below the world card's control, which its tooltip never covers.
const NEIGHBOUR_ROW_FILL: Color = Color::srgb(0.1, 0.5, 0.1);
/// The world card host panel's fill around its marked card: far enough from the backdrop to
/// find the panel's edge, and light enough that the tooltip's shadow darkens it.
const WORLD_CARD_PANEL_FILL: Color = Color::srgb(0.6, 0.6, 0.6);
/// The world card control's fill, so its row can be found.
const WORLD_CARD_CONTROL_FILL: Color = Color::srgb(0.1, 0.2, 0.7);

/// Whether a pixel shows the neighbour row's green.
fn is_neighbour_row(colour: [u8; 3]) -> bool {
    let [red, green, blue] = colour.map(i16::from);
    green - red.max(blue) > 60
}

/// Whether a pixel shows the world card control's blue.
fn is_world_card_control(colour: [u8; 3]) -> bool {
    let [red, green, blue] = colour.map(i16::from);
    blue - red.max(green) > 60
}

/// Renders a world tooltip whose control sits at the right edge of a marked card padded inside
/// its host panel, with a row below it where the tooltip would fit inside the card. It opens on
/// its preferred right side, level with its control and outside the whole panel, not only the
/// card: it changes no panel or neighbour row pixel, its painted left edge, shadow included,
/// lies at least `TOOLTIP_MARGIN_PX` right of the panel, and its painted rows span the control's
/// centre row.
#[test]
fn world_tooltip_in_a_marked_card_opens_outside_its_panel_level_with_its_control()
-> Result<(), String> {
    let (hidden, shown) = capture_world_card_hidden_then_shown()?;
    let backdrop = window_color(&hidden, UVec2::ZERO);
    let panel = window_bounds(window_pixels().filter(|&pixel| {
        window_color(&hidden, pixel)
            .into_iter()
            .zip(backdrop)
            .any(|(drawn, clear)| drawn.abs_diff(clear) > BODY_PAINT_CHANGE)
    }))
    .ok_or("the world card host drew nothing")?;
    let neighbour = window_bounds(
        window_pixels().filter(|&pixel| is_neighbour_row(window_color(&hidden, pixel))),
    )
    .ok_or("the neighbour row drew nothing")?;
    let control = window_bounds(
        window_pixels().filter(|&pixel| is_world_card_control(window_color(&hidden, pixel))),
    )
    .ok_or("the control drew nothing")?;
    let painted = window_bounds(
        window_pixels()
            .filter(|&pixel| channel_change(&shown, &hidden, pixel) > UNPAINTED_TOLERANCE),
    )
    .ok_or("the tooltip painted nothing")?;
    eprintln!("panel {panel:?}, neighbour {neighbour:?}, control {control:?}, painted {painted:?}");
    if painted.left < panel.right + TOOLTIP_MARGIN_PX {
        return Err(format!(
            "the tooltip painted {painted:?} inside or within {TOOLTIP_MARGIN_PX} px of its host \
             panel {panel:?}, not outside it on its preferred right side"
        ));
    }
    let control_centre_row = u32::midpoint(control.top, control.bottom);
    if painted.top > control_centre_row || painted.bottom < control_centre_row {
        return Err(format!(
            "the tooltip painted {painted:?} off its control {control:?}'s centre row \
             {control_centre_row}, not level with it"
        ));
    }
    let covers_neighbour = painted.left <= neighbour.right
        && painted.right >= neighbour.left
        && painted.top <= neighbour.bottom
        && painted.bottom >= neighbour.top;
    if covers_neighbour {
        return Err(format!(
            "the tooltip painted {painted:?} over the neighbour row {neighbour:?}"
        ));
    }
    Ok(())
}

/// Pure red, far from the tooltip's dark fill, and with no green: the frame's grey is the only
/// green the tooltip draws over it.
const RED_HOST_FILL: Color = Color::srgb(0.6, 0.0, 0.0);
/// The share of an edge's frame ink its brightest pixel holds; a 1 px frame line split across
/// two pixels holds about two thirds.
const CRISP_EDGE_INK_SHARE: f64 = 0.9;
/// The share of the frame's brightest pixel each edge's brightest pixel reaches; a split line
/// reaches about half.
const CRISP_EDGE_PEAK_SHARE: f64 = 0.9;
/// How far a body corner may sit from whole pixels: float noise in its placement, far below the
/// tenths of a pixel a word-sized body would sit off the grid by.
const GRID_TOLERANCE_PX: f32 = 0.01;
/// Peak frame strength a drawn frame reaches over the dark overlay backdrop.
const FRAME_STRENGTH_FLOOR: f64 = 0.1;

/// How strongly a pixel shows the tooltip's grey frame: its drawn green, which the red host and
/// the black backdrop lack and the dark fill barely has.
fn frame_strength(colour: [u8; 3]) -> f64 { f64::from(colour[1]) / 255.0 }

fn physical_pixel_index(coordinate: f32) -> u32 { Vec2::splat(coordinate).as_uvec2().x }

/// A floating screen panel placed as an app places one: anchored by its top-right corner at a
/// fractional point, sized off the pixel grid, with its control in a marked column narrower
/// than the control's word tooltip. The tooltip opens to the control's left, outside the panel.
fn floating_word_tooltip_host(window: Entity) -> Result<DiegeticPanel, String> {
    floating_word_tooltip_host_with_root(
        window,
        El::column().width(Sizing::FIT).height(Sizing::FIT),
    )
}

fn floating_word_tooltip_host_with_root(
    window: Entity,
    root: El<Column>,
) -> Result<DiegeticPanel, String> {
    let mut tooltip = Tooltip::new(root)
        .show_after(Duration::ZERO)
        .source_anchor(Anchor::CenterRight)
        .target_anchor(Anchor::CenterLeft);
    tooltip.text("Attaches or detaches the card below.");
    let size = Vec2::new(167.2, 91.26);
    let mut layout = LayoutBuilder::with_root(
        El::column()
            .size(Px(size.x), Px(size.y))
            .padding(Padding::all(Px(13.3)))
            .background(RED_HOST_FILL),
    );
    layout.with(El::column().tooltip_boundary(), |layout| {
        layout.with(
            El::new()
                .size(Px(74.0), Px(24.0))
                .button("control")
                .tooltip(tooltip),
            |_| {},
        );
    });
    DiegeticPanel::screen()
        .size(Px(size.x), Px(size.y))
        .anchor(Anchor::TopRight)
        .screen_position(SHADOW_WINDOW.x - 14.67, 14.67)
        .window_entity(window)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid floating host: {error}"))
}

/// One edge of a drawn frame, read across the edge at its middle.
#[derive(Debug)]
struct FrameEdge {
    name:      &'static str,
    /// Frame strength of the five pixels from two outside the edge to two inside it.
    strengths: Vec<f64>,
}

impl FrameEdge {
    /// The brightest pixel across the edge.
    fn peak(&self) -> f64 { self.strengths.iter().copied().fold(0.0, f64::max) }

    /// The share of the edge's frame ink, its strength above the tooltip's fill, that the
    /// brightest pixel holds.
    fn peak_ink_share(&self, fill: f64) -> f64 {
        let ink = |strength: f64| (strength - fill).max(0.0);
        let total: f64 = self.strengths.iter().copied().map(ink).sum();
        if total > 0.0 {
            ink(self.peak()) / total
        } else {
            0.0
        }
    }
}

/// Renders a word tooltip as the overlay camera draws it, opened beside a floating panel's
/// control off the pixel grid. Its body lands on whole pixels on all four edges, and each edge
/// of its 1 px frame draws in one pixel at the frame's colour: across each edge's middle, the
/// brightest pixel holds `CRISP_EDGE_INK_SHARE` of the edge's ink and reaches
/// `CRISP_EDGE_PEAK_SHARE` of the frame's brightest pixel, its corners included.
#[test]
fn screen_word_tooltip_frame_draws_each_edge_in_one_pixel() -> Result<(), String> {
    let WindowCaptures { shown, body, .. } =
        capture_window_hidden_then_shown_by(floating_word_tooltip_host, WindowCamera::Overlay)?;
    let off_grid = |corner: Vec2| (corner - corner.round()).abs().max_element() > GRID_TOLERANCE_PX;
    if off_grid(body.min) || off_grid(body.max) {
        return Err(format!(
            "the tooltip body {body:?} lands off the pixel grid beside its floating host"
        ));
    }
    let first = body.min.as_uvec2();
    let last = body.max.as_uvec2() - UVec2::ONE;
    let centre = body.center().as_uvec2();
    let strength = |pixel: UVec2| frame_strength(window_color(&shown, pixel));
    let across = |edge: u32, inward: bool| -> Vec<u32> {
        (0..5)
            .map(|step| {
                if inward {
                    edge + step - 2
                } else {
                    edge + 2 - step
                }
            })
            .collect()
    };
    let edges = [
        FrameEdge {
            name:      "top",
            strengths: across(first.y, true)
                .into_iter()
                .map(|y| strength(UVec2::new(centre.x, y)))
                .collect(),
        },
        FrameEdge {
            name:      "bottom",
            strengths: across(last.y, false)
                .into_iter()
                .map(|y| strength(UVec2::new(centre.x, y)))
                .collect(),
        },
        FrameEdge {
            name:      "left",
            strengths: across(first.x, true)
                .into_iter()
                .map(|x| strength(UVec2::new(x, centre.y)))
                .collect(),
        },
        FrameEdge {
            name:      "right",
            strengths: across(last.x, false)
                .into_iter()
                .map(|x| strength(UVec2::new(x, centre.y)))
                .collect(),
        },
    ];
    let fill = edges[2].strengths[4];
    let outline = (first.x..=last.x)
        .flat_map(|x| [UVec2::new(x, first.y), UVec2::new(x, last.y)])
        .chain((first.y..=last.y).flat_map(|y| [UVec2::new(first.x, y), UVec2::new(last.x, y)]));
    let brightest = outline.map(strength).fold(0.0, f64::max);
    eprintln!("frame edges {edges:?}, fill {fill:.3}, brightest {brightest:.3}, body {body:?}");
    if brightest < FRAME_STRENGTH_FLOOR {
        return Err(format!(
            "the tooltip drew no frame: edges {edges:?}, body {body:?}"
        ));
    }
    let soft: Vec<_> = edges
        .iter()
        .filter(|edge| {
            edge.peak_ink_share(fill) < CRISP_EDGE_INK_SHARE
                || edge.peak() < CRISP_EDGE_PEAK_SHARE * brightest
        })
        .map(|edge| {
            (
                edge.name,
                edge.peak_ink_share(fill),
                edge.peak() / brightest,
            )
        })
        .collect();
    if soft.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "frame edges (name, peak ink share, peak / brightest) {soft:?} spread over more than \
             one pixel: edges {edges:?}, fill {fill:.3}, brightest {brightest:.3}, body {body:?}"
        ))
    }
}

#[test]
fn authored_two_pixel_tooltip_border_draws_two_pixels() -> Result<(), String> {
    let palette = ControlPalette::system_dark();
    let WindowCaptures { shown, body, .. } = capture_window_hidden_then_shown_by(
        |window| {
            floating_word_tooltip_host_with_root(
                window,
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .border(Border::all(Px(2.0), palette.tooltip_border)),
            )
        },
        WindowCamera::Overlay,
    )?;
    let x = physical_pixel_index(body.center().x);
    let y = physical_pixel_index(body.min.y);
    let strengths: Vec<_> = (y..=y + 2)
        .map(|row| frame_strength(window_color(&shown, UVec2::new(x, row))))
        .collect();
    if strengths[0] < 0.35 || strengths[1] < 0.35 || strengths[2] >= 0.35 {
        return Err(format!(
            "authored two-pixel top border drew {strengths:?} at body {body:?}"
        ));
    }
    Ok(())
}

/// The Apply tooltip's world host: 380 layout pixels across a 1 m face, with the control
/// near its right side. The camera below projects one layout pixel to about 1.5 window pixels.
fn apply_world_tooltip_host_with_root_and_text(
    root: El<Column>,
    label: &'static str,
) -> Result<DiegeticPanel, String> {
    let mut help = Tooltip::new(root)
        .with_label_size(Pt(11.0))
        .show_after(Duration::ZERO)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    help.text(label);
    let mut layout = LayoutBuilder::with_root(
        El::column()
            .size(Px(380.0), Px(200.0))
            .background(RED_HOST_FILL),
    );
    layout.with(El::row().size(Px(380.0), Px(70.0)), |row| {
        row.with(El::new().size(Px(325.0), Px(70.0)), |_| {});
        row.with(
            El::new()
                .size(Px(40.0), Px(24.0))
                .button("control")
                .tooltip(help),
            |_| {},
        );
    });
    DiegeticPanel::world()
        .size(Px(380.0), Px(200.0))
        .world_width(0.99)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid Apply-like host: {error}"))
}

/// The Apply-like control at the right of a marked world panel, with a taller control directly
/// below it and enough window space for the tooltip to move to its right.
fn apply_world_tooltip_host_with_neighbour() -> Result<DiegeticPanel, String> {
    let mut help = Tooltip::new(El::column().width(Sizing::FIT).height(Sizing::FIT))
        .with_label_size(Pt(11.0))
        .show_after(Duration::ZERO)
        .source_anchor(Anchor::TopCenter)
        .target_anchor(Anchor::BottomCenter);
    help.text("Reports each press in the readout.");
    let mut layout = LayoutBuilder::with_root(
        El::column()
            .size(Px(380.0), Px(200.0))
            .background(RED_HOST_FILL),
    );
    layout.with(
        El::column().size(Px(333.0), Px(80.0)).tooltip_boundary(),
        |card| {
            card.with(El::row().size(Px(333.0), Px(24.0)), |row| {
                row.with(El::new().size(Px(268.0), Px(24.0)), |_| {});
                row.with(
                    El::new()
                        .size(Px(65.0), Px(24.0))
                        .background(WORLD_CARD_CONTROL_FILL)
                        .button("control")
                        .tooltip(help),
                    |_| {},
                );
            });
            card.with(El::new().size(Px(333.0), Px(8.0)), |_| {});
            card.with(El::row().size(Px(333.0), Px(28.0)), |row| {
                row.with(El::new().size(Px(259.0), Px(28.0)), |_| {});
                row.with(
                    El::new()
                        .size(Px(74.0), Px(28.0))
                        .background(NEIGHBOUR_ROW_FILL)
                        .button("neighbour"),
                    |_| {},
                );
            });
        },
    );
    DiegeticPanel::world()
        .size(Px(380.0), Px(200.0))
        .world_width(0.99)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid Apply-like landing host: {error}"))
}

/// A metre-authored world panel carrying a fitted tooltip like Hana's jack help.
fn jack_world_tooltip_host() -> Result<DiegeticPanel, String> {
    let mut help = Tooltip::new(El::column().width(Sizing::FIT).height(Sizing::FIT))
        .with_label_size(Mm(7.0))
        .font_unit(Unit::Meters)
        .show_after(Duration::ZERO)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    help.text("Patch dragging now");
    let mut layout =
        LayoutBuilder::with_root(El::column().size(0.10, 0.06).background(RED_HOST_FILL));
    layout.with(El::row().size(0.10, 0.03), |row| {
        row.with(El::new().size(0.06, 0.03), |_| {});
        row.with(
            El::new().size(0.02, 0.02).button("control").tooltip(help),
            |_| {},
        );
    });
    DiegeticPanel::world()
        .font_unit(Unit::Meters)
        .size(0.10, 0.06)
        .world_width(0.10)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid jack-like host: {error}"))
}

/// Captures the world tooltip in the widgets example's 1280 x 720, scale-one view. Its own
/// window camera drives placement; the image camera uses the same pose and projection.
fn capture_apply_world_tooltip() -> Result<(Vec<u8>, Rect, [Vec2; 4], f32, f32), String> {
    capture_apply_world_tooltip_with_root(
        El::column().width(Sizing::FIT).height(Sizing::FIT),
        CameraZoom::Initial,
        ControlAnchorDepth::PanelPlane,
        HostOrientation::ViewAligned,
    )
}

fn show_while_camera_moves(
    shown: &mut ShownTooltip,
    camera: Entity,
    pose: &mut Transform,
) -> Result<(), String> {
    shown.app.world_mut().trigger(RequestWidgetFocus {
        window: shown.window,
        widget: shown.control,
    });
    for _ in 0..30 {
        pose.translation.z += 0.0025;
        shown.app.world_mut().entity_mut(camera).insert(*pose);
        shown.app.update();
    }
    if shown.app.world().get::<Visibility>(shown.tip) == Some(&Visibility::Hidden) {
        return Err("help never reveals while its camera moves".to_owned());
    }
    for _ in 0..30 {
        shown.app.update();
    }
    Ok(())
}

fn capture_apply_world_tooltip_with_root(
    root: El<Column>,
    camera_zoom: CameraZoom,
    control_anchor_depth: ControlAnchorDepth,
    host_orientation: HostOrientation,
) -> Result<(Vec<u8>, Rect, [Vec2; 4], f32, f32), String> {
    capture_apply_world_tooltip_case(
        root,
        "Reports each press in the readout.",
        camera_zoom,
        control_anchor_depth,
        host_orientation,
        match control_anchor_depth {
            ControlAnchorDepth::PanelPlane => 2.0203,
            ControlAnchorDepth::InFront => 1.8,
        },
    )
}

fn capture_apply_world_tooltip_case(
    root: El<Column>,
    label: &'static str,
    camera_zoom: CameraZoom,
    control_anchor_depth: ControlAnchorDepth,
    host_orientation: HostOrientation,
    camera_z: f32,
) -> Result<(Vec<u8>, Rect, [Vec2; 4], f32, f32), String> {
    let mut app = CaseApp::new();
    let size = UVec2::new(1280, 720);
    let mut window = Window::default();
    window.resolution.set(size.as_vec2().x, size.as_vec2().y);
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    let mut camera_pose = Transform::from_xyz(0.2511, 0.6734, camera_z);
    let window_camera = spawn_apply_window_camera(&mut app, window, camera_pose);
    let host = app
        .world_mut()
        .spawn((
            apply_world_tooltip_host_with_root_and_text(root, label)?,
            Transform::from_xyz(0.055, 0.5775, 0.507).with_rotation(match host_orientation {
                HostOrientation::ViewAligned => Quat::IDENTITY,
                HostOrientation::Tilted => {
                    Quat::from_rotation_y(0.18) * Quat::from_rotation_z(0.11)
                },
            }),
        ))
        .id();
    app.settle()?;
    let control = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(host, &PanelElementId::named("control"))
        })
        .map_err(|error| format!("widget query failed: {error}"))?
        .ok_or_else(|| "host has no Apply control".to_string())?;
    if matches!(control_anchor_depth, ControlAnchorDepth::InFront) {
        offset_control_anchor_geometry(&mut app, control)?;
    }
    let tip = app
        .world()
        .get::<Tooltips>(control)
        .and_then(|controllers| controllers.iter().next())
        .ok_or_else(|| "Apply control has no tooltip".to_string())?;
    let mut shown = ShownTooltip {
        app,
        window,
        control,
        tip,
    };
    if matches!(camera_zoom, CameraZoom::DuringReveal) {
        show_while_camera_moves(&mut shown, window_camera, &mut camera_pose)?;
    } else {
        set_tooltip_shown(&mut shown, true)?;
    }
    if matches!(control_anchor_depth, ControlAnchorDepth::InFront) {
        assert_front_anchor_world_depth(&shown)?;
    }
    if matches!(camera_zoom, CameraZoom::AfterMaterialization) {
        zoom_apply_tooltip_and_show_again(&mut shown, window_camera, &mut camera_pose)?;
    }
    let size = UVec2::new(1280, 720);
    let target = shown
        .app
        .target(CaptureRegion::new(size, UVec2::ZERO, size));
    shown.app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Image(target.clone().into()),
        Msaa::Off,
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        camera_pose,
    ));
    let source_grid_alignment = match (camera_zoom, control_anchor_depth) {
        (CameraZoom::Initial, ControlAnchorDepth::PanelPlane) => SourceGridAlignment::OffGrid,
        _ => SourceGridAlignment::Unchecked,
    };
    let (body, edge_centres, screen_per_layout_px, view_alignment) =
        projected_apply_tooltip_body(&shown, window_camera, host, control, source_grid_alignment)?;
    let pixels = capture_after_frames(&mut shown.app, target)?;
    Ok((
        pixels,
        body,
        edge_centres,
        screen_per_layout_px,
        view_alignment,
    ))
}

fn capture_jack_world_tooltip(
    camera_z: f32,
) -> Result<(Vec<u8>, Rect, [Vec2; 4], f32, f32), String> {
    let mut app = CaseApp::new();
    let size = UVec2::new(1280, 720);
    let mut window = Window::default();
    window.resolution.set(size.as_vec2().x, size.as_vec2().y);
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    let camera_pose = Transform::from_xyz(0.041, 0.019, camera_z);
    let window_camera = spawn_apply_window_camera(&mut app, window, camera_pose);
    let host = app
        .world_mut()
        .spawn((jack_world_tooltip_host()?, Transform::IDENTITY))
        .id();
    app.settle()?;
    let control = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(host, &PanelElementId::named("control"))
        })
        .map_err(|error| format!("widget query failed: {error}"))?
        .ok_or_else(|| "host has no jack control".to_string())?;
    let tip = app
        .world()
        .get::<Tooltips>(control)
        .and_then(|controllers| controllers.iter().next())
        .ok_or_else(|| "jack control has no tooltip".to_string())?;
    let mut shown = ShownTooltip {
        app,
        window,
        control,
        tip,
    };
    set_tooltip_shown(&mut shown, true)?;
    let target = shown
        .app
        .target(CaptureRegion::new(size, UVec2::ZERO, size));
    shown.app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Image(target.clone().into()),
        Msaa::Off,
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        camera_pose,
    ));
    let (body, edge_centres, pixels_per_metre, view_alignment) = projected_apply_tooltip_body(
        &shown,
        window_camera,
        host,
        control,
        SourceGridAlignment::Unchecked,
    )?;
    let pixels = capture_after_frames(&mut shown.app, target)?;
    Ok((pixels, body, edge_centres, pixels_per_metre, view_alignment))
}

fn assert_front_anchor_world_depth(shown: &ShownTooltip) -> Result<(), String> {
    let world = shown.app.world();
    let target = world
        .get::<GlobalTransform>(shown.control)
        .ok_or_else(|| "front anchor target has no transform".to_string())?;
    let geometry = world
        .get::<ResolvedAnchorGeometry>(shown.control)
        .ok_or_else(|| "front anchor target lost its geometry".to_string())?;
    let anchor = geometry
        .frame(AnchorSite::EdgeMidpoint(1))
        .map_err(|error| format!("front anchor has no right site: {error}"))?;
    let anchor_world = target.transform_point(anchor.position().into_inner());
    let body = world
        .get::<GlobalTransform>(shown.tip)
        .ok_or_else(|| "front anchor tooltip has no transform".to_string())?;
    eprintln!(
        "front anchor depths: target {}, anchor {}, tooltip {}",
        target.translation().z,
        anchor_world.z,
        body.translation().z
    );
    if anchor_world.z - target.translation().z < 0.005 {
        return Err("front anchor fixture did not project ahead of target".to_string());
    }
    if body.translation().z - anchor_world.z < 0.0049 {
        return Err("front anchor tooltip did not draw past the anchor".to_string());
    }
    Ok(())
}

fn projected_apply_tooltip_body(
    shown: &ShownTooltip,
    window_camera: Entity,
    host: Entity,
    control: Entity,
    source_grid_alignment: SourceGridAlignment,
) -> Result<(Rect, [Vec2; 4], f32, f32), String> {
    let panel = shown
        .app
        .world()
        .get::<DiegeticPanel>(shown.tip)
        .ok_or_else(|| "Apply tooltip has no panel".to_string())?;
    let transform = shown
        .app
        .world()
        .get::<GlobalTransform>(shown.tip)
        .ok_or_else(|| "Apply tooltip has no world transform".to_string())?;
    let plane = PanelPlane::from_panel(panel, transform)
        .map_err(|error| format!("Apply tooltip has no plane: {error}"))?;
    let camera = shown
        .app
        .world()
        .get::<Camera>(window_camera)
        .ok_or_else(|| "Apply window has no camera".to_string())?;
    let camera_transform = shown
        .app
        .world()
        .get::<GlobalTransform>(window_camera)
        .ok_or_else(|| "Apply camera has no world transform".to_string())?;
    if matches!(source_grid_alignment, SourceGridAlignment::OffGrid) {
        assert_apply_sources_off_grid(&shown.app, camera, camera_transform, host, control)?;
    }
    let project = |anchor| {
        camera
            .world_to_viewport(camera_transform, plane.point(anchor))
            .map_err(|error| format!("cannot project Apply tooltip: {error}"))
    };
    let corners = [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::BottomLeft,
    ]
    .map(project)
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let body = Rect::from_corners(
        corners
            .iter()
            .copied()
            .fold(Vec2::splat(f32::INFINITY), Vec2::min),
        corners
            .iter()
            .copied()
            .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max),
    );
    let edge_centres = [
        project(Anchor::TopCenter)?,
        project(Anchor::BottomCenter)?,
        project(Anchor::CenterLeft)?,
        project(Anchor::CenterRight)?,
    ];
    let top_width = project(Anchor::TopRight)?.x - body.min.x;
    let screen_per_layout_px = top_width / panel.width();
    let normal = transform.affine().transform_vector3(Vec3::Z).normalize();
    let view_alignment = normal.dot(*camera_transform.forward()).abs();
    Ok((body, edge_centres, screen_per_layout_px, view_alignment))
}

struct NeighbourLanding {
    control:   Rect,
    neighbour: Rect,
    tooltip:   Rect,
}

fn capture_world_tooltip_neighbour_landing() -> Result<NeighbourLanding, String> {
    let mut app = CaseApp::new();
    let size = UVec2::new(1280, 720);
    let mut window = Window::default();
    window.resolution.set(size.as_vec2().x, size.as_vec2().y);
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    let camera_pose = Transform::from_xyz(0.2511, 0.6734, 2.0203);
    let window_camera = spawn_apply_window_camera(&mut app, window, camera_pose);
    let cube = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 1.0, 1.0));
    let cube_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.world_mut().spawn((
        Mesh3d(cube),
        MeshMaterial3d(cube_material),
        Transform::from_xyz(0.0, 0.6, 0.0),
    ));
    let side_panel = DiegeticPanel::world()
        .size(Px(456.0), Px(456.0))
        .world_width(1.0)
        .anchor(Anchor::TopLeft)
        .with_tree(LayoutBuilder::new(456.0, 456.0).build())
        .build()
        .map_err(|error| format!("invalid right-face panel: {error}"))?;
    app.world_mut().spawn((
        side_panel,
        Transform::from_xyz(0.501, 1.1, 0.5)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
    ));
    let host = app
        .world_mut()
        .spawn((
            apply_world_tooltip_host_with_neighbour()?,
            Transform::from_xyz(0.022, 0.5775, 0.507),
        ))
        .id();
    app.settle()?;
    let control = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(host, &PanelElementId::named("control"))
        })
        .map_err(|error| format!("widget query failed: {error}"))?;
    let control = control.ok_or_else(|| "host has no Apply control".to_string())?;
    let tip = app
        .world()
        .get::<Tooltips>(control)
        .and_then(|controllers| controllers.iter().next())
        .ok_or_else(|| "Apply control has no tooltip".to_string())?;
    let mut shown = ShownTooltip {
        app,
        window,
        control,
        tip,
    };
    set_tooltip_shown(&mut shown, true)?;
    let (tooltip, _, scale, _) = projected_apply_tooltip_body(
        &shown,
        window_camera,
        host,
        control,
        SourceGridAlignment::Unchecked,
    )?;
    let control = projected_anchor_geometry_bounds(&shown.app, window_camera, control)?;
    let neighbour_size = Vec2::new(74.0, 28.0) * scale;
    let neighbour_min = Vec2::new(
        control.max.x - neighbour_size.x,
        32.0f32.mul_add(scale, control.min.y),
    );
    let neighbour = Rect::from_corners(neighbour_min, neighbour_min + neighbour_size);
    Ok(NeighbourLanding {
        control,
        neighbour,
        tooltip,
    })
}

#[test]
fn world_tooltip_lands_clear_of_a_taller_neighbour_control_below() -> Result<(), String> {
    let NeighbourLanding {
        control,
        neighbour,
        tooltip,
    } = capture_world_tooltip_neighbour_landing()?;
    eprintln!("control {control:?}, neighbour {neighbour:?}, tooltip {tooltip:?}");
    let covers_neighbour = tooltip.min.x < neighbour.max.x
        && tooltip.max.x > neighbour.min.x
        && tooltip.min.y < neighbour.max.y
        && tooltip.max.y > neighbour.min.y;
    if covers_neighbour {
        return Err(format!(
            "the tooltip {tooltip:?} covers part of the neighbouring control {neighbour:?}"
        ));
    }
    Ok(())
}

fn projected_anchor_geometry_bounds(
    app: &CaseApp,
    camera: Entity,
    target: Entity,
) -> Result<Rect, String> {
    let geometry = app
        .world()
        .get::<ResolvedAnchorGeometry>(target)
        .ok_or_else(|| format!("target {target:?} has no anchor geometry"))?;
    let target_transform = app
        .world()
        .get::<GlobalTransform>(target)
        .ok_or_else(|| format!("target {target:?} has no world transform"))?;
    let camera_component = app
        .world()
        .get::<Camera>(camera)
        .ok_or_else(|| "Apply window has no camera".to_string())?;
    let camera_transform = app
        .world()
        .get::<GlobalTransform>(camera)
        .ok_or_else(|| "Apply camera has no world transform".to_string())?;
    let corners = [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::BottomLeft,
    ]
    .map(|anchor| {
        let frame = geometry
            .frame(AnchorSite::from(anchor))
            .map_err(|error| format!("target {target:?} has no {anchor:?} frame: {error}"))?;
        camera_component
            .world_to_viewport(
                camera_transform,
                target_transform.transform_point(frame.position().into_inner()),
            )
            .map_err(|error| format!("cannot project target {target:?}: {error}"))
    })
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    Ok(Rect::from_corners(
        corners
            .iter()
            .copied()
            .fold(Vec2::splat(f32::INFINITY), Vec2::min),
        corners
            .iter()
            .copied()
            .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max),
    ))
}

fn offset_control_anchor_geometry(app: &mut CaseApp, control: Entity) -> Result<(), String> {
    let half = Vec2::new(40.0, 24.0) * (0.99 / 380.0) * 0.5;
    let control_global = app
        .world()
        .get::<GlobalTransform>(control)
        .ok_or_else(|| "Apply control has no global transform".to_string())?;
    let local_depth = 0.0056 / control_global.affine().transform_vector3(Vec3::Z).length();
    let positions = [
        (AnchorSite::Vertex(0), Vec2::new(-half.x, half.y)),
        (AnchorSite::Vertex(1), Vec2::new(half.x, half.y)),
        (AnchorSite::Vertex(2), Vec2::new(half.x, -half.y)),
        (AnchorSite::Vertex(3), Vec2::new(-half.x, -half.y)),
        (AnchorSite::EdgeMidpoint(0), Vec2::new(0.0, half.y)),
        (AnchorSite::EdgeMidpoint(1), Vec2::new(half.x, 0.0)),
        (AnchorSite::EdgeMidpoint(2), Vec2::new(0.0, -half.y)),
        (AnchorSite::EdgeMidpoint(3), Vec2::new(-half.x, 0.0)),
        (AnchorSite::Center, Vec2::ZERO),
    ];
    let frames = positions
        .map(|(site, xy)| {
            AnchorFrame::try_new(Position::from(xy.extend(local_depth)), default())
                .map(|frame| (site, frame))
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot shift Apply anchor: {error}"))?;
    let shifted = ResolvedAnchorGeometry::try_new(frames, [])
        .map_err(|error| format!("cannot rebuild Apply anchor geometry: {error}"))?;
    app.world_mut().entity_mut(control).insert(shifted);
    Ok(())
}

fn zoom_apply_tooltip_and_show_again(
    shown: &mut ShownTooltip,
    camera: Entity,
    pose: &mut Transform,
) -> Result<(), String> {
    set_tooltip_shown(shown, false)?;
    pose.translation.z -= 0.27;
    *shown
        .app
        .world_mut()
        .get_mut::<Transform>(camera)
        .ok_or_else(|| "Apply camera lost its pose".to_string())? = *pose;
    set_tooltip_shown(shown, true)
}

fn spawn_apply_window_camera(app: &mut CaseApp, window: Entity, pose: Transform) -> Entity {
    app.world_mut()
        .spawn((
            Camera3d::default(),
            RenderTarget::Window(WindowRef::Entity(window)),
            Msaa::Off,
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            pose,
        ))
        .id()
}

fn assert_apply_sources_off_grid(
    app: &CaseApp,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    host: Entity,
    control: Entity,
) -> Result<(), String> {
    for (name, entity) in [("host", host), ("control", control)] {
        let source = app
            .world()
            .get::<GlobalTransform>(entity)
            .ok_or_else(|| format!("Apply {name} has no world transform"))?;
        let screen = camera
            .world_to_viewport(camera_transform, source.translation())
            .map_err(|error| format!("cannot project Apply {name}: {error}"))?;
        eprintln!("Apply {name} source projects to {screen:?}");
        assert!(
            [screen.x, screen.y]
                .into_iter()
                .any(|edge| (edge - edge.round()).abs() >= 0.1),
            "Apply {name} source is already on the physical pixel grid: {screen:?}"
        );
    }
    Ok(())
}

/// A world tooltip at the Apply button's screen scale keeps all four 1 px frame strokes in one
/// screen pixel, with equally bright sides and body edges on the physical pixel grid.
#[test]
fn world_word_tooltip_frame_draws_each_edge_in_one_pixel() -> Result<(), String> {
    let (shown, body, edge_centres, scale, alignment) = capture_apply_world_tooltip()?;
    let size = UVec2::new(1280, 720);
    let strength = |x: u32, y: u32| {
        let index = ((y * size.x + x) * 4) as usize;
        frame_strength([shown[index], shown[index + 1], shown[index + 2]])
    };
    let names = ["top", "bottom", "left", "right"];
    let edges: Vec<_> = edge_centres
        .into_iter()
        .enumerate()
        .map(|(side, point)| {
            let edge = if side < 2 { point.y } else { point.x };
            let axis = physical_pixel_index(edge.floor());
            let strengths: Vec<_> = (axis.saturating_sub(2)..=axis + 2)
                .map(|pixel| {
                    if side < 2 {
                        strength(physical_pixel_index(point.x.round()), pixel)
                    } else {
                        strength(pixel, physical_pixel_index(point.y.round()))
                    }
                })
                .collect();
            (names[side], edge.fract(), strengths)
        })
        .collect();
    let centre = body.center().as_uvec2();
    let fill = strength(physical_pixel_index(body.min.x.floor()) + 4, centre.y);
    let frame: Vec<_> = edges
        .iter()
        .map(|(name, fraction, strengths)| {
            let (peak_index, peak) = strengths
                .iter()
                .copied()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .ok_or_else(|| format!("{name} frame sample is empty"))?;
            let next = [peak_index.checked_sub(1), Some(peak_index + 1)]
                .into_iter()
                .flatten()
                .filter_map(|index| strengths.get(index))
                .copied()
                .fold(0.0, f64::max);
            let ink = |value: f64| (value - fill).max(0.0);
            let width = strengths.iter().copied().map(ink).sum::<f64>() / ink(peak).max(0.001);
            Ok((*name, *fraction, peak, next, width, strengths))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !(1.45..=1.55).contains(&scale) {
        return Err(format!(
            "world tooltip scale {scale:.4}, expected about 1.5 px/layout px"
        ));
    }
    if alignment < 0.999 {
        return Err(format!(
            "world tooltip plane is tilted relative to live Apply's view: alignment {alignment:.4}"
        ));
    }
    let body_edges = [body.min.x, body.min.y, body.max.x, body.max.y];
    assert!(
        body_edges
            .into_iter()
            .all(|edge| (edge - edge.round()).abs() <= 0.05),
        "world tooltip body edges miss physical pixels: {body:?}"
    );
    let brightest = frame.iter().map(|edge| edge.2).fold(0.0, f64::max);
    if brightest < FRAME_STRENGTH_FLOOR {
        return Err(format!("world tooltip frame did not draw: {frame:?}"));
    }
    let soft: Vec<_> = frame
        .iter()
        .filter(|edge| edge.2 < 3.0 * edge.3 || edge.2 < 0.85 * brightest)
        .collect();
    if soft.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "world tooltip frame spreads across pixels or has unequal edges: {soft:?}"
        ))
    }
}

/// A parallel metre-unit world tooltip keeps its frame crisp when one design pixel projects to
/// less than one physical pixel, and remains finite at a much smaller positive scale.
#[test]
fn subpixel_scale_world_tooltip_frame_draws_each_edge_in_one_pixel() -> Result<(), String> {
    const LABEL_MILLIMETRES: f32 = 7.0;
    const DEFAULT_LABEL_DESIGN_PIXELS: f32 = 11.0 * 4.0 / 3.0;
    let design_pixel_metres = LABEL_MILLIMETRES / DEFAULT_LABEL_DESIGN_PIXELS / 1000.0;
    let (shown, body, edge_centres, pixels_per_metre, alignment) =
        capture_jack_world_tooltip(0.448)?;
    let scale = pixels_per_metre * design_pixel_metres;
    if !(0.91..=0.97).contains(&scale) {
        return Err(format!(
            "jack tooltip scale {scale:.4}, expected about 0.9375 physical px/design px"
        ));
    }
    if alignment < 0.999 {
        return Err(format!(
            "jack tooltip plane is tilted relative to the view: alignment {alignment:.4}"
        ));
    }
    let body_edges = [body.min.x, body.max.x, body.min.y, body.max.y];
    let grid_errors = body_edges.map(|edge| (edge - edge.round()).abs());
    let body_misses_grid = grid_errors.into_iter().any(|error| error > 0.05);

    let size = UVec2::new(1280, 720);
    let strength = |x: u32, y: u32| {
        let index = ((y * size.x + x) * 4) as usize;
        frame_strength([shown[index], shown[index + 1], shown[index + 2]])
    };
    let centre = body.center().as_uvec2();
    let fill = strength(physical_pixel_index(body.min.x.floor()) + 4, centre.y);
    let names = ["top", "bottom", "left", "right"];
    let widths = edge_centres
        .into_iter()
        .enumerate()
        .map(|(side, point)| {
            let edge = if side < 2 { point.y } else { point.x };
            let axis = physical_pixel_index(edge.floor());
            let strengths: Vec<_> = (axis.saturating_sub(2)..=axis + 2)
                .map(|pixel| {
                    if side < 2 {
                        strength(physical_pixel_index(point.x.round()), pixel)
                    } else {
                        strength(pixel, physical_pixel_index(point.y.round()))
                    }
                })
                .collect();
            let peak = strengths.iter().copied().fold(0.0, f64::max);
            let ink = |value: f64| (value - fill).max(0.0);
            let width = strengths.iter().copied().map(ink).sum::<f64>() / ink(peak).max(0.001);
            (names[side], width, peak, strengths)
        })
        .collect::<Vec<_>>();
    let narrowest = widths
        .iter()
        .map(|edge| edge.1)
        .fold(f64::INFINITY, f64::min);
    let widest = widths.iter().map(|edge| edge.1).fold(0.0, f64::max);
    eprintln!(
        "jack frame body {body:?}, scale {scale:.4}, grid errors {grid_errors:?}, widths {widths:?}"
    );
    if body_misses_grid
        || widths.iter().any(|edge| edge.2 < FRAME_STRENGTH_FLOOR)
        || widest > 1.15
        || widest > narrowest * 1.15
    {
        return Err(format!(
            "jack tooltip frame misses the grid or is soft: body {body:?}, errors {grid_errors:?}, widths {widths:?}, fill {fill:.3}"
        ));
    }

    let (_, tiny_body, _, tiny_pixels_per_metre, _) = capture_jack_world_tooltip(2.08)?;
    let tiny_scale = tiny_pixels_per_metre * design_pixel_metres;
    if !(0.18..=0.22).contains(&tiny_scale)
        || !tiny_body.min.is_finite()
        || !tiny_body.max.is_finite()
        || tiny_body.width() <= 0.0
        || tiny_body.height() <= 0.0
    {
        return Err(format!(
            "jack tooltip at scale {tiny_scale:.4} has an invalid body {tiny_body:?}"
        ));
    }
    Ok(())
}

fn drawn_world_body_edges_on_grid(
    pixels: &[u8],
    body: Rect,
    _edge_centres: [Vec2; 4],
) -> Result<(), String> {
    let size = UVec2::new(1280, 720);
    let pixel = body.center().as_uvec2().min(size - UVec2::ONE);
    let index = ((pixel.y * size.x + pixel.x) * 4) as usize;
    if frame_strength([pixels[index], pixels[index + 1], pixels[index + 2]]) < 0.05 {
        return Err(format!(
            "world tooltip did not paint at its centre, body {body:?}"
        ));
    }
    let edges = [body.min.x, body.min.y, body.max.x, body.max.y];
    if edges.iter().any(|edge| (edge - edge.round()).abs() > 0.05) {
        return Err(format!("world body edges miss physical pixels: {body:?}"));
    }
    Ok(())
}

#[test]
fn front_anchor_world_tooltip_draws_all_edges_on_pixels() -> Result<(), String> {
    for camera_z in [1.55, 1.65, 1.75, 1.85] {
        let (pixels, body, centres, _, _) = capture_apply_world_tooltip_case(
            El::column()
                .width(Sizing::Fixed(Px(101.0).into()))
                .height(Sizing::FIT),
            "Hi",
            CameraZoom::Initial,
            ControlAnchorDepth::InFront,
            HostOrientation::ViewAligned,
            camera_z,
        )?;
        drawn_world_body_edges_on_grid(&pixels, body, centres)
            .map_err(|error| format!("camera z={camera_z}: {error}"))?;
    }
    Ok(())
}

#[test]
fn fixed_width_world_tooltip_draws_all_edges_on_pixels() -> Result<(), String> {
    let (pixels, body, centres, _, _) = capture_apply_world_tooltip_case(
        El::column()
            .width(Sizing::Fixed(Px(101.0).into()))
            .height(Sizing::FIT),
        "Hi",
        CameraZoom::Initial,
        ControlAnchorDepth::PanelPlane,
        HostOrientation::ViewAligned,
        2.0203,
    )?;
    drawn_world_body_edges_on_grid(&pixels, body, centres)
}

#[test]
fn zoomed_world_tooltip_rebuilds_frame_on_pixel_grid() -> Result<(), String> {
    let (pixels, body, centres, _, _) = capture_apply_world_tooltip_with_root(
        El::column().width(Sizing::FIT).height(Sizing::FIT),
        CameraZoom::AfterMaterialization,
        ControlAnchorDepth::PanelPlane,
        HostOrientation::ViewAligned,
    )?;
    drawn_world_body_edges_on_grid(&pixels, body, centres)
}

#[test]
fn moving_camera_world_tooltip_reveals_then_settles_on_the_pixel_grid() -> Result<(), String> {
    let (pixels, body, centres, _, _) = capture_apply_world_tooltip_with_root(
        El::column().width(Sizing::FIT).height(Sizing::FIT),
        CameraZoom::DuringReveal,
        ControlAnchorDepth::PanelPlane,
        HostOrientation::ViewAligned,
    )?;
    drawn_world_body_edges_on_grid(&pixels, body, centres)
}

#[test]
fn tilted_world_tooltip_edge_keeps_partial_coverage() -> Result<(), String> {
    let (pixels, body, centres, _, alignment) = capture_apply_world_tooltip_with_root(
        El::column().width(Sizing::FIT).height(Sizing::FIT),
        CameraZoom::Initial,
        ControlAnchorDepth::PanelPlane,
        HostOrientation::Tilted,
    )?;
    if alignment >= 0.9999 {
        return Err(format!("tilted fixture stayed view aligned: {alignment}"));
    }
    let left = centres[2];
    let size = UVec2::new(1280, 720);
    let strength = |x: u32, y: u32| {
        let index = ((y * size.x + x) * 4) as usize;
        frame_strength([pixels[index], pixels[index + 1], pixels[index + 2]])
    };
    let left_x = physical_pixel_index(left.x);
    let left_y = physical_pixel_index(left.y);
    let mut partial_rows = 0;
    let mut transition_min = f64::INFINITY;
    let mut transition_max = f64::NEG_INFINITY;
    for y in left_y.saturating_sub(8)..=left_y + 8 {
        let first = left_x.saturating_sub(5)..=left_x + 5;
        let edge = first.into_iter().find(|&x| strength(x, y) > 0.55);
        if let Some(x) = edge
            && (0.16..0.52).contains(&strength(x + 1, y))
            && strength(x + 2, y) < 0.15
        {
            partial_rows += 1;
            transition_min = transition_min.min(strength(x + 1, y));
            transition_max = transition_max.max(strength(x + 1, y));
        }
    }
    let transition_spread = transition_max - transition_min;
    if partial_rows < 4 || transition_spread < 0.2 {
        let rows: Vec<_> = (left_y.saturating_sub(8)..=left_y + 8)
            .step_by(2)
            .map(|y| {
                (
                    y,
                    (left_x.saturating_sub(5)..=left_x + 5)
                        .map(|x| strength(x, y))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        return Err(format!(
            "tilted edge has hard stairs: {partial_rows} partially covered rows, transition spread {transition_spread:.3}, body {body:?}, samples {rows:?}"
        ));
    }
    Ok(())
}

/// `DEFAULT_TOOLTIP_GAP` (`widgets/constants.rs`): the gap a tooltip keeps from its control,
/// in logical pixels.
const TOOLTIP_GAP_PX: f64 = 8.0;
/// Dark, so the tooltip's shadow moves it by less than `BODY_PAINT_CHANGE`.
const GAP_CONTROL_FILL: Color = Color::srgb(0.1, 0.1, 0.4);
/// A control this far into its host leaves the tooltip room on every side.
const ROOMY_LEAD_PX: f32 = 140.0;
/// A control this near its host's left edge leaves the tooltip no room on its left.
const CRAMPED_LEAD_PX: f32 = 10.0;

/// A light screen host, centred in the window, whose dark control sits `lead` pixels in from
/// the host's left edge and halfway down, its tooltip anchored on `placement`'s side with the
/// default gap.
fn gap_host(placement: Placement, lead: f32, window: Entity) -> Result<DiegeticPanel, String> {
    let (source, target) = placement.anchors();
    let tooltip = Tooltip::new(El::column().size(Px(90.0), Px(24.0)))
        .show_after(Duration::ZERO)
        .source_anchor(source)
        .target_anchor(target);
    let mut layout = LayoutBuilder::with_root(
        El::row()
            .size(Px(320.0), Px(160.0))
            .padding(Padding::new(Px(lead), Px(0.0), Px(70.0), Px(0.0)))
            .background(SHADOW_HOST_FILL),
    );
    layout.with(
        El::new()
            .size(Px(40.0), Px(20.0))
            .background(GAP_CONTROL_FILL)
            .button("control")
            .tooltip(tooltip),
        |_| {},
    );
    DiegeticPanel::screen()
        .size(Px(320.0), Px(160.0))
        .anchor(Anchor::Center)
        .screen_position(0.5 * SHADOW_WINDOW.x, 0.5 * SHADOW_WINDOW.y)
        .window_entity(window)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid gap host: {error}"))
}

/// Whether a pixel shows the gap control's dark blue.
fn is_gap_control(colour: [u8; 3]) -> bool {
    let [red, green, blue] = colour.map(i16::from);
    blue - red.max(green) > 40
}

/// The drawn gap between a tooltip's body on `side` of its control and the control, and how
/// far the body's centre sits from the control's across that side, in pixels.
fn drawn_gap(side: Placement, body: &PixelBounds, control: &PixelBounds) -> (f64, f64) {
    let gap = |near: u32, far: u32| f64::from(far) - f64::from(near) - 1.0;
    let centre = |start: u32, end: u32| f64::midpoint(f64::from(start), f64::from(end));
    let rows = centre(body.top, body.bottom) - centre(control.top, control.bottom);
    let columns = centre(body.left, body.right) - centre(control.left, control.right);
    match side {
        Placement::Left => (gap(body.right, control.left), rows),
        Placement::Right => (gap(control.right, body.left), rows),
        Placement::Above => (gap(body.bottom, control.top), columns),
        Placement::Below => (gap(control.bottom, body.top), columns),
    }
}

/// Renders the gap host's control `lead` pixels into its host, with its tooltip hidden, then
/// shown with `preferred`'s anchors. The drawn body lands on `landed`'s side,
/// `DEFAULT_TOOLTIP_GAP` from the control's drawn edge within a pixel, with its centre within
/// a pixel of the control's along that edge.
fn keeps_its_gap(preferred: Placement, lead: f32, landed: Placement) -> Result<(), String> {
    let WindowCaptures { hidden, shown, .. } =
        capture_window_hidden_then_shown(|window| gap_host(preferred, lead, window))?;
    let control = window_bounds(
        window_pixels().filter(|&pixel| is_gap_control(window_color(&hidden, pixel))),
    )
    .ok_or_else(|| {
        "the hidden frame shows no control pixels: the control drew under the host's fill"
            .to_string()
    })?;
    let body = window_bounds(
        window_pixels().filter(|&pixel| channel_change(&shown, &hidden, pixel) > BODY_PAINT_CHANGE),
    )
    .ok_or_else(|| "the shown tooltip painted no body pixels".to_string())?;
    let (gap, centre_offset) = drawn_gap(landed, &body, &control);
    // Above, the 11 px downward shadow needs more room than the authored 8 px
    // body gap. The complete painted tooltip keeps 2 px clear of its source.
    let expected_gap = if matches!(landed, Placement::Above) {
        TOOLTIP_GAP_PX.max(f64::from(SHADOW_REACH_BELOW_PX) + 2.0)
    } else {
        TOOLTIP_GAP_PX
    };
    eprintln!(
        "{preferred:?} tooltip landed {landed:?}: gap {gap} px, centre offset {centre_offset} \
         px, body {body:?}, control {control:?}"
    );
    if (gap - expected_gap).abs() > 1.0 || centre_offset.abs() > 1.0 {
        return Err(format!(
            "{preferred:?} tooltip should land {landed:?} of its control, {expected_gap} px \
             clear and centred along its edge: drawn gap {gap} px, centre offset \
             {centre_offset} px; body {body:?}, control {control:?}"
        ));
    }
    Ok(())
}

fn screen_tooltip_placed_left_keeps_its_gap() -> Result<(), String> {
    keeps_its_gap(Placement::Left, ROOMY_LEAD_PX, Placement::Left)
}

fn screen_tooltip_placed_right_keeps_its_gap() -> Result<(), String> {
    keeps_its_gap(Placement::Right, ROOMY_LEAD_PX, Placement::Right)
}

fn screen_tooltip_flipped_from_left_to_right_keeps_its_gap() -> Result<(), String> {
    keeps_its_gap(Placement::Left, CRAMPED_LEAD_PX, Placement::Right)
}

fn screen_tooltip_placed_above_keeps_its_gap() -> Result<(), String> {
    keeps_its_gap(Placement::Above, ROOMY_LEAD_PX, Placement::Above)
}

#[test]
fn screen_tooltip_keeps_its_gap_from_its_control() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "screen_tooltip_placed_left_keeps_its_gap",
            run:  screen_tooltip_placed_left_keeps_its_gap,
        },
        DrawnCase {
            name: "screen_tooltip_placed_right_keeps_its_gap",
            run:  screen_tooltip_placed_right_keeps_its_gap,
        },
        DrawnCase {
            name: "screen_tooltip_flipped_from_left_to_right_keeps_its_gap",
            run:  screen_tooltip_flipped_from_left_to_right_keeps_its_gap,
        },
        DrawnCase {
            name: "screen_tooltip_placed_above_keeps_its_gap",
            run:  screen_tooltip_placed_above_keeps_its_gap,
        },
    ])
}

#[test]
fn world_panel_words_match_drawn_pixels() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "world_tooltip_words_reach_drawn_pixels",
            run:  world_tooltip_words_reach_drawn_pixels,
        },
        DrawnCase {
            name: "empty_world_tooltip_has_no_drawn_word_pixels",
            run:  empty_world_tooltip_has_no_drawn_word_pixels,
        },
        DrawnCase {
            name: "plain_world_panel_words_reach_drawn_pixels",
            run:  plain_world_panel_words_reach_drawn_pixels,
        },
        DrawnCase {
            name: "repeated_world_panel_words_upload_cold_glyph_atlas",
            run:  repeated_world_panel_words_upload_cold_glyph_atlas,
        },
        DrawnCase {
            name: "fitted_world_help_has_even_drawn_padding_and_a_readable_cap",
            run:  fitted_world_help_has_even_drawn_padding_and_a_readable_cap,
        },
        DrawnCase {
            name: "fitted_descender_help_has_even_drawn_ink_padding",
            run:  fitted_descender_help_has_even_drawn_ink_padding,
        },
        DrawnCase {
            name: "fitted_help_padding_tracks_drawn_cap_at_two_sizes",
            run:  fitted_help_padding_tracks_drawn_cap_at_two_sizes,
        },
        DrawnCase {
            name: "world_help_softly_darkens_the_surface_below_it",
            run:  world_help_softly_darkens_the_surface_below_it,
        },
        DrawnCase {
            name: "stacked_card_help_draws_outside_on_its_jack_row_at_scaled_spacing",
            run:  stacked_card_help_draws_outside_on_its_jack_row_at_scaled_spacing,
        },
        DrawnCase {
            name: "title_row_help_draws_above_when_horizontal_edges_do_not_fit",
            run:  title_row_help_draws_above_when_horizontal_edges_do_not_fit,
        },
        DrawnCase {
            name: "undersized_fitted_help_fails_readable_cap_ratio",
            run:  undersized_fitted_help_fails_readable_cap_ratio,
        },
        DrawnCase {
            name: "world_tooltip_keeps_its_fill_and_readable_words_in_lit_scenes",
            run:  world_tooltip_keeps_its_fill_and_readable_words_in_lit_scenes,
        },
        DrawnCase {
            name: "world_tooltip_shadow_draws_between_body_and_host",
            run:  world_tooltip_shadow_draws_between_body_and_host,
        },
        DrawnCase {
            name: "world_tooltip_shadow_draws_between_body_and_host_under_oit",
            run:  world_tooltip_shadow_draws_between_body_and_host_under_oit,
        },
        DrawnCase {
            name: "screen_tooltip_shadow_draws_between_body_and_host",
            run:  screen_tooltip_shadow_draws_between_body_and_host,
        },
        DrawnCase {
            name: "screen_tooltip_shadow_draws_between_body_and_host_under_oit",
            run:  screen_tooltip_shadow_draws_between_body_and_host_under_oit,
        },
        DrawnCase {
            name: "world_tooltip_shadow_draws_when_only_the_shadow_is_in_view",
            run:  world_tooltip_shadow_draws_when_only_the_shadow_is_in_view,
        },
        DrawnCase {
            name: "world_tooltip_shadow_leaves_a_nearer_blended_surface_its_colour",
            run:  world_tooltip_shadow_leaves_a_nearer_blended_surface_its_colour,
        },
    ])
}

fn world_tooltip_has_readable_front_and_plain_back() -> Result<(), String> {
    let (mut app, tip) = shown_panel_app(DrawnPanelCase::TooltipWords, SceneLighting::Unlit)?;
    let mut view = world_shadow_view(&app, tip)?;
    // Frame the whole word panel rather than the shadow helper's detail crop.
    view.viewport_height = PANEL_VIEW_HEIGHT;
    let target = app.target(full_region());
    let camera = spawn_shadow_camera(
        &mut app,
        &view,
        TooltipSpace::World,
        CaptureTransparency::Stable,
        target.clone(),
    );
    let front = capture_after_frames(&mut app, target.clone())?;
    let focus = view.transform.translation + *view.transform.forward() * 0.25;
    let rear_pose = Transform::from_translation(focus + *view.transform.forward() * 0.25)
        .looking_at(focus, *view.transform.up());
    app.world_mut().entity_mut(camera).insert(rear_pose);
    let rear = capture_after_frames(&mut app, target.clone())?;
    let world = app.world_mut();
    let batches: Vec<_> = world
        .query_filtered::<Entity, With<hana_diegetic::DiegeticTextBatch>>()
        .iter(world)
        .collect();
    for batch in batches {
        world.entity_mut(batch).insert(Visibility::Hidden);
    }
    let plain_rear = capture_after_frames(&mut app, target.clone())?;
    app.world_mut().entity_mut(camera).insert(view.transform);
    let plain_front = capture_after_frames(&mut app, target.clone())?;
    app.world_mut().entity_mut(tip).insert(Visibility::Hidden);
    app.world_mut().entity_mut(camera).insert(rear_pose);
    let empty_rear = capture_after_frames(&mut app, target)?;
    let changed = |a: &[u8], b: &[u8]| {
        a.as_chunks::<4>()
            .0
            .iter()
            .zip(b.as_chunks::<4>().0)
            .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 2))
            .count()
    };
    assert!(
        changed(&front, &plain_front) > 20,
        "tooltip must retain front word ink"
    );
    assert_eq!(
        changed(&rear, &plain_rear),
        0,
        "tooltip back must have no word or glyph-shadow ink"
    );
    assert!(
        changed(&plain_rear, &empty_rear) > 20,
        "tooltip must retain its plain panel back"
    );
    dump_pixels::<false>(
        "panel_text_sidedness",
        "tooltip-front",
        "front",
        full_region(),
        full_region(),
        &front,
    )?;
    dump_pixels::<false>(
        "panel_text_sidedness",
        "tooltip-back",
        "back",
        full_region(),
        full_region(),
        &rear,
    )?;
    Ok(())
}

#[test]
fn tooltip_panel_text_never_draws_mirrored() -> Result<(), String> {
    run_cases(&[DrawnCase {
        name: "tooltip_front_and_plain_back",
        run:  world_tooltip_has_readable_front_and_plain_back,
    }])
}
