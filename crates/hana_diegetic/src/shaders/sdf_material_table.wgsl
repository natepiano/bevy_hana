#define_import_path hana_diegetic::sdf_material_table

#import bevy_pbr::pbr_types

#ifndef SDF_STRIPPED_MATERIAL_GROUP
#ifndef PREPASS_PIPELINE
#import bevy_pbr::pbr_fragment::pbr_input_from_standard_material
#endif
#endif

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::VertexOutput
#else
#import bevy_pbr::forward_io::VertexOutput
#endif

#ifndef SDF_STRIPPED_MATERIAL_GROUP
#import hana_diegetic::material_table::{
    compute_material_sampled_uv,
    INVALID_GPU_MATERIAL_SLOT,
    MaterialSlotValues,
}
#endif

#ifndef SDF_STRIPPED_MATERIAL_GROUP
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var<storage, read> material_table: array<MaterialSlotValues>;
#endif

#ifndef SDF_STRIPPED_MATERIAL_GROUP
#ifndef PREPASS_PIPELINE
fn collapsed_table_pbr_input(in: VertexOutput, is_front: bool) -> pbr_types::PbrInput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = vec4<f32>(0.0);
    return pbr_input;
}
#endif
#endif

fn stripped_material_group_pbr_input(in: VertexOutput) -> pbr_types::PbrInput {
    var pbr_input = pbr_types::pbr_input_new();
    pbr_input.material.base_color = vec4<f32>(0.0);
    pbr_input.world_position = in.world_position;
#ifdef PREPASS_PIPELINE
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    pbr_input.world_normal = in.world_normal;
    pbr_input.N = normalize(in.world_normal);
#endif
#else
    pbr_input.world_normal = in.world_normal;
    pbr_input.N = normalize(in.world_normal);
#endif
    return pbr_input;
}

#ifndef SDF_STRIPPED_MATERIAL_GROUP
fn apply_material_slot_values(
    pbr_input: ptr<function, pbr_types::PbrInput>,
    values: MaterialSlotValues,
) {
    let flags = (*pbr_input).material.flags;
    let sampled_base_color = (*pbr_input).material.base_color;
    let sampled_emissive = (*pbr_input).material.emissive;
    let sampled_metallic = (*pbr_input).material.metallic;
    let sampled_roughness = (*pbr_input).material.perceptual_roughness;

    (*pbr_input).material.base_color = values.base_color;
    if (flags & pbr_types::STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u {
        (*pbr_input).material.base_color *= sampled_base_color;
    }

    (*pbr_input).material.emissive = values.emissive;
    if (flags & pbr_types::STANDARD_MATERIAL_FLAGS_EMISSIVE_TEXTURE_BIT) != 0u {
        (*pbr_input).material.emissive = vec4<f32>(
            values.emissive.rgb * sampled_emissive.rgb,
            values.emissive.a,
        );
    }

    (*pbr_input).material.attenuation_color = values.attenuation_color;
    (*pbr_input).material.uv_transform = values.uv_transform;
    (*pbr_input).material.reflectance = values.reflectance;
    (*pbr_input).material.perceptual_roughness = values.roughness;
    (*pbr_input).material.metallic = values.metallic;
    if (flags & pbr_types::STANDARD_MATERIAL_FLAGS_METALLIC_ROUGHNESS_TEXTURE_BIT) != 0u {
        (*pbr_input).material.perceptual_roughness = values.roughness * sampled_roughness;
        (*pbr_input).material.metallic = values.metallic * sampled_metallic;
    }

    (*pbr_input).material.diffuse_transmission = values.diffuse_transmission;
    (*pbr_input).material.specular_transmission = values.specular_transmission;
    (*pbr_input).material.thickness = values.thickness;
    (*pbr_input).material.ior = values.ior;
    (*pbr_input).material.attenuation_distance = values.attenuation_distance;
    (*pbr_input).material.clearcoat = values.clearcoat;
    (*pbr_input).material.clearcoat_perceptual_roughness = values.clearcoat_perceptual_roughness;
    (*pbr_input).material.anisotropy_strength = values.anisotropy_strength;
    (*pbr_input).material.anisotropy_rotation = values.anisotropy_rotation;
}
#endif

fn pbr_input_from_material_table(
    in: VertexOutput,
    is_front: bool,
    role_present: bool,
    material_id: u32,
) -> pbr_types::PbrInput {
#ifdef SDF_STRIPPED_MATERIAL_GROUP
    return stripped_material_group_pbr_input(in);
#else
#ifdef PREPASS_PIPELINE
    // The depth/shadow prepass needs only base_color.a for the alpha cutout. A
    // depth-only VertexOutput carries no world_normal, so read the table row
    // directly instead of the lit `pbr_input_from_standard_material` path.
    var pbr_input = pbr_types::pbr_input_new();
    pbr_input.world_position = in.world_position;
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    pbr_input.world_normal = in.world_normal;
    pbr_input.N = normalize(in.world_normal);
#endif
    let table_has_row = role_present
        && material_id != INVALID_GPU_MATERIAL_SLOT
        && material_id < arrayLength(&material_table);
    if table_has_row {
        pbr_input.material.base_color = material_table[material_id].base_color;
    } else {
        pbr_input.material.base_color = vec4<f32>(0.0);
    }
    return pbr_input;
#else
    if !role_present {
        return collapsed_table_pbr_input(in, is_front);
    }
    if material_id == INVALID_GPU_MATERIAL_SLOT {
        return collapsed_table_pbr_input(in, is_front);
    }
    if material_id >= arrayLength(&material_table) {
        return collapsed_table_pbr_input(in, is_front);
    }

    let material_index = material_id;
    let values = material_table[material_index];
    var sampled_input = in;
#ifdef VERTEX_UVS_A
    sampled_input.uv = compute_material_sampled_uv(in.uv, values.uv_transform);
#endif
    var pbr_input = pbr_input_from_standard_material(sampled_input, is_front);
    apply_material_slot_values(&pbr_input, values);
    return pbr_input;
#endif
#endif
}
