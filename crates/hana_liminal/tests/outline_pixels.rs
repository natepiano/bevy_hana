//! Rendered occlusion checks for a selected body's outline.

use std::sync::mpsc;

use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::gizmos::GizmoAsset;
use bevy::gizmos::config::GizmoLineConfig;
use bevy::gizmos::retained::Gizmo as RetainedGizmo;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::PollType;
use bevy::render::render_resource::TextureFormat;
use bevy::render::renderer::RenderDevice;
use bevy::render::view::Tonemapping;
use bevy::render::view::screenshot::Screenshot;
use bevy::render::view::screenshot::ScreenshotCaptured;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;
use hana_liminal::OverlapMode;

const IMAGE_SIDE: u32 = 256;

fn capture(app: &mut App, target: Handle<Image>) -> Result<Vec<u8>, String> {
    let (sender, receiver) = mpsc::channel();
    app.world_mut().spawn(Screenshot::image(target)).observe(
        move |event: On<ScreenshotCaptured>| {
            let _ = sender.send(event.image.data.clone());
        },
    );
    // Bevy finishes the readback on its async compute pool, which shader
    // compilation can hold for many frames, so this waits on the capture itself.
    loop {
        app.update();
        app.world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(PollType::Wait {
                submission_index: None,
                timeout:          None,
            })
            .map_err(|error| format!("outline screenshot readback failed: {error}"))?;
        if let Ok(Some(pixels)) = receiver.try_recv() {
            return Ok(pixels);
        }
    }
}

const fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let offset = ((y * IMAGE_SIDE + x) * 4) as usize;
    [pixels[offset], pixels[offset + 1], pixels[offset + 2]]
}

fn outline_scene() -> (App, Handle<Image>, Entity) {
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
            .disable::<WinitPlugin>(),
    );
    app.add_plugins(LiminalPlugin);
    app.insert_resource(ClearColor(Color::BLACK));
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            IMAGE_SIDE,
            IMAGE_SIDE,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Image(target.clone().into()),
        Msaa::Off,
        OutlineCamera,
        Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let cube = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.5, 1.5, 1.5));
    let card = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.5, 0.7, 0.05));
    let body_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.45, 0.6),
            unlit: true,
            ..default()
        });
    let card_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.8, 0.12, 0.1),
            unlit: true,
            ..default()
        });
    let selected = app
        .world_mut()
        .spawn((
            Mesh3d(cube),
            MeshMaterial3d(body_material),
            Outline::jump_flood(5.0)
                .with_color(Color::WHITE)
                .with_overlap(OverlapMode::Merged)
                .build(),
            Transform::IDENTITY,
        ))
        .id();
    app.world_mut().spawn((
        Mesh3d(card),
        MeshMaterial3d(card_material),
        Transform::from_xyz(0.0, 0.52, 0.95),
    ));
    (app, target, selected)
}

#[test]
fn a_selected_outline_hides_behind_a_card_in_front() -> Result<(), String> {
    let (mut app, target, selected) = outline_scene();
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    let outlined = capture(&mut app, target.clone())?;
    app.world_mut().entity_mut(selected).remove::<Outline>();
    for _ in 0..4 {
        app.update();
    }
    let plain = capture(&mut app, target)?;
    let mut card_pixels = 0;
    let mut visible_outline_pixels = 0;
    for y in 16..IMAGE_SIDE - 16 {
        for x in 16..IMAGE_SIDE - 16 {
            let before = pixel(&plain, x, y);
            let after = pixel(&outlined, x, y);
            if u16::from(before[0]) > u16::from(before[1]) * 2
                && u16::from(before[0]) > u16::from(before[2]) * 2
            {
                card_pixels += 1;
                assert_eq!(
                    after, before,
                    "outline crossed the foreground card at {x}, {y}"
                );
            } else if after != before {
                visible_outline_pixels += 1;
            }
        }
    }
    assert!(card_pixels > 100, "foreground card did not draw");
    assert!(visible_outline_pixels > 20, "selected outline did not draw");
    Ok(())
}

fn floor_outline_scene() -> Result<(App, Handle<Image>, Entity), String> {
    let (mut app, target, selected) = outline_scene();
    let plate = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.5, 1.5, 0.04));
    app.world_mut().entity_mut(selected).insert(Mesh3d(plate));
    let card = {
        let world = app.world_mut();
        let mut meshes = world.query_filtered::<Entity, With<Mesh3d>>();
        meshes.iter(world).find(|&entity| entity != selected)
    };
    if let Some(card) = card {
        app.world_mut().despawn(card);
    }
    let camera = {
        let world = app.world_mut();
        let mut cameras = world.query_filtered::<Entity, With<Camera3d>>();
        cameras.iter(world).next()
    }
    .ok_or("outline scene has no camera")?;
    app.world_mut().entity_mut(camera).insert((
        Hdr,
        Bloom::default(),
        Tonemapping::KhronosPbrNeutral,
        Transform::from_xyz(0.0, 1.2, 4.0).looking_at(Vec3::new(0.0, -0.2, 0.0), Vec3::Y),
    ));
    let mut outline = app
        .world_mut()
        .get_mut::<Outline>(selected)
        .ok_or("selected box lost its outline")?;
    outline.color = Color::srgb(0.2, 0.5, 1.0);
    outline.intensity = 35.0;

    let floor_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.08, 0.09, 0.1),
            unlit: true,
            ..default()
        });
    let floor_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(5.0, 0.004, 5.0));
    app.world_mut().spawn((
        Mesh3d(floor_mesh),
        MeshMaterial3d(floor_material),
        Transform::from_xyz(0.0, -0.754, 0.0),
    ));
    let mut grid = GizmoAsset::new();
    let mut grid_x = -1.44_f32;
    for _ in 0..13 {
        grid.line(
            Vec3::new(grid_x, -0.75, -2.5),
            Vec3::new(grid_x, -0.75, 2.5),
            Color::srgb(0.45, 0.46, 0.47),
        );
        grid_x += 0.24;
    }
    let grid = app
        .world_mut()
        .resource_mut::<Assets<GizmoAsset>>()
        .add(grid);
    app.world_mut().spawn(RetainedGizmo {
        handle: grid,
        line_config: GizmoLineConfig {
            width: 1.5,
            ..default()
        },
        ..default()
    });
    let border_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.38, 0.49, 0.62),
            unlit: true,
            ..default()
        });
    for (size, position) in [
        (Vec3::new(1.54, 0.04, 0.006), Vec3::new(0.0, 0.766, 0.023)),
        (Vec3::new(0.04, 1.54, 0.006), Vec3::new(-0.766, 0.0, 0.023)),
    ] {
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::from_size(size));
        app.world_mut().spawn((
            Mesh3d(mesh),
            MeshMaterial3d(border_material.clone()),
            Transform::from_translation(position),
            ChildOf(selected),
        ));
    }
    Ok((app, target, selected))
}

#[test]
fn a_selected_box_on_the_floor_has_one_continuous_outline_on_all_four_edges() -> Result<(), String>
{
    let (mut app, target, selected) = floor_outline_scene()?;
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    let outlined = capture(&mut app, target.clone())?;
    app.world_mut().entity_mut(selected).remove::<Outline>();
    for _ in 0..4 {
        app.update();
    }
    let plain = capture(&mut app, target)?;
    assert_solid_outline(&outlined, &plain)
}

fn assert_solid_outline(outlined: &[u8], plain: &[u8]) -> Result<(), String> {
    let changed = |x, y| {
        let before = pixel(plain, x, y);
        let after = pixel(outlined, x, y);
        after
            .iter()
            .zip(before)
            .map(|(&a, b)| i32::from(a) - i32::from(b))
            .sum::<i32>()
    };
    let is_core = |x, y| {
        let color = pixel(outlined, x, y);
        color[0] > 210 && color[1] > 210 && color[2] > 235
    };
    let mut bounds = (IMAGE_SIDE, IMAGE_SIDE, 0, 0);
    for y in 16..IMAGE_SIDE - 16 {
        for x in 16..IMAGE_SIDE - 16 {
            if is_core(x, y) {
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x);
                bounds.3 = bounds.3.max(y);
            }
        }
    }
    assert!(
        bounds.2 > bounds.0 + 50 && bounds.3 > bounds.1 + 50,
        "selected box has no four-sided outline: {bounds:?}"
    );
    let (left, top, right, bottom) = bounds;
    let middle_x = u32::midpoint(left, right);
    let middle_y = u32::midpoint(top, bottom);
    let edge_peak = |axis: u32, center: u32, vertical: bool| {
        (axis.saturating_sub(12)..=axis + 12)
            .flat_map(|edge| (center - 4..=center + 4).map(move |along| (edge, along)))
            .filter(|&(edge, along)| edge < IMAGE_SIDE && along < IMAGE_SIDE)
            .map(|(edge, along)| {
                if vertical {
                    pixel(outlined, edge, along)
                } else {
                    pixel(outlined, along, edge)
                }
            })
            .map(|color| color.into_iter().map(i32::from).sum::<i32>())
            .max()
            .unwrap_or_default()
    };
    let peaks = [
        edge_peak(left, middle_y, true),
        edge_peak(top, middle_x, false),
        edge_peak(right, middle_y, true),
        edge_peak(bottom, middle_x, false),
    ];
    let weakest = *peaks.iter().min().ok_or("no outline edges")?;
    let strongest = *peaks.iter().max().ok_or("no outline edges")?;
    assert!(
        weakest > 650 && strongest - weakest < 60,
        "all four edges need the same bright core and glow: {peaks:?}"
    );
    assert_matching_glow_profiles(
        &changed,
        [
            (left, middle_y, true, true),
            (top, middle_x, false, true),
            (right, middle_y, true, false),
            (bottom, middle_x, false, false),
        ],
    );
    for offset in -20..=20 {
        let x = middle_x
            .checked_add_signed(offset)
            .ok_or("outline x left image")?;
        let y = middle_y
            .checked_add_signed(offset)
            .ok_or("outline y left image")?;
        for (name, axis, along, vertical) in [
            ("left", left, y, true),
            ("top", top, x, false),
            ("right", right, y, true),
            ("bottom", bottom, x, false),
        ] {
            let core = edge_peak(axis, along, vertical);
            assert!(
                core > 650,
                "{name} edge loses its bright core at {along}: {core}"
            );
        }
    }
    for x in left + 20..right - 20 {
        let core = (bottom.saturating_sub(4)..=bottom + 4).any(|y| y < IMAGE_SIDE && is_core(x, y));
        assert!(core, "floor grid interrupted the bottom outline at x={x}");
    }
    Ok(())
}

fn assert_matching_glow_profiles(
    changed: &impl Fn(u32, u32) -> i32,
    edges: [(u32, u32, bool, bool); 4],
) {
    let energies = edges.map(|(axis, middle, vertical, inward_positive)| {
        (0..=20)
            .map(|distance| {
                let cross = if inward_positive {
                    axis.saturating_sub(distance)
                } else {
                    axis + distance
                };
                (middle - 20..=middle + 20)
                    .map(|along| {
                        if vertical {
                            changed(cross, along)
                        } else {
                            changed(along, cross)
                        }
                    })
                    .max()
                    .unwrap_or_default()
                    .max(0)
            })
            .sum::<i32>()
    });
    let weakest = energies.into_iter().min().unwrap_or_default();
    let strongest = energies.into_iter().max().unwrap_or_default();
    assert!(
        weakest > 1_700 && strongest <= weakest + weakest * 3 / 4,
        "the four edges need matching core and glow energy: {energies:?}"
    );
}
