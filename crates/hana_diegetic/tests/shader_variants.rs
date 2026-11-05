//! Links current diegetic shaders for production scene and prepass descriptors.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use bevy::camera::RenderTarget;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::core_pipeline::prepass::NormalPrepass;
use bevy::ecs::system::RunSystemOnce;
use bevy::log::LogPlugin;
use bevy::material::MaterialProperties;
use bevy::material::key::ErasedMaterialPipelineKey;
use bevy::material::key::ErasedMeshPipelineKey;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::MATERIAL_BIND_GROUP_INDEX;
use bevy::pbr::MeshPipelineKey;
use bevy::pbr::PreparedMaterial;
use bevy::pbr::PrepassPipeline;
use bevy::pbr::PrepassPipelineSpecializer;
use bevy::pbr::RenderMaterialInstances;
use bevy::pbr::RenderMeshInstances;
use bevy::pbr::alpha_mode_pipeline_key;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::RenderPlugin;
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::mesh::RenderMesh;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::PipelineCache;
use bevy::render::render_resource::PipelineDescriptor;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::render::render_resource::SpecializedMeshPipeline;
use bevy::render::render_resource::TextureFormat;
use bevy::render::renderer::RenderDevice;
use bevy::shader::ShaderCache;
use bevy::shader::ShaderDefVal;
use bevy::window::ExitCondition;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use bevy::winit::WinitPlugin;
use hana_diegetic::Anchor;
use hana_diegetic::CascadeEntityCommandsExt;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Lighting;
use hana_diegetic::Mm;
use hana_diegetic::PanelDraw;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelLine;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::Px;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::ScreenSpaceCamera;
use hana_diegetic::SurfaceShadow;
use hana_diegetic::TextStyle;
use hana_diegetic::Tooltip;
use hana_shader_testing::ShaderRecord;
use hana_shader_testing::call_sites;
use hana_shader_testing::definition_enabled;
use hana_shader_testing::load_module;

struct ShaderSource {
    source_path: &'static str,
    asset_path:  &'static str,
}

const SOURCES: [ShaderSource; 7] = [
    ShaderSource {
        source_path: "crates/hana_diegetic/src/render/analytic_paths/analytic_path.wesl",
        asset_path:  "embedded://hana_diegetic/analytic_path.wesl",
    },
    ShaderSource {
        source_path: "crates/hana_diegetic/src/render/analytic_paths/analytic_path_vertex_pull.wesl",
        asset_path:  "embedded://hana_diegetic/analytic_path_vertex_pull.wesl",
    },
    ShaderSource {
        source_path: "crates/hana_diegetic/src/render/material_table.wesl",
        asset_path:  "embedded://hana_diegetic/material_table.wesl",
    },
    ShaderSource {
        source_path: "crates/hana_diegetic/src/render/sdf_stroke.wesl",
        asset_path:  "embedded://hana_diegetic/sdf_stroke.wesl",
    },
    ShaderSource {
        source_path: "crates/hana_diegetic/src/shaders/sdf_material_table.wesl",
        asset_path:  "embedded://hana_diegetic/sdf_material_table.wesl",
    },
    ShaderSource {
        source_path: "crates/hana_diegetic/src/shaders/image_panel.wesl",
        asset_path:  "embedded://hana_diegetic/image_panel.wesl",
    },
    ShaderSource {
        source_path: "crates/hana_diegetic/src/shaders/sdf_panel.wesl",
        asset_path:  "embedded://hana_diegetic/sdf_panel.wesl",
    },
];
const ALPHA_MODES: [AlphaMode; 7] = [
    AlphaMode::Opaque,
    AlphaMode::Mask(0.5),
    AlphaMode::Blend,
    AlphaMode::Premultiplied,
    AlphaMode::Add,
    AlphaMode::Multiply,
    AlphaMode::AlphaToCoverage,
];

fn scenario_tree(image: Handle<Image>, alpha_mode: AlphaMode) -> LayoutTree {
    let line = PanelLine::new((2.0, 2.0), (30.0, 18.0))
        .width(Mm(1.0))
        .color(Color::WHITE);
    let mut tree = LayoutBuilder::with_root(
        El::column()
            .size(40.0, 24.0)
            .background(Color::WHITE)
            .draw(PanelDraw::lines([line])),
    );
    tree.text((
        "Ag",
        TextStyle::new(Mm(4.0))
            .with_alpha_mode(alpha_mode)
            .with_shadow_mode(GlyphShadowMode::Cast),
    ));
    tree.image(El::new().size(4.0, 4.0), image, Color::WHITE);
    tree.build()
}

fn spawn_panels(app: &mut App, window: Entity, image: &Handle<Image>) -> Result<(), String> {
    for alpha_mode in ALPHA_MODES {
        for lighting in [Lighting::Lit, Lighting::Unlit] {
            let material = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    alpha_mode,
                    unlit: lighting == Lighting::Unlit,
                    double_sided: true,
                    ..default()
                });
            let world_panel = DiegeticPanel::world()
                .size(Mm(40.0), Mm(24.0))
                .anchor(Anchor::Center)
                .surface_shadow(SurfaceShadow::On)
                .material(material.clone())
                .text_material(material.clone())
                .shape_material(material.clone())
                .with_tree(scenario_tree(image.clone(), alpha_mode))
                .build()
                .map_err(|error| error.to_string())?;
            let screen_panel = DiegeticPanel::screen()
                .size(Px(40.0), Px(24.0))
                .window_entity(window)
                .screen_position(32.0, 32.0)
                .material(material.clone())
                .text_material(material.clone())
                .shape_material(material)
                .with_tree(scenario_tree(image.clone(), alpha_mode))
                .build()
                .map_err(|error| error.to_string())?;
            for entity in [
                app.world_mut()
                    .spawn((world_panel, Transform::default()))
                    .id(),
                app.world_mut().spawn(screen_panel).id(),
            ] {
                app.world_mut()
                    .commands()
                    .entity(entity)
                    .override_lighting(lighting);
            }
        }
    }
    Ok(())
}

fn spawn_tooltip_host(app: &mut App) -> Result<Entity, String> {
    let tooltip = Tooltip::new(El::column().size(24.0, 8.0)).show_after(Duration::ZERO);
    let mut tree = LayoutBuilder::with_root(El::column().size(40.0, 24.0).background(Color::WHITE));
    tree.with(
        El::new()
            .size(20.0, 8.0)
            .button("record-control")
            .tooltip(tooltip),
        |_| {},
    );
    let panel = DiegeticPanel::world()
        .size(Mm(40.0), Mm(24.0))
        .anchor(Anchor::Center)
        .with_tree(tree.build())
        .build()
        .map_err(|error| error.to_string())?;
    Ok(app
        .world_mut()
        .spawn((panel, Transform::from_xyz(0.0, 0.0, 0.02)))
        .id())
}

fn spawn_cameras(app: &mut App, window: Entity) {
    for oit in [false, true] {
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    64,
                    64,
                    TextureFormat::Rgba8UnormSrgb,
                    None,
                ));
        let mut camera = app.world_mut().spawn((
            Camera3d::default(),
            Msaa::Off,
            DepthPrepass,
            NormalPrepass,
            RenderTarget::Image(target.into()),
            Transform::from_xyz(0.0, 0.0, 0.1).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        if oit {
            camera.insert(OrderIndependentTransparencySettings::default());
        }
    }
    app.world_mut().spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(1.0, 1.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    app.world_mut().spawn((
        Camera3d::default(),
        Msaa::Off,
        RenderTarget::Window(WindowRef::Entity(window)),
        Transform::from_xyz(0.0, 0.0, 0.1).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn diegetic_app() -> Result<App, String> {
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
            .disable::<PipelinedRenderingPlugin>()
            .disable::<LogPlugin>(),
    )
    .add_plugins(DiegeticUiPlugin);
    app.finish();
    app.cleanup();
    let window = app
        .world_mut()
        .spawn((
            Window {
                resolution: (64, 64).into(),
                ..default()
            },
            PrimaryWindow,
        ))
        .id();
    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    spawn_panels(&mut app, window, &image)?;
    spawn_cameras(&mut app, window);
    let host = spawn_tooltip_host(&mut app)?;
    for _ in 0..3 {
        app.update();
    }
    let control = app
        .world_mut()
        .run_system_once(move |reader: PanelWidgetReader| {
            reader.entity(host, &PanelElementId::named("record-control"))
        })
        .map_err(|error| error.to_string())?
        .ok_or("tooltip control absent")?;
    app.world_mut().trigger(RequestWidgetFocus {
        window,
        widget: control,
    });
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            64,
            64,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    let world = app.world_mut();
    let mut cameras =
        world.query_filtered::<(&mut RenderTarget, &mut Msaa), With<ScreenSpaceCamera>>();
    for (mut destination, mut msaa) in cameras.iter_mut(world) {
        *destination = RenderTarget::Image(target.clone().into());
        *msaa = Msaa::Off;
    }
    for _ in 0..8 {
        app.update();
    }
    let world = app.world_mut();
    let cameras = world
        .query_filtered::<Entity, With<ScreenSpaceCamera>>()
        .iter(world)
        .collect::<Vec<_>>();
    for camera in cameras {
        world
            .entity_mut(camera)
            .insert(OrderIndependentTransparencySettings::default());
    }
    for _ in 0..4 {
        app.update();
    }
    hana_shader_testing::wait_for_shaders(&mut app)?;
    Ok(app)
}

fn stripped_prepass_descriptor(
    specializer: &PrepassPipelineSpecializer,
    key: ErasedMaterialPipelineKey,
    layout: &MeshVertexBufferLayoutRef,
) -> Result<RenderPipelineDescriptor, String> {
    let properties = &specializer.properties;
    let base = PrepassPipelineSpecializer {
        pipeline:   specializer.pipeline.clone(),
        properties: Arc::new(MaterialProperties {
            material_layout: properties.material_layout.clone(),
            shaders: properties.shaders.clone(),
            bindless: properties.bindless,
            ..default()
        }),
    };
    let mut descriptor = base
        .specialize(key.clone(), layout)
        .map_err(|error| error.to_string())?;
    descriptor
        .layout
        .get_mut(MATERIAL_BIND_GROUP_INDEX)
        .ok_or("prepass material layout absent")?
        .entries
        .clear();
    let specialize = properties
        .user_specialize
        .ok_or("material specializer absent")?;
    specialize(
        &specializer.pipeline.material_pipeline,
        &mut descriptor,
        layout,
        key,
    )
    .map_err(|error| error.to_string())?;
    Ok(descriptor)
}

fn prepass_descriptors(app: &App) -> Result<Vec<RenderPipelineDescriptor>, String> {
    let world = app.sub_app(RenderApp).world();
    let materials = world.resource::<ErasedRenderAssets<PreparedMaterial>>();
    let meshes = world.resource::<RenderAssets<RenderMesh>>();
    let instances = world.resource::<RenderMaterialInstances>();
    let mesh_instances = world.resource::<RenderMeshInstances>();
    let mut descriptors = Vec::new();
    let mut visited = HashSet::new();
    for (entity, instance) in &instances.instances {
        if !visited.insert(instance.asset_id) {
            continue;
        }
        let Some(material) = materials.get(instance.asset_id) else {
            continue;
        };
        let Some(mesh_instance) = mesh_instances.render_mesh_queue_data(*entity) else {
            continue;
        };
        let Some(mesh) = meshes.get(mesh_instance.mesh_asset_id()) else {
            continue;
        };
        let specializer = PrepassPipelineSpecializer {
            pipeline:   world.resource::<PrepassPipeline>().clone(),
            properties: material.properties.clone(),
        };
        let material_bits: MeshPipelineKey = material.properties.mesh_pipeline_key_bits.downcast();
        for prepass in [
            MeshPipelineKey::DEPTH_PREPASS,
            MeshPipelineKey::DEPTH_PREPASS | MeshPipelineKey::NORMAL_PREPASS,
            MeshPipelineKey::DEPTH_PREPASS
                | MeshPipelineKey::NORMAL_PREPASS
                | MeshPipelineKey::MOTION_VECTOR_PREPASS,
            MeshPipelineKey::DEPTH_PREPASS | MeshPipelineKey::DEFERRED_PREPASS,
            MeshPipelineKey::DEPTH_PREPASS | MeshPipelineKey::UNCLIPPED_DEPTH_ORTHO,
        ] {
            let mesh_key = material_bits
                | MeshPipelineKey::from_bits_retain(mesh.key_bits.bits())
                | MeshPipelineKey::from_msaa_samples(1)
                | prepass
                | alpha_mode_pipeline_key(material.properties.alpha_mode, &Msaa::Off);
            let key = ErasedMaterialPipelineKey {
                mesh_key:     ErasedMeshPipelineKey::new(mesh_key),
                material_key: material.properties.material_key.clone(),
                type_id:      instance.asset_id.type_id(),
            };
            let descriptor = specializer
                .specialize(key.clone(), &mesh.layout)
                .map_err(|error| error.to_string())?;
            descriptors.push(descriptor);
            if prepass == MeshPipelineKey::DEPTH_PREPASS
                || prepass == (MeshPipelineKey::DEPTH_PREPASS | MeshPipelineKey::NORMAL_PREPASS)
            {
                descriptors.push(stripped_prepass_descriptor(
                    &specializer,
                    key,
                    &mesh.layout,
                )?);
            }
        }
    }
    Ok(descriptors)
}

#[test]
fn shader_variants() -> Result<(), String> {
    let app = diegetic_app()?;
    let mut descriptors = prepass_descriptors(&app)?;
    descriptors.extend(
        app.sub_app(RenderApp)
            .world()
            .resource::<PipelineCache>()
            .pipelines()
            .filter_map(|pipeline| match &pipeline.descriptor {
                PipelineDescriptor::RenderPipelineDescriptor(descriptor) => {
                    Some((**descriptor).clone())
                },
                PipelineDescriptor::ComputePipelineDescriptor(_)
                | PipelineDescriptor::MeshPipelineDescriptor(_) => None,
            }),
    );
    let shaders = app.world().resource::<Assets<Shader>>();
    let device = app.world().resource::<RenderDevice>();
    let mut cache = ShaderCache::new((), |(), source, validate| load_module(source, validate));
    for (id, shader) in shaders.iter() {
        let mut shader = shader.clone();
        shader.shader_defs.push(ShaderDefVal::UInt(
            "AVAILABLE_STORAGE_BUFFER_BINDINGS".into(),
            device.limits().max_storage_buffers_per_shader_stage,
        ));
        cache.set_shader(id, shader);
    }
    let owned = shaders
        .iter()
        .filter_map(|(id, shader)| {
            let source = SOURCES
                .iter()
                .find(|source| shader.path.replace('\\', "/") == source.asset_path)?;
            Some((id, source.source_path))
        })
        .collect::<HashMap<_, _>>();
    assert_eq!(
        owned.len(),
        SOURCES.len(),
        "plugin must register every inventoried source"
    );
    let source_paths = SOURCES.map(|source| source.source_path);
    let mut record = ShaderRecord::new("hana_diegetic", &source_paths);
    let mut covered = HashSet::new();
    let mut definitions_seen = HashSet::new();
    for (index, descriptor) in descriptors.iter().enumerate() {
        for (handle, definitions) in
            std::iter::once((&descriptor.vertex.shader, &descriptor.vertex.shader_defs)).chain(
                descriptor
                    .fragment
                    .iter()
                    .map(|fragment| (&fragment.shader, &fragment.shader_defs)),
            )
        {
            let Some(path) = owned.get(&handle.id()) else {
                continue;
            };
            let module = cache
                .get(index, handle.id(), definitions)
                .map_err(|error| format!("{path}: {definitions:?}: {error}"))?;
            if *path == source_paths[0] || *path == source_paths[1] {
                let mode = if *path == source_paths[1] {
                    AnalyticShaderMode::VertexPull
                } else {
                    AnalyticShaderMode::Coverage
                };
                require_analytic_link(&module, definitions, mode)?;
            }
            if *path == source_paths[6] {
                require_sdf_imports(&module, definitions)?;
            }
            if !definition_enabled(definitions, "MATERIAL_OIT_ENABLED") {
                assert!(
                    !module.functions.iter().any(|(_, function)| function
                        .name
                        .as_deref()
                        .is_some_and(|name| name.ends_with("oit_draw"))),
                    "view OIT without material OIT linked oit_draw: {path}: {definitions:?}"
                );
            }
            record.record(path, definitions, &module)?;
            covered.insert(*path);
            for definition in definitions {
                if let ShaderDefVal::Bool(name, true) = definition {
                    definitions_seen.insert(name.as_ref());
                }
            }
        }
    }
    require_scenario_coverage(&covered, &definitions_seen);
    record.finish()
}

fn require_scenario_coverage(covered: &HashSet<&str>, definitions_seen: &HashSet<&str>) {
    let source_paths = SOURCES.map(|source| source.source_path);
    for path in [
        source_paths[0],
        source_paths[1],
        source_paths[5],
        source_paths[6],
    ] {
        assert!(covered.contains(path), "root descriptor absent: {path}");
    }
    for definition in [
        "NON_TEXT_PATH_COVERAGE",
        "PATH_PREMULTIPLIED_OIT",
        "PATH_PREMULTIPLIED_NON_OIT",
        "FRAGMENT_DATA_FROM_BATCHED_PATHS",
        "SDF_STRIPPED_MATERIAL_GROUP",
        "SDF_PBR_LIGHTING",
        "SDF_TOOLTIP_SHADOW",
        "PREPASS_PIPELINE",
    ] {
        assert!(
            definitions_seen.contains(definition),
            "scenario definition absent: {definition}"
        );
    }
}

fn named_function<'a>(module: &'a naga::Module, name: &str) -> Result<&'a naga::Function, String> {
    module
        .functions
        .iter()
        .find_map(|(_, function)| (function.name.as_deref() == Some(name)).then_some(function))
        .ok_or_else(|| format!("analytic function {name} is absent"))
}

fn calls_to(module: &naga::Module, caller: &naga::Function, name: &str) -> usize {
    call_sites(caller)
        .iter()
        .filter(|(function, _)| module.functions[*function].name.as_deref() == Some(name))
        .count()
}

fn require_one_analytic_call(
    module: &naga::Module,
    caller: &naga::Function,
    name: &str,
) -> Result<(), String> {
    // Count syntactic paths, including repeated calls to the same helper. WGSL
    // forbids recursive calls, so walking the linked call graph terminates.
    let mut pending = vec![caller];
    let mut reachable = 0;
    while let Some(function) = pending.pop() {
        for (target, _) in call_sites(function) {
            let target_function = &module.functions[target];
            if target_function.name.as_deref() == Some(name) {
                reachable += 1;
            } else {
                pending.push(target_function);
            }
        }
    }
    let total: usize = module
        .functions
        .iter()
        .map(|(_, function)| calls_to(module, function, name))
        .chain(
            module
                .entry_points
                .iter()
                .map(|entry| calls_to(module, &entry.function, name)),
        )
        .sum();
    if reachable != 1 || total != 1 {
        return Err(format!(
            "analytic call {name}: expected one path from {:?} and one call site total, actual {reachable}/{total}",
            caller.name
        ));
    }
    Ok(())
}

enum AnalyticShaderMode {
    VertexPull,
    Coverage,
}

// PathExtension::instances uses this binding in its AsBindGroup declaration;
// production exposes no shared constant for that attribute value.
const ANALYTIC_INSTANCE_BINDING: u32 = 104;

fn require_analytic_link(
    module: &naga::Module,
    definitions: &[ShaderDefVal],
    mode: AnalyticShaderMode,
) -> Result<(), String> {
    let entry = module
        .entry_points
        .first()
        .ok_or("analytic entry point is absent")?;
    if matches!(mode, AnalyticShaderMode::VertexPull) {
        require_one_analytic_call(module, &entry.function, "pull_vertex")?;
        return Ok(());
    }
    let prepass = definition_enabled(definitions, "PREPASS_PIPELINE");
    // Camera prepasses use render_coverage -> path_coverage -> sample_terms;
    // shadow prepasses call the curve helpers directly from the fragment.
    for name in ["band_walk_terms", "lane_winding_at"] {
        require_one_analytic_call(module, &entry.function, name)?;
    }
    if !prepass {
        require_one_analytic_call(
            module,
            named_function(module, "path_coverage")?,
            "sample_terms",
        )?;
        for suffix in [
            "sdf_material_table__4pbr_input_from_material_table",
            "material_table__3INVALID_GPU_MATERIAL_SLOT",
        ] {
            let resolved = module.functions.iter().any(|(_, function)| {
                function
                    .name
                    .as_deref()
                    .is_some_and(|name| name.ends_with(suffix))
            }) || module.constants.iter().any(|(_, constant)| {
                constant
                    .name
                    .as_deref()
                    .is_some_and(|name| name.ends_with(suffix))
            });
            if !resolved {
                return Err(format!("analytic Hana import {suffix} did not resolve"));
            }
        }
        let polygon = module
            .functions
            .iter()
            .any(|(_, function)| function.name.as_deref() == Some("analytic_polygon_coverage"));
        if polygon != definition_enabled(definitions, "NON_TEXT_PATH_COVERAGE") {
            return Err("analytic polygon coverage does not match NON_TEXT_PATH_COVERAGE".into());
        }
    }
    let batched = module.global_variables.iter().any(|(_, variable)| {
        variable.binding.as_ref().is_some_and(|binding| {
            usize::try_from(binding.group).ok() == Some(MATERIAL_BIND_GROUP_INDEX)
                && binding.binding == ANALYTIC_INSTANCE_BINDING
        })
    });
    if batched != definition_enabled(definitions, "FRAGMENT_DATA_FROM_BATCHED_PATHS") {
        return Err(
            "analytic instances declaration does not match FRAGMENT_DATA_FROM_BATCHED_PATHS".into(),
        );
    }
    Ok(())
}

#[test]
fn shader_variants_analytic_fragment_path_rejects_missing_or_repeated_helpers() -> Result<(), String>
{
    for (source, expected_valid) in [
        (
            "fn band_walk_terms() {} fn sample_terms() { band_walk_terms(); } fn fragment() { sample_terms(); }",
            true,
        ),
        (
            "fn band_walk_terms() {} fn sample_terms() { band_walk_terms(); } fn fragment() {}",
            false,
        ),
        (
            "fn band_walk_terms() {} fn sample_terms() { band_walk_terms(); } fn fragment() { sample_terms(); sample_terms(); }",
            false,
        ),
        (
            "fn band_walk_terms() {} fn sample_terms() {} fn fragment() { sample_terms(); }",
            false,
        ),
    ] {
        let module = naga::front::wgsl::parse_str(source).map_err(|error| error.to_string())?;
        assert_eq!(
            require_one_analytic_call(
                &module,
                named_function(&module, "fragment")?,
                "band_walk_terms"
            )
            .is_ok(),
            expected_valid,
            "fragment call graph: {source}",
        );
    }
    Ok(())
}

#[test]
fn shader_variants_analytic_curve_calls_reject_multiple_sites() -> Result<(), String> {
    let module = naga::front::wgsl::parse_str("fn band_walk_terms() {} fn sample_terms() { band_walk_terms(); if true { band_walk_terms(); } }")
        .map_err(|error| error.to_string())?;
    assert!(
        require_one_analytic_call(
            &module,
            named_function(&module, "sample_terms")?,
            "band_walk_terms"
        )
        .is_err()
    );
    let module = naga::front::wgsl::parse_str("fn band_walk_terms() {} fn sample_terms() { band_walk_terms(); } fn other() { band_walk_terms(); }")
        .map_err(|error| error.to_string())?;
    assert!(
        require_one_analytic_call(
            &module,
            named_function(&module, "sample_terms")?,
            "band_walk_terms"
        )
        .is_err()
    );
    let module = naga::front::wgsl::parse_str("fn band_walk_terms() {} fn sample_terms() { for (var i = 0; i < 4; i++) { band_walk_terms(); } }")
        .map_err(|error| error.to_string())?;
    require_one_analytic_call(
        &module,
        named_function(&module, "sample_terms")?,
        "band_walk_terms",
    )
}

fn require_sdf_imports(module: &naga::Module, definitions: &[ShaderDefVal]) -> Result<(), String> {
    if definition_enabled(definitions, "SDF_STRIPPED_MATERIAL_GROUP") {
        return Ok(());
    }
    let function = |name: &str| {
        module.functions.iter().any(|(_, function)| {
            function
                .name
                .as_deref()
                .is_some_and(|linked| linked.ends_with(name))
        })
    };
    if !function("sdf_material_table__4pbr_input_from_material_table") {
        return Err("SDF material module function did not resolve".into());
    }
    let material_constant = module.constants.iter().any(|(_, constant)| {
        constant
            .name
            .as_deref()
            .is_some_and(|linked| linked.ends_with("material_table__3INVALID_GPU_MATERIAL_SLOT"))
    });
    if !material_constant {
        return Err("SDF material table sentinel did not resolve".into());
    }
    if !definition_enabled(definitions, "PREPASS_PIPELINE")
        && !definition_enabled(definitions, "SDF_TOOLTIP_SHADOW")
    {
        for name in [
            "sdf_stroke__2centered_stroke_alpha",
            "sdf_stroke__2distance_field_band",
            "sdf_stroke__3inflate_subpixel_half_size",
        ] {
            if !function(name) {
                return Err(format!("SDF stroke module function {name} did not resolve"));
            }
        }
    }
    Ok(())
}
