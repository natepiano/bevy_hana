//! Inspects call sites in nested Naga control flow.

use bevy::shader::ShaderDefVal;
use naga::Expression;
use naga::Function;
use naga::Handle;
use naga::Statement;

/// Returns function calls from all nested blocks.
#[must_use]
pub fn call_sites(function: &Function) -> Vec<(Handle<Function>, &[Handle<Expression>])> {
    let mut calls = Vec::new();
    let mut blocks = vec![&function.body];
    while let Some(block) = blocks.pop() {
        for statement in block {
            match statement {
                Statement::Block(block) => blocks.push(block),
                Statement::If { accept, reject, .. } => blocks.extend([accept, reject]),
                Statement::Loop {
                    body, continuing, ..
                } => blocks.extend([body, continuing]),
                Statement::Switch { cases, .. } => {
                    blocks.extend(cases.iter().map(|case| &case.body));
                },
                Statement::Call {
                    function,
                    arguments,
                    ..
                } => calls.push((*function, arguments.as_slice())),
                _ => {},
            }
        }
    }
    calls
}

/// Returns whether the definitions enable the Boolean flag.
#[must_use]
pub fn definition_enabled(definitions: &[ShaderDefVal], flag: &str) -> bool {
    definitions.iter().any(|definition| {
        matches!(definition,
        bevy::shader::ShaderDefVal::Bool(name, true) if name == flag)
    })
}
