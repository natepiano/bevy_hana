//! Known-colour composites prove physical order independently of the OIT key.

mod drawn_support;

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::prelude::*;
use bevy::render::view::DebandDither;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DisplayCompensationReadiness;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_kana::ToU8;

const SIZE: UVec2 = UVec2::splat(128);

fn paint(builder: &mut LayoutBuilder, layers: u8, colour: Color) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(colour),
        |builder| {
            if layers > 1 {
                paint(builder, layers - 1, colour);
            }
        },
    );
}

fn panel(
    app: &mut App,
    distance: f32,
    width: f32,
    layers: u8,
    colour: Color,
) -> Result<(), String> {
    let mut builder = LayoutBuilder::new(100.0, 100.0);
    paint(&mut builder, layers, colour);
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(100.0))
        .world_width(width)
        .anchor(Anchor::Center)
        .with_tree(builder.build())
        .build()
        .map_err(|error| error.to_string())?;
    let entity = app
        .world_mut()
        .spawn((panel, Transform::from_xyz(0.0, 0.0, -distance)))
        .id();
    app.world_mut()
        .commands()
        .entity(entity)
        .override_lighting(Lighting::Unlit);
    Ok(())
}

fn check(projection: Projection, distance: f32, gap: f32, name: &str) -> Result<(), String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(SIZE, UVec2::ZERO, SIZE);
    let target = app.target(region);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::None,
    ));
    let mut camera = Camera {
        clear_color: ClearColorConfig::Custom(Color::BLACK),
        ..default()
    };
    region.apply_to_camera(&mut camera);
    app.world_mut().spawn((
        Camera3d::default(),
        camera,
        RenderTarget::Image(target.clone().into()),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        StableTransparency,
        projection,
        Transform::IDENTITY,
    ));
    panel(
        &mut app,
        distance,
        distance * 0.4,
        1,
        Color::linear_rgba(1.0, 0.0, 0.0, 0.5),
    )?;
    panel(
        &mut app,
        distance + gap,
        distance * 0.4,
        4,
        Color::linear_rgba(0.0, 0.0, 1.0, 0.5),
    )?;
    app.settle()?;
    assert_eq!(
        *app.world().resource::<DisplayCompensationReadiness>(),
        DisplayCompensationReadiness::Identity,
        "an untonemapped scene has an identity display table"
    );
    let pixels = app.capture(target)?;
    drawn_support::dump_pixels::<false>(
        "oit_physical_order",
        name,
        "known front red; four rear blue layers",
        region,
        region,
        &pixels,
    )?;
    // Independent OVER, on black. Alpha is quantized to eight bits by OIT.
    // No camera-depth encoding or reference OIT render participates here.
    let alpha = 128.0_f32 / 255.0;
    let rear_alpha = 1.0 - (1.0 - alpha).powi(4);
    let expected = LinearRgba::new(alpha, 0.0, (1.0 - alpha) * rear_alpha, 1.0);
    let expected = Srgba::from(expected)
        .to_f32_array()
        .map(|channel| (channel * 255.0).round().to_u8());
    for y in 60..68_usize {
        for x in 60..68_usize {
            let actual = &pixels[(y * 128 + x) * 4..][..4];
            assert!(
                actual[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, e)| a.abs_diff(e) <= 3),
                "{name}: independently blended {expected:?}, drawn {actual:?} at {x},{y}"
            );
        }
    }
    Ok(())
}

fn perspective(near: f32) -> Projection {
    Projection::Perspective(PerspectiveProjection {
        near,
        aspect_ratio: 1.0,
        ..default()
    })
}
fn short_near() -> Result<(), String> { check(perspective(0.0001), 10.0, 0.0001, "short-near") }
fn short_near_far() -> Result<(), String> {
    check(perspective(0.0001), 1000.0, 0.02, "short-near-far")
}
fn long_near_far() -> Result<(), String> { check(perspective(1.0), 1000.0, 0.01, "long-near-far") }
fn orthographic() -> Result<(), String> {
    check(
        Projection::Orthographic(OrthographicProjection {
            near: 0.0,
            far: 1000.0,
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 8.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        10.0,
        0.0002,
        "orthographic",
    )
}

#[test]
fn p33_physical_depth_precedes_local_layers() -> Result<(), String> {
    drawn_support::run_cases(&[
        DrawnCase {
            name: "short-near",
            run:  short_near,
        },
        DrawnCase {
            name: "short-near-far",
            run:  short_near_far,
        },
        DrawnCase {
            name: "long-near-far",
            run:  long_near_far,
        },
        DrawnCase {
            name: "orthographic",
            run:  orthographic,
        },
    ])
}
