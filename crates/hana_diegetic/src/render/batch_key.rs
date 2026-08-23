//! Shared visual batch-key pieces.
//!
//! Text and panel-line batching keep separate stores and source-lifecycle
//! payloads, but they split on the same resource and pipeline facts. Scalar
//! PBR values live in the frame material table, not in batch keys.

use std::fmt;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::hash::Hash;
use std::hash::Hasher;

use bevy::camera::visibility::RenderLayers;
use bevy::image::Image;
use bevy::material::OpaqueRendererMethod;
use bevy::mesh::UvChannel;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::prelude::Color;
use bevy::prelude::Handle;
use bevy::render::render_resource::Face;
use hana_kana::ToU32;
use hana_kana::ToUsize;

use crate::DrawZIndex;
use crate::layout::GlyphShadowMode;
use crate::layout::ShadowCasting;
use crate::panel::BatchSummary;
use crate::panel::SurfaceShadow;

/// `AlphaMode` re-encoded so it can sit in a hash-map key: `AlphaMode` has a
/// manual `Eq` but no `Hash` (`Mask` carries an `f32`), so `Mask` stores the
/// threshold's bits here.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BatchAlphaMode {
    Opaque,
    Mask(u32),
    Blend,
    Premultiplied,
    AlphaToCoverage,
    Add,
    Multiply,
}

/// `OpaqueRendererMethod` re-encoded for hashable pipeline compatibility.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BatchOpaqueRendererMethod {
    Forward,
    Deferred,
    Auto,
}

impl From<OpaqueRendererMethod> for BatchOpaqueRendererMethod {
    fn from(method: OpaqueRendererMethod) -> Self {
        match method {
            OpaqueRendererMethod::Forward => Self::Forward,
            OpaqueRendererMethod::Deferred => Self::Deferred,
            OpaqueRendererMethod::Auto => Self::Auto,
        }
    }
}

impl From<BatchOpaqueRendererMethod> for OpaqueRendererMethod {
    fn from(method: BatchOpaqueRendererMethod) -> Self {
        match method {
            BatchOpaqueRendererMethod::Forward => Self::Forward,
            BatchOpaqueRendererMethod::Deferred => Self::Deferred,
            BatchOpaqueRendererMethod::Auto => Self::Auto,
        }
    }
}

/// `UvChannel` re-encoded for hashable resource compatibility.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BatchUvChannel {
    Uv0,
    Uv1,
}

impl From<&UvChannel> for BatchUvChannel {
    fn from(channel: &UvChannel) -> Self {
        match channel {
            UvChannel::Uv0 => Self::Uv0,
            UvChannel::Uv1 => Self::Uv1,
        }
    }
}

impl From<BatchUvChannel> for UvChannel {
    fn from(channel: BatchUvChannel) -> Self {
        match channel {
            BatchUvChannel::Uv0 => Self::Uv0,
            BatchUvChannel::Uv1 => Self::Uv1,
        }
    }
}

impl From<AlphaMode> for BatchAlphaMode {
    fn from(mode: AlphaMode) -> Self {
        match mode {
            AlphaMode::Opaque => Self::Opaque,
            AlphaMode::Mask(threshold) => Self::Mask(threshold.to_bits()),
            AlphaMode::Blend => Self::Blend,
            AlphaMode::Premultiplied => Self::Premultiplied,
            AlphaMode::AlphaToCoverage => Self::AlphaToCoverage,
            AlphaMode::Add => Self::Add,
            AlphaMode::Multiply => Self::Multiply,
        }
    }
}

impl From<BatchAlphaMode> for AlphaMode {
    fn from(mode: BatchAlphaMode) -> Self {
        match mode {
            BatchAlphaMode::Opaque => Self::Opaque,
            BatchAlphaMode::Mask(bits) => Self::Mask(f32::from_bits(bits)),
            BatchAlphaMode::Blend => Self::Blend,
            BatchAlphaMode::Premultiplied => Self::Premultiplied,
            BatchAlphaMode::AlphaToCoverage => Self::AlphaToCoverage,
            BatchAlphaMode::Add => Self::Add,
            BatchAlphaMode::Multiply => Self::Multiply,
        }
    }
}

/// `RenderLayers` behind a hashable newtype: it derives `Eq` but not `Hash`.
/// Hashing the iterated layer indices is consistent with the derived
/// `PartialEq` because `RenderLayers` trims trailing empty blocks.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct BatchRenderLayers(pub RenderLayers);

impl Hash for BatchRenderLayers {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for layer in self.0.iter() {
            layer.hash(state);
        }
    }
}

impl Debug for BatchRenderLayers {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.iter()).finish()
    }
}

/// Shadow participation shared by visual batch keys.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum VisualShadow {
    Cast,
    None,
}

impl From<GlyphShadowMode> for VisualShadow {
    fn from(shadow: GlyphShadowMode) -> Self {
        match shadow {
            GlyphShadowMode::Cast => Self::Cast,
            GlyphShadowMode::None => Self::None,
        }
    }
}

impl From<ShadowCasting> for VisualShadow {
    fn from(shadow: ShadowCasting) -> Self {
        match shadow {
            ShadowCasting::On => Self::Cast,
            ShadowCasting::Off => Self::None,
        }
    }
}

impl From<SurfaceShadow> for VisualShadow {
    fn from(shadow: SurfaceShadow) -> Self {
        match shadow {
            SurfaceShadow::On => Self::Cast,
            SurfaceShadow::Off => Self::None,
        }
    }
}

/// Pipeline-specialization facts that split material-table batches.
///
/// These fields are copied from the resolved `StandardMaterial` because changing
/// them can alter Bevy PBR shader definitions, pass routing, culling, lighting,
/// or alpha behavior. Scalar/vector PBR values are stored in
/// `MaterialSlotValues`, not in this key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PipelineCompatibility {
    /// `StandardMaterial::alpha_mode` as a hashable alpha pipeline selector.
    pub alpha:                     BatchAlphaMode,
    /// `StandardMaterial::double_sided` controls the PBR normal-flip shader flag.
    pub double_sided:              bool,
    /// `StandardMaterial::cull_mode` selects front, back, or no face culling.
    pub cull_mode:                 Option<Face>,
    /// `StandardMaterial::unlit` selects the lit or unlit PBR shader path.
    pub unlit:                     bool,
    /// `StandardMaterial::fog_enabled` selects the fog-aware shader path.
    pub fog_enabled:               bool,
    /// `StandardMaterial::opaque_render_method` selects forward or deferred opaque routing.
    pub opaque_render_method:      BatchOpaqueRendererMethod,
    /// `StandardMaterial::deferred_lighting_pass_id` selects the deferred lighting pass.
    pub deferred_lighting_pass_id: u8,
}

impl From<&StandardMaterial> for PipelineCompatibility {
    fn from(material: &StandardMaterial) -> Self {
        // Field-approval gate: every StandardMaterial field must be classified
        // here when Bevy changes. Decide one of: table, pipeline, resource,
        // draw-order, unsupported, deferred. Do not add `..`; E0027 is the
        // intended reminder that a new field needs an explicit decision.
        let StandardMaterial {
            base_color: _base_color_table,
            base_color_channel: _base_color_channel_resource,
            base_color_texture: _base_color_texture_resource,
            emissive: _emissive_table,
            emissive_exposure_weight: _emissive_exposure_table,
            emissive_channel: _emissive_channel_resource,
            emissive_texture: _emissive_texture_resource,
            perceptual_roughness: _roughness_table,
            metallic: _metallic_table,
            metallic_roughness_channel: _metallic_roughness_channel_resource,
            metallic_roughness_texture: _metallic_roughness_texture_resource,
            reflectance: _reflectance_table,
            specular_tint: _specular_tint_table,
            diffuse_transmission: _diffuse_transmission_table,
            specular_transmission: _specular_transmission_table,
            thickness: _thickness_table,
            ior: _ior_table,
            attenuation_distance: _attenuation_distance_table,
            attenuation_color: _attenuation_color_table,
            normal_map_channel: _normal_map_channel_resource,
            normal_map_texture: _normal_map_texture_resource,
            flip_normal_map_y: _flip_normal_map_y_resource,
            occlusion_channel: _occlusion_channel_resource,
            occlusion_texture: _occlusion_texture_resource,
            clearcoat: _clearcoat_table,
            clearcoat_perceptual_roughness: _clearcoat_roughness_table,
            anisotropy_strength: _anisotropy_strength_table,
            anisotropy_rotation: _anisotropy_rotation_table,
            double_sided,
            cull_mode,
            unlit,
            fog_enabled,
            alpha_mode,
            depth_bias: _depth_bias_draw_order,
            depth_map: _depth_map_resource,
            parallax_depth_scale: _parallax_depth_scale_deferred,
            parallax_mapping_method: _parallax_mapping_method_deferred,
            max_parallax_layer_count: _max_parallax_layer_count_deferred,
            lightmap_exposure: _lightmap_exposure_deferred,
            opaque_render_method,
            deferred_lighting_pass_id,
            uv_transform: _uv_transform_table,
        } = material;
        Self {
            alpha:                     (*alpha_mode).into(),
            double_sided:              *double_sided,
            cull_mode:                 *cull_mode,
            unlit:                     *unlit,
            fog_enabled:               *fog_enabled,
            opaque_render_method:      (*opaque_render_method).into(),
            deferred_lighting_pass_id: *deferred_lighting_pass_id,
        }
    }
}

/// Texture and UV-channel facts that split material-table batches.
///
/// The frame material table stores scalar/vector values only. These fields
/// identify texture resources, UV-channel requirements, and shader-definition
/// requirements that must be copied into the `StandardMaterial` half of a batch
/// render material.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResourceCompatibility {
    /// `StandardMaterial::base_color_texture` bound for base-color sampling.
    pub base_color_texture:         Option<Handle<Image>>,
    /// `StandardMaterial::base_color_channel` selects the mesh UV channel for base color.
    pub base_color_channel:         BatchUvChannel,
    /// `StandardMaterial::emissive_texture` bound for emissive sampling.
    pub emissive_texture:           Option<Handle<Image>>,
    /// `StandardMaterial::emissive_channel` selects the mesh UV channel for emissive.
    pub emissive_channel:           BatchUvChannel,
    /// `StandardMaterial::metallic_roughness_texture` bound for metallic/roughness sampling.
    pub metallic_roughness_texture: Option<Handle<Image>>,
    /// `StandardMaterial::metallic_roughness_channel` selects the mesh UV channel for
    /// metallic/roughness.
    pub metallic_roughness_channel: BatchUvChannel,
    /// `StandardMaterial::normal_map_texture` requires normals, tangents, and normal sampling.
    pub normal_map_texture:         Option<Handle<Image>>,
    /// `StandardMaterial::normal_map_channel` selects the mesh UV channel for normals.
    pub normal_map_channel:         BatchUvChannel,
    /// `StandardMaterial::flip_normal_map_y` changes normal-map decoding.
    pub flip_normal_map_y:          bool,
    /// `StandardMaterial::occlusion_texture` bound for ambient-occlusion sampling.
    pub occlusion_texture:          Option<Handle<Image>>,
    /// `StandardMaterial::occlusion_channel` selects the mesh UV channel for occlusion.
    pub occlusion_channel:          BatchUvChannel,
    /// `StandardMaterial::depth_map` enables parallax/depth-map shader requirements.
    pub depth_map:                  Option<Handle<Image>>,
}

impl From<&StandardMaterial> for ResourceCompatibility {
    fn from(material: &StandardMaterial) -> Self {
        // Field-approval gate: every StandardMaterial field must be classified
        // here when Bevy changes. Decide one of: table, pipeline, resource,
        // draw-order, unsupported, deferred. Do not add `..`; E0027 is the
        // intended reminder that a new field needs an explicit decision.
        let StandardMaterial {
            base_color: _base_color_table,
            base_color_channel,
            base_color_texture,
            emissive: _emissive_table,
            emissive_exposure_weight: _emissive_exposure_table,
            emissive_channel,
            emissive_texture,
            perceptual_roughness: _roughness_table,
            metallic: _metallic_table,
            metallic_roughness_channel,
            metallic_roughness_texture,
            reflectance: _reflectance_table,
            specular_tint: _specular_tint_table,
            diffuse_transmission: _diffuse_transmission_table,
            specular_transmission: _specular_transmission_table,
            thickness: _thickness_table,
            ior: _ior_table,
            attenuation_distance: _attenuation_distance_table,
            attenuation_color: _attenuation_color_table,
            normal_map_channel,
            normal_map_texture,
            flip_normal_map_y,
            occlusion_channel,
            occlusion_texture,
            clearcoat: _clearcoat_table,
            clearcoat_perceptual_roughness: _clearcoat_roughness_table,
            anisotropy_strength: _anisotropy_strength_table,
            anisotropy_rotation: _anisotropy_rotation_table,
            double_sided: _double_sided_pipeline,
            cull_mode: _cull_mode_pipeline,
            unlit: _unlit_pipeline,
            fog_enabled: _fog_pipeline,
            alpha_mode: _alpha_pipeline,
            depth_bias: _depth_bias_draw_order,
            depth_map,
            parallax_depth_scale: _parallax_depth_scale_deferred,
            parallax_mapping_method: _parallax_mapping_method_deferred,
            max_parallax_layer_count: _max_parallax_layer_count_deferred,
            lightmap_exposure: _lightmap_exposure_deferred,
            opaque_render_method: _opaque_render_method_pipeline,
            deferred_lighting_pass_id: _deferred_pass_pipeline,
            uv_transform: _uv_transform_table,
        } = material;
        Self {
            base_color_texture:         base_color_texture.clone(),
            base_color_channel:         BatchUvChannel::from(base_color_channel),
            emissive_texture:           emissive_texture.clone(),
            emissive_channel:           BatchUvChannel::from(emissive_channel),
            metallic_roughness_texture: metallic_roughness_texture.clone(),
            metallic_roughness_channel: BatchUvChannel::from(metallic_roughness_channel),
            normal_map_texture:         normal_map_texture.clone(),
            normal_map_channel:         BatchUvChannel::from(normal_map_channel),
            flip_normal_map_y:          *flip_normal_map_y,
            occlusion_texture:          occlusion_texture.clone(),
            occlusion_channel:          BatchUvChannel::from(occlusion_channel),
            depth_map:                  depth_map.clone(),
        }
    }
}

/// Builds the public per-batch summary shown by the batch-validation diagnostic.
///
/// Decodes the batch-key discriminants a viewer needs to see why a family split
/// into more than one draw — render layer, lit/unlit path, alpha mode, and
/// base-color texture binding — into [`BatchSummary`]. Every visual batch store
/// (text, panel-line, SDF) shares this so the breakdowns stay uniform.
///
/// The `DiegeticPerfStats` push that consumes each summary stays per-family:
/// `ImageBatchKey` carries no [`PipelineCompatibility`]/[`ResourceCompatibility`],
/// so `image_batch_summary` fills [`BatchSummary`] from the image key directly;
/// the text store lives inside `GlyphCache` rather than its own `Resource`; and
/// each push sits inside its family's upload loop next to family-only work
/// (SDF also fills `SdfRecordDiagnostics` in the same iteration).
#[must_use]
pub(crate) fn batch_summary(
    z_index: impl Into<DrawZIndex>,
    layers: &BatchRenderLayers,
    shadow: VisualShadow,
    pipeline: &PipelineCompatibility,
    resource: &ResourceCompatibility,
    record_count: u32,
) -> BatchSummary {
    let z_index = z_index.into();
    BatchSummary {
        z_index,
        render_layers: layers.0.iter().map(usize::to_u32).collect(),
        casts_shadow: matches!(shadow, VisualShadow::Cast),
        unlit: pipeline.unlit,
        alpha_mode: format!("{:?}", pipeline.alpha),
        textured: resource.base_color_texture.is_some(),
        record_count: record_count.to_usize().into(),
    }
}

/// Applies texture-resource compatibility to a cloned batch base material.
///
/// Non-texture pipeline facts stay in [`PipelineCompatibility`]. This helper is
/// the only direction where texture handles move from a material-table batch key
/// into the `StandardMaterial` half of an extended render material.
#[must_use]
pub(super) fn apply_resource_compatibility_to_standard_material(
    base: &StandardMaterial,
    compatibility: &ResourceCompatibility,
) -> StandardMaterial {
    let mut material = base.clone();
    material
        .base_color_texture
        .clone_from(&compatibility.base_color_texture);
    material.base_color_channel = UvChannel::Uv0;
    if compatibility.base_color_texture.is_some() {
        material.base_color = Color::WHITE;
    }
    material
        .emissive_texture
        .clone_from(&compatibility.emissive_texture);
    material.emissive_channel = UvChannel::Uv0;
    if compatibility.emissive_texture.is_some() {
        material.emissive = Color::WHITE.into();
    }
    material
        .metallic_roughness_texture
        .clone_from(&compatibility.metallic_roughness_texture);
    material.metallic_roughness_channel = UvChannel::Uv0;
    if compatibility.metallic_roughness_texture.is_some() {
        material.metallic = 1.0;
        material.perceptual_roughness = 1.0;
    }
    material
        .normal_map_texture
        .clone_from(&compatibility.normal_map_texture);
    material.normal_map_channel = UvChannel::Uv0;
    material.flip_normal_map_y = compatibility.flip_normal_map_y;
    material
        .occlusion_texture
        .clone_from(&compatibility.occlusion_texture);
    material.occlusion_channel = UvChannel::Uv0;
    material.depth_map.clone_from(&compatibility.depth_map);
    material
}

/// Applies material-derived pipeline compatibility to a batch render material.
///
/// Scalar/vector fields stay in `MaterialSlotValues`; this helper copies only
/// `PipelineCompatibility` fields that Bevy's PBR pipeline observes while
/// specializing, culling, lighting, or selecting pass routes.
pub(super) fn apply_pipeline_compatibility_to_standard_material(
    material: &mut StandardMaterial,
    compatibility: PipelineCompatibility,
) {
    material.alpha_mode = compatibility.alpha.into();
    material.double_sided = compatibility.double_sided;
    material.cull_mode = compatibility.cull_mode;
    material.unlit = compatibility.unlit;
    material.fog_enabled = compatibility.fog_enabled;
    material.opaque_render_method = compatibility.opaque_render_method.into();
    material.deferred_lighting_pass_id = compatibility.deferred_lighting_pass_id;
}

#[cfg(test)]
mod tests {
    use bevy::mesh::UvChannel;
    use bevy::prelude::Color;

    use super::*;

    #[test]
    fn scalar_material_changes_do_not_change_resource_compatibility() {
        let first = StandardMaterial {
            base_color: Color::srgb(0.2, 0.3, 0.4),
            metallic: 0.1,
            reflectance: 0.8,
            ..Default::default()
        };
        let second = StandardMaterial {
            base_color: Color::srgb(0.8, 0.3, 0.1),
            metallic: 0.7,
            reflectance: 0.2,
            ..Default::default()
        };

        assert_eq!(
            ResourceCompatibility::from(&first),
            ResourceCompatibility::from(&second)
        );
    }

    #[test]
    fn apply_resource_compatibility_copies_textures_and_routes_box_uv() {
        let base = StandardMaterial {
            base_color: Color::srgb(0.1, 0.2, 0.3),
            emissive: Color::srgb(0.4, 0.5, 0.6).into(),
            metallic: 0.25,
            perceptual_roughness: 0.5,
            ..Default::default()
        };
        let source = StandardMaterial {
            base_color_texture: Some(Handle::default()),
            base_color_channel: UvChannel::Uv1,
            emissive_texture: Some(Handle::default()),
            emissive_channel: UvChannel::Uv1,
            metallic_roughness_texture: Some(Handle::default()),
            metallic_roughness_channel: UvChannel::Uv1,
            normal_map_texture: Some(Handle::default()),
            normal_map_channel: UvChannel::Uv1,
            flip_normal_map_y: true,
            occlusion_texture: Some(Handle::default()),
            occlusion_channel: UvChannel::Uv1,
            depth_map: Some(Handle::default()),
            ..Default::default()
        };
        let compatibility = ResourceCompatibility::from(&source);

        let patched = apply_resource_compatibility_to_standard_material(&base, &compatibility);

        assert_eq!(patched.base_color_texture, compatibility.base_color_texture);
        assert_eq!(patched.emissive_texture, compatibility.emissive_texture);
        assert_eq!(
            patched.metallic_roughness_texture,
            compatibility.metallic_roughness_texture
        );
        assert_eq!(patched.normal_map_texture, compatibility.normal_map_texture);
        assert_eq!(patched.occlusion_texture, compatibility.occlusion_texture);
        assert_eq!(patched.depth_map, compatibility.depth_map);

        assert_eq!(patched.base_color_channel, UvChannel::Uv0);
        assert_eq!(patched.emissive_channel, UvChannel::Uv0);
        assert_eq!(patched.metallic_roughness_channel, UvChannel::Uv0);
        assert_eq!(patched.normal_map_channel, UvChannel::Uv0);
        assert_eq!(patched.occlusion_channel, UvChannel::Uv0);
        assert_eq!(patched.base_color, Color::WHITE);
        assert_eq!(patched.emissive, Color::WHITE.into());
        assert_eq!(patched.metallic.to_bits(), 1.0_f32.to_bits());
        assert_eq!(patched.perceptual_roughness.to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn resource_compatibility_excludes_scalar_values() {
        let first = StandardMaterial {
            base_color: Color::srgb(0.1, 0.2, 0.3),
            emissive: Color::srgb(0.4, 0.5, 0.6).into(),
            metallic: 0.1,
            perceptual_roughness: 0.2,
            reflectance: 0.3,
            ..Default::default()
        };
        let second = StandardMaterial {
            base_color: Color::srgb(0.8, 0.7, 0.6),
            emissive: Color::srgb(0.6, 0.5, 0.4).into(),
            metallic: 0.9,
            perceptual_roughness: 0.8,
            reflectance: 0.7,
            ..Default::default()
        };

        assert_eq!(
            ResourceCompatibility::from(&first),
            ResourceCompatibility::from(&second)
        );
    }
}
