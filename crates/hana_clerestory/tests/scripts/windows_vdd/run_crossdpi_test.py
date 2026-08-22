"""Focused Windows cross-DPI restore validation using the VDD virtual monitor.

Run from an ELEVATED shell (enabling/disabling the virtual display needs admin).
It builds the restore probe, enables the virtual monitor, forces it to scale 1
(so it differs from the real scale-2 monitors), discovers, runs the two cross-DPI
restore cases with a fresh-probe retry, disables the monitor, and prints the result.

    python crates/hana_clerestory/tests/scripts/windows_vdd/run_crossdpi_test.py

Iterating on cross-DPI only; the full suite (run_suite.py) is not needed until this
passes. See phase15-cross-dpi-provisioning in the project memory.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import cast

VDD_DIR = Path(__file__).resolve().parent
SCRIPTS = VDD_DIR.parent
CRATE = SCRIPTS.parent.parent
REPO = CRATE.parent.parent
CONFIG = CRATE / "tests" / "config" / "windows.json"
PROBE = REPO / "target" / "debug" / "examples" / "restore_window.exe"
OUT = Path(tempfile.gettempdir()) / "clerestory_crossdpi"
CASES = ("cross_high_to_low", "cross_low_to_high")
ATTEMPTS = 2


def powershell(*args: str) -> int:
    completed = subprocess.run(
        ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", *args],
        check=False,
        timeout=90,
    )
    return completed.returncode


def build_probe() -> None:
    print("building restore_window ...", flush=True)
    subprocess.run(
        ["cargo", "build", "-p", "hana_clerestory", "--example", "restore_window"],
        cwd=str(REPO),
        check=True,
    )


def discover() -> tuple[Path, dict[str, str]]:
    env_file = OUT / "discovery.env"
    subprocess.run(
        [
            sys.executable, str(SCRIPTS / "run_test.py"), "--discover",
            "--config", str(CONFIG), "--env-file", str(env_file), "--backend", "native",
            "--executable", str(PROBE), "--base-port", "49820",
            "--ron-path", str(OUT / "discovery.ron"), "--artifact-dir", str(OUT / "discovery"),
        ],
        cwd=str(CRATE),
        check=False,
        timeout=180,
    )
    env: dict[str, str] = {}
    if env_file.is_file():
        for line in env_file.read_text().splitlines():
            line = line.removeprefix("export ").strip()
            if "=" in line:
                key, value = line.split("=", 1)
                env[key] = value
    return env_file, env


def run_case(case_id: str, env_file: Path, port: int) -> tuple[str, str]:
    artifact = OUT / case_id
    result_path = artifact / "case-result.json"
    result_path.unlink(missing_ok=True)
    subprocess.run(
        [
            sys.executable, str(SCRIPTS / "run_test.py"),
            "--config", str(CONFIG), "--test-id", case_id, "--backend", "native",
            "--env-file", str(env_file), "--executable", str(PROBE),
            "--base-port", str(port), "--ron-path", str(artifact / "windows.ron"),
            "--artifact-dir", str(artifact), "--result-json", str(result_path),
        ],
        cwd=str(CRATE),
        check=False,
        timeout=300,
    )
    if not result_path.is_file():
        return "no-result", "runner produced no result file"
    data = cast("dict[str, object]", json.loads(result_path.read_text()))
    return str(data.get("outcome", "unknown")), str(data.get("detail", ""))


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    build_probe()
    print("enabling virtual monitor ...", flush=True)
    _ = powershell("-File", str(VDD_DIR / "enable.ps1"))
    print("forcing virtual monitor to scale 1 ...", flush=True)
    _ = powershell("-File", str(VDD_DIR / "dpi_scale.ps1"), "-Match", "MTT1337", "-Action", "setrel", "-Rel", "0")
    try:
        _env_file, env = discover()
        env_file = _env_file
        print(
            f"discovery: DIFFERENT_SCALES={env.get('DIFFERENT_SCALES')} "
            f"high={env.get('HIGH_SCALE_MONITOR_INDEX')} low={env.get('LOW_SCALE_MONITOR_INDEX')}",
            flush=True,
        )
        results: dict[str, str] = {}
        for case_index, case_id in enumerate(CASES):
            for attempt in range(1, ATTEMPTS + 1):
                port = 49830 + case_index * 10 + attempt
                outcome, detail = run_case(case_id, env_file, port)
                print(f"{case_id} attempt {attempt}/{ATTEMPTS}: {outcome} {detail}", flush=True)
                results[case_id] = outcome
                if outcome == "passed":
                    break
    finally:
        print("disabling virtual monitor ...", flush=True)
        _ = powershell("-File", str(VDD_DIR / "disable.ps1"))
    every_passed = all(outcome == "passed" for outcome in results.values())
    print("\n==== CROSS-DPI:", "PASSED" if every_passed else "FAILED", "====")
    for case_id, outcome in results.items():
        print(f"  {case_id}: {outcome}")
    print("artifacts:", OUT)
    return 0 if every_passed else 1


if __name__ == "__main__":
    sys.exit(main())
