#!/usr/bin/env python3
"""Run the physical reconnect cases supported by an explicit hardware profile."""

from __future__ import annotations

import argparse
import json
import os
import secrets
import sys
import time
import uuid
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import cast
from typing import final

from clerestory_test.app_session import AppSession
from clerestory_test.app_session import available_port_pair
from clerestory_test.case_result import AssertionResult
from clerestory_test.case_result import CaseResult
from clerestory_test.case_result import CleanupResult
from clerestory_test.case_result import Evidence
from clerestory_test.case_result import Interaction
from clerestory_test.case_result import Outcome
from clerestory_test.hardware_profile import HardwareProfile
from clerestory_test.probe_client import ProbeClient


AUTOMATIC_WINDOW_KEYS = ("primary", "hotplug-automatic")
# Monitor index a probe starts on before the snapshot says which panels the host can fingerprint.
BOOTSTRAP_MONITOR_INDEX = 0
# Subdirectory holding the second probe's artifacts when the bootstrap monitor is unidentifiable,
# so its logs and `windows.ron` do not overwrite the bootstrap probe's.
VERIFIED_MONITOR_ARTIFACTS = "verified-monitor"
# WARP software rendering on a GPU-less Windows host (the VMware test guest) makes the
# per-window surface reconfiguration during a reconnect very slow; give the cycle far more
# time there. Hardware renderers (macOS, GPU hosts) complete quickly on the original bounds.
if sys.platform == "win32":
    PROBE_START_TIMEOUT_SECONDS = 90.0
    MONITOR_CHANGE_TIMEOUT_SECONDS = 60.0
    RECOVERY_TIMEOUT_SECONDS = 60.0
else:
    PROBE_START_TIMEOUT_SECONDS = 45.0
    MONITOR_CHANGE_TIMEOUT_SECONDS = 45.0
    RECOVERY_TIMEOUT_SECONDS = 60.0

# On the GPU-less Windows VM the WARP software rasterizer intermittently deadlocks or
# clamps a window surface during the reconnect topology change (an environment limitation,
# not a defect in the reconnect logic, which is proven correct). Retry the whole windowed
# cycle a few times with a fresh probe; a non-flaky attempt passes cleanly.
WINDOWS_RECONNECT_ATTEMPTS = 4 if sys.platform == "win32" else 1


def _dict_list(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list):
        return []
    return [
        cast("dict[str, object]", item)
        for item in cast("list[object]", value)
        if isinstance(item, dict)
    ]


def _as_int(value: object, default: int = 0) -> int:
    if isinstance(value, int):
        return int(value)
    if isinstance(value, str):
        return int(value)
    return default


def _as_float(value: object, default: float = 0.0) -> float:
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str):
        return float(value)
    return default


def _recovery_counts(window: dict[str, object]) -> dict[str, object] | None:
    counts = window.get("recovery_counts")
    return cast("dict[str, object]", counts) if isinstance(counts, dict) else None


@dataclass(frozen=True)
class CycleEvidence:
    before: dict[str, object]
    fallback: dict[str, object]
    returned: dict[str, object]
    records: list[dict[str, object]]
    initial_verified_id: str


@final
class ProbeProcess:
    def __init__(
        self,
        executable: Path,
        artifact_directory: Path,
        monitor_index: int,
        startup_mode: str,
        suite_run_id: str,
    ) -> None:
        self.executable = executable
        self.artifact_directory = artifact_directory
        self.monitor_index = monitor_index
        self.startup_mode = startup_mode
        self.run_id = f"{suite_run_id}-{startup_mode}-{uuid.uuid4().hex[:8]}"
        self.boot_nonce = secrets.token_hex(16)
        self.capability = secrets.token_urlsafe(32)
        self.target_verified_id: str | None = None
        self.session, self.client = self._new_session(1)
        self.snapshot_number = 0

    def _new_session(self, attempt: int) -> tuple[AppSession, ProbeClient]:
        port_pair = available_port_pair()
        client = ProbeClient(port_pair.base, self.capability)
        environment = dict(os.environ)
        # Match the restore partition, which forces X11 on Linux (`run_test.py`, backend "x11").
        # Under Wayland the compositor exposes no stable physical monitor identity, so every
        # monitor stays `Unverified` and a windowed automatic key never arms; the probe then never
        # reports `recovery-ready` and every case here times out waiting for a state this backend
        # cannot reach.
        if sys.platform.startswith("linux"):
            environment["WAYLAND_DISPLAY"] = ""
        environment.update(
            {
                "CLERESTORY_PROBE_RUN_ID": self.run_id,
                "CLERESTORY_PROBE_BOOT_NONCE": self.boot_nonce,
                "CLERESTORY_PROBE_CAPABILITY": self.capability,
                "CLERESTORY_PROBE_PORT": str(port_pair.base),
                "CLERESTORY_PROBE_RENDER_PORT": str(port_pair.render),
                "CLERESTORY_PROBE_MONITOR_INDEX": str(self.monitor_index),
                "CLERESTORY_PROBE_STARTUP_MODE": self.startup_mode,
                "CLERESTORY_PROBE_PERSISTENCE_PATH": str(
                    self.artifact_directory / "windows.ron"
                ),
            }
        )
        session = AppSession(
            executable=self.executable,
            argv=[],
            environment=environment,
            stdout_path=self.artifact_directory / f"probe-{attempt}.stdout.log",
            stderr_path=self.artifact_directory / f"probe-{attempt}.stderr.log",
            working_directory=Path.cwd(),
        )
        return session, client

    def start(self, *, pin_verified_identity: bool = True) -> dict[str, object]:
        """Launch the probe and wait for its ready snapshot.

        Pinning resolves the target monitor by verified id afterwards, which is what keeps a
        reconnect case following the same panel once a disconnect renumbers the winit indices.
        Cases that never re-resolve the target pass `pin_verified_identity=False` so a host
        whose panels expose no EDID can still run them; the cases that do depend on identity
        demand it themselves and still fail loudly when it is missing.
        """
        self.artifact_directory.mkdir(parents=True, exist_ok=True)
        for attempt in range(1, 4):
            if attempt > 1:
                self.session, self.client = self._new_session(attempt)
            self.session.start()
            try:
                snapshot = self.client.wait_ready(
                    self.run_id, self.boot_nonce, PROBE_START_TIMEOUT_SECONDS
                )
            except BaseException:
                _ = self.session.stop()
                stderr = self.session.stderr_path
                if attempt < 3 and _is_bind_collision(stderr):
                    continue
                raise
            if pin_verified_identity:
                self.target_verified_id = verified_id(self.target_monitor(snapshot))
            self.save_snapshot("ready", snapshot)
            return snapshot
        raise RuntimeError("probe launch retries were exhausted")

    def target_monitor(self, snapshot: dict[str, object]) -> dict[str, object]:
        return probe_target_monitor(
            snapshot,
            self.monitor_index,
            self.target_verified_id,
        )

    def save_snapshot(self, label: str, snapshot: dict[str, object]) -> None:
        self.snapshot_number += 1
        path = self.artifact_directory / f"snapshot-{self.snapshot_number:03d}-{label}.json"
        _ = path.write_text(json.dumps(snapshot, indent=2, sort_keys=True) + "\n")

    def save_records(self, records: list[dict[str, object]]) -> None:
        path = self.artifact_directory / "probe-records.jsonl"
        with path.open("a") as output:
            for record in records:
                _ = output.write(json.dumps(record, sort_keys=True) + "\n")

    def stop(self) -> None:
        _ = self.session.stop(graceful_shutdown=self.client.shutdown)


def _is_bind_collision(stderr_path: Path) -> bool:
    try:
        stderr = stderr_path.read_text(errors="replace").lower()
    except OSError:
        return False
    return any(
        text in stderr
        for text in (
            "address already in use",
            "os error 48",
            "os error 98",
            "os error 10048",
        )
    )


@final
class PowerSafety:
    def __init__(
        self,
        profile: HardwareProfile,
        marker: Path,
        progress: Callable[[str], None],
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        self.profile = profile
        self.marker = marker
        self.progress = progress
        self.sleep = sleep
        self.environment = dict(os.environ)
        self.inventory_path = marker.parent / "hardware-inventory.jsonl"
        self.action_path = marker.parent / "hardware-actions.jsonl"

    def record_action(self, action: str, started: float, return_code: int) -> None:
        self.action_path.parent.mkdir(parents=True, exist_ok=True)
        with self.action_path.open("a") as output:
            _ = output.write(
                json.dumps(
                    {
                        "action": action,
                        "elapsed_seconds": time.monotonic() - started,
                        "return_code": return_code,
                        "timestamp_unix_seconds": time.time(),
                    },
                    sort_keys=True,
                )
                + "\n"
            )

    def ensure_on(self) -> None:
        self.progress("requesting configured monitor power on")
        started = time.monotonic()
        completed = self.profile.power_on.run(self.environment)
        self.record_action("power-on", started, completed.returncode)
        self.sleep(self.profile.minimum_on_seconds)
        self.wait_for_inventory(1, MONITOR_CHANGE_TIMEOUT_SECONDS)
        self.marker.unlink(missing_ok=True)

    def power_off(self) -> None:
        self.marker.parent.mkdir(parents=True, exist_ok=True)
        _ = self.marker.write_text(
            json.dumps(
                {
                    "profile": self.profile.name,
                    "target": self.profile.target_matcher,
                    "created_unix_seconds": time.time(),
                },
                sort_keys=True,
            )
            + "\n"
        )
        self.progress("requesting configured monitor power off")
        started = time.monotonic()
        completed = self.profile.power_off.run(self.environment)
        self.record_action("power-off", started, completed.returncode)
        self.sleep(self.profile.minimum_off_seconds)

    def wait_for_inventory(self, expected_count: int, timeout_seconds: float) -> None:
        deadline = time.monotonic() + timeout_seconds
        last_count = -1
        while time.monotonic() < deadline:
            inventory, count = self.profile.inventory_snapshot(self.environment)
            if count != last_count:
                self.inventory_path.parent.mkdir(parents=True, exist_ok=True)
                with self.inventory_path.open("a") as output:
                    _ = output.write(
                        json.dumps(
                            {
                                "timestamp_unix_seconds": time.time(),
                                "matching_target_count": count,
                                "inventory": inventory,
                            },
                            sort_keys=True,
                        )
                        + "\n"
                    )
            last_count = count
            if last_count == expected_count:
                return
            self.sleep(0.5)
        raise TimeoutError(
            f"expected {expected_count} matching monitor inventory entries; last count was {last_count}"
        )


def raise_preserving_failures(
    run_failure: BaseException | None,
    cleanup_failure: BaseException | None,
) -> None:
    if run_failure is not None and cleanup_failure is not None:
        raise BaseExceptionGroup(
            "physical reconnect run and monitor restoration both failed",
            [run_failure, cleanup_failure],
        )
    if run_failure is not None:
        raise run_failure
    if cleanup_failure is not None:
        raise cleanup_failure


def monitor_index(
    environment: dict[str, str], profile: HardwareProfile
) -> int:
    monitor_count = int(environment.get("NUM_MONITORS", "0"))
    matches = [
        index
        for index in range(monitor_count)
        if profile.probe_monitor_matcher.matches(environment, index)
    ]
    if len(matches) != 1:
        raise RuntimeError(
            "expected the probe monitor matcher to select exactly one discovered "
            + f"monitor; found {len(matches)}"
        )
    return matches[0]


def monitor_at_index(snapshot: dict[str, object], index: int) -> dict[str, object]:
    monitors = snapshot.get("monitors", [])
    if not isinstance(monitors, list):
        raise RuntimeError("probe monitor inventory was malformed")
    matches = [
        monitor for monitor in _dict_list(cast("list[object]", monitors)) if monitor.get("index") == index
    ]
    if len(matches) != 1:
        raise RuntimeError(
            f"expected one probe monitor at index {index}; found {len(matches)}"
        )
    return matches[0]


def monitor_with_verified_id(
    snapshot: dict[str, object], target_verified_id: str
) -> dict[str, object]:
    monitors = snapshot.get("monitors", [])
    if not isinstance(monitors, list):
        raise RuntimeError("probe monitor inventory was malformed")
    matches = [
        monitor
        for monitor in _dict_list(cast("list[object]", monitors))
        if monitor.get("verified_id") == target_verified_id
    ]
    if len(matches) != 1:
        raise RuntimeError(
            f"expected one probe monitor with {target_verified_id}; found {len(matches)}"
        )
    return matches[0]


def probe_target_monitor(
    snapshot: dict[str, object],
    bootstrap_index: int,
    target_verified_id: str | None,
) -> dict[str, object]:
    if target_verified_id is not None:
        return monitor_with_verified_id(snapshot, target_verified_id)
    return monitor_at_index(snapshot, bootstrap_index)


def verified_id(monitor: dict[str, object]) -> str:
    value = monitor.get("verified_id")
    if not isinstance(value, str) or not value:
        raise RuntimeError("target monitor did not have a verified identity")
    return value


def verified_monitor_index(snapshot: dict[str, object]) -> int | None:
    """Lowest monitor index the app fingerprinted, or `None` when it fingerprinted none of them."""
    indices = [
        _as_int(monitor.get("index"))
        for monitor in _dict_list(snapshot.get("monitors"))
        if isinstance(identity := monitor.get("verified_id"), str) and identity
    ]
    return min(indices) if indices else None


def windows_by_key(snapshot: dict[str, object]) -> dict[str, dict[str, object]]:
    windows = snapshot.get("windows", [])
    if not isinstance(windows, list):
        raise RuntimeError("probe windows were malformed")
    return {
        str(window["key"]): window
        for window in _dict_list(cast("list[object]", windows))
        if "key" in window
    }


def window_key_count(snapshot: dict[str, object], key: str) -> int:
    windows = snapshot.get("windows", [])
    if not isinstance(windows, list):
        return 0
    return sum(1 for window in _dict_list(cast("list[object]", windows)) if window.get("key") == key)


def record_fields(record: dict[str, object]) -> dict[str, str]:
    fields = record.get("fields", {})
    if not isinstance(fields, dict):
        return {}
    return {
        str(name): str(value)
        for name, value in cast("dict[object, object]", fields).items()
    }


def wait_for_probe_state(
    probe: ProbeProcess,
    cursor: int,
    expected: str,
    predicate: Callable[[dict[str, object], list[dict[str, object]]], bool],
    timeout_seconds: float,
) -> tuple[dict[str, object], list[dict[str, object]]]:
    deadline = time.monotonic() + timeout_seconds
    quiet_period_seconds = 0.25
    matched_since: float | None = None
    last_snapshot: dict[str, object] = {}
    last_records: list[dict[str, object]] = []
    while time.monotonic() < deadline:
        last_snapshot = probe.client.snapshot()
        response = probe.client.records(cursor)
        records = response.get("records", [])
        if isinstance(records, list):
            last_records = _dict_list(cast("list[object]", records))
        if predicate(last_snapshot, last_records):
            if matched_since is None:
                matched_since = time.monotonic()
            if time.monotonic() - matched_since < quiet_period_seconds:
                time.sleep(0.05)
                continue
            probe.save_snapshot(expected.replace(" ", "-"), last_snapshot)
            probe.save_records(last_records)
            return last_snapshot, last_records
        matched_since = None
        if probe.session.poll() is not None:
            raise RuntimeError(f"probe exited while waiting for {expected}")
        time.sleep(0.1)
    probe.save_snapshot("timed-out", last_snapshot)
    probe.save_records(last_records)
    raise TimeoutError(f"timed out waiting for {expected}")


def run_cycle(
    probe: ProbeProcess,
    power: PowerSafety,
    initial_snapshot: dict[str, object],
    *,
    expect_automatic_return: bool,
    expect_application_return: bool = False,
    expect_native_fullscreen: bool = False,
    automatic_return_keys: tuple[str, ...] = AUTOMATIC_WINDOW_KEYS,
    fallback_window_keys: tuple[str, ...] = AUTOMATIC_WINDOW_KEYS,
) -> CycleEvidence:
    initial_target = probe.target_monitor(initial_snapshot)
    initial_verified_id = verified_id(initial_target)
    cursor = _as_int(initial_snapshot.get("record_cursor", 0))
    power.power_off()
    power.wait_for_inventory(0, MONITOR_CHANGE_TIMEOUT_SECONDS)

    def disconnected(
        snapshot: dict[str, object], records: list[dict[str, object]]
    ) -> bool:
        monitor_identities = {
            monitor.get("verified_id")
            for monitor in _dict_list(snapshot.get("monitors"))
        }
        removal = any(
            record.get("kind") == "monitor-disconnected"
            and initial_verified_id in str(record_fields(record).get("monitor", ""))
            for record in records
        )
        automatic_present = all(
            key in windows_by_key(snapshot) for key in fallback_window_keys
        )
        return (
            initial_verified_id not in monitor_identities
            and removal
            and automatic_present
        )

    fallback, _ = wait_for_probe_state(
        probe,
        cursor,
        "target removal and fallback",
        disconnected,
        MONITOR_CHANGE_TIMEOUT_SECONDS,
    )
    power.ensure_on()

    def returned(
        snapshot: dict[str, object], records: list[dict[str, object]]
    ) -> bool:
        try:
            returned_target = monitor_with_verified_id(snapshot, initial_verified_id)
        except RuntimeError:
            return False
        if verified_id(returned_target) != initial_verified_id:
            return False
        connection = any(
            record.get("kind") == "monitor-connected"
            and initial_verified_id in str(record_fields(record).get("monitor", ""))
            for record in records
        )
        if not connection:
            return False
        windows = windows_by_key(snapshot)
        if expect_automatic_return:
            for key in automatic_return_keys:
                window = windows.get(key)
                if not window or not current_monitor_matches(window, initial_verified_id):
                    return False
        if expect_application_return:
            application = windows.get("hotplug-application")
            if not application or not current_monitor_matches(
                application, initial_verified_id
            ):
                return False
        if expect_native_fullscreen:
            automatic = windows.get("hotplug-automatic", {})
            if (
                automatic.get("native_fullscreen") != "fullscreen"
                or automatic.get("covers_current_monitor") != "full"
            ):
                return False
        return True

    returned_snapshot, records = wait_for_probe_state(
        probe,
        cursor,
        "same verified target and eligible returns",
        returned,
        RECOVERY_TIMEOUT_SECONDS,
    )
    return CycleEvidence(
        before=initial_snapshot,
        fallback=fallback,
        returned=returned_snapshot,
        records=records,
        initial_verified_id=initial_verified_id,
    )


def current_monitor_matches(window: dict[str, object], target_verified_id: str) -> bool:
    current_monitor = window.get("current_monitor")
    if not isinstance(current_monitor, dict):
        return False
    return cast("dict[str, object]", current_monitor).get("verified_id") == target_verified_id


def generic_cycle_assertions(
    evidence: CycleEvidence,
    automatic_window_keys: tuple[str, ...] = AUTOMATIC_WINDOW_KEYS,
) -> list[AssertionResult]:
    returned_windows = windows_by_key(evidence.returned)
    fallback_windows = windows_by_key(evidence.fallback)
    return [
        AssertionResult(
            "target-removal-edge",
            any(record.get("kind") == "monitor-disconnected" for record in evidence.records),
            "probe recorded the target monitor lifetime ending",
        ),
        AssertionResult(
            "automatic-fallback-count",
            all(key in fallback_windows for key in automatic_window_keys),
            f"fallback contains {', '.join(automatic_window_keys)}",
        ),
        AssertionResult(
            "same-verified-target",
            all(
                current_monitor_matches(returned_windows.get(key, {}), evidence.initial_verified_id)
                for key in automatic_window_keys
            ),
            f"{', '.join(automatic_window_keys)} returned to {evidence.initial_verified_id}",
        ),
        AssertionResult(
            "no-duplicate-automatic-keys",
            all(window_key_count(evidence.returned, key) == 1 for key in automatic_window_keys),
            f"exactly one snapshot entry exists for {', '.join(automatic_window_keys)}",
        ),
        AssertionResult(
            "no-recovery-mismatch",
            evidence.returned.get("terminal_failure") == "absent",
            "probe did not record WindowRestoreMismatch",
        ),
    ]


def automatic_cancellation_assertions(
    probe: ProbeProcess,
    power: PowerSafety,
    initial: dict[str, object],
) -> list[AssertionResult]:
    initial_verified_id = verified_id(probe.target_monitor(initial))
    initial_automatic = windows_by_key(initial).get("hotplug-automatic", {})
    initial_recovery = _recovery_counts(initial_automatic)
    accepted_before = initial_recovery.get("accepted") if initial_recovery is not None else None
    cursor = _as_int(initial.get("record_cursor", 0))
    power.power_off()
    power.wait_for_inventory(0, MONITOR_CHANGE_TIMEOUT_SECONDS)

    fallback, _ = wait_for_probe_state(
        probe,
        cursor,
        "automatic cancellation fallback",
        lambda snapshot, records: (
            initial_verified_id
            not in {
                monitor.get("verified_id")
                for monitor in _dict_list(snapshot.get("monitors"))
            }
            and any(
                record.get("kind") == "monitor-disconnected"
                for record in records
            )
            and "hotplug-automatic" in windows_by_key(snapshot)
        ),
        MONITOR_CHANGE_TIMEOUT_SECONDS,
    )
    fallback_automatic = windows_by_key(fallback)["hotplug-automatic"]
    fallback_monitor = fallback_automatic.get("current_monitor")
    if not isinstance(fallback_monitor, dict):
        raise RuntimeError("managed automatic fallback monitor was unavailable")
    fallback_monitor_fields = cast("dict[str, object]", fallback_monitor)
    fallback_verified_id = verified_id(fallback_monitor_fields)
    fallback_position = fallback_monitor_fields.get("physical_position")
    if not isinstance(fallback_position, list):
        raise RuntimeError("managed automatic fallback position was malformed")
    position_values = cast("list[object]", fallback_position)
    if len(position_values) != 2:
        raise RuntimeError("managed automatic fallback position was malformed")
    first_position, second_position = position_values[0], position_values[1]
    if not (isinstance(first_position, int) and isinstance(second_position, int)):
        raise RuntimeError("managed automatic fallback position was malformed")
    requested_position = [first_position + 80, second_position + 80]
    original_position = fallback_automatic.get("position")
    original_size = fallback_automatic.get("physical_size")

    move_receipt = probe.client.command(
        "cancel-cycle-move",
        {
            "kind": "move",
            "window": "automatic",
            "position": requested_position,
        },
    )
    resize_receipt = probe.client.command(
        "cancel-cycle-resize",
        {"kind": "resize", "window": "automatic", "size": [900, 650]},
    )
    changed, _ = wait_for_probe_state(
        probe,
        cursor,
        "fallback geometry changes",
        lambda snapshot, _records: (
            (automatic := windows_by_key(snapshot).get("hotplug-automatic", {})).get(
                "position"
            )
            != original_position
            and automatic.get("physical_size") != original_size
        ),
        RECOVERY_TIMEOUT_SECONDS,
    )
    borderless_receipt = probe.client.command(
        "cancel-cycle-borderless",
        {
            "kind": "set-mode",
            "window": "automatic",
            "mode": "borderless",
        },
    )
    _, _ = wait_for_probe_state(
        probe,
        cursor,
        "fallback borderless mode",
        lambda snapshot, _records: str(
            windows_by_key(snapshot)
            .get("hotplug-automatic", {})
            .get("requested_mode")
        ).startswith("BorderlessFullscreen"),
        RECOVERY_TIMEOUT_SECONDS,
    )
    windowed_receipt = probe.client.command(
        "cancel-cycle-windowed",
        {
            "kind": "set-mode",
            "window": "automatic",
            "mode": "windowed",
        },
    )
    before_cancellation, _ = wait_for_probe_state(
        probe,
        cursor,
        "fallback windowed mode",
        lambda snapshot, _records: (
            windows_by_key(snapshot)
            .get("hotplug-automatic", {})
            .get("requested_mode")
            == "Windowed"
        ),
        RECOVERY_TIMEOUT_SECONDS,
    )
    cancel_receipt = probe.client.command(
        "cancel-cycle-recovery", {"kind": "cancel-recovery"}
    )
    power.ensure_on()

    returned, records = wait_for_probe_state(
        probe,
        cursor,
        "primary return with managed cancellation",
        lambda snapshot, observed_records: (
            current_monitor_matches(
                windows_by_key(snapshot).get("primary", {}), initial_verified_id
            )
            and current_monitor_matches(
                windows_by_key(snapshot).get("hotplug-automatic", {}),
                fallback_verified_id,
            )
            and any(
                record.get("kind") == "recovery-cancellation-requested"
                and "hotplug-automatic"
                in record_fields(record).get("window_key", "")
                for record in observed_records
            )
        ),
        RECOVERY_TIMEOUT_SECONDS,
    )
    after_recovery = _recovery_counts(
        windows_by_key(before_cancellation).get("hotplug-automatic", {})
    )
    accepted_after = after_recovery.get("accepted") if after_recovery is not None else None
    receipts = [
        move_receipt,
        resize_receipt,
        borderless_receipt,
        windowed_receipt,
        cancel_receipt,
    ]
    return [
        AssertionResult(
            "authenticated-controls-applied",
            all(receipt.get("status") == "applied" for receipt in receipts),
            "move, resize, both mode changes, and cancellation were applied",
        ),
        AssertionResult(
            "geometry-and-mode-preserved-registration",
            accepted_after == accepted_before,
            f"accepted count remained {accepted_before}",
        ),
        AssertionResult(
            "managed-automatic-stayed-on-fallback",
            current_monitor_matches(
                windows_by_key(returned).get("hotplug-automatic", {}),
                fallback_verified_id,
            ),
            f"managed automatic remained on {fallback_verified_id}",
        ),
        AssertionResult(
            "primary-automatic-still-returned",
            current_monitor_matches(
                windows_by_key(returned).get("primary", {}), initial_verified_id
            ),
            f"primary returned to {initial_verified_id}",
        ),
        AssertionResult(
            "cancellation-recorded",
            any(
                record.get("kind") == "recovery-cancellation-requested"
                and "hotplug-automatic"
                in record_fields(record).get("window_key", "")
                for record in records
            ),
            "the ordered record stream contains managed automatic cancellation",
        ),
        AssertionResult(
            "no-cancellation-cycle-mismatch",
            returned.get("terminal_failure") == "absent",
            "the cancellation cycle did not record WindowRestoreMismatch",
        ),
        AssertionResult(
            "geometry-change-observed",
            windows_by_key(changed)
            .get("hotplug-automatic", {})
            .get("physical_size")
            != original_size,
            "the fallback window size changed before cancellation",
        ),
    ]


def passed_case(case_id: str, assertions: list[AssertionResult], artifact: Path) -> CaseResult:
    result = CaseResult(
        case_id=case_id,
        interaction=Interaction.AUTOMATED,
        evidence=Evidence.PHYSICAL,
        assertions=assertions,
        artifacts=[str(artifact)],
    )
    result.finish_from_assertions()
    return result


def physical_error_case(
    case_id: str,
    error: BaseException,
    artifact: Path,
) -> CaseResult:
    outcome = Outcome.TIMED_OUT if isinstance(error, TimeoutError) else Outcome.HARNESS_ERROR
    return CaseResult(
        case_id=case_id,
        interaction=Interaction.AUTOMATED,
        evidence=Evidence.PHYSICAL,
        outcome=outcome,
        assertions=[
            AssertionResult(
                "expected-state-reached",
                False,
                str(error),
            )
        ],
        artifacts=[str(artifact)],
    )


def _retarget_to_verified_monitor(
    probe: ProbeProcess,
    snapshot: dict[str, object],
    executable: Path,
    artifact_directory: Path,
    suite_run_id: str,
) -> tuple[ProbeProcess, dict[str, object]]:
    """Restart `probe` on a monitor the app fingerprinted, when it did not start on one.

    `accept_eligible_registrations` skips a window whose monitor identity is `Unverified`, so no
    `recovery-accepted` record is ever written for a panel the host cannot fingerprint and
    `retained-recovery-trace` cannot pass against it. Apple's internal panels publish no EDID at
    all, which leaves monitor 0 on a MacBook permanently unidentifiable while an attached external
    panel identifies normally, so the bootstrap index alone is not a usable target.

    A host that fingerprints no monitor keeps the bootstrap probe and fails
    `retained-recovery-trace`. That is the accurate outcome rather than a missing capability:
    reconnect recovery arms nowhere on such a host, and reporting the case as unavailable would
    let a real regression in registration acceptance pass as a host limitation.
    """
    target_index = verified_monitor_index(snapshot)
    if target_index is None or target_index == probe.monitor_index:
        return probe, snapshot
    probe.stop()
    retargeted = ProbeProcess(
        executable,
        artifact_directory / VERIFIED_MONITOR_ARTIFACTS,
        target_index,
        probe.startup_mode,
        suite_run_id,
    )
    return retargeted, retargeted.start()


def run_zero_window_case(
    executable: Path,
    artifact_directory: Path,
    suite_run_id: str,
) -> CaseResult:
    probe = ProbeProcess(
        executable, artifact_directory, BOOTSTRAP_MONITOR_INDEX, "windowed", suite_run_id
    )
    try:
        # This case closes every window and asserts liveness, record retention, command
        # idempotence, and reconstruction. Only `retained-recovery-trace` depends on the target
        # monitor's identity, so the probe starts unpinned on the bootstrap index and moves to a
        # fingerprinted monitor only when the bootstrap one turns out not to be fingerprinted.
        initial = probe.start(pin_verified_identity=False)
        probe, initial = _retarget_to_verified_monitor(
            probe, initial, executable, artifact_directory, suite_run_id
        )
        initial_cursor = _as_int(initial.get("record_cursor", 0))
        for selector in ("control", "application", "automatic", "primary"):
            receipt = probe.client.command(
                f"close-{selector}", {"kind": "close", "window": selector}
            )
            if receipt.get("status") != "applied":
                raise RuntimeError(f"close-{selector} was not applied: {receipt}")

        empty, _ = wait_for_probe_state(
            probe,
            initial_cursor,
            "zero owned windows",
            lambda snapshot, _records: snapshot.get("windows") == [],
            RECOVERY_TIMEOUT_SECONDS,
        )
        full_record_response = probe.client.records(0)
        records = _dict_list(full_record_response.get("records"))
        first_receipt = probe.client.command(
            "replace-application", {"kind": "replace-application"}
        )
        replay_receipt = probe.client.command(
            "replace-application", {"kind": "replace-application"}
        )
        reconstructed, _ = wait_for_probe_state(
            probe,
            _as_int(empty.get("record_cursor"), initial_cursor),
            "application replacement",
            lambda snapshot, _records: "hotplug-application" in windows_by_key(snapshot),
            RECOVERY_TIMEOUT_SECONDS,
        )
        assertions = [
            AssertionResult(
                "zero-window-http-liveness",
                empty.get("run_id") == probe.run_id and empty.get("windows") == [],
                "the owned process returned an authenticated snapshot with zero windows",
            ),
            AssertionResult(
                "retained-recovery-trace",
                any(record.get("kind") == "recovery-accepted" for record in records),
                "accepted recovery records remained readable after every window closed",
            ),
            AssertionResult(
                "idempotent-command-replay",
                first_receipt == replay_receipt,
                "replaying the same command id returned the original receipt",
            ),
            AssertionResult(
                "applicable-window-reconstructed",
                "hotplug-application" in windows_by_key(reconstructed),
                "the application-controlled window was reconstructed through the owned endpoint",
            ),
        ]
        return passed_case("zero_owned_windows", assertions, artifact_directory)
    finally:
        probe.stop()


def windowed_cases(
    executable: Path,
    artifact_directory: Path,
    selected_monitor_index: int,
    power: PowerSafety,
    suite_run_id: str,
    progress: Callable[[str], None],
) -> list[CaseResult]:
    probe_directory = artifact_directory / "windowed-repeated"
    probe = ProbeProcess(
        executable, probe_directory, selected_monitor_index, "windowed", suite_run_id
    )
    try:
        snapshot = probe.start()
        acceptance: dict[str, object] = {}
        for key in AUTOMATIC_WINDOW_KEYS:
            counts = _recovery_counts(windows_by_key(snapshot)[key])
            acceptance[key] = counts.get("accepted") if counts is not None else None
        progress("windowed physical cycle 1/3")
        first = run_cycle(
            probe,
            power,
            snapshot,
            expect_automatic_return=True,
            expect_application_return=True,
        )
        same_connection = passed_case(
            "same_dell_same_connection",
            generic_cycle_assertions(first),
            probe_directory,
        )
        original_scale = _as_float(probe.target_monitor(first.before)["scale"])
        fallback_scales: set[float] = set()
        for key in AUTOMATIC_WINDOW_KEYS:
            window = windows_by_key(first.fallback).get(key)
            if window is None:
                continue
            current = window.get("current_monitor")
            if not isinstance(current, dict):
                continue
            scale = cast("dict[str, object]", current).get("scale")
            if isinstance(scale, (int, float)):
                fallback_scales.add(float(scale))
        cross_dpi = passed_case(
            "dell_builtin_cross_dpi_return",
            generic_cycle_assertions(first)
            + [
                AssertionResult(
                    "different-fallback-scale",
                    any(scale != original_scale for scale in fallback_scales),
                    f"target scale {original_scale}; fallback scales {sorted(fallback_scales)}",
                )
            ],
            probe_directory,
        )
        progress(
            "physical cases 1/6 and 2/6 captured: same connection and cross-DPI return"
        )
        evidence = [first]
        try:
            snapshot = first.returned
            for cycle in range(2, 4):
                progress(f"windowed physical cycle {cycle}/3")
                cycle_evidence = run_cycle(
                    probe,
                    power,
                    snapshot,
                    expect_automatic_return=True,
                )
                evidence.append(cycle_evidence)
                snapshot = cycle_evidence.returned
            final_windows = windows_by_key(evidence[-1].returned)
            unchanged_acceptance = all(
                (final_counts := _recovery_counts(final_windows[key])) is not None
                and final_counts.get("accepted") == acceptance[key]
                for key in AUTOMATIC_WINDOW_KEYS
            )
            progress(
                "windowed cancellation cycle: authenticated move, resize, mode, and cancel"
            )
            cancellation_assertions = automatic_cancellation_assertions(
                probe, power, evidence[-1].returned
            )
            repeated = passed_case(
                "repeated_dell_power_cycles",
                [
                    assertion
                    for cycle in evidence
                    for assertion in generic_cycle_assertions(cycle)
                ]
                + [
                    AssertionResult(
                        "accepted-generation-count-unchanged",
                        unchanged_acceptance,
                        f"initial accepted counts remained {acceptance}",
                    ),
                    AssertionResult(
                        "application-controlled-first-return",
                        current_monitor_matches(
                            windows_by_key(evidence[0].returned).get(
                                "hotplug-application", {}
                            ),
                            evidence[0].initial_verified_id,
                        ),
                        "the application-controlled replacement returned after the first loss",
                    ),
                    AssertionResult(
                        "application-controlled-second-cycle-cancelled",
                        any(
                            record.get("kind")
                            == "recovery-cancellation-requested"
                            and "hotplug-application"
                            in record_fields(record).get("window_key", "")
                            for record in evidence[1].records
                        )
                        and "hotplug-application"
                        not in windows_by_key(evidence[1].returned),
                        "the second loss cancelled the application-controlled recovery",
                    ),
                ]
                + cancellation_assertions,
                probe_directory,
            )
        except Exception as error:
            repeated = physical_error_case(
                "repeated_dell_power_cycles", error, probe_directory
            )
        progress("physical case 3/6 captured: three repeated cycles")
        return [same_connection, cross_dpi, repeated]
    finally:
        probe.stop()


def one_cycle_case(
    case_id: str,
    startup_mode: str,
    executable: Path,
    artifact_directory: Path,
    selected_monitor_index: int,
    power: PowerSafety,
    suite_run_id: str,
    *,
    exclusive: bool = False,
) -> CaseResult:
    probe_directory = artifact_directory / case_id
    probe = ProbeProcess(
        executable, probe_directory, selected_monitor_index, startup_mode, suite_run_id
    )
    try:
        initial = probe.start()
        initial_windows = windows_by_key(initial)
        assertions: list[AssertionResult] = []
        if startup_mode == "borderless":
            initial, _ = wait_for_probe_state(
                probe,
                _as_int(initial.get("record_cursor", 0)),
                "initial native fullscreen completion",
                lambda snapshot, _records: (
                    windows_by_key(snapshot)
                    .get("hotplug-automatic", {})
                    .get("native_fullscreen")
                    == "fullscreen"
                    and windows_by_key(snapshot)
                    .get("hotplug-automatic", {})
                    .get("covers_current_monitor")
                    == "full"
                ),
                RECOVERY_TIMEOUT_SECONDS,
            )
            initial_windows = windows_by_key(initial)
            automatic = initial_windows.get("hotplug-automatic", {})
            assertions.extend(
                [
                    AssertionResult(
                        "initial-native-fullscreen",
                        automatic.get("native_fullscreen") == "fullscreen",
                        "AppKit reported fullscreen before power loss",
                    ),
                    AssertionResult(
                        "initial-target-coverage",
                        automatic.get("covers_current_monitor") == "full",
                        "managed automatic covered the target display before power loss",
                    ),
                ]
            )
        if exclusive:
            initial_counts = _recovery_counts(initial_windows.get("hotplug-automatic", {}))
            assertions.append(
                AssertionResult(
                    "exclusive-automatic-unarmed",
                    initial_counts is not None and initial_counts.get("accepted") == 0,
                    "exclusive managed automatic has no accepted recovery",
                )
            )
        evidence = run_cycle(
            probe,
            power,
            initial,
            expect_automatic_return=not exclusive,
            expect_native_fullscreen=startup_mode == "borderless",
            automatic_return_keys=(
                ("hotplug-automatic",)
                if startup_mode == "borderless"
                else AUTOMATIC_WINDOW_KEYS
            ),
            fallback_window_keys=("primary",) if exclusive else AUTOMATIC_WINDOW_KEYS,
        )
        if exclusive:
            returned_automatic = windows_by_key(evidence.returned).get(
                "hotplug-automatic"
            )
            restored_count = 0
            if isinstance(returned_automatic, dict):
                returned_counts = _recovery_counts(returned_automatic)
                if returned_counts is not None:
                    restored_count = _as_int(returned_counts.get("restored", 0))
            assertions.append(
                AssertionResult(
                    "exclusive-automatic-not-restored",
                    restored_count == 0,
                    f"managed automatic restored count is {restored_count}",
                )
            )
            assertions.extend(
                assertion
                for assertion in generic_cycle_assertions(evidence, ("primary",))
                if assertion.name not in ("same-verified-target", "automatic-fallback-count")
            )
        else:
            assertions.extend(
                generic_cycle_assertions(
                    evidence,
                    ("hotplug-automatic",)
                    if startup_mode == "borderless"
                    else AUTOMATIC_WINDOW_KEYS,
                )
            )
        if startup_mode == "borderless":
            automatic = windows_by_key(evidence.returned).get(
                "hotplug-automatic", {}
            )
            assertions.extend(
                [
                    AssertionResult(
                        "returned-native-fullscreen",
                        automatic.get("native_fullscreen") == "fullscreen",
                        "AppKit reported fullscreen after return",
                    ),
                    AssertionResult(
                        "returned-target-coverage",
                        automatic.get("covers_current_monitor") == "full",
                        "managed automatic covered the returned target display",
                    ),
                ]
            )
        return passed_case(case_id, assertions, probe_directory)
    finally:
        probe.stop()


def rapid_cycle_case(
    executable: Path,
    artifact_directory: Path,
    selected_monitor_index: int,
    power: PowerSafety,
    suite_run_id: str,
) -> CaseResult:
    case_id = "rapid_dell_power_cycle"
    probe_directory = artifact_directory / case_id
    probe = ProbeProcess(
        executable, probe_directory, selected_monitor_index, "windowed", suite_run_id
    )
    try:
        initial = probe.start()
        initial_verified_id = verified_id(probe.target_monitor(initial))
        cursor = _as_int(initial.get("record_cursor", 0))
        power.power_off()
        power.wait_for_inventory(0, MONITOR_CHANGE_TIMEOUT_SECONDS)
        power.progress(
            "OS removal confirmed; requesting power on without waiting for the app notification"
        )
        power.ensure_on()

        def returned_after_both_edges(
            snapshot: dict[str, object], records: list[dict[str, object]]
        ) -> bool:
            kinds = [record.get("kind") for record in records]
            if "monitor-disconnected" not in kinds or "monitor-connected" not in kinds:
                return False
            try:
                returned_target = monitor_with_verified_id(
                    snapshot, initial_verified_id
                )
            except RuntimeError:
                return False
            if verified_id(returned_target) != initial_verified_id:
                return False
            windows = windows_by_key(snapshot)
            return all(
                current_monitor_matches(windows.get(key, {}), initial_verified_id)
                for key in AUTOMATIC_WINDOW_KEYS
            )

        returned, records = wait_for_probe_state(
            probe,
            cursor,
            "rapid removal and return edges",
            returned_after_both_edges,
            RECOVERY_TIMEOUT_SECONDS,
        )
        removal_sequences = [
            _as_int(record["sequence"])
            for record in records
            if record.get("kind") == "monitor-disconnected"
        ]
        connection_sequences = [
            _as_int(record["sequence"])
            for record in records
            if record.get("kind") == "monitor-connected"
        ]
        assertions = [
            AssertionResult(
                "ordered-removal-and-return",
                bool(removal_sequences and connection_sequences)
                and min(removal_sequences) < max(connection_sequences),
                f"removal sequences {removal_sequences}; return sequences {connection_sequences}",
            ),
            AssertionResult(
                "same-verified-target",
                all(
                    current_monitor_matches(
                        windows_by_key(returned).get(key, {}), initial_verified_id
                    )
                    for key in AUTOMATIC_WINDOW_KEYS
                ),
                f"automatic windows returned to {initial_verified_id}",
            ),
            AssertionResult(
                "no-duplicate-automatic-keys",
                all(
                    window_key_count(returned, key) == 1
                    for key in AUTOMATIC_WINDOW_KEYS
                ),
                "one window exists for each automatic key after the rapid cycle",
            ),
            AssertionResult(
                "no-recovery-mismatch",
                returned.get("terminal_failure") == "absent",
                "probe did not record WindowRestoreMismatch",
            ),
        ]
        return passed_case(case_id, assertions, probe_directory)
    finally:
        probe.stop()


def run_windows_reconnect(
    executable: Path,
    power: PowerSafety,
    selected_monitor_index: int,
    artifact_directory: Path,
    suite_run_id: str,
    progress: Callable[[str], None],
) -> list[CaseResult]:
    """Windows automated reconnect: one windowed disconnect/reconnect cycle.

    The macOS groups exercise AppKit native fullscreen and cross-DPI returns,
    neither of which applies on the Windows VMware guest (native_fullscreen is
    "unavailable"; the virtual monitor is a single scale). The generic windowed
    cycle is the portable core: a window on the virtual monitor survives the
    monitor's disconnect and returns to the same verified id on reconnect.
    """
    case_id = "windows_windowed_reconnect"
    group_artifact = artifact_directory / case_id
    cleanup_failure: BaseException | None = None
    result: CaseResult | None = None
    try:
        for attempt in range(1, WINDOWS_RECONNECT_ATTEMPTS + 1):
            try:
                power.ensure_on()
                progress(
                    f"windows reconnect: windowed cycle "
                    f"(attempt {attempt}/{WINDOWS_RECONNECT_ATTEMPTS})"
                )
                result = one_cycle_case(
                    case_id,
                    "windowed",
                    executable,
                    artifact_directory,
                    selected_monitor_index,
                    power,
                    suite_run_id,
                )
            except Exception as error:
                progress(f"attempt {attempt} failed: {error}")
                result = physical_error_case(case_id, error, group_artifact)
            if result.outcome is Outcome.PASSED:
                break
            if attempt < WINDOWS_RECONNECT_ATTEMPTS:
                progress(
                    f"attempt {attempt} did not pass ({result.outcome.value}); "
                    "retrying with a fresh probe (software renderer flakiness)"
                )
    finally:
        try:
            power.ensure_on()
        except Exception as error:
            cleanup_failure = error
    if result is None:
        result = physical_error_case(
            case_id, RuntimeError("no reconnect attempt ran"), group_artifact
        )
    result.cleanup = CleanupResult(
        attempted=True,
        succeeded=cleanup_failure is None,
        detail=(
            "virtual display monitor is present in the OS inventory"
            if cleanup_failure is None
            else f"monitor restoration failed: {cleanup_failure}"
        ),
    )
    return [result]


def run_automated_reconnect(
    executable: Path,
    hardware_profile: HardwareProfile,
    discovery_environment: dict[str, str],
    artifact_directory: Path,
    suite_run_id: str,
    progress: Callable[[str], None],
) -> list[CaseResult]:
    selected_monitor_index = monitor_index(discovery_environment, hardware_profile)
    marker = artifact_directory.parent / "monitor-restore-required.json"
    power = PowerSafety(hardware_profile, marker, progress)
    if sys.platform == "win32":
        return run_windows_reconnect(
            executable,
            power,
            selected_monitor_index,
            artifact_directory,
            suite_run_id,
            progress,
        )
    results: list[CaseResult] = []
    cleanup_failure: BaseException | None = None

    def run_group(
        label: str,
        case_ids: tuple[str, ...],
        group_artifact: Path,
        action: Callable[[], list[CaseResult]],
    ) -> bool:
        nonlocal cleanup_failure
        progress(label)
        try:
            results.extend(action())
        except Exception as error:
            progress(f"{label} failed: {error}")
            results.extend(
                physical_error_case(case_id, error, group_artifact)
                for case_id in case_ids
            )
            try:
                power.ensure_on()
            except Exception as error:
                cleanup_failure = error
                return False
        progress(f"{label} completed")
        return True

    groups: list[
        tuple[
            str,
            tuple[str, ...],
            Path,
            Callable[[], list[CaseResult]],
        ]
    ] = [
        (
            "physical probe 1/4: three windowed cycles",
            (
                "same_dell_same_connection",
                "dell_builtin_cross_dpi_return",
                "repeated_dell_power_cycles",
            ),
            artifact_directory / "windowed-repeated",
            lambda: windowed_cases(
                executable,
                artifact_directory,
                selected_monitor_index,
                power,
                suite_run_id,
                progress,
            ),
        ),
        (
            "physical probe 2/4: rapid off/on",
            ("rapid_dell_power_cycle",),
            artifact_directory / "rapid_dell_power_cycle",
            lambda: [
                rapid_cycle_case(
                    executable,
                    artifact_directory,
                    selected_monitor_index,
                    power,
                    suite_run_id,
                )
            ],
        ),
        (
            "physical probe 3/4: borderless return",
            ("borderless_return",),
            artifact_directory / "borderless_return",
            lambda: [
                one_cycle_case(
                    "borderless_return",
                    "borderless",
                    executable,
                    artifact_directory,
                    selected_monitor_index,
                    power,
                    suite_run_id,
                )
            ],
        ),
        (
            "physical probe 4/4: exclusive automatic-unarmed",
            ("exclusive_automatic_unarmed",),
            artifact_directory / "exclusive_automatic_unarmed",
            lambda: [
                one_cycle_case(
                    "exclusive_automatic_unarmed",
                    "exclusive",
                    executable,
                    artifact_directory,
                    selected_monitor_index,
                    power,
                    suite_run_id,
                    exclusive=True,
                )
            ],
        ),
    ]

    started_case_ids: set[str] = set()
    setup_failure: BaseException | None = None
    try:
        power.ensure_on()
        for label, case_ids, group_artifact, action in groups:
            started_case_ids.update(case_ids)
            if not run_group(label, case_ids, group_artifact, action):
                break
    except Exception as error:
        progress(f"physical reconnect setup failed: {error}")
        setup_failure = error
    finally:
        try:
            power.ensure_on()
            cleanup_failure = None
        except Exception as error:
            cleanup_failure = error

    result_case_ids = {result.case_id for result in results}
    for _label, case_ids, group_artifact, _action in groups:
        for case_id in case_ids:
            if case_id in result_case_ids:
                continue
            outcome = (
                Outcome.HARNESS_ERROR
                if setup_failure is not None or case_id in started_case_ids
                else Outcome.ABORTED
            )
            results.append(
                CaseResult(
                    case_id=case_id,
                    interaction=Interaction.AUTOMATED,
                    evidence=Evidence.PHYSICAL,
                    outcome=outcome,
                    artifacts=[str(group_artifact)],
                )
            )

    cleanup = CleanupResult(
        attempted=True,
        succeeded=cleanup_failure is None,
        detail=(
            "configured monitor is on and present in the OS inventory"
            if cleanup_failure is None
            else f"monitor restoration failed: {cleanup_failure}"
        ),
    )
    for result in results:
        result.cleanup = cleanup
    case_order = {
        "same_dell_same_connection": 0,
        "rapid_dell_power_cycle": 1,
        "borderless_return": 2,
        "exclusive_automatic_unarmed": 3,
        "dell_builtin_cross_dpi_return": 4,
        "repeated_dell_power_cycles": 5,
    }
    return sorted(results, key=lambda result: case_order[result.case_id])


def load_environment_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text().splitlines():
        if line.startswith("export ") and "=" in line:
            key, value = line.removeprefix("export ").split("=", 1)
            values[key] = value
    return values


@dataclass(frozen=True)
class ReconnectArgs:
    executable: Path
    hardware_profile: Path | None
    environment_file: Path | None
    artifact_dir: Path
    run_id: str
    result_json: Path
    zero_window_only: bool


def parse_args() -> ReconnectArgs:
    parser = argparse.ArgumentParser(description=__doc__)
    _ = parser.add_argument("--executable", type=Path, required=True)
    _ = parser.add_argument("--hardware-profile", type=Path)
    _ = parser.add_argument("--environment-file", type=Path)
    _ = parser.add_argument("--artifact-dir", type=Path, required=True)
    _ = parser.add_argument("--run-id", required=True)
    _ = parser.add_argument("--result-json", type=Path, required=True)
    _ = parser.add_argument("--zero-window-only", action="store_true")
    namespace = parser.parse_args()
    return ReconnectArgs(
        executable=cast("Path", namespace.executable),
        hardware_profile=cast("Path | None", namespace.hardware_profile),
        environment_file=cast("Path | None", namespace.environment_file),
        artifact_dir=cast("Path", namespace.artifact_dir),
        run_id=cast("str", namespace.run_id),
        result_json=cast("Path", namespace.result_json),
        zero_window_only=cast("bool", namespace.zero_window_only),
    )


def main() -> int:
    args = parse_args()
    if args.zero_window_only:
        result = run_zero_window_case(
            args.executable.resolve(), args.artifact_dir.resolve(), args.run_id
        )
        args.result_json.parent.mkdir(parents=True, exist_ok=True)
        _ = args.result_json.write_text(
            json.dumps(result.to_dict(), indent=2, sort_keys=True) + "\n"
        )
        return int(result.outcome is not Outcome.PASSED)
    if args.hardware_profile is None or args.environment_file is None:
        raise SystemExit(
            "--hardware-profile and --environment-file are required for physical cases"
        )
    profile = HardwareProfile.load(args.hardware_profile)
    results = run_automated_reconnect(
        args.executable.resolve(),
        profile,
        load_environment_file(args.environment_file),
        args.artifact_dir.resolve(),
        args.run_id,
        lambda message: print(message, flush=True),
    )
    args.result_json.parent.mkdir(parents=True, exist_ok=True)
    _ = args.result_json.write_text(
        json.dumps([result.to_dict() for result in results], indent=2, sort_keys=True)
        + "\n"
    )
    return int(any(result.outcome is not Outcome.PASSED for result in results))


if __name__ == "__main__":
    sys.exit(main())
