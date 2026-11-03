use bevy::asset::uuid_handle;
use bevy::prelude::Handle;
use bevy::shader::Shader;

/// Coverage stage of every analytic-path pipeline. Loaded behind a stable
/// handle at plugin build, so the first frame's pipelines never wait on it.
pub(super) const ANALYTIC_PATH_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("cac363d0-8d00-4b97-9cab-439e4a383c68");

/// Vertex-pulling stage swapped in by `TextExtension::specialize` for
/// materials whose `vertex_pull` flag is set. Loaded behind a stable handle so
/// specialization can name it without an asset server.
pub(super) const ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("3f8a5c21-9d4b-4e6f-8a07-5b2c9e1d4a73");
