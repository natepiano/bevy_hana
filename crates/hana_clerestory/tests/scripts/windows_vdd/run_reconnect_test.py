"""Focused Windows monitor-reconnect validation using the VDD virtual monitor.

Run from an ELEVATED shell (enabling/disabling the virtual display needs admin).
It builds the probes, enables the virtual monitor, runs the single windowed
disconnect/reconnect cycle, disables the virtual monitor, and prints the result.

    # from an elevated PowerShell / cmd, at the repo root or anywhere:
    python crates/hana_clerestory/tests/scripts/windows_vdd/run_reconnect_test.py

The 256 MB stack the probe needs under WARP software rendering is applied
automatically by the crate's build.rs; the longer Windows timeouts live in
run_reconnect.py. See README.md in this directory.
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
PROFILE = CRATE / "tests" / "config" / "hardware.windows-vm.local.json"
CONFIG = CRATE / "tests" / "config" / "windows.json"
PROBE = REPO / "target" / "debug" / "examples" / "restore_after_reconnect.exe"
DISCOVERY = REPO / "target" / "debug" / "examples" / "restore_window.exe"
OUT = Path(tempfile.gettempdir()) / "clerestory_windows_reconnect"


def powershell(script: Path) -> int:
    completed = subprocess.run(
        ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)],
        check=False,
        timeout=90,
    )
    return completed.returncode


def build_probes() -> None:
    for example in ("restore_window", "restore_after_reconnect"):
        print(f"building {example} ...", flush=True)
        subprocess.run(
            ["cargo", "build", "-p", "hana_clerestory", "--example", example, "--features", "monitor-probe"],
            cwd=str(REPO),
            check=True,
        )


def discover() -> Path:
    env_file = OUT / "discovery.env"
    subprocess.run(
        [
            sys.executable, str(SCRIPTS / "run_test.py"), "--discover",
            "--config", str(CONFIG), "--env-file", str(env_file), "--backend", "native",
            "--executable", str(DISCOVERY), "--base-port", "49780",
            "--ron-path", str(OUT / "discovery.ron"), "--artifact-dir", str(OUT / "discovery"),
        ],
        cwd=str(CRATE),
        check=False,
        timeout=180,
    )
    return env_file


def run_reconnect(env_file: Path) -> Path:
    result_json = OUT / "result.json"
    subprocess.run(
        [
            sys.executable, str(SCRIPTS / "run_reconnect.py"),
            "--executable", str(PROBE), "--hardware-profile", str(PROFILE),
            "--environment-file", str(env_file), "--artifact-dir", str(OUT / "reconnect"),
            "--run-id", "windows-reconnect", "--result-json", str(result_json),
        ],
        cwd=str(CRATE),
        check=False,
        timeout=900,
    )
    return result_json


def report(result_json: Path) -> bool:
    if not result_json.is_file():
        print("NO RESULT FILE — see", OUT)
        return False
    cases = cast("list[dict[str, object]]", json.loads(result_json.read_text()))
    every_passed = True
    for case in cases:
        outcome = case.get("outcome")
        print(f"\n{case.get('case_id')}: {outcome}")
        assertions = case.get("assertions")
        if isinstance(assertions, list):
            for assertion in cast("list[dict[str, object]]", assertions):
                passed = bool(assertion.get("passed"))
                every_passed = every_passed and passed
                mark = "PASS" if passed else "FAIL"
                print(f"  {mark} {assertion.get('name')}")
        every_passed = every_passed and outcome == "passed"
    return every_passed


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    build_probes()
    print("enabling virtual monitor ...", flush=True)
    _ = powershell(VDD_DIR / "enable.ps1")
    try:
        env_file = discover()
        result_json = run_reconnect(env_file)
    finally:
        print("disabling virtual monitor ...", flush=True)
        _ = powershell(VDD_DIR / "disable.ps1")
    passed = report(result_json)
    print("\n==== RECONNECT TEST:", "PASSED" if passed else "FAILED", "====")
    print("artifacts:", OUT)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
