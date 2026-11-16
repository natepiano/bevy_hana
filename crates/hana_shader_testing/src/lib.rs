//! Loads and validates shader variants for workspace contract tests.

mod calls;
mod directory;

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::path::PathBuf;

use bevy::prelude::App;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Shader;
use bevy::shader::ShaderCacheError;
use bevy::shader::ShaderCacheSource;
use bevy::shader::ShaderImport;
use bevy::shader::Source;
use bevy::shader::ValidateShader;
use bevy::tasks::block_on;
use bevy::tasks::futures_lite::StreamExt;
pub use calls::call_sites;
pub use calls::definition_enabled;
pub use directory::ShaderTestDirectory;
use naga::Module;
use naga::valid::Capabilities;
use naga::valid::ValidationFlags;
use naga::valid::Validator;

const SHADER_LOAD_MAX_UPDATES: usize = 1024;

/// Waits for shader libraries and their transitive imports before copying them into a cache.
///
/// # Errors
/// Returns an error if shader imports or loads remain incomplete.
pub fn wait_for_shaders(app: &mut App) -> Result<(), String> {
    // Assets::iter excludes pending loads. Enumerate the embedded source so even a library
    // with no loaded importer yet participates in the AssetServer readiness check.
    let embedded_paths = block_on(async {
        let server = app.world().resource::<AssetServer>();
        let source = server
            .get_source("embedded")
            .map_err(|error| error.to_string())?;
        let reader = source.reader();
        let mut directories = vec![PathBuf::new()];
        let mut paths = Vec::new();
        while let Some(directory) = directories.pop() {
            let mut entries = reader
                .read_directory(&directory)
                .await
                .map_err(|error| error.to_string())?;
            while let Some(path) = entries.next().await {
                if reader
                    .is_directory(&path)
                    .await
                    .map_err(|error| error.to_string())?
                {
                    directories.push(path);
                } else {
                    paths.push(format!(
                        "embedded://{}",
                        path.to_string_lossy().replace('\\', "/")
                    ));
                }
            }
        }
        Ok::<_, String>(paths)
    })?;
    let mut missing = BTreeSet::new();
    let mut pending = BTreeSet::new();
    for _ in 0..SHADER_LOAD_MAX_UPDATES {
        app.update();
        let shaders = app.world().resource::<Assets<Shader>>();
        let server = app.world().resource::<AssetServer>();
        let provided = shaders
            .iter()
            .map(|(_, shader)| &shader.import_path)
            .collect::<HashSet<_>>();
        missing.clear();
        pending.clear();
        for path in &embedded_paths {
            if let Some(handle) = server.get_handle::<Shader>(path.clone())
                && (!server.is_loaded_with_dependencies(handle.id())
                    || !shaders.contains(handle.id()))
            {
                pending.insert(format!("{path}: {:?}", server.get_load_states(handle.id())));
            }
        }
        for (id, shader) in shaders.iter() {
            // WESL scans module and item candidates, including inactive branches.
            // Only the cache can resolve the imports required by an actual variant.
            if !matches!(shader.source, Source::Wesl(_)) {
                for import in &shader.imports {
                    if !provided.contains(import) {
                        let module = match import {
                            ShaderImport::AssetPath(path) | ShaderImport::Custom(path) => path,
                        };
                        missing.insert(format!("{} needs {module}", shader.path));
                    }
                }
            }
            if server.is_managed(id) && !server.is_loaded_with_dependencies(id) {
                pending.insert(format!("{}: {:?}", shader.path, server.get_load_states(id)));
            }
            for dependency in &shader.file_dependencies {
                if !server.is_loaded_with_dependencies(dependency.id())
                    || !shaders.contains(dependency.id())
                {
                    pending.insert(format!("{} needs {dependency:?}", shader.path));
                }
            }
        }
        if missing.is_empty() && pending.is_empty() {
            return Ok(());
        }
    }
    Err(format!(
        "shaders not ready after {SHADER_LOAD_MAX_UPDATES} updates; missing imports: {missing:?}; pending shader loads: {pending:?}"
    ))
}

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
