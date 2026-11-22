//! Checks whether shader libraries and their imports have loaded.

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::path::PathBuf;

use bevy::asset::io::AssetReaderError;
use bevy::asset::io::MissingAssetSourceError;
use bevy::prelude::App;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Shader;
use bevy::shader::ShaderImport;
use bevy::shader::Source;
use bevy::tasks::block_on;
use bevy::tasks::futures_lite::StreamExt;

/// An error that stopped the shader readiness check.
#[derive(Debug, thiserror::Error)]
pub enum ShaderWaitError {
    /// The embedded asset source is unavailable.
    #[error("{0}")]
    MissingSource(#[from] MissingAssetSourceError),
    /// The embedded asset reader could not inspect a path.
    #[error("{source}")]
    ReadSource {
        /// Path that the asset reader could not inspect.
        path:   PathBuf,
        /// Error from the asset reader.
        #[source]
        source: AssetReaderError,
    },
    /// Shader imports or loads remained incomplete after the last update.
    #[error(
        "shaders not ready after {SHADER_LOAD_MAX_UPDATES} updates; missing imports: {missing:?}; pending shader loads: {pending:?}"
    )]
    NotReady {
        /// Imports with no loaded shader.
        missing: BTreeSet<String>,
        /// Shader loads that the asset server did not finish.
        pending: BTreeSet<String>,
    },
}

const SHADER_LOAD_MAX_UPDATES: usize = 1024;

/// Waits for shader libraries and their transitive imports before copying them into a cache.
///
/// # Errors
/// Returns an error if the embedded asset source cannot be read or shaders remain incomplete.
pub fn wait_for_shaders(app: &mut App) -> Result<(), ShaderWaitError> {
    // Assets::iter excludes pending loads. Enumerate the embedded source so even a library
    // with no loaded importer yet participates in the AssetServer readiness check.
    let embedded_paths = block_on(async {
        let server = app.world().resource::<AssetServer>();
        let source = server.get_source("embedded")?;
        let reader = source.reader();
        let mut directories = vec![PathBuf::new()];
        let mut paths = Vec::new();
        while let Some(directory) = directories.pop() {
            let mut entries = reader.read_directory(&directory).await.map_err(|source| {
                ShaderWaitError::ReadSource {
                    path: directory,
                    source,
                }
            })?;
            while let Some(path) = entries.next().await {
                if reader.is_directory(&path).await.map_err(|source| {
                    ShaderWaitError::ReadSource {
                        path: path.clone(),
                        source,
                    }
                })? {
                    directories.push(path);
                } else {
                    paths.push(format!(
                        "embedded://{}",
                        path.to_string_lossy().replace('\\', "/")
                    ));
                }
            }
        }
        Ok::<_, ShaderWaitError>(paths)
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
    Err(ShaderWaitError::NotReady { missing, pending })
}
