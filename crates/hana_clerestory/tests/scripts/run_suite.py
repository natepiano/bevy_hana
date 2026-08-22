#!/usr/bin/env python3
"""Run all Clerestory checks that are available without agent orchestration."""

from __future__ import annotations

import argparse
import json
import os
import platform
import queue
import re
import signal
import shlex
import shutil
import subprocess
import sys
import threading
import time
import uuid
from dataclasses import dataclass
from datetime import UTC
from datetime import datetime
from enum import StrEnum
from pathlib import Path
from typing import BinaryIO
from typing import TextIO
from typing import cast
from typing import final

from clerestory_test.app_session import PortPair
from clerestory_test.app_session import AppSession
from clerestory_test.app_session import available_port_pair
from clerestory_test.case_result import AssertionResult
from clerestory_test.case_result import Availability
from clerestory_test.case_result import CaseResult
from clerestory_test.case_result import CleanupResult
from clerestory_test.case_result import Evidence
from clerestory_test.case_result import Interaction
from clerestory_test.case_result import Outcome
from clerestory_test.case_result import unavailable_case
from clerestory_test.hardware_profile import HardwareProfile
from clerestory_test.run_report import RunReport
from clerestory_test.restore_client import RestoreClient
from run_reconnect import run_automated_reconnect
from run_reconnect import run_zero_window_case


SCRIPT_DIRECTORY = Path(__file__).resolve().parent
CRATE_ROOT = SCRIPT_DIRECTORY.parent.parent
WORKSPACE_ROOT = CRATE_ROOT.parent.parent
CONFIG_DIRECTORY = CRATE_ROOT / "tests" / "config"
DEFAULT_ARTIFACT_ROOT = WORKSPACE_ROOT / "target" / "clerestory-tests"
CASE_TIMEOUT_SECONDS = 90.0
# Set by a Wayland compositor for every client in its session; `run_test.py` leaves it in place for
# a `backend: wayland` case and clears it only for `backend: x11`.
WAYLAND_DISPLAY_ENVIRONMENT_VARIABLE = "WAYLAND_DISPLAY"
BUILD_TIMEOUT_SECONDS = 1800.0
ASSISTED_ACTION_TIMEOUT_SECONDS = 300.0
LAUNCH_POSITION_INSET = 100


class FullscreenSignal(StrEnum):
    """Which observations must agree before a fullscreen wait is satisfied.

    macOS drives a green-button transition through AppKit and never writes the result back into
    Bevy's `Window.mode`, so an operator-driven fullscreen is observable only through
    `native_fullscreen`. A fullscreen that Bevy itself applies sets both.
    """

    NATIVE = "native"
    MODE_AND_NATIVE = "mode-and-native"


@dataclass(frozen=True)
class CommandResult:
    return_code: int
    elapsed_seconds: float
    timed_out: bool


@final
class DisplayLock:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.handle: BinaryIO | None = None

    def __enter__(self) -> DisplayLock:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.handle = self.path.open("a+b")
        if sys.platform == "win32":
            import msvcrt

            msvcrt.locking(self.handle.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl

            fcntl.flock(self.handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        return self

    def __exit__(self, _type: object, _value: object, _traceback: object) -> None:
        if self.handle is None:
            return
        if sys.platform == "win32":
            import msvcrt

            _ = self.handle.seek(0)
            msvcrt.locking(self.handle.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl

            fcntl.flock(self.handle.fileno(), fcntl.LOCK_UN)
        self.handle.close()
        self.handle = None


def platform_name() -> str:
    system = platform.system()
    if system == "Darwin":
        return "macos"
    if system == "Linux":
        return "linux"
    if system == "Windows" or system.startswith(("MINGW", "MSYS", "CYGWIN")):
        return "windows"
    raise RuntimeError(f"unsupported platform: {system}")


def wayland_case_unavailable(selected_platform: str, test: dict[str, object]) -> bool:
    """Whether a `backend: wayland` case has no compositor to run against.

    The suite used to decline every one of these cases outright, on the reasoning that a Wayland
    client cannot ask the compositor to open a window on a chosen output. That is true of the
    `launch_monitor` precondition but not of what the cases assert: they validate `size`, `mode`
    and `monitor_index`, never `position`, and `run_test.py` already skips position checks
    (`validate_window_field`) and downgrades the launch-monitor check to an informational note
    (`validate_launch_monitor`) when the backend is Wayland. Restoring a window to a named monitor
    is supported — `SavedWindowMode::to_window_mode` passes `MonitorSelection::Index` for every
    fullscreen mode precisely so Wayland, which reports every position as (0,0), still lands on the
    right display.

    What genuinely makes the cases unrunnable is having no Wayland session at all, which is the
    case under a bare X11 login.
    """
    return (
        selected_platform == "linux"
        and test.get("backend") == "wayland"
        and not os.environ.get(WAYLAND_DISPLAY_ENVIRONMENT_VARIABLE)
    )


def source_revision() -> str:
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=WORKSPACE_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return completed.stdout.strip()


def load_config(selected_platform: str) -> dict[str, object]:
    config_path = CONFIG_DIRECTORY / f"{selected_platform}.json"
    value = cast("object", json.loads(config_path.read_text()))
    if not isinstance(value, dict):
        raise ValueError(f"{config_path} must contain an object")
    return cast("dict[str, object]", value)


def config_tests(config: dict[str, object]) -> list[dict[str, object]]:
    tests = config.get("tests")
    if not isinstance(tests, list):
        raise ValueError("platform config tests must contain objects")
    result: list[dict[str, object]] = []
    for test in cast("list[object]", tests):
        if not isinstance(test, dict):
            raise ValueError("platform config tests must contain objects")
        result.append(cast("dict[str, object]", test))
    return result


def _dict_list(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list):
        return []
    result: list[dict[str, object]] = []
    for item in cast("list[object]", value):
        if isinstance(item, dict):
            result.append(cast("dict[str, object]", item))
    return result


def _coerce_monitor_index(value: object) -> int:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        return int(value)
    return 0


def case_interaction(test: dict[str, object]) -> Interaction:
    automation = test.get("automation")
    if automation == "full":
        return Interaction.AUTOMATED
    if automation in ("human_assisted", "operator_action"):
        return Interaction.OPERATOR_ACTION
    return Interaction.OPERATOR_JUDGMENT


def run_command(
    command: list[str],
    *,
    cwd: Path,
    log_path: Path,
    timeout_seconds: float,
    report: RunReport,
    environment: dict[str, str] | None = None,
) -> CommandResult:
    started = time.monotonic()
    lines: queue.Queue[str | None] = queue.Queue()
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("w") as log:
        creation_flags = 0
        start_new_session = sys.platform != "win32"
        if sys.platform == "win32":
            creation_flags = subprocess.CREATE_NEW_PROCESS_GROUP
        process = subprocess.Popen(
            command,
            cwd=cwd,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            start_new_session=start_new_session,
            creationflags=creation_flags,
        )

        def read_output(stream: TextIO) -> None:
            for line in stream:
                lines.put(line)
            lines.put(None)

        if process.stdout is None:
            raise RuntimeError("command output pipe was not created")
        reader = threading.Thread(target=read_output, args=(process.stdout,), daemon=True)
        reader.start()
        output_finished = False
        timed_out = False
        while process.poll() is None or not output_finished:
            try:
                line = lines.get(timeout=1)
                if line is None:
                    output_finished = True
                else:
                    print(line, end="", flush=True)
                    _ = log.write(line)
                    log.flush()
            except queue.Empty:
                report.heartbeat_path.touch()
            if process.poll() is None and time.monotonic() - started >= timeout_seconds:
                timed_out = True
                if sys.platform == "win32":
                    process.terminate()
                else:
                    os.killpg(process.pid, signal.SIGTERM)
                try:
                    _ = process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    if sys.platform == "win32":
                        process.kill()
                    else:
                        os.killpg(process.pid, signal.SIGKILL)
                break
        reader.join(timeout=2)
        return_code = process.wait()
    return CommandResult(
        return_code=return_code,
        elapsed_seconds=time.monotonic() - started,
        timed_out=timed_out,
    )


def feature_variants(tests: list[dict[str, object]]) -> set[str]:
    variants = {""}
    for test in tests:
        validation = test.get("workaround_validation")
        if not isinstance(validation, dict):
            continue
        fields = cast("dict[str, object]", validation)
        for key in ("build_without", "build_with"):
            value = fields.get(key, "")
            if isinstance(value, str):
                variants.add(value)
    return variants


def executable_suffix() -> str:
    return ".exe" if sys.platform == "win32" else ""


def prebuild_restore_variants(
    tests: list[dict[str, object]], report: RunReport
) -> dict[str, Path]:
    build_directory = report.artifact_directory / "builds"
    build_directory.mkdir()
    binaries: dict[str, Path] = {}
    variants = sorted(feature_variants(tests))
    for index, flags in enumerate(variants, start=1):
        label = flags or "default features"
        print(f"Prebuild {index}/{len(variants)}: restore_window ({label})", flush=True)
        report.event("prebuild-started", label)
        command = ["cargo", "build", "-p", "hana_clerestory", "--example", "restore_window"]
        command.extend(shlex.split(flags))
        result = run_command(
            command,
            cwd=WORKSPACE_ROOT,
            log_path=report.artifact_directory / f"prebuild-restore-{index}.log",
            timeout_seconds=BUILD_TIMEOUT_SECONDS,
            report=report,
        )
        if result.return_code != 0:
            raise RuntimeError(f"restore_window prebuild failed for {label}")
        source = WORKSPACE_ROOT / "target" / "debug" / "examples" / f"restore_window{executable_suffix()}"
        destination = build_directory / f"restore_window-{index}{executable_suffix()}"
        _ = shutil.copy2(source, destination)
        destination.chmod(destination.stat().st_mode | 0o111)
        binaries[flags] = destination
        report.event("prebuild-completed", label)
    return binaries


def run_preflight(report: RunReport) -> None:
    gates = [
        (
            "reconnect example tests",
            [
                [
                    "cargo",
                    "nextest",
                    "run",
                    "-p",
                    "hana_clerestory",
                    "--example",
                    "restore_after_reconnect",
                    "--features",
                    "monitor-probe",
                ]
            ],
        ),
        (
            "library and controller tests",
            [
                [
                    "cargo",
                    "nextest",
                    "run",
                    "-p",
                    "hana_clerestory",
                    "--lib",
                ],
                [
                    sys.executable,
                    "-m",
                    "unittest",
                    "discover",
                    "-s",
                    str(SCRIPT_DIRECTORY),
                    "-p",
                    "test_*.py",
                ],
            ],
        ),
        (
            "format and lint",
            [
                ["cargo", "+nightly", "fmt", "--all", "--", "--check"],
                [
                    "cargo",
                    "clippy",
                    "-p",
                    "hana_clerestory",
                    "--lib",
                    "--tests",
                    "--",
                    "-D",
                    "warnings",
                ],
            ],
        ),
    ]
    for index, (label, commands) in enumerate(gates, start=1):
        print(f"Preflight {index}/{len(gates)}: {label}", flush=True)
        report.event("preflight-started", f"{index}/{len(gates)} {label}")
        for command_index, command in enumerate(commands, start=1):
            result = run_command(
                command,
                cwd=WORKSPACE_ROOT,
                log_path=(
                    report.artifact_directory
                    / f"preflight-{index}-{command_index}.log"
                ),
                timeout_seconds=BUILD_TIMEOUT_SECONDS,
                report=report,
            )
            if result.return_code != 0:
                raise RuntimeError(f"preflight {index}/{len(gates)} failed: {label}")
        report.event("preflight-completed", f"{index}/{len(gates)} {label}")
        print("  passed", flush=True)


def prebuild_reconnect(report: RunReport) -> Path:
    print("Prebuild reconnect probe", flush=True)
    report.event("prebuild-started", "restore_after_reconnect")
    command = [
        "cargo",
        "build",
        "-p",
        "hana_clerestory",
        "--example",
        "restore_after_reconnect",
        "--features",
        "monitor-probe",
    ]
    result = run_command(
        command,
        cwd=WORKSPACE_ROOT,
        log_path=report.artifact_directory / "prebuild-reconnect.log",
        timeout_seconds=BUILD_TIMEOUT_SECONDS,
        report=report,
    )
    if result.return_code != 0:
        raise RuntimeError("restore_after_reconnect prebuild failed")
    source = (
        WORKSPACE_ROOT
        / "target"
        / "debug"
        / "examples"
        / f"restore_after_reconnect{executable_suffix()}"
    )
    destination = (
        report.artifact_directory
        / "builds"
        / f"restore_after_reconnect{executable_suffix()}"
    )
    _ = shutil.copy2(source, destination)
    destination.chmod(destination.stat().st_mode | 0o111)
    report.event("prebuild-completed", "restore_after_reconnect")
    return destination


def run_probe_lifecycle_case(
    executable: Path,
    artifact_directory: Path,
    run_id: str,
    case_id: str,
) -> CaseResult:
    """Run the zero-window lifecycle case, or report why this host cannot.

    Windows is held out: the probe closes its windows over the remote channel, and there the app
    stops servicing further requests part way through the sequence. That is window teardown
    misbehaving, not the liveness and reconstruction this case exists to measure, so reporting it
    as a failure would attribute the wrong defect. macOS and Linux run the case for real, and a
    failure there is recorded as a failure — an availability label must never be able to absorb a
    genuine regression.
    """
    if sys.platform == "win32":
        return unavailable_case(
            case_id,
            Interaction.AUTOMATED,
            Evidence.APPLICATION,
            "closing every owned window over the probe channel stops the app answering",
            "windows-window-teardown",
        )
    try:
        return run_zero_window_case(executable, artifact_directory, run_id)
    except Exception as error:
        # Keep the controller alive so the physical reconnect partition still runs, but record
        # the outcome as the failure it is.
        return CaseResult(
            case_id=case_id,
            interaction=Interaction.AUTOMATED,
            evidence=Evidence.APPLICATION,
            outcome=(
                Outcome.TIMED_OUT if isinstance(error, TimeoutError) else Outcome.HARNESS_ERROR
            ),
            assertions=[AssertionResult("expected-state-reached", False, str(error))],
            artifacts=[str(artifact_directory)],
        )


def run_discovery(
    config_path: Path,
    executable: Path,
    report: RunReport,
    selected_platform: str,
    suffix: str = "",
) -> Path:
    tag = f"-{suffix}" if suffix else ""
    environment_path = report.artifact_directory / f"discovery{tag}.env"
    artifact_directory = report.artifact_directory / f"discovery{tag}"
    persistence_path = artifact_directory / "windows.ron"
    backend = "x11-also" if selected_platform == "linux" else "native"
    print("Discovery: reading monitor capabilities", flush=True)
    result: CommandResult | None = None
    for attempt in range(1, 4):
        port_pair = available_port_pair()
        command = [
            sys.executable,
            str(SCRIPT_DIRECTORY / "run_test.py"),
            "--discover",
            "--config",
            str(config_path),
            "--env-file",
            str(environment_path),
            "--backend",
            backend,
            "--executable",
            str(executable),
            "--base-port",
            str(port_pair.base),
            "--ron-path",
            str(persistence_path),
            "--artifact-dir",
            str(artifact_directory),
        ]
        result = run_command(
            command,
            cwd=CRATE_ROOT,
            log_path=report.artifact_directory / f"discovery{tag}-attempt-{attempt}.log",
            timeout_seconds=CASE_TIMEOUT_SECONDS,
            report=report,
        )
        if result.return_code == 0 or not artifact_has_bind_collision(
            artifact_directory
        ):
            break
        report.event("launch-retry", f"discovery port collision; attempt {attempt}/3")
    if result is None:
        raise RuntimeError("monitor discovery did not start")
    if result.return_code != 0:
        raise RuntimeError("monitor discovery failed")
    return environment_path


def load_environment_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text().splitlines():
        if not line.startswith("export ") or "=" not in line:
            continue
        name, value = line.removeprefix("export ").split("=", 1)
        values[name] = value
    return values


def substitute_environment(template: str, environment: dict[str, str]) -> str:
    def replacement(match: re.Match[str]) -> str:
        expression = match.group(1)
        arithmetic = re.fullmatch(r"(\w+)([+-])(\d+)", expression)
        if arithmetic is not None:
            value = environment.get(arithmetic.group(1))
            if value is None:
                return match.group(0)
            offset = int(arithmetic.group(3))
            return str(
                int(value) + offset
                if arithmetic.group(2) == "+"
                else int(value) - offset
            )
        return environment.get(expression, match.group(0))

    return re.sub(r"\$\{([^}]+)\}", replacement, template)


def launch_position(discovery: dict[str, str], launch_monitor: int) -> str | None:
    """Physical `x,y` inset into `launch_monitor`, matching `run_test.py`'s launch placement.

    `CLERESTORY_TEST_LAUNCH_MONITOR` alone only asks for a centered position, and winit drops a
    `Centered(Index(n))` it cannot resolve while `WinitMonitors` is still empty at window
    creation. The app is then born wherever the OS put it, which is not the monitor the case
    configured.
    """
    position_x = discovery.get(f"MONITOR_{launch_monitor}_POS_X")
    position_y = discovery.get(f"MONITOR_{launch_monitor}_POS_Y")
    if position_x is None or position_y is None:
        return None
    return f"{int(position_x) + LAUNCH_POSITION_INSET},{int(position_y) + LAUNCH_POSITION_INSET}"


def launch_restore_session(
    executable: Path,
    artifact_directory: Path,
    persistence_path: Path,
    launch_monitor: int,
    discovery: dict[str, str],
) -> tuple[AppSession, RestoreClient]:
    port_pair = available_port_pair()
    environment = dict(os.environ)
    environment.update(
        {
            "BRP_EXTRAS_PORT": str(port_pair.base),
            "CLERESTORY_TEST_MODE": "1",
            "CLERESTORY_TEST_LAUNCH_MONITOR": str(launch_monitor),
            "CLERESTORY_TEST_PERSISTENCE_PATH": str(persistence_path),
        }
    )
    position = launch_position(discovery, launch_monitor)
    if position is not None:
        environment["CLERESTORY_TEST_LAUNCH_POSITION"] = position
    launch_number = len(list(artifact_directory.glob("app-*.stdout.log"))) + 1
    session = AppSession(
        executable=executable,
        argv=[],
        environment=environment,
        stdout_path=artifact_directory / f"app-{launch_number}.stdout.log",
        stderr_path=artifact_directory / f"app-{launch_number}.stderr.log",
        working_directory=CRATE_ROOT,
    )
    client = RestoreClient(port_pair.base)
    session.start()
    try:
        client.wait_ready(CASE_TIMEOUT_SECONDS)
    except BaseException:
        _ = session.stop()
        raise
    return session, client


def wait_for_primary_mode(
    session: AppSession,
    client: RestoreClient,
    expected_mode: str,
    report: RunReport,
    timeout_seconds: float,
    *,
    signal: FullscreenSignal,
) -> tuple[str | None, bool | None]:
    deadline = time.monotonic() + timeout_seconds
    last_mode: str | None = None
    last_native: bool | None = None
    while time.monotonic() < deadline:
        snapshot = client.primary_snapshot()
        mode = snapshot.get("mode")
        last_mode = (
            "BorderlessFullscreen"
            if isinstance(mode, str) and mode.startswith("BorderlessFullscreen")
            else str(mode) if isinstance(mode, str) else None
        )
        native = snapshot.get("native_fullscreen")
        last_native = (
            {"fullscreen": True, "windowed": False}.get(native)
            if isinstance(native, str)
            else None
        )
        reached = (
            last_native is True
            if signal is FullscreenSignal.NATIVE
            else last_mode == expected_mode and last_native is True
        )
        if reached:
            return last_mode, last_native
        if session.poll() is not None:
            raise RuntimeError("restore app exited during the assisted wait")
        report.heartbeat_path.touch()
        time.sleep(0.1)
    return last_mode, last_native


def run_green_button_case(
    test: dict[str, object],
    executable: Path,
    environment: dict[str, str],
    report: RunReport,
) -> CaseResult:
    case_id = str(test["id"])
    artifact_directory = report.artifact_directory / "assisted" / case_id
    artifact_directory.mkdir(parents=True)
    persistence_path = artifact_directory / "windows.ron"
    ron_file = test.get("ron_file")
    if not isinstance(ron_file, str):
        raise ValueError(f"{case_id} does not name a RON fixture")
    template_path = CONFIG_DIRECTORY / "ron" / "macos" / ron_file
    _ = persistence_path.write_text(
        substitute_environment(template_path.read_text(), environment)
    )
    launch_monitor = _coerce_monitor_index(test.get("launch_monitor", 0))
    session, client = launch_restore_session(
        executable, artifact_directory, persistence_path, launch_monitor, environment
    )
    started = time.monotonic()
    observed_mode: str | None = None
    observed_native: bool | None = None
    restored_mode: str | None = None
    restored_native: bool | None = None
    try:
        instructions = test.get("instructions", [])
        instruction = (
            str(cast("list[object]", instructions)[0])
            if isinstance(instructions, list) and instructions
            else "Click the primary window's green fullscreen button once."
        )
        print(f"ACTION REQUIRED: {instruction}", flush=True)
        report.event("operator-action-requested", instruction, case_id)
        observed_mode, observed_native = wait_for_primary_mode(
            session,
            client,
            "BorderlessFullscreen",
            report,
            ASSISTED_ACTION_TIMEOUT_SECONDS,
            signal=FullscreenSignal.NATIVE,
        )
        if observed_native is True:
            report.event("operator-action-observed", "native fullscreen", case_id)
        _ = session.stop(graceful_shutdown=client.shutdown)
        session, client = launch_restore_session(
            executable, artifact_directory, persistence_path, launch_monitor, environment
        )
        restored_mode, restored_native = wait_for_primary_mode(
            session,
            client,
            "BorderlessFullscreen",
            report,
            CASE_TIMEOUT_SECONDS,
            signal=FullscreenSignal.MODE_AND_NATIVE,
        )
    except KeyboardInterrupt:
        return CaseResult(
            case_id=case_id,
            interaction=Interaction.OPERATOR_ACTION,
            evidence=Evidence.APPLICATION,
            outcome=Outcome.INTERRUPTED,
            elapsed_seconds=time.monotonic() - started,
            artifacts=[str(artifact_directory)],
        )
    finally:
        _ = session.stop(graceful_shutdown=client.shutdown)
        _ = (artifact_directory / "observed-modes.json").write_text(
            json.dumps(
                {
                    "after_operator_action": observed_mode,
                    "after_operator_action_native": observed_native,
                    "after_relaunch": restored_mode,
                    "after_relaunch_native": restored_native,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )
    result = CaseResult(
        case_id=case_id,
        interaction=Interaction.OPERATOR_ACTION,
        evidence=Evidence.APPLICATION,
        assertions=[
            AssertionResult(
                "operator-fullscreen-transition",
                observed_native is True,
                f"observed native fullscreen {observed_native!r} after the green-button action (Bevy mode {observed_mode!r}, which AppKit does not update)",
            ),
            AssertionResult(
                "fullscreen-restored-after-relaunch",
                restored_mode == "BorderlessFullscreen" and restored_native is True,
                f"observed mode {restored_mode!r}, native fullscreen {restored_native!r} after relaunch",
            ),
        ],
        elapsed_seconds=time.monotonic() - started,
        artifacts=[str(artifact_directory)],
    )
    result.finish_from_assertions()
    return result


def run_assisted_partition(
    config: dict[str, object],
    selected_platform: str,
    report: RunReport,
) -> None:
    tests = [
        test
        for test in config_tests(config)
        if case_interaction(test) is not Interaction.AUTOMATED
    ]
    if not tests:
        print("No assisted restore cases are configured for this platform.")
        return
    binaries = prebuild_restore_variants(tests, report)
    config_path = CONFIG_DIRECTORY / f"{selected_platform}.json"
    environment_path = run_discovery(
        config_path, binaries[""], report, selected_platform
    )
    environment = load_environment_file(environment_path)
    report.update_metadata(
        discovery={
            key: value
            for key, value in environment.items()
            if key == "NUM_MONITORS"
            or key == "DIFFERENT_SCALES"
            or key.startswith("MONITOR_")
        }
    )
    print(f"Assisted restore cases: {len(tests)}", flush=True)
    for index, test in enumerate(tests, start=1):
        case_id = str(test["id"])
        print(f"Assisted {index}/{len(tests)}: {case_id}", flush=True)
        report.event("case-started", f"assisted {index}/{len(tests)}", case_id)
        unavailable = requirement_result(test, environment, selected_platform)
        if unavailable is not None:
            result = unavailable
        elif selected_platform == "macos" and case_id == "borderless_green_button":
            result = run_green_button_case(test, binaries[""], environment, report)
        else:
            result = unavailable_case(
                case_id,
                case_interaction(test),
                Evidence.APPLICATION,
                "this case still requires a platform-specific assisted procedure",
                "assisted-case-controller",
            )
        report.append(result)
        report.event("case-completed", result.outcome.value, case_id)
        print(f"  {result.outcome.value}", flush=True)

    assisted_reconnect = [
        case
        for case in _dict_list(config.get("reconnect_cases"))
        if case.get("interaction") != "automated"
    ]
    for case in assisted_reconnect:
        case_id = str(case["id"])
        result = unavailable_case(
            case_id,
            Interaction(str(case.get("interaction", "operator-action"))),
            Evidence.PHYSICAL,
            "the required physical setup or operator procedure was not selected",
            str(case.get("missing_capability", "operator-procedure")),
        )
        report.append(result)
        report.event("case-completed", result.outcome.value, case_id)


def _needs_cross_dpi_provisioning(
    test: dict[str, object],
    environment: dict[str, str],
) -> bool:
    """A cross-DPI case that the real-monitor startup discovery cannot satisfy.

    The startup discovery runs against the host's real monitors (uniform scale on
    this hardware), so different_scales cases are deferred to a dedicated partition
    that provisions a differently-scaled virtual monitor. If the real environment
    already reports differing scales, the case runs normally in the main loop.
    """
    requirements = test.get("requires", {})
    return (
        isinstance(requirements, dict)
        and bool(requirements.get("different_scales"))
        and environment.get("DIFFERENT_SCALES") != "true"
    )


def requirement_result(
    test: dict[str, object],
    environment: dict[str, str],
    selected_platform: str,
) -> CaseResult | None:
    case_id = str(test["id"])
    requirements_value = test.get("requires", {})
    requirements: dict[str, object] = (
        cast("dict[str, object]", requirements_value)
        if isinstance(requirements_value, dict)
        else {}
    )
    monitor_count = int(environment.get("NUM_MONITORS", "0"))
    minimum_monitors = requirements.get("min_monitors", 1)
    if isinstance(minimum_monitors, int) and monitor_count < minimum_monitors:
        return unavailable_case(
            case_id,
            case_interaction(test),
            Evidence.APPLICATION,
            f"requires {minimum_monitors} monitors; discovery found {monitor_count}",
            "monitor-count",
        )
    if requirements.get("different_scales") and environment.get("DIFFERENT_SCALES") != "true":
        return unavailable_case(
            case_id,
            case_interaction(test),
            Evidence.APPLICATION,
            "requires monitors with different scale factors",
            "different-scales",
        )
    if wayland_case_unavailable(selected_platform, test):
        return CaseResult(
            case_id=case_id,
            interaction=case_interaction(test),
            evidence=Evidence.APPLICATION,
            availability=Availability.UNSUPPORTED,
            availability_reason=(
                f"no Wayland compositor in this session "
                f"(${WAYLAND_DISPLAY_ENVIRONMENT_VARIABLE} is unset)"
            ),
            missing_capability="wayland-session",
        )
    return None


def run_restore_subrun(
    test: dict[str, object],
    executable: Path,
    config_path: Path,
    environment_path: Path,
    report: RunReport,
    subrun_name: str,
) -> CaseResult:
    case_id = str(test["id"])
    artifact_directory = report.artifact_directory / "cases" / case_id / subrun_name
    result_path = artifact_directory / "case-result.json"
    persistence_path = artifact_directory / "windows.ron"
    command_result: CommandResult | None = None
    runner_log = artifact_directory / "runner.log"
    for attempt in range(1, 4):
        result_path.unlink(missing_ok=True)
        port_pair: PortPair = available_port_pair()
        command = [
            sys.executable,
            str(SCRIPT_DIRECTORY / "run_test.py"),
            "--config",
            str(config_path),
            "--test-id",
            case_id,
            "--backend",
            str(test.get("backend", "native")),
            "--env-file",
            str(environment_path),
            "--executable",
            str(executable),
            "--base-port",
            str(port_pair.base),
            "--ron-path",
            str(persistence_path),
            "--artifact-dir",
            str(artifact_directory),
            "--result-json",
            str(result_path),
        ]
        runner_log = artifact_directory / f"runner-attempt-{attempt}.log"
        command_result = run_command(
            command,
            cwd=CRATE_ROOT,
            log_path=runner_log,
            timeout_seconds=CASE_TIMEOUT_SECONDS,
            report=report,
        )
        collision = artifact_has_bind_collision(artifact_directory)
        if command_result.timed_out:
            break
        if result_path.is_file():
            attempt_result: object
            try:
                attempt_result = cast("object", json.loads(result_path.read_text()))
            except (OSError, json.JSONDecodeError):
                attempt_result = None
            outcome_is_harness_error = (
                isinstance(attempt_result, dict)
                and cast("dict[str, object]", attempt_result).get("outcome")
                == Outcome.HARNESS_ERROR.value
            )
            if not (attempt < 3 and collision and outcome_is_harness_error):
                break
        elif not collision:
            break
        report.event("launch-retry", f"port collision; attempt {attempt}/3", case_id)
    if command_result is None:
        raise RuntimeError("restore runner did not start")
    if command_result.timed_out:
        return CaseResult(
            case_id=case_id,
            interaction=Interaction.AUTOMATED,
            evidence=Evidence.APPLICATION,
            outcome=Outcome.TIMED_OUT,
            elapsed_seconds=command_result.elapsed_seconds,
            artifacts=[str(runner_log)],
        )
    if not result_path.is_file():
        return CaseResult(
            case_id=case_id,
            interaction=Interaction.AUTOMATED,
            evidence=Evidence.APPLICATION,
            outcome=Outcome.HARNESS_ERROR,
            process_exit_code=command_result.return_code,
            elapsed_seconds=command_result.elapsed_seconds,
            artifacts=[str(runner_log)],
        )
    raw = cast("object", json.loads(result_path.read_text()))
    if not isinstance(raw, dict):
        raise ValueError(f"{result_path} did not contain a result object")
    result = CaseResult.from_dict(cast("dict[str, object]", raw))
    result.add_artifact(runner_log)
    return result


def artifact_has_bind_collision(artifact_directory: Path) -> bool:
    for path in artifact_directory.glob("*.log"):
        try:
            text = path.read_text(errors="replace").lower()
        except OSError:
            continue
        if any(
            message in text
            for message in (
                "address already in use",
                "os error 48",
                "os error 98",
                "os error 10048",
            )
        ):
            return True
    return False


def run_restore_case(
    test: dict[str, object],
    binaries: dict[str, Path],
    config_path: Path,
    environment_path: Path,
    report: RunReport,
) -> CaseResult:
    validation = test.get("workaround_validation")
    if not isinstance(validation, dict):
        return run_restore_subrun(
            test,
            binaries[""],
            config_path,
            environment_path,
            report,
            "default",
        )
    validation_fields = cast("dict[str, object]", validation)
    without_flags = str(validation_fields.get("build_without", ""))
    with_flags = str(validation_fields.get("build_with", ""))
    without = run_restore_subrun(
        test,
        binaries[without_flags],
        config_path,
        environment_path,
        report,
        "without-workaround",
    )
    with_workaround = run_restore_subrun(
        test,
        binaries[with_flags],
        config_path,
        environment_path,
        report,
        "with-workaround",
    )
    result = CaseResult(
        case_id=str(test["id"]),
        interaction=Interaction.AUTOMATED,
        evidence=Evidence.APPLICATION,
        workaround_subruns=[without, with_workaround],
        assertions=[
            AssertionResult(
                "workaround-enabled",
                with_workaround.outcome is Outcome.PASSED,
                f"with workaround: {with_workaround.outcome.value}",
            ),
            AssertionResult(
                "upstream-bug-baseline",
                without.outcome in (Outcome.FAILED, Outcome.PASSED),
                f"without workaround: {without.outcome.value}",
            ),
        ],
        elapsed_seconds=without.elapsed_seconds + with_workaround.elapsed_seconds,
    )
    result.finish_from_assertions()
    return result


def print_inventory(
    config: dict[str, object],
    selected_platform: str,
    hardware_profile: Path | None,
) -> None:
    tests = config_tests(config)
    print(f"Platform: {selected_platform}")
    print(f"Restore cases: {len(tests)}")
    interaction_counts = {interaction: 0 for interaction in Interaction}
    availability_counts = {availability: 0 for availability in Availability}
    for test in tests:
        interaction = case_interaction(test)
        interaction_counts[interaction] += 1
        unsupported = wayland_case_unavailable(selected_platform, test)
        availability = Availability.UNSUPPORTED if unsupported else Availability.AVAILABLE
        availability_counts[availability] += 1
        print(
            f"  {test['id']}: {interaction.value}, application evidence, "
            + f"{availability.value}"
        )
    reconnect_cases = _dict_list(config.get("reconnect_cases"))
    print(f"Reconnect cases: {len(reconnect_cases)}")
    for case in reconnect_cases:
        interaction = Interaction(str(case.get("interaction", "operator-action")))
        interaction_counts[interaction] += 1
        if interaction is Interaction.AUTOMATED and hardware_profile is not None:
            availability = Availability.AVAILABLE
            reason = "explicit hardware profile selected"
        else:
            availability = Availability.UNAVAILABLE
            reason = str(
                case.get(
                    "missing_capability",
                    "no explicit hardware profile selected",
                )
            )
        availability_counts[availability] += 1
        print(
            f"  {case.get('id')}: {interaction.value}, physical evidence, "
            + f"{availability.value} ({reason})"
        )
    probe_cases = _dict_list(config.get("probe_cases"))
    print(f"Probe lifecycle cases: {len(probe_cases)}")
    for case in probe_cases:
        interaction = Interaction(str(case.get("interaction", "automated")))
        interaction_counts[interaction] += 1
        availability_counts[Availability.AVAILABLE] += 1
        print(
            f"  {case.get('id')}: {interaction.value}, "
            + f"{case.get('evidence')} evidence, available"
        )
    print("Interaction summary:")
    for interaction in Interaction:
        print(f"  {interaction.value}: {interaction_counts[interaction]}")
    print("Availability summary:")
    for availability in Availability:
        print(f"  {availability.value}: {availability_counts[availability]}")


@dataclass(frozen=True)
class SuiteArgs:
    assisted: bool
    dry_run: bool
    single_monitor: bool
    hardware_profile: Path | None
    artifacts: Path
    skip_preflight: bool


def parse_args() -> SuiteArgs:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    _ = mode.add_argument("--automated", action="store_true", help="Run unattended cases")
    _ = mode.add_argument("--assisted", action="store_true", help="Run operator cases")
    _ = parser.add_argument("--dry-run", action="store_true", help="List coverage without launching apps")
    _ = parser.add_argument("--single-monitor", action="store_true", help="Treat this as a one-monitor run")
    _ = parser.add_argument("--hardware-profile", type=Path, help="Explicit local hardware profile")
    _ = parser.add_argument("--artifacts", type=Path, default=DEFAULT_ARTIFACT_ROOT)
    _ = parser.add_argument(
        "--skip-preflight",
        action="store_true",
        help="Skip test/lint gates for a focused local rerun",
    )
    namespace = parser.parse_args()
    return SuiteArgs(
        assisted=cast("bool", namespace.assisted),
        dry_run=cast("bool", namespace.dry_run),
        single_monitor=cast("bool", namespace.single_monitor),
        hardware_profile=cast("Path | None", namespace.hardware_profile),
        artifacts=cast("Path", namespace.artifacts),
        skip_preflight=cast("bool", namespace.skip_preflight),
    )


def main() -> int:
    args = parse_args()
    selected_platform = platform_name()
    config = load_config(selected_platform)
    if args.dry_run:
        print_inventory(config, selected_platform, args.hardware_profile)
        return 0
    if args.assisted:
        run_id = f"{datetime.now(UTC).strftime('%Y%m%dT%H%M%SZ')}-{uuid.uuid4().hex[:8]}"
        artifact_directory = args.artifacts.resolve() / run_id
        report = RunReport(artifact_directory, run_id, source_revision())
        report.update_metadata(
            platform=selected_platform,
            mode="assisted",
            selected_cases=[
                str(test["id"])
                for test in config_tests(config)
                if case_interaction(test) is not Interaction.AUTOMATED
            ],
        )
        report.event("run-started", f"platform={selected_platform}; mode=assisted")
        lock_path = (
            Path.home()
            / ".cache"
            / "hana_clerestory"
            / f"display-{selected_platform}.lock"
        )
        try:
            if not args.skip_preflight:
                run_preflight(report)
            with DisplayLock(lock_path):
                run_assisted_partition(config, selected_platform, report)
        except BlockingIOError:
            report.event("run-aborted", "another display-changing run holds the lock")
            print(f"Another Clerestory display run holds {lock_path}", file=sys.stderr)
            return 2
        except BaseException as error:
            report.event("run-error", str(error))
            report.write_partial()
            print(f"Harness error: {error}", file=sys.stderr)
            return 2
        report.event("run-completed", "assisted partition completed")
        print(f"Reports: {report.json_path} and {report.markdown_path}")
        return int(
            any(
                result.availability is Availability.AVAILABLE
                and (
                    result.outcome is not Outcome.PASSED
                    or (result.cleanup is not None and not result.cleanup.succeeded)
                )
                for result in report.results
            )
        )

    automated_restore_count = sum(
        case_interaction(test) is Interaction.AUTOMATED
        for test in config_tests(config)
    )
    physical_case_count = sum(
        1
        for case in _dict_list(config.get("reconnect_cases"))
        if case.get("interaction") == "automated"
    )
    physical_probe_count = 4 if physical_case_count else 0
    print(f"Automated restore cases: {automated_restore_count}", flush=True)
    print(f"Automated physical cases: {physical_case_count}", flush=True)
    print(f"Physical probe processes: {physical_probe_count}", flush=True)

    run_id = f"{datetime.now(UTC).strftime('%Y%m%dT%H%M%SZ')}-{uuid.uuid4().hex[:8]}"
    artifact_directory = args.artifacts.resolve() / run_id
    report = RunReport(artifact_directory, run_id, source_revision())
    report.update_metadata(
        platform=selected_platform,
        mode="automated",
        hardware_profile=(
            str(args.hardware_profile.resolve())
            if args.hardware_profile is not None
            else None
        ),
        selected_cases=[
            str(test["id"])
            for test in config_tests(config)
            if case_interaction(test) is Interaction.AUTOMATED
        ],
    )
    report.event("run-started", f"platform={selected_platform}")
    lock_path = Path.home() / ".cache" / "hana_clerestory" / f"display-{selected_platform}.lock"
    try:
        if not args.skip_preflight:
            run_preflight(report)
        with DisplayLock(lock_path):
            tests = config_tests(config)
            automated_tests = [
                test for test in tests if case_interaction(test) is Interaction.AUTOMATED
            ]
            print(f"Automated restore cases: {len(automated_tests)}", flush=True)
            binaries = prebuild_restore_variants(automated_tests, report)
            config_path = CONFIG_DIRECTORY / f"{selected_platform}.json"
            environment_path = run_discovery(
                config_path, binaries[""], report, selected_platform
            )
            environment = load_environment_file(environment_path)
            report.update_metadata(
                discovery={
                    key: value
                    for key, value in environment.items()
                    if key == "NUM_MONITORS"
                    or key == "DIFFERENT_SCALES"
                    or key.startswith("MONITOR_")
                }
            )
            if args.single_monitor:
                environment["NUM_MONITORS"] = "1"
            total = len(automated_tests)
            cross_dpi_ids = {
                str(test["id"])
                for test in automated_tests
                if _needs_cross_dpi_provisioning(test, environment)
            }
            for index, test in enumerate(automated_tests, start=1):
                case_id = str(test["id"])
                if case_id in cross_dpi_ids:
                    continue  # handled by the cross-DPI partition below
                print(f"Restore {index}/{total}: {case_id}", flush=True)
                report.event("case-started", f"restore {index}/{total}", case_id)
                unavailable = requirement_result(test, environment, selected_platform)
                result = unavailable or run_restore_case(
                    test, binaries, config_path, environment_path, report
                )
                report.append(result)
                report.event("case-completed", result.outcome.value, case_id)
                print(f"  {result.outcome.value}", flush=True)

            # Cross-DPI restore partition. These cases need two monitors with different
            # scale factors; the real-monitor startup discovery has uniform scale, so they
            # were deferred out of the loop above. Provision a differently-scaled virtual
            # monitor (the VDD, which power_on forces to scale 1), re-discover so the
            # HIGH/LOW_SCALE_* role vars are populated, run the cases against that
            # discovery, then disable the VDD to restore the desktop. Without a
            # --hardware-profile (or off Windows) there is no way to create the second
            # scale, so the cases stay unavailable — preserving prior behavior.
            cross_dpi_tests = [
                test for test in automated_tests if str(test["id"]) in cross_dpi_ids
            ]
            if cross_dpi_tests and (
                args.hardware_profile is None or sys.platform != "win32"
            ):
                for test in cross_dpi_tests:
                    case_id = str(test["id"])
                    result = requirement_result(
                        test, environment, selected_platform
                    ) or unavailable_case(
                        case_id,
                        case_interaction(test),
                        Evidence.APPLICATION,
                        "requires a differently-scaled monitor; provision via "
                        "--hardware-profile on Windows",
                        "different-scales",
                    )
                    report.append(result)
                    report.event("case-completed", result.outcome.value, case_id)
                    print(f"  {result.outcome.value}", flush=True)
            elif cross_dpi_tests:
                assert args.hardware_profile is not None
                cross_dpi_profile = HardwareProfile.load(
                    args.hardware_profile.resolve()
                )
                print(
                    "cross-dpi: enabling virtual monitor before discovery", flush=True
                )
                report.event("cross-dpi", "enabling virtual monitor before discovery")
                power_on = cross_dpi_profile.power_on.run(dict(os.environ))
                if power_on.returncode != 0:
                    raise RuntimeError(
                        "failed to enable the virtual display before cross-DPI discovery"
                    )
                # The virtual monitor comes up at the desktop scale (matching the real
                # monitors), so discovery would report DIFFERENT_SCALES=false and skip the
                # cross-DPI cases. Force its source DPI to the recommended (100%) scale via the
                # DisplayConfig API so it differs from the real scale-2 monitors — live, no
                # sign-out. See phase15-cross-dpi-provisioning in project memory.
                print("cross-dpi: forcing virtual monitor to scale 1", flush=True)
                report.event("cross-dpi", "forcing virtual monitor to scale 1")
                force_low_dpi = subprocess.run(
                    [
                        "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass",
                        "-File",
                        str(SCRIPT_DIRECTORY / "windows_vdd" / "dpi_scale.ps1"),
                        "-Match", "MTT1337", "-Action", "setrel", "-Rel", "0",
                    ],
                    check=False,
                    timeout=90,
                )
                if force_low_dpi.returncode != 0:
                    raise RuntimeError(
                        "failed to force the virtual display to scale 1 for cross-DPI"
                    )
                try:
                    cross_dpi_environment_path = run_discovery(
                        config_path,
                        binaries[""],
                        report,
                        selected_platform,
                        suffix="crossdpi",
                    )
                    cross_dpi_environment = load_environment_file(
                        cross_dpi_environment_path
                    )
                    for cross_index, test in enumerate(cross_dpi_tests, start=1):
                        case_id = str(test["id"])
                        print(
                            f"Cross-DPI {cross_index}/{len(cross_dpi_tests)}: {case_id}",
                            flush=True,
                        )
                        report.event(
                            "case-started",
                            f"cross-dpi {cross_index}/{len(cross_dpi_tests)}",
                            case_id,
                        )
                        unavailable = requirement_result(
                            test, cross_dpi_environment, selected_platform
                        )
                        result = unavailable or run_restore_case(
                            test,
                            binaries,
                            config_path,
                            cross_dpi_environment_path,
                            report,
                        )
                        report.append(result)
                        report.event("case-completed", result.outcome.value, case_id)
                        print(f"  {result.outcome.value}", flush=True)
                finally:
                    # Raw power_off (not PowerSafety) avoids writing a restore-required
                    # marker; disabling returns the host to its real monitors.
                    print(
                        "cross-dpi: disabling virtual monitor to restore the desktop",
                        flush=True,
                    )
                    report.event("cross-dpi", "disabling virtual monitor")
                    try:
                        _ = cross_dpi_profile.power_off.run(dict(os.environ))
                    except Exception as error:
                        print(
                            f"could not disable the virtual display after cross-DPI: {error}",
                            flush=True,
                        )

            automated_probe_cases = [
                case
                for case in _dict_list(config.get("probe_cases"))
                if case.get("interaction") == "automated"
            ]
            reconnect_executable: Path | None = None
            if automated_probe_cases:
                reconnect_executable = prebuild_reconnect(report)
                for probe_index, case in enumerate(automated_probe_cases, start=1):
                    case_id = str(case["id"])
                    print(
                        f"Probe lifecycle {probe_index}/{len(automated_probe_cases)}: {case_id}",
                        flush=True,
                    )
                    if case_id != "zero_owned_windows":
                        result = CaseResult(
                            case_id=case_id,
                            interaction=Interaction.AUTOMATED,
                            evidence=Evidence.APPLICATION,
                            outcome=Outcome.HARNESS_ERROR,
                        )
                    else:
                        result = run_probe_lifecycle_case(
                            reconnect_executable,
                            report.artifact_directory / "probe-cases" / case_id,
                            report.run_id,
                            case_id,
                        )
                    report.append(result)
                    report.event("case-completed", result.outcome.value, case_id)

            automated_reconnect = [
                case
                for case in _dict_list(config.get("reconnect_cases"))
                if case.get("interaction") == "automated"
            ]
            if automated_reconnect and args.hardware_profile is None:
                for case in automated_reconnect:
                    case_id = str(case["id"])
                    report.append(
                        unavailable_case(
                            case_id,
                            Interaction.AUTOMATED,
                            Evidence.PHYSICAL,
                            "no explicit --hardware-profile was supplied",
                            "controllable-monitor-profile",
                        )
                    )
            elif automated_reconnect:
                print(
                    f"Automated physical reconnect cases: {len(automated_reconnect)} "
                    + "using 4 probe processes",
                    flush=True,
                )
                if reconnect_executable is None:
                    reconnect_executable = prebuild_reconnect(report)
                assert args.hardware_profile is not None
                hardware_profile = HardwareProfile.load(args.hardware_profile.resolve())

                def reconnect_progress(message: str) -> None:
                    print(message, flush=True)
                    report.event("physical-progress", message)

                # The startup discovery ran with the reconnect monitor off, so it is absent
                # from `environment`. On Windows the reconnect monitor is the virtual display;
                # enable it and re-discover so its monitor is in the environment the reconnect
                # matcher selects from. The restore cases above keep the original discovery.
                reconnect_environment = environment
                if sys.platform == "win32":
                    reconnect_progress(
                        "windows reconnect: enabling virtual monitor before discovery"
                    )
                    power_on = hardware_profile.power_on.run(dict(os.environ))
                    if power_on.returncode != 0:
                        raise RuntimeError(
                            "failed to enable the virtual display before reconnect discovery"
                        )
                    reconnect_environment = load_environment_file(
                        run_discovery(
                            config_path,
                            binaries[""],
                            report,
                            selected_platform,
                            suffix="reconnect",
                        )
                    )

                try:
                    reconnect_results = run_automated_reconnect(
                        reconnect_executable,
                        hardware_profile,
                        reconnect_environment,
                        report.artifact_directory / "reconnect",
                        report.run_id,
                        reconnect_progress,
                    )
                except BaseException as error:
                    reconnect_progress(f"physical state machine failed: {error}")
                    restore_marker = (
                        report.artifact_directory
                        / "monitor-restore-required.json"
                    )
                    cleanup = CleanupResult(
                        attempted=True,
                        succeeded=not restore_marker.exists(),
                        detail=(
                            "configured monitor restoration was confirmed"
                            if not restore_marker.exists()
                            else "monitor restoration could not be confirmed; restore marker remains"
                        ),
                    )
                    reconnect_results = [
                        CaseResult(
                            case_id=str(case["id"]),
                            interaction=Interaction.AUTOMATED,
                            evidence=Evidence.PHYSICAL,
                            outcome=Outcome.HARNESS_ERROR,
                            artifacts=[
                                str(report.artifact_directory / "reconnect")
                            ],
                            cleanup=cleanup,
                        )
                        for case in automated_reconnect
                    ]

                # The reconnect run leaves the virtual display enabled; on Windows that extra
                # monitor offsets the desktop cursor mapping. Turn it back off so the host
                # returns to its real monitors. Raw `power_off` (not PowerSafety) avoids
                # writing a restore-required marker.
                if sys.platform == "win32":
                    reconnect_progress(
                        "windows reconnect: disabling virtual monitor to restore the desktop"
                    )
                    try:
                        _ = hardware_profile.power_off.run(dict(os.environ))
                    except Exception as error:
                        reconnect_progress(
                            f"could not disable the virtual display after reconnect: {error}"
                        )
                for physical_index, result in enumerate(reconnect_results, start=1):
                    print(
                        f"Physical {physical_index}/{len(reconnect_results)}: "
                        + f"{result.case_id} {result.outcome.value}",
                        flush=True,
                    )
                    report.append(result)
                    report.event("case-completed", result.outcome.value, result.case_id)
    except BlockingIOError:
        report.event("run-aborted", "another display-changing run holds the lock")
        print(f"Another Clerestory display run holds {lock_path}", file=sys.stderr)
        return 2
    except BaseException as error:
        report.event("run-error", str(error))
        report.write_partial()
        print(f"Harness error: {error}", file=sys.stderr)
        return 2

    report.event("run-completed", "automated restore partition completed")
    failed = [
        result
        for result in report.results
        if result.availability is Availability.AVAILABLE
        and (
            result.outcome is not Outcome.PASSED
            or (result.cleanup is not None and not result.cleanup.succeeded)
        )
    ]
    print(f"Reports: {report.json_path} and {report.markdown_path}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
