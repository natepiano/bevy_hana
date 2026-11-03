//! First-frame text fitting matches a fresh panel, and quiet frames publish no buffers.

mod drawn_support;

#[path = "drawn_support/paired_capture.rs"]
mod paired_capture;

use bevy::asset::AssetId;
use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::ScalingMode;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::message::MessageCursor;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::render::view::DebandDither;
use bevy::render::view::Tonemapping;
use bevy::window::WindowRef;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::DiegeticTextBatch;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelText;
use hana_diegetic::StableTransparency;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;

const VIEW_SIZE: UVec2 = UVec2::new(128, 96);
const LABEL: &str = "text-fit-label";

fn camera(app: &mut CaseApp, layer: usize) -> Handle<Image> {
    if layer == 0 {
        app.world_mut().spawn((
            Camera3d::default(),
            RenderTarget::Window(WindowRef::Primary),
            Hdr,
            Tonemapping::KhronosPbrNeutral,
            RenderLayers::none(),
        ));
    }
    let region = CaptureRegion::new(VIEW_SIZE, UVec2::ZERO, VIEW_SIZE);
    let target = app.target(region);
    let mut capture_camera = Camera::default();
    region.apply_to_camera(&mut capture_camera);
    app.world_mut().spawn((
        Camera3d::default(),
        capture_camera,
        RenderTarget::Image(target.clone().into()),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        DebandDither::Disabled,
        Msaa::Off,
        StableTransparency,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 0.096,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 0.0, 0.25).looking_at(Vec3::ZERO, Vec3::Y),
        RenderLayers::layer(layer),
    ));
    target
}

fn panel(app: &mut CaseApp, layer: usize, text: &str, width: f32) -> Result<Entity, String> {
    let mut builder =
        LayoutBuilder::with_root(El::new().size(width, 48.0).padding(Padding::all(8.0)));
    builder.text(
        Text::new(
            text,
            TextStyle::new(Mm(8.0)).with_shadow_mode(GlyphShadowMode::None),
        )
        .id(LABEL),
    );
    let panel = DiegeticPanel::world()
        .size(Mm(width), Mm(48.0))
        .anchor(Anchor::Center)
        .with_tree(builder.build())
        .build()
        .map_err(|error| format!("text fit panel: {error}"))?;
    Ok(app
        .world_mut()
        .spawn((panel, Transform::default(), RenderLayers::layer(layer)))
        .id())
}

fn text_mesh(app: &mut CaseApp, layer: usize) -> Result<AssetId<Mesh>, String> {
    let world = app.world_mut();
    let meshes = world
        .query_filtered::<(&Mesh3d, &RenderLayers), With<DiegeticTextBatch>>()
        .iter(world)
        .filter(|(_, layers)| **layers == RenderLayers::layer(layer))
        .map(|(mesh, _)| mesh.id())
        .collect::<Vec<_>>();
    let [mesh] = meshes.as_slice() else {
        return Err(format!(
            "expected one text batch on layer {layer}, found {meshes:?}"
        ));
    };
    Ok(*mesh)
}

fn quiet_payload_uploads() -> Result<(), String> {
    let mut app = CaseApp::new();
    camera(&mut app, 0);
    panel(&mut app, 0, "AAAAA", 72.0)?;
    app.settle()?;
    let mut cursor = MessageCursor::<AssetEvent<ShaderBuffer>>::default();
    cursor.clear(app.world().resource::<Messages<AssetEvent<ShaderBuffer>>>());
    app.update();
    let perf = app.world().resource::<DiegeticPerfStats>();
    assert_eq!(perf.batch.instance_uploads.get(), 0);
    assert_eq!(perf.batch.run_table_uploads.get(), 0);
    let changed = cursor
        .read(app.world().resource::<Messages<AssetEvent<ShaderBuffer>>>())
        .filter(|event| {
            matches!(
                event,
                AssetEvent::Added { .. } | AssetEvent::Modified { .. }
            )
        })
        .collect::<Vec<_>>();
    assert!(
        changed.is_empty(),
        "quiet frame publishes buffers: {changed:?}"
    );
    Ok(())
}

enum TextMeshAssets {
    ExistingBatches,
    DistinctAssets,
}

fn changed_text_pixels() -> Result<(), String> {
    text_pixels_with_mesh_assets(TextMeshAssets::ExistingBatches)
}

fn distinct_mesh_text_pixels() -> Result<(), String> {
    text_pixels_with_mesh_assets(TextMeshAssets::DistinctAssets)
}

fn text_pixels_with_mesh_assets(assets: TextMeshAssets) -> Result<(), String> {
    let mut app = CaseApp::new();
    let retained = camera(&mut app, 0);
    let reference = camera(&mut app, 1);
    let changed = panel(&mut app, 0, "AAAAA", 72.0)?;
    panel(&mut app, 1, "BBBBB", 72.0)?;
    app.settle()?;
    match assets {
        TextMeshAssets::ExistingBatches => {},
        TextMeshAssets::DistinctAssets => {
            // Identical payloads in separate mesh assets isolate sharing from
            // the retained text update and its first-frame buffer publication.
            let world = app.world_mut();
            let batches = world
                .query_filtered::<(Entity, &Mesh3d), With<DiegeticTextBatch>>()
                .iter(world)
                .map(|(entity, mesh)| (entity, mesh.0.clone()))
                .collect::<Vec<_>>();
            for (entity, mesh) in batches {
                let distinct = {
                    let mut assets = world.resource_mut::<Assets<Mesh>>();
                    let payload = assets.get(&mesh).ok_or("control mesh missing")?.clone();
                    assets.add(payload)
                };
                world.entity_mut(entity).insert(Mesh3d(distinct));
            }
            assert_ne!(text_mesh(&mut app, 0)?, text_mesh(&mut app, 1)?);
        },
    }
    let reference_mesh = text_mesh(&mut app, 1)?;
    let before = app.capture(retained.clone())?;
    let mesh = text_mesh(&mut app, 0)?;
    let updated = app
        .world_mut()
        .run_system_once(move |mut text: PanelText| {
            text.set_text(changed, &PanelElementId::named(LABEL), "BBBBB")
        })
        .map_err(|error| format!("set retained text: {error}"))?;
    assert!(updated, "named text run was not changed");
    let [actual, expected] = paired_capture::capture_pair(&mut app, [retained, reference])?;
    let region = CaptureRegion::new(VIEW_SIZE, UVec2::ZERO, VIEW_SIZE);
    drawn_support::dump_pixels::<false>(
        "panel_text_fit_pixels",
        "",
        "retained text=BBBBB",
        region,
        region,
        &actual,
    )?;
    drawn_support::dump_pixels::<false>(
        "panel_text_fit_pixels",
        "",
        "fresh text=BBBBB",
        region,
        region,
        &expected,
    )?;
    assert_eq!(mesh, text_mesh(&mut app, 0)?);
    assert_eq!(reference_mesh, text_mesh(&mut app, 1)?);
    assert_ne!(before, actual, "new glyph payload never drew");
    assert!(
        expected
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 32)
    );
    let mismatches = actual
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (actual, expected))| actual.abs_diff(**expected) > 1)
        .map(|(index, (actual, expected))| (index, *actual, *expected))
        .collect::<Vec<_>>();
    assert_eq!(
        mismatches.len(),
        0,
        "retained text differs from fresh final text: {:?}",
        &mismatches[..mismatches.len().min(16)]
    );
    Ok(())
}

#[test]
fn unchanged_frame_publishes_no_buffer_payloads() -> Result<(), String> {
    drawn_support::run_cases(&[DrawnCase {
        name: "quiet_payload_uploads",
        run:  quiet_payload_uploads,
    }])
}

#[test]
fn changed_text_matches_fresh_text_on_first_changed_frame() -> Result<(), String> {
    drawn_support::run_cases(&[DrawnCase {
        name: "changed_text_pixels",
        run:  changed_text_pixels,
    }])
}

#[test]
fn changed_text_draws_new_pixels_with_distinct_mesh_assets() -> Result<(), String> {
    drawn_support::run_cases(&[DrawnCase {
        name: "distinct_mesh_text_pixels",
        run:  distinct_mesh_text_pixels,
    }])
}
