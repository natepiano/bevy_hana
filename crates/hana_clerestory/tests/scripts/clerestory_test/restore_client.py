"""Small JSON-RPC client for the restore-window test example."""

from __future__ import annotations

import json
import time
from http.client import HTTPResponse
from typing import cast
from typing import final
from urllib.error import URLError
from urllib.request import Request
from urllib.request import urlopen


WINDOW_COMPONENT = "bevy_window::window::Window"
PRIMARY_COMPONENT = "bevy_window::window::PrimaryWindow"


@final
class RestoreClient:
    def __init__(self, port: int) -> None:
        self.url = f"http://127.0.0.1:{port}/jsonrpc"
        self.request_id = 0

    def call(self, method: str, params: object = None) -> object:
        self.request_id += 1
        request = Request(
            self.url,
            data=json.dumps(
                {
                    "jsonrpc": "2.0",
                    "method": method,
                    "id": self.request_id,
                    "params": params,
                }
            ).encode(),
            headers={"Content-Type": "application/json"},
        )
        with cast("HTTPResponse", urlopen(request, timeout=10)) as response:
            body: bytes = response.read()
        decoded = cast("object", json.loads(body))
        if not isinstance(decoded, dict):
            raise RuntimeError("restore client response was not an object")
        value = cast("dict[str, object]", decoded)
        if "error" in value:
            raise RuntimeError(f"restore client returned {value['error']}")
        return value.get("result")

    def wait_ready(self, timeout_seconds: float) -> None:
        deadline = time.monotonic() + timeout_seconds
        last_error = "no response"
        while time.monotonic() < deadline:
            try:
                _ = self.call("rpc.discover")
                return
            except (OSError, URLError, json.JSONDecodeError, RuntimeError) as error:
                last_error = str(error)
            time.sleep(0.1)
        raise TimeoutError(f"restore app did not become ready: {last_error}")

    def primary_mode(self) -> str | None:
        snapshot = self.primary_snapshot()
        mode = snapshot.get("mode")
        if not isinstance(mode, str):
            return None
        if mode.startswith("BorderlessFullscreen"):
            return "BorderlessFullscreen"
        if mode.startswith("Fullscreen"):
            return "Fullscreen"
        return mode

    def primary_snapshot(self) -> dict[str, object]:
        result = self.call("clerestory/window_snapshot")
        if not isinstance(result, dict):
            raise RuntimeError("window snapshot was not an object")
        return cast("dict[str, object]", result)

    def primary_mode_from_world(self) -> str | None:
        result = self.call(
            "world.query",
            {
                "data": {"components": [WINDOW_COMPONENT]},
                "filter": {"with": [PRIMARY_COMPONENT]},
            },
        )
        if not isinstance(result, list):
            return None
        entities = cast("list[object]", result)
        if len(entities) != 1:
            return None
        entity = entities[0]
        if not isinstance(entity, dict):
            return None
        components = cast("dict[str, object]", entity).get("components")
        if not isinstance(components, dict):
            return None
        window = cast("dict[str, object]", components).get(WINDOW_COMPONENT)
        if not isinstance(window, dict):
            return None
        mode = cast("dict[str, object]", window).get("mode")
        if isinstance(mode, str):
            return mode
        if isinstance(mode, dict) and len(cast("dict[str, object]", mode)) == 1:
            return str(next(iter(cast("dict[str, object]", mode))))
        return None

    def shutdown(self) -> None:
        try:
            _ = self.call("clerestory/shutdown")
        except (OSError, URLError, json.JSONDecodeError, RuntimeError):
            pass
