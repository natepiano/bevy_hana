//! Pixel regressions for opaque foreground bodies, without outline markers.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::mpsc;

use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::pbr::ExtendedMaterial;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::RenderPlugin;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::view::DebandDither;
use bevy::render::view::Tonemapping;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineBarrier;
use hana_liminal::OutlineCamera;
use hana_liminal::OutlineDepthTest;
use hana_liminal::OutlineDepthTexture;

const SIDE: u16 = 256;

struct OutlineDepthFixture {
    app:                App,
    target:             Handle<Image>,
    foreground_columns: Range<usize>,
    foreground_rows:    Range<usize>,
    outlined_body:      Entity,
}

impl OutlineDepthFixture {
    fn new(front_z: f32) -> Self { Self::with_foreground(front_z, 1.0, 1.0, 160..224) }

    fn with_foreground(
        front_z: f32,
        center_x: f32,
        width: f32,
        foreground_columns: Range<usize>,
    ) -> Self {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<WinitPlugin>()
                .disable::<PipelinedRenderingPlugin>(),
        )
        .add_plugins((
            LiminalPlugin,
            MaterialPlugin::<ExtendedMaterial<StandardMaterial, OutlineDepthTest>>::default(),
        ))
        .insert_resource(ClearColor(Color::BLACK));
        let mut image = Image::new_target_texture(
            u32::from(SIDE),
            u32::from(SIDE),
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.world_mut().spawn((
            Camera3d::default(),
            OutlineCamera,
            Msaa::Off,
            Tonemapping::None,
            DebandDither::Disabled,
            RenderTarget::Image(target.clone().into()),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: 4.0,
                },
                ..OrthographicProjection::default_3d()
            }),
            Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let bodies = [
            (
                Vec3::new(2.0, 2.0, 0.04),
                Vec3::ZERO,
                Color::srgb(0.1, 0.1, 0.1),
                true,
            ),
            (
                Vec3::new(width, 1.0, 0.04),
                Vec3::new(center_x, 0.0, front_z),
                Color::srgb(0.8, 0.1, 0.1),
                false,
            ),
        ]
        .map(|(size, pose, color, outlined)| {
            let mesh = app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(Cuboid::from_size(size));
            let material = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    base_color: color,
                    unlit: true,
                    alpha_mode: AlphaMode::Opaque,
                    ..default()
                });
            let mut body = app.world_mut().spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_translation(pose),
            ));
            if outlined {
                body.insert(
                    Outline::jump_flood(5.0)
                        .with_color(Color::srgb(0.05, 0.2, 1.0))
                        .build(),
                );
            }
            body.id()
        });
        app.finish();
        app.cleanup();
        Self {
            app,
            target,
            foreground_columns,
            foreground_rows: 96..160,
            outlined_body: bodies[0],
        }
    }

    fn pixels(&self) -> Result<Vec<u8>, String> {
        let world = self.app.sub_app(RenderApp).world();
        let image = world
            .resource::<RenderAssets<GpuImage>>()
            .get(self.target.id())
            .ok_or("target not uploaded")?;
        let device = world.resource::<RenderDevice>();
        let buffer = device.create_buffer(&BufferDescriptor {
            label:              Some("Outline depth pixels"),
            size:               u64::from(SIDE) * u64::from(SIDE) * 4,
            usage:              BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            image.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout {
                    offset:         0,
                    bytes_per_row:  Some(u32::from(SIDE) * 4),
                    rows_per_image: Some(u32::from(SIDE)),
                },
            },
            Extent3d {
                width:                 u32::from(SIDE),
                height:                u32::from(SIDE),
                depth_or_array_layers: 1,
            },
        );
        world.resource::<RenderQueue>().submit([encoder.finish()]);
        let (sender, receiver) = mpsc::channel();
        let slice = buffer.slice(..);
        slice.map_async(MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .wgpu_device()
            .poll(PollType::wait_indefinitely())
            .map_err(|error| error.to_string())?;
        receiver
            .recv()
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let pixels = slice
            .get_mapped_range()
            .map_err(|error| format!("GPU mapped buffer range: {error}"))?
            .to_vec();
        buffer.unmap();
        Ok(pixels)
    }

    fn assert_occlusion(&mut self) -> Result<(), String> {
        let pixels = loop {
            self.app.update();
            if self
                .app
                .sub_app(RenderApp)
                .world()
                .resource::<PipelineCache>()
                .waiting_pipelines()
                .next()
                .is_some()
            {
                continue;
            }
            let pixels = self.pixels()?;
            // Wait for actual red foreground and blue exterior pixels, not a frame count.
            if pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] > 180 && p[1] < 120)
                && pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[2] > 200 && p[0] < 100)
            {
                break pixels;
            }
        };
        let mut inside = 0;
        let mut outside = 0;
        let mut buckets = BTreeMap::new();
        for (index, p) in pixels.as_chunks::<4>().0.iter().enumerate() {
            let bucket = [p[0] / 16, p[1] / 16, p[2] / 16];
            *buckets.entry(bucket).or_insert(0_usize) += 1;
            if bucket[2] <= bucket[0].saturating_add(2) || bucket[2] <= bucket[1].saturating_add(1)
            {
                continue;
            }
            let x = index % usize::from(SIDE);
            let y = index / usize::from(SIDE);
            // The orthographic red body covers these pixel centres.
            // Include its boundary pixels; this view has no antialiasing.
            if self.foreground_columns.contains(&x) && self.foreground_rows.contains(&y) {
                inside += 1;
            } else {
                outside += 1;
            }
        }
        println!("foreground outline pixels {inside}; exterior {outside}; buckets {buckets:?}");
        assert_eq!(
            inside, 0,
            "an opaque foreground body must hide the outline inside its silhouette; exterior {outside}"
        );
        assert!(outside > 100, "visible outline disappeared");
        Ok(())
    }
    fn add_camera_presentation_plate(&mut self) -> Result<(), String> {
        // Camera panels cover the outlined picture with an opaque plate 0.2 mm
        // forward. Its OutlineBarrier leaves the picture root owning the border.
        let mesh = self
            .app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(2.0, 2.0, 0.000_002));
        let material = self
            .app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::srgb(0.1, 0.1, 0.1),
                unlit: true,
                ..default()
            });
        self.app.world_mut().spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, 0.0, 0.0202),
            OutlineBarrier,
            ChildOf(self.outlined_body),
        ));
        let world = self.app.world_mut();
        let camera = world
            .query_filtered::<Entity, With<Camera3d>>()
            .single(world)
            .map_err(|error| error.to_string())?;
        world
            .entity_mut(camera)
            .insert(Projection::Perspective(PerspectiveProjection {
                fov: 2.0 * 0.5_f32.atan(),
                near: 0.004,
                ..default()
            }));
        self.foreground_columns = 164..235;
        self.foreground_rows = 92..164;
        Ok(())
    }
}

#[test]
fn outline_is_occluded_by_an_unmarked_opaque_body() -> Result<(), String> {
    OutlineDepthFixture::new(0.4).assert_occlusion()
}

#[test]
fn outline_is_occluded_by_a_close_unmarked_opaque_body() -> Result<(), String> {
    OutlineDepthFixture::new(0.003).assert_occlusion()
}

#[test]
fn outline_is_occluded_by_the_measured_camera_foreground_gap() -> Result<(), String> {
    // The live Control frame stood 0.5 mm ahead of the Camera border. That
    // foreground gap must not inherit the Camera's own 0.2 mm plate contact.
    OutlineDepthFixture::new(0.0005).assert_occlusion()
}

#[test]
fn outline_stroke_is_occluded_when_its_seed_is_outside_the_opaque_body() -> Result<(), String> {
    // The red body's left edge is beyond the grey body's silhouette. Only the
    // expanded blue stroke overlaps it; rejecting covered seeds is insufficient.
    OutlineDepthFixture::with_foreground(0.4, 1.285, 0.5, 194..226).assert_occlusion()
}

#[test]
fn outline_keeps_the_camera_border_beside_its_near_coplanar_presentation_plate()
-> Result<(), String> {
    let mut fixture = OutlineDepthFixture::new(0.4);
    fixture.add_camera_presentation_plate()?;
    fixture.assert_occlusion()
}

#[test]
fn translucent_outlined_surface_hides_back_wire_but_keeps_front_and_exterior_wire()
-> Result<(), String> {
    assert_translucent_wire_occlusion(true)
}

#[test]
fn translucent_surface_without_an_outline_hides_back_wire() -> Result<(), String> {
    assert_translucent_wire_occlusion(false)
}

fn assert_translucent_wire_occlusion(outlined: bool) -> Result<(), String> {
    let mut fixture = OutlineDepthFixture::with_foreground(0.4, 10.0, 1.0, 0..0);
    // Camera sizing publishes its shared outline-depth target in PostUpdate.
    fixture.app.update();
    let world = fixture.app.world_mut();
    let depth = world
        .query::<&OutlineDepthTexture>()
        .single(world)
        .map_err(|error| error.to_string())?
        .clone();
    let material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgba(0.8, 0.86, 0.95, 0.25),
            alpha_mode: AlphaMode::AlphaToCoverage,
            specular_transmission: 0.8,
            thickness: 0.4,
            ior: 1.0,
            unlit: true,
            ..default()
        });
    world.entity_mut(fixture.outlined_body).insert((
        MeshMaterial3d(material),
        Outline::jump_flood(5.0).with_color(Color::WHITE).build(),
    ));
    if !outlined {
        world.entity_mut(fixture.outlined_body).remove::<Outline>();
    }
    for (x, z, color) in [
        (0.0, -0.2, Color::srgb(1.0, 0.0, 0.0)),
        (1.03, -0.2, Color::srgb(1.0, 0.0, 0.0)),
        (0.5, 0.2, Color::srgb(0.0, 1.0, 0.0)),
    ] {
        let mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(0.06, 3.0, 0.01));
        let material = world
            .resource_mut::<Assets<ExtendedMaterial<StandardMaterial, OutlineDepthTest>>>()
            .add(ExtendedMaterial {
                base:      StandardMaterial {
                    base_color: color,
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                },
                extension: OutlineDepthTest {
                    translucent: depth.translucent.clone(),
                    depth:       depth.surface.clone(),
                    strokes:     depth.strokes.clone(),
                },
            });
        world.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_xyz(x, 0.0, z),
        ));
    }
    let mut pixels = Vec::new();
    for _ in 0..128 {
        fixture.app.update();
        pixels = fixture.pixels()?;
        let exterior = &pixels[(40 * usize::from(SIDE) + 128) * 4..][..4];
        let front = &pixels[(128 * usize::from(SIDE) + 160) * 4..][..4];
        let outline = &pixels[(128 * usize::from(SIDE) + 61) * 4..][..4];
        let back = &pixels[(128 * usize::from(SIDE) + 128) * 4..][..4];
        if exterior[0] > 200
            && front[1] > 200
            && if outlined {
                outline[..3].iter().all(|value| *value > 200)
            } else {
                back[0] <= back[1].saturating_add(20)
            }
        {
            break;
        }
    }
    let pixel = |x, y| &pixels[(y * usize::from(SIDE) + x) * 4..][..4];
    assert!(
        !outlined || pixel(61, 128)[..3].iter().all(|value| *value > 200),
        "outline mask and composition must be ready within 128 frames: {:?}",
        pixel(61, 128)
    );
    assert!(
        pixel(128, 40)[0] > 200,
        "back wire must remain visible outside the surface"
    );
    assert!(
        pixel(160, 128)[1] > 200,
        "front wire must remain visible over the surface"
    );
    let back = pixel(128, 128);
    assert!(
        back[0] <= back[1].saturating_add(20),
        "translucent surface must hide back wire: {back:?}"
    );
    let fringe = pixel(194, 128);
    assert!(
        !outlined || fringe[0] <= fringe[1].saturating_add(20),
        "expanded outline must hide back wire outside the surface: {fringe:?}"
    );
    Ok(())
}
