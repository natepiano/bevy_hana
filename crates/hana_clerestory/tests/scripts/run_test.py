#!/usr/bin/env python3
"""Single-test runner for hana_clerestory integration tests (v3).

Usage:
  # Prebuild:
  python3 tests/scripts/run_test.py --prebuild

  # Discovery:
  python3 tests/scripts/run_test.py --discover \
    --config tests/config/macos.json \
    --env-file /tmp/claude/discovery.env

  # Run a single test:
  python3 tests/scripts/run_test.py \
    --config tests/config/macos.json \
    --test-id same_monitor_restore_mon0 \
    --env-file /tmp/claude/discovery.env

Exit codes: 0 = all pass, 1 = any fail, 2 = script error
"""

from __future__ import annotations

import argparse
import atexit
import json
import os
import platform
import random
import re
import signal
import subprocess
import sys
import tempfile
import time
import traceback
from pathlib import Path
from typing import Final
from typing import NoReturn
from typing import TypedDict
from typing import cast
from typing import final
from urllib.error import URLError
from urllib.request import Request
from urllib.request import urlopen

from clerestory_test.app_session import AppSession
from clerestory_test.case_result import AssertionResult
from clerestory_test.case_result import Availability
from clerestory_test.case_result import CaseResult
from clerestory_test.case_result import Evidence
from clerestory_test.case_result import Interaction
from clerestory_test.case_result import Outcome

# =============================================================================
# Constants
# =============================================================================

DEFAULT_BRP_PORT: Final = 15702
POLL_INTERVAL: Final = 0.05
MAX_POLLS: Final = 200  # 10s timeout

COMP_WINDOW: Final = "bevy_window::window::Window"
COMP_PRIMARY: Final = "bevy_window::window::PrimaryWindow"
COMP_MANAGED: Final = "hana_clerestory::managed::ManagedWindow"
COMP_CURRENT_MONITOR: Final = "hana_clerestory::monitors::CurrentMonitor"
COMP_LAUNCH_INFO: Final = "hana_clerestory::restore::RestoreDiagnostics"
COMP_TARGET_POSITION: Final = "hana_clerestory::restore::TargetPosition"
COMP_MONITOR: Final = "bevy_window::monitor::Monitor"
COMP_PERSISTENCE: Final = "hana_clerestory::managed::ManagedWindowPersistence"
RES_RESTORED: Final = "restore_window::events::WindowRestoredReceived"
RES_MISMATCH: Final = "restore_window::events::WindowRestoreMismatchReceived"
RES_SETTLED_COUNT: Final = "restore_window::events::WindowsSettledCount"

# JSON dict type alias for BRP responses (deeply nested, untyped JSON)
type JsonDict = dict[str, object]

# =============================================================================
# TypedDicts for JSON config structures
# =============================================================================


class TestRequirements(TypedDict, total=False):
    min_monitors: int
    different_scales: bool


class MutationConfig(TypedDict, total=False):
    target_monitor: str
    position_offset: list[int]
    size: list[int]


class WorkaroundValidation(TypedDict, total=False):
    feature_flag: str
    build_without: str
    build_with: str
    without_behavior: str
    with_behavior: str


class PersistenceValidation(TypedDict, total=False):
    mode: str
    expected_ron_keys: list[str]
    unexpected_ron_keys: list[str]


class WindowConfig(TypedDict, total=False):
    validate: list[str]
    expected_mode: str
    spawn_event: str
    position_readback_offset: list[int]


class _TestEntryRequired(TypedDict):
    id: str
    ron_file: str


class TestEntry(_TestEntryRequired, total=False):
    description: str
    automation: str
    launch_monitor: int
    launch_monitor_role: str
    launch_size: str
    requires: TestRequirements
    mutation: MutationConfig
    workaround_validation: WorkaroundValidation
    workaround_keys: list[str]
    windows: dict[str, WindowConfig]
    persistence_validation: PersistenceValidation
    expected_log_warning: str
    backend: str
    instructions: list[str]
    instructions_without_workaround: list[str]
    instructions_with_workaround: list[str]
    success_criteria: str | dict[str, str]
    success_criteria_without: str
    success_criteria_with: str
    wait_for_restore_event: bool


class PlatformConfig(TypedDict):
    platform: str
    example_ron_path: str
    test_ron_dir: str
    tests: list[TestEntry]


class RonWindowValues(TypedDict, total=False):
    pos_x: str
    pos_y: str
    # "offset"  -> pos_x/pos_y are logical pixels from the monitor's own top-left corner (v3).
    # "desktop" -> pos_x/pos_y are an absolute logical desktop coordinate (v1/v2 templates).
    pos_units: str
    captured_scale: str
    width: str
    height: str
    monitor: str
    mode: str
    mutation_physical_x: str
    mutation_physical_y: str
    mutation_physical_w: str
    mutation_physical_h: str


# =============================================================================
# Typed argparse
# =============================================================================


class Args(argparse.Namespace):
    prebuild: bool = False
    discover: bool = False
    config: str = ""
    test_id: str = ""
    backend: str = "native"
    env_file: str = ""
    executable: str = ""
    base_port: int = DEFAULT_BRP_PORT
    ron_path: str = ""
    artifact_dir: str = ""
    result_json: str = ""


# =============================================================================
# Globals
# =============================================================================

app_process: subprocess.Popen[bytes] | None = None
app_session: AppSession | None = None
pass_count = 0
fail_count = 0
skip_count = 0
assertion_results: list[AssertionResult] = []
result_json_path: Path | None = None
result_case_id = ""
result_artifacts: list[str] = []
result_started = 0.0
configured_executable: Path | None = None
configured_base_port = DEFAULT_BRP_PORT
configured_artifact_directory: Path | None = None
configured_persistence_path: Path | None = None
configured_launch_monitor: int | None = None
configured_launch_position: tuple[int, int] | None = None
configured_launch_size: str | None = None


# =============================================================================
# Output helpers
# =============================================================================


def pass_line(key: str, field: str, details: str) -> None:
    global pass_count
    print(f"PASS {key} {field} {details}")
    pass_count += 1
    assertion_results.append(AssertionResult(f"{key}.{field}", True, details))


def fail_line(key: str, field: str, details: str) -> None:
    global fail_count
    print(f"FAIL {key} {field} {details}")
    fail_count += 1
    assertion_results.append(AssertionResult(f"{key}.{field}", False, details))


def skip_line(key: str, field: str, details: str) -> None:
    global skip_count
    print(f"SKIP {key} {field} {details}")
    skip_count += 1


def note_line(key: str, field: str, details: str) -> None:
    print(f"NOTE {key} {field} {details}")


def die(msg: str) -> NoReturn:
    print(f"ERROR {msg}", file=sys.stderr)
    sys.exit(2)


# =============================================================================
# JSON navigation helpers
# =============================================================================


def json_get(obj: object, *keys: str) -> object:
    """Safely navigate nested JSON dicts."""
    val = obj
    for k in keys:
        if isinstance(val, dict):
            val = cast(JsonDict, val).get(k)
        else:
            return None
    return val


def json_str(val: object) -> str:
    """Convert a JSON value to string for comparison.

    Normalizes numeric types to integers when possible (BRP may return 100.0
    while RON parsing captures "100"). Returns empty string for None.
    """
    if val is None:
        return ""
    if isinstance(val, float):
        int_val = int(val)
        if val == float(int_val):
            return str(int_val)
    return str(val)


def json_list(val: object) -> list[object]:
    """Convert a JSON value to list, returning empty list for None."""
    if isinstance(val, list):
        return cast(list[object], val)
    return []


def json_dict(val: object) -> JsonDict:
    """Convert a JSON value to dict, returning empty dict for None."""
    if isinstance(val, dict):
        return cast(JsonDict, val)
    return {}


# =============================================================================
# BRP Client
# =============================================================================


@final
class BrpClient:
    def __init__(self, port: int = DEFAULT_BRP_PORT) -> None:
        self.url = f"http://127.0.0.1:{port}/jsonrpc"

    def call(self, method: str, params: object = None) -> JsonDict:
        payload = {
            "jsonrpc": "2.0",
            "method": method,
            "id": 1,
            "params": params,
        }
        data = json.dumps(payload).encode()
        req = Request(self.url, data=data, headers={"Content-Type": "application/json"})
        resp = urlopen(req, timeout=10)  # pyright: ignore[reportAny]
        raw: object = json.loads(resp.read())  # pyright: ignore[reportAny]
        resp.close()  # pyright: ignore[reportAny]
        return json_dict(raw)

    def wait_ready(self) -> None:
        for _ in range(MAX_POLLS):
            try:
                _ = self.call("rpc.discover")
                return
            except (URLError, OSError, json.JSONDecodeError):
                time.sleep(POLL_INTERVAL)
        die("BRP did not become ready within timeout")

    def query_primary(self) -> JsonDict:
        return self.call("world.query", {
            "data": {
                "components": [COMP_WINDOW, COMP_CURRENT_MONITOR],
                "option": [COMP_LAUNCH_INFO],
            },
            "filter": {"with": [COMP_PRIMARY]},
        })

    def query_managed(self) -> JsonDict:
        return self.call("world.query", {
            "data": {"components": [COMP_WINDOW, COMP_MANAGED, COMP_CURRENT_MONITOR]},
            "filter": {"with": [COMP_MANAGED], "without": [COMP_PRIMARY]},
        })

    def shutdown(self) -> None:
        try:
            _ = self.call("clerestory/shutdown")
        except (URLError, OSError):
            pass


brp = BrpClient()


# =============================================================================
# RON operations
# =============================================================================


def substitute_ron(template: str, env_vars: dict[str, str]) -> str:
    def _replacer(match: re.Match[str]) -> str:
        expr = match.group(1)
        # Support ${VAR+N} and ${VAR-N} arithmetic
        arith = re.match(r"^(\w+)([+-])(\d+)$", expr)
        if arith:
            var_name = arith.group(1)
            op = arith.group(2)
            offset = int(arith.group(3))
            val = env_vars.get(var_name)
            if val is not None:
                result = int(val) + offset if op == "+" else int(val) - offset
                return str(result)
            return match.group(0)
        val = env_vars.get(expr)
        if val is not None:
            return val
        return match.group(0)
    return re.sub(r"\$\{([^}]+)\}", _replacer, template)


def parse_ron_values(content: str) -> dict[str, RonWindowValues]:
    """Parse RON file into per-window-key dicts of expected values."""
    result: dict[str, RonWindowValues] = {}
    current_key = ""

    re_managed = re.compile(r'key: Managed\("([^"]+)"\)')
    # v3: an offset from the window's own monitor. Scale-independent.
    re_offset = re.compile(r"position: MonitorOffset\(\((-?\d+),\s*(-?\d+)\)\)")
    # v3: a pre-v3 coordinate carried forward with the scale that wrote it.
    re_unrebased = re.compile(
        r"position: Unrebased\(\(logical: \((-?\d+),\s*(-?\d+)\),\s*captured_scale: ([0-9.]+)\)\)"
    )
    re_unpositioned = re.compile(r"position: Unpositioned")
    # v1/v2: an absolute logical desktop coordinate.
    re_position = re.compile(r"logical_position: Some\(\((-?\d+),\s*(-?\d+)\)\)")
    re_position_none = re.compile(r"logical_position: None")
    re_captured_scale = re.compile(r"monitor_scale: ([0-9.]+)")
    re_width = re.compile(r"logical_width: (\d+)")
    re_height = re.compile(r"logical_height: (\d+)")
    re_monitor = re.compile(r"monitor_index: (\d+)")
    re_mode = re.compile(r"mode: (Windowed|BorderlessFullscreen|Fullscreen)")

    for line in content.splitlines():
        if "key: Primary" in line:
            current_key = "primary"
            if current_key not in result:
                result[current_key] = RonWindowValues()
        else:
            m = re_managed.search(line)
            if m:
                current_key = m.group(1)
                if current_key not in result:
                    result[current_key] = RonWindowValues()
                continue

        if not current_key:
            continue

        entry = result[current_key]

        m = re_offset.search(line)
        if m:
            entry["pos_x"] = m.group(1)
            entry["pos_y"] = m.group(2)
            entry["pos_units"] = "offset"
            continue

        m = re_unrebased.search(line)
        if m:
            entry["pos_x"] = m.group(1)
            entry["pos_y"] = m.group(2)
            entry["captured_scale"] = m.group(3)
            entry["pos_units"] = "desktop"
            continue

        if re_unpositioned.search(line):
            entry["pos_x"] = ""
            entry["pos_y"] = ""
            entry["pos_units"] = "offset"
            continue

        m = re_position.search(line)
        if m:
            entry["pos_x"] = m.group(1)
            entry["pos_y"] = m.group(2)
            entry["pos_units"] = "desktop"
            continue

        if re_position_none.search(line):
            entry["pos_x"] = ""
            entry["pos_y"] = ""
            entry["pos_units"] = "desktop"
            continue

        m = re_captured_scale.search(line)
        if m:
            entry["captured_scale"] = m.group(1)
            continue

        m = re_width.search(line)
        if m:
            entry["width"] = m.group(1)
            continue

        m = re_height.search(line)
        if m:
            entry["height"] = m.group(1)
            continue

        m = re_monitor.search(line)
        if m:
            entry["monitor"] = m.group(1)
            continue

        m = re_mode.search(line)
        if m:
            entry["mode"] = m.group(1)
            continue

    return result


# =============================================================================
# Env file I/O
# =============================================================================


def load_env_file(path: str) -> dict[str, str]:
    """Load 'export K=V' lines into a dict."""
    env: dict[str, str] = {}
    if not os.path.isfile(path):
        return env
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line.startswith("#") or not line:
                continue
            if line.startswith("export "):
                line = line[7:]
            eq = line.find("=")
            if eq > 0:
                env[line[:eq]] = line[eq + 1:]
    return env


def write_env_file(path: str, env: dict[str, str]) -> None:
    with open(path, "w") as f:
        for key, val in env.items():
            _ = f.write(f"export {key}={val}\n")


# =============================================================================
# Process management
# =============================================================================


def _stderr_has_panic(path: str) -> bool:
    """Check if a stderr log file contains a Rust panic."""
    try:
        with open(path) as f:
            content = f.read()
        return "panicked at" in content
    except OSError:
        return False


def cleanup() -> None:
    global app_process
    global app_session
    if app_session is not None:
        _ = app_session.stop(timeout_seconds=0)
    app_session = None
    app_process = None


_ = atexit.register(cleanup)


def _handle_signal(_signum: int, _frame: object) -> None:
    cleanup()
    sys.exit(2)


_ = signal.signal(signal.SIGTERM, _handle_signal)
_ = signal.signal(signal.SIGINT, _handle_signal)


def wait_for_restore() -> str:
    """Poll for `WindowRestoredReceived` or `WindowRestoreMismatchReceived`.

    Returns "restored", "mismatch", or "crashed" depending on outcome.
    """
    for _ in range(MAX_POLLS):
        if app_process is not None and app_process.poll() is not None:
            return "crashed"
        try:
            result = brp.call("world.get_resources", {"resource": RES_RESTORED})
            value = json_get(result, "result", "value")
            if value is not None:
                return "restored"
        except (URLError, OSError):
            if app_process is not None and app_process.poll() is not None:
                return "crashed"
        try:
            result = brp.call("world.get_resources", {"resource": RES_MISMATCH})
            value = json_get(result, "result", "value")
            if value is not None:
                return "mismatch"
        except (URLError, OSError):
            if app_process is not None and app_process.poll() is not None:
                return "crashed"
        time.sleep(POLL_INTERVAL)
    die("Neither WindowRestoredReceived nor WindowRestoreMismatchReceived found within timeout")


def launch_app(
    backend: str = "native",
    wait_restore: bool = True,
    test_mode: bool = True,
) -> tuple[str | None, str]:
    """Launch the restore_window example. Returns (stderr_path, restore_result)."""
    global app_process
    global app_session

    if configured_executable is None:
        die("No prebuilt restore_window executable was provided")

    env = dict(os.environ)
    if test_mode:
        env["CLERESTORY_TEST_MODE"] = "1"
    if backend == "x11":
        env["WAYLAND_DISPLAY"] = ""
    env["BRP_EXTRAS_PORT"] = str(configured_base_port)
    env["CLERESTORY_TEST_RENDER_PORT"] = str(configured_base_port + 1)
    if configured_persistence_path is not None:
        env["CLERESTORY_TEST_PERSISTENCE_PATH"] = str(configured_persistence_path)
    if configured_launch_monitor is not None:
        env["CLERESTORY_TEST_LAUNCH_MONITOR"] = str(configured_launch_monitor)
    if configured_launch_position is not None:
        env["CLERESTORY_TEST_LAUNCH_POSITION"] = (
            f"{configured_launch_position[0]},{configured_launch_position[1]}"
        )
    if configured_launch_size is not None:
        env["CLERESTORY_TEST_LAUNCH_SIZE"] = configured_launch_size

    artifact_directory = configured_artifact_directory
    if artifact_directory is None:
        artifact_directory = Path(tempfile.mkdtemp(prefix="clerestory-test-"))
    artifact_directory.mkdir(parents=True, exist_ok=True)
    launch_number = sum(
        1 for path in artifact_directory.glob("app-*.stderr.log") if path.is_file()
    ) + 1
    stdout_path = artifact_directory / f"app-{launch_number}.stdout.log"
    stderr_path = artifact_directory / f"app-{launch_number}.stderr.log"
    result_artifacts.extend([str(stdout_path), str(stderr_path)])
    app_session = AppSession(
        executable=configured_executable,
        argv=[],
        environment=env,
        stdout_path=stdout_path,
        stderr_path=stderr_path,
        working_directory=Path.cwd(),
    )
    app_session.start()
    app_process = app_session.process
    print(f"# App stdout: {stdout_path}", flush=True)
    print(f"# App stderr: {stderr_path}", flush=True)

    brp.wait_ready()
    restore_result = ""
    if wait_restore:
        restore_result = wait_for_restore()
    return str(stderr_path), restore_result


def shutdown_app() -> None:
    """Graceful BRP shutdown + wait + force kill."""
    global app_process
    global app_session
    if app_session is not None:
        _ = app_session.stop(graceful_shutdown=brp.shutdown)
    app_session = None
    app_process = None


# =============================================================================
# Entity extraction helpers
# =============================================================================


def extract_from_entity(entity: JsonDict, component: str, *path: str) -> object:
    """Navigate nested BRP JSON: entity['components'][component][path[0]][...]"""
    val = json_get(entity, "components", component)
    for p in path:
        if isinstance(val, dict):
            val = cast(JsonDict, val).get(p)
        else:
            return None
    return val


def normalize_mode(mode_raw: object) -> str:
    if isinstance(mode_raw, str):
        return mode_raw
    if isinstance(mode_raw, dict):
        d = cast(JsonDict, mode_raw)
        if "BorderlessFullscreen" in d:
            return "BorderlessFullscreen"
        if "Fullscreen" in d:
            return "Fullscreen"
    return str(cast(object, mode_raw))


def extract_position_at(entity: JsonDict, component: str) -> tuple[str, str]:
    """Extract position.At[x, y] from an entity, returning ("", "") if unavailable."""
    pos = extract_from_entity(entity, component, "position")
    if isinstance(pos, dict):
        d = cast(JsonDict, pos)
        at = json_list(d.get("At"))
        if len(at) >= 2:
            return (json_str(at[0]), json_str(at[1]))
    return ("", "")


# =============================================================================
# Validation helpers
# =============================================================================


def _current_monitor_index(entity: JsonDict) -> str:
    """Read the current-enumeration adapter index from reflected monitor metadata."""
    reflected = extract_from_entity(
        entity,
        COMP_CURRENT_MONITOR,
        "monitor_info",
        "descriptor",
        "index",
    )
    tuple_values = json_list(reflected)
    if tuple_values:
        return json_str(tuple_values[0])
    if isinstance(reflected, dict):
        return json_str(cast(JsonDict, reflected).get("0"))
    return json_str(reflected)


def _monitor_scale(entity: JsonDict, env_vars: dict[str, str]) -> float:
    """Get the scale factor for the monitor the window is currently on.

    Uses the actual monitor index from BRP + discovery env vars, which is
    always correct.  Falls back to the window's reported scale_factor when
    env vars aren't available (e.g. no discovery env file).
    """
    idx = _current_monitor_index(entity)
    if idx and f"MONITOR_{idx}_SCALE" in env_vars:
        return float(env_vars[f"MONITOR_{idx}_SCALE"])
    return float(json_str(extract_from_entity(entity, COMP_WINDOW, "resolution", "scale_factor")))


def _rust_round(x: float) -> int:
    """Round like Rust's f64::round() — half away from zero (not banker's rounding)."""
    if x >= 0:
        return int(x + 0.5)
    return -int(-x + 0.5)


def _logical_to_physical_pos(logical: int, scale: float) -> int:
    """Convert logical position to physical. Matches Rust: (logical * scale).round() as i32."""
    return _rust_round(float(logical) * scale)


def _physical_to_logical_pos(physical: int, scale: float) -> int:
    """Convert physical position to logical. Matches Rust: (physical / scale).round() as i32."""
    return _rust_round(float(physical) / scale)


def _monitor_origin(entity: JsonDict, env_vars: dict[str, str]) -> tuple[int, int] | None:
    """Physical top-left corner of the monitor the window is currently on.

    Returns None when discovery env vars are unavailable, in which case the caller falls back to
    treating saved values as absolute desktop coordinates.
    """
    idx = _current_monitor_index(entity)
    if not idx or f"MONITOR_{idx}_POS_X" not in env_vars:
        return None
    return int(env_vars[f"MONITOR_{idx}_POS_X"]), int(env_vars[f"MONITOR_{idx}_POS_Y"])


def _physical_to_monitor_offset(physical: int, origin: int, scale: float) -> int:
    """Invert the production placement: physical -> logical offset from the monitor corner.

    Production computes `origin + round(offset * scale)`. Recomputing that here would mean a
    formula error cancels on both sides and the assertion passes anyway, which is how the
    original defect survived the suite. Inverting instead compares against the number actually
    written to the file. The inversion is exact for every scale >= 1: the residue from
    production's rounding is at most 0.5 physical pixels, so dividing by the scale leaves less
    than half a logical pixel, and at scale 1.0 there is no residue at all.
    """
    return _rust_round(float(physical - origin) / scale)


def _logical_to_physical_size(logical: int, scale: float) -> int:
    """Convert logical size to physical. Matches Rust: (logical * scale) as u32 (truncation)."""
    return int(float(logical) * scale)


def _physical_to_logical_size(physical: int, scale: float) -> int:
    """Convert physical size to logical. Matches Bevy: (physical as f32 / scale) as u32 (truncation)."""
    return int(float(physical) / scale)


def _check_field(
    key: str,
    field: str,
    expected: str,
    actual: str,
    prefix: str = "",
) -> None:
    """Compare expected vs actual and emit PASS/FAIL line."""
    if actual == expected:
        pass_line(key, f"{prefix}{field}", f"expected={expected} actual={actual}")
    else:
        fail_line(key, f"{prefix}{field}", f"expected={expected} actual={actual}")


def _check_field_pair(
    key: str,
    field: str,
    exp_a: str,
    exp_b: str,
    act_a: str,
    act_b: str,
    prefix: str = "",
) -> None:
    """Compare a pair of values (position x,y or size w,h) and emit PASS/FAIL."""
    if act_a == exp_a and act_b == exp_b:
        pass_line(key, f"{prefix}{field}", f"expected=[{exp_a},{exp_b}] actual=[{act_a},{act_b}]")
    else:
        fail_line(key, f"{prefix}{field}", f"expected=[{exp_a},{exp_b}] actual=[{act_a},{act_b}]")


def _check_persistence_key(key: str, ron_content: str, expect_present: bool) -> None:
    """Check if a window key is present/absent in RON content."""
    marker = "key: Primary" if key == "primary" else f'key: Managed("{key}")'
    is_present = marker in ron_content

    if expect_present:
        if is_present:
            pass_line("persistence", f"key={key}", "present")
        else:
            fail_line("persistence", f"key={key}", "missing")
    else:
        if is_present:
            fail_line("persistence", f"key={key}", "should_be_absent")
        else:
            pass_line("persistence", f"key={key}", "absent")


def _check_position_saved(key: str, ron_content: str) -> None:
    """Check that a window's saved position is Some(...), not None."""
    parsed = parse_ron_values(ron_content)
    entry = parsed.get(key)
    if entry is None:
        fail_line("persistence", f"position_saved={key}", "window_key_not_found")
        return
    pos_x = entry.get("pos_x", "")
    if pos_x:
        pass_line("persistence", f"position_saved={key}", f"position=({pos_x},{entry.get('pos_y', '')})")
    else:
        fail_line("persistence", f"position_saved={key}", "position=None")


# =============================================================================
# Validation (single function, handles initial / mutation / relaunch)
# =============================================================================


def validate_window(
    key: str,
    validate_fields: list[str],
    entity: JsonDict,
    ron_values: RonWindowValues,
    prefix: str = "",
    expected_mode_override: str = "",
    backend: str = "native",
    env_vars: dict[str, str] | None = None,
    position_readback_offset: list[int] | None = None,
) -> None:
    for field in validate_fields:
        if field == "position":
            if backend == "wayland":
                continue
            _check_position(
                key, entity, ron_values, prefix, env_vars or {}, position_readback_offset
            )

        elif field == "size":
            actual_w = json_str(extract_from_entity(entity, COMP_WINDOW, "resolution", "physical_width"))
            actual_h = json_str(extract_from_entity(entity, COMP_WINDOW, "resolution", "physical_height"))
            scale = _monitor_scale(entity, env_vars or {})
            logical_w = ron_values.get("width", "")
            logical_h = ron_values.get("height", "")
            exp_w = str(_logical_to_physical_size(int(logical_w), scale)) if logical_w else ""
            exp_h = str(_logical_to_physical_size(int(logical_h), scale)) if logical_h else ""
            _check_field_pair(key, "size", exp_w, exp_h, actual_w, actual_h, prefix)

        elif field == "mode":
            mode_raw = extract_from_entity(entity, COMP_WINDOW, "mode")
            actual_mode = normalize_mode(mode_raw)
            exp_mode = expected_mode_override if expected_mode_override else ron_values.get("mode", "")
            _check_field(key, "mode", exp_mode, actual_mode, prefix)

        elif field == "monitor_index":
            actual_idx = _current_monitor_index(entity)
            exp_idx = ron_values.get("monitor", "")
            _check_field(key, "monitor_index", exp_idx, actual_idx, prefix)

        elif field == "exit_code":
            pass  # handled separately


def _check_position(
    key: str,
    entity: JsonDict,
    ron_values: RonWindowValues,
    prefix: str,
    env_vars: dict[str, str],
    position_readback_offset: list[int] | None,
) -> None:
    """Compare the window's live position against the saved one, in the saved value's own units.

    A v3 offset is compared as an offset — the live physical position is converted back to a
    monitor-relative logical offset and matched against the number in the file. A v1/v2 absolute
    coordinate is compared the legacy way, because that is what those templates still mean.
    """
    actual_x, actual_y = extract_position_at(entity, COMP_WINDOW)
    saved_x = ron_values.get("pos_x", "")
    saved_y = ron_values.get("pos_y", "")
    if not saved_x:
        _check_field_pair(key, "position", "", "", actual_x, actual_y, prefix)
        return

    scale = _monitor_scale(entity, env_vars)
    if position_readback_offset:
        saved_x = str(int(saved_x) + position_readback_offset[0])
        saved_y = str(int(saved_y) + position_readback_offset[1])

    origin = _monitor_origin(entity, env_vars)
    if origin is None:
        # No discovery env: fall back to the pre-v3 comparison. Only reachable when running
        # without a hardware profile, where position assertions are already approximate.
        exp_x = str(_logical_to_physical_pos(int(saved_x), scale))
        exp_y = str(_logical_to_physical_pos(int(saved_y), scale))
        _check_field_pair(key, "position", exp_x, exp_y, actual_x, actual_y, prefix)
        return

    if ron_values.get("pos_units", "desktop") == "offset":
        expected_offset_x, expected_offset_y = int(saved_x), int(saved_y)
    else:
        # A v1/v2 seed holds an absolute desktop coordinate. Convert it to an offset the same way
        # the app's migration does — subtract the monitor's logical origin *at the scale that
        # wrote the file*, not at the live scale. Comparing the absolute coordinate directly would
        # reintroduce the very dependence on the desktop layout that v3 removes, and would be off
        # by a pixel whenever the monitor's physical origin is odd.
        captured_scale = float(ron_values.get("captured_scale", "") or scale)
        expected_offset_x = int(saved_x) - _rust_round(origin[0] / captured_scale)
        expected_offset_y = int(saved_y) - _rust_round(origin[1] / captured_scale)

    actual_offset_x = _physical_to_monitor_offset(int(actual_x), origin[0], scale)
    actual_offset_y = _physical_to_monitor_offset(int(actual_y), origin[1], scale)
    _check_field_pair(
        key,
        "position",
        str(expected_offset_x),
        str(expected_offset_y),
        str(actual_offset_x),
        str(actual_offset_y),
        prefix,
    )


def _strategy_name(raw: object) -> str:
    """Variant name of a serialized MonitorScaleStrategy.

    Unit variants serialize as a bare string ("ApplyUnchanged", "LowerToHigher");
    data-carrying variants as a single-key object ({"HigherToLower": "NeedInitialMove"}).
    """
    if isinstance(raw, str):
        return raw
    if isinstance(raw, dict):
        keys = list(cast(JsonDict, raw).keys())
        if keys:
            return keys[0]
    return "unknown"


def resolve_launch_monitor(test: TestEntry, env_vars: dict[str, str]) -> int | None:
    """Launch monitor index for a case, by scale ROLE when set, else fixed slot.

    A case may pin its launch monitor to a scale role instead of a numeric slot by
    setting "launch_monitor_role" to "high" or "low". The role resolves against the
    HIGH_SCALE_MONITOR_INDEX / LOW_SCALE_MONITOR_INDEX exported by discovery, so the
    differently-scaled monitor can land on any index. Cases without a role fall back
    to the numeric "launch_monitor" (unchanged for every non-cross-DPI case).
    """
    role = test.get("launch_monitor_role")
    if role is not None:
        key = "HIGH_SCALE_MONITOR_INDEX" if role == "high" else "LOW_SCALE_MONITOR_INDEX"
        value = env_vars.get(key)
        return int(value) if value is not None else None
    return test.get("launch_monitor")


def validate_launch_monitor(
    test: TestEntry, entity: JsonDict, backend: str, env_vars: dict[str, str]
) -> None:
    """Verify the window launched on the test's launch_monitor and, for
    different-scale tests, genuinely exercised a cross-DPI strategy.

    The launch monitor is environmental on macOS (the OS picks the spawn
    display), so a cross-DPI test can silently spawn on the target monitor —
    a same-scale restore that passes position/size/monitor checks without ever
    running the cross-DPI path. RestoreDiagnostics records the launch monitor
    and chosen strategy durably so the precondition can be asserted instead of
    assumed.

    On Wayland the launch monitor is undetectable: the window is created hidden
    (never mapped to an output during init_winit_info) and Wayland has no
    outer_position(), so starting_monitor_index always collapses to the monitor
    at the origin. The precondition is also unnecessary there — Wayland tests
    restore size + monitor_index only (no cross-DPI position path) — so skip it.

    The launch monitor only matters for cross-DPI (different_scales) tests: there
    it proves the cross-DPI path genuinely ran instead of a hollow same-scale
    restore on the target monitor. Every other test validates the restore itself,
    not where the window manager chose to open the app (KDE opens on the X11
    primary regardless of terminal focus), so for those it is informational only
    and never fails.
    """
    if backend == "wayland":
        note_line(
            "primary",
            "launch_monitor",
            "undetectable on Wayland (hidden surface, no outer_position)",
        )
        return

    diagnostics = extract_from_entity(entity, COMP_LAUNCH_INFO)
    if not isinstance(diagnostics, dict):
        fail_line("primary", "launch_monitor", "RestoreDiagnostics missing (restore did not run?)")
        return
    diagnostics = cast(JsonDict, diagnostics)

    expected_launch = resolve_launch_monitor(test, env_vars)
    if expected_launch is None:
        expected_launch = 0
    actual_launch = diagnostics.get("starting_monitor_index")
    strategy = _strategy_name(diagnostics.get("monitor_scale_strategy"))
    starting_scale = diagnostics.get("starting_scale")
    target_scale = diagnostics.get("target_scale")
    detail = f"strategy={strategy} starting_scale={starting_scale} target_scale={target_scale}"

    requires = test.get("requires", {})
    if not requires.get("different_scales"):
        if actual_launch == expected_launch:
            pass_line("primary", "launch_monitor", f"expected={expected_launch} actual={actual_launch} ({detail})")
        else:
            note_line(
                "primary",
                "launch_monitor",
                f"expected={expected_launch} actual={actual_launch} — WM-chosen spawn monitor, not asserted for non-cross-DPI test ({detail})",
            )
        return

    if actual_launch != expected_launch:
        reason = "launch precondition not met; re-run (macOS spawn monitor is environmental)"
        fail_line(
            "primary",
            "launch_monitor",
            f"expected={expected_launch} actual={actual_launch} ({detail}) — {reason}",
        )
        return
    pass_line("primary", "launch_monitor", f"expected={expected_launch} actual={actual_launch} ({detail})")

    if strategy == "ApplyUnchanged":
        fail_line(
            "primary",
            "strategy",
            f"cross-DPI test but strategy=ApplyUnchanged — cross-DPI path not exercised ({detail})",
        )


# =============================================================================
# Window resolution (primary vs managed with spawn polling)
# =============================================================================


def resolve_primary_entity() -> JsonDict | None:
    result = brp.query_primary()
    entities = json_list(result.get("result"))
    if entities:
        return json_dict(entities[0])
    return None


def wait_for_all_windows_settled(expected_count: int) -> None:
    """Poll until WindowsSettledCount reaches expected_count."""
    for _ in range(MAX_POLLS):
        try:
            result = brp.call("world.get_resources", {"resource": RES_SETTLED_COUNT})
            value = json_get(result, "result", "value")
            if isinstance(value, dict):
                count = cast(JsonDict, value).get("value", 0)
                if isinstance(count, int) and count >= expected_count:
                    return
        except (URLError, OSError):
            pass
        time.sleep(POLL_INTERVAL)
    die(f"WindowsSettledCount did not reach {expected_count} within timeout")


def spawn_and_poll_managed(
    spawn_event: str,
    expected_total_windows: int,
) -> list[JsonDict]:
    _ = brp.call("world.trigger_event", {"event": spawn_event, "value": None})

    # Wait for all windows (primary + managed) to settle
    wait_for_all_windows_settled(expected_total_windows)

    # Now query managed entities
    for _ in range(20):
        time.sleep(0.2)
        try:
            result = brp.query_managed()
            raw_entities = json_list(result.get("result"))
            entities = [json_dict(e) for e in raw_entities]
            if entities:
                return entities
        except (URLError, OSError):
            pass

    die("Managed window not found after settling")


def get_managed_by_name(entities: list[JsonDict], window_name: str) -> JsonDict | None:
    for ent in entities:
        managed = extract_from_entity(ent, COMP_MANAGED)
        if isinstance(managed, dict):
            d = cast(JsonDict, managed)
            if d.get("name") == window_name:
                return ent
    return None


# =============================================================================
# Validate all windows in a test
# =============================================================================


def validate_all_windows(
    windows: dict[str, WindowConfig],
    ron_values: dict[str, RonWindowValues],
    prefix: str = "",
    backend: str = "native",
    env_vars: dict[str, str] | None = None,
) -> None:
    triggered_events: set[str] = set()
    managed_entities: list[JsonDict] = []
    total_window_count = len(windows)

    for wkey, wconfig in windows.items():
        validate_fields = wconfig.get("validate", [])
        expected_mode_override = wconfig.get("expected_mode", "")
        spawn_event = wconfig.get("spawn_event", "")
        position_readback_offset = wconfig.get("position_readback_offset")

        entity: JsonDict | None = None

        if wkey == "primary":
            entity = resolve_primary_entity()
        else:
            if spawn_event and spawn_event not in triggered_events:
                managed_entities = spawn_and_poll_managed(
                    spawn_event,
                    expected_total_windows=total_window_count,
                )
                triggered_events.add(spawn_event)

            if managed_entities:
                entity = get_managed_by_name(managed_entities, wkey)

        if entity is None:
            fail_line(wkey, f"{prefix}query", "window not found")
            continue

        wkey_values = ron_values.get(wkey, RonWindowValues())
        validate_window(
            wkey,
            validate_fields,
            entity,
            wkey_values,
            prefix=prefix,
            expected_mode_override=expected_mode_override,
            backend=backend,
            env_vars=env_vars,
            position_readback_offset=position_readback_offset,
        )


# =============================================================================
# Mutations
# =============================================================================


def apply_mutations(
    test: TestEntry,
    entity: JsonDict,
    ron_values: dict[str, RonWindowValues],
    env_vars: dict[str, str] | None = None,
) -> None:
    entity_id = entity.get("entity")
    mutation = test.get("mutation", {})
    scale = _monitor_scale(entity, env_vars or {})

    if "position_offset" in mutation:
        offset = mutation["position_offset"]
        primary_vals = ron_values.get("primary", RonWindowValues())
        logical_x = int(primary_vals.get("pos_x", "0") or "0")
        logical_y = int(primary_vals.get("pos_y", "0") or "0")
        # A v3 saved value is measured from the monitor's corner, so the physical target has to
        # be anchored there too. Reading it as an absolute desktop coordinate would silently move
        # the window to whatever monitor holds that coordinate — on monitor 0 at origin (0,0) the
        # two agree, which is why this reads as harmless until a test runs on monitor 1.
        monitor_origin = _monitor_origin(entity, env_vars or {})
        if primary_vals.get("pos_units", "desktop") == "offset" and monitor_origin is not None:
            anchor_x, anchor_y = monitor_origin
        else:
            anchor_x, anchor_y = 0, 0
        physical_x = anchor_x + _logical_to_physical_pos(logical_x, scale) + offset[0]
        physical_y = anchor_y + _logical_to_physical_pos(logical_y, scale) + offset[1]
        # From here on the value is whatever the app will write, and the app only writes v3
        # offsets — the seed's format stops mattering the moment it has been read.
        store_x, store_y = monitor_origin if monitor_origin is not None else (0, 0)
        primary_vals["pos_units"] = "offset" if monitor_origin is not None else "desktop"

        _ = brp.call("world.mutate_components", {
            "entity": entity_id,
            "component": COMP_WINDOW,
            "path": ".position",
            "value": {"At": [physical_x, physical_y]},
        })

        # Store the moved-to value back in the units the file uses, so the relaunch comparison
        # and the saved entry stay in the same coordinate space.
        primary_vals["pos_x"] = str(_physical_to_monitor_offset(physical_x, store_x, scale))
        primary_vals["pos_y"] = str(_physical_to_monitor_offset(physical_y, store_y, scale))
        # Store physical values for immediate mutation verification
        primary_vals["mutation_physical_x"] = str(physical_x)
        primary_vals["mutation_physical_y"] = str(physical_y)

    if "size" in mutation:
        new_w = mutation["size"][0]
        new_h = mutation["size"][1]

        _ = brp.call("world.mutate_components", {
            "entity": entity_id,
            "component": COMP_WINDOW,
            "path": ".resolution",
            "value": {
                "physical_width": new_w,
                "physical_height": new_h,
                "scale_factor": scale,
                "scale_factor_override": None,
            },
        })

        primary_vals = ron_values.get("primary", RonWindowValues())
        primary_vals["width"] = str(_physical_to_logical_size(new_w, scale))
        primary_vals["height"] = str(_physical_to_logical_size(new_h, scale))
        # Store physical values for immediate mutation verification
        primary_vals["mutation_physical_w"] = str(new_w)
        primary_vals["mutation_physical_h"] = str(new_h)


def verify_mutations(
    ron_values: dict[str, RonWindowValues],
    backend: str = "native",
    env_vars: dict[str, str] | None = None,
) -> None:
    primary_entity = resolve_primary_entity()
    if primary_entity is None:
        fail_line("primary", "mutation_query", "primary window not found")
        return

    primary_vals = ron_values.get("primary", RonWindowValues())

    # Verify size (ron_values stores logical, BRP reports physical)
    actual_w = json_str(extract_from_entity(primary_entity, COMP_WINDOW, "resolution", "physical_width"))
    actual_h = json_str(extract_from_entity(primary_entity, COMP_WINDOW, "resolution", "physical_height"))
    scale = _monitor_scale(primary_entity, env_vars or {})
    # Use stored physical values if available (avoids round-trip rounding errors)
    exp_w = primary_vals.get("mutation_physical_w", "")
    exp_h = primary_vals.get("mutation_physical_h", "")
    if not exp_w:
        logical_w = primary_vals.get("width", "")
        logical_h = primary_vals.get("height", "")
        exp_w = str(_logical_to_physical_size(int(logical_w), scale)) if logical_w else ""
        exp_h = str(_logical_to_physical_size(int(logical_h), scale)) if logical_h else ""
    _check_field_pair("primary", "mutation_size", exp_w, exp_h, actual_w, actual_h)

    # Verify position if set
    # On X11, Window.position readback is offset by the title bar height due to
    # winit #4445 — outer_position() returns client area, not frame position.
    # We update ron_values with the actual readback so the relaunch comparison
    # checks position stability (no drift) rather than exact match against the
    # value we set.
    logical_px = primary_vals.get("pos_x", "")
    if logical_px:
        actual_x, actual_y = extract_position_at(primary_entity, COMP_WINDOW)
        # Use stored physical values if available (avoids round-trip rounding errors)
        exp_px = primary_vals.get("mutation_physical_x", "")
        exp_py = primary_vals.get("mutation_physical_y", "")
        readback_origin = _monitor_origin(primary_entity, env_vars or {})
        if readback_origin is not None:
            readback_anchor_x, readback_anchor_y = readback_origin
            primary_vals["pos_units"] = "offset"
        else:
            readback_anchor_x, readback_anchor_y = 0, 0
        if not exp_px:
            logical_py = primary_vals.get("pos_y", "")
            exp_px = (
                str(readback_anchor_x + _logical_to_physical_pos(int(logical_px), scale))
                if logical_px
                else ""
            )
            exp_py = (
                str(readback_anchor_y + _logical_to_physical_pos(int(logical_py), scale))
                if logical_py
                else ""
            )
        if backend == "x11":
            # W6 bug: readback is offset by title bar — just record actual for
            # relaunch stability check, don't fail on mutation readback
            pass_line("primary", "mutation_position",
                      f"x11_readback=[{actual_x},{actual_y}] (W6 offset expected)")
        else:
            _check_field_pair("primary", "mutation_position", exp_px, exp_py, actual_x, actual_y)
        # Update expected values to the actual readback, in the file's own units, so the relaunch
        # comparison checks stability rather than exact match against what we set.
        primary_vals["pos_x"] = (
            str(_physical_to_monitor_offset(int(actual_x), readback_anchor_x, scale))
            if actual_x
            else ""
        )
        primary_vals["pos_y"] = (
            str(_physical_to_monitor_offset(int(actual_y), readback_anchor_y, scale))
            if actual_y
            else ""
        )


# =============================================================================
# Persistence validation
# =============================================================================


def validate_persistence(test: TestEntry, ron_path: str) -> None:
    persistence = test.get("persistence_validation", {})
    ron_content = Path(ron_path).read_text()

    for key in persistence.get("expected_ron_keys", []):
        _check_persistence_key(key, ron_content, expect_present=True)

    for key in persistence.get("unexpected_ron_keys", []):
        _check_persistence_key(key, ron_content, expect_present=False)

    for key in cast(list[str], persistence.get("expect_position_saved", [])):
        _check_position_saved(key, ron_content)


# =============================================================================
# RON template write
# =============================================================================


def write_ron(
    ron_file: str,
    ron_dir: str,
    ron_path: str,
    env_vars: dict[str, str],
) -> None:
    template_path = os.path.join(ron_dir, ron_file)
    if not os.path.isfile(template_path):
        die(f"RON template not found: {template_path}")

    template = Path(template_path).read_text()
    substituted = substitute_ron(template, env_vars)
    os.makedirs(os.path.dirname(ron_path), exist_ok=True)
    _ = Path(ron_path).write_text(substituted)


# =============================================================================
# Shared test setup
# =============================================================================


def find_test(config: PlatformConfig, test_id: str) -> TestEntry:
    """Find a test entry by ID, or die if not found."""
    for t in config["tests"]:
        if t["id"] == test_id:
            return t
    die(f"Test '{test_id}' not found in config")


def resolve_backend(backend: str, test: TestEntry) -> str:
    """Override backend from test config if needed."""
    test_backend = test.get("backend", "native")
    if backend == "native" and test_backend and test_backend != "native":
        return test_backend
    return backend


def _swap_x11_env_vars(env_vars: dict[str, str]) -> None:
    """Swap MONITOR_* env vars with their X11 counterparts for X11 tests.

    Replaces POS, LOGICAL_POS, and SCALE with X11-discovered values so that
    RON template substitution and validation use X11 coordinates.
    """
    suffixes = ["_POS_X", "_POS_Y", "_LOGICAL_POS_X", "_LOGICAL_POS_Y", "_SCALE"]
    i = 0
    while f"MONITOR_{i}_SCALE" in env_vars:
        for suffix in suffixes:
            x11_key = f"MONITOR_{i}_X11{suffix}"
            base_key = f"MONITOR_{i}{suffix}"
            if x11_key in env_vars:
                env_vars[base_key] = env_vars[x11_key]
        i += 1


def setup_test(
    config: PlatformConfig,
    test_id: str,
    ron_dir: str,
    ron_path: str,
    env_file: str,
    backend: str,
) -> tuple[TestEntry, str, dict[str, str]]:
    """Prepare one owned restore-window test run.

    Returns (test, resolved_backend, env_vars).
    """
    global configured_launch_monitor
    global configured_launch_position
    global configured_launch_size
    test = find_test(config, test_id)
    resolved_backend = resolve_backend(backend, test)
    env_vars = load_env_file(env_file) if env_file else {}

    # For X11 backend, swap monitor positions and scales with X11-discovered values
    # so that RON template substitution and validation use X11 coordinates.
    if resolved_backend == "x11":
        _swap_x11_env_vars(env_vars)

    # Resolve after env is loaded so a scale-role launch monitor can look up its index.
    configured_launch_monitor = resolve_launch_monitor(test, env_vars)

    configured_launch_position = None
    if configured_launch_monitor is not None:
        position_x = env_vars.get(f"MONITOR_{configured_launch_monitor}_POS_X")
        position_y = env_vars.get(f"MONITOR_{configured_launch_monitor}_POS_Y")
        if position_x is not None and position_y is not None:
            configured_launch_position = (int(position_x) + 100, int(position_y) + 100)

    # A small launch window keeps the born-on monitor from receiving an oversized software
    # (WARP) swapchain before the restore relocates it; only the cross-DPI cases set it.
    configured_launch_size = test.get("launch_size")

    write_ron(test["ron_file"], ron_dir, ron_path, env_vars)
    return test, resolved_backend, env_vars


# =============================================================================
# Discovery mode
# =============================================================================


def _extract_video_modes(
    entity: JsonDict,
    prefix: str,
    env_vars: dict[str, str],
) -> None:
    """Extract a random video mode from a monitor entity and write to env_vars."""
    modes_raw = extract_from_entity(entity, COMP_MONITOR, "video_modes")
    video_modes = json_list(modes_raw)
    if not video_modes:
        return

    selected = json_dict(random.choice(video_modes))
    phys = json_list(selected.get("physical_size"))
    phys_w = json_str(phys[0]) if len(phys) > 0 else "0"
    phys_h = json_str(phys[1]) if len(phys) > 1 else "0"
    depth = json_str(selected.get("bit_depth"))
    refresh = json_str(selected.get("refresh_rate_millihertz"))

    env_vars[f"{prefix}WIDTH"] = phys_w
    env_vars[f"{prefix}HEIGHT"] = phys_h
    env_vars[f"{prefix}DEPTH"] = depth
    env_vars[f"{prefix}REFRESH"] = refresh

    # Compute logical video mode dimensions (physical / monitor scale)
    scale_key = prefix.replace("VIDEO_MODE_", "") + "SCALE"
    scale_val = float(env_vars.get(scale_key, "1") or "1")
    logical_w = str(int(float(phys_w) / scale_val))
    logical_h = str(int(float(phys_h) / scale_val))
    env_vars[f"{prefix}LOGICAL_WIDTH"] = logical_w
    env_vars[f"{prefix}LOGICAL_HEIGHT"] = logical_h

    print(f"export {prefix}WIDTH={phys_w}")
    print(f"export {prefix}HEIGHT={phys_h}")
    print(f"export {prefix}DEPTH={depth}")
    print(f"export {prefix}REFRESH={refresh}")
    print(f"export {prefix}LOGICAL_WIDTH={logical_w}")
    print(f"export {prefix}LOGICAL_HEIGHT={logical_h}")


def run_discovery(
    ron_dir: str,
    ron_path: str,
    env_file: str,
    backend: str,
) -> None:
    env_vars: dict[str, str] = {}

    write_ron("discovery.ron", ron_dir, ron_path, env_vars)
    _ = launch_app(backend="native", wait_restore=False)

    # Poll the example's structured snapshot until Clerestory has installed a topology.
    monitor_list: list[JsonDict] = []
    for _ in range(MAX_POLLS):
        try:
            result = brp.call("clerestory/monitor_snapshot")
            monitor_list_raw = json_list(json_get(result, "result", "monitors"))
            monitor_list = [json_dict(monitor) for monitor in monitor_list_raw]
            if monitor_list:
                break
        except (URLError, OSError, KeyError, RuntimeError):
            pass
        time.sleep(POLL_INTERVAL)
    else:
        die("Monitor snapshot not populated within timeout")

    num_monitors = len(monitor_list)
    env_vars["NUM_MONITORS"] = str(num_monitors)

    print(f"export NUM_MONITORS={num_monitors}")

    for i, mon in enumerate(monitor_list):
        pos = json_list(mon.get("physical_position"))
        size = json_list(mon.get("physical_size"))
        scale = json_str(mon.get("scale"))
        pos_x = json_str(pos[0]) if len(pos) > 0 else "0"
        pos_y = json_str(pos[1]) if len(pos) > 1 else "0"
        size_w = json_str(size[0]) if len(size) > 0 else "0"
        size_h = json_str(size[1]) if len(size) > 1 else "0"
        refresh_rate = json_str(mon.get("refresh_rate_millihertz"))

        scale_f = float(scale) if scale else 1.0
        logical_pos_x = str(round(float(pos_x) / scale_f)) if scale_f != 0 else pos_x
        logical_pos_y = str(round(float(pos_y) / scale_f)) if scale_f != 0 else pos_y

        env_vars[f"MONITOR_{i}_POS_X"] = pos_x
        env_vars[f"MONITOR_{i}_POS_Y"] = pos_y
        env_vars[f"MONITOR_{i}_LOGICAL_POS_X"] = logical_pos_x
        env_vars[f"MONITOR_{i}_LOGICAL_POS_Y"] = logical_pos_y
        env_vars[f"MONITOR_{i}_WIDTH"] = size_w
        env_vars[f"MONITOR_{i}_HEIGHT"] = size_h
        env_vars[f"MONITOR_{i}_SCALE"] = scale
        env_vars[f"MONITOR_{i}_REFRESH_RATE_MILLIHERTZ"] = refresh_rate
        name = mon.get("name")
        if isinstance(name, str):
            env_vars[f"MONITOR_{i}_NAME"] = name

        print(f"export MONITOR_{i}_POS_X={pos_x}")
        print(f"export MONITOR_{i}_POS_Y={pos_y}")
        print(f"export MONITOR_{i}_LOGICAL_POS_X={logical_pos_x}")
        print(f"export MONITOR_{i}_LOGICAL_POS_Y={logical_pos_y}")
        print(f"export MONITOR_{i}_WIDTH={size_w}")
        print(f"export MONITOR_{i}_HEIGHT={size_h}")
        print(f"export MONITOR_{i}_SCALE={scale}")
        print(f"export MONITOR_{i}_REFRESH_RATE_MILLIHERTZ={refresh_rate}")
        if isinstance(name, str):
            print(f"export MONITOR_{i}_NAME={name}")

    # Query video modes
    try:
        monitor_query = brp.call("world.query", {
            "data": {"components": [COMP_MONITOR]},
            "filter": {},
        })
        monitor_entities = [json_dict(e) for e in json_list(monitor_query.get("result"))]

        for entity in monitor_entities:
            name = extract_from_entity(entity, COMP_MONITOR, "name")
            position = json_list(extract_from_entity(entity, COMP_MONITOR, "physical_position"))
            width = json_str(extract_from_entity(entity, COMP_MONITOR, "physical_width"))
            height = json_str(extract_from_entity(entity, COMP_MONITOR, "physical_height"))
            if len(position) < 2 or not isinstance(name, str):
                continue
            position_x = json_str(position[0])
            position_y = json_str(position[1])
            for monitor_index in range(num_monitors):
                if (
                    env_vars.get(f"MONITOR_{monitor_index}_POS_X") == position_x
                    and env_vars.get(f"MONITOR_{monitor_index}_POS_Y") == position_y
                    and env_vars.get(f"MONITOR_{monitor_index}_WIDTH") == width
                    and env_vars.get(f"MONITOR_{monitor_index}_HEIGHT") == height
                ):
                    env_vars[f"MONITOR_{monitor_index}_NAME"] = name
                    print(f"export MONITOR_{monitor_index}_NAME={name}")
                    break

        for i in range(min(len(monitor_entities), num_monitors)):
            _extract_video_modes(monitor_entities[i], f"MONITOR_{i}_VIDEO_MODE_", env_vars)
    except (URLError, OSError):
        pass

    # Validate WindowRestored or WindowRestoreMismatch event
    try:
        restored_result = brp.call("world.get_resources", {"resource": RES_RESTORED})
        restored_value = json_get(restored_result, "result", "value")
        if restored_value is not None:
            print("# WindowRestoredReceived validation: OK")
        else:
            mismatch_result = brp.call("world.get_resources", {"resource": RES_MISMATCH})
            mismatch_value = json_get(mismatch_result, "result", "value")
            if mismatch_value is not None:
                print("# WindowRestoreMismatchReceived validation: OK (mismatch detected)")
            else:
                print("# WARNING: Neither WindowRestoredReceived nor WindowRestoreMismatchReceived found")
    except (URLError, OSError):
        print("# WARNING: WindowRestoredReceived resource query failed")

    # Compute DIFFERENT_SCALES across ALL monitors (not just 0 vs 1) and, when scales differ,
    # expose the highest- and lowest-scale monitor indices plus their logical origins. Cross-DPI
    # cases pick their launch/target monitor by scale role instead of a fixed slot, so the
    # differently-scaled monitor can be any discovered display (e.g. a provisioned virtual one).
    scales = [(i, float(json_str(m.get("scale")))) for i, m in enumerate(monitor_list)]
    if len({scale for _, scale in scales}) >= 2:
        different = "true"
        high_index = max(scales, key=lambda pair: pair[1])[0]
        low_index = min(scales, key=lambda pair: pair[1])[0]
        for role, index in (("HIGH", high_index), ("LOW", low_index)):
            env_vars[f"{role}_SCALE_MONITOR_INDEX"] = str(index)
            print(f"export {role}_SCALE_MONITOR_INDEX={index}")
            for suffix in ("LOGICAL_POS_X", "LOGICAL_POS_Y"):
                value = env_vars.get(f"MONITOR_{index}_{suffix}", "0")
                env_vars[f"{role}_SCALE_MONITOR_{suffix}"] = value
                print(f"export {role}_SCALE_MONITOR_{suffix}={value}")
    else:
        different = "false"

    env_vars["DIFFERENT_SCALES"] = different
    print(f"export DIFFERENT_SCALES={different}")

    shutdown_app()

    # Linux X11 discovery
    if backend == "x11-also":
        print("# X11 discovery...")
        _ = launch_app(backend="x11", wait_restore=False)

        try:
            x11_result = brp.call("clerestory/monitor_snapshot")
            x11_list = [
                json_dict(m)
                for m in json_list(json_get(x11_result, "result", "monitors"))
            ]

            for i in range(min(len(x11_list), num_monitors)):
                x11_scale = json_str(x11_list[i].get("scale"))
                env_vars[f"MONITOR_{i}_X11_SCALE"] = x11_scale
                print(f"export MONITOR_{i}_X11_SCALE={x11_scale}")

                x11_pos = json_list(x11_list[i].get("physical_position"))
                x11_pos_x = json_str(x11_pos[0]) if len(x11_pos) > 0 else "0"
                x11_pos_y = json_str(x11_pos[1]) if len(x11_pos) > 1 else "0"
                x11_scale_f = float(x11_scale) if x11_scale else 1.0
                x11_lpos_x = str(round(float(x11_pos_x) / x11_scale_f)) if x11_scale_f != 0 else x11_pos_x
                x11_lpos_y = str(round(float(x11_pos_y) / x11_scale_f)) if x11_scale_f != 0 else x11_pos_y

                env_vars[f"MONITOR_{i}_X11_POS_X"] = x11_pos_x
                env_vars[f"MONITOR_{i}_X11_POS_Y"] = x11_pos_y
                env_vars[f"MONITOR_{i}_X11_LOGICAL_POS_X"] = x11_lpos_x
                env_vars[f"MONITOR_{i}_X11_LOGICAL_POS_Y"] = x11_lpos_y
                print(f"export MONITOR_{i}_X11_POS_X={x11_pos_x}")
                print(f"export MONITOR_{i}_X11_POS_Y={x11_pos_y}")
                print(f"export MONITOR_{i}_X11_LOGICAL_POS_X={x11_lpos_x}")
                print(f"export MONITOR_{i}_X11_LOGICAL_POS_Y={x11_lpos_y}")

            x11_query = brp.call("world.query", {
                "data": {"components": [COMP_MONITOR]},
                "filter": {},
            })
            x11_entities = [json_dict(e) for e in json_list(x11_query.get("result"))]

            for i in range(min(len(x11_entities), num_monitors)):
                _extract_video_modes(x11_entities[i], f"MONITOR_{i}_X11_VIDEO_MODE_", env_vars)
        except (URLError, OSError):
            pass

        shutdown_app()

    write_env_file(env_file, env_vars)
    sys.exit(0)


# =============================================================================
# Prebuild mode
# =============================================================================


def run_prebuild() -> None:
    uname = platform.system()
    if uname == "Darwin":
        plat = "macos"
    elif uname == "Linux":
        plat = "linux"
    elif uname in ("Windows", "MINGW", "MSYS", "CYGWIN") or uname.startswith("MINGW"):
        plat = "windows"
    else:
        die(f"Unsupported platform: {uname}")

    config_file = f"tests/config/{plat}.json"
    print(f"PLATFORM={plat}")
    print(f"CONFIG={config_file}")

    if not os.path.isfile(config_file):
        die(f"Config file not found: {config_file}")

    # Ensure tmp directory exists
    default_tmpdir = "/private/tmp/claude-501" if sys.platform == "darwin" else "/tmp/claude"
    tmpdir = os.environ.get("TMPDIR", default_tmpdir)
    os.makedirs(tmpdir, exist_ok=True)

    # Build default variant
    result = subprocess.run(
        ["cargo", "build", "--example", "restore_window"],
        capture_output=False,
    )
    if result.returncode == 0:
        print("BUILD_DEFAULT=ok")
    else:
        print("BUILD_DEFAULT=failed")
        sys.exit(1)

    # Build all unique feature flag variants from workaround tests
    with open(config_file) as f:
        config: PlatformConfig = json.load(f)  # pyright: ignore[reportAny]

    flag_sets: set[str] = set()
    for t in config["tests"]:
        wv = t.get("workaround_validation")
        if wv:
            build_without = wv.get("build_without", "")
            build_with = wv.get("build_with", "")
            if build_without:
                flag_sets.add(build_without)
            if build_with:
                flag_sets.add(build_with)

    if flag_sets:
        for i, flags in enumerate(sorted(flag_sets)):
            cmd = ["cargo", "build", "--example", "restore_window"] + flags.split()
            print(f"BUILD_VARIANT_{i}={flags}")
            result = subprocess.run(cmd, capture_output=False)
            if result.returncode == 0:
                print(f"BUILD_VARIANT_{i}=ok")
            else:
                print(f"BUILD_VARIANT_{i}=failed")
                sys.exit(1)
        print(f"BUILD_VARIANTS={len(flag_sets)} ok")
    else:
        print("BUILD_VARIANTS=skipped")


# =============================================================================
# Test mode
# =============================================================================


def run_test(
    config: PlatformConfig,
    test_id: str,
    ron_dir: str,
    ron_path: str,
    env_file: str,
    backend: str,
) -> None:
    global app_process
    test, resolved_backend, env_vars = setup_test(config, test_id, ron_dir, ron_path, env_file, backend)

    # Cross-DPI tests need monitors with different scale factors, decided by the
    # active backend's real scales. X11 exposes one global Xft.dpi to every
    # monitor (uniform scale), so the cross-DPI scenario is unreachable there
    # even when Wayland reports differing per-monitor scales. Skip
    # deterministically rather than launching and failing the strategy
    # precondition. (env_vars holds X11-swapped scales for the x11 backend.)
    requires = test.get("requires", {})
    if requires.get("different_scales"):
        seen_scales: set[str] = set()
        monitor_i = 0
        while f"MONITOR_{monitor_i}_SCALE" in env_vars:
            seen_scales.add(env_vars[f"MONITOR_{monitor_i}_SCALE"])
            monitor_i += 1
        if len(seen_scales) < 2:
            skip_line(
                "primary",
                "different_scales",
                f"backend={resolved_backend} reports uniform scale; cross-DPI scenario unavailable",
            )
            sys.exit(0)

    # Parse expected values from written RON
    ron_content = Path(ron_path).read_text()
    ron_values = parse_ron_values(ron_content)

    # Determine test features
    has_mutation = "mutation" in test
    has_persistence = "persistence_validation" in test
    expected_log = test.get("expected_log_warning", "")
    windows = test.get("windows", {})

    # Check if any window has exit_code validation
    has_exit_code = any(
        "exit_code" in wconfig.get("validate", [])
        for wconfig in windows.values()
    )

    stderr_path, restore_result = launch_app(
        resolved_backend,
        wait_restore=test.get("wait_for_restore_event", True),
        test_mode=True,
    )
    expect_mismatch = test.get("expect_mismatch", False)
    if restore_result == "mismatch":
        if expect_mismatch:
            pass_line("restore", "event", "WindowRestoreMismatch fired (expected)")
        else:
            fail_line("restore", "event", "WindowRestoreMismatch fired (restore state did not match)")

    # Exit code only test — check for panics in stderr
    if has_exit_code:
        # Give the app a moment for any deferred panics to appear in stderr
        time.sleep(2)
        panicked = _stderr_has_panic(stderr_path) if stderr_path else False
        if panicked:
            fail_line("primary", "exit_code", "expected=no_panic actual=panic_detected")
        else:
            pass_line("primary", "exit_code", "expected=no_panic actual=no_panic")
        if app_process is not None and app_process.poll() is not None:
            app_process = None
        else:
            shutdown_app()
        sys.exit(1 if fail_count > 0 else 0)

    # Initial validation
    validate_all_windows(windows, ron_values, prefix="", backend=resolved_backend, env_vars=env_vars)

    # Launch-monitor precondition: confirm the window actually launched on the
    # test's launch_monitor (and ran a cross-DPI strategy when one was intended),
    # rather than spawning same-scale and passing the checks above hollowly.
    primary_entity = resolve_primary_entity()
    if primary_entity is not None:
        validate_launch_monitor(test, primary_entity, resolved_backend, env_vars)

    # Persistence setup (set mode before shutdown)
    if has_persistence:
        persistence = test.get("persistence_validation", {})
        persist_mode = persistence.get("mode", "")
        if persist_mode == "ActiveOnly":
            _ = brp.call("world.insert_resources", {
                "resource": COMP_PERSISTENCE,
                "value": "ActiveOnly",
            })

    # Apply mutations
    if has_mutation:
        primary_entity = resolve_primary_entity()
        if primary_entity is None:
            fail_line("primary", "mutation_query", "primary window not found")
        else:
            apply_mutations(test, primary_entity, ron_values, env_vars=env_vars)
            time.sleep(0.5)
            verify_mutations(ron_values, backend=resolved_backend, env_vars=env_vars)

    # Shutdown
    shutdown_app()

    # Check expected log warning
    if expected_log and stderr_path and os.path.isfile(stderr_path):
        stderr_content = Path(stderr_path).read_text()
        if expected_log in stderr_content:
            pass_line("log", "expected_warning", f'found="{expected_log}"')
        else:
            fail_line("log", "expected_warning", f'not_found="{expected_log}"')
        os.unlink(stderr_path)

    # Persistence validation
    if has_persistence:
        validate_persistence(test, ron_path)

    # Relaunch and validate restore (if mutation)
    if has_mutation:
        _, relaunch_result = launch_app(resolved_backend, test_mode=True)
        if relaunch_result == "mismatch":
            fail_line("restore", "relaunch_event", "WindowRestoreMismatch fired on relaunch")
        validate_all_windows(windows, ron_values, prefix="relaunch ", backend=resolved_backend, env_vars=env_vars)
        shutdown_app()

    sys.exit(1 if fail_count > 0 else 0)


# =============================================================================
# Main
# =============================================================================


def main() -> None:
    parser = argparse.ArgumentParser(description="hana_clerestory test runner v3")
    _ = parser.add_argument("--prebuild", action="store_true", help="Build both binary variants")
    _ = parser.add_argument("--discover", action="store_true", help="Discovery mode")
    _ = parser.add_argument("--config", default="", help="Platform config JSON file")
    _ = parser.add_argument("--test-id", default="", help="Test ID to run")
    _ = parser.add_argument("--backend", default="native", help="Backend: native, x11, x11-also")
    _ = parser.add_argument("--env-file", default="", help="Env file for MONITOR_* vars")
    _ = parser.add_argument("--executable", default="", help="Prebuilt restore_window executable")
    _ = parser.add_argument("--base-port", type=int, default=DEFAULT_BRP_PORT, help="Main BRP HTTP port")
    _ = parser.add_argument("--ron-path", default="", help="Isolated persistence file for this case")
    _ = parser.add_argument("--artifact-dir", default="", help="Directory for child-process logs")
    _ = parser.add_argument("--result-json", default="", help="Write a structured CaseResult JSON file")

    args = cast(Args, parser.parse_args())
    global brp
    global configured_artifact_directory
    global configured_base_port
    global configured_executable
    global configured_persistence_path
    global result_case_id
    global result_json_path
    configured_base_port = args.base_port
    brp = BrpClient(configured_base_port)
    configured_executable = Path(args.executable).resolve() if args.executable else None
    configured_artifact_directory = Path(args.artifact_dir).resolve() if args.artifact_dir else None
    result_json_path = Path(args.result_json).resolve() if args.result_json else None
    result_case_id = args.test_id

    if args.prebuild:
        run_prebuild()
        return

    if not args.config:
        die("Missing --config")
    if not os.path.isfile(args.config):
        die(f"Config file not found: {args.config}")

    with open(args.config) as f:
        config: PlatformConfig = json.load(f)  # pyright: ignore[reportAny]

    # Derive ron_dir and ron_path from config
    ron_dir = config["test_ron_dir"]
    if not ron_dir:
        die("Config missing test_ron_dir")

    raw_ron_path = args.ron_path or config["example_ron_path"]
    if not raw_ron_path:
        die("Config missing example_ron_path")

    # Only expand leading ~ (not tildes elsewhere in the path)
    if raw_ron_path.startswith("~"):
        ron_path = os.path.expanduser("~") + raw_ron_path[1:]
    else:
        ron_path = raw_ron_path
    appdata = os.environ.get("APPDATA", "")
    if appdata:
        ron_path = ron_path.replace("%APPDATA%", appdata)
    configured_persistence_path = Path(ron_path).resolve()

    if args.discover:
        if not args.env_file:
            die("Missing --env-file (required for --discover)")
        run_discovery(ron_dir, ron_path, args.env_file, args.backend)
    else:
        if not args.test_id:
            die("Missing --test-id (required unless --discover or --prebuild)")
        run_test(
            config,
            args.test_id,
            ron_dir,
            ron_path,
            args.env_file,
            args.backend,
        )


def write_case_result(exit_code: int, outcome_override: Outcome | None = None) -> None:
    if result_json_path is None or not result_case_id:
        return
    availability = Availability.UNAVAILABLE if skip_count else Availability.AVAILABLE
    if outcome_override is not None:
        outcome = outcome_override
    elif availability is not Availability.AVAILABLE:
        outcome = Outcome.NOT_RUN
    elif exit_code == 0 and fail_count == 0:
        outcome = Outcome.PASSED
    elif exit_code == 1 or fail_count:
        outcome = Outcome.FAILED
    else:
        outcome = Outcome.HARNESS_ERROR
    case_result = CaseResult(
        case_id=result_case_id,
        interaction=Interaction.AUTOMATED,
        evidence=Evidence.APPLICATION,
        availability=availability,
        outcome=outcome,
        availability_reason="test requirements were not available" if skip_count else None,
        missing_capability="different-scales" if skip_count else None,
        assertions=list(assertion_results),
        process_exit_code=exit_code,
        elapsed_seconds=time.monotonic() - result_started,
        artifacts=list(result_artifacts),
    )
    result_json_path.parent.mkdir(parents=True, exist_ok=True)
    temporary_path = result_json_path.with_suffix(result_json_path.suffix + ".tmp")
    _ = temporary_path.write_text(json.dumps(case_result.to_dict(), indent=2, sort_keys=True) + "\n")
    _ = temporary_path.replace(result_json_path)


def entrypoint() -> int:
    global result_started
    result_started = time.monotonic()
    try:
        main()
    except KeyboardInterrupt:
        write_case_result(130, Outcome.INTERRUPTED)
        return 130
    except SystemExit as exit_request:
        exit_code = exit_request.code if isinstance(exit_request.code, int) else 2
        write_case_result(exit_code)
        return exit_code
    except BaseException:
        traceback.print_exc()
        write_case_result(2, Outcome.HARNESS_ERROR)
        return 2
    write_case_result(0)
    return 0


if __name__ == "__main__":
    sys.exit(entrypoint())
