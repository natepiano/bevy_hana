//! Overflow must composite straight Blend colours with exactly one alpha factor.

mod drawn_support;

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::prelude::*;
use bevy::render::view::DebandDither;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use hana_kana::ToF32;
use hana_kana::ToU8;
use hana_kana::ToUsize;

const SIZE: u32 = 128;
const SORTED_FRAGMENTS: u32 = 8;
const OVERFLOW_FRAGMENTS: u32 = 12;
const ALPHA: f32 = 64.0 / 255.0;
const STRAIGHT_RGB: Vec3 = Vec3::new(1.0, 0.5, 0.25);
const LAYER_SPACING: f32 = 0.1;
const BASE_DISTANCE: f32 = 2.0;
// Bevy allocates in twelve-byte units; Hana uses sixteen-byte nodes. Thirty-two
// units provide twenty-four actual nodes per pixel, twice the twelve
// full-viewport fragments in the overflow cases.
const FRAGMENT_UNITS_PER_PIXEL: f32 = 32.0;
const VIEWPORT_HEIGHT: f32 = 2.0;
// Wider and taller than the viewport, so every quad covers every pixel.
const QUAD_SIDE: f32 = 4.0;
// Three sRGB code values allow RGB9E5 packing, HDR target rounding and final
// eight-bit readback. Overflow's extra alpha costs far more.
const CHANNEL_TOLERANCE: u8 = 3;
const SAMPLE_REGION: CaptureRegion =
    CaptureRegion::new(UVec2::splat(SIZE), UVec2::splat(60), UVec2::splat(8));

#[derive(Clone, Copy, Debug)]
enum FragmentTraversal {
    NearToFar,
    FarToNear,
}

fn check_composite(fragments: u32, traversal: FragmentTraversal) -> Result<(), String> {
    let mut app = CaseApp::new();
    let region = CaptureRegion::new(UVec2::splat(SIZE), UVec2::ZERO, UVec2::splat(SIZE));
    let target = app.target(region);
    // The window camera's `Tonemapping::None` lets display compensation reach
    // `DisplayCompensationReadiness::Identity`, which `CaseApp::settle` waits for.
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
        OrderIndependentTransparencySettings {
            sorted_fragment_max_count: SORTED_FRAGMENTS,
            fragments_per_pixel_average: FRAGMENT_UNITS_PER_PIXEL,
            ..default()
        },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: VIEWPORT_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::IDENTITY,
    ));
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(QUAD_SIDE, QUAD_SIDE));
    for layer in 0..fragments {
        let distance = layer.to_f32().mul_add(LAYER_SPACING, BASE_DISTANCE);
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::linear_rgba(
                    STRAIGHT_RGB.x,
                    STRAIGHT_RGB.y,
                    STRAIGHT_RGB.z,
                    ALPHA,
                ),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                // Transparent3d sorts ascending view-z + bias; the OIT linked
                // list reverses draw order. Reversing the sort key forces the
                // farther-fragment eviction and nearer-fragment early-blend
                // branches respectively, without moving the physical quads.
                depth_bias: match traversal {
                    FragmentTraversal::NearToFar => 0.0,
                    FragmentTraversal::FarToNear => 2.0 * distance,
                },
                ..default()
            });
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, 0.0, -distance),
        ));
    }
    app.settle()?;
    let pixels = app.capture(target)?;
    drawn_support::dump_pixels::<false>(
        "oit_overflow",
        "",
        &format!("fragments={fragments} traversal={traversal:?}"),
        region,
        SAMPLE_REGION,
        &pixels,
    )?;
    assert_straight_composite(&pixels, fragments, traversal);
    Ok(())
}

fn assert_straight_composite(pixels: &[u8], fragments: u32, traversal: FragmentTraversal) {
    // Independent straight-alpha OVER on black. All fragments have the same
    // colour, so the resolver's approximate ordering cannot affect this oracle.
    // Alpha is exactly representable in the resolver's eight-bit alpha field.
    let coverage = 1.0 - (1.0 - ALPHA).powf(fragments.to_f32());
    let expected = Srgba::from(LinearRgba::new(
        STRAIGHT_RGB.x * coverage,
        STRAIGHT_RGB.y * coverage,
        STRAIGHT_RGB.z * coverage,
        1.0,
    ))
    .to_f32_array()
    .map(|channel| (channel * 255.0).round().to_u8());
    for y in SAMPLE_REGION.origin.y..SAMPLE_REGION.origin.y + SAMPLE_REGION.size.y {
        for x in SAMPLE_REGION.origin.x..SAMPLE_REGION.origin.x + SAMPLE_REGION.size.x {
            let index = (y.to_usize() * SIZE.to_usize() + x.to_usize()) * 4;
            let actual = &pixels[index..index + 4];
            assert!(
                actual[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&actual_channel, expected_channel)| {
                        actual_channel.abs_diff(expected_channel) <= CHANNEL_TOLERANCE
                    }),
                "{fragments} Blend fragments, {traversal:?}: straight composite {expected:?}, \
                 drawn {actual:?} at {x},{y}"
            );
        }
    }
}

fn sorted_control() -> Result<(), String> {
    check_composite(SORTED_FRAGMENTS, FragmentTraversal::NearToFar)
}

fn evict_nearest() -> Result<(), String> {
    check_composite(OVERFLOW_FRAGMENTS, FragmentTraversal::NearToFar)
}

fn blend_incoming_nearest() -> Result<(), String> {
    check_composite(OVERFLOW_FRAGMENTS, FragmentTraversal::FarToNear)
}

#[test]
fn oit_overflow_applies_fractional_alpha_once_in_both_branches() -> Result<(), String> {
    drawn_support::run_cases(&[
        DrawnCase {
            name: "eight_sorted_fragments",
            run:  sorted_control,
        },
        DrawnCase {
            name: "overflow_evicts_nearest",
            run:  evict_nearest,
        },
        DrawnCase {
            name: "overflow_blends_incoming_nearest",
            run:  blend_incoming_nearest,
        },
    ])
}
