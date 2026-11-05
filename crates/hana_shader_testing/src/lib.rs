//! Checks that each shader variant links and validates and each contract matches. JSON records are
//! optional output.

mod calls;
mod directory;
mod schema;
mod types;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::ErrorKind;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use bevy::prelude::App;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Shader;
use bevy::shader::ShaderCacheError;
use bevy::shader::ShaderCacheSource;
use bevy::shader::ShaderDefVal;
use bevy::shader::ShaderImport;
use bevy::shader::Source;
use bevy::shader::ValidateShader;
use bevy::tasks::block_on;
use bevy::tasks::futures_lite::StreamExt;
pub use calls::call_sites;
pub use calls::definition_enabled;
pub use directory::RecordDirectory;
use naga::Module;
use naga::valid::Capabilities;
use naga::valid::ValidationFlags;
use naga::valid::Validator;
use schema::BindingGroup;
use schema::ResourceBindingContract;
use schema::ShaderContract;
use schema::ShaderDefinition;
use schema::ShaderEntryPoint;
use schema::ShaderEvidence;
use schema::ShaderRootRecord;
use schema::ShaderSourceDigest;
use schema::ShaderVariantRecord;
use schema::SharedShaderContract;

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

fn sha256(bytes: &[u8]) -> Result<String, String> {
    let mut process = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    process
        .stdin
        .take()
        .ok_or("hash input unavailable")?
        .write_all(bytes)
        .map_err(|error| error.to_string())?;
    let output = process
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("sha256sum failed".into());
    }
    String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| "hash absent".into())
}

struct LinkedVariant {
    contract: ShaderContract,
    text:     String,
}

/// Records distinct ordered descriptor definitions for one owning crate.
pub struct ShaderRecord {
    crate_name: String,
    sources:    Vec<String>,
    roots:      BTreeMap<String, BTreeMap<Vec<ShaderDefinition>, LinkedVariant>>,
}

fn contract(module: &Module, definitions: &[ShaderDefVal]) -> Result<ShaderContract, String> {
    let material_group = definitions.iter().find_map(|definition| match definition {
        ShaderDefVal::UInt(name, value) if name == "MATERIAL_BIND_GROUP" => Some(*value),
        ShaderDefVal::Int(name, value) if name == "MATERIAL_BIND_GROUP" => {
            u32::try_from(*value).ok()
        },
        _ => None,
    });
    let mut bindings = Vec::new();
    for (_, global) in module.global_variables.iter() {
        let Some(binding) = &global.binding else {
            continue;
        };
        let group = if material_group == Some(binding.group) {
            BindingGroup::Material
        } else {
            BindingGroup::Numeric(binding.group)
        };
        bindings.push(ResourceBindingContract {
            group,
            binding: binding.binding,
            resource_type: format!(
                "{}:{}",
                types::address_space(global.space)?,
                types::binding_type(module, global.ty)?
            ),
        });
    }
    bindings.sort();
    let mut entry_points = module
        .entry_points
        .iter()
        .map(|entry| ShaderEntryPoint {
            name:  entry.name.clone(),
            stage: types::stage_name(entry.stage).into(),
        })
        .collect::<Vec<_>>();
    entry_points.sort();
    Ok(ShaderContract {
        entry_points,
        bindings,
    })
}

impl ShaderRecord {
    /// Names product sources, including imported modules, without reading evidence.
    #[must_use]
    pub fn new(crate_name: &str, sources: &[&str]) -> Self {
        Self {
            crate_name: crate_name.into(),
            sources:    sources.iter().map(|source| (*source).into()).collect(),
            roots:      BTreeMap::new(),
        }
    }

    /// Extracts a contract only when output is requested; the cache already validates.
    ///
    /// # Errors
    /// Returns an error if the requested shader record cannot validate or serialize.
    pub fn record(
        &mut self,
        source_path: &str,
        definitions: &[ShaderDefVal],
        module: &Module,
    ) -> Result<(), String> {
        if std::env::var_os("HANA_SHADER_RECORD_DIR").is_none() {
            return Ok(());
        }
        self.record_linked(source_path, definitions, module)
    }

    /// Records a validated variant without an environment variable check.
    ///
    /// # Errors
    /// Returns an error if the shader contract cannot validate or serialize.
    pub fn record_linked(
        &mut self,
        source_path: &str,
        definitions: &[ShaderDefVal],
        module: &Module,
    ) -> Result<(), String> {
        let definitions_key = definitions
            .iter()
            .map(ShaderDefinition::from_shader)
            .collect::<Vec<_>>();
        let variants = self.roots.entry(source_path.into()).or_default();
        if variants.contains_key(&definitions_key) {
            return Ok(());
        }
        let body =
            contract(module, definitions).map_err(|error| format!("{source_path}: {error}"))?;
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(module)
            .map_err(|error| format!("{source_path}: {error}"))?;
        let text =
            naga::back::wgsl::write_string(module, &info, naga::back::wgsl::WriterFlags::empty())
                .map_err(|error| format!("{source_path}: {error}"))?;
        variants.insert(
            definitions_key,
            LinkedVariant {
                contract: body,
                text,
            },
        );
        Ok(())
    }

    /// Replaces this crate's evidence in the combined record when output is requested.
    ///
    /// # Errors
    /// Returns an error if record files cannot read, hash, or write.
    pub fn finish(self) -> Result<(), String> {
        let Some(directory) = std::env::var_os("HANA_SHADER_RECORD_DIR") else {
            return Ok(());
        };
        self.finish_in(Path::new(&directory))
    }

    /// Writes this crate's records to the specified directory.
    ///
    /// # Errors
    /// Returns an error if record files cannot read, hash, or write.
    pub fn finish_in(self, directory: &Path) -> Result<(), String> {
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let record_path = directory.join("variants.json");
        let mut evidence = match std::fs::read_to_string(&record_path) {
            Ok(text) => ShaderEvidence::read(&text)?,
            Err(error) if error.kind() == ErrorKind::NotFound => ShaderEvidence::default(),
            Err(error) => return Err(error.to_string()),
        };
        let replaced_paths = evidence
            .sources
            .iter()
            .filter(|source| source.owner == self.crate_name)
            .map(|source| source.path.clone())
            .chain(self.sources.iter().cloned())
            .collect::<BTreeSet<_>>();
        evidence
            .sources
            .retain(|source| source.owner != self.crate_name);
        evidence.roots.retain(|root| root.owner != self.crate_name);
        let digest_path = directory.join("linked.sha256");
        let mut digests = read_digests(&digest_path)?;
        digests.retain(|row| !replaced_paths.contains(&row.root));
        let target = directory.join(&self.crate_name);
        match std::fs::remove_dir_all(&target) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for path in &self.sources {
            let bytes = std::fs::read(workspace.join(path)).map_err(|error| error.to_string())?;
            evidence.sources.push(ShaderSourceDigest {
                owner:  self.crate_name.clone(),
                path:   path.clone(),
                sha256: sha256(&bytes)?,
            });
        }
        self.append_roots(&mut evidence, &mut digests, &target)?;
        evidence.sources.sort_by(|a, b| a.path.cmp(&b.path));
        evidence
            .roots
            .sort_by(|a, b| a.source_path.cmp(&b.source_path));
        digests.sort();
        std::fs::write(record_path, evidence.write()?).map_err(|error| error.to_string())?;
        let mut text = String::new();
        for row in digests {
            writeln!(
                text,
                "{} {} {} {}",
                row.root, row.variant, row.bytes, row.sha256
            )
            .map_err(|error| error.to_string())?;
        }
        std::fs::write(digest_path, text).map_err(|error| error.to_string())
    }

    fn append_roots(
        self,
        evidence: &mut ShaderEvidence,
        digests: &mut Vec<LinkedDigest>,
        target: &Path,
    ) -> Result<(), String> {
        let mut contracts = std::mem::take(&mut evidence.contracts)
            .into_iter()
            .map(|contract| (contract.id, contract.body))
            .collect::<BTreeMap<_, _>>();
        for (path, variants) in self.roots {
            let stem = Path::new(&path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or("root name absent")?;
            let output = target.join(stem);
            std::fs::create_dir_all(&output).map_err(|error| error.to_string())?;
            let mut root_variants = Vec::new();
            for (definitions, variant) in variants {
                let id = format!("v{}", sha256(&schema::definition_bytes(&definitions)?)?);
                let contract_id =
                    format!("c{}", sha256(&schema::contract_bytes(&variant.contract)?)?);
                contracts.insert(contract_id.clone(), variant.contract);
                digests.push(LinkedDigest {
                    root:    path.clone(),
                    variant: id.clone(),
                    bytes:   variant.text.len(),
                    sha256:  sha256(variant.text.as_bytes())?,
                });
                std::fs::write(output.join(format!("{id}.wgsl")), variant.text)
                    .map_err(|error| error.to_string())?;
                root_variants.push(ShaderVariantRecord {
                    id,
                    definitions,
                    contract_id,
                });
            }
            let source = evidence
                .sources
                .iter()
                .find(|source| source.path == path)
                .ok_or("root absent from inventory")?;
            evidence.roots.push(ShaderRootRecord {
                owner:         self.crate_name.clone(),
                source_path:   path,
                source_sha256: source.sha256.clone(),
                variants:      root_variants,
            });
        }
        let used_contracts = evidence
            .roots
            .iter()
            .flat_map(|root| &root.variants)
            .map(|variant| variant.contract_id.as_str())
            .collect::<BTreeSet<_>>();
        evidence.contracts = contracts
            .into_iter()
            .filter(|(id, _)| used_contracts.contains(id.as_str()))
            .map(|(id, body)| SharedShaderContract { id, body })
            .collect();
        Ok(())
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct LinkedDigest {
    root:    String,
    variant: String,
    bytes:   usize,
    sha256:  String,
}

fn read_digests(path: &Path) -> Result<Vec<LinkedDigest>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    text.lines()
        .map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let [root, variant, bytes, digest] = fields.as_slice() else {
                return Err("invalid linked digest row".into());
            };
            Ok(LinkedDigest {
                root:    (*root).into(),
                variant: (*variant).into(),
                bytes:   bytes.parse::<usize>().map_err(|error| error.to_string())?,
                sha256:  (*digest).into(),
            })
        })
        .collect()
}
