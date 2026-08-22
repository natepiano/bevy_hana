//! Keymap failures as palette rows.
//!
//! The rows sit above the query field so the commands that repair a broken
//! keymap stay in the same box as the report of what broke.

use std::path::Path;
use std::path::PathBuf;

use hana_rubric::Diagnostic;
use hana_rubric::DiagnosticOrigin;
use hana_rubric::DiagnosticSeverity;
use hana_rubric::KeymapLoadFailures;
use hana_rubric::KeymapPathAvailability;

/// What the palette offers for repairing one keymap failure.
///
/// [`DiagnosticOrigin`] names files that were never written — a companion file
/// that failed to publish, a schema that was never generated — so a row only
/// carries an action once the location is confirmed to exist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum KeymapFailureAction {
    /// Open this existing keymap file in the user's editor.
    OpenFile(PathBuf),
    /// Reveal this configuration directory in the platform file browser.
    RevealDirectory(PathBuf),
    /// The failure names nothing that can be opened, so the row is text only.
    NoAction,
}

/// What a failure row prints in its action column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KeymapFailureActionLabel {
    /// The clickable verb this action offers.
    Verb(&'static str),
    /// The failure names nothing that can be opened, so the column is empty.
    NoAction,
}

impl KeymapFailureAction {
    /// Returns the short verb the row renders in its action column.
    pub(super) const fn label(&self) -> KeymapFailureActionLabel {
        match self {
            Self::OpenFile(_) => KeymapFailureActionLabel::Verb("Open"),
            Self::RevealDirectory(_) => KeymapFailureActionLabel::Verb("Reveal"),
            Self::NoAction => KeymapFailureActionLabel::NoAction,
        }
    }
}

/// One keymap failure, rendered as a palette row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct KeymapFailureRow {
    /// Whether the failure stopped the keymap from loading or is an authoring
    /// note. The palette renders the two weights differently.
    pub(super) severity: DiagnosticSeverity,
    /// Where the failure came from — a file path, a directory, a condition
    /// name, or the sentence explaining why no path resolved.
    pub(super) location: String,
    /// What was wrong.
    pub(super) message:  String,
    /// The repair action offered for this row.
    pub(super) action:   KeymapFailureAction,
}

/// Builds one row per retained and reload-produced keymap failure.
///
/// Both collections are read: context-registration failures live only in
/// [`KeymapLoadFailures::retained_diagnostics`], so reading the reload half
/// alone shows an empty list on exactly the worst failure.
#[must_use]
pub(super) fn keymap_failure_rows(
    keymap_load_failures: &KeymapLoadFailures,
    keymap_path_availability: &KeymapPathAvailability,
) -> Vec<KeymapFailureRow> {
    keymap_load_failures
        .diagnostics
        .iter()
        .chain(&keymap_load_failures.retained_diagnostics)
        .map(|diagnostic| keymap_failure_row(diagnostic, keymap_path_availability))
        .collect()
}

fn keymap_failure_row(
    diagnostic: &Diagnostic,
    keymap_path_availability: &KeymapPathAvailability,
) -> KeymapFailureRow {
    let (location, action) = match &diagnostic.origin {
        DiagnosticOrigin::KeymapFile(path) => (
            source_location(diagnostic, path),
            open_action_if_present(path),
        ),
        DiagnosticOrigin::KeymapDirectory(path) => (
            path.display().to_string(),
            KeymapFailureAction::RevealDirectory(path.clone()),
        ),
        DiagnosticOrigin::EmbeddedDefaults => keymap_path_availability.resolved().map_or_else(
            |_| (diagnostic.origin.to_string(), KeymapFailureAction::NoAction),
            |keymap_paths| {
                let default_keymap = keymap_paths.default_keymap();
                (
                    source_location(diagnostic, default_keymap),
                    open_action_if_present(default_keymap),
                )
            },
        ),
        DiagnosticOrigin::ContextRegistration => (
            condition_location(diagnostic),
            KeymapFailureAction::NoAction,
        ),
        DiagnosticOrigin::CommandRegistration
        | DiagnosticOrigin::DiskWorker
        | DiagnosticOrigin::PathsUnavailable(_) => {
            (diagnostic.origin.to_string(), KeymapFailureAction::NoAction)
        },
    };

    KeymapFailureRow {
        severity: diagnostic.severity,
        location,
        message: diagnostic.message.clone(),
        action,
    }
}

/// Names the document position of a document-backed diagnostic: the file's name
/// and the source position within it.
///
/// The containing directory is dropped. The row is clipped to the panel width,
/// and an absolute path is long enough to push the message that says what is
/// wrong off the end of the line; the file name and line number are what a
/// reader acts on, and the row's repair action opens the file itself.
///
/// A startup diagnostic writes `line: 0`, which means "no location" rather than
/// line zero, so the file name is rendered alone.
fn source_location(diagnostic: &Diagnostic, path: &Path) -> String {
    let file_name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    if diagnostic.line == 0 {
        file_name
    } else {
        format!("{file_name}:{}:{}", diagnostic.line, diagnostic.column)
    }
}

/// Reports the condition or context-variant name a registration failure names,
/// which is the only location a reader can act on.
fn condition_location(diagnostic: &Diagnostic) -> String {
    if diagnostic.context.is_empty() {
        diagnostic.origin.to_string()
    } else {
        diagnostic.context.clone()
    }
}

fn open_action_if_present(path: &Path) -> KeymapFailureAction {
    if path.exists() {
        KeymapFailureAction::OpenFile(path.to_path_buf())
    } else {
        KeymapFailureAction::NoAction
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;

    use hana_rubric::Diagnostic;
    use hana_rubric::DiagnosticKind;
    use hana_rubric::DiagnosticOrigin;
    use hana_rubric::DiagnosticSeverity;
    use hana_rubric::KeymapLoadFailures;
    use hana_rubric::KeymapPathAvailability;
    use hana_rubric::KeymapPathFailure;

    use super::KeymapFailureAction;
    use super::keymap_failure_rows;

    const CONDITION_NAME: &str = "dimension_lock";
    const FAILURE_MESSAGE: &str = "The command is unavailable.";
    const MISSING_KEYMAP: &str = "/tmp/fairy-dust-keymap-that-does-not-exist/keymap.jsonc";
    const MISSING_KEYMAP_FILE_NAME: &str = "keymap.jsonc";
    const SOURCE_COLUMN: usize = 7;
    const SOURCE_LINE: usize = 12;

    fn diagnostic(origin: DiagnosticOrigin) -> Diagnostic {
        Diagnostic {
            origin,
            byte_range: 0..0,
            line: 0,
            column: 0,
            block_index: 0,
            context: String::new(),
            original_keystroke: String::new(),
            command_id: String::new(),
            kind: DiagnosticKind::Command,
            severity: DiagnosticSeverity::Failure,
            message: FAILURE_MESSAGE.to_owned(),
            suggestions: Vec::new(),
        }
    }

    fn unavailable() -> KeymapPathAvailability {
        KeymapPathAvailability::Unavailable(KeymapPathFailure::AppNameNotConfigured)
    }

    #[test]
    fn no_failures_render_no_rows() {
        assert!(
            keymap_failure_rows(&KeymapLoadFailures::default(), &unavailable()).is_empty(),
            "an application with a clean keymap must render no failure rows"
        );
    }

    #[test]
    fn retained_diagnostics_render_alongside_reload_diagnostics() {
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![diagnostic(DiagnosticOrigin::KeymapFile(PathBuf::from(
                MISSING_KEYMAP,
            )))],
            retained_diagnostics: vec![diagnostic(DiagnosticOrigin::ContextRegistration)],
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].location, MISSING_KEYMAP_FILE_NAME);
        assert_eq!(rows[1].location, "context registration");
    }

    #[test]
    fn severity_is_carried_through_so_advisories_render_distinguishably() {
        let mut advisory = diagnostic(DiagnosticOrigin::ContextRegistration);
        advisory.severity = DiagnosticSeverity::Advisory;
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![diagnostic(DiagnosticOrigin::ContextRegistration), advisory],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(rows[0].severity, DiagnosticSeverity::Failure);
        assert_eq!(rows[1].severity, DiagnosticSeverity::Advisory);
    }

    #[test]
    fn keymap_file_offers_an_open_action_only_when_the_file_exists() {
        let existing = std::env::current_exe().expect("the test binary path resolves");
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![
                diagnostic(DiagnosticOrigin::KeymapFile(existing.clone())),
                diagnostic(DiagnosticOrigin::KeymapFile(PathBuf::from(MISSING_KEYMAP))),
            ],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(rows[0].action, KeymapFailureAction::OpenFile(existing));
        assert_eq!(rows[1].action, KeymapFailureAction::NoAction);
    }

    #[test]
    fn keymap_directory_offers_a_reveal_action() {
        let directory = std::env::temp_dir();
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![diagnostic(DiagnosticOrigin::KeymapDirectory(
                directory.clone(),
            ))],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(rows[0].location, directory.display().to_string());
        assert_eq!(
            rows[0].action,
            KeymapFailureAction::RevealDirectory(directory)
        );
    }

    #[test]
    fn embedded_defaults_report_the_published_default_keymap_file() {
        let resolved =
            KeymapPathAvailability::for_app_name("fairy_dust_command_palette_failure_row_test");
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![diagnostic(DiagnosticOrigin::EmbeddedDefaults)],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &resolved);

        match resolved.resolved() {
            Ok(keymap_paths) => assert_eq!(
                rows[0].location,
                keymap_paths
                    .default_keymap()
                    .file_name()
                    .expect("the published default keymap is a file")
                    .to_string_lossy()
            ),
            Err(_) => assert_eq!(rows[0].location, "embedded defaults"),
        }
    }

    #[test]
    fn embedded_defaults_fall_back_to_their_label_without_resolved_paths() {
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![diagnostic(DiagnosticOrigin::EmbeddedDefaults)],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(rows[0].location, "embedded defaults");
        assert_eq!(rows[0].action, KeymapFailureAction::NoAction);
    }

    #[test]
    fn context_registration_reports_the_condition_name() {
        let mut context_failure = diagnostic(DiagnosticOrigin::ContextRegistration);
        context_failure.context = CONDITION_NAME.to_owned();
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![context_failure],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(rows[0].location, CONDITION_NAME);
        assert_eq!(rows[0].action, KeymapFailureAction::NoAction);
    }

    #[test]
    fn command_registration_and_disk_worker_report_their_own_labels() {
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![
                diagnostic(DiagnosticOrigin::CommandRegistration),
                diagnostic(DiagnosticOrigin::DiskWorker),
            ],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(rows[0].location, "command registration");
        assert_eq!(rows[0].action, KeymapFailureAction::NoAction);
        assert_eq!(rows[1].location, "keymap disk worker");
        assert_eq!(rows[1].action, KeymapFailureAction::NoAction);
    }

    /// The diagnostic's own message is already the path failure's reason, so the
    /// location column names the origin instead of printing that reason twice.
    #[test]
    fn unresolved_paths_name_their_origin_rather_than_repeating_the_reason() {
        let keymap_path_failure = KeymapPathFailure::NoPlatformConfigurationDirectory;
        let origin = DiagnosticOrigin::PathsUnavailable(keymap_path_failure);
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![diagnostic(origin.clone())],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(rows[0].location, origin.to_string());
        assert_ne!(rows[0].location, rows[0].message);
        assert_eq!(rows[0].action, KeymapFailureAction::NoAction);
    }

    /// The row is one clipped line, so the directory is dropped and the file
    /// name carries the source position. The full path would spend the whole
    /// line before the message that says what is wrong.
    #[test]
    fn a_document_backed_diagnostic_names_its_file_and_source_position() {
        let mut located = diagnostic(DiagnosticOrigin::KeymapFile(PathBuf::from(MISSING_KEYMAP)));
        located.line = SOURCE_LINE;
        located.column = SOURCE_COLUMN;
        let keymap_load_failures = KeymapLoadFailures {
            diagnostics:          vec![located],
            retained_diagnostics: Vec::new(),
        };

        let rows = keymap_failure_rows(&keymap_load_failures, &unavailable());

        assert_eq!(
            rows[0].location,
            format!("{MISSING_KEYMAP_FILE_NAME}:{SOURCE_LINE}:{SOURCE_COLUMN}")
        );
    }
}
