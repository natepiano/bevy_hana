// Shared material-table constants and helpers.
//
// Rust mirror: render/material_table.rs.

#define_import_path hana_diegetic::material_table

// PathExtension::uniforms uniform buffer.
const PATH_UNIFORM_BINDING: u32 = 100u;
// PathExtension::curves storage buffer.
const PATH_CURVES_BINDING: u32 = 101u;
// PathExtension::bands storage buffer.
const PATH_BANDS_BINDING: u32 = 102u;
// PathExtension::path_records storage buffer.
const PATH_RECORDS_BINDING: u32 = 103u;
// PathExtension::instances storage buffer.
const PATH_INSTANCES_BINDING: u32 = 104u;
// PathExtension::run_records storage buffer.
const PATH_RUN_RECORDS_BINDING: u32 = 105u;
// Shared MaterialSlotValues storage buffer.
const MATERIAL_TABLE_BINDING: u32 = 106u;
// Batched SDF render-record storage buffer.
const SDF_RENDER_RECORDS_BINDING: u32 = 107u;
// Batched SDF mesh-record storage buffer.
const SDF_MESH_BINDING: u32 = 108u;

// SdfPaintMaterial::NotAuthored GPU row sentinel.
const INVALID_GPU_MATERIAL_SLOT: u32 = 4294967295u;

// Scalar/vector StandardMaterial values stored per material table row.
struct MaterialSlotValues {
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    attenuation_color: vec4<f32>,
    uv_transform: mat3x3<f32>,
    reflectance: vec3<f32>,
    roughness: f32,
    metallic: f32,
    diffuse_transmission: f32,
    specular_transmission: f32,
    thickness: f32,
    ior: f32,
    attenuation_distance: f32,
    clearcoat: f32,
    clearcoat_perceptual_roughness: f32,
    anisotropy_strength: f32,
    anisotropy_rotation: vec2<f32>,
}

// Element-local material sampling convention shared by SDF, text, and shapes:
// (0, 0) is the top-left of the layout box and (1, 1) is the bottom-right.
fn compute_material_sampled_uv(box_uv: vec2<f32>, uv_transform: mat3x3<f32>) -> vec2<f32> {
    return (uv_transform * vec3<f32>(box_uv, 1.0)).xy;
}
