//! The colour of a world-panel fill is checked at the final drawn pixel.

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;

mod drawn_support;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

const SIZE: u32 = 128;
const FILL: [u8; 3] = [80, 100, 74];
const RIM_BORDER_WIDTH: f32 = 0.72;
const RIM_PANEL_SIZE: f32 = 64.0;
const RIM_PANEL_SIZE_PX: u32 = 64;
const RIM_VIEW_HEIGHT: f32 = 0.128;

#[derive(Clone, Copy, Debug)]
enum PanelDrawingPath {
    StandardTransparency,
    StableTransparency,
}

fn spawn_fill_panel(app: &mut App) -> Result<(), String> {
    let mut layout = LayoutBuilder::new(100.0, 100.0);
    layout.with(
        El::new()
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0))
            .background(Color::srgb_u8(FILL[0], FILL[1], FILL[2])),
        |_| {},
    );
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(100.0))
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid panel fill: {error}"))?;
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
    path: PanelDrawingPath,
    region: CaptureRegion,
) {
    let bloom = Bloom {
        prefilter: BloomPrefilter {
            threshold:          1.0,
            threshold_softness: 0.1,
        },
        ..default()
    };
    // The world display table is selected from a window camera. This camera
    // supplies Hana's configuration while the image camera supplies pixels.
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom.clone(),
    ));
    let mut image_camera = Camera::default();
    region.apply_to_camera(&mut image_camera);
    let camera = app
        .world_mut()
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
        .id();
    if matches!(path, PanelDrawingPath::StableTransparency) {
        app.world_mut()
            .entity_mut(camera)
            .insert(StableTransparency);
    }
}

fn drawn_center_pixel(path: PanelDrawingPath) -> Result<[u8; 4], String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(UVec2::splat(SIZE), UVec2::splat(60), UVec2::splat(8));
    let target = app.target(region);
    spawn_hana_camera(&mut app, target.clone(), path, region);

    spawn_fill_panel(&mut app)?;

    app.settle()?;
    let pixels = app.capture(target)?;
    let center = UVec2::splat(SIZE / 2);
    dump_pixels::<true>(
        "panel_fill_colour",
        &format!("{path:?}"),
        &format!(
            "px_per_em={:.9}",
            f64::from(SIZE) / (0.5 * (45.0_f64.to_radians() * 0.5).tan())
        ),
        region,
        CaptureRegion::new(UVec2::splat(SIZE), center, UVec2::ONE),
        &pixels,
    )?;
    let local_center = center - region.origin;
    let offset = ((local_center.y * region.size.x + local_center.x) * 4) as usize;
    let pixel = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| "screenshot has no center pixel".to_string())?;
    Ok([pixel[0], pixel[1], pixel[2], pixel[3]])
}

fn check_panel_fill(path: PanelDrawingPath) -> Result<(), String> {
    let drawn = drawn_center_pixel(path)?;
    eprintln!("panel fill path={path:?}: requested={FILL:?}, drawn={drawn:?}");
    for (channel, (actual, requested)) in drawn[..3].iter().zip(FILL).enumerate() {
        assert!(
            actual.abs_diff(requested) <= 1,
            "path={path:?}, channel={channel}: requested={FILL:?}, drawn={drawn:?}"
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct DrawnRim {
    physical_width:  f32,
    strongest_green: u8,
}

fn spawn_rim_panel(app: &mut App) -> Result<(), String> {
    let mut layout = LayoutBuilder::new(RIM_PANEL_SIZE, RIM_PANEL_SIZE);
    layout.with(
        El::new()
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0))
            .background(Color::BLACK)
            .border(Border::all(Px(RIM_BORDER_WIDTH), Color::srgb_u8(0, 255, 0))),
        |_| {},
    );
    let panel = DiegeticPanel::world()
        .size(Mm(RIM_PANEL_SIZE), Mm(RIM_PANEL_SIZE))
        .world_width(RIM_PANEL_SIZE / 1000.0)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid rim panel: {error}"))?;
    let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
    app.world_mut()
        .commands()
        .entity(panel)
        .override_lighting(Lighting::Unlit);
    Ok(())
}

fn drawn_rim(scale_factor: u16) -> Result<DrawnRim, String> {
    let mut app = CaseApp::new();
    let physical_scale = u32::from(scale_factor);
    let physical_size = UVec2::splat(SIZE * physical_scale);
    let region = CaptureRegion::new(physical_size, UVec2::ZERO, physical_size);
    let target = app.target(region);

    let mut window = Window::default();
    window
        .resolution
        .set_scale_factor_override(Some(f32::from(scale_factor)));
    window
        .resolution
        .set_physical_resolution(physical_size.x, physical_size.y);
    app.world_mut().spawn((window, PrimaryWindow));

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
    app.world_mut().spawn((
        Camera3d::default(),
        image_camera,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: RIM_VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        RenderTarget::Image(target.clone().into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        bloom,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    spawn_rim_panel(&mut app)?;

    app.settle()?;
    let pixels = app.capture(target)?;
    let edge_x = ((SIZE - RIM_PANEL_SIZE_PX) / 2 * physical_scale).cast_signed();
    let y = physical_size.y / 2;
    let green = |x: u32| {
        let index = ((y * physical_size.x + x) * 4) as usize;
        pixels[index + 1]
    };
    let sample_radius = (8 * physical_scale).cast_signed();
    let sample = (edge_x - sample_radius)..=(edge_x + sample_radius);
    let strengths: Vec<u8> = sample
        .filter_map(|x| u32::try_from(x).ok())
        .map(green)
        .collect();
    let physical_width = strengths
        .iter()
        .map(|&channel| f32::from(channel) / 255.0)
        .sum();
    let strongest_green = strengths.iter().copied().max().unwrap_or_default();
    eprintln!(
        "panel rim at scale {scale_factor}: {physical_width:.3} physical px, {:.3} logical px, strongest green {strongest_green}, samples {strengths:?}",
        physical_width / f32::from(scale_factor),
    );
    Ok(DrawnRim {
        physical_width,
        strongest_green,
    })
}

fn green_bucket(channel: u8) -> u16 { (u16::from(channel) + 8) / 16 }

#[test]
fn subpixel_panel_rim_keeps_its_logical_width_at_scale_two() -> Result<(), String> {
    let scale_one = drawn_rim(1)?;
    let scale_two = drawn_rim(2)?;
    let scale_one_logical = scale_one.physical_width;
    let scale_two_logical = scale_two.physical_width / 2.0;
    assert!(
        (scale_one_logical - scale_two_logical).abs() <= 0.5,
        "rim changed from {scale_one_logical:.3} logical px at scale 1 to {scale_two_logical:.3} logical px at scale 2"
    );
    assert_eq!(
        green_bucket(scale_one.strongest_green),
        green_bucket(scale_two.strongest_green),
        "rim strongest green changed from {} at scale 1 to {} at scale 2",
        scale_one.strongest_green,
        scale_two.strongest_green,
    );
    Ok(())
}

#[test]
fn panel_fill_matches_requested_colour_with_and_without_oit() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "standard_transparency_fill_matches_requested_colour",
            run:  || check_panel_fill(PanelDrawingPath::StandardTransparency),
        },
        DrawnCase {
            name: "stable_transparency_fill_matches_requested_colour",
            run:  || check_panel_fill(PanelDrawingPath::StableTransparency),
        },
    ])
}
