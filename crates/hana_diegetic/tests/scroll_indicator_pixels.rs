//! Scroll-indicator ink checked against public records and ancestor clipping.

use std::time::Duration;

use bevy::camera::Hdr;
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::ecs::message::Messages;
use bevy::input::mouse::MouseScrollUnit;
use bevy::input::touch::TouchPhase;
use bevy::picking::PickingSettings;
use bevy::picking::backend::HitData;
use bevy::picking::events::pointer_events;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerAction;
use bevy::picking::pointer::PointerId;
use bevy::picking::pointer::PointerInput;
use bevy::picking::pointer::PointerLocation;
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use bevy::time::TimeUpdateStrategy;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::BoundingBox;
use hana_diegetic::Column;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ComputedScrollRecord;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::Pt;
use hana_diegetic::ScrollIndicatorState;
use hana_diegetic::Sizing;
use hana_kana::ToF32;
use hana_kana::ToU32;
use hana_kana::ToUsize;

mod drawn_support;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

const SIZE: u32 = 256;
const PANEL_SIZE: f32 = 100.0;
const PANEL_ORIGIN: f32 = 28.0;
const PIXELS_PER_POINT: f32 = 2.0;
const BACKGROUND: u8 = 24;
const COLOUR_TOLERANCE: u8 = 6;
const POINTER: PointerId = PointerId::Mouse;

struct IndicatorScene {
    app:    CaseApp,
    panel:  Entity,
    target: Handle<Image>,
    region: CaptureRegion,
}

impl IndicatorScene {
    fn new(rows: usize, clipped: bool) -> Result<Self, String> {
        Self::with_view(
            rows,
            clipped,
            Color::srgb_u8(BACKGROUND, BACKGROUND, BACKGROUND),
            El::column(),
        )
    }

    fn with_view(
        rows: usize,
        clipped: bool,
        background: Color,
        view: El<Column>,
    ) -> Result<Self, String> {
        let mut app = CaseApp::new();
        // Paused fixed time retains messages; a fresh cached reader must not
        // replay the previous scene's wheel input against this scene's panel.
        app.world_mut()
            .resource_mut::<Messages<PointerInput>>()
            .clear();
        app.insert_resource(PickingSettings {
            is_hover_enabled: false,
            ..default()
        });
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 120.0,
        )));
        let region = CaptureRegion::new(UVec2::splat(SIZE), UVec2::ZERO, UVec2::splat(SIZE));
        let target = app.target(region);
        let mut window = Window::default();
        window.resolution.set(SIZE.to_f32(), SIZE.to_f32());
        app.world_mut().spawn((window, PrimaryWindow));
        // A window camera selects the display table, as in panel_fill_colour.
        app.world_mut().spawn((
            Camera3d::default(),
            RenderTarget::Window(WindowRef::Primary),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
        ));
        let mut camera = Camera::default();
        region.apply_to_camera(&mut camera);
        app.world_mut().spawn((
            Camera3d::default(),
            camera,
            RenderTarget::Image(target.clone().into()),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: 0.128,
                },
                ..OrthographicProjection::default_3d()
            }),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            Msaa::Off,
            Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let mut builder = LayoutBuilder::with_root(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(background),
        );
        let populate = |builder: &mut LayoutBuilder| {
            builder.with(
                view.width(Sizing::GROW)
                    .height(Sizing::fixed(PANEL_SIZE))
                    .scroll_view("body"),
                |builder| {
                    for _ in 0..rows {
                        builder.with(
                            El::new().width(Sizing::GROW).height(Sizing::fixed(20.0)),
                            |_| {},
                        );
                    }
                },
            );
        };
        if clipped {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(50.0))
                    .clip(),
                populate,
            );
        } else {
            populate(&mut builder);
        }
        let panel = DiegeticPanel::world()
            .size(Pt(PANEL_SIZE), Pt(PANEL_SIZE))
            .world_width(0.1)
            .anchor(Anchor::Center)
            .with_tree(builder.build())
            .build()
            .map_err(|error| error.to_string())?;
        let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
        app.world_mut()
            .spawn((POINTER, PointerLocation::new(Self::location())));
        app.settle()?;
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        Ok(Self {
            app,
            panel,
            target,
            region,
        })
    }

    fn location() -> Location {
        Location {
            target:   NormalizedRenderTarget::None {
                width:  SIZE,
                height: SIZE,
            },
            position: Vec2::splat(SIZE.to_f32() / 2.0),
        }
    }

    fn record(&self) -> Result<ComputedScrollRecord, String> {
        self.app
            .world()
            .get::<ComputedDiegeticPanel>(self.panel)
            .and_then(|computed| {
                computed
                    .scroll_records()
                    .iter()
                    .find(|record| record.id == PanelElementId::named("body"))
            })
            .cloned()
            .ok_or_else(|| "missing drawn scroll record".to_string())
    }

    fn scroll(&mut self, distance: f32) -> Result<(), String> {
        self.dispatch_pixels(distance, TouchPhase::Moved)
    }

    fn dispatch_pixels(&mut self, distance: f32, phase: TouchPhase) -> Result<(), String> {
        let record = self.record()?;
        // The top quarter stays inside the clipped ancestor in the clipping case.
        let point = Vec2::new(
            record.bounds.x + record.bounds.width / 2.0,
            record.bounds.y + record.bounds.height / 4.0,
        );
        let panel = self
            .app
            .world()
            .get::<DiegeticPanel>(self.panel)
            .ok_or("missing panel")?;
        let transform = self
            .app
            .world()
            .get::<GlobalTransform>(self.panel)
            .ok_or("missing panel transform")?;
        let scale = panel.points_to_world();
        let (anchor_x, anchor_y) = panel.anchor_offsets();
        let position = transform.transform_point(Vec3::new(
            point.x.mul_add(scale, -anchor_x),
            point.y.mul_add(-scale, anchor_y),
            0.0,
        ));
        self.app.world_mut().resource_mut::<HoverMap>().insert(
            POINTER,
            std::iter::once((
                self.panel,
                HitData::new(Entity::PLACEHOLDER, 0.0, Some(position), None),
            ))
            .collect(),
        );
        self.app.world_mut().write_message(PointerInput::new(
            POINTER,
            Self::location(),
            PointerAction::Scroll {
                unit: MouseScrollUnit::Pixel,
                x: 0.0,
                y: -distance,
                phase,
            },
        ));
        self.app
            .world_mut()
            .run_system_cached(pointer_events)
            .map_err(|error| error.to_string())?;
        self.app.world_mut().flush();
        // A zero-time solve exposes the 1:1 input before the hold clock advances.
        self.app.update();
        Ok(())
    }

    fn advance(&mut self, frames: usize) {
        self.app
            .world_mut()
            .resource_mut::<Time<Virtual>>()
            .unpause();
        for _ in 0..frames {
            self.app.update();
        }
        self.app.world_mut().resource_mut::<Time<Virtual>>().pause();
        // Clear the last nonzero delta before readback adds its own frames.
        self.app.update();
    }

    fn capture(&mut self) -> Result<Vec<u8>, String> {
        let record = self.record()?;
        let pixels = self.app.capture(self.target.clone())?;
        dump_pixels::<true>(
            "scroll_indicator_pixels",
            "",
            &format!("record={record:?}"),
            self.region,
            self.region,
            &pixels,
        )?;
        Ok(pixels)
    }
}

/// Colour buckets tolerate llvmpipe fill dithering without counting it as ink.
fn grey_bucket(pixel: &[u8]) -> i16 {
    let strongest = pixel[..3].iter().copied().max().unwrap_or(0);
    let weakest = pixel[..3].iter().copied().min().unwrap_or(0);
    assert!(
        strongest.abs_diff(weakest) <= COLOUR_TOLERANCE,
        "indicator fixture produced non-grey ink: {pixel:?}"
    );
    (i16::from(pixel[0]) + i16::from(pixel[1]) + i16::from(pixel[2]))
        / 3
        / i16::from(COLOUR_TOLERANCE)
}

fn ink(before: &[u8], after: &[u8]) -> Vec<UVec2> {
    assert_eq!(before.len(), after.len());
    before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0)
        .enumerate()
        .filter_map(|(index, (before, after))| {
            (grey_bucket(after) - grey_bucket(before) > 1)
                .then_some(UVec2::new(index.to_u32() % SIZE, index.to_u32() / SIZE))
        })
        .collect()
}

fn thumb_pixels(record: &ComputedScrollRecord) -> Result<BoundingBox, String> {
    match &record.indicator {
        ScrollIndicatorState::Hidden => Err("drawn indicator has no recorded thumb".to_string()),
        ScrollIndicatorState::Shown { thumb, .. } => Ok(BoundingBox {
            x:      thumb.x.mul_add(PIXELS_PER_POINT, PANEL_ORIGIN),
            y:      thumb.y.mul_add(PIXELS_PER_POINT, PANEL_ORIGIN),
            width:  thumb.width * PIXELS_PER_POINT,
            height: thumb.height * PIXELS_PER_POINT,
        }),
    }
}

fn ink_bounds(ink: &[UVec2]) -> Result<BoundingBox, String> {
    let first = ink.first().ok_or("indicator draws no detectable ink")?;
    let (mut min, mut max) = (*first, *first);
    for point in ink {
        min = min.min(*point);
        max = max.max(*point);
    }
    Ok(BoundingBox {
        x:      min.x.to_f32(),
        y:      min.y.to_f32(),
        width:  (max.x - min.x + 1).to_f32(),
        height: (max.y - min.y + 1).to_f32(),
    })
}

fn close_pixel(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.0,
        "indicator ink edge={actual}, record edge={expected}"
    );
}

fn fitting_ink() -> Result<(), String> {
    let mut scene = IndicatorScene::new(2, false)?;
    let before = scene.capture()?;
    scene.scroll(40.0)?;
    scene.advance(140);
    let after = scene.capture()?;
    assert_eq!(ink(&before, &after), Vec::<UVec2>::new());
    assert!(matches!(
        scene.record()?.indicator,
        ScrollIndicatorState::Hidden
    ));
    let strip = (PANEL_SIZE - 5.0)
        .mul_add(PIXELS_PER_POINT, PANEL_ORIGIN)
        .to_u32();
    for y in 32..224 {
        for x in strip..strip + 6 {
            let index = ((y * SIZE + x) * 4).to_usize();
            for channel in &before[index..index + 3] {
                assert!(
                    channel.abs_diff(BACKGROUND) <= COLOUR_TOLERANCE,
                    "fitting view draws indicator ink at {x},{y}: {}",
                    before[index]
                );
            }
        }
    }
    Ok(())
}

fn before_first_scroll() -> Result<(), String> {
    let mut scene = IndicatorScene::new(20, false)?;
    assert!(matches!(
        scene.record()?.indicator,
        ScrollIndicatorState::Hidden
    ));
    let pixels = scene.capture()?;
    let strip = (PANEL_SIZE - 5.0)
        .mul_add(PIXELS_PER_POINT, PANEL_ORIGIN)
        .to_u32();
    for y in 32..224 {
        for x in strip..strip + 6 {
            let index = ((y * SIZE + x) * 4).to_usize();
            assert!(
                pixels[index].abs_diff(BACKGROUND) <= COLOUR_TOLERANCE,
                "resting indicator ink at {x},{y}: {}",
                pixels[index]
            );
        }
    }
    Ok(())
}

fn recorded_thumb() -> Result<(), String> {
    let mut scene = IndicatorScene::new(20, false)?;
    let before = scene.capture()?;
    scene.scroll(30.0)?;
    let expected = thumb_pixels(&scene.record()?)?;
    let after = scene.capture()?;
    let actual = ink_bounds(&ink(&before, &after))?;
    close_pixel(actual.x, expected.x);
    close_pixel(actual.y, expected.y);
    close_pixel(actual.x + actual.width, expected.x + expected.width);
    close_pixel(actual.y + actual.height, expected.y + expected.height);
    // The thumb core is brighter than the panel and stays mid-grey.
    let core = UVec2::new(
        (expected.x + expected.width / 2.0).to_u32(),
        (expected.y + expected.height / 2.0).to_u32(),
    );
    let index = ((core.y * SIZE + core.x) * 4).to_usize();
    assert!(grey_bucket(&after[index..index + 4]) > grey_bucket(&before[index..index + 4]) + 1);
    Ok(())
}

fn moves_down() -> Result<(), String> {
    let mut scene = IndicatorScene::new(20, false)?;
    let before = scene.capture()?;
    scene.scroll(30.0)?;
    let first_record = thumb_pixels(&scene.record()?)?;
    let first = ink_bounds(&ink(&before, &scene.capture()?))?;
    scene.scroll(120.0)?;
    let second_record = thumb_pixels(&scene.record()?)?;
    let second = ink_bounds(&ink(&before, &scene.capture()?))?;
    assert!(second.y > first.y);
    close_pixel(second.y - first.y, second_record.y - first_record.y);
    close_pixel(first.height, second.height);
    Ok(())
}

fn fades_away() -> Result<(), String> {
    let mut scene = IndicatorScene::new(20, false)?;
    let before = scene.capture()?;
    scene.scroll(30.0)?;
    scene.dispatch_pixels(0.0, TouchPhase::Ended)?;
    let full = scene.capture()?;
    let expected = thumb_pixels(&scene.record()?)?;
    scene.advance(84);
    let held = scene.capture()?;
    assert_eq!(
        ink_bounds(&ink(&before, &full))?,
        ink_bounds(&ink(&before, &held))?
    );
    scene.advance(27);
    let fading = scene.capture()?;
    let core = UVec2::new(
        (expected.x + expected.width / 2.0).to_u32(),
        (expected.y + expected.height / 2.0).to_u32(),
    );
    let index = ((core.y * SIZE + core.x) * 4).to_usize();
    let baseline = grey_bucket(&before[index..index + 4]);
    let full = grey_bucket(&full[index..index + 4]);
    let fading = grey_bucket(&fading[index..index + 4]);
    assert!(
        fading > baseline && fading < full,
        "baseline={baseline}, fading={fading}, full={full}"
    );
    scene.advance(30);
    assert!(matches!(
        scene.record()?.indicator,
        ScrollIndicatorState::Hidden
    ));
    assert_eq!(ink(&before, &scene.capture()?), Vec::<UVec2>::new());
    Ok(())
}

fn clipped_ink() -> Result<(), String> {
    let mut scene = IndicatorScene::new(20, true)?;
    let before = scene.capture()?;
    scene.scroll(150.0)?;
    let expected = thumb_pixels(&scene.record()?)?;
    let clip_bottom = 50.0_f32.mul_add(PIXELS_PER_POINT, PANEL_ORIGIN);
    assert!(
        expected.y < clip_bottom && expected.y + expected.height > clip_bottom,
        "fixture must make the thumb cross the ancestor clip: {expected:?}"
    );
    let changed = ink(&before, &scene.capture()?);
    assert!(!changed.is_empty(), "clipping hid the whole indicator");
    for pixel in changed {
        assert!(
            pixel.y.to_f32() < clip_bottom,
            "indicator leaks below ancestor at {pixel:?}, clip_bottom={clip_bottom}"
        );
        assert!(
            pixel.x.to_f32() >= expected.x - 1.0
                && pixel.x.to_f32() < expected.x + expected.width + 1.0,
            "indicator leaks outside view at {pixel:?}"
        );
    }
    Ok(())
}

fn padded_border_inset() -> Result<(), String> {
    let mut scene = IndicatorScene::with_view(
        20,
        false,
        Color::srgb_u8(BACKGROUND, BACKGROUND, BACKGROUND),
        El::column().padding(Padding::all(8.0)),
    )?;
    let before = scene.capture()?;
    scene.scroll(0.0)?;
    let first = ink_bounds(&ink(&before, &scene.capture()?))?;
    close_pixel(
        first.x + first.width,
        (PANEL_SIZE - 2.0).mul_add(PIXELS_PER_POINT, PANEL_ORIGIN),
    );
    close_pixel(first.y, 2.0_f32.mul_add(PIXELS_PER_POINT, PANEL_ORIGIN));
    let max = scene.record()?.extent.max_offset().y;
    scene.scroll(max * PIXELS_PER_POINT)?;
    let last = ink_bounds(&ink(&before, &scene.capture()?))?;
    close_pixel(
        last.y + last.height,
        (PANEL_SIZE - 2.0).mul_add(PIXELS_PER_POINT, PANEL_ORIGIN),
    );
    Ok(())
}

fn padded_bordered_view_inset() -> Result<(), String> {
    const BORDER: f32 = 6.0;
    let mut scene = IndicatorScene::with_view(
        20,
        false,
        Color::srgb_u8(BACKGROUND, BACKGROUND, BACKGROUND),
        El::column()
            .padding(Padding::all(8.0))
            .border(Border::all(BORDER, Color::srgb_u8(128, 128, 128))),
    )?;
    let before = scene.capture()?;
    scene.scroll(0.0)?;
    let first = ink_bounds(&ink(&before, &scene.capture()?))?;
    let right = (PANEL_SIZE - BORDER - 2.0).mul_add(PIXELS_PER_POINT, PANEL_ORIGIN);
    close_pixel(first.x + first.width, right);
    close_pixel(
        first.y,
        (BORDER + 2.0).mul_add(PIXELS_PER_POINT, PANEL_ORIGIN),
    );
    let max = scene.record()?.extent.max_offset().y;
    scene.scroll(max * PIXELS_PER_POINT)?;
    let last = ink_bounds(&ink(&before, &scene.capture()?))?;
    close_pixel(last.x + last.width, right);
    close_pixel(
        last.y + last.height,
        (PANEL_SIZE - BORDER - 2.0).mul_add(PIXELS_PER_POINT, PANEL_ORIGIN),
    );
    Ok(())
}

fn pixel_luminance(pixel: &[u8]) -> f32 {
    let linear = Color::srgb_u8(pixel[0], pixel[1], pixel[2]).to_linear();
    linear
        .red
        .mul_add(0.2126, linear.green.mul_add(0.7152, linear.blue * 0.0722))
}

fn assert_thumb_contrast(background: Color, view: El<Column>) -> Result<(), String> {
    let mut scene = IndicatorScene::with_view(20, false, background, view)?;
    let before = scene.capture()?;
    scene.scroll(30.0)?;
    let thumb = thumb_pixels(&scene.record()?)?;
    let after = scene.capture()?;
    let core = UVec2::new(
        (thumb.x + thumb.width / 2.0).to_u32(),
        (thumb.y + thumb.height / 2.0).to_u32(),
    );
    let index = ((core.y * SIZE + core.x) * 4).to_usize();
    let stock = pixel_luminance(&before[index..index + 4]);
    let ink = pixel_luminance(&after[index..index + 4]);
    let contrast = (stock.max(ink) + 0.05) / (stock.min(ink) + 0.05);
    eprintln!(
        "drawn thumb contrast={contrast}, stock={:?}, ink={:?}",
        &before[index..index + 4],
        &after[index..index + 4]
    );
    assert!(
        contrast >= 3.0,
        "drawn thumb contrast={contrast}, stock={:?}, ink={:?}",
        &before[index..index + 4],
        &after[index..index + 4]
    );
    Ok(())
}

fn light_view_contrast() -> Result<(), String> {
    assert_thumb_contrast(
        Color::BLACK,
        El::column().background(Color::srgb_u8(230, 230, 230)),
    )
}

fn dark_view_contrast() -> Result<(), String> {
    assert_thumb_contrast(Color::srgb_u8(31, 33, 38), El::column())
}

#[test]
fn scroll_indicator_pixels() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "padded_bordered_view_thumb_sits_inside_the_border",
            run:  padded_bordered_view_inset,
        },
        DrawnCase {
            name: "padded_view_thumb_hugs_the_border_box",
            run:  padded_border_inset,
        },
        DrawnCase {
            name: "light_view_thumb_reaches_three_to_one",
            run:  light_view_contrast,
        },
        DrawnCase {
            name: "dark_view_thumb_reaches_three_to_one",
            run:  dark_view_contrast,
        },
        DrawnCase {
            name: "no_indicator_ink_is_drawn_while_content_fits",
            run:  fitting_ink,
        },
        DrawnCase {
            name: "no_indicator_ink_is_drawn_before_the_first_scroll",
            run:  before_first_scroll,
        },
        DrawnCase {
            name: "indicator_ink_matches_the_recorded_thumb_within_one_pixel",
            run:  recorded_thumb,
        },
        DrawnCase {
            name: "indicator_ink_moves_down_as_the_content_scrolls",
            run:  moves_down,
        },
        DrawnCase {
            name: "indicator_ink_fades_and_is_gone_after_the_hold_and_fade_time",
            run:  fades_away,
        },
        DrawnCase {
            name: "indicator_is_clipped_to_the_view",
            run:  clipped_ink,
        },
    ])
}
