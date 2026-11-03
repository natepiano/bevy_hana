//! Linked contracts checked by the single liminal descriptor recorder.

#[path = "../../../hana_effects/tests/shader_record/calls.rs"]
mod shader_calls;

use std::collections::BTreeMap;
use std::collections::HashSet;

use bevy::shader::ShaderDefVal;
use naga::AddressSpace;
use naga::ArraySize;
use naga::Expression;
use naga::Module;
use naga::TypeInner;
use shader_calls::call_sites;
use shader_calls::definition_enabled;

use super::SOURCES;
use crate::constants::HULL_MESH_BIND_GROUP_SLOT;
use crate::constants::HULL_OUTLINE_BIND_GROUP_SLOT;
use crate::constants::OUTLINE_INSTANCE_BATCH_SIZE_SHADER_DEF;
use crate::constants::OUTLINE_UNIFORM_BIND_GROUP_ENTRY_BINDING;

// Bevy mesh_bindings declares its mesh uniform at binding zero.
const MESH_UNIFORM_BINDING: u32 = 0;

#[derive(Default)]
pub(super) struct OutlineContracts {
    roots:       HashSet<&'static str>,
    morph_roots: HashSet<&'static str>,
}

impl OutlineContracts {
    pub(super) fn check(
        &mut self,
        path: &'static str,
        module: &Module,
        definitions: &[ShaderDefVal],
    ) -> Result<(), String> {
        if path.ends_with("/hull.wesl") || path.ends_with("/mask.wesl") {
            require_outline_batch(module, definitions)?;
            if definition_enabled(definitions, "MORPH_TARGETS")
                && !definition_enabled(definitions, "SKINS_USE_UNIFORM_BUFFERS")
            {
                require_morph_descriptor_arguments(module, definitions)?;
                self.morph_roots.insert(path);
            }
        }
        self.roots.insert(path);
        Ok(())
    }

    pub(super) fn finish(&self) {
        for path in &SOURCES[..4] {
            assert!(self.roots.contains(path), "product root not linked: {path}");
        }
        for path in &SOURCES[2..4] {
            assert!(
                self.morph_roots.contains(path),
                "product root lacks storage morph variants: {path}"
            );
        }
    }
}

fn require_outline_batch(module: &Module, definitions: &[ShaderDefVal]) -> Result<(), String> {
    let expected = definitions.iter().find_map(|definition| match definition {
        ShaderDefVal::UInt(name, value) if name == OUTLINE_INSTANCE_BATCH_SIZE_SHADER_DEF => {
            Some(*value)
        },
        _ => None,
    });
    let variable = module
        .global_variables
        .iter()
        .find_map(|(_, variable)| {
            variable
                .binding
                .as_ref()
                .filter(|binding| {
                    usize::try_from(binding.group).ok() == Some(HULL_OUTLINE_BIND_GROUP_SLOT)
                        && binding.binding == OUTLINE_UNIFORM_BIND_GROUP_ENTRY_BINDING
                })
                .map(|_| variable)
        })
        .ok_or("outline instances binding 3:0 is absent")?;
    match (expected, &module.types[variable.ty].inner, variable.space) {
        (
            Some(expected),
            TypeInner::Array {
                size: ArraySize::Constant(actual),
                ..
            },
            AddressSpace::Uniform,
        ) if actual.get() == expected => Ok(()),
        (
            None,
            TypeInner::Array {
                size: ArraySize::Dynamic,
                ..
            },
            AddressSpace::Storage { .. },
        ) => Ok(()),
        _ => Err(format!(
            "outline instances batch mismatch: expected {expected:?}, actual {:?}, address {:?}",
            module.types[variable.ty].inner, variable.space
        )),
    }
}

fn require_morph_descriptor_arguments(
    module: &Module,
    definitions: &[ShaderDefVal],
) -> Result<(), String> {
    let caller = module
        .functions
        .iter()
        .find_map(|(_, function)| {
            (function.name.as_deref() == Some("morph_vertex")).then_some(function)
        })
        .ok_or("morph_vertex is absent")?;
    let (mesh, variable) = module
        .global_variables
        .iter()
        .find(|(_, variable)| {
            variable.binding.as_ref().is_some_and(|binding| {
                usize::try_from(binding.group).ok() == Some(HULL_MESH_BIND_GROUP_SLOT)
                    && binding.binding == MESH_UNIFORM_BINDING
            })
        })
        .ok_or("mesh binding 2:0 is absent")?;
    let TypeInner::Array { base, .. } = module.types[variable.ty].inner else {
        return Err("mesh binding is not an array".into());
    };
    let TypeInner::Struct { ref members, .. } = module.types[base].inner else {
        return Err("mesh element is not a struct".into());
    };
    let member = members
        .iter()
        .position(|member| member.name.as_deref() == Some("morph_descriptor_index"))
        .ok_or("mesh element lacks morph_descriptor_index")?;
    let mut calls = BTreeMap::new();
    for (function, arguments) in call_sites(caller) {
        let linked = module.functions[function]
            .name
            .as_deref()
            .ok_or("unnamed morph call")?;
        for (name, argument_index) in [
            ("layer_count", 0),
            ("weight_at", 1),
            ("morph_position", 2),
            ("morph_normal", 2),
            ("morph_tangent", 2),
        ] {
            let suffix = format!("morph__1{name}");
            if !linked.ends_with(&suffix) {
                continue;
            }
            let argument = *arguments
                .get(argument_index)
                .ok_or("morph call lacks descriptor argument")?;
            let derived = match caller.expressions[argument] {
                Expression::Load { pointer } => match caller.expressions[pointer] {
                    Expression::AccessIndex { base, index }
                        if usize::try_from(index).ok() == Some(member) =>
                    {
                        match caller.expressions[base] {
                            Expression::Access { base, index } => {
                                matches!(caller.expressions[base], naga::Expression::GlobalVariable(global) if global == mesh)
                                    && matches!(
                                        caller.expressions[index],
                                        naga::Expression::FunctionArgument(1)
                                    )
                            },
                            _ => false,
                        }
                    },
                    _ => false,
                },
                _ => false,
            };
            if !derived {
                return Err(format!(
                    "morph call {name} argument {argument_index} is not a load of mesh[instance_index].morph_descriptor_index: {:?}",
                    caller.expressions[argument]
                ));
            }
            *calls.entry(name).or_insert(0) += 1;
        }
    }
    for (name, flag) in [
        ("layer_count", "MORPH_TARGETS"),
        ("weight_at", "MORPH_TARGETS"),
        ("morph_position", "MORPH_TARGETS"),
        ("morph_normal", "VERTEX_NORMALS"),
        ("morph_tangent", "VERTEX_TANGENTS"),
    ] {
        let expected = usize::from(definition_enabled(definitions, flag));
        let actual = calls.get(name).copied().unwrap_or(0);
        if actual != expected {
            return Err(format!(
                "morph call {name}: expected {expected} descriptor loads, actual {actual}"
            ));
        }
    }
    Ok(())
}
