use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::constants::COMPANION_FILE_MODE;
use super::constants::TEMPORARY_FILE_ATTEMPTS;
use super::paths::KeymapPaths;
use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;

static NEXT_TEMPORARY_FILE_ID: AtomicUsize = AtomicUsize::new(0);

pub(super) fn publish_companion_files(
    paths: &KeymapPaths,
    default_keymap: &[u8],
    schema: Option<&[u8]>,
) -> Vec<Diagnostic> {
    if let Err(error) = fs::create_dir_all(paths.config_directory()) {
        return vec![companion_diagnostic(
            DiagnosticOrigin::KeymapDirectory(paths.config_directory().to_path_buf()),
            error,
        )];
    }

    let mut diagnostics = Vec::new();

    let mut companions = vec![(paths.default_keymap(), default_keymap)];
    if let Some(schema) = schema {
        companions.push((paths.schema(), schema));
    }
    for (path, contents) in companions {
        if let Err(error) = publish_companion_file(path, contents) {
            diagnostics.push(companion_diagnostic(
                DiagnosticOrigin::KeymapFile(path.to_path_buf()),
                error,
            ));
        }
    }

    diagnostics
}

fn publish_companion_file(destination: &Path, contents: &[u8]) -> io::Result<()> {
    if existing_regular_file_matches(destination, contents)? {
        return Ok(());
    }

    let (mut temporary_file, temporary_path) = create_temporary_file(destination)?;
    let result = write_temporary_file(&mut temporary_file, contents);
    drop(temporary_file);

    if let Err(error) = result {
        remove_temporary_file(&temporary_path);
        return Err(error);
    }

    if let Err(error) = set_companion_permissions(&temporary_path) {
        remove_temporary_file(&temporary_path);
        return Err(error);
    }

    if let Err(error) = replace_file(&temporary_path, destination) {
        remove_temporary_file(&temporary_path);
        return Err(error);
    }

    Ok(())
}

fn existing_regular_file_matches(destination: &Path, contents: &[u8]) -> io::Result<bool> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(fs::read(destination)? == contents),
        Ok(_) => Ok(false),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn create_temporary_file(destination: &Path) -> io::Result<(File, PathBuf)> {
    let directory = destination.parent().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            "companion destination has no parent directory",
        )
    })?;
    let file_name = destination.file_name().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            "companion destination has no file name",
        )
    })?;

    for _ in 0..TEMPORARY_FILE_ATTEMPTS {
        let temporary_file_id = NEXT_TEMPORARY_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temporary_path = directory.join(format!(
            ".{}.{}.{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            temporary_file_id
        ));

        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((file, temporary_path)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        "could not create a unique companion temporary file",
    ))
}

fn write_temporary_file(temporary_file: &mut File, contents: &[u8]) -> io::Result<()> {
    temporary_file.write_all(contents)?;
    temporary_file.sync_all()
}

#[cfg(unix)]
fn set_companion_permissions(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(COMPANION_FILE_MODE))
}

#[cfg(not(unix))]
fn set_companion_permissions(path: &Path) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(COMPANION_FILE_MODE & 0o222 == 0);

    fs::set_permissions(path, permissions)
}

fn replace_file(temporary_path: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary_path, destination)
}

fn remove_temporary_file(temporary_path: &Path) { let _ = fs::remove_file(temporary_path); }

fn companion_diagnostic(origin: DiagnosticOrigin, error: Error) -> Diagnostic {
    Diagnostic {
        origin,
        byte_range: 0..0,
        line: 0,
        column: 0,
        block_index: 0,
        context: String::new(),
        original_keystroke: String::new(),
        command_id: String::new(),
        kind: DiagnosticKind::Companion,
        severity: DiagnosticSeverity::Failure,
        message: format!("Could not publish the companion file: {error}"),
        suggestions: Vec::new(),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests stop when their isolated companion-file setup fails"
)]
mod tests {
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::publish_companion_files;
    use crate::DiagnosticKind;
    use crate::disk::paths::ENVIRONMENT_LOCK;
    use crate::disk::paths::KeymapPathAvailability;
    use crate::disk::paths::TestDirectory;
    use crate::disk::paths::XdgConfigHome;

    const DEFAULT_BYTES: &[u8] = b"// generated default\n{\"bindings\": []}\n";
    const SCHEMA_BYTES: &[u8] = b"{\"type\": \"object\"}\n";
    const TEST_APP_NAME: &str = "hana-rubric-companion-test";

    #[cfg(unix)]
    #[test]
    fn replaces_an_existing_read_only_companion_file() {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("read-only-file").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test app name and config path are valid");

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        fs::create_dir_all(paths.config_directory()).expect("configuration directory exists");
        fs::write(paths.default_keymap(), b"outdated default").expect("old default exists");
        fs::set_permissions(paths.default_keymap(), fs::Permissions::from_mode(0o444))
            .expect("old default is read only");
        let direct_write = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(paths.default_keymap());

        assert!(direct_write.is_err());

        let diagnostics = publish_companion_files(&paths, DEFAULT_BYTES, Some(SCHEMA_BYTES));

        assert!(diagnostics.is_empty());
        assert_eq!(
            fs::read(paths.default_keymap()).expect("default is readable"),
            DEFAULT_BYTES
        );
        assert_eq!(
            fs::metadata(paths.default_keymap())
                .expect("default metadata exists")
                .permissions()
                .mode()
                & 0o777,
            0o444
        );
        assert_eq!(
            fs::read(paths.schema()).expect("schema is readable"),
            SCHEMA_BYTES
        );
        assert_eq!(
            fs::metadata(paths.schema())
                .expect("schema metadata exists")
                .permissions()
                .mode()
                & 0o777,
            0o444
        );

        drop(xdg_config_home);
        drop(environment_lock);
    }

    #[cfg(unix)]
    #[test]
    fn leaves_matching_regular_files_untouched() {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("matching-file").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test app name and config path are valid");

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        fs::create_dir_all(paths.config_directory()).expect("configuration directory exists");
        fs::write(paths.default_keymap(), DEFAULT_BYTES).expect("matching default exists");
        fs::set_permissions(paths.default_keymap(), fs::Permissions::from_mode(0o640))
            .expect("matching default permissions are set");
        let before = fs::metadata(paths.default_keymap()).expect("default metadata exists");

        let diagnostics = publish_companion_files(&paths, DEFAULT_BYTES, Some(SCHEMA_BYTES));
        let after = fs::metadata(paths.default_keymap()).expect("default metadata exists");

        assert!(diagnostics.is_empty());
        assert_eq!(after.ino(), before.ino());
        assert_eq!(after.permissions().mode() & 0o777, 0o640);

        drop(xdg_config_home);
        drop(environment_lock);
    }

    #[cfg(unix)]
    #[test]
    fn reports_a_read_only_directory_without_failing_publication() {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("read-only-directory").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test app name and config path are valid");

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        fs::create_dir_all(paths.config_directory()).expect("configuration directory exists");
        fs::set_permissions(paths.config_directory(), fs::Permissions::from_mode(0o555))
            .expect("configuration directory is read only");

        let diagnostics = publish_companion_files(&paths, DEFAULT_BYTES, Some(SCHEMA_BYTES));

        assert!(!diagnostics.is_empty());
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.kind == DiagnosticKind::Companion)
        );

        fs::set_permissions(paths.config_directory(), fs::Permissions::from_mode(0o755))
            .expect("configuration directory permissions are restored");
        drop(xdg_config_home);
        drop(environment_lock);
    }

    #[cfg(unix)]
    #[test]
    fn replaces_a_destination_symlink_without_following_it() {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("symlink").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test app name and config path are valid");
        let linked_file = temporary_directory.path().join("linked-schema.json");

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        fs::create_dir_all(paths.config_directory()).expect("configuration directory exists");
        fs::write(&linked_file, b"linked file contents").expect("linked file exists");
        symlink(&linked_file, paths.schema()).expect("schema path is a symlink");

        let diagnostics = publish_companion_files(&paths, DEFAULT_BYTES, Some(SCHEMA_BYTES));

        assert!(diagnostics.is_empty());
        assert_eq!(
            fs::read(&linked_file).expect("linked file is readable"),
            b"linked file contents"
        );
        assert!(
            fs::symlink_metadata(paths.schema())
                .expect("schema metadata exists")
                .file_type()
                .is_file()
        );
        assert_eq!(
            fs::read(paths.schema()).expect("schema is readable"),
            SCHEMA_BYTES
        );

        drop(xdg_config_home);
        drop(environment_lock);
    }
}
