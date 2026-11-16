//! The shared panel renderer keeps readable front ink and a plain panel back.

mod drawn_support;

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::tonemapping::DebandDither;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::window::WindowRef;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::Icon;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LucideIcon;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::RichText;
use hana_diegetic::Sidedness;
use hana_diegetic::StableTransparency;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_kana::Cascade;

const SIZE: UVec2 = UVec2::new(240, 160);
const SURFACE: Color = Color::srgba(0.12, 0.18, 0.24, 0.55);

#[derive(Clone, Copy, Debug)]
enum Ink {
    Plain(AlphaMode),
    Rich,
    Icon,
}

fn camera(app: &mut CaseApp) -> (Entity, Handle<Image>) {
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Window(WindowRef::Primary),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        RenderLayers::none(),
    ));
    let region = CaptureRegion::new(SIZE, UVec2::ZERO, SIZE);
    let target = app.target(region);
    let mut capture = Camera::default();
    region.apply_to_camera(&mut capture);
    let entity = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            capture,
            RenderTarget::Image(target.clone().into()),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            DebandDither::Disabled,
            Msaa::Off,
            StableTransparency,
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: 0.080,
                },
                ..OrthographicProjection::default_3d()
            }),
            Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
            RenderLayers::layer(0),
        ))
        .id();
    (entity, target)
}

#[derive(Clone, Copy)]
enum InkVisibility {
    Drawn,
    Absent,
}

#[derive(Clone, Copy, Debug)]
enum PanelFace {
    Front,
    Back,
}

fn panel(
    app: &mut CaseApp,
    layer: usize,
    ink: Ink,
    visibility: InkVisibility,
) -> Result<(), String> {
    let mut builder = LayoutBuilder::with_root(
        El::new()
            .size(100.0, 60.0)
            .padding(Padding::all(6.0))
            .background(SURFACE),
    );
    let color = match visibility {
        InkVisibility::Drawn => Color::WHITE,
        InkVisibility::Absent => Color::NONE,
    };
    let style = TextStyle::new(Mm(10.0))
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None);
    match ink {
        Ink::Plain(alpha) => {
            let alpha = match visibility {
                InkVisibility::Drawn => alpha,
                InkVisibility::Absent => AlphaMode::Blend,
            };
            builder.text(Text::new("Front 42", style.with_alpha_mode(alpha)));
        },
        Ink::Rich => {
            builder.rich_text(RichText::new(style), |spans| {
                spans.span("Front ");
                spans.span("42");
            });
        },
        Ink::Icon => {
            builder.icon(Icon::new(LucideIcon::Info).size(28.0).color(color));
        },
    }
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(60.0))
        .anchor(Anchor::Center)
        .with_tree(builder.build())
        .build()
        .map_err(|error| format!("panel: {error}"))?;
    app.world_mut().spawn((
        panel,
        Transform::default(),
        RenderLayers::layer(layer),
        Cascade::Override(if layer == 1 {
            Sidedness::FrontOnly
        } else {
            Sidedness::BothSides
        }),
    ));
    Ok(())
}

fn capture(
    app: &mut CaseApp,
    camera: Entity,
    target: &Handle<Image>,
    layer: usize,
    face: PanelFace,
) -> Result<Vec<u8>, String> {
    let z = match face {
        PanelFace::Front => 0.25,
        PanelFace::Back => -0.25,
    };
    app.world_mut().entity_mut(camera).insert((
        RenderLayers::layer(layer),
        Transform::from_xyz(0.0, 0.0, z).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.settle()?;
    let pixels = app.capture(target.clone())?;
    let region = CaptureRegion::new(SIZE, UVec2::ZERO, SIZE);
    drawn_support::dump_pixels::<false>(
        "panel_text_sidedness",
        "",
        &format!("layer={layer} face={face:?}"),
        region,
        region,
        &pixels,
    )?;
    Ok(pixels)
}

fn changed_pixels(actual: &[u8], expected: &[u8]) -> usize {
    actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.as_chunks::<4>().0)
        .filter(|(actual, expected)| {
            actual[..3]
                .iter()
                .zip(&expected[..3])
                .any(|(actual, expected)| actual.abs_diff(*expected) > 1)
        })
        .count()
}

fn check_ink(ink: Ink) -> Result<(), String> {
    let mut app = CaseApp::new();
    let (camera, target) = camera(&mut app);
    panel(&mut app, 0, ink, InkVisibility::Drawn)?;
    panel(&mut app, 1, ink, InkVisibility::Drawn)?;
    panel(&mut app, 2, ink, InkVisibility::Absent)?;
    let front = capture(&mut app, camera, &target, 0, PanelFace::Front)?;
    let reference = capture(&mut app, camera, &target, 1, PanelFace::Front)?;
    let plain_front = capture(&mut app, camera, &target, 2, PanelFace::Front)?;
    assert_eq!(
        changed_pixels(&front, &reference),
        0,
        "front changed: {ink:?}"
    );
    let ink_pixels = changed_pixels(&front, &plain_front);
    assert!(ink_pixels > 20, "front ink missing: {ink:?}: {ink_pixels}");
    let back = capture(&mut app, camera, &target, 0, PanelFace::Back)?;
    let plain_back = capture(&mut app, camera, &target, 2, PanelFace::Back)?;
    let back_ink = changed_pixels(&back, &plain_back);
    eprintln!("{ink:?}: front ink={ink_pixels}, back ink={back_ink}");
    assert!(
        plain_back
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[2] > 20),
        "plain panel back disappeared"
    );
    assert_eq!(back_ink, 0, "mirrored ink on plain panel back: {ink:?}");
    Ok(())
}

fn blended() -> Result<(), String> { check_ink(Ink::Plain(AlphaMode::Blend)) }
fn masked() -> Result<(), String> { check_ink(Ink::Plain(AlphaMode::Mask(0.5))) }
fn opaque() -> Result<(), String> { check_ink(Ink::Plain(AlphaMode::Opaque)) }
fn rich() -> Result<(), String> { check_ink(Ink::Rich) }
fn icon() -> Result<(), String> { check_ink(Ink::Icon) }

#[test]
fn panel_text_never_draws_mirrored() -> Result<(), String> {
    drawn_support::run_cases(&[
        DrawnCase {
            name: "blended",
            run:  blended,
        },
        DrawnCase {
            name: "masked",
            run:  masked,
        },
        DrawnCase {
            name: "opaque",
            run:  opaque,
        },
        DrawnCase {
            name: "rich",
            run:  rich,
        },
        DrawnCase {
            name: "icon",
            run:  icon,
        },
    ])
}
