use bevy::prelude::KeyCode;
use bevy::prelude::Vec3;

use crate::scenarios::ScenarioDefinition;
use crate::scenarios::ScenarioKind;

// auto-mode
pub(super) const AUTO_EXIT_DELAY_SECS: f32 = 2.0;
pub(super) const AUTO_MODE_ENV_VAR: &str = "BENCHMARK_AUTO";
pub(super) const AUTO_MODE_ENV_VAR_ENABLED_VALUE: &str = "1";
pub(super) const AUTO_STARTUP_DELAY_SECS: f32 = 5.0;

// camera
pub(super) const CAMERA_LOOK_AT: Vec3 = Vec3::new(0.0, 4.0, 0.0);
pub(super) const CAMERA_POSITION: Vec3 = Vec3::new(8.0, 2.0, 14.0);

// cube fill ratios
pub(super) const CUBE_FILL_RATIO_FOR_5_ENTITIES: f32 = 0.45;
pub(super) const CUBE_FILL_RATIO_FOR_10_ENTITIES: f32 = 0.65;
pub(super) const CUBE_FILL_RATIO_FOR_100_ENTITIES: f32 = 0.55;
pub(super) const CUBE_FILL_RATIO_FOR_1000_ENTITIES: f32 = 0.35;
pub(super) const CUBE_FILL_RATIO_FOR_10000_ENTITIES: f32 = 0.25;
pub(super) const CUBE_FILL_RATIO_FOR_50000_ENTITIES: f32 = 0.15;

// grid layout
pub(super) const DEPTH_SPACING_MULTIPLIER: f32 = 3.0;
pub(super) const GRID_3D_COLUMNS: u32 = 10;
pub(super) const GRID_3D_ROWS: u32 = 10;
pub(super) const GRID_CENTER_DIVISOR: f32 = 2.0;
pub(super) const GRID_CENTER_OFFSET: f32 = 1.0;
pub(super) const GRID_FILL_FRACTION: f32 = 0.95;
pub(super) const GRID_TO_3D_THRESHOLD: u32 = 100;
pub(super) const GROUND_PLANE_SIZE: f32 = 100.0;
pub(super) const GROUND_PLANE_SUBDIVISIONS: u32 = 10;
pub(super) const GROUND_PLANE_Y: f32 = -3.0;
pub(super) const VIEWPORT_FOV_DIVISOR: f32 = 2.0;
pub(super) const VIEWPORT_HEIGHT_MULTIPLIER: f32 = 2.0;

// hud
pub(super) const BENCHMARK_LABEL: &str = "Bench:";
pub(super) const BENCHMARK_MODE_AUTO_LABEL: &str = "Auto";
pub(super) const BENCHMARK_MODE_INTERACTIVE_LABEL: &str = "Interactive";
pub(super) const BENCHMARK_PHASE_ANALYZE_LABEL: &str = "Analyzing...";
pub(super) const BENCHMARK_PHASE_IDLE_LABEL: &str = "Idle";
pub(super) const BENCHMARK_PHASE_SETUP_LABEL: &str = "Setting up...";
pub(super) const HEADS_UP_DISPLAY_CONTROLS: &str =
    "\n\n#: Switch scenario  M: Cycle mode  R: Auto run  L: Log results";
pub(super) const HEADS_UP_DISPLAY_FONT_SIZE: f32 = 18.0;
pub(super) const HEADS_UP_DISPLAY_FPS_PRECISION: usize = 0;
pub(super) const HEADS_UP_DISPLAY_FPS_WIDTH: usize = 4;
pub(super) const HEADS_UP_DISPLAY_MILLISECONDS_PRECISION: usize = 2;
pub(super) const HEADS_UP_DISPLAY_PADDING: f32 = 10.0;
pub(super) const HEADS_UP_DISPLAY_RESULTS_HEADER: &str = "\n\n--- Results ---";
pub(super) const HEADS_UP_DISPLAY_SECONDS_PRECISION: usize = 0;
pub(super) const HEADS_UP_DISPLAY_UPDATE_INTERVAL: f32 = 0.25;

// lighting
pub(super) const AMBIENT_LIGHT_BRIGHTNESS: f32 = 200.0;
pub(super) const LIGHT_INTENSITY: f32 = 10_000_000.0;
pub(super) const LIGHT_POSITION: Vec3 = Vec3::new(8.0, 16.0, 8.0);
pub(super) const LIGHT_RANGE: f32 = 100.0;

// measurement
pub(super) const MEASURE_FRAMES: u32 = 600;
pub(super) const MEDIAN_PERCENTILE: f64 = 50.0;
pub(super) const MILLISECONDS_PER_SECOND: f64 = 1000.0;
pub(super) const NINETY_FIFTH_PERCENTILE: f64 = 95.0;
pub(super) const NINETY_NINTH_PERCENTILE: f64 = 99.0;
pub(super) const WARMUP_FRAMES: u32 = 120;

// outline defaults
pub(super) const DEFAULT_OUTLINE_INTENSITY: f32 = 1.0;
pub(super) const DEFAULT_OUTLINE_WIDTH: f32 = 5.0;

// outline labels
pub(super) const OUTLINE_METHOD_JUMP_FLOOD_LABEL: &str = "JumpFlood";
pub(super) const OUTLINE_METHOD_SCREEN_HULL_LABEL: &str = "ScreenHull";
pub(super) const OUTLINE_METHOD_WORLD_HULL_LABEL: &str = "WorldHull";
pub(super) const OUTLINE_PRESENCE_DISABLED_LABEL: &str = "off";
pub(super) const OUTLINE_PRESENCE_ENABLED_LABEL: &str = "on";

// results
pub(super) const BENCHMARK_CSV_FRAMES_PER_SECOND_PRECISION: usize = 0;
pub(super) const BENCHMARK_CSV_HEADER: &str = "scenario,frames,average_ms,median_ms,percentile_95_ms,percentile_99_ms,min_ms,max_ms,average_frames_per_second";
pub(super) const BENCHMARK_CSV_MILLISECONDS_PRECISION: usize = 2;
pub(super) const BENCHMARK_RESULTS_BANNER: &str = "\n=== hana_liminal Benchmark Results ===\n";
pub(super) const BENCHMARK_RESULTS_FILE_PREFIX: &str = "benchmark_";
pub(super) const BENCHMARK_RESULTS_FILE_SUFFIX: &str = ".csv";
pub(super) const BENCHMARK_RESULTS_FRAMES_COLUMN_WIDTH: usize = 6;
pub(super) const BENCHMARK_RESULTS_FRAMES_PER_SECOND_PRECISION: usize = 0;
pub(super) const BENCHMARK_RESULTS_MILLISECONDS_PRECISION: usize = 2;
pub(super) const BENCHMARK_RESULTS_NAME_COLUMN_WIDTH: usize = 18;
pub(super) const BENCHMARK_RESULTS_NUMERIC_COLUMN_WIDTH: usize = 11;
pub(super) const BENCHMARK_RESULTS_SEPARATOR_FRAMES_COLUMN_WIDTH: usize = 8;
pub(super) const BENCHMARK_RESULTS_SEPARATOR_NUMERIC_COLUMN_WIDTH: usize = 13;
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_AVERAGE: &str = "Average(ms)";
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_FPS: &str = "FPS";
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_FRAMES: &str = "Frames";
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_MAX: &str = "Max(ms)";
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_MEDIAN: &str = "Median(ms)";
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_MIN: &str = "Min(ms)";
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_PERCENTILE_95: &str = "95th(ms)";
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_PERCENTILE_99: &str = "99th(ms)";
pub(super) const BENCHMARK_RESULTS_TABLE_HEADER_SCENARIO: &str = "Scenario";
pub(super) const DATE_COMMAND: &str = "date";
pub(super) const DATE_COMMAND_ARG_FORMAT: &str = "+%Y_%m_%d_%H_%M";
pub(super) const DATE_COMMAND_ARG_REFERENCE_TIME: &str = "-r";
pub(super) const RESULT_LABEL_PADDING: usize = 6;
pub(super) const RESULTS_DIRECTORY_NAME: &str = "results";
pub(super) const RESULTS_PER_SCENARIO: usize = 2;

// scenarios
pub(super) const SCENARIOS: &[ScenarioDefinition] = &[
    ScenarioDefinition {
        name:          "Entities1",
        key:           KeyCode::Digit1,
        scenario_kind: ScenarioKind::Grid {
            count:     1,
            width:     DEFAULT_OUTLINE_WIDTH,
            cube_fill: CUBE_FILL_RATIO_FOR_5_ENTITIES,
        },
    },
    ScenarioDefinition {
        name:          "Entities5",
        key:           KeyCode::Digit2,
        scenario_kind: ScenarioKind::Grid {
            count:     5,
            width:     DEFAULT_OUTLINE_WIDTH,
            cube_fill: CUBE_FILL_RATIO_FOR_5_ENTITIES,
        },
    },
    ScenarioDefinition {
        name:          "Entities10",
        key:           KeyCode::Digit3,
        scenario_kind: ScenarioKind::Grid {
            count:     10,
            width:     DEFAULT_OUTLINE_WIDTH,
            cube_fill: CUBE_FILL_RATIO_FOR_10_ENTITIES,
        },
    },
    ScenarioDefinition {
        name:          "Entities100",
        key:           KeyCode::Digit4,
        scenario_kind: ScenarioKind::Grid {
            count:     100,
            width:     DEFAULT_OUTLINE_WIDTH,
            cube_fill: CUBE_FILL_RATIO_FOR_100_ENTITIES,
        },
    },
    ScenarioDefinition {
        name:          "Entities1000",
        key:           KeyCode::Digit5,
        scenario_kind: ScenarioKind::Grid {
            count:     1000,
            width:     DEFAULT_OUTLINE_WIDTH,
            cube_fill: CUBE_FILL_RATIO_FOR_1000_ENTITIES,
        },
    },
    ScenarioDefinition {
        name:          "Entities10000",
        key:           KeyCode::Digit6,
        scenario_kind: ScenarioKind::Grid {
            count:     10000,
            width:     DEFAULT_OUTLINE_WIDTH,
            cube_fill: CUBE_FILL_RATIO_FOR_10000_ENTITIES,
        },
    },
    ScenarioDefinition {
        name:          "Entities50000",
        key:           KeyCode::Digit7,
        scenario_kind: ScenarioKind::Grid {
            count:     50000,
            width:     DEFAULT_OUTLINE_WIDTH,
            cube_fill: CUBE_FILL_RATIO_FOR_50000_ENTITIES,
        },
    },
];

// startup
pub(super) const BENCHMARK_WINDOW_TITLE: &str = "hana_liminal benchmark";
pub(super) const INITIALIZING_BENCHMARK_TEXT: &str = "Initializing benchmark...";

// tick
pub(super) const AUTO_BENCHMARK_COMPLETE_MESSAGE: &str = "Auto benchmark complete";
pub(super) const AUTO_BENCHMARK_START_MESSAGE: &str = "Starting auto benchmark run";
pub(super) const EXITING_MESSAGE: &str = "Exiting";
pub(super) const SCENARIO_SWITCH_MESSAGE: &str = "Switching to scenario: {}";
pub(super) const STARTUP_COMPLETE_MESSAGE: &str =
    "Startup delay complete, beginning auto benchmark";
