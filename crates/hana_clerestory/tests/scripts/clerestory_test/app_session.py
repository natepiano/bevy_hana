"""Ownership of one directly launched Clerestory test process."""

from __future__ import annotations

import os
import signal
import socket
import subprocess
import sys
import time
from collections.abc import Callable
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path
from typing import BinaryIO
from typing import cast


class PortPairUnavailable(RuntimeError):
    pass


@dataclass(frozen=True)
class PortPair:
    base: int
    render: int


def available_port_pair(candidate_ports: Iterable[int] | None = None) -> PortPair:
    """Find two consecutive loopback ports that are free at the same instant."""
    candidates: Iterable[int | None]
    if candidate_ports is None:
        candidates = (None for _ in range(128))
    else:
        candidates = candidate_ports
    for candidate in candidates:
        with socket.socket() as base_socket:
            try:
                base_socket.bind(("127.0.0.1", candidate or 0))
            except OSError:
                continue
            sockname = cast("tuple[object, ...]", base_socket.getsockname())
            bound_port = sockname[1]
            assert isinstance(bound_port, int)
            base = bound_port
            if base >= 65535:
                continue
            with socket.socket() as render_socket:
                try:
                    render_socket.bind(("127.0.0.1", base + 1))
                except OSError:
                    continue
                return PortPair(base=base, render=base + 1)
    raise PortPairUnavailable("could not reserve a consecutive loopback port pair")


@dataclass
class AppSession:
    executable: Path
    argv: list[str]
    environment: dict[str, str]
    stdout_path: Path
    stderr_path: Path
    working_directory: Path | None = None
    process: subprocess.Popen[bytes] | None = None
    _stdout: BinaryIO | None = None
    _stderr: BinaryIO | None = None

    def start(self) -> None:
        if self.process is not None:
            raise RuntimeError("app session is already running")
        self.stdout_path.parent.mkdir(parents=True, exist_ok=True)
        self._stdout = self.stdout_path.open("wb")
        self._stderr = self.stderr_path.open("wb")
        creation_flags = 0
        start_new_session = sys.platform != "win32"
        if sys.platform == "win32":
            creation_flags = subprocess.CREATE_NEW_PROCESS_GROUP
        try:
            self.process = subprocess.Popen(
                [str(self.executable), *self.argv],
                cwd=self.working_directory or self.executable.parent,
                env=self.environment,
                stdout=self._stdout,
                stderr=self._stderr,
                start_new_session=start_new_session,
                creationflags=creation_flags,
            )
        except BaseException:
            self._close_logs()
            raise

    def poll(self) -> int | None:
        return None if self.process is None else self.process.poll()

    def stop(
        self,
        graceful_shutdown: Callable[[], None] | None = None,
        timeout_seconds: float = 10.0,
    ) -> int | None:
        process = self.process
        if process is None:
            self._close_logs()
            return None
        interruption: BaseException | None = None
        if process.poll() is None and graceful_shutdown is not None:
            try:
                graceful_shutdown()
            except (KeyboardInterrupt, SystemExit) as error:
                interruption = error
            except Exception:
                pass
        deadline = time.monotonic() + timeout_seconds
        while process.poll() is None and time.monotonic() < deadline:
            time.sleep(0.05)
        if process.poll() is None:
            self._terminate_owned_process(process)
        try:
            exit_code = process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            if sys.platform == "win32":
                process.kill()
            else:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            exit_code = process.wait(timeout=5)
        self.process = None
        self._close_logs()
        if interruption is not None:
            raise interruption
        return exit_code

    def _terminate_owned_process(self, process: subprocess.Popen[bytes]) -> None:
        if sys.platform == "win32":
            process.terminate()
            return
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return

    def _close_logs(self) -> None:
        for log in (self._stdout, self._stderr):
            if log is not None:
                log.close()
        self._stdout = None
        self._stderr = None
