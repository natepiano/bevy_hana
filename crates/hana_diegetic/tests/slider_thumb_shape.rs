//! Solved slider geometry for the macOS thumb and capsule track proportions.
#![allow(
    clippy::expect_used,
    reason = "invalid test fixtures should fail immediately"
)]

use bevy::camera::Hdr;
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::PerspectiveProjection;
use bevy::camera::Projection;
use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::ecs::system::RunSystemOnce;
use bevy::picking::backend::HitData;
use bevy::picking::events::Pointer;
use bevy::picking::events::PointerDrag;
use bevy::picking::events::PointerPress;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerButton;
use bevy::picking::pointer::PointerId;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::App;
use bevy::prelude::Camera;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::Msaa;
use bevy::prelude::Transform;
use bevy::prelude::UVec2;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::render::view::Tonemapping;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ControlMetrics;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::Sizing;
use hana_diegetic::Slider;
use hana_diegetic::SliderDirection;
use hana_diegetic::SliderState;
use hana_diegetic::slider_self_update;

mod drawn_support;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

const TOLERANCE: f32 = 1e-3;
const VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);
const POINTER: PointerId = PointerId::Touch(11);
const IMAGE_SIZE: u32 = 512;

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < TOLERANCE,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn metric_sized_slider_solves_a_wide_thumb_and_capsule_track_at_both_ends() {
    let metrics = ControlMetrics::from_label_size(13.0_f32);
    let tree = slider_tree(&metrics);
    let panel = DiegeticPanel::world()
        .size(Mm(200.0), Mm(40.0))
        .world_height(0.5)
        .with_tree(tree.clone())
        .build()
        .expect("slider panel should build");
    let (mut app, camera, target) = app_with_pointer();
    let panel = app.world_mut().spawn(panel).id();
    app.update();

    let computed = app
        .world()
        .get::<ComputedDiegeticPanel>(panel)
        .expect("panel should solve");
    let slider = computed.content_bounds().expect("slider box should solve");
    let record = |id: &'static str| {
        computed
            .scroll_records()
            .iter()
            .find(|record| record.id == PanelElementId::named(id))
            .expect("named slider part should solve")
    };
    let track_record = record("track");
    let thumb_record = record("thumb");
    let track = track_record.bounds;
    let thumb = thumb_record.bounds;

    close(thumb.width / thumb.height, 52.0 / 32.0);
    close(thumb.height / track.height, 12.0 / 6.0);
    close(
        tree.element_corner_radius(thumb_record.element_index)
            .top_left
            .value,
        metrics.slider.thumb.height * 0.5,
    );
    close(
        tree.element_corner_radius(track_record.element_index)
            .top_left
            .value,
        metrics.slider.track_radius(),
    );
    close(
        metrics.slider.track_radius(),
        metrics.slider.track_height * 0.5,
    );
    // The laid-out track fills the slider box. The thumb's presentation
    // translation, checked by the pixel test below, keeps it inside the ends.
    close(thumb.x, slider.x);
    close(track.x, slider.x);
    close(track.width, slider.width);
    assert!(thumb.y >= slider.y - TOLERANCE);
    assert!(thumb.y + thumb.height <= slider.y + slider.height + TOLERANCE);

    let widget = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named("level"))
        })
        .expect("widget lookup should run")
        .expect("slider widget should reify");
    let location = |position| Location {
        target: target.clone(),
        position,
    };
    app.world_mut().trigger(PointerPress {
        entity:  widget,
        pointer: Pointer::new(POINTER, location(VIEWPORT * 0.5)),
        button:  PointerButton::Primary,
        hit:     HitData::new(camera, 0.0, None, None),
        count:   1,
    });
    app.world_mut().flush();
    drag_to(&mut app, widget, location(Vec2::new(0.0, VIEWPORT.y * 0.5)));
    assert_slider_value(&app, widget, 0.0);
    drag_to(
        &mut app,
        widget,
        location(Vec2::new(VIEWPORT.x, VIEWPORT.y * 0.5)),
    );
    assert_slider_value(&app, widget, 1.0);
}

fn slider_tree(metrics: &ControlMetrics<f32>) -> LayoutTree {
    let mut builder = LayoutBuilder::new(260.0, 40.0);
    builder.with(
        El::overlay()
            .size(metrics.slider.width, metrics.slider.thumb.height)
            .alignment(AlignX::Left, AlignY::Center)
            .widget("level", Slider::new(0.0..=1.0).value(0.0)),
        |builder| {
            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(metrics.slider.thumb.height))
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    builder.with(
                        builder
                            .child(El::overlay())
                            .id("track")
                            .width(Sizing::GROW)
                            .height(Sizing::fixed(metrics.slider.track_height))
                            .corner_radius(CornerRadius::all(metrics.slider.track_radius()))
                            .scroll_x(0.0),
                        |builder| {
                            builder.with(El::new().size(1.0, 1.0), |_| {});
                        },
                    );
                },
            );
            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.with(
                        builder
                            .child(El::overlay())
                            .id("thumb")
                            .size(metrics.slider.thumb.width, metrics.slider.thumb.height)
                            .corner_radius(CornerRadius::all(metrics.slider.thumb.corner_radius()))
                            .scroll_x(0.0)
                            .slider_thumb(),
                        |builder| {
                            builder.with(El::new().size(1.0, 1.0), |_| {});
                        },
                    );
                },
            );
        },
    );
    builder.build()
}

fn app_with_pointer() -> (App, Entity, NormalizedRenderTarget) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(DiegeticTextMeasurer::default())
        .add_plugins(HeadlessDiegeticUiPlugin)
        .add_observer(slider_self_update);
    app.finish();
    let window = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: VIEWPORT.as_uvec2(),
        scale_factor:  1.0,
    });
    let mut projection = Projection::Perspective(PerspectiveProjection::default());
    projection.update(VIEWPORT.x, VIEWPORT.y);
    camera.computed.clip_from_view = projection.get_clip_from_view();
    let transform = Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
    let camera = app
        .world_mut()
        .spawn((
            camera,
            transform,
            GlobalTransform::from(transform),
            RenderTarget::Window(WindowRef::Primary),
        ))
        .id();
    let target = RenderTarget::Window(WindowRef::Primary)
        .normalize(Some(window))
        .expect("primary window should normalize");
    (app, camera, target)
}

fn drag_to(app: &mut App, widget: Entity, location: Location) {
    app.world_mut().trigger(PointerDrag {
        entity:   widget,
        pointer:  Pointer::new(POINTER, location),
        button:   PointerButton::Primary,
        distance: Vec2::ZERO,
        delta:    Vec2::ZERO,
    });
    app.world_mut().flush();
    app.update();
}

fn assert_slider_value(app: &App, widget: Entity, value: f32) {
    close(
        app.world()
            .get::<SliderState>(widget)
            .expect("slider state should remain")
            .value(),
        value,
    );
}

fn rendered_slider_tree(direction: SliderDirection) -> LayoutTree {
    let metrics = ControlMetrics::from_label_size(13.0_f32);
    let horizontal = matches!(
        direction,
        SliderDirection::LeftToRight | SliderDirection::RightToLeft
    );
    let (width, height) = if horizontal {
        (metrics.slider.width, metrics.slider.thumb.height)
    } else {
        (metrics.slider.thumb.width, metrics.slider.width)
    };
    let (track_width, track_height) = if horizontal {
        (width, metrics.slider.track_height)
    } else {
        (metrics.slider.track_height, height)
    };
    let mut builder = LayoutBuilder::new(200.0, 200.0);
    builder.with(
        El::overlay()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.with(
                El::overlay()
                    .size(width, height)
                    .alignment(AlignX::Center, AlignY::Center)
                    .widget(
                        "level",
                        Slider::new(0.0..=1.0).value(0.0).direction(direction),
                    ),
                |builder| {
                    builder.with(
                        El::new()
                            .id("track")
                            .size(track_width, track_height)
                            .background(Color::srgb(0.1, 0.45, 0.9))
                            .corner_radius(CornerRadius::all(metrics.slider.track_radius())),
                        |_| {},
                    );
                    builder.with(
                        El::new()
                            .id("thumb")
                            .size(metrics.slider.thumb.width, metrics.slider.thumb.height)
                            .background(Color::WHITE)
                            .corner_radius(CornerRadius::all(metrics.slider.thumb.corner_radius()))
                            .slider_thumb(),
                        |_| {},
                    );
                },
            );
        },
    );
    builder.build()
}

fn render_slider(direction: SliderDirection) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(
        UVec2::splat(IMAGE_SIZE),
        UVec2::ZERO,
        UVec2::splat(IMAGE_SIZE),
    );
    let target = app.target(region);
    let bloom = Bloom {
        prefilter: BloomPrefilter {
            threshold:          1.0,
            threshold_softness: 0.1,
        },
        ..Default::default()
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
    app.world_mut().spawn((
        Camera3d::default(),
        image_camera,
        RenderTarget::Image(target.clone().into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 0.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let panel = DiegeticPanel::world()
        .size(Mm(200.0), Mm(200.0))
        .anchor(Anchor::Center)
        .with_tree(rendered_slider_tree(direction))
        .build()
        .map_err(|error| format!("invalid slider panel: {error}"))?;
    let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    app.settle()?;
    let widget = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(panel, &PanelElementId::named("level"))
        })
        .map_err(|error| format!("widget lookup failed: {error}"))?
        .ok_or("slider widget did not reify")?;
    let mut images = Vec::new();
    for value in [0.0, 1.0] {
        app.world_mut()
            .get_mut::<SliderState>(widget)
            .ok_or("slider state did not reify")?
            .set_value(value)
            .map_err(|error| format!("slider value rejected: {error}"))?;
        for _ in 0..4 {
            app.update();
        }
        images.push(app.capture(target.clone())?);
    }
    Ok((images.remove(0), images.remove(0)))
}

enum SliderPart {
    Track,
    Thumb,
}

fn colored_axis_bounds(
    pixels: &[u8],
    direction: SliderDirection,
    part: SliderPart,
) -> Result<(u32, u32), String> {
    let mut first = IMAGE_SIZE;
    let mut last = 0;
    for y in 0..IMAGE_SIZE {
        for x in 0..IMAGE_SIZE {
            let offset = ((y * IMAGE_SIZE + x) * 4) as usize;
            let rgb = pixels
                .get(offset..offset + 3)
                .ok_or("screenshot has no slider pixels")?;
            let belongs = match part {
                SliderPart::Thumb => rgb.iter().all(|channel| *channel > 170),
                SliderPart::Track => {
                    rgb[2] > rgb[0].saturating_add(30) && rgb[2] > rgb[1].saturating_add(20)
                },
            };
            if belongs {
                let coordinate = match direction {
                    SliderDirection::LeftToRight | SliderDirection::RightToLeft => x,
                    SliderDirection::BottomToTop | SliderDirection::TopToBottom => y,
                };
                first = first.min(coordinate);
                last = last.max(coordinate);
            }
        }
    }
    if first >= last {
        let name = match part {
            SliderPart::Track => "track",
            SliderPart::Thumb => "thumb",
        };
        return Err(format!("{name} was not drawn"));
    }
    Ok((first, last))
}

fn dump_slider_pixels(case: &str, pixels: &[u8]) -> Result<(), String> {
    dump_pixels::<true>(
        "slider_thumb_shape",
        case,
        &format!(
            "px_per_em={:.9}",
            f64::from(IMAGE_SIZE) / (45.0_f64.to_radians() * 0.5).tan()
        ),
        CaptureRegion::new(
            UVec2::splat(IMAGE_SIZE),
            UVec2::ZERO,
            UVec2::splat(IMAGE_SIZE),
        ),
        CaptureRegion::new(
            UVec2::splat(IMAGE_SIZE),
            UVec2::ZERO,
            UVec2::splat(IMAGE_SIZE),
        ),
        pixels,
    )
}

fn check_slider_direction(direction: SliderDirection) -> Result<(), String> {
    let (minimum, maximum) = render_slider(direction)?;
    dump_slider_pixels(&format!("{direction:?}_minimum"), &minimum)?;
    dump_slider_pixels(&format!("{direction:?}_maximum"), &maximum)?;
    let track_min = colored_axis_bounds(&minimum, direction, SliderPart::Track)?;
    let track_max = colored_axis_bounds(&maximum, direction, SliderPart::Track)?;
    let track = (track_min.0.min(track_max.0), track_min.1.max(track_max.1));
    for (value, pixels) in [(0.0, &minimum), (1.0, &maximum)] {
        let thumb = colored_axis_bounds(pixels, direction, SliderPart::Thumb)?;
        eprintln!("slider {direction:?} value={value}: drawn track={track:?}, thumb={thumb:?}");
        assert!(
            thumb.0 >= track.0 && thumb.1 <= track.1,
            "slider {direction:?} value={value}: drawn track={track:?}, thumb={thumb:?}"
        );
        let (track_end, thumb_end) = match (direction, value) {
            (SliderDirection::LeftToRight | SliderDirection::TopToBottom, 0.0)
            | (SliderDirection::RightToLeft | SliderDirection::BottomToTop, 1.0) => {
                (track.0, thumb.0)
            },
            _ => (track.1, thumb.1),
        };
        assert!(
            track_end.abs_diff(thumb_end) <= 1,
            "slider {direction:?} value={value}: drawn track={track:?}, thumb={thumb:?}, expected edge={track_end}, drawn edge={thumb_end}"
        );
    }
    Ok(())
}

#[test]
fn drawn_slider_thumb_reaches_each_track_end_in_every_direction() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "left_to_right_thumb_reaches_both_track_ends",
            run:  || check_slider_direction(SliderDirection::LeftToRight),
        },
        DrawnCase {
            name: "right_to_left_thumb_reaches_both_track_ends",
            run:  || check_slider_direction(SliderDirection::RightToLeft),
        },
        DrawnCase {
            name: "bottom_to_top_thumb_reaches_both_track_ends",
            run:  || check_slider_direction(SliderDirection::BottomToTop),
        },
        DrawnCase {
            name: "top_to_bottom_thumb_reaches_both_track_ends",
            run:  || check_slider_direction(SliderDirection::TopToBottom),
        },
    ])
}
