//! Rendering constants for diegetic panels.

use bevy::asset::uuid_handle;
use bevy::prelude::Handle;
use bevy::prelude::Shader;
use bevy::prelude::Vec4;

// Draw order
/// Per-`DrawZIndexRank` spacing for Bevy's hardware depth-bias path.
///
/// Bevy packs this through `i32` into `DepthBiasState.constant`.
/// A batch material uses its `DrawZIndexRank` times this value, so the hardware
/// depth-bias integer changes once per authored z-index band.
pub(super) const LAYER_DEPTH_BIAS: f32 = 1.0;
/// Per-command OIT depth offset for coplanar fragment ordering.
///
/// Added to `position.z` in the fragment shader before `oit_draw`
/// stores the fragment. Pipeline `depth_bias` does NOT affect
/// `in.position.z`, so we apply this offset manually.
/// Reverse-Z: positive = closer to camera = composited in front.
///
/// Calibration: `hana_lagrange` syncs the perspective near plane to
/// `radius × 0.001`, so a fragment at the camera's focus distance has
/// `position.z = near / d ≈ 0.001`. At `1e-6`, one step spans ~17 quanta of the
/// 24-bit OIT depth packing, so adjacent draw-order indices stay distinct while
/// ordinary per-panel spans remain small relative to focus depth.
///
/// Panels much farther than the camera focus shrink `position.z`. The
/// `OIT_MIN_DEPTH` floor in `sdf_panel.wgsl` and `analytic_path.wgsl` keeps
/// those fragments storable; past that bound their coplanar ordering collapses
/// to OIT-list insertion order instead of going invisible.
pub(super) const OIT_DEPTH_STEP: f32 = 0.000_001;

// material defaults
/// Default metallic value for panel surfaces. Non-metallic (dielectric).
pub(super) const DEFAULT_METALLIC: f32 = 0.0;
/// Default reflectance for panel surfaces. Very low specular to avoid
/// washing out colors under bright lights.
pub(super) const DEFAULT_REFLECTANCE: f32 = 0.02;
/// Default roughness for panel surfaces. Matte paper-like appearance.
pub(super) const DEFAULT_ROUGHNESS: f32 = 0.95;

// sdf rendering
/// World-space padding added to each SDF quad mesh beyond the SDF boundary.
/// Gives the exterior anti-aliasing ramp room to render — without this, the
/// mesh edge coincides with the SDF boundary and the AA fade-out is clipped.
pub(super) const SDF_AA_PADDING: f32 = 0.001;
/// Internal-asset handle for the `sdf_stroke.wgsl` shader.
pub(super) const SDF_STROKE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("536f3741-5418-4d7a-a0b2-8cfb1d30e8a1");

// text rendering
/// Fixed panel-local Z for text and image meshes.
///
/// Layering is handled by `StandardMaterial::depth_bias`, `ClipDepthNudge`, or
/// `OitDepthOffset`, so panel-local geometry stays coplanar.
pub(super) const TEXT_Z_OFFSET: f32 = 0.0;
/// Default clip rect for unclipped text: effectively infinite panel-local
/// bounds so the shader clip test becomes a no-op.
pub(super) const UNCLIPPED_TEXT_CLIP_RECT: Vec4 = Vec4::new(-1e6, -1e6, 1e6, 1e6);
