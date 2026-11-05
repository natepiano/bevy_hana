//! Typed evidence records; JSON values are confined to the file boundary.

use bevy::shader::ShaderDefVal;
use serde_json::Value;
use serde_json::json;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ShaderDefinition {
    Bool(String, bool),
    Int(String, i32),
    UInt(String, u32),
}

impl ShaderDefinition {
    pub(super) fn from_shader(definition: &ShaderDefVal) -> Self {
        match definition {
            ShaderDefVal::Bool(name, value) => Self::Bool(name.to_string(), *value),
            ShaderDefVal::Int(name, value) => Self::Int(name.to_string(), *value),
            ShaderDefVal::UInt(name, value) => Self::UInt(name.to_string(), *value),
        }
    }

    fn spelling(&self) -> String {
        match self {
            Self::Bool(name, true) => name.clone(),
            Self::Bool(name, false) => format!("{name}=false"),
            Self::Int(name, value) => format!("{name}={value}i"),
            Self::UInt(name, value) => format!("{name}={value}u"),
        }
    }

    fn parse(text: &str) -> Result<Self, String> {
        let Some((name, value)) = text.split_once('=') else {
            return Ok(Self::Bool(text.into(), true));
        };
        if value == "false" {
            Ok(Self::Bool(name.into(), false))
        } else if let Some(number) = value.strip_suffix('i') {
            Ok(Self::Int(
                name.into(),
                number.parse::<i32>().map_err(|error| error.to_string())?,
            ))
        } else if let Some(number) = value.strip_suffix('u') {
            Ok(Self::UInt(
                name.into(),
                number.parse::<u32>().map_err(|error| error.to_string())?,
            ))
        } else {
            Err(format!("invalid definition {text}"))
        }
    }
}

#[derive(Clone)]
pub(super) struct ShaderSourceDigest {
    pub(super) owner:  String,
    pub(super) path:   String,
    pub(super) sha256: String,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum BindingGroup {
    Numeric(u32),
    Material,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ResourceBindingContract {
    pub(super) group:         BindingGroup,
    pub(super) binding:       u32,
    pub(super) resource_type: String,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ShaderEntryPoint {
    pub(super) name:  String,
    pub(super) stage: String,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ShaderContract {
    pub(super) entry_points: Vec<ShaderEntryPoint>,
    pub(super) bindings:     Vec<ResourceBindingContract>,
}

pub(super) struct SharedShaderContract {
    pub(super) id:   String,
    pub(super) body: ShaderContract,
}

pub(super) struct ShaderVariantRecord {
    pub(super) id:          String,
    pub(super) definitions: Vec<ShaderDefinition>,
    pub(super) contract_id: String,
}

pub(super) struct ShaderRootRecord {
    pub(super) owner:         String,
    pub(super) source_path:   String,
    pub(super) source_sha256: String,
    pub(super) variants:      Vec<ShaderVariantRecord>,
}

#[derive(Default)]
pub(super) struct ShaderEvidence {
    pub(super) sources:   Vec<ShaderSourceDigest>,
    pub(super) roots:     Vec<ShaderRootRecord>,
    pub(super) contracts: Vec<SharedShaderContract>,
}

fn entry_json(entry: &ShaderEntryPoint) -> Value {
    json!({"name": entry.name, "stage": entry.stage})
}

fn binding_json(binding: &ResourceBindingContract) -> Value {
    let group = match binding.group {
        BindingGroup::Numeric(group) => json!(group),
        BindingGroup::Material => json!("MATERIAL_BIND_GROUP"),
    };
    json!({"binding": binding.binding, "group": group, "type": binding.resource_type})
}

fn contract_json(contract: &ShaderContract) -> Value {
    json!({"bindings": contract.bindings.iter().map(binding_json).collect::<Vec<_>>(),
        "entry_points": contract.entry_points.iter().map(entry_json).collect::<Vec<_>>()})
}

fn json_text(value: &Value) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn json_lines(values: &[Value], indent: &str) -> Result<String, String> {
    values
        .iter()
        .map(|value| Ok(format!("{indent}{}", json_text(value)?)))
        .collect::<Result<Vec<_>, String>>()
        .map(|lines| lines.join(",\n"))
}

fn root_text(root: &ShaderRootRecord) -> Result<String, String> {
    let metadata = json_text(
        &json!({"crate": root.owner, "source_path": root.source_path,
        "source_sha256": root.source_sha256}),
    )?;
    let metadata = metadata
        .strip_suffix('}')
        .ok_or("root must be a JSON object")?;
    let variants = root.variants.iter().map(|variant| {
        json!({"contract": variant.contract_id,
            "definitions": variant.definitions.iter().map(ShaderDefinition::spelling).collect::<Vec<_>>(),
            "id": variant.id})
    }).collect::<Vec<_>>();
    Ok(format!(
        "    {metadata},\"variants\":[\n{}\n    ]}}",
        json_lines(&variants, "      ")?
    ))
}

fn contract_text(contract: &SharedShaderContract) -> Result<String, String> {
    let entries = contract
        .body
        .entry_points
        .iter()
        .map(entry_json)
        .collect::<Vec<_>>();
    let bindings = contract
        .body
        .bindings
        .iter()
        .map(binding_json)
        .collect::<Vec<_>>();
    Ok(format!(
        "    {{\"id\":{},\"entry_points\":[\n{}\n    ],\"bindings\":[\n{}\n    ]}}",
        json_text(&json!(contract.id))?,
        json_lines(&entries, "      ")?,
        json_lines(&bindings, "      ")?
    ))
}

pub(super) fn contract_bytes(contract: &ShaderContract) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&contract_json(contract)).map_err(|error| error.to_string())
}

pub(super) fn definition_bytes(definitions: &[ShaderDefinition]) -> Result<Vec<u8>, String> {
    serde_json::to_vec(
        &definitions
            .iter()
            .map(ShaderDefinition::spelling)
            .collect::<Vec<_>>(),
    )
    .map_err(|error| error.to_string())
}

impl ShaderEvidence {
    pub(super) fn write(&self) -> Result<String, String> {
        let sources = self.sources.iter().map(|source| {
            json!({"crate": source.owner, "path": source.path, "sha256": source.sha256})
        }).collect::<Vec<_>>();
        let roots = self
            .roots
            .iter()
            .map(root_text)
            .collect::<Result<Vec<_>, _>>()?
            .join(",\n");
        let contracts = self
            .contracts
            .iter()
            .map(contract_text)
            .collect::<Result<Vec<_>, _>>()?
            .join(",\n");
        Ok(format!(
            "{{\n  \"schema_version\":2,\n  \"sources\":[\n{}\n  ],\n  \"roots\":[\n{roots}\n  ],\n  \"contracts\":[\n{contracts}\n  ]\n}}\n",
            json_lines(&sources, "    ")?
        ))
    }

    pub(super) fn read(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if value["schema_version"] != 2 {
            return Err("record requires schema_version 2".into());
        }
        let sources = list(&value, "sources")?
            .iter()
            .map(|source| {
                Ok(ShaderSourceDigest {
                    owner:  string(source, "crate")?,
                    path:   string(source, "path")?,
                    sha256: string(source, "sha256")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let roots = list(&value, "roots")?
            .iter()
            .map(read_root)
            .collect::<Result<Vec<_>, String>>()?;
        let contracts = list(&value, "contracts")?
            .iter()
            .map(read_contract)
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            sources,
            roots,
            contracts,
        })
    }
}

fn string(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("record field {key} must be text"))
}

fn list<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("record field {key} must be a list"))
}

fn read_root(value: &Value) -> Result<ShaderRootRecord, String> {
    let variants = list(value, "variants")?
        .iter()
        .map(|variant| {
            let definitions = list(variant, "definitions")?
                .iter()
                .map(|definition| {
                    ShaderDefinition::parse(definition.as_str().ok_or("definition must be text")?)
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(ShaderVariantRecord {
                id: string(variant, "id")?,
                definitions,
                contract_id: string(variant, "contract")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ShaderRootRecord {
        owner: string(value, "crate")?,
        source_path: string(value, "source_path")?,
        source_sha256: string(value, "source_sha256")?,
        variants,
    })
}

fn read_contract(value: &Value) -> Result<SharedShaderContract, String> {
    let entry_points = list(value, "entry_points")?
        .iter()
        .map(|entry| {
            Ok(ShaderEntryPoint {
                name:  string(entry, "name")?,
                stage: string(entry, "stage")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let bindings = list(value, "bindings")?
        .iter()
        .map(|binding| {
            let group = if binding["group"] == "MATERIAL_BIND_GROUP" {
                BindingGroup::Material
            } else {
                BindingGroup::Numeric(number(binding, "group")?)
            };
            Ok(ResourceBindingContract {
                group,
                binding: number(binding, "binding")?,
                resource_type: string(binding, "type")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(SharedShaderContract {
        id:   string(value, "id")?,
        body: ShaderContract {
            entry_points,
            bindings,
        },
    })
}

fn number(value: &Value, key: &str) -> Result<u32, String> {
    value[key]
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| format!("record field {key} must be a u32"))
}
