//! Drawn proof of the shared slider face at Hana's close jack-control scale.
#![allow(
    clippy::expect_used,
    reason = "invalid test fixtures should fail immediately"
)]

use std::f32::consts::FRAC_PI_8;

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::ecs::system::RunSystemOnce;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::Camera;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Image;
use bevy::prelude::Msaa;
use bevy::prelude::Transform;
use bevy::prelude::UVec2;
use bevy::prelude::Vec3;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::BoundingBox;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ControlMetrics;
use hana_diegetic::ControlPalette;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::Slider;
use hana_diegetic::SliderPushedValue;
use hana_diegetic::SliderReachableBand;
use hana_diegetic::SliderState;
use hana_diegetic::Text;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_kana::ToF32;
use hana_kana::ToU32;

mod drawn_support;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

const SIDE: u32 = 256;
const SIDE_FLOAT: f32 = 256.0;
const FIELD_IMAGE_WIDTH: u32 = 1280;
const FIELD_IMAGE_HEIGHT: u32 = 720;
// `crates/hana/src/world_panel_palette.rs::REAR_CARD_GROUP_BACKGROUND`.
const HANA_JACK_PANEL_BACKGROUND: Color = Color::srgb(0.19, 0.215, 0.26);
// `crates/hana/src/world_panel_palette.rs::INFO_PANEL_BACKGROUND`.
const HANA_SHAPE_PANEL_BACKGROUND: Color = Color::srgb(46.0 / 255.0, 52.0 / 255.0, 64.0 / 255.0);
// `crates/hana_diegetic/examples/widgets.rs::PANEL_BACKGROUND`.
const WIDGETS_EXAMPLE_PANEL_BACKGROUND: Color = Color::srgba(0.02, 0.03, 0.07, 0.92);
// A 5.5 mm Hana face label draws at 20 px in the close Shape shot.
const LABEL_PIXELS: f32 = 0.0055 * (34.0 / (0.0055 * 22.0 / 13.0));
const HANA_FACE_LABEL_SIZE: f32 = 5.5;

fn spawn_camera(
    app: &mut CaseApp,
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
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom.clone(),
    ));
    let mut camera = Camera::default();
    region.apply_to_camera(&mut camera);
    let distance = panel_height * 0.001 * 0.5 / FRAC_PI_8.tan();
    app.world_mut().spawn((
        Camera3d::default(),
        camera,
        RenderTarget::Image(target.into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, distance).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn capture_face_on(
    panel_background: Color,
    band: SliderReachableBand,
    case_name: &str,
) -> Result<Vec<u8>, String> {
    let metrics = ControlMetrics::from_label_size(LABEL_PIXELS);
    let mut builder = LayoutBuilder::new(SIDE_FLOAT, SIDE_FLOAT);
    builder.with(
        El::overlay()
            .size(SIDE_FLOAT, SIDE_FLOAT)
            .background(panel_background)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.slider("level", Slider::new(0.0..=1.0).value(0.5), &metrics, band);
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(SIDE_FLOAT), Mm(SIDE_FLOAT))
        .font_unit(Unit::Millimeters)
        .anchor(Anchor::Center)
        .with_tree(builder.build())
        .build()
        .map_err(|error| format!("invalid slider panel: {error}"))?;
    let mut app = CaseApp::new();
    let size = UVec2::splat(SIDE);
    let region = CaptureRegion::new(size, UVec2::ZERO, size);
    let target = app.target(region);
    spawn_camera(&mut app, target.clone(), region, SIDE_FLOAT);
    let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    app.settle()?;
    if band == SliderReachableBand::Drawn {
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("level"))
            })
            .map_err(|error| format!("slider widget lookup failed: {error}"))?
            .ok_or("slider widget did not reify")?;
        app.world_mut()
            .get_mut::<SliderState>(widget)
            .ok_or("slider state did not reify")?
            .set_pushed(SliderPushedValue::PushedTo {
                value:      0.75,
                band_start: 0.0,
                band_end:   1.0,
            });
        for _ in 0..4 {
            app.update();
        }
    }
    let pixels = app.capture(target)?;
    dump_pixels::<true>(
        "slider_face",
        case_name,
        "live close scale",
        region,
        region,
        &pixels,
    )?;
    Ok(pixels)
}

fn capture_face(band: SliderReachableBand) -> Result<Vec<u8>, String> {
    capture_face_on(HANA_JACK_PANEL_BACKGROUND, band, "face")
}

fn capture_value_field(panel_background: Color, case_name: &str) -> Result<Vec<u8>, String> {
    let metrics = ControlMetrics::from_label_size(HANA_FACE_LABEL_SIZE);
    let pixels_per_millimetre = LABEL_PIXELS / HANA_FACE_LABEL_SIZE;
    let panel_width = FIELD_IMAGE_WIDTH.to_f32() / pixels_per_millimetre;
    let panel_height = FIELD_IMAGE_HEIGHT.to_f32() / pixels_per_millimetre;
    let mut builder = LayoutBuilder::new(panel_width, panel_height);
    builder.with(
        El::overlay()
            .size(panel_width, panel_height)
            .background(panel_background)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.text(
                Text::new(
                    "1",
                    TextStyle::new(metrics.label_size)
                        .with_color(ControlPalette::system_dark().knob)
                        .with_align(TextAlign::Right),
                )
                .layout(El::row().slider_value_field(
                    "value",
                    ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("value")),
                    &metrics,
                )),
            );
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(panel_width), Mm(panel_height))
        .font_unit(Unit::Millimeters)
        .anchor(Anchor::Center)
        .with_tree(builder.build())
        .build()
        .map_err(|error| format!("invalid value field panel: {error}"))?;
    let mut app = CaseApp::new();
    let size = UVec2::new(FIELD_IMAGE_WIDTH, FIELD_IMAGE_HEIGHT);
    let region = CaptureRegion::new(size, UVec2::ZERO, size);
    let target = app.target(region);
    spawn_camera(&mut app, target.clone(), region, panel_height);
    // Full-coverage hairlines need pixel-aligned edges, including when the
    // shared value field projects to an odd number of pixels.
    let field_left = metrics
        .slider
        .value_width
        .mul_add(-pixels_per_millimetre, FIELD_IMAGE_WIDTH.to_f32())
        * 0.5;
    let offset_millimetres = (field_left.round() - field_left) / pixels_per_millimetre;
    let panel = app
        .world_mut()
        .spawn((
            panel,
            Transform::from_xyz(offset_millimetres * 0.001, 0.0, 0.0),
        ))
        .id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    app.settle()?;
    let pixels = app.capture(target)?;
    dump_pixels::<true>(
        "slider_face",
        case_name,
        "live close scale",
        region,
        region,
        &pixels,
    )?;
    Ok(pixels)
}

fn rgb(pixels: &[u8], x: u32, y: u32) -> [u8; 3] { rgb_at(pixels, SIDE, x, y) }

fn rgb_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = usize::try_from((y * width + x) * 4).expect("image offset fits usize");
    pixels[at..at + 3].try_into().expect("RGBA pixel")
}

/// Match the opaque accent read back through this renderer, excluding its reach tint.
const fn is_live_fill(ink: [u8; 3], opaque_fill: [u8; 3]) -> bool {
    ink[0].abs_diff(opaque_fill[0]) <= 2
        && ink[1].abs_diff(opaque_fill[1]) <= 2
        && ink[2].abs_diff(opaque_fill[2]) <= 2
}

fn cabled_slider_fill_reference() -> Result<[u8; 3], String> {
    let reference = capture_pushed_slider_face(
        0.15,
        SliderPushedValue::PushedTo {
            value:      0.85,
            band_start: 0.0,
            band_end:   1.0,
        },
    )?;
    let x = reference
        .track
        .width
        .mul_add(0.5, reference.track.x)
        .floor()
        .to_u32();
    let fill = rgb(&reference.pixels, x, reference.centre_row());
    assert!(
        fill[2] > fill[0].saturating_add(50) && fill[2] > fill[1].saturating_add(40),
        "cabled fill reference must sample live blue clear of the authored knob: {fill:?}"
    );
    Ok(fill)
}

fn luminance(rgb: [u8; 3]) -> f32 {
    let linear = |channel: u8| {
        let channel = f32::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126_f32.mul_add(
        linear(rgb[0]),
        0.7152_f32.mul_add(linear(rgb[1]), 0.0722 * linear(rgb[2])),
    )
}

fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn slider_face_draws_the_switch_capsule_without_a_separate_rim() -> Result<(), String> {
    let pixels = capture_face(SliderReachableBand::Hidden)?;
    let centre = rgb(&pixels, SIDE / 2, SIDE / 2);
    let image = &pixels;
    let rim = (116..140)
        .flat_map(|y| (112..144).map(move |x| rgb(image, x, y)))
        .filter(|rgb| rgb.iter().all(|channel| *channel >= 210))
        .map(luminance)
        .fold(0.0_f32, f32::max);
    let fill = luminance(centre);
    eprintln!("slider knob centre {centre:?}, fill luminance {fill:.3}, brightest rim {rim:.3}");
    assert!(
        centre.iter().all(|channel| *channel >= 205),
        "knob is not near-white: {centre:?}"
    );
    assert!(
        rim <= fill + 0.015,
        "slider has a separate bright rim absent from the switch knob"
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum CapsuleControl {
    Slider,
    Switch,
}

fn capture_capsule(control: CapsuleControl) -> Result<Vec<u8>, String> {
    let metrics = ControlMetrics::from_label_size(13.0_f32);
    let mut builder = LayoutBuilder::new(SIDE_FLOAT, SIDE_FLOAT);
    builder.with(
        El::overlay()
            .size(SIDE_FLOAT, SIDE_FLOAT)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| match control {
            CapsuleControl::Switch => {
                let size = metrics.switch;
                builder.with(
                    El::overlay()
                        .size(size.track_width, size.track_height)
                        .padding(hana_diegetic::Padding::all(size.track_padding))
                        .border(hana_diegetic::Border::all(size.border_width, Color::BLACK))
                        .corner_radius(size.track_radius())
                        .alignment(AlignX::Left, AlignY::Center)
                        .switch("power"),
                    |builder| {
                        builder.switch_knob(&size);
                    },
                );
            },
            CapsuleControl::Slider => {
                builder.slider(
                    "level",
                    Slider::new(0.0..=1.0).value(0.5),
                    &metrics,
                    SliderReachableBand::Hidden,
                );
            },
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(SIDE_FLOAT), Mm(SIDE_FLOAT))
        .anchor(Anchor::Center)
        .with_tree(builder.build())
        .build()
        .map_err(|error| error.to_string())?;
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(UVec2::splat(SIDE), UVec2::ZERO, UVec2::splat(SIDE));
    let target = app.target(region);
    spawn_camera(&mut app, target.clone(), region, SIDE_FLOAT);
    // Both outlines land at the same pixel phase. The on knob's center is
    // half the track's free travel to the right of its track center.
    let offset = match control {
        CapsuleControl::Switch => {
            (metrics.switch.track_width - metrics.switch.thumb.width)
                .mul_add(0.5, -metrics.switch.track_padding)
                - metrics.switch.border_width
        },
        CapsuleControl::Slider => 0.0,
    };
    let panel = app
        .world_mut()
        .spawn((panel, Transform::from_xyz(-offset * 0.001, 0.0, 0.0)))
        .id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    app.settle()?;
    app.capture(target)
}

#[test]
fn slider_thumb_drawn_outline_matches_the_switch_knob() -> Result<(), String> {
    run_cases(&[DrawnCase {
        name: "slider_thumb_drawn_outline_matches_the_switch_knob",
        run:  || {
            let slider = capture_capsule(CapsuleControl::Slider)?;
            let switch = capture_capsule(CapsuleControl::Switch)?;
            let mask = |pixels: &[u8]| {
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|pixel| pixel[..3].iter().all(|channel| *channel >= 170))
                    .collect::<Vec<_>>()
            };
            let slider_mask = mask(&slider);
            assert!(
                slider_mask.iter().filter(|inside| **inside).count() > 100,
                "capsule was not drawn"
            );
            let switch_mask = mask(&switch);
            // The blue switch track and gray slider track blend differently
            // into antialiased edges. Compare each scanline's contour, allowing
            // one pixel at the edge rather than requiring equal blended RGB.
            let side = usize::try_from(SIDE).map_err(|error| error.to_string())?;
            for (y, (slider_row, switch_row)) in slider_mask
                .chunks(side)
                .zip(switch_mask.chunks(side))
                .enumerate()
            {
                let span = |row: &[bool]| {
                    row.iter()
                        .position(|inside| *inside)
                        .zip(row.iter().rposition(|inside| *inside))
                };
                match (span(slider_row), span(switch_row)) {
                    (Some((left, right)), Some((switch_left, switch_right))) => {
                        assert!(
                            left.abs_diff(switch_left) <= 1 && right.abs_diff(switch_right) <= 1,
                            "capsule contour differs on row {y}: slider {left}..={right}, switch {switch_left}..={switch_right}"
                        );
                    },
                    (slider, switch) => assert_eq!(slider, switch, "capsule height at row {y}"),
                }
            }
            Ok(())
        },
    }])
}

#[test]
fn slider_face_draws_accent_before_knob_and_three_to_one_track_after_it() -> Result<(), String> {
    let pixels = capture_face(SliderReachableBand::Hidden)?;
    let backdrop = rgb(&pixels, 10, 128);
    let accent = rgb(&pixels, 70, 128);
    let track = rgb(&pixels, 186, 128);
    let ratio = contrast(track, backdrop);
    eprintln!(
        "slider accent {accent:?}, faint track {track:?}, card {backdrop:?}, contrast {ratio:.2}:1"
    );
    assert!(
        accent[2] > accent[0].saturating_add(50),
        "filled track is not accent blue"
    );
    assert!(
        ratio >= 3.0,
        "unfilled track contrast {ratio:.2}:1 falls below 3:1"
    );
    Ok(())
}

#[test]
fn slider_track_and_values_read_apart_on_every_host_panel() -> Result<(), String> {
    let mut failures = Vec::new();
    for (name, background) in [
        ("hana_jack_panel", HANA_JACK_PANEL_BACKGROUND),
        ("hana_shape_panel", HANA_SHAPE_PANEL_BACKGROUND),
        ("widgets_example_panel", WIDGETS_EXAMPLE_PANEL_BACKGROUND),
    ] {
        let pixels = capture_face_on(background, SliderReachableBand::Hidden, name)?;
        let panel = rgb(&pixels, 10, 128);
        let fill = rgb(&pixels, 70, 128);
        let track = rgb(&pixels, 186, 128);
        let image = &pixels;
        let knob = (116..140)
            .flat_map(|y| (112..144).map(move |x| rgb(image, x, y)))
            .max_by(|left, right| luminance(*left).total_cmp(&luminance(*right)))
            .ok_or("knob did not draw")?;
        let track_panel = contrast(track, panel);
        let knob_track = contrast(knob, track);
        let fill_track = contrast(fill, track);
        let largest_hue_difference = fill
            .iter()
            .zip(track)
            .map(|(fill, track)| fill.abs_diff(track))
            .max()
            .ok_or("slider colours have no channels")?;
        let track_spread = track
            .iter()
            .max()
            .expect("track has channels")
            .saturating_sub(*track.iter().min().expect("track has channels"));
        eprintln!(
            "{name}: panel {panel:?}, track {track:?} ({track_panel:.2}:1), knob {knob:?} ({knob_track:.2}:1 against track), fill {fill:?} ({fill_track:.2}:1 against track)"
        );
        if track_panel < 3.0 {
            failures.push(format!(
                "{name} track-to-panel contrast {track_panel:.2}:1 falls below 3:1"
            ));
        }
        if knob_track < 2.5 {
            failures.push(format!(
                "{name} knob-to-track contrast {knob_track:.2}:1 falls below 2.5:1"
            ));
        }
        if largest_hue_difference < 64 {
            failures.push(format!(
                "{name} fill {fill:?} differs from track {track:?} by only {largest_hue_difference} in its strongest channel"
            ));
        }
        if fill[0] > 8 || fill[1].abs_diff(123) > 8 || fill[2].abs_diff(226) > 8 {
            failures.push(format!(
                "{name} fill {fill:?} falls outside the rendered accent-blue bucket"
            ));
        }
        if track_spread > 8 {
            failures.push(format!("{name} track {track:?} is not neutral"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}

#[test]
fn slider_face_centres_knob_on_track_within_one_pixel() -> Result<(), String> {
    let pixels = capture_face(SliderReachableBand::Hidden)?;
    let pale: Vec<u32> = (100..156)
        .filter(|&y| rgb(&pixels, 128, y).iter().all(|channel| *channel >= 205))
        .collect();
    let top = pale.first().ok_or("knob not drawn")?;
    let bottom = pale.last().ok_or("knob not drawn")?;
    let centre = f32::from(u16::try_from(*top + *bottom).expect("row sum fits u16")) * 0.5;
    eprintln!("slider knob rows {top}..={bottom}, centre {centre:.1}, track centre 127.5");
    assert!(
        (centre - 127.5).abs() <= 1.0,
        "knob misses track centre by more than 1 px"
    );
    Ok(())
}

#[test]
fn drawn_pushed_value_replaces_marker_with_blue_fill() -> Result<(), String> {
    run_cases(&[DrawnCase {
        name: "pushed_value_fill",
        run:  || {
            let hidden = capture_face(SliderReachableBand::Hidden)?;
            let opaque_fill = cabled_slider_fill_reference()?;
            let drawn = capture_face(SliderReachableBand::Drawn)?;
            let changes = (105..151)
                .flat_map(|y| (105..231).map(move |x| (x, y)))
                .filter(|&(x, y)| rgb(&hidden, x, y) != rgb(&drawn, x, y))
                .filter(|&(x, y)| {
                    let ink = rgb(&drawn, x, y);
                    is_live_fill(ink, opaque_fill)
                })
                .count();
            eprintln!("pushed-value fill changed {changes} blue pixels");
            assert!(
                changes >= 100,
                "pushed value does not draw a blue bar from the handle"
            );
            Ok(())
        },
    }])
}

struct SliderFacePicture {
    pixels:             Vec<u8>,
    track:              BoundingBox,
    travel_thumb_width: f32,
    knob_shadow_width:  f32,
    opaque_knob:        [u8; 3],
}

impl SliderFacePicture {
    fn centre_row(&self) -> u32 {
        self.track
            .height
            .mul_add(0.5, self.track.y)
            .floor()
            .to_u32()
    }

    fn columns(&self) -> std::ops::Range<u32> {
        self.track.x.ceil().to_u32()..(self.track.x + self.track.width).floor().to_u32()
    }

    fn live_fill_columns(&self, opaque_fill: [u8; 3]) -> Vec<u32> {
        self.columns()
            .filter(|&x| {
                let ink = rgb(&self.pixels, x, self.centre_row());
                is_live_fill(ink, opaque_fill)
            })
            .collect()
    }

    fn live_fill_with_antialiased_cap(
        &self,
        opaque_fill: [u8; 3],
        setting: f32,
        live: f32,
    ) -> Vec<u32> {
        let tip = live.mul_add(
            self.track.width - self.travel_thumb_width,
            self.travel_thumb_width.mul_add(0.5, self.track.x),
        );
        let inside = if live > setting {
            tip.floor().to_u32() - 1
        } else {
            tip.ceil().to_u32() + 1
        };
        // A pale backdrop changes the cap's antialiased RGB. Read that paint
        // separately, keeping the opaque core and the same RGB tolerance.
        let cap = rgb(&self.pixels, inside, self.centre_row());
        let band_fraction = if live > setting { 0.05_f32 } else { 0.95_f32 };
        let band_x = band_fraction
            .mul_add(self.track.width, self.track.x)
            .floor()
            .to_u32();
        let band = rgb(&self.pixels, band_x, self.centre_row());
        assert!(
            cap[2] > cap[0].saturating_add(50)
                && cap[2].saturating_sub(cap[1])
                    > band[2].saturating_sub(band[1]).saturating_add(2),
            "the live cap must read as accent beyond the band at its endpoint: {cap:?}/{band:?}"
        );
        self.columns()
            .filter(|&x| {
                let ink = rgb(&self.pixels, x, self.centre_row());
                is_live_fill(ink, opaque_fill) || is_live_fill(ink, cap)
            })
            .collect()
    }

    fn knob_columns(&self) -> Vec<u32> {
        self.columns()
            .filter(|&x| {
                let ink = rgb(&self.pixels, x, self.centre_row());
                ink.into_iter()
                    .zip(self.opaque_knob)
                    .all(|(drawn, reference)| drawn.abs_diff(reference) <= 2)
            })
            .collect()
    }
}

fn capture_pushed_slider_face(
    setting: f32,
    pushed: SliderPushedValue,
) -> Result<SliderFacePicture, String> {
    let metrics = ControlMetrics::from_label_size(LABEL_PIXELS);
    let mut builder = LayoutBuilder::new(SIDE_FLOAT, SIDE_FLOAT);
    builder.with(
        El::overlay()
            .size(SIDE_FLOAT, SIDE_FLOAT)
            .background(HANA_JACK_PANEL_BACKGROUND)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.slider(
                "level",
                Slider::new(0.0..=1.0).value(setting),
                &metrics,
                SliderReachableBand::Drawn,
            );
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(SIDE_FLOAT), Mm(SIDE_FLOAT))
        .font_unit(Unit::Millimeters)
        .anchor(Anchor::Center)
        .with_tree(builder.build())
        .build()
        .map_err(|error| format!("invalid pushed slider panel: {error}"))?;
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(UVec2::splat(SIDE), UVec2::ZERO, UVec2::splat(SIDE));
    let target = app.target(region);
    spawn_camera(&mut app, target.clone(), region, SIDE_FLOAT);
    let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    app.settle()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let widget: Entity = loop {
        app.update();
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("level"))
            })
            .map_err(|error| format!("pushed slider lookup: {error}"))?;
        if let Some(widget) =
            widget.filter(|&widget| app.world().get::<SliderState>(widget).is_some())
        {
            break widget;
        }
        if std::time::Instant::now() >= deadline {
            return Err("pushed slider never reified".to_string());
        }
    };
    app.world_mut()
        .get_mut::<SliderState>(widget)
        .ok_or("pushed slider state")?
        .set_pushed(pushed);
    let mut previous = app.capture(target.clone())?;
    let pixels = loop {
        let current = app.capture(target.clone())?;
        if current == previous {
            break current;
        }
        if std::time::Instant::now() >= deadline {
            return Err("pushed slider picture never settled".to_string());
        }
        previous = current;
    };
    slider_face_picture_from_drawn_bounds(&app, panel, pixels, setting)
}

fn slider_face_picture_from_drawn_bounds(
    app: &CaseApp,
    panel: Entity,
    pixels: Vec<u8>,
    setting: f32,
) -> Result<SliderFacePicture, String> {
    let layout = app
        .world()
        .get::<ComputedDiegeticPanel>(panel)
        .and_then(|computed| computed.layout().solved())
        .ok_or("pushed slider did not solve")?;
    let bounds_for = |colour| {
        layout.commands.iter().find_map(|command| {
            matches!(command.kind, RenderCommandKind::Rectangle { color, .. }
                if color == colour)
            .then_some(command.bounds)
        })
    };
    let raw_track = bounds_for(ControlPalette::system_dark().track).ok_or("pushed slider track")?;
    let panel_data = app
        .world()
        .get::<DiegeticPanel>(panel)
        .ok_or("pushed slider panel")?;
    let (anchor_x, anchor_y) = panel_data.anchor_offsets();
    let pixels_per_metre = SIDE_FLOAT / panel_data.world_height();
    let pixels_per_point = panel_data.points_to_world() * pixels_per_metre;
    let track = BoundingBox {
        x:      raw_track.x.mul_add(
            pixels_per_point,
            anchor_x.mul_add(-pixels_per_metre, SIDE_FLOAT * 0.5),
        ),
        y:      raw_track.y.mul_add(
            pixels_per_point,
            anchor_y.mul_add(-pixels_per_metre, SIDE_FLOAT * 0.5),
        ),
        width:  raw_track.width * pixels_per_point,
        height: raw_track.height * pixels_per_point,
    };
    let metrics = ControlMetrics::from_label_size(LABEL_PIXELS).slider;
    // The fixture authors millimetres. The solved bounds use layout points.
    let travel_thumb_width =
        raw_track.width * (metrics.thumb_hit_width / metrics.width) * pixels_per_point;
    let knob_shadow_width = raw_track.width
        * (metrics.thumb.width * hana_diegetic::SliderMetrics::<f32>::THUMB_SHADOW_SCALE
            / metrics.width)
        * pixels_per_point;
    let knob_centre = setting.mul_add(
        track.width - travel_thumb_width,
        travel_thumb_width.mul_add(0.5, track.x),
    );
    let knob_x = knob_centre.floor().to_u32();
    let knob_y = track.height.mul_add(0.5, track.y).floor().to_u32();
    let opaque_knob = rgb(&pixels, knob_x, knob_y);
    assert!(
        opaque_knob.iter().all(|&channel| channel >= 205),
        "opaque knob reference: {opaque_knob:?}"
    );
    // Read the opaque capsule core. The antialiased rim blends with the live
    // fill under it and can change colour without moving the capsule.
    Ok(SliderFacePicture {
        pixels,
        track,
        travel_thumb_width,
        knob_shadow_width,
        opaque_knob,
    })
}

#[test]
fn pushed_slider_fill_spans_handle_to_live_value_in_both_directions() -> Result<(), String> {
    let opaque_fill = cabled_slider_fill_reference()?;
    let right = capture_pushed_slider_face(
        0.5,
        SliderPushedValue::PushedTo {
            value:      0.8,
            band_start: 0.0,
            band_end:   1.0,
        },
    )?;
    let left = capture_pushed_slider_face(
        0.5,
        SliderPushedValue::PushedTo {
            value:      0.2,
            band_start: 0.0,
            band_end:   1.0,
        },
    )?;
    let knob = right.knob_columns();
    assert_eq!(
        knob,
        left.knob_columns(),
        "live signal moves the fill while the knob stays fixed"
    );
    let knob_left = *knob.first().ok_or("authored knob not drawn")?;
    let knob_right = *knob.last().ok_or("authored knob not drawn")?;
    assert!(
        !right.live_fill_columns(opaque_fill).is_empty()
            && !left.live_fill_columns(opaque_fill).is_empty(),
        "both live directions draw a blue bar"
    );
    let right_blue = right.live_fill_with_antialiased_cap(opaque_fill, 0.5, 0.8);
    let left_blue = left.live_fill_with_antialiased_cap(opaque_fill, 0.5, 0.2);
    assert!(
        right_blue.iter().all(|&x| x > knob_right),
        "positive fill extends behind the setting"
    );
    assert!(
        left_blue.iter().all(|&x| x < knob_left),
        "negative fill extends beyond the setting"
    );
    // The opaque core excludes the inset rim and the Add shadow. The blue
    // becomes unobstructed at the shadow edge, not at the core edge.
    let knob_centre = f32::midpoint(knob_left.to_f32(), knob_right.to_f32());
    let shadow_half = right.knob_shadow_width * 0.5;
    assert!(
        right_blue.first().expect("blue bar").to_f32() - knob_centre - shadow_half <= 2.0,
        "positive fill must meet the knob shadow: core {knob_left}..{knob_right}, shadow width {}, blue {right_blue:?}",
        right.knob_shadow_width
    );
    assert!(
        knob_centre - shadow_half - left_blue.last().expect("blue bar").to_f32() <= 2.0,
        "negative fill must meet the knob shadow: core {knob_left}..{knob_right}, shadow width {}, blue {left_blue:?}",
        left.knob_shadow_width
    );
    let value_x = |value: f32| {
        value.mul_add(
            right.track.width - right.travel_thumb_width,
            right.travel_thumb_width.mul_add(0.5, right.track.x),
        )
    };
    assert!(
        (right_blue.last().expect("blue bar").to_f32() - value_x(0.8)).abs() <= 2.0,
        "positive bar misses the live value"
    );
    assert!(
        (left_blue.first().expect("blue bar").to_f32() - value_x(0.2)).abs() <= 2.0,
        "negative bar misses the live value"
    );
    Ok(())
}

fn drawn_band_ends(
    picture: &SliderFacePicture,
    bare: &SliderFacePicture,
) -> Result<(f32, f32), String> {
    let changed = picture
        .columns()
        .filter(|&x| {
            rgb(&picture.pixels, x, picture.centre_row())
                .iter()
                .zip(rgb(&bare.pixels, x, bare.centre_row()))
                .any(|(drawn, bare)| drawn.abs_diff(bare) >= 3)
        })
        .collect::<Vec<_>>();
    Ok((
        changed
            .first()
            .ok_or("reachable band not visible")?
            .to_f32(),
        changed.last().ok_or("reachable band not visible")?.to_f32(),
    ))
}

#[test]
fn reachable_slider_band_halves_with_depth_and_clamps_at_track_end() -> Result<(), String> {
    let opaque_fill = cabled_slider_fill_reference()?;
    let bare = capture_pushed_slider_face(
        0.5,
        SliderPushedValue::PushedTo {
            value:      0.5,
            band_start: 0.5,
            band_end:   0.5,
        },
    )?;
    let full = capture_pushed_slider_face(
        0.5,
        SliderPushedValue::PushedTo {
            value:      0.5,
            band_start: 0.0,
            band_end:   1.0,
        },
    )?;
    let half = capture_pushed_slider_face(
        0.5,
        SliderPushedValue::PushedTo {
            value:      0.5,
            band_start: 0.25,
            band_end:   0.75,
        },
    )?;
    let (full_left, full_right) = drawn_band_ends(&full, &bare)?;
    let (half_left, half_right) = drawn_band_ends(&half, &bare)?;
    assert!(
        (half_right - half_left)
            .mul_add(-2.0, full_right - full_left)
            .abs()
            <= 3.0,
        "depth 1 band {full_left}..{full_right} does not halve at depth 0.5: {half_left}..{half_right}"
    );
    assert!(
        full.live_fill_columns(opaque_fill).is_empty()
            && half.live_fill_columns(opaque_fill).is_empty(),
        "reachable band is faint and distinct from the live blue fill"
    );

    let end_bare = capture_pushed_slider_face(
        0.9,
        SliderPushedValue::PushedTo {
            value:      0.9,
            band_start: 0.9,
            band_end:   0.9,
        },
    )?;
    let end = capture_pushed_slider_face(
        0.9,
        SliderPushedValue::PushedTo {
            value:      0.9,
            band_start: 0.4,
            band_end:   1.0,
        },
    )?;
    let (end_left, end_right) = drawn_band_ends(&end, &end_bare)?;
    assert!(
        (end_right - full_right).abs() <= 2.0,
        "reachable band does not stop at the track end"
    );
    assert!(
        (full_right - full_left)
            .mul_add(-0.6, end_right - end_left)
            .abs()
            <= 3.0,
        "end-clamped band does not show setting 0.9 ± depth 1 half-range"
    );
    assert!(
        end_left >= end.track.x && end_right <= end.track.x + end.track.width,
        "reachable band escapes the drawn track"
    );
    Ok(())
}

#[test]
fn uncabled_slider_draws_the_existing_authored_fill_without_a_band() -> Result<(), String> {
    let existing = capture_face(SliderReachableBand::Hidden)?;
    let uncabled = capture_pushed_slider_face(0.5, SliderPushedValue::AtHandle)?;
    assert_eq!(
        uncabled.pixels, existing,
        "an unused cable presentation must preserve the authored slider picture"
    );
    Ok(())
}

#[test]
fn slider_value_field_draws_fill_hairline_and_right_aligned_digit() -> Result<(), String> {
    let pixels = capture_value_field(HANA_JACK_PANEL_BACKGROUND, "value_field")?;
    let centre_y = FIELD_IMAGE_HEIGHT / 2;
    let pixel = |x, y| rgb_at(&pixels, FIELD_IMAGE_WIDTH, x, y);
    let backdrop = pixel(30, centre_y);
    let field: Vec<u32> = (FIELD_IMAGE_WIDTH / 2 - 100..FIELD_IMAGE_WIDTH / 2 + 100)
        .filter(|&x| {
            pixel(x, centre_y)
                .iter()
                .zip(backdrop)
                .any(|(a, b)| a.abs_diff(b) > 5)
        })
        .collect();
    let left = *field.first().ok_or("value field did not draw")?;
    let right = *field.last().ok_or("value field did not draw")?;
    let centre_x = left.midpoint(right);
    let rows: Vec<u32> = (centre_y - 80..centre_y + 80)
        .filter(|&y| {
            pixel(centre_x, y)
                .iter()
                .zip(backdrop)
                .any(|(a, b)| a.abs_diff(b) > 5)
        })
        .collect();
    let top = *rows.first().ok_or("value field did not draw")?;
    let bottom = *rows.last().ok_or("value field did not draw")?;
    let height = rows.len();
    let metrics = ControlMetrics::from_label_size(LABEL_PIXELS);
    let fill = pixel(centre_x, centre_y);
    let border = pixel(left, centre_y);
    let ink: Vec<(u32, u32)> = (centre_y - 50..centre_y + 50)
        .flat_map(|y| (left + 2..right - 1).map(move |x| (x, y)))
        .filter(|&(x, y)| luminance(pixel(x, y)) > 0.35)
        .collect();
    let ink_left = ink
        .iter()
        .map(|(x, _)| *x)
        .min()
        .ok_or("value digit did not draw")?;
    let ink_right = ink
        .iter()
        .map(|(x, _)| *x)
        .max()
        .ok_or("value digit did not draw")?;
    let left_gap = ink_left - left;
    let right_gap = right - ink_right;
    let profiles = [
        (left..left + 6)
            .map(|x| pixel(x, centre_y))
            .collect::<Vec<_>>(),
        (right - 5..=right)
            .rev()
            .map(|x| pixel(x, centre_y))
            .collect(),
        (top..top + 6).map(|y| pixel(centre_x, y)).collect(),
        (bottom - 5..=bottom)
            .rev()
            .map(|y| pixel(centre_x, y))
            .collect(),
    ];
    let full_coverage_widths: Vec<usize> = profiles
        .iter()
        .map(|profile| {
            let brightest = profile.iter().copied().map(luminance).fold(0.0, f32::max);
            profile
                .iter()
                .copied()
                .map(luminance)
                .filter(|sample| (brightest - sample).abs() <= 0.01)
                .count()
        })
        .collect();
    eprintln!(
        "value field x={left}..={right}, y={top}..={bottom}, height {height}, backdrop {backdrop:?}, fill {fill:?}, border {border:?}, full-coverage widths {full_coverage_widths:?}, digit x={ink_left}..={ink_right}, gaps {left_gap}/{right_gap}"
    );
    assert!(
        (right - left + 1).abs_diff(metrics.slider.value_width.round().to_u32()) <= 2,
        "value field width does not follow slider.value_width"
    );
    assert!(
        (height.to_f32() - metrics.field.height).abs() <= 2.0,
        "value field height does not follow field.height"
    );
    assert!(
        luminance(fill) < luminance(backdrop),
        "value field has no inset fill"
    );
    assert!(
        luminance(border) > luminance(fill),
        "value field has no bright border hairline"
    );
    assert_eq!(
        full_coverage_widths,
        [1, 1, 1, 1],
        "value field outline is not one full-coverage pixel on every side"
    );
    assert!(
        left_gap > right_gap + 4,
        "value digit is not right-aligned inside its field"
    );
    Ok(())
}

#[test]
fn slider_value_field_outline_reaches_three_to_one_on_every_host_panel() -> Result<(), String> {
    let mut failures = Vec::new();
    for (name, background) in [
        ("hana_jack_panel", HANA_JACK_PANEL_BACKGROUND),
        ("hana_shape_panel", HANA_SHAPE_PANEL_BACKGROUND),
        ("widgets_example_panel", WIDGETS_EXAMPLE_PANEL_BACKGROUND),
    ] {
        let pixels = capture_value_field(background, name)?;
        let centre_y = FIELD_IMAGE_HEIGHT / 2;
        let pixel = |x, y| rgb_at(&pixels, FIELD_IMAGE_WIDTH, x, y);
        let distant_panel = pixel(30, centre_y);
        let field: Vec<u32> = (FIELD_IMAGE_WIDTH / 2 - 100..FIELD_IMAGE_WIDTH / 2 + 100)
            .filter(|&x| {
                pixel(x, centre_y)
                    .iter()
                    .zip(distant_panel)
                    .any(|(field, panel)| field.abs_diff(panel) > 5)
            })
            .collect();
        let left = *field.first().ok_or("value field did not draw")?;
        let outline = pixel(left, centre_y);
        let panel = pixel(left - 2, centre_y);
        let ratio = contrast(outline, panel);
        eprintln!("{name}: outline {outline:?}, adjacent panel {panel:?}, contrast {ratio:.2}:1");
        if ratio < 3.0 {
            failures.push(format!(
                "{name} value-field outline contrast {ratio:.2}:1 falls below 3:1"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}
