//! Drawn confirmation buttons through real pointer input.

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Button;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ConfirmChoice;
use hana_diegetic::ConfirmDialog;
use hana_diegetic::ConfirmTone;
use hana_diegetic::ControlMetrics;
use hana_diegetic::ControlPalette;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::Unit;
use hana_kana::ToU32;
use hana_kana::ToUsize;

mod drawn_support;
#[path = "../src/test_pointer.rs"]
mod test_pointer;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

#[test]
fn confirm_dialog_faces_change_with_pointer_hover_and_press() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "normal",
            run:  || drawn_confirm(ConfirmTone::Normal),
        },
        DrawnCase {
            name: "destructive",
            run:  || drawn_confirm(ConfirmTone::Destructive),
        },
    ])
}

#[test]
fn confirm_dialog_labels_and_hints_have_contrast_in_every_state() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "accent",
            run:  || confirm_ink(ConfirmTone::Normal),
        },
        DrawnCase {
            name: "warning",
            run:  || confirm_ink(ConfirmTone::Destructive),
        },
    ])
}

fn contrast(first: Color, second: Color) -> f32 {
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn luminance(color: Color) -> f32 {
    let color = color.to_linear();
    0.2126_f32.mul_add(
        color.red,
        0.7152_f32.mul_add(color.green, 0.0722 * color.blue),
    )
}

#[test]
fn confirm_neutral_hint_keeps_drawn_contrast_with_bloom() -> Result<(), String> {
    run_cases(&[DrawnCase {
        name: "neutral_hint",
        run:  neutral_hint_with_bloom,
    }])
}

fn neutral_hint_with_bloom() -> Result<(), String> {
    let mut scene = confirm_scene(ConfirmTone::Normal, ConfirmDisplay::EditorBloom)?;
    let (centre, fill_point) = sample_button(&scene, "cancel")?;
    let hint = if cfg!(target_os = "macos") {
        "⎋"
    } else {
        "Esc"
    };
    let label_region = confirm_text_region(&scene, "Cancel")?;
    let hint_region = confirm_text_region(&scene, hint)?;
    let mut failures = Vec::new();
    for state in ["rest", "hover", "press"] {
        if state == "hover" {
            test_pointer::move_world(&mut scene.app, scene.camera, centre);
        } else if state == "press" {
            test_pointer::press(&mut scene.app, PointerButton::Primary);
        }
        scene.app.settle()?;
        let pixels = capture_state(&mut scene, state)?;
        let fill = pixel_color(&pixels, scene.size, fill_point.as_uvec2())?;
        let label = brightest_ink(&pixels, scene.size, label_region)?;
        let hint_ink = brightest_ink(&pixels, scene.size, hint_region)?;
        let ratio = contrast(hint_ink, fill);
        if ratio < 4.5 || luminance(hint_ink) >= luminance(label) {
            failures.push(format!(
                "Cancel {state}: hint contrast {ratio:.3}:1; hint {hint_ink:?}, label {label:?}, fill {fill:?}"
            ));
        }
        if state == "hover" {
            // Row 3 of the live Linux Save shot (p38r4/save-strip.png) shows this fill.
            let observed_fill = Color::srgb_u8(0x4c, 0x54, 0x60);
            let ratio = contrast(hint_ink, observed_fill);
            if ratio < 4.5 {
                failures.push(format!(
                    "Cancel hover: hint contrast {ratio:.3}:1 against the live fill; hint {hint_ink:?}, fill {observed_fill:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

fn confirm_text_region(scene: &ConfirmScene, label: &str) -> Result<(UVec2, UVec2), String> {
    let panel = scene
        .app
        .world()
        .get::<DiegeticPanel>(scene.panel)
        .ok_or("panel")?;
    let layout = scene
        .app
        .world()
        .get::<ComputedDiegeticPanel>(scene.panel)
        .ok_or("layout")?;
    let command = layout.layout().solved().ok_or("solved confirm")?.commands.iter().find(|command| {
        matches!(&command.kind, hana_diegetic::RenderCommandKind::Text { text, .. } if text == label)
    }).ok_or("confirm text run")?;
    let (anchor_x, anchor_y) = panel.anchor_offsets();
    let camera = scene
        .app
        .world()
        .get::<Camera>(scene.camera)
        .ok_or("camera")?;
    let view = scene
        .app
        .world()
        .get::<GlobalTransform>(scene.camera)
        .ok_or("camera pose")?;
    let project = |x: f32, y: f32| {
        let scale = panel.points_to_world();
        camera
            .world_to_viewport(
                view,
                Vec3::new(
                    x.mul_add(scale, -anchor_x),
                    y.mul_add(-scale, anchor_y),
                    0.0,
                ),
            )
            .map_err(|error| error.to_string())
    };
    let bounds = command.bounds;
    let start = project(bounds.x, bounds.y)?.ceil().as_uvec2();
    let end = project(bounds.x + bounds.width, bounds.y + bounds.height)?
        .floor()
        .as_uvec2();
    Ok((start, end))
}

fn pixel_color(pixels: &[u8], size: UVec2, point: UVec2) -> Result<Color, String> {
    let offset = (point.y * size.x + point.x).to_usize() * 4;
    let [r, g, b] = pixels
        .get(offset..offset + 3)
        .ok_or("pixel outside image")?
    else {
        return Err("pixel has no RGB triplet".into());
    };
    Ok(Color::srgb_u8(*r, *g, *b))
}

fn brightest_ink(
    pixels: &[u8],
    size: UVec2,
    (start, end): (UVec2, UVec2),
) -> Result<Color, String> {
    let mut brightest = Color::BLACK;
    for y in start.y..end.y {
        for x in start.x..end.x {
            let ink = pixel_color(pixels, size, UVec2::new(x, y))?;
            if luminance(ink) > luminance(brightest) {
                brightest = ink;
            }
        }
    }
    Ok(brightest)
}

fn confirm_ink(tone: ConfirmTone) -> Result<(), String> {
    let mut scene = confirm_scene(tone, ConfirmDisplay::Standard)?;
    let mut failures = Vec::new();
    for id in ["save", "cancel"] {
        let label = if id == "save" { "Save" } else { "Cancel" };
        let (centre, _) = sample_button(&scene, id)?;
        for state in ["rest", "hover", "press"] {
            if state == "hover" {
                test_pointer::move_world(&mut scene.app, scene.camera, centre);
            }
            if state == "press" {
                test_pointer::press(&mut scene.app, PointerButton::Primary);
            }
            scene.app.settle()?;
            inspect_confirm_ink(&scene, tone, label, state, &mut failures)?;
        }
        test_pointer::release(&mut scene.app, PointerButton::Primary);
        test_pointer::move_world(&mut scene.app, scene.camera, Vec3::new(1.0, 1.0, 0.0));
        scene.app.settle()?;
    }
    assert!(failures.is_empty(), "{tone:?}: {}", failures.join("\n"));
    Ok(())
}

fn inspect_confirm_ink(
    scene: &ConfirmScene,
    tone: ConfirmTone,
    label: &str,
    state: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    use hana_diegetic::RenderCommandKind;
    let computed = scene
        .app
        .world()
        .get::<ComputedDiegeticPanel>(scene.panel)
        .ok_or("dialog layout")?;
    let commands = &computed.layout().solved().ok_or("solved dialog")?.commands;
    let label_run = commands.iter().find(|command| matches!(
        &command.kind, RenderCommandKind::Text { text, .. } if text == label || text.starts_with(&format!("{label} "))
    )).ok_or("label run")?;
    let RenderCommandKind::Text { config, .. } = &label_run.kind else {
        return Err("label kind".into());
    };
    let bounds = label_run.bounds;
    let face = commands
        .iter()
        .filter(|command| {
            matches!(command.kind, RenderCommandKind::Rectangle { .. })
                && command.bounds.x <= bounds.x
                && command.bounds.y <= bounds.y
                && command.bounds.x + command.bounds.width >= bounds.x + bounds.width
                && command.bounds.y + command.bounds.height >= bounds.y + bounds.height
        })
        .min_by(|a, b| {
            (a.bounds.width * a.bounds.height).total_cmp(&(b.bounds.width * b.bounds.height))
        })
        .ok_or("button face")?;
    let RenderCommandKind::Rectangle { color, .. } = face.kind else {
        return Err("face kind".into());
    };
    let ratio = contrast(config.color(), color);
    if ratio < 4.5 {
        failures.push(format!("{label} {state} label contrast {ratio:.3}:1"));
    }
    let hint = match (tone, label, cfg!(target_os = "macos")) {
        (ConfirmTone::Destructive, "Save", _) => "",
        (ConfirmTone::Destructive, _, true) => "↩ / ⎋",
        (ConfirmTone::Destructive, _, false) => "Return / Esc",
        (_, "Save", true) => "↩",
        (_, "Save", false) => "Return",
        (_, _, true) => "⎋",
        (_, _, false) => "Esc",
    };
    if hint.is_empty() {
        if !matches!(&label_run.kind, RenderCommandKind::Text { text, .. } if text == label) {
            failures.push(format!("{label} {state} must have no key hint"));
        }
        return Ok(());
    }
    let hint_run = commands.iter().find(
        |command| matches!(&command.kind, RenderCommandKind::Text { text, .. } if text == hint),
    );
    if let Some(hint_run) = hint_run {
        let RenderCommandKind::Text {
            config: hint_config,
            ..
        } = &hint_run.kind
        else {
            return Err("hint kind".into());
        };
        let ratio = contrast(hint_config.color(), color);
        if ratio < 4.5 {
            failures.push(format!("{label} {state} hint contrast {ratio:.3}:1"));
        }
        let gap = hint_run.bounds.x - bounds.x - bounds.width;
        let measurer = hana_diegetic::DiegeticTextMeasurer::default();
        let style = config.as_measure();
        let word_space =
            (measurer.measure_fn)("a a", &style).width - (measurer.measure_fn)("aa", &style).width;
        let expected = word_space * 2.0;
        let metric_gap = scene.metrics.button.hint_gap * Unit::Millimeters.to_points();
        if (gap - metric_gap).abs() > 0.1 {
            failures.push(format!(
                "{label} {state} hint gap {gap} differs from its metric {metric_gap}"
            ));
        }
        if gap + 0.1 < expected {
            failures.push(format!(
                "{label} {state} hint gap {gap}, needs two word spaces ({expected})"
            ));
        }
        if contrast(ControlPalette::system_dark().field_text, color) >= 4.5
            && hint_config.color() == config.color()
        {
            failures.push(format!(
                "{label} {state} must use the available secondary ink"
            ));
        }
    } else {
        failures.push(format!("{label} {state} needs a separate hint run"));
    }
    Ok(())
}

struct ConfirmScene {
    app:     CaseApp,
    size:    UVec2,
    target:  Handle<Image>,
    camera:  Entity,
    panel:   Entity,
    metrics: ControlMetrics<f32>,
}

#[derive(Clone, Copy)]
enum ConfirmDisplay {
    Standard,
    EditorBloom,
}

fn confirm_scene(tone: ConfirmTone, display: ConfirmDisplay) -> Result<ConfirmScene, String> {
    let size = UVec2::new(640, 180);
    let mut app = CaseApp::new();
    test_pointer::install(&mut app);
    let window = test_pointer::primary_window(&mut app, size);
    let target = app.target(CaptureRegion::new(size, UVec2::ZERO, size));
    let metrics = ControlMetrics::from_label_size(5.5_f32);
    let dialog = ConfirmDialog::new(
        ConfirmChoice::new("save", "Save", Button::new()),
        ConfirmChoice::new("cancel", "Cancel", Button::new()),
        tone,
    );
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .alignment(AlignX::Center, AlignY::Center),
    );
    dialog.append_actions(&mut builder, &metrics, &ControlPalette::system_dark());
    builder.text(Button::new().push("other", "Other", &metrics));
    let panel = DiegeticPanel::world()
        .size(Mm(160.0), Mm(45.0))
        .font_unit(Unit::Millimeters)
        .anchor(Anchor::Center)
        .picking(hana_diegetic::PanelPicking::INTERACTIVE)
        .with_tree(builder.build())
        .build()
        .map_err(|error| error.to_string())?;
    let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
    let pose = Transform::from_xyz(0.0, 0.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y);
    let projection = Projection::Orthographic(OrthographicProjection {
        scaling_mode: ScalingMode::FixedVertical {
            viewport_height: 0.045,
        },
        ..OrthographicProjection::default_3d()
    });
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            RenderTarget::Window(WindowRef::Entity(window)),
            projection.clone(),
            pose,
        ))
        .id();
    let mut image_camera = Camera::default();
    CaptureRegion::new(size, UVec2::ZERO, size).apply_to_camera(&mut image_camera);
    let image_camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            image_camera,
            RenderTarget::Image(target.clone().into()),
            projection,
            pose,
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            Msaa::Off,
            StableTransparency,
        ))
        .id();
    if matches!(display, ConfirmDisplay::EditorBloom) {
        let bloom = Bloom {
            prefilter: BloomPrefilter {
                threshold:          1.0,
                threshold_softness: 0.1,
            },
            ..default()
        };
        app.world_mut().entity_mut(camera).insert((
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            bloom.clone(),
        ));
        app.world_mut().entity_mut(image_camera).insert(bloom);
    }
    app.settle()?;
    Ok(ConfirmScene {
        app,
        size,
        target,
        camera,
        panel,
        metrics,
    })
}

fn sample_button(scene: &ConfirmScene, id: &str) -> Result<(Vec3, Vec2), String> {
    let ConfirmScene {
        app,
        panel,
        camera,
        metrics,
        ..
    } = scene;
    let data = app
        .world()
        .get::<DiegeticPanel>(*panel)
        .ok_or("dialog panel")?;
    let computed = app
        .world()
        .get::<ComputedDiegeticPanel>(*panel)
        .ok_or("dialog layout")?;
    let bounds = computed
        .visible_widget_bounds(data)
        .filter(|(choice, _, _)| choice.as_str() == Some(id))
        .map(|(_, _, bounds)| bounds)
        .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
        .ok_or("choice bounds")?;
    let (x, y) = data.anchor_offsets();
    let scale = data.points_to_world();
    let centre = Vec3::new(
        f32::mul_add(bounds.width, 0.5, bounds.x).mul_add(scale, -x),
        f32::mul_add(bounds.height, 0.5, bounds.y).mul_add(-scale, y),
        0.0,
    );
    let fill_point = Vec3::new(
        (metrics.button.side_padding * Unit::Millimeters.to_points())
            .mul_add(0.5, bounds.x)
            .mul_add(scale, -x),
        centre.y,
        0.0,
    );
    let view = app.world().get::<Camera>(*camera).ok_or("pointer camera")?;
    let transform = app
        .world()
        .get::<GlobalTransform>(*camera)
        .ok_or("camera pose")?;
    let pixel = view
        .world_to_viewport(transform, fill_point)
        .map_err(|error| error.to_string())?;
    Ok((centre, pixel))
}

fn capture_state(scene: &mut ConfirmScene, state: &str) -> Result<Vec<u8>, String> {
    let pixels = scene.app.capture(scene.target.clone())?;
    let region = CaptureRegion::new(scene.size, UVec2::ZERO, scene.size);
    dump_pixels::<false>("confirm_dialog", "", state, region, region, &pixels)?;
    Ok(pixels)
}

fn drawn_confirm(tone: ConfirmTone) -> Result<(), String> {
    let mut scene = confirm_scene(tone, ConfirmDisplay::Standard)?;
    let size = scene.size;
    let camera = scene.camera;
    let (centre, default_pixel) = sample_button(&scene, "save")?;
    let (_, cancel_pixel) = sample_button(&scene, "cancel")?;
    let (_, other_pixel) = sample_button(&scene, "other")?;
    let colour = |pixels: &[u8], point: Vec2| -> Result<[u8; 3], String> {
        let offset = (point.y.floor().to_u32() * size.x + point.x.floor().to_u32()).to_usize() * 4;
        pixels
            .get(offset..offset + 3)
            .ok_or("button sample outside image")?
            .try_into()
            .map_err(|error| format!("RGB sample: {error}"))
    };
    let rest = capture_state(&mut scene, "rest")?;
    let default_fill = colour(&rest, default_pixel)?;
    let cancel_fill = colour(&rest, cancel_pixel)?;
    assert_ne!(
        default_fill, cancel_fill,
        "the default must remain visible beside Cancel"
    );
    assert_ne!(cancel_fill, [0; 3], "Cancel must draw a button face");
    assert_ne!(
        colour(&rest, other_pixel)?,
        [0; 3],
        "other actions must draw button faces"
    );
    test_pointer::move_world(&mut scene.app, camera, centre);
    scene.app.settle()?;
    let hovered = capture_state(&mut scene, "hover")?;
    assert_ne!(
        colour(&hovered, default_pixel)?,
        default_fill,
        "hover must change the drawn face"
    );
    test_pointer::press(&mut scene.app, PointerButton::Primary);
    scene.app.settle()?;
    let pressed = capture_state(&mut scene, "press")?;
    assert_ne!(
        colour(&pressed, default_pixel)?,
        colour(&hovered, default_pixel)?,
        "press must change the drawn face"
    );
    test_pointer::release(&mut scene.app, PointerButton::Primary);
    scene.app.settle()?;
    let (neutral_centre, neutral_pixel) = sample_button(&scene, "other")?;
    test_pointer::move_world(&mut scene.app, camera, neutral_centre);
    scene.app.settle()?;
    let neutral_hover = capture_state(&mut scene, "neutral_hover")?;
    assert_ne!(
        colour(&neutral_hover, neutral_pixel)?,
        colour(&rest, other_pixel)?,
        "the neutral button must change on hover"
    );
    test_pointer::press(&mut scene.app, PointerButton::Primary);
    scene.app.settle()?;
    let neutral_press = capture_state(&mut scene, "neutral_press")?;
    assert_ne!(
        colour(&neutral_press, neutral_pixel)?,
        colour(&neutral_hover, neutral_pixel)?,
        "the neutral button must change on press"
    );
    assert_ne!(
        colour(&hovered, default_pixel)?,
        colour(&neutral_hover, neutral_pixel)?,
        "the default button must keep its {tone:?} tone on hover"
    );
    assert_ne!(
        colour(&pressed, default_pixel)?,
        colour(&neutral_press, neutral_pixel)?,
        "the default button must keep its {tone:?} tone on press"
    );
    test_pointer::release(&mut scene.app, PointerButton::Primary);
    Ok(())
}
