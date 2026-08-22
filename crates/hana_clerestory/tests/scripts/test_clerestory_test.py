from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import cast
from typing import final

from clerestory_test.app_session import AppSession
from clerestory_test.app_session import PortPairUnavailable
from clerestory_test.app_session import available_port_pair
from clerestory_test.case_result import AssertionResult
from clerestory_test.case_result import Availability
from clerestory_test.case_result import CaseResult
from clerestory_test.case_result import CleanupResult
from clerestory_test.case_result import Evidence
from clerestory_test.case_result import Interaction
from clerestory_test.case_result import Outcome
from clerestory_test.hardware_profile import HardwareProfile
from clerestory_test.polling import WaitTimedOut
from clerestory_test.polling import poll_until
from clerestory_test.probe_client import ProbeClient
from clerestory_test.run_report import RunReport
from run_reconnect import PowerSafety
from run_reconnect import CycleEvidence
from run_reconnect import generic_cycle_assertions
from run_reconnect import physical_error_case
from run_reconnect import probe_target_monitor
from run_reconnect import raise_preserving_failures


@final
class FakeClock:
    def __init__(self) -> None:
        self.now = 0.0

    def monotonic(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.now += seconds


class CaseResultTests(unittest.TestCase):
    def test_unavailable_case_cannot_become_passed(self) -> None:
        result = CaseResult(
            case_id="needs-panel",
            interaction=Interaction.OPERATOR_ACTION,
            evidence=Evidence.PHYSICAL,
            availability=Availability.UNAVAILABLE,
            assertions=[AssertionResult("placeholder", True, "not evidence")],
        )

        result.finish_from_assertions()

        self.assertEqual(result.outcome, Outcome.NOT_RUN)

    def test_available_case_requires_assertions_to_pass(self) -> None:
        result = CaseResult(
            case_id="state",
            interaction=Interaction.AUTOMATED,
            evidence=Evidence.APPLICATION,
            assertions=[AssertionResult("ready", True, "observed")],
        )

        result.finish_from_assertions()

        self.assertEqual(result.outcome, Outcome.PASSED)


class PollingTests(unittest.TestCase):
    def test_polling_uses_the_injected_clock_without_sleeping(self) -> None:
        clock = FakeClock()
        observations = iter([0, 1, 2])

        result = poll_until(
            lambda: next(observations),
            lambda value: value == 2,
            expected="revision increment",
            timeout_seconds=5,
            interval_seconds=1,
            clock=clock,
        )

        self.assertEqual(result.first, 0)
        self.assertEqual(result.final, 2)
        self.assertEqual(result.observations, 3)
        self.assertEqual(result.elapsed_seconds, 2)

    def test_timeout_preserves_the_last_observation(self) -> None:
        clock = FakeClock()

        with self.assertRaises(WaitTimedOut) as caught:
            _ = poll_until(
                lambda: "unchanged",
                lambda value: value == "changed",
                expected="monitor removal",
                timeout_seconds=2,
                interval_seconds=1,
                clock=clock,
            )

        self.assertEqual(caught.exception.last, "unchanged")
        self.assertEqual(caught.exception.elapsed_seconds, 2)


class AppSessionTests(unittest.TestCase):
    def test_port_pair_selection_rejects_a_collision(self) -> None:
        with socket.socket() as collision:
            collision.bind(("127.0.0.1", 0))
            blocked_sockname = cast("tuple[object, ...]", collision.getsockname())
            blocked_port = blocked_sockname[1]
            assert isinstance(blocked_port, int)

            with self.assertRaises(PortPairUnavailable):
                _ = available_port_pair([blocked_port])

        self.assertGreater(available_port_pair().base, 0)

    def test_stopping_one_session_does_not_stop_an_unowned_process(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            unowned = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(30)"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            session = AppSession(
                executable=Path(sys.executable),
                argv=["-c", "import time; time.sleep(30)"],
                environment=dict(os.environ),
                stdout_path=root / "stdout.log",
                stderr_path=root / "stderr.log",
            )
            try:
                session.start()
                _ = session.stop(timeout_seconds=0)
                self.assertIsNone(unowned.poll())
            finally:
                unowned.terminate()
                _ = unowned.wait(timeout=5)

    def test_process_crash_is_observable_without_name_wide_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            session = AppSession(
                executable=Path(sys.executable),
                argv=["-c", "raise SystemExit(7)"],
                environment=dict(os.environ),
                stdout_path=root / "stdout.log",
                stderr_path=root / "stderr.log",
            )

            session.start()
            exit_code = session.stop()

            self.assertEqual(exit_code, 7)


class HardwareProfileTests(unittest.TestCase):
    def test_profile_keeps_commands_as_executable_and_arguments(self) -> None:
        profile_path = (
            Path(__file__).parent.parent / "config" / "hardware.example.json"
        )

        profile = HardwareProfile.load(profile_path)

        self.assertEqual(profile.power_off.executable, "/usr/bin/shortcuts")
        self.assertEqual(profile.power_off.arguments, ("run", "dell monitor off"))
        self.assertEqual(profile.minimum_on_seconds, 5)
        self.assertEqual(
            profile.probe_monitor_matcher.refresh_rate_millihertz, 120000
        )


@final
class FakeHardwareCommand:
    def __init__(self, failure: BaseException | None = None) -> None:
        self.failure = failure
        self.calls = 0

    def run(self, _environment: dict[str, str]) -> object:
        self.calls += 1
        if self.failure is not None:
            raise self.failure
        return subprocess.CompletedProcess[bytes]([], 0)


@final
class FakeHardwareProfile:
    def __init__(self, inventory_counts: list[int]) -> None:
        self.name = "fake panel"
        self.target_matcher = "fake"
        self.minimum_on_seconds = 0.0
        self.minimum_off_seconds = 0.0
        self.power_on = FakeHardwareCommand()
        self.power_off = FakeHardwareCommand()
        self.inventory_counts = iter(inventory_counts)

    def inventory_match_count(self, _environment: dict[str, str]) -> int:
        return next(self.inventory_counts)

    def inventory_snapshot(self, _environment: dict[str, str]) -> tuple[object, int]:
        count = next(self.inventory_counts)
        return {"fake": True}, count


class PowerSafetyTests(unittest.TestCase):
    def test_marker_is_cleared_only_after_inventory_confirms_monitor_on(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "monitor-restore-required.json"
            profile = FakeHardwareProfile([1])
            power = PowerSafety(
                cast("HardwareProfile", cast("object", profile)),
                marker,
                lambda _message: None,
                sleep=lambda _seconds: None,
            )

            power.power_off()
            self.assertTrue(marker.is_file())
            power.ensure_on()

            self.assertFalse(marker.exists())
            self.assertEqual(profile.power_off.calls, 1)
            self.assertEqual(profile.power_on.calls, 1)

    def test_cleanup_failure_does_not_replace_the_original_failure(self) -> None:
        original = RuntimeError("original probe failure")
        cleanup = RuntimeError("monitor-on failure")

        with self.assertRaises(ExceptionGroup) as caught:
            raise_preserving_failures(original, cleanup)

        self.assertEqual(caught.exception.exceptions, (original, cleanup))


class ProbeClientTests(unittest.TestCase):
    def test_wrong_session_identity_times_out_without_real_sleeping(self) -> None:
        clock = FakeClock()
        client = ProbeClient(
            1,
            "capability",
            monotonic=clock.monotonic,
            sleep=clock.sleep,
        )
        client.snapshot = lambda: {  # type: ignore[method-assign]
            "run_id": "wrong-run",
            "boot_nonce": "wrong-boot",
            "ready": "ready",
        }

        with self.assertRaises(TimeoutError) as caught:
            _ = client.wait_ready("expected-run", "expected-boot", 0.3)

        self.assertIn("session identity did not match", str(caught.exception))


class ReconnectAssertionTests(unittest.TestCase):
    def test_probe_timeout_is_not_reported_as_a_harness_error(self) -> None:
        result = physical_error_case(
            "borderless_return",
            TimeoutError("native fullscreen did not complete"),
            Path("artifacts"),
        )

        self.assertEqual(result.outcome, Outcome.TIMED_OUT)
        self.assertEqual(result.assertions[0].detail, "native fullscreen did not complete")

    def test_verified_target_survives_monitor_index_reordering(self) -> None:
        snapshot: dict[str, object] = {
            "monitors": [
                {"index": 0, "verified_id": "MonitorId(0)"},
                {"index": 1, "verified_id": "MonitorId(2)"},
                {"index": 2, "verified_id": "MonitorId(1)"},
            ]
        }

        target = probe_target_monitor(snapshot, 1, "MonitorId(1)")

        self.assertEqual(target["index"], 2)

    def test_duplicate_restore_and_mismatch_are_independent_failures(self) -> None:
        monitor = {"verified_id": "MonitorId(1)"}
        automatic = {
            "key": "primary",
            "current_monitor": monitor,
        }
        managed = {
            "key": "hotplug-automatic",
            "current_monitor": monitor,
        }
        evidence = CycleEvidence(
            before={},
            fallback={"windows": [automatic, managed]},
            returned={
                "windows": [automatic, automatic, managed],
                "terminal_failure": "present",
            },
            records=[{"kind": "monitor-disconnected", "fields": {}}],
            initial_verified_id="MonitorId(1)",
        )

        assertions = {
            assertion.name: assertion for assertion in generic_cycle_assertions(evidence)
        }

        self.assertFalse(assertions["no-duplicate-automatic-keys"].passed)
        self.assertFalse(assertions["no-recovery-mismatch"].passed)


class RunReportTests(unittest.TestCase):
    def test_each_result_is_immediately_present_in_both_reports(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report = RunReport(Path(temporary) / "run", "run-1", "abc123")
            result = CaseResult(
                case_id="restore",
                interaction=Interaction.AUTOMATED,
                evidence=Evidence.APPLICATION,
                outcome=Outcome.PASSED,
            )

            report.append(result)

            payload = cast("dict[str, object]", json.loads(report.json_path.read_text()))
            results = cast("list[dict[str, object]]", payload["results"])
            self.assertEqual(results[0]["case_id"], "restore")
            self.assertIn("| `restore` | automated | application |", report.markdown_path.read_text())

    def test_cleanup_failure_is_serialized_without_replacing_case_outcome(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            report = RunReport(Path(temporary) / "run", "run-1", "abc123")
            result = CaseResult(
                case_id="physical-restore",
                interaction=Interaction.AUTOMATED,
                evidence=Evidence.PHYSICAL,
                outcome=Outcome.FAILED,
                cleanup=CleanupResult(
                    attempted=True,
                    succeeded=False,
                    detail="monitor-on command failed",
                ),
            )

            report.append(result)
            recovered_payload = cast(
                "dict[str, object]", json.loads(report.json_path.read_text())
            )
            recovered_results = cast("list[dict[str, object]]", recovered_payload["results"])
            recovered = CaseResult.from_dict(recovered_results[0])

            self.assertEqual(recovered.outcome, Outcome.FAILED)
            self.assertIsNotNone(recovered.cleanup)
            assert recovered.cleanup is not None
            self.assertFalse(recovered.cleanup.succeeded)


if __name__ == "__main__":
    _ = unittest.main()
