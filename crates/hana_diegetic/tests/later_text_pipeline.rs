//! The first frame draws its text with ready analytic-path pipelines, and text
//! added after the scene shows never holds a frame while its pipeline compiles.

use std::collections::HashSet;
use std::path::Path;

use bevy::camera::RenderTarget;
use bevy::pbr::SpecializedShadowMaterialPipelineCache;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::RenderPlugin;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::render_resource::CachedPipelineState;
use bevy::render::render_resource::PipelineCache;
use bevy::render::render_resource::PipelineDescriptor;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::render::render_resource::TextureFormat;
use bevy::shader::ShaderCacheError;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt as _;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;

/// File name of the coverage stage every analytic-path pipeline compiles.
const ANALYTIC_PATH_SHADER: &str = "analytic_path.wesl";
/// File name of the vertex stage that pulls path quads, the one analytic-path
/// stage of a shadow pipeline with no fragment stage.
const ANALYTIC_PATH_VERTEX_PULL_SHADER: &str = "analytic_path_vertex_pull.wesl";
/// File names of the shaders the first frame's text and panels compile.
const FIRST_FRAME_SHADERS: [&str; 3] = [ANALYTIC_PATH_SHADER, "sdf_panel.wesl", "image_panel.wesl"];
const CAMERA_DISTANCE: f32 = 0.1;
const PANEL_SIZE_MM: Vec2 = Vec2::new(40.0, 10.0);
const PANEL_SPACING: f32 = 0.012;
const TARGET_SIDE: u32 = 128;
const TEXT_SIZE_MM: f32 = 4.0;

/// Where one analytic-path pipeline is at the end of a frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PipelineReadiness {
    /// Queued, or waiting on a shader that has not loaded yet.
    Waiting,
    Creating,
    Ready,
    Failed,
}

impl From<&CachedPipelineState> for PipelineReadiness {
    fn from(state: &CachedPipelineState) -> Self {
        match state {
            CachedPipelineState::Queued
            | CachedPipelineState::Err(
                ShaderCacheError::ShaderNotLoaded(_)
                | ShaderCacheError::ShaderImportNotYetAvailable,
            ) => Self::Waiting,
            CachedPipelineState::Creating(_) => Self::Creating,
            CachedPipelineState::Ok(_) => Self::Ready,
            CachedPipelineState::Err(_) => Self::Failed,
        }
    }
}

/// One analytic-path pipeline: its index in `PipelineCache::pipelines`, which
/// is its `CachedRenderPipelineId`, and its readiness.
#[derive(Clone, Copy, Debug)]
struct AnalyticPathPipeline {
    index:     usize,
    readiness: PipelineReadiness,
}

/// Builds a headless renderer that compiles pipelines on Bevy's async compute
/// pool, with the render sub-app run inline so each `App::update` ends after
/// its frame is submitted. A shadow-casting light makes shadow-casting text
/// queue shadow pipelines too.
fn asynchronous_compile_app() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: false,
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<PipelinedRenderingPlugin>(),
    );
    app.add_plugins(DiegeticUiPlugin);
    app.world_mut().insert_resource(Lighting::Unlit);
    app.finish();
    app.cleanup();
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            TARGET_SIDE,
            TARGET_SIDE,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Image(target.into()),
        Transform::from_xyz(0.0, 0.0, CAMERA_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.world_mut().spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(1.0, 1.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app
}

/// One text panel's words, how they blend and whether they cast a shadow.
struct TextPanel<'a> {
    text:        &'a str,
    alpha_mode:  AlphaMode,
    shadow_mode: GlyphShadowMode,
    height:      f32,
}

fn spawn_text_panel(app: &mut App, panel: TextPanel) -> Result<(), String> {
    let TextPanel {
        text,
        alpha_mode,
        shadow_mode,
        height,
    } = panel;
    let mut layout = LayoutBuilder::new(PANEL_SIZE_MM.x, PANEL_SIZE_MM.y);
    layout.with(
        El::new()
            .width(Sizing::grow_min(0.0))
            .height(Sizing::grow_min(0.0)),
        |builder| {
            builder.text((
                text,
                TextStyle::new(Mm(TEXT_SIZE_MM))
                    .with_shadow_mode(shadow_mode)
                    .with_alpha_mode(alpha_mode),
            ));
        },
    );
    let panel = DiegeticPanel::world()
        .size(Mm(PANEL_SIZE_MM.x), Mm(PANEL_SIZE_MM.y))
        .anchor(Anchor::Center)
        .with_tree(layout.build())
        .build()
        .map_err(|error| format!("invalid text panel: {error}"))?;
    let entity = app
        .world_mut()
        .spawn((panel, Transform::from_xyz(0.0, height, 0.0)))
        .id();
    app.world_mut()
        .commands()
        .entity(entity)
        .override_lighting(Lighting::Unlit);
    Ok(())
}

/// The file name a shader was loaded from. The crate loads its shaders behind
/// stable handles, which carry no asset path, so the name comes from the
/// loaded `Shader` itself.
fn shader_file_name<'a>(shaders: &'a Assets<Shader>, handle: &Handle<Shader>) -> Option<&'a str> {
    Path::new(&shaders.get(handle)?.path).file_name()?.to_str()
}

/// Whether a pipeline draws analytic paths: its fragment stage computes path
/// coverage or its vertex stage pulls path quads.
fn draws_analytic_paths(shaders: &Assets<Shader>, descriptor: &RenderPipelineDescriptor) -> bool {
    shader_file_name(shaders, &descriptor.vertex.shader) == Some(ANALYTIC_PATH_VERTEX_PULL_SHADER)
        || descriptor.fragment.as_ref().is_some_and(|fragment| {
            shader_file_name(shaders, &fragment.shader) == Some(ANALYTIC_PATH_SHADER)
        })
}

fn analytic_path_pipelines(app: &App) -> Vec<AnalyticPathPipeline> {
    let shaders = app.world().resource::<Assets<Shader>>();
    app.sub_app(RenderApp)
        .world()
        .resource::<PipelineCache>()
        .pipelines()
        .enumerate()
        .filter_map(|(index, pipeline)| {
            let descriptor = match &pipeline.descriptor {
                PipelineDescriptor::RenderPipelineDescriptor(descriptor) => descriptor,
                PipelineDescriptor::ComputePipelineDescriptor(_)
                | PipelineDescriptor::MeshPipelineDescriptor(_) => return None,
            };
            draws_analytic_paths(shaders, descriptor).then(|| AnalyticPathPipeline {
                index,
                readiness: PipelineReadiness::from(&pipeline.state),
            })
        })
        .collect()
}

/// Indices of the pipelines the render world specialized for shadow views.
fn shadow_pipeline_indices(app: &App) -> HashSet<usize> {
    app.sub_app(RenderApp)
        .world()
        .resource::<SpecializedShadowMaterialPipelineCache>()
        .values()
        .flat_map(|view| view.values().map(|&(id, _, _)| id.id()))
        .collect()
}

/// The shaders a first frame compiles are in place once the plugins are
/// built, before any update: none of them waits on an asset load.
fn assert_first_frame_shaders_in_place(app: &App) -> Result<(), String> {
    let shaders = app.world().resource::<Assets<Shader>>();
    let loaded: HashSet<_> = shaders
        .iter()
        .filter_map(|(_, shader)| Path::new(&shader.path).file_name()?.to_str())
        .collect();
    let missing: Vec<_> = FIRST_FRAME_SHADERS
        .into_iter()
        .filter(|name| !loaded.contains(name))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "plugin build left shaders to load later: {missing:?}"
        ))
    }
}

/// Text spawned before the first update draws on the first frame: that frame
/// ends with every analytic-path pipeline it queued ready, its shadow
/// pipelines included.
fn assert_first_frame_text_pipelines_ready(app: &mut App) -> Result<(), String> {
    spawn_text_panel(
        app,
        TextPanel {
            text:        "shown first",
            alpha_mode:  AlphaMode::Blend,
            shadow_mode: GlyphShadowMode::Cast,
            height:      PANEL_SPACING,
        },
    )?;
    app.update();
    let first_frame = analytic_path_pipelines(app);
    let shadow_indices = shadow_pipeline_indices(app);
    if !first_frame
        .iter()
        .any(|pipeline| shadow_indices.contains(&pipeline.index))
    {
        return Err(format!(
            "the first frame queued no analytic-path shadow pipeline: {first_frame:?}"
        ));
    }
    if first_frame
        .iter()
        .any(|pipeline| pipeline.readiness != PipelineReadiness::Ready)
    {
        return Err(format!(
            "the first frame ended with analytic-path pipelines not ready, so its text did not \
             draw: {first_frame:?}"
        ));
    }
    Ok(())
}

/// Runs frames until every analytic-path pipeline from `first_index` on is
/// ready, and fails when one of them fails to compile.
fn update_until_ready(app: &mut App, first_index: usize) -> Result<(), String> {
    loop {
        app.update();
        let pipelines: Vec<_> = analytic_path_pipelines(app)
            .into_iter()
            .filter(|pipeline| pipeline.index >= first_index)
            .collect();
        if pipelines
            .iter()
            .any(|pipeline| pipeline.readiness == PipelineReadiness::Failed)
        {
            return Err(format!("an analytic-path pipeline failed: {pipelines:?}"));
        }
        if !pipelines.is_empty()
            && pipelines
                .iter()
                .all(|pipeline| pipeline.readiness == PipelineReadiness::Ready)
        {
            return Ok(());
        }
    }
}

/// Runs frames until no pipeline has a compile task in flight. Only
/// `PipelineCache::process_queue`, inside `App::update`, starts those tasks, so
/// dropping the app afterwards never tears the render device down while the
/// NVIDIA driver is inside `create_render_pipeline`, which segfaults the process.
fn update_until_no_pipeline_compiles(app: &mut App) {
    while app
        .sub_app(RenderApp)
        .world()
        .resource::<PipelineCache>()
        .pipelines()
        .any(|pipeline| matches!(pipeline.state, CachedPipelineState::Creating(_)))
    {
        app.update();
    }
}

fn assert_later_text_pipeline_does_not_hold_its_frame(app: &mut App) -> Result<(), String> {
    update_until_ready(app, 0)?;

    // Premultiplied text needs a pipeline the blended text did not compile.
    let first_later_index = app
        .sub_app(RenderApp)
        .world()
        .resource::<PipelineCache>()
        .pipelines()
        .count();
    spawn_text_panel(
        app,
        TextPanel {
            text:        "added later",
            alpha_mode:  AlphaMode::Premultiplied,
            shadow_mode: GlyphShadowMode::None,
            height:      -PANEL_SPACING,
        },
    )?;
    let queued = loop {
        app.update();
        let queued: Vec<_> = analytic_path_pipelines(app)
            .into_iter()
            .filter(|pipeline| pipeline.index >= first_later_index)
            .collect();
        if !queued.is_empty() {
            break queued;
        }
    };
    // `PipelineCache::process_queue` starts a queued pipeline's compile task and
    // polls it on the next call, so a frame that did not block on the task ends
    // with it still creating.
    if queued
        .iter()
        .any(|pipeline| pipeline.readiness != PipelineReadiness::Creating)
    {
        return Err(format!(
            "the frame that queued later text ended with its pipelines past creating, so it \
             blocked on them: {queued:?}"
        ));
    }
    update_until_ready(app, first_later_index)
}

#[test]
fn first_frame_waits_for_its_text_and_later_text_compiles_without_holding_a_frame()
-> Result<(), String> {
    let mut app = asynchronous_compile_app();
    let result = assert_first_frame_shaders_in_place(&app)
        .and_then(|()| assert_first_frame_text_pipelines_ready(&mut app))
        .and_then(|()| assert_later_text_pipeline_does_not_hold_its_frame(&mut app));
    update_until_no_pipeline_compiles(&mut app);
    result
}
