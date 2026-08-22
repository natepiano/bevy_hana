use std::env;
#[cfg(test)]
use std::ffi::OsString;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::io;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering;

use bevy::prelude::Reflect;
use bevy::prelude::Resource;

use super::constants::DEFAULT_KEYMAP_FILE_NAME;
use super::constants::SCHEMA_FILE_NAME;
use super::constants::USER_KEYMAP_FILE_NAME;
use super::constants::XDG_CONFIG_HOME;

/// Where the keymap plugin keeps the user's own keymap document.
///
/// [`KeymapConfigurationDirectory::Unconfigured`] is the state
/// [`KeymapPathFailure::AppNameNotConfigured`] reports: an application that named no directory
/// runs from its embedded defaults and writes nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeymapConfigurationDirectory {
    /// Read and write the user's keymap under this application name in the platform
    /// configuration directory.
    ForApplication(String),
    /// Run from the embedded defaults alone, touching no configuration directory.
    Unconfigured,
}

/// Whether an application's keymap configuration directory resolved.
///
/// The plugin inserts this resource on both outcomes, so a reader never has to tell "this
/// application has no keymap directory" apart from "keymap assembly has not run yet".
#[derive(Clone, Debug, Eq, PartialEq, Resource)]
pub enum KeymapPathAvailability {
    /// Every keymap file has a location under a resolved configuration directory.
    Resolved(KeymapPaths),
    /// No configuration directory resolved, so no keymap file can be read or written.
    Unavailable(KeymapPathFailure),
}

impl From<&KeymapConfigurationDirectory> for KeymapPathAvailability {
    fn from(keymap_configuration_directory: &KeymapConfigurationDirectory) -> Self {
        match keymap_configuration_directory {
            KeymapConfigurationDirectory::ForApplication(app_name) => Self::for_app_name(app_name),
            KeymapConfigurationDirectory::Unconfigured => {
                Self::Unavailable(KeymapPathFailure::AppNameNotConfigured)
            },
        }
    }
}

impl KeymapPathAvailability {
    /// Resolves the keymap paths for one application name.
    #[must_use]
    pub fn for_app_name(app_name: &str) -> Self {
        let Some(app_name) = valid_app_name(app_name) else {
            return Self::Unavailable(KeymapPathFailure::AppNameNotOnePathComponent);
        };
        let Some(configuration_directory) = configuration_directory() else {
            return Self::Unavailable(KeymapPathFailure::NoPlatformConfigurationDirectory);
        };

        Self::Resolved(KeymapPaths::from_config_directory(
            configuration_directory.join(app_name),
        ))
    }

    /// Returns the resolved paths, or the reason the application has none.
    ///
    /// # Errors
    ///
    /// Returns the [`KeymapPathFailure`] recorded when the configuration directory did not
    /// resolve.
    pub const fn resolved(&self) -> Result<&KeymapPaths, KeymapPathFailure> {
        match self {
            Self::Resolved(keymap_paths) => Ok(keymap_paths),
            Self::Unavailable(keymap_path_failure) => Err(*keymap_path_failure),
        }
    }

    /// Takes the resolved paths, or reports the reason the application has none.
    ///
    /// # Errors
    ///
    /// Returns the [`KeymapPathFailure`] recorded when the configuration directory did not
    /// resolve.
    pub fn into_resolved(self) -> Result<KeymapPaths, KeymapPathFailure> {
        match self {
            Self::Resolved(keymap_paths) => Ok(keymap_paths),
            Self::Unavailable(keymap_path_failure) => Err(keymap_path_failure),
        }
    }
}

/// Why an application's keymap configuration directory did not resolve.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum KeymapPathFailure {
    /// The keymap plugin was built [`KeymapConfigurationDirectory::Unconfigured`], with no
    /// application name to name a directory after.
    AppNameNotConfigured,
    /// The configured application name is not one ordinary path component.
    AppNameNotOnePathComponent,
    /// The platform reported no configuration directory to place the application directory in.
    NoPlatformConfigurationDirectory,
}

impl KeymapPathFailure {
    /// Returns the sentence a startup diagnostic reports this failure with.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::AppNameNotConfigured => {
                "Could not resolve a configuration directory because no keymap application name \
                 was configured."
            },
            Self::AppNameNotOnePathComponent => {
                "Could not resolve a configuration directory because the keymap application name \
                 is not one path component."
            },
            Self::NoPlatformConfigurationDirectory => {
                "Could not resolve a configuration directory for the keymap application."
            },
        }
    }
}

/// Resolved locations for an application's user keymap and generated companions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeymapPaths {
    config_directory: PathBuf,
    default_keymap:   PathBuf,
    schema:           PathBuf,
    user_keymap:      PathBuf,
}

impl KeymapPaths {
    /// Returns the directory that contains every keymap file for this application.
    #[must_use]
    pub fn config_directory(&self) -> &Path { &self.config_directory }

    /// Returns the generated, read-only default keymap reference path.
    #[must_use]
    pub fn default_keymap(&self) -> &Path { &self.default_keymap }

    /// Returns the generated, read-only JSON Schema path.
    #[must_use]
    pub fn schema(&self) -> &Path { &self.schema }

    /// Returns the user-authored JSONC keymap path.
    #[must_use]
    pub fn user_keymap(&self) -> &Path { &self.user_keymap }

    fn from_config_directory(config_directory: PathBuf) -> Self {
        Self {
            default_keymap: config_directory.join(DEFAULT_KEYMAP_FILE_NAME),
            schema: config_directory.join(SCHEMA_FILE_NAME),
            user_keymap: config_directory.join(USER_KEYMAP_FILE_NAME),
            config_directory,
        }
    }
}

fn valid_app_name(app_name: &str) -> Option<&Path> {
    let app_name = Path::new(app_name);
    let mut components = app_name.components();

    matches!(components.next(), Some(Component::Normal(_)))
        .then_some(())
        .filter(|()| components.next().is_none())
        .map(|()| app_name)
}

fn configuration_directory() -> Option<PathBuf> {
    configuration_root(
        env::var_os(XDG_CONFIG_HOME).map(PathBuf::from),
        dirs::config_dir(),
        dirs::home_dir(),
    )
}

fn configuration_root(
    xdg_config_home: Option<PathBuf>,
    platform_config_directory: Option<PathBuf>,
    home_directory: Option<PathBuf>,
) -> Option<PathBuf> {
    xdg_config_home
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            if cfg!(target_os = "macos") {
                home_directory.map(|home_directory| home_directory.join(".config"))
            } else {
                platform_config_directory
            }
        })
}

#[cfg(test)]
pub(crate) static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
static NEXT_TEST_DIRECTORY_ID: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) struct TestDirectory {
    path: PathBuf,
}

#[cfg(test)]
impl TestDirectory {
    pub(crate) fn new(label: &str) -> io::Result<Self> {
        let directory_id = NEXT_TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "hana-rubric-{label}-{}-{directory_id}",
            std::process::id()
        ));

        fs::create_dir_all(&path)?;

        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path { &self.path }
}

#[cfg(test)]
impl Drop for TestDirectory {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.path); }
}

#[cfg(test)]
pub(crate) struct XdgConfigHome {
    previous: Option<OsString>,
}

#[cfg(test)]
impl XdgConfigHome {
    pub(crate) fn set(path: &Path) -> Self {
        let previous = env::var_os(XDG_CONFIG_HOME);

        // SAFETY: ENVIRONMENT_LOCK serializes all test mutations of this process-wide variable.
        unsafe { env::set_var(XDG_CONFIG_HOME, path) };

        Self { previous }
    }
}

#[cfg(test)]
impl Drop for XdgConfigHome {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(previous) => {
                // SAFETY: ENVIRONMENT_LOCK serializes all test mutations of this variable.
                unsafe { env::set_var(XDG_CONFIG_HOME, previous) };
            },
            None => {
                // SAFETY: ENVIRONMENT_LOCK serializes all test mutations of this variable.
                unsafe { env::remove_var(XDG_CONFIG_HOME) };
            },
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests stop when their isolated configuration setup fails"
)]
mod tests {
    use super::ENVIRONMENT_LOCK;
    use super::KeymapPathAvailability;
    use super::KeymapPathFailure;
    use super::TestDirectory;
    use super::XdgConfigHome;
    #[cfg(target_os = "macos")]
    use super::configuration_root;

    const TEST_APP_NAME: &str = "hana-rubric-paths-test";

    #[test]
    fn paths_resolve_under_xdg_config_home() {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory = TestDirectory::new("paths").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test app name and config path are valid");

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        assert_eq!(
            paths.user_keymap(),
            temporary_directory
                .path()
                .join(TEST_APP_NAME)
                .join("keymap.jsonc")
        );
        assert_eq!(
            paths.default_keymap(),
            temporary_directory
                .path()
                .join(TEST_APP_NAME)
                .join("keymap.default.jsonc")
        );
        assert_eq!(
            paths.schema(),
            temporary_directory
                .path()
                .join(TEST_APP_NAME)
                .join("keymap.schema.json")
        );

        drop(xdg_config_home);
        drop(environment_lock);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_without_xdg_uses_dot_config_under_home() {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("macos-fallback").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let injected_home = temporary_directory.path().join("home");

        assert_eq!(
            configuration_root(None, None, Some(injected_home.clone())),
            Some(injected_home.join(".config"))
        );

        drop(xdg_config_home);
        drop(environment_lock);
    }

    #[test]
    fn rejects_app_names_that_are_not_one_path_component() {
        for rejected_app_name in ["", "hana/rubric", "../hana"] {
            assert_eq!(
                KeymapPathAvailability::for_app_name(rejected_app_name).into_resolved(),
                Err(KeymapPathFailure::AppNameNotOnePathComponent)
            );
        }
    }
}
