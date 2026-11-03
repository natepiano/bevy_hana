//! Crate-wide constants shared across multiple modules.

use bevy::asset::uuid_handle;
use bevy::prelude::Handle;
use bevy::shader::Shader;

/// Tracing target for developer diagnostics, never for an app's user-facing log.
pub const DIAGNOSTIC_LOG_TARGET: &str = "hana_diegetic::diagnostic";

// shader assets
/// Stable handle to `sdf_panel.wesl`, every panel fill pass's shader. Loaded
/// synchronously at plugin build, so the first frame's panel pipelines never
/// wait on it.
pub(crate) const SDF_PANEL_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("0a3fda2b-511a-4401-b923-01d72345237b");

/// Stable handle to `image_panel.wesl`, every image panel pass's shader.
/// Loaded synchronously at plugin build, like [`SDF_PANEL_SHADER_HANDLE`].
pub(crate) const IMAGE_PANEL_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("2c0cae47-cd86-4651-a6d7-e9d925bd9b1e");

/// Stable handle to `material_table.wesl`, registered as a synchronous shader
/// library so the `hana_diegetic::material_table` import composes deterministically
/// (matches how bevy registers shared type modules like `mesh_types`).
pub(crate) const MATERIAL_TABLE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("c2f5e8b3-4d70-4960-8b1e-9f3d6a0c2e57");

/// Stable handle to `sdf_material_table.wesl`, registered as a synchronous shader
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

/// World tooltip vertical inset in logical pixels; keeps compact help beside a jack.
pub(crate) const TOOLTIP_WORLD_VERTICAL_PADDING_PIXELS: f32 = 2.0;
