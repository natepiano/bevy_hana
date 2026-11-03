//! Rendering constants for diegetic panels.

use bevy::asset::uuid_handle;
use bevy::prelude::Handle;
use bevy::prelude::Shader;
use bevy::prelude::Vec4;
use bevy::render::render_resource::TextureFormat;

// display compensation
pub(super) const DISPLAY_TABLE_SIZE: u16 = 48;
pub(super) const DISPLAY_TABLE_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
pub(super) const TONY_LUT_SIZE: u32 = 48;

// Draw order
/// Per-`DrawZIndexRank` spacing for Bevy's hardware depth-bias path.
///
/// Bevy packs this through `i32` into `DepthBiasState.constant`.
/// A batch material uses its `DrawZIndexRank` times this value, so the hardware
/// depth-bias integer changes once per authored z-index band.
pub(super) const LAYER_DEPTH_BIAS: f32 = 1.0;
/// Range used to upload a normalized panel-local OIT paint rank.
/// The shader stores this rank separately from physical depth.
pub(super) const OIT_PACKED_PAINT_RANGE: f32 = 16_777_215.0;
/// Two local paint-rank units per stack layer. These units occupy the
/// secondary key only: they never displace or round the physical depth.
pub(super) const OIT_PAINT_STEP: f32 = 2.0 / OIT_PACKED_PAINT_RANGE;
/// Relative float rounding between the touching edges of two abutting command
/// bounds.
///
/// Scaled by the pair's largest edge coordinate, a shared extent at or below it
/// is rounding, not overlap, so abutting rows share one stack layer instead of
/// climbing a layer each.
pub(super) const ABUTTING_EDGE_ROUNDING: f32 = 4.0 * f32::EPSILON;

// material defaults
/// Default metallic value for panel surfaces. Non-metallic (dielectric).
pub(super) const DEFAULT_METALLIC: f32 = 0.0;
/// Default reflectance for panel surfaces. Very low specular to avoid
/// washing out colors under bright lights.
pub(super) const DEFAULT_REFLECTANCE: f32 = 0.02;
/// Default roughness for panel surfaces. Matte paper-like appearance.
pub(super) const DEFAULT_ROUGHNESS: f32 = 0.95;

// private view layers
/// First layer available for private views. Lower layers remain available for
/// authored panels and scene isolation.
pub(super) const FIRST_PRIVATE_VIEW_RENDER_LAYER: usize = 32;

// sdf rendering
/// World-space padding added to each SDF quad mesh beyond the SDF boundary.
/// Gives the exterior anti-aliasing ramp room to render — without this, the
/// mesh edge coincides with the SDF boundary and the AA fade-out is clipped.
pub(super) const SDF_AA_PADDING: f32 = 0.001;
/// Internal-asset handle for the `sdf_stroke.wesl` shader.
pub(super) const SDF_STROKE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("536f3741-5418-4d7a-a0b2-8cfb1d30e8a1");

// text rendering
/// Fixed panel-local Z for text and image meshes.
///
/// Layering is handled by `StandardMaterial::depth_bias`, `ClipDepthNudge`, or
/// `OitPaintOrder`, so panel-local geometry stays coplanar.
pub(super) const TEXT_Z_OFFSET: f32 = 0.0;
/// Default clip rect for unclipped text: effectively infinite panel-local
/// bounds so the shader clip test becomes a no-op.
pub(super) const UNCLIPPED_TEXT_CLIP_RECT: Vec4 = Vec4::new(-1e6, -1e6, 1e6, 1e6);
