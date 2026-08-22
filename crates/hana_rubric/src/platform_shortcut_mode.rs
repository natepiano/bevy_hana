/// Selects whether the platform's primary shortcut modifier is `Cmd` (macOS)
/// or `Ctrl` (everywhere else).
#[derive(Clone, Copy)]
pub(crate) enum PlatformShortcutMode {
    Command,
    Control,
}

impl PlatformShortcutMode {
    pub(crate) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Command
        } else {
            Self::Control
        }
    }
}
