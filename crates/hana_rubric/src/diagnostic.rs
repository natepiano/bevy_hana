use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::ops::Range;
use std::path::PathBuf;

use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;

use crate::disk::KeymapPathFailure;

/// Categorizes the keymap failure described by a [`Diagnostic`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum DiagnosticKind {
    /// The JSONC document does not follow the keymap syntax.
    Syntax,
    /// A binding contains invalid keystroke text.
    Keystroke,
    /// A binding refers to an unavailable command.
    Command,
    /// A binding refers to an unavailable input context.
    Context,
    /// The keymap file could not be read or watched.
    Disk,
    /// A companion file could not be published.
    Companion,
    /// Multiple command events declare the same command ID.
    DuplicateCommandId,
    /// A command event has no palette title text.
    MissingCommandTitle,
    /// A command event has no palette description text.
    MissingCommandDescription,
    /// A command event declares malformed command ID text.
    InvalidCommandId,
    /// A command event has no reflected event trigger handle.
    CommandEventNotReflected,
    /// A hold-to-act command was bound to a multi-keystroke sequence.
    HeldCommandInSequence,
    /// A bare modifier was not the sole keystroke of a hold-to-act command.
    BareModifierRequiresHeldCommand,
    /// A permanently registered recovery command appeared in a keymap file.
    UnremappableCommand,
    /// A keymap sequence begins with a protected recovery keystroke.
    ReservedKeystroke,
    /// A configured protected recovery command association is invalid.
    ProtectedCommandBinding,
    /// The keymap plugin was assembled without an embedded default keymap.
    MissingDefaultKeymap,
    /// The keymap plugin was assembled without an application name, embedded defaults, or a
    /// protected keystroke, so it has nothing to compile bindings from.
    UnconfiguredKeymapPlugin,
}

/// What produced a [`Diagnostic`]. Only [`DiagnosticOrigin::KeymapFile`] names a keymap source
/// file; a consumer picks its rendering from the variant and never inspects the path's form.
#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub enum DiagnosticOrigin {
    /// A keymap file read from disk, named by the path it was read from.
    KeymapFile(PathBuf),
    /// The keymap configuration directory itself, for failures that belong to no single file —
    /// creating the directory, or watching it.
    KeymapDirectory(PathBuf),
    /// The default keymap the application compiled into its binary.
    EmbeddedDefaults,
    /// Registering the application's keymap contexts, which reads no keymap source.
    ContextRegistration,
    /// Registering the application's keymap commands, which reads no keymap source.
    CommandRegistration,
    /// The disk worker itself, for reports about delivery rather than about any keymap source.
    DiskWorker,
    /// The keymap configuration directory did not resolve, so no keymap file has a location.
    ///
    /// Render [`KeymapPathFailure::reason`] to report why.
    PathsUnavailable(KeymapPathFailure),
}

impl Display for DiagnosticOrigin {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeymapFile(path) | Self::KeymapDirectory(path) => {
                write!(formatter, "{}", path.display())
            },
            Self::EmbeddedDefaults => formatter.write_str("embedded defaults"),
            Self::ContextRegistration => formatter.write_str("context registration"),
            Self::CommandRegistration => formatter.write_str("command registration"),
            Self::DiskWorker => formatter.write_str("keymap disk worker"),
            Self::PathsUnavailable(failure) => {
                write!(
                    formatter,
                    "unresolved keymap directory: {}",
                    failure.reason()
                )
            },
        }
    }
}

/// Describes whether a keymap diagnostic prevents its binding from loading.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum DiagnosticSeverity {
    /// The affected binding or document cannot be loaded.
    Failure,
    /// The document loaded, but the authored source should be corrected.
    Advisory,
}

/// One keymap problem retained for application diagnostics and BRP inspection.
#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct Diagnostic {
    /// What produced this diagnostic.
    pub origin:             DiagnosticOrigin,
    /// Byte range within the contents of [`Self::origin`] that identifies
    /// the problem.
    pub byte_range:         Range<usize>,
    /// One-based source line containing [`Self::byte_range`].
    pub line:               usize,
    /// One-based source column containing [`Self::byte_range`].
    pub column:             usize,
    /// Zero-based keymap block that produced this diagnostic.
    pub block_index:        usize,
    /// Input context text associated with the binding.
    pub context:            String,
    /// Original keystroke text from the binding.
    pub original_keystroke: String,
    /// Command ID text from the binding.
    pub command_id:         String,
    /// Diagnostic category reported for this source location.
    pub kind:               DiagnosticKind,
    /// Whether this diagnostic prevents the affected binding from loading.
    pub severity:           DiagnosticSeverity,
    /// Human-readable sentence describing this diagnostic.
    ///
    /// Unlike [`Self::suggestions`], this is not machine-applicable replacement text. This
    /// required field uses an empty string when no message applies.
    pub message:            String,
    /// Text replacements that can repair the keymap source.
    pub suggestions:        Vec<String>,
}

/// Problems reported by keymap loads and diagnostics retained from startup.
///
/// A later reload transaction retains advisories after a successful load, while startup
/// diagnostics remain available because a reload cannot reproduce them.
#[derive(Clone, Debug, Default, Reflect, Resource)]
#[reflect(Resource)]
pub struct KeymapLoadFailures {
    /// Diagnostics from the current or most recent keymap load.
    pub diagnostics:          Vec<Diagnostic>,
    /// Diagnostics produced during startup that no keymap reload can reproduce.
    pub retained_diagnostics: Vec<Diagnostic>,
}

impl KeymapLoadFailures {
    /// Iterates over both reload-produced and startup diagnostics.
    pub fn all_diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().chain(&self.retained_diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::ops::Range;
    use std::path::PathBuf;

    use super::Diagnostic;
    use super::DiagnosticKind;
    use super::DiagnosticOrigin;
    use super::DiagnosticSeverity;
    use super::KeymapLoadFailures;
    use crate::KeymapPathFailure;

    const KEYMAP_FIXTURE: &str = "keymap.jsonc";
    const KEYMAP_DIRECTORY_FIXTURE: &str = "keymap-config";

    #[test]
    fn diagnostic_fields_use_only_stable_data_types() {
        let diagnostic = Diagnostic {
            origin:             DiagnosticOrigin::KeymapFile(PathBuf::from(KEYMAP_FIXTURE)),
            byte_range:         0..1,
            line:               1,
            column:             1,
            block_index:        0,
            context:            String::from("editor"),
            original_keystroke: String::from("cmd-k"),
            command_id:         String::from("camera::home"),
            kind:               DiagnosticKind::Command,
            severity:           DiagnosticSeverity::Failure,
            message:            String::from("The command is unavailable."),
            suggestions:        vec![String::from("camera::home")],
        };
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![diagnostic],
            retained_diagnostics: Vec::new(),
        };

        let diagnostic = &keymap_load_failures.diagnostics[0];
        let _: &Range<usize> = &diagnostic.byte_range;
        let _: &DiagnosticOrigin = &diagnostic.origin;
        let _: &String = &diagnostic.context;
        let _: &String = &diagnostic.original_keystroke;
        let _: &String = &diagnostic.command_id;
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Failure);
        let _: &String = &diagnostic.message;
        assert_eq!(diagnostic.message, "The command is unavailable.");
        let _: &Vec<String> = &diagnostic.suggestions;
        let _: &Vec<Diagnostic> = &keymap_load_failures.diagnostics;
        assert_eq!(keymap_load_failures.all_diagnostics().count(), 1);
    }

    #[test]
    fn every_diagnostic_origin_renders_a_distinct_label() {
        let labels = [
            DiagnosticOrigin::KeymapFile(PathBuf::from(KEYMAP_FIXTURE)),
            DiagnosticOrigin::KeymapDirectory(PathBuf::from(KEYMAP_DIRECTORY_FIXTURE)),
            DiagnosticOrigin::EmbeddedDefaults,
            DiagnosticOrigin::ContextRegistration,
            DiagnosticOrigin::CommandRegistration,
            DiagnosticOrigin::DiskWorker,
            DiagnosticOrigin::PathsUnavailable(KeymapPathFailure::AppNameNotConfigured),
        ]
        .map(|diagnostic_origin| diagnostic_origin.to_string());

        assert_eq!(labels[0], KEYMAP_FIXTURE);
        assert_eq!(labels.iter().collect::<HashSet<_>>().len(), labels.len());
    }

    #[test]
    fn diagnostic_kind_includes_command_registry_failures() {
        let diagnostic_kinds = [
            DiagnosticKind::DuplicateCommandId,
            DiagnosticKind::MissingCommandTitle,
            DiagnosticKind::MissingCommandDescription,
            DiagnosticKind::InvalidCommandId,
            DiagnosticKind::CommandEventNotReflected,
        ];

        assert_eq!(diagnostic_kinds.len(), 5);
    }
}
