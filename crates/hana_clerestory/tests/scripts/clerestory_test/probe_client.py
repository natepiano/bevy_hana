"""Authenticated JSON-RPC client for the reconnect example."""

from __future__ import annotations

import json
import time
from collections.abc import Callable
from http.client import HTTPResponse
from typing import cast
from typing import final
from urllib.error import URLError
from urllib.request import Request
from urllib.request import urlopen


class ProbeRpcError(RuntimeError):
    pass


@final
class ProbeClient:
    def __init__(
        self,
        port: int,
        capability: str,
        monotonic: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        self.url = f"http://127.0.0.1:{port}/jsonrpc"
        self.capability = capability
        self.request_id = 0
        self.monotonic = monotonic
        self.sleep = sleep

    def call(self, method: str, params: dict[str, object] | None = None) -> dict[str, object]:
        self.request_id += 1
        authenticated: dict[str, object] = {"capability": self.capability}
        if params:
            authenticated.update(params)
        request = Request(
            self.url,
            data=json.dumps(
                {
                    "jsonrpc": "2.0",
                    "method": method,
                    "id": self.request_id,
                    "params": authenticated,
                }
            ).encode(),
            headers={"Content-Type": "application/json"},
        )
        with cast("HTTPResponse", urlopen(request, timeout=10)) as response:
            body: bytes = response.read()
        decoded = cast("object", json.loads(body))
        if not isinstance(decoded, dict):
            raise ProbeRpcError("probe response was not an object")
        value = cast("dict[str, object]", decoded)
        if "error" in value:
            raise ProbeRpcError(f"probe returned {value['error']}")
        result = value.get("result")
        if not isinstance(result, dict):
            raise ProbeRpcError("probe result was not an object")
        return cast("dict[str, object]", result)

    def wait_ready(self, run_id: str, boot_nonce: str, timeout_seconds: float = 30) -> dict[str, object]:
        deadline = self.monotonic() + timeout_seconds
        last_error = "no response"
        while self.monotonic() < deadline:
            try:
                snapshot = self.snapshot()
                if snapshot.get("run_id") != run_id or snapshot.get("boot_nonce") != boot_nonce:
                    last_error = "session identity did not match"
                elif snapshot.get("ready") == "ready":
                    return snapshot
                else:
                    last_error = "probe has not reached recovery-ready"
            except (OSError, URLError, json.JSONDecodeError, ProbeRpcError) as error:
                last_error = str(error)
            self.sleep(0.1)
        raise TimeoutError(f"probe did not become ready: {last_error}")

    def snapshot(self) -> dict[str, object]:
        return self.call("clerestory/probe_snapshot")

    def records(self, after_sequence: int) -> dict[str, object]:
        return self.call(
            "clerestory/probe_records", {"after_sequence": after_sequence}
        )

    def command(
        self, command_id: str, command: dict[str, object]
    ) -> dict[str, object]:
        return self.call(
            "clerestory/probe_command",
            {"command_id": command_id, "command": command},
        )

    def shutdown(self) -> None:
        try:
            _ = self.call("clerestory/probe_shutdown")
        except (OSError, URLError, ProbeRpcError):
            pass
