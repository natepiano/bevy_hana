//! Crate-wide constants shared across multiple modules.

use bevy::asset::uuid_handle;
use bevy::prelude::Handle;
use bevy::shader::Shader;

// shader assets
pub(crate) const EMBEDDED_SDF_PANEL_BATCH_SHADER_PATH: &str =
    "embedded://hana_diegetic/shaders/sdf_panel.wgsl";

/// Stable handle to `material_table.wgsl`, registered as a synchronous shader
/// library so the `hana_diegetic::material_table` import composes deterministically
/// (matches how bevy registers shared type modules like `mesh_types`).
pub(crate) const MATERIAL_TABLE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("c2f5e8b3-4d70-4960-8b1e-9f3d6a0c2e57");

/// Stable handle to `sdf_material_table.wgsl`, registered as a synchronous shader
/// library so the `hana_diegetic::sdf_material_table` import composes
/// deterministically for the batched SDF fragment path.
pub(crate) const SDF_MATERIAL_TABLE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("a7d2c4e9-6b81-4f3a-9c5d-2e8f0b3a1d64");

// text measurement
/// Estimated character width as a fraction of font size for monospace approximation.
pub(crate) const MONOSPACE_WIDTH_RATIO: f32 = 0.6;

// timing
/// Conversion factor from seconds to milliseconds for timing diagnostics.
pub(crate) const MILLISECONDS_PER_SECOND: f32 = 1000.0;
