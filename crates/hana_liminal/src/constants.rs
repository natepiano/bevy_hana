use bevy::asset::Handle;
use bevy::asset::uuid_handle;
use bevy::mesh::MeshVertexAttribute;
use bevy::prelude::LinearRgba;
use bevy::prelude::Shader;
use bevy::render::render_resource::VertexFormat;

// batching
pub(super) const MISSING_BATCH_SET_INDEX: u32 = !0;

// bind group entries
pub(super) const OUTLINE_UNIFORM_BIND_GROUP_ENTRY_BINDING: u32 = 0;

// bind group labels
pub(super) const COMPOSE_OUTPUT_BIND_GROUP_LABEL: &str = "compose_output_bind_group";
pub(super) const HULL_DEPTH_BIND_GROUP_LABEL: &str = "hull_depth_bind_group";
pub(super) const HULL_OUTLINE_BIND_GROUP_LABEL: &str = "hull_outline_bind_group";
pub(super) const JUMP_FLOOD_BIND_GROUP_LABEL: &str = "outline_jump_flood_bind_group";
pub(super) const OUTLINE_BIND_GROUP_LABEL: &str = "outline_bind_group";

// bind group layout labels
pub(super) const HULL_DEPTH_BIND_GROUP_LAYOUT_LABEL: &str = "HullDepth";
pub(super) const HULL_OUTLINE_INSTANCE_BIND_GROUP_LAYOUT_LABEL: &str = "HullOutlineInstance";
pub(super) const JUMP_FLOOD_BIND_GROUP_LAYOUT_LABEL: &str = "outline_jump_flood_bind_group_layout";
pub(super) const OUTLINE_COMPOSE_OUTPUT_BIND_GROUP_LAYOUT_LABEL: &str =
    "outline_compose_output_bind_group_layout";
pub(super) const OUTLINE_COMPOSE_OUTPUT_MSAA_BIND_GROUP_LAYOUT_LABEL: &str =
    "outline_compose_output_bind_group_layout_msaa";
pub(super) const OUTLINE_INSTANCE_BIND_GROUP_LAYOUT_LABEL: &str = "OutlineInstance";

// bind group slots
pub(super) const COMPOSE_BIND_GROUP_SLOT: usize = 0;
pub(super) const HULL_DEPTH_BIND_GROUP_SLOT: usize = 4;
pub(super) const HULL_MESH_BIND_GROUP_SLOT: usize = 2;
pub(super) const HULL_MESH_VIEW_BIND_GROUP_SLOT: usize = 0;
pub(super) const HULL_MESH_VIEW_BINDING_ARRAY_BIND_GROUP_SLOT: usize = 1;
pub(super) const HULL_OUTLINE_BIND_GROUP_SLOT: usize = 3;
pub(super) const JUMP_FLOOD_BIND_GROUP_SLOT: usize = 0;
pub(super) const MESH_BIND_GROUP_SLOT: usize = 2;
pub(super) const MESH_VIEW_BIND_GROUP_SLOT: usize = 0;
pub(super) const MESH_VIEW_BINDING_ARRAY_BIND_GROUP_SLOT: usize = 1;
pub(super) const OUTLINE_BIND_GROUP_SLOT: usize = 3;

// logging
pub(super) const FAILED_TO_SPECIALIZE_HULL_MESH_PIPELINE_WARNING: &str =
    "Failed to specialize hull mesh pipeline";
pub(super) const FAILED_TO_SPECIALIZE_MESH_PIPELINE_WARNING: &str =
    "Failed to specialize mesh pipeline";
pub(super) const FLOOD_INIT_RENDER_ERROR: &str =
    "Error encountered while rendering the outline flood init phase";
pub(super) const GET_BATCH_DATA_GPU_MODE_ERROR: &str =
    "`get_batch_data` should never be called in GPU mesh uniform building mode";
pub(super) const GET_BINNED_BATCH_DATA_GPU_MODE_ERROR: &str =
    "`get_binned_batch_data` should never be called in GPU mesh uniform building mode";
pub(super) const GET_BINNED_INDEX_CPU_MODE_ERROR: &str =
    "`get_binned_index` should never be called in CPU mesh uniform building mode";
pub(super) const GET_INDEX_AND_COMPARE_DATA_CPU_MODE_ERROR: &str =
    "`get_index_and_compare_data` should never be called in CPU mesh uniform building mode";
pub(super) const HULL_OUTLINE_RENDER_ERROR: &str =
    "Error encountered while rendering hull outline phase";
pub(super) const LIMINAL_TRACING_TARGET: &str = "hana_liminal";
pub(super) const NO_GLOBAL_DEPTH_TEXTURE_WARNING: &str = "No global depth texture found";
pub(super) const NO_MESH_FOUND_WARNING: &str = "No mesh found for entity";
pub(super) const NO_MESH_INSTANCE_FOUND_WARNING: &str = "No mesh instance found for entity";

// outline rendering constants
/// Custom vertex attribute storing pre-computed smoothed outline normals.
///
/// These normals are averaged across all faces sharing a vertex position,
/// weighted by the angle at each face, producing smooth silhouette extrusion
/// even on hard-edged meshes.
pub const ATTRIBUTE_OUTLINE_NORMAL: MeshVertexAttribute =
    MeshVertexAttribute::new("Outline_Normal", 988_540_917, VertexFormat::Float32x3);

pub(super) const DEFAULT_OUTLINE_DRAW_PRIORITY: f32 = 0.0;

/// Multiplicative identity — no scaling applied to the outline color.
pub(super) const DEFAULT_OUTLINE_INTENSITY: f32 = 1.0;

/// Minimum edge length below which a triangle vertex is considered degenerate
/// and its angle-weighted normal contribution is skipped.
pub(super) const DEGENERATE_EDGE_THRESHOLD: f32 = 1e-10;

pub(super) const FULL_SCREEN_DRAW_INSTANCE_COUNT: u32 = 1;
pub(super) const HULL_DEPTH_BIAS_CLAMP: f32 = 0.0;
pub(super) const HULL_DEPTH_BIAS_CONSTANT: i32 = 0;
pub(super) const HULL_DEPTH_BIAS_SLOPE_SCALE: f32 = 0.0;
pub(super) const JUMP_FLOOD_DIAMETER_SCALE: f32 = 2.0;
pub(super) const JUMP_FLOOD_FINAL_COMPOSE_PASS_COUNT: u32 = 1;

/// Clear color for the `JumpFlood` seed texture. Negative coordinates signal
/// "no seed" to the flood-fill shader.
pub(super) const JUMP_FLOOD_NO_SEED_CLEAR_COLOR: LinearRgba =
    LinearRgba::new(-1.0, -1.0, -1.0, 0.0);

pub(super) const JUMP_FLOOD_RADIUS_DIVISOR: u32 = 2;
pub(super) const MERGED_OVERLAP_SHADER_FACTOR: f32 = 0.0;
pub(super) const MSAA_DISABLED_SAMPLE_COUNT: u32 = 1;
pub(super) const NO_FLOOD_PASS_COUNT: u32 = 0;
pub(super) const NO_FLOOD_WIDTH_THRESHOLD: f32 = 0.0;
pub(super) const NO_RENDER_PIPELINE_IMMEDIATE_DATA_SIZE: u32 = 0;
pub(super) const NON_SCREEN_HULL_SHELL_MODE_SHADER_FACTOR: f32 = 0.0;

/// Reverse-Z far-plane sentinel used when clearing the outline depth texture.
/// Cleared to 0.0 so that any rendered outline fragment (closer than the far
/// plane) will pass the depth comparison.
pub(super) const OUTLINE_DEPTH_FAR_PLANE_CLEAR: f32 = 0.0;

pub(super) const OUTLINE_NORMAL_COSINE_CLAMP_MAX: f32 = 1.0;
pub(super) const OUTLINE_NORMAL_COSINE_CLAMP_MIN: f32 = -1.0;

/// Shader binding location for the outline normal vertex attribute.
pub(super) const OUTLINE_NORMAL_SHADER_LOCATION: u32 = 8;

/// Outline render targets use only the base mip level — no mipmap chain.
pub(super) const OUTLINE_TEXTURE_BASE_MIP_ONLY: u32 = 1;

/// Outline render targets are 2D textures with a single (non-array) layer.
pub(super) const OUTLINE_TEXTURE_SINGLE_LAYER: u32 = 1;

pub(super) const OWNER_DATA_RESERVED_CHANNEL: f32 = 0.0;

/// Offset added to entity indices when computing owner IDs. Zero is reserved as
/// "no owner" in the shader, so all valid owner IDs start at 1.0.
pub(super) const OWNER_ID_OFFSET: f32 = 1.0;

pub(super) const SCREEN_HULL_SHELL_MODE_SHADER_FACTOR: f32 = 1.0;
pub(super) const SEPARATED_OVERLAP_SHADER_FACTOR: f32 = 1.0;

/// Number of vertices in each triangle primitive (mesh indices, draw counts).
pub(super) const TRIANGLE_VERTEX_COUNT: u32 = 3;

// pipeline labels
pub(super) const HULL_OUTLINE_PIPELINE_LABEL: &str = "hull_outline_pipeline";
pub(super) const OUTLINE_COMPOSE_OUTPUT_MSAA_PIPELINE_LABEL: &str =
    "outline_compose_output_pipeline_msaa";
pub(super) const OUTLINE_COMPOSE_OUTPUT_PIPELINE_LABEL: &str = "outline_compose_output_pipeline";
pub(super) const OUTLINE_JUMP_FLOOD_PIPELINE_LABEL: &str = "outline_jump_flood_pipeline";
pub(super) const OUTLINE_PIPELINE_LABEL: &str = "outline_pipeline";

// render pass labels
pub(super) const HULL_OUTLINE_PASS_LABEL: &str = "hull_outline_pass";
pub(super) const OUTLINE_FLOOD_INIT_PASS_LABEL: &str = "outline_flood_init";
pub(super) const OUTLINE_JUMP_FLOOD_PASS_LABEL: &str = "outline_jump_flood_pass";
pub(super) const POST_PROCESS_PASS_LABEL: &str = "post_process_pass";

// shader defs
pub(super) const HAS_OUTLINE_NORMALS_SHADER_DEF: &str = "HAS_OUTLINE_NORMALS";
pub(super) const MULTISAMPLED_SHADER_DEF: &str = "MULTISAMPLED";
pub(super) const PER_OBJECT_BUFFER_BATCH_SIZE_SHADER_DEF: &str = "PER_OBJECT_BUFFER_BATCH_SIZE";

// shader entry points
pub(super) const FRAGMENT_SHADER_ENTRY_POINT: &str = "fragment";

// shader handles
pub(super) const COMPOSE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("6fe0f3ef-e31f-40e7-a20a-ed002ac4bb3f");
pub(super) const FLOOD_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("a06a9919-18e3-4e91-a312-a1463bb6d719");
pub(super) const HULL_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("6b6c1df4-e857-4f9f-a4a3-4ca5f0bc4df4");
pub(super) const MASK_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("4c41a7eb-b802-4e76-97f1-3327d80743dd");
pub(super) const VIEW_HELPERS_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("a3e7c2b1-9d4f-4e8a-b5c6-1f2d3e4a5b6c");

// texture labels
pub(super) const OUTLINE_DEPTH_TEXTURE_LABEL: &str = "outline depth texture";

// view
pub(super) const PRIMARY_SUBVIEW_INDEX: u32 = 0;
