//! Warm-up batch creation, retirement, and first reveal keep their drawn pixels.

mod drawn_support;

use bevy::asset::RenderAssetUsages;
use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureDimension;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelDraw;
use hana_diegetic::PanelLine;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;

const VIEW_SIZE: UVec2 = UVec2::new(256, 160);
const VIEW_HEIGHT: f32 = 0.160;
const WARMUP_COLUMNS: usize = 6;
const WARMUP_ROWS: usize = 4;
const WARMUP_COUNT: usize = WARMUP_COLUMNS * WARMUP_ROWS;
const COVER_SIZE: Vec2 = Vec2::new(220.0, 130.0);
const SCENE_SIZE: Vec2 = Vec2::new(86.0, 56.0);
const WARMUP_SIZE: Vec2 = Vec2::new(32.0, 22.0);
const COVER_Z: f32 = 0.010;
const WARMUP_Z: f32 = 0.020;
const RED_IMAGE: [u8; 4] = [225, 30, 55, 255];
const GREEN_IMAGE: [u8; 4] = [25, 220, 90, 255];

#[derive(Clone)]
struct SequenceImages {
    red:   Handle<Image>,
    green: Handle<Image>,
}

impl SequenceImages {
    fn alternating(&self, index: usize) -> Handle<Image> {
        if index.is_multiple_of(2) {
            self.red.clone()
        } else {
            self.green.clone()
        }
    }
}

#[derive(Clone, Copy)]
enum SequenceFrame {
    CoveredWithWarmup,
    Covered,
    Revealed,
}

struct LiveSequence {
    app:    CaseApp,
    target: Handle<Image>,
    cover:  Entity,
    images: SequenceImages,
}

fn solid_image(color: [u8; 4]) -> Image {
    Image::new_fill(
        Extent3d {
            width:                 2,
            height:                2,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &color,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn add_images(app: &mut CaseApp) -> SequenceImages {
    let mut images = app.world_mut().resource_mut::<Assets<Image>>();
    SequenceImages {
        red:   images.add(solid_image(RED_IMAGE)),
        green: images.add(solid_image(GREEN_IMAGE)),
    }
}

fn spawn_camera(app: &mut CaseApp, target: Handle<Image>, region: CaptureRegion) {
    let mut image_camera = Camera::default();
    region.apply_to_camera(&mut image_camera);
    app.world_mut().spawn((
        Camera3d::default(),
        image_camera,
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
    ));
    app.world_mut().spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        RenderTarget::Image(target.into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        Msaa::Off,
        StableTransparency,
        Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn scene_tree(index: usize, image: Handle<Image>) -> LayoutTree {
    let fill = if index.is_multiple_of(2) {
        Color::srgb_u8(28, 58, 105)
    } else {
        Color::srgb_u8(92, 42, 30)
    };
    let line_color = if index.is_multiple_of(2) {
        Color::srgb_u8(245, 205, 45)
    } else {
        Color::srgb_u8(45, 230, 210)
    };
    let line = PanelLine::new((4.0, 48.0), (82.0, 8.0))
        .width(Mm(2.5))
        .color(line_color);
    let mut builder = LayoutBuilder::with_root(
        El::row()
            .size(SCENE_SIZE.x, SCENE_SIZE.y)
            .padding(Padding::all(5.0))
            .gap(5.0)
            .background(fill)
            .draw(PanelDraw::lines([line])),
    );
    builder.text(
        Text::new(
            if index == 0 { "READY" } else { "VOICE" },
            TextStyle::new(Mm(9.0))
                .with_color(Color::WHITE)
                .with_shadow_mode(GlyphShadowMode::None),
        )
        .layout(
            El::new()
                .width(Sizing::fixed(48.0))
                .height(Sizing::fixed(24.0)),
        ),
    );
    builder.image(
        El::new()
            .width(Sizing::fixed(22.0))
            .height(Sizing::fixed(22.0)),
        image,
        Color::WHITE,
    );
    builder.build()
}

fn warmup_tree(index: usize, image: Handle<Image>) -> LayoutTree {
    let fill = if index.is_multiple_of(2) {
        Color::srgb_u8(62, 32, 105)
    } else {
        Color::srgb_u8(25, 92, 86)
    };
    let line = PanelLine::new((2.0, 19.0), (30.0, 3.0))
        .width(Mm(1.5))
        .color(Color::srgb_u8(245, 220, 75));
    let mut builder = LayoutBuilder::with_root(
        El::row()
            .size(WARMUP_SIZE.x, WARMUP_SIZE.y)
            .padding(Padding::all(2.0))
            .gap(2.0)
            .background(fill)
            .draw(PanelDraw::lines([line])),
    );
    builder.text(
        Text::new(
            format!("{index:02}"),
            TextStyle::new(Mm(4.0))
                .with_color(Color::WHITE)
                .with_shadow_mode(GlyphShadowMode::None),
        )
        .layout(
            El::new()
                .width(Sizing::fixed(16.0))
                .height(Sizing::fixed(8.0)),
        ),
    );
    builder.image(
        El::new()
            .width(Sizing::fixed(8.0))
            .height(Sizing::fixed(8.0)),
        image,
        Color::WHITE,
    );
    builder.build()
}

fn cover_tree() -> LayoutTree {
    LayoutBuilder::with_root(
        El::new()
            .size(COVER_SIZE.x, COVER_SIZE.y)
            .background(Color::srgb_u8(12, 16, 28)),
    )
    .build()
}

fn spawn_panel(
    app: &mut CaseApp,
    size: Vec2,
    tree: LayoutTree,
    translation: Vec3,
) -> Result<Entity, String> {
    let panel = DiegeticPanel::world()
        .size(Mm(size.x), Mm(size.y))
        .anchor(Anchor::Center)
        .with_tree(tree)
        .build()
        .map_err(|error| format!("invalid sequence panel: {error}"))?;
    let entity = app
        .world_mut()
        .spawn((panel, Transform::from_translation(translation)))
        .id();
    app.world_mut()
        .commands()
        .entity(entity)
        .override_lighting(Lighting::Unlit);
    Ok(entity)
}

fn spawn_scene_panels(app: &mut CaseApp, images: &SequenceImages) -> Result<(), String> {
    for (index, x) in [-0.052, 0.052].into_iter().enumerate() {
        spawn_panel(
            app,
            SCENE_SIZE,
            scene_tree(index, images.alternating(index)),
            Vec3::new(x, 0.0, 0.0),
        )?;
    }
    Ok(())
}

fn spawn_cover(app: &mut CaseApp) -> Result<Entity, String> {
    spawn_panel(app, COVER_SIZE, cover_tree(), Vec3::new(0.0, 0.0, COVER_Z))
}

const fn warmup_translation(index: usize) -> Vec3 {
    let column = index % WARMUP_COLUMNS;
    let row = index / WARMUP_COLUMNS;
    let x = [-0.090, -0.054, -0.018, 0.018, 0.054, 0.090][column];
    let y = [0.045, 0.015, -0.015, -0.045][row];
    Vec3::new(x, y, WARMUP_Z)
}

fn spawn_warmup_panels(app: &mut CaseApp, images: &SequenceImages) -> Result<Vec<Entity>, String> {
    (0..WARMUP_COUNT)
        .map(|index| {
            spawn_panel(
                app,
                WARMUP_SIZE,
                warmup_tree(index, images.alternating(index)),
                warmup_translation(index),
            )
        })
        .collect()
}

fn setup_frame(frame: SequenceFrame) -> Result<(CaseApp, Handle<Image>), String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(VIEW_SIZE, UVec2::ZERO, VIEW_SIZE);
    let target = app.target(region);
    spawn_camera(&mut app, target.clone(), region);
    let images = add_images(&mut app);
    spawn_scene_panels(&mut app, &images)?;
    if matches!(
        frame,
        SequenceFrame::CoveredWithWarmup | SequenceFrame::Covered
    ) {
        spawn_cover(&mut app)?;
    }
    if matches!(frame, SequenceFrame::CoveredWithWarmup) {
        spawn_warmup_panels(&mut app, &images)?;
    }
    app.settle()?;
    Ok((app, target))
}

fn setup_live_sequence() -> Result<LiveSequence, String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(VIEW_SIZE, UVec2::ZERO, VIEW_SIZE);
    let target = app.target(region);
    spawn_camera(&mut app, target.clone(), region);
    let images = add_images(&mut app);
    spawn_scene_panels(&mut app, &images)?;
    let cover = spawn_cover(&mut app)?;
    app.settle()?;
    Ok(LiveSequence {
        app,
        target,
        cover,
        images,
    })
}

fn fresh_pixels(frame: SequenceFrame) -> Result<Vec<u8>, String> {
    let (mut app, target) = setup_frame(frame)?;
    app.capture(target)
}

fn pixel_difference(left: &[u8], right: &[u8]) -> (usize, u8) {
    left.iter()
        .zip(right)
        .fold((0, 0), |(count, largest), (&left, &right)| {
            let difference = left.abs_diff(right);
            (
                count + usize::from(difference != 0),
                largest.max(difference),
            )
        })
}

fn assert_fresh_frame(label: &str, actual: &[u8], expected: &[u8]) -> Result<(), String> {
    let region = CaptureRegion::new(VIEW_SIZE, UVec2::ZERO, VIEW_SIZE);
    dump_pixels::<false>(
        "panel_batch_start_sequence_pixels",
        "",
        label,
        region,
        region,
        actual,
    )?;
    let difference = pixel_difference(actual, expected);
    if difference.0 != 0 {
        return Err(format!(
            "{label} differs from a fresh draw in {} channels (largest difference {})",
            difference.0, difference.1,
        ));
    }
    Ok(())
}

fn check_warmup_batch_retirement_and_first_reveal() -> Result<(), String> {
    let created_expected = fresh_pixels(SequenceFrame::CoveredWithWarmup)?;
    let retired_expected = fresh_pixels(SequenceFrame::Covered)?;
    let revealed_expected = fresh_pixels(SequenceFrame::Revealed)?;
    if pixel_difference(&created_expected, &retired_expected).0 == 0 {
        return Err("warm-up panels do not change the covered fixture".to_string());
    }
    if pixel_difference(&retired_expected, &revealed_expected).0 == 0 {
        return Err("lifting the cover does not reveal the scene fixture".to_string());
    }

    let mut sequence = setup_live_sequence()?;
    let warmup = spawn_warmup_panels(&mut sequence.app, &sequence.images)?;
    let created = sequence.app.capture(sequence.target.clone())?;
    assert_fresh_frame("warm-up creation frame", &created, &created_expected)?;

    for panel in warmup {
        sequence
            .app
            .world_mut()
            .entity_mut(panel)
            .remove::<DiegeticPanel>();
    }
    let retired = sequence.app.capture(sequence.target.clone())?;
    assert_fresh_frame("warm-up retirement frame", &retired, &retired_expected)?;

    sequence
        .app
        .world_mut()
        .entity_mut(sequence.cover)
        .remove::<DiegeticPanel>();
    let revealed = sequence.app.capture(sequence.target)?;
    assert_fresh_frame("first revealed frame", &revealed, &revealed_expected)
}

#[test]
fn warmup_batch_retirement_and_first_reveal_match_fresh_pixels() -> Result<(), String> {
    run_cases(&[DrawnCase {
        name: "warmup_batch_retirement_and_first_reveal_match_fresh_pixels",
        run:  check_warmup_batch_retirement_and_first_reveal,
    }])
}
