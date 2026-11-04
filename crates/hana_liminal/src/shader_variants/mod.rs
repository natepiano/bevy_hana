//! Links every descriptor definition set for this crate's embedded shader roots.

mod contracts;

use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::RenderPlugin;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::shader::ShaderCache;
use bevy::shader::ShaderDefVal;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use contracts::OutlineContracts;

#[path = "../../../hana_diegetic/tests/shader_record/mod.rs"]
mod shader_record;

use std::collections::HashMap;

use bevy::log::LogPlugin;
use bevy::mesh::VertexAttributeValues;
use bevy::pbr::MeshPipelineKey;
use bevy::pbr::MeshUniform;
use bevy::render::settings::WgpuSettings;
use shader_record::ShaderRecord;

use crate::constants::ATTRIBUTE_OUTLINE_NORMAL;
use crate::constants::HAS_OUTLINE_NORMALS_SHADER_DEF;
use crate::constants::HULL_SHADER_HANDLE;
use crate::constants::MASK_SHADER_HANDLE;
use crate::constants::OUTLINE_INSTANCE_BATCH_SIZE_SHADER_DEF;
use crate::hull_pipeline::DynamicRange;
use crate::hull_pipeline::HullPipeline;
use crate::hull_pipeline::HullPipelineKey;
use crate::hull_pipeline::OutlineNormalPresence;
use crate::mask_pipeline::MeshMaskPipeline;
use crate::uniforms::OutlineUniform;

#[derive(Clone, Copy)]
enum InstanceBuffers {
    Storage,
    Uniform,
}

fn headless_app(buffers: InstanceBuffers) -> App {
    let mut app = App::new();
    let mut settings = WgpuSettings::default();
    if matches!(buffers, InstanceBuffers::Uniform) {
        settings.constrained_limits = Some(WgpuLimits {
            max_storage_buffers_per_shader_stage: 0,
            ..WgpuLimits::default()
        });
    }
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: settings.into(),
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<PipelinedRenderingPlugin>()
            .disable::<LogPlugin>(),
    );
    app
}

fn shader_variants_for_descriptors(
    app: &App,
    descriptors: &[RenderPipelineDescriptor],
    record: &mut ShaderRecord,
    contracts: &mut OutlineContracts,
) -> Result<(), String> {
    let device = app.world().resource::<RenderDevice>();
    let mut cache = ShaderCache::new((), |(), source, validate| {
        shader_record::load_module(source, validate)
    });
    let shaders = app.world().resource::<Assets<Shader>>();
    for (id, shader) in shaders.iter() {
        let mut shader = shader.clone();
        shader.shader_defs.push(ShaderDefVal::UInt(
            "AVAILABLE_STORAGE_BUFFER_BINDINGS".into(),
            device.limits().max_storage_buffers_per_shader_stage,
        ));
        cache.set_shader(id, shader);
    }
    let registered_sources = shaders
        .iter()
        .filter_map(|(id, shader)| {
            SOURCES
                .iter()
                .copied()
                .find(|path| {
                    shader.path.replace('\\', "/").ends_with(
                        path.strip_prefix("crates/")
                            .unwrap_or(path)
                            .replace("/src/shaders/", "/")
                            .as_str(),
                    )
                })
                .map(|path| (id, path))
        })
        .collect::<HashMap<_, _>>();
    assert_eq!(
        registered_sources.len(),
        SOURCES.len(),
        "plugin must register all product outline shaders"
    );
    let mut variants = std::collections::HashSet::new();
    let mut roots = std::collections::HashSet::new();
    for descriptor in descriptors {
        let stages = std::iter::once((&descriptor.vertex.shader, &descriptor.vertex.shader_defs))
            .chain(
                descriptor
                    .fragment
                    .iter()
                    .map(|fragment| (&fragment.shader, &fragment.shader_defs)),
            );
        for (handle, definitions) in stages {
            let Some(&path) = registered_sources.get(&handle.id()) else {
                continue;
            };
            if !variants.insert((handle.id(), definitions.clone())) {
                continue;
            }
            let module = cache
                .get(0, handle.id(), definitions)
                .map_err(|error| format!("{path}: {definitions:?}: {error}"))?;
            contracts.check(path, &module, definitions)?;
            record.record(path, definitions, &module)?;
            roots.insert(path);
        }
    }
    assert_eq!(
        roots.len(),
        ROOT_COUNT,
        "descriptor scenarios must cover every root"
    );
    println!(
        "{} (storage bindings: {}): {} roots, {} distinct ordered definition lists",
        CRATE_NAME,
        device.limits().max_storage_buffers_per_shader_stage,
        roots.len(),
        variants.len()
    );
    Ok(())
}

fn captured_descriptors(app: &App) -> Vec<RenderPipelineDescriptor> {
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
        })
        .collect()
}

const CRATE_NAME: &str = "hana_liminal";
const SOURCES: &[&str] = &[
    "crates/hana_liminal/src/shaders/compose_output.wesl",
    "crates/hana_liminal/src/shaders/flood.wesl",
    "crates/hana_liminal/src/shaders/hull.wesl",
    "crates/hana_liminal/src/shaders/mask.wesl",
    "crates/hana_liminal/src/shaders/view_helpers.wesl",
];
const ROOT_COUNT: usize = 4;

#[test]
fn shader_variants_link_morph_targets_and_distinct_uniform_arrays() -> Result<(), String> {
    let mut record = shader_record::ShaderRecord::new(CRATE_NAME, SOURCES);
    let mut contracts = OutlineContracts::default();
    for buffers in [InstanceBuffers::Storage, InstanceBuffers::Uniform] {
        let mut app = headless_app(buffers);
        app.add_plugins(crate::LiminalPlugin);
        app.finish();
        app.cleanup();
        shader_record::wait_for_shaders(&mut app)?;
        let mut descriptors = captured_descriptors(&app);
        descriptors.extend(outline_descriptors(
            app.sub_app(RenderApp).world(),
            buffers,
        )?);
        if matches!(buffers, InstanceBuffers::Storage) {
            append_nondefault_outline_batches(&mut descriptors);
        }
        shader_variants_for_descriptors(&app, &descriptors, &mut record, &mut contracts)?;
    }
    contracts.finish();
    record.finish()
}

// A deliberately non-default extent catches loss of the WESL numeric constant.
const TEST_OUTLINE_BATCH_SIZE: u32 = 7;

fn append_nondefault_outline_batches(descriptors: &mut Vec<RenderPipelineDescriptor>) {
    let additional = descriptors
        .iter()
        .filter(|descriptor| {
            descriptor.vertex.shader == HULL_SHADER_HANDLE
                || descriptor.vertex.shader == MASK_SHADER_HANDLE
        })
        .cloned()
        .map(|mut descriptor| {
            for definitions in std::iter::once(&mut descriptor.vertex.shader_defs).chain(
                descriptor
                    .fragment
                    .iter_mut()
                    .map(|fragment| &mut fragment.shader_defs),
            ) {
                definitions.retain(|definition| {
                    !matches!(definition,
                    ShaderDefVal::UInt(name, _) if name == OUTLINE_INSTANCE_BATCH_SIZE_SHADER_DEF)
                });
                definitions.push(ShaderDefVal::UInt(
                    OUTLINE_INSTANCE_BATCH_SIZE_SHADER_DEF.into(),
                    TEST_OUTLINE_BATCH_SIZE,
                ));
            }
            descriptor
        })
        .collect::<Vec<_>>();
    descriptors.extend(additional);
}

fn outline_descriptors(
    world: &World,
    buffers: InstanceBuffers,
) -> Result<Vec<RenderPipelineDescriptor>, String> {
    let hull = world.resource::<HullPipeline>();
    let mask = world.resource::<MeshMaskPipeline>();
    let limits = world.resource::<RenderDevice>().limits();
    let capacity = match GpuArrayBuffer::<OutlineUniform>::batch_size(&limits) {
        Some(outline) => OutlineInstanceCapacity::UniformArrays {
            outline,
            mesh: GpuArrayBuffer::<MeshUniform>::batch_size(&limits)
                .ok_or("mesh uniform array capacity absent")?,
        },
        None => OutlineInstanceCapacity::UnboundedStorage,
    };
    assert_eq!(
        matches!(capacity, OutlineInstanceCapacity::UniformArrays { .. }),
        matches!(buffers, InstanceBuffers::Uniform)
    );
    let mut layouts = bevy::mesh::MeshVertexBufferLayouts::default();
    let mut descriptors = Vec::new();
    for features in [
        MeshFeatures::Positions,
        MeshFeatures::Surface,
        MeshFeatures::AllAttributes,
        MeshFeatures::Skinned,
    ] {
        for normals in [
            OutlineNormalPresence::Absent,
            OutlineNormalPresence::Present,
        ] {
            let mut mesh = features.mesh();
            if matches!(normals, OutlineNormalPresence::Present) {
                let vertex_count = mesh.count_vertices();
                mesh.insert_attribute(
                    ATTRIBUTE_OUTLINE_NORMAL,
                    vec![[0.0, 0.0, 1.0]; vertex_count],
                );
            }
            let layout = mesh.get_mesh_vertex_buffer_layout(&mut layouts);
            for dynamic_range in [DynamicRange::Sdr, DynamicRange::Hdr] {
                for view_key in outline_view_keys(&mesh, dynamic_range) {
                    for morph_targets in [MorphTargetPresence::Absent, MorphTargetPresence::Present]
                    {
                        if matches!(features, MeshFeatures::Positions)
                            && matches!(morph_targets, MorphTargetPresence::Present)
                        {
                            continue;
                        }
                        let key = view_key | morph_targets.pipeline_key();
                        let hull_descriptor = hull
                            .specialize(
                                HullPipelineKey {
                                    mesh_pipeline_key: key,
                                    dynamic_range,
                                    outline_normal_presence: normals,
                                },
                                &layout,
                            )
                            .map_err(|error| error.to_string())?;
                        let mask_descriptor = mask
                            .specialize(key, &layout)
                            .map_err(|error| error.to_string())?;
                        assert_product_definitions(
                            &hull_descriptor,
                            &HULL_SHADER_HANDLE,
                            capacity,
                        )?;
                        assert_product_definitions(
                            &mask_descriptor,
                            &MASK_SHADER_HANDLE,
                            capacity,
                        )?;
                        assert_eq!(
                            hull_descriptor
                                .vertex
                                .shader_defs
                                .contains(&ShaderDefVal::Bool(
                                    HAS_OUTLINE_NORMALS_SHADER_DEF.into(),
                                    true
                                )),
                            matches!(normals, OutlineNormalPresence::Present)
                        );
                        for descriptor in [&hull_descriptor, &mask_descriptor] {
                            assert_eq!(
                                descriptor
                                    .vertex
                                    .shader_defs
                                    .contains(&ShaderDefVal::Bool("MORPH_TARGETS".into(), true)),
                                matches!(morph_targets, MorphTargetPresence::Present)
                            );
                        }
                        descriptors.push(hull_descriptor);
                        descriptors.push(mask_descriptor);
                    }
                }
            }
        }
    }
    Ok(descriptors)
}

fn outline_view_keys(mesh: &Mesh, dynamic_range: DynamicRange) -> [MeshPipelineKey; 4] {
    [
        (1, MeshPipelineKey::VIEW_PROJECTION_PERSPECTIVE),
        (1, MeshPipelineKey::VIEW_PROJECTION_ORTHOGRAPHIC),
        (4, MeshPipelineKey::VIEW_PROJECTION_PERSPECTIVE),
        (4, MeshPipelineKey::VIEW_PROJECTION_ORTHOGRAPHIC),
    ]
    .map(|(sample_count, projection)| {
        MeshPipelineKey::from_msaa_samples(sample_count)
            | MeshPipelineKey::from_target_format(if matches!(dynamic_range, DynamicRange::Hdr) {
                TextureFormat::Rgba16Float
            } else {
                TextureFormat::Rgba8UnormSrgb
            })
            | MeshPipelineKey::from_primitive_topology_and_strip_index(
                mesh.primitive_topology(),
                None,
            )
            | projection
    })
}

fn assert_product_definitions(
    descriptor: &RenderPipelineDescriptor,
    shader: &Handle<Shader>,
    capacity: OutlineInstanceCapacity,
) -> Result<(), String> {
    assert_eq!(&descriptor.vertex.shader, shader);
    let fragment = descriptor
        .fragment
        .as_ref()
        .ok_or("outline fragment stage absent")?;
    assert_eq!(&fragment.shader, shader);
    if let OutlineInstanceCapacity::UniformArrays { mesh, .. } = capacity {
        assert_eq!(
            descriptor
                .vertex
                .shader_defs
                .iter()
                .filter(|definition| matches!(definition,
                ShaderDefVal::UInt(name, _) if name == "PER_OBJECT_BUFFER_BATCH_SIZE"))
                .collect::<Vec<_>>(),
            vec![&ShaderDefVal::UInt(
                "PER_OBJECT_BUFFER_BATCH_SIZE".into(),
                mesh
            )]
        );
    }
    for definitions in [&descriptor.vertex.shader_defs, &fragment.shader_defs] {
        let actual = definitions.iter().rev().find(|definition| {
            matches!(definition,
            ShaderDefVal::UInt(name, _) if name == OUTLINE_INSTANCE_BATCH_SIZE_SHADER_DEF)
        });
        match capacity {
            OutlineInstanceCapacity::UnboundedStorage => assert!(actual.is_none()),
            OutlineInstanceCapacity::UniformArrays { outline, mesh } => {
                assert_eq!(
                    actual,
                    Some(&ShaderDefVal::UInt(
                        OUTLINE_INSTANCE_BATCH_SIZE_SHADER_DEF.into(),
                        outline
                    ))
                );
                assert_ne!(outline, mesh, "mesh and outline element capacities differ");
            },
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum OutlineInstanceCapacity {
    UnboundedStorage,
    UniformArrays { outline: u32, mesh: u32 },
}

#[derive(Clone, Copy)]
enum MorphTargetPresence {
    Absent,
    Present,
}

impl MorphTargetPresence {
    const fn pipeline_key(self) -> MeshPipelineKey {
        match self {
            Self::Absent => MeshPipelineKey::NONE,
            Self::Present => MeshPipelineKey::MORPH_TARGETS,
        }
    }
}

#[derive(Clone, Copy)]
enum MeshFeatures {
    Positions,
    Surface,
    AllAttributes,
    Skinned,
}

impl MeshFeatures {
    fn mesh(self) -> Mesh {
        let mut mesh = Mesh::from(Cuboid::default());
        let vertices = mesh.count_vertices();
        match self {
            Self::Positions => {
                mesh.remove_attribute(Mesh::ATTRIBUTE_NORMAL);
                mesh.remove_attribute(Mesh::ATTRIBUTE_UV_0);
            },
            Self::Surface => {},
            Self::AllAttributes | Self::Skinned => {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0, 0.0]; vertices]);
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_TANGENT,
                    vec![[1.0, 0.0, 0.0, 1.0]; vertices],
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, 1.0]; vertices]);
            },
        }
        if matches!(self, Self::Skinned) {
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_JOINT_INDEX,
                VertexAttributeValues::Uint16x4(vec![[0, 0, 0, 0]; vertices]),
            );
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_JOINT_WEIGHT,
                vec![[1.0, 0.0, 0.0, 0.0]; vertices],
            );
        }
        mesh
    }
}
