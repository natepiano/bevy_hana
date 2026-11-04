//! Inspects call sites in nested Naga control flow.

pub(super) fn call_sites(
    function: &naga::Function,
) -> Vec<(
    naga::Handle<naga::Function>,
    &[naga::Handle<naga::Expression>],
)> {
    let mut calls = Vec::new();
    let mut blocks = vec![&function.body];
    while let Some(block) = blocks.pop() {
        for statement in block {
            match statement {
                naga::Statement::Block(block) => blocks.push(block),
                naga::Statement::If { accept, reject, .. } => blocks.extend([accept, reject]),
                naga::Statement::Loop {
                    body, continuing, ..
                } => blocks.extend([body, continuing]),
                naga::Statement::Switch { cases, .. } => {
                    blocks.extend(cases.iter().map(|case| &case.body));
                },
                naga::Statement::Call {
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

pub(super) fn definition_enabled(definitions: &[bevy::shader::ShaderDefVal], flag: &str) -> bool {
    definitions.iter().any(|definition| {
        matches!(definition,
        bevy::shader::ShaderDefVal::Bool(name, true) if name == flag)
    })
}
