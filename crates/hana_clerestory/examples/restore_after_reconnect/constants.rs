// fields
pub(super) const FIELD_ATTEMPT: &str = "attempt";
pub(super) const FIELD_CANDIDATE: &str = "candidate";
pub(super) const FIELD_ENDING: &str = "ending";
pub(super) const FIELD_MONITOR: &str = "monitor";
pub(super) const FIELD_RECOVERY_CYCLE: &str = "recovery_cycle";
pub(super) const FIELD_ROLE: &str = "role";
pub(super) const FIELD_ROLE_STATUS: &str = "role_status";
pub(super) const FIELD_SELECTED_MONITOR_INDEX: &str = "selected_monitor_index";
pub(super) const FIELD_STARTUP_MODE: &str = "startup_mode";
pub(super) const FIELD_WINDOW: &str = "window";
pub(super) const FIELD_WINDOW_KEY: &str = "window_key";

// kinds
pub(super) const KIND_ATTEMPT_ENDED: &str = "attempt-ended";
pub(super) const KIND_IDENTITY_QUESTION_RAISED: &str = "identity-question-raised";
pub(super) const KIND_MONITOR_CONNECTED: &str = "monitor-connected";
pub(super) const KIND_MONITOR_DISCONNECTED: &str = "monitor-disconnected";
pub(super) const KIND_PROBE_SESSION: &str = "probe-session";
pub(super) const KIND_RECOVERY_AVAILABLE: &str = "recovery-available";
pub(super) const KIND_RECOVERY_CANCELLATION_REQUESTED: &str = "recovery-cancellation-requested";
pub(super) const KIND_RECOVERY_MISMATCH: &str = "recovery-mismatch";
pub(super) const KIND_RECOVERY_PENDING: &str = "recovery-pending";
pub(super) const KIND_RECOVERY_READY: &str = "recovery-ready";
pub(super) const KIND_RECOVERY_RESTORE_REQUESTED: &str = "recovery-restore-requested";
pub(super) const KIND_RECOVERY_RESTORED: &str = "recovery-restored";
pub(super) const KIND_ROLE_STATUS_CHANGED: &str = "role-status-changed";
pub(super) const KIND_WINDOW_CREATED: &str = "window-created";

// probe configuration
pub(super) const DEFAULT_EXTERNAL_MONITOR_INDEX: usize = 1;
pub(super) const DEFAULT_PROBE_PORT: u16 = 15_702;
pub(super) const EXIT_AFTER_FRAME_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_PROBE_EXIT_AFTER_FRAME";
pub(super) const KEYBOARD_COMMAND_ID_PREFIX: &str = "keyboard";
pub(super) const MONITOR_INDEX_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_PROBE_MONITOR_INDEX";
pub(super) const PERSISTENCE_FILE_PREFIX: &str = "hana-clerestory-hotplug-probe";
pub(super) const PROBE_BOOT_NONCE_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_PROBE_BOOT_NONCE";
pub(super) const PROBE_CAPABILITY_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_PROBE_CAPABILITY";
pub(super) const PROBE_PERSISTENCE_PATH_ENVIRONMENT_VARIABLE: &str =
    "CLERESTORY_PROBE_PERSISTENCE_PATH";
pub(super) const PROBE_PORT_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_PROBE_PORT";
pub(super) const PROBE_RUN_ID_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_PROBE_RUN_ID";
pub(super) const PROBE_SCHEMA_VERSION: u32 = 1;
pub(super) const SECOND_RECOVERY_CYCLE: &str = "2";
pub(super) const STARTUP_MODE_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_PROBE_STARTUP_MODE";

// producers
pub(super) const PRODUCER_APPLICATION_RECOVERY_CANCELLATION_REQUESTED: &str =
    "observer::LiveRoleChanged::application_consumer";
pub(super) const PRODUCER_ATTEMPT_ENDED: &str = "observer::LiveRoleChanged::AttemptEnded";
pub(super) const PRODUCER_AUTOMATIC_RECOVERY_CANCELLATION_REQUESTED: &str =
    "observer::ProbeCommandIntent::automatic_consumer";
pub(super) const PRODUCER_IDENTITY_QUESTION_RAISED: &str = "observer::IdentityQuestionRaised";
pub(super) const PRODUCER_MONITOR_CONNECTED: &str = "observer::DeviceArrived";
pub(super) const PRODUCER_MONITOR_DISCONNECTED: &str = "observer::DeviceChange";
pub(super) const PRODUCER_RECOVERY_AVAILABLE: &str = "observer::LiveRoleChanged";
pub(super) const PRODUCER_RECOVERY_MISMATCH: &str = "observer::WindowRestoreMismatch";
pub(super) const PRODUCER_RECOVERY_PENDING: &str = "observer::LiveRoleChanged";
pub(super) const PRODUCER_RECOVERY_READY: &str = "Last::record_probe_readiness";
pub(super) const PRODUCER_RECOVERY_RESTORE_REQUESTED: &str =
    "observer::LiveRoleChanged::application_consumer";
pub(super) const PRODUCER_RECOVERY_RESTORED: &str = "observer::WindowRestored";
pub(super) const PRODUCER_ROLE_STATUS_CHANGED: &str = "observer::LiveRoleChanged::Status";
pub(super) const PRODUCER_STARTUP_SESSION: &str = "Startup::trace_probe_session";
pub(super) const PRODUCER_WINDOW_CREATED: &str = "observer::Add<ProbeWindowScenario>";

// remote methods
pub(super) const PROBE_COMMAND_METHOD: &str = "clerestory/probe_command";
pub(super) const PROBE_RECORDS_METHOD: &str = "clerestory/probe_records";
pub(super) const PROBE_SHUTDOWN_METHOD: &str = "clerestory/probe_shutdown";
pub(super) const PROBE_SNAPSHOT_METHOD: &str = "clerestory/probe_snapshot";

// startup modes
pub(super) const STARTUP_MODE_BORDERLESS: &str = "borderless";
pub(super) const STARTUP_MODE_EXCLUSIVE: &str = "exclusive";
pub(super) const STARTUP_MODE_WINDOWED: &str = "windowed";

// windows
pub(super) const APPLICATION_WINDOW_KEY: &str = "hotplug-application";
pub(super) const APPLICATION_WINDOW_TITLE: &str =
    "Clerestory Reconnect Consumer - Application Controlled";
pub(super) const AUTOMATIC_WINDOW_KEY: &str = "hotplug-automatic";
pub(super) const AUTOMATIC_WINDOW_TITLE: &str = "Clerestory Reconnect Consumer - Managed Automatic";
pub(super) const CONTROL_WINDOW_TITLE: &str =
    "Clerestory Reconnect Consumer - Unregistered Control";
pub(super) const PRIMARY_WINDOW_TITLE: &str = "Clerestory Reconnect Consumer - Primary Automatic";
pub(super) const PROBE_WINDOW_COUNT: usize = 5;
pub(super) const PROBE_WINDOW_HEIGHT: u32 = 540;
pub(super) const PROBE_WINDOW_WIDTH: u32 = 800;
pub(super) const RESTORE_ONLY_WINDOW_KEY: &str = "hotplug-restore-only";
pub(super) const RESTORE_ONLY_WINDOW_TITLE: &str = "Clerestory Reconnect Consumer - Restore Only";
