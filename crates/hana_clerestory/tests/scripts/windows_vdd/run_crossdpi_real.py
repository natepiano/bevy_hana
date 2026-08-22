"""Run the two cross-DPI restore cases against REAL monitors already at different scales.

Use this after configuring a genuine mixed-DPI setup by hand (set one monitor to a
different scale than the others in Windows Display settings, then sign out and back in
so the scale actually applies). Unlike run_crossdpi_test.py, this does NOT touch the
virtual display driver and needs NO elevation: it just builds the probe, discovers the
real monitors, confirms they differ in scale, and runs the two cross cases.

    python crates/hana_clerestory/tests/scripts/windows_vdd/run_crossdpi_real.py

See phase15-cross-dpi-provisioning in the project memory for why the VDD path deadlocks
WARP and why a real mixed-DPI setup is expected to work.
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
OUT = Path(tempfile.gettempdir()) / "clerestory_crossdpi_real"
CASES = ("cross_high_to_low", "cross_low_to_high")
ATTEMPTS = 2


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
    env_file, env = discover()
    print(
        f"discovery: DIFFERENT_SCALES={env.get('DIFFERENT_SCALES')} "
        f"high={env.get('HIGH_SCALE_MONITOR_INDEX')} low={env.get('LOW_SCALE_MONITOR_INDEX')}",
        flush=True,
    )
    if env.get("DIFFERENT_SCALES") != "true":
        print(
            "\nNo two real monitors have different scales. Set one monitor to a different "
            "scale in Display settings, sign out and back in, then re-run.",
            flush=True,
        )
        return 2
    results: dict[str, str] = {}
    for case_index, case_id in enumerate(CASES):
        for attempt in range(1, ATTEMPTS + 1):
            port = 49830 + case_index * 10 + attempt
            outcome, detail = run_case(case_id, env_file, port)
            print(f"{case_id} attempt {attempt}/{ATTEMPTS}: {outcome} {detail}", flush=True)
            results[case_id] = outcome
            if outcome == "passed":
                break
    every_passed = all(outcome == "passed" for outcome in results.values())
    print("\n==== CROSS-DPI (real monitors):", "PASSED" if every_passed else "FAILED", "====")
    for case_id, outcome in results.items():
        print(f"  {case_id}: {outcome}")
    print("artifacts:", OUT)
    return 0 if every_passed else 1


if __name__ == "__main__":
    sys.exit(main())
