//! The switch knob is centered on the track as drawn, including its border.

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::ecs::system::RunSystemOnce;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::ControlMetrics;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::SwitchSide;
use hana_diegetic::SwitchState;

mod drawn_support;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

const SIZE: u32 = 1024;

fn spawn_switch(app: &mut App) -> Result<Entity, String> {
    let metrics = ControlMetrics::from_label_size(Px(13.0));
    let border_width = metrics.switch.border_width;
    let mut layout = LayoutBuilder::new(100.0, 100.0);
    layout.with(
        El::overlay()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .size(metrics.switch.track_width, metrics.switch.track_height)
                    .padding(Padding::all(metrics.switch.track_padding))
                    .alignment(AlignX::Left, AlignY::Center)
                    .background(Color::BLACK)
                    .border(Border::all(border_width, Color::srgb(0.25, 0.25, 0.25)))
                    .corner_radius(CornerRadius::all(metrics.switch.track_radius()))
                    .switch("power"),
                |builder| {
                    builder.with(
                        El::new()
                            .size(metrics.switch.thumb.width, metrics.switch.thumb.height)
                            .background(Color::WHITE)
                            .corner_radius(CornerRadius::all(metrics.switch.thumb.corner_radius()))
                            .switch_knob(),
                        |_| {},
                    );
                },
            );
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(100.0))
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid switch panel: {error}"))?;
    let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    Ok(panel)
}

fn spawn_hana_camera(app: &mut App, target: Handle<Image>, region: CaptureRegion) -> Entity {
    let bloom = Bloom {
        prefilter: BloomPrefilter {
            threshold:          1.0,
            threshold_softness: 0.1,
        },
        ..default()
    };
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom.clone(),
    ));
    let mut image_camera = Camera::default();
    region.apply_to_camera(&mut image_camera);
    app.world_mut()
        .spawn((
            Camera3d::default(),
            image_camera,
            RenderTarget::Image(target.into()),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            bloom,
            Msaa::Off,
            Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id()
}

struct DrawnSwitch {
    track_left:     u32,
    track_right:    u32,
    track_center_y: f64,
    knob_left:      u32,
    knob_right:     u32,
    knob_top:       u32,
    knob_bottom:    u32,
    knob_center_x:  f64,
    knob_center_y:  f64,
}

fn drawn_switch(pixels: &[u8]) -> Result<DrawnSwitch, String> {
    let mut track_left = SIZE;
    let mut track_right = 0;
    let mut knob_left = SIZE;
    let mut knob_right = 0;
    let mut knob_top = SIZE;
    let mut knob_bottom = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let offset = ((y * SIZE + x) * 4) as usize;
            let rgb = pixels
                .get(offset..offset + 3)
                .ok_or("screenshot has no switch pixels")?;
            if rgb.iter().any(|channel| *channel > 25) {
                track_left = track_left.min(x);
                track_right = track_right.max(x);
            }
            if rgb.iter().all(|channel| *channel > 170) {
                knob_left = knob_left.min(x);
                knob_right = knob_right.max(x);
                knob_top = knob_top.min(y);
                knob_bottom = knob_bottom.max(y);
            }
        }
    }
    if track_left >= track_right || knob_left >= knob_right || knob_top >= knob_bottom {
        return Err(format!(
            "switch track or knob was not drawn: track x={track_left}..{track_right}, knob x={knob_left}..{knob_right}, knob y={knob_top}..{knob_bottom}"
        ));
    }
    let middle_x = track_left.midpoint(track_right);
    let mut track_top = SIZE;
    let mut track_bottom = 0;
    for y in 0..SIZE {
        let offset = ((y * SIZE + middle_x) * 4) as usize;
        let rgb = &pixels[offset..offset + 3];
        if rgb.iter().any(|channel| *channel > 25) {
            track_top = track_top.min(y);
            track_bottom = track_bottom.max(y);
        }
    }
    if track_top >= track_bottom {
        return Err("switch track center column was not drawn".to_string());
    }
    Ok(DrawnSwitch {
        track_left,
        track_right,
        track_center_y: f64::from(track_top + track_bottom) * 0.5,
        knob_left,
        knob_right,
        knob_top,
        knob_bottom,
        knob_center_x: f64::from(knob_left + knob_right) * 0.5,
        knob_center_y: f64::from(knob_top + knob_bottom) * 0.5,
    })
}

fn dump_switch_pixels(case: &str, pixels: &[u8]) -> Result<(), String> {
    dump_pixels::<true>(
        "switch_knob_centre",
        case,
        &format!(
            "px_per_em={:.9}",
            f64::from(SIZE) / (0.5 * (45.0_f64.to_radians() * 0.5).tan())
        ),
        CaptureRegion::new(UVec2::splat(SIZE), UVec2::ZERO, UVec2::splat(SIZE)),
        CaptureRegion::new(UVec2::splat(SIZE), UVec2::ZERO, UVec2::splat(SIZE)),
        pixels,
    )
}

fn check_switch_side(side: SwitchSide, check_shape: bool) -> Result<(), String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(UVec2::splat(SIZE), UVec2::ZERO, UVec2::splat(SIZE));
    let target = app.target(region);
    spawn_hana_camera(&mut app, target.clone(), region);
    let panel = spawn_switch(&mut app)?;
    app.settle()?;
    let widget = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named("power"))
        })
        .map_err(|error| format!("widget lookup failed: {error}"))?
        .ok_or("switch widget did not reify")?;
    app.world_mut()
        .get_mut::<SwitchState>(widget)
        .ok_or("switch state did not reify")?
        .set_initial_side(side);
    for _ in 0..4 {
        app.update();
    }
    let pixels = app.capture(target)?;
    dump_switch_pixels(&format!("{side:?}"), &pixels)?;
    let drawn = drawn_switch(&pixels)?;
    eprintln!(
        "switch {side:?}: drawn track x={}..{}, center y={}, knob center x={}, y={}, vertical delta={}",
        drawn.track_left,
        drawn.track_right,
        drawn.track_center_y,
        drawn.knob_center_x,
        drawn.knob_center_y,
        drawn.knob_center_y - drawn.track_center_y
    );
    assert!(
        (drawn.knob_center_y - drawn.track_center_y).abs() <= 0.5,
        "switch {side:?}: drawn track center y={}, knob center y={}",
        drawn.track_center_y,
        drawn.knob_center_y
    );
    if check_shape {
        assert_capsule_shape(&pixels, &drawn, side);
    }
    let track_center_x = f64::from(drawn.track_left + drawn.track_right) * 0.5;
    let reaches_stop = match side {
        SwitchSide::Off => drawn.knob_center_x < track_center_x,
        SwitchSide::On => drawn.knob_center_x > track_center_x,
    };
    assert!(
        reaches_stop,
        "switch {side:?}: drawn track x={}..{}, center x={track_center_x}, knob center x={}",
        drawn.track_left, drawn.track_right, drawn.knob_center_x
    );
    Ok(())
}

#[test]
fn bordered_switch_knob_draws_centered_at_off_and_on_stops() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "off_knob_is_centered_on_track",
            run:  || check_switch_side(SwitchSide::Off, false),
        },
        DrawnCase {
            name: "on_knob_is_centered_on_track",
            run:  || check_switch_side(SwitchSide::On, false),
        },
    ])
}

fn assert_capsule_shape(pixels: &[u8], drawn: &DrawnSwitch, side: SwitchSide) {
    let width = drawn.knob_right - drawn.knob_left + 1;
    let height = drawn.knob_bottom - drawn.knob_top + 1;
    let aspect = f64::from(width) / f64::from(height);
    assert!(
        (aspect - 52.0 / 32.0).abs() < 0.08,
        "switch {side:?}: capsule bounds {width} x {height}, aspect {aspect}"
    );
    let is_knob = |x: u32, y: u32| {
        let offset = ((y * SIZE + x) * 4) as usize;
        pixels[offset..offset + 3]
            .iter()
            .all(|channel| *channel > 170)
    };
    // Leave corner probes outside the semicircle after pixel-centre rounding.
    let inset = (height / 20).max(1);
    let mid_x = drawn.knob_left.midpoint(drawn.knob_right);
    let mid_y = drawn.knob_top.midpoint(drawn.knob_bottom);
    for x in [drawn.knob_left + inset, drawn.knob_right - inset] {
        assert!(
            is_knob(x, mid_y),
            "switch {side:?}: missing rounded end centre"
        );
        for y in [drawn.knob_top + inset, drawn.knob_bottom - inset] {
            assert!(
                !is_knob(x, y),
                "switch {side:?}: square thumb corner at {x},{y}"
            );
        }
    }
    for y in [drawn.knob_top + inset, drawn.knob_bottom - inset] {
        assert!(
            is_knob(mid_x, y),
            "switch {side:?}: missing straight capsule middle"
        );
    }
}

#[test]
fn switch_capsule_shape_draws_at_off_and_on_stops() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "off_switch_thumb_draws_as_measured_capsule",
            run:  || check_switch_side(SwitchSide::Off, true),
        },
        DrawnCase {
            name: "on_switch_thumb_draws_as_measured_capsule",
            run:  || check_switch_side(SwitchSide::On, true),
        },
    ])
}

fn check_switch_oit_stability() -> Result<(), String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(UVec2::splat(SIZE), UVec2::ZERO, UVec2::splat(SIZE));
    let target = app.target(region);
    let camera = spawn_hana_camera(&mut app, target.clone(), region);
    app.world_mut().entity_mut(camera).insert((
        StableTransparency,
        Transform::from_xyz(0.125, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let panel = spawn_switch(&mut app)?;
    app.settle()?;
    assert!(
        app.world()
            .get::<OrderIndependentTransparencySettings>(camera)
            .is_some()
    );
    let widget = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named("power"))
        })
        .map_err(|error| format!("widget lookup failed: {error}"))?
        .ok_or("switch widget did not reify")?;
    for side in [SwitchSide::Off, SwitchSide::On] {
        app.world_mut()
            .get_mut::<SwitchState>(widget)
            .ok_or("switch state did not reify")?
            .set_initial_side(side);
        for _ in 0..8 {
            app.update();
        }
        let reference = app.capture(target.clone())?;
        let drawn = drawn_switch(&reference).map_err(|error| format!("OIT {side:?}: {error}"))?;
        assert!(drawn.knob_right > drawn.knob_left);
        for shot in 1..8 {
            let pixels = app.capture(target.clone())?;
            assert_eq!(pixels.len(), reference.len());
            let changed = reference
                .as_chunks::<4>()
                .0
                .iter()
                .zip(pixels.as_chunks::<4>().0)
                .filter(|(before, after)| before != after)
                .count();
            assert_eq!(
                changed, 0,
                "switch {side:?}: OIT shot {shot} changed pixels"
            );
        }
    }
    Ok(())
}

#[test]
fn switch_pixels_are_identical_across_eight_oit_shots_at_both_stops() -> Result<(), String> {
    run_cases(&[DrawnCase {
        name: "off_and_on_switch_are_stable_under_oit",
        run:  check_switch_oit_stability,
    }])
}
