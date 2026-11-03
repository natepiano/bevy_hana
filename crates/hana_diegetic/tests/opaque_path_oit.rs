//! OIT slate panels respect the depth written by nearer analytic title ink.

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::render::view::NoIndirectDrawing;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::TextStyle;

mod drawn_support;

use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;

const SIZE: u32 = 512;

struct TitleScene {
    app:    CaseApp,
    target: Handle<Image>,
    slate:  Entity,
    region: CaptureRegion,
    camera: Entity,
}

fn spawn_title(app: &mut App, alpha: AlphaMode) -> Result<Entity, String> {
    let mut layout = LayoutBuilder::new(100.0, 60.0);
    layout.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.text((
                "Frame time",
                TextStyle::new(Mm(8.0))
                    .with_color(Color::WHITE)
                    .with_alpha_mode(alpha)
                    .with_shadow_mode(GlyphShadowMode::None),
            ));
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(60.0))
        .world_width(0.1)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid title: {error}"))?;
    Ok(app
        .world_mut()
        .spawn((panel, Transform::from_xyz(0.0, 0.0, 0.001)))
        .id())
}

fn spawn_slate(app: &mut App) -> Result<Entity, String> {
    let mut layout = LayoutBuilder::new(120.0, 80.0);
    layout.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(Color::srgb_u8(24, 32, 40)),
        |_| {},
    );
    let panel = DiegeticPanel::world()
        .size(Mm(120.0), Mm(80.0))
        .world_width(0.12)
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid slate: {error}"))?;
    Ok(app.world_mut().spawn((panel, Transform::IDENTITY)).id())
}

fn setup_title_scene(alpha: AlphaMode) -> Result<TitleScene, String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(UVec2::splat(SIZE), UVec2::ZERO, UVec2::splat(SIZE));
    let target = app.target(region);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
    ));
    let mut camera = Camera::default();
    region.apply_to_camera(&mut camera);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            camera,
            RenderTarget::Image(target.clone().into()),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            Msaa::Off,
            StableTransparency,
            DepthPrepass,
            Transform::from_xyz(0.125, 0.10, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    spawn_title(&mut app, alpha)?;
    let slate = spawn_slate(&mut app)?;
    app.settle()?;
    Ok(TitleScene {
        app,
        target,
        slate,
        region,
        camera,
    })
}

fn is_title_ink(pixel: [u8; 4]) -> bool { pixel[..3].iter().all(|channel| *channel > 200) }

fn check_title_ink(alpha: AlphaMode) -> Result<(), String> {
    check_title_scene_ink(alpha, setup_title_scene(alpha)?)
}

fn check_title_scene_ink(alpha: AlphaMode, scene: TitleScene) -> Result<(), String> {
    let TitleScene {
        mut app,
        target,
        slate,
        region,
        camera,
    } = scene;
    let with_slate = app.capture(target.clone())?;
    dump_pixels::<true>(
        "opaque_path_oit",
        "",
        &format!("alpha={alpha:?}"),
        region,
        region,
        &with_slate,
    )?;
    app.world_mut().entity_mut(slate).insert(Visibility::Hidden);
    // An independent reference must also expose any ink lost to its own
    // prepass. With the slate hidden, main depth is sufficient for this shot.
    app.world_mut().entity_mut(camera).remove::<DepthPrepass>();
    app.settle()?;
    let without_slate = app.capture(target)?;
    let reference = without_slate.as_chunks::<4>().0;
    let ink_pixels = reference
        .iter()
        .filter(|pixel| is_title_ink(**pixel))
        .count();
    assert!(
        ink_pixels > 100,
        "{alpha:?}: reference title has {ink_pixels} ink pixels"
    );
    let covered_ink = reference
        .iter()
        .zip(with_slate.as_chunks::<4>().0)
        .filter(|(ink, drawn)| is_title_ink(**ink) && !is_title_ink(**drawn))
        .count();
    let slate_pixels = with_slate
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[2] > 30 && pixel[0] < 50)
        .count();
    assert!(
        slate_pixels > 1000,
        "the background surface must be visible behind the title"
    );
    eprintln!("{alpha:?}: {ink_pixels} reference ink pixels, {covered_ink} covered by slate");
    assert_eq!(
        covered_ink, 0,
        "{alpha:?}: background surface covered title ink"
    );
    Ok(())
}

#[test]
fn opaque_masked_and_blended_text_draw_in_front_of_oit_slate_at_home_pose() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "opaque_title_ink_survives_oit_slate",
            run:  || check_title_ink(AlphaMode::Opaque),
        },
        DrawnCase {
            name: "masked_title_ink_survives_oit_slate",
            run:  || check_title_ink(AlphaMode::Mask(0.5)),
        },
        DrawnCase {
            name: "blended_title_ink_survives_oit_slate",
            run:  || check_title_ink(AlphaMode::Blend),
        },
    ])
}

#[test]
fn blended_title_ink_draws_over_a_coplanar_opaque_mesh_without_a_prepass() -> Result<(), String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(UVec2::splat(1536), UVec2::ZERO, UVec2::splat(1536));
    let target = app.target(region);
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
    ));
    // This direct-drawing orthographic view exposes inverse-plane rounding against
    // the hardware depth of a coplanar mesh. No physical separation is authored.
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            RenderTarget::Image(target.clone().into()),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: 0.615_308_8,
                },
                ..OrthographicProjection::default_3d()
            }),
            Tonemapping::None,
            Msaa::Off,
            StableTransparency,
            NoIndirectDrawing,
            Transform::from_xyz(0.0, 0.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    let title = spawn_title(&mut app, AlphaMode::Blend)?;
    app.world_mut()
        .entity_mut(title)
        .insert(Transform::from_xyz(0.0, 0.0, 0.0035));
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(0.12, 0.08));
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb_u8(0, 0, 100),
            unlit: true,
            ..default()
        });
    let slate = app
        .world_mut()
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, 0.0, 0.0035),
        ))
        .id();
    app.settle()?;
    check_title_scene_ink(
        AlphaMode::Blend,
        TitleScene {
            app,
            target,
            slate,
            region,
            camera,
        },
    )
}
