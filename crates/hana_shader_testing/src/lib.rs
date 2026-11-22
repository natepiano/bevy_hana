//! Loads and validates shader variants for workspace contract tests.

mod calls;
mod directory;
mod shader_wait;

use bevy::shader::ShaderCacheError;
use bevy::shader::ShaderCacheSource;
use bevy::shader::ValidateShader;
pub use calls::call_sites;
pub use calls::definition_enabled;
pub use directory::ShaderTestDirectory;
pub use directory::ShaderTestDirectoryError;
use naga::Module;
use naga::valid::Capabilities;
use naga::valid::ValidationFlags;
use naga::valid::Validator;
pub use shader_wait::ShaderWaitError;
pub use shader_wait::wait_for_shaders;

/// Keeps a validated composed module without creating a GPU shader module.
///
/// # Errors
/// Returns an error if the shader source cannot parse or validate.
pub fn load_module(
    source: ShaderCacheSource<'_>,
    _: &ValidateShader,
) -> Result<Module, ShaderCacheError> {
    let module = match source {
        ShaderCacheSource::Wgsl(text) => naga::front::wgsl::parse_str(&text)
            .map_err(|error| ShaderCacheError::CreateShaderModule(error.to_string()))?,
        ShaderCacheSource::SpirV(_) => {
            return Err(ShaderCacheError::CreateShaderModule("expected WGSL".into()));
        },
    };
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|error| ShaderCacheError::CreateShaderModule(error.to_string()))?;
    Ok(module)
}
