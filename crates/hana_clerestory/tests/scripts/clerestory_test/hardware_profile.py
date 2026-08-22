"""Explicit command configuration for controllable monitor hardware."""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import cast


@dataclass(frozen=True)
class HardwareCommand:
    executable: str
    arguments: tuple[str, ...]
    timeout_seconds: float
    accepted_exit_codes: tuple[int, ...] = (0,)
    working_directory: str | None = None
    inherit_environment: tuple[str, ...] = ()
    environment: tuple[tuple[str, str], ...] = ()
    output_limit_bytes: int = 1_000_000

    @classmethod
    def from_dict(cls, value: dict[str, object]) -> HardwareCommand:
        executable = value.get("executable")
        timeout_seconds = value.get("timeout_seconds")
        if not isinstance(executable, str) or not executable:
            raise ValueError("hardware command needs a nonempty executable")
        arguments = _string_tuple(
            value.get("arguments", []), "hardware command arguments must be strings"
        )
        if not isinstance(timeout_seconds, int | float) or timeout_seconds <= 0:
            raise ValueError("hardware command timeout_seconds must be positive")
        accepted = _integer_tuple(
            value.get("accepted_exit_codes", [0]), "accepted_exit_codes must be integers"
        )
        environment = _string_pairs(
            value.get("environment", {}),
            "hardware command environment must contain string pairs",
        )
        inherit_environment = _string_tuple(
            value.get("inherit_environment", []),
            "inherit_environment must contain key names",
        )
        working_directory = value.get("working_directory")
        if working_directory is not None and not isinstance(working_directory, str):
            raise ValueError("working_directory must be a string")
        output_limit = value.get("output_limit_bytes", 1_000_000)
        if not isinstance(output_limit, int) or output_limit <= 0:
            raise ValueError("output_limit_bytes must be positive")
        return cls(
            executable=executable,
            arguments=arguments,
            timeout_seconds=float(timeout_seconds),
            accepted_exit_codes=accepted,
            working_directory=working_directory,
            inherit_environment=inherit_environment,
            environment=environment,
            output_limit_bytes=output_limit,
        )

    def run(self, inherited_environment: dict[str, str]) -> subprocess.CompletedProcess[bytes]:
        environment = {
            key: inherited_environment[key]
            for key in self.inherit_environment
            if key in inherited_environment
        }
        environment.update(self.environment)
        completed = subprocess.run(
            [self.executable, *self.arguments],
            cwd=self.working_directory,
            env=environment,
            capture_output=True,
            timeout=self.timeout_seconds,
            check=False,
        )
        if (
            len(completed.stdout) > self.output_limit_bytes
            or len(completed.stderr) > self.output_limit_bytes
        ):
            raise RuntimeError(
                f"hardware command output exceeded {self.output_limit_bytes} bytes"
            )
        if completed.returncode not in self.accepted_exit_codes:
            stderr = completed.stderr[: self.output_limit_bytes].decode(errors="replace")
            raise RuntimeError(
                f"hardware command exited {completed.returncode}: {stderr.strip()}"
            )
        return completed


@dataclass(frozen=True)
class ProbeMonitorMatcher:
    name: str | None = None
    refresh_rate_millihertz: int | None = None
    physical_position: tuple[int, int] | None = None
    physical_size: tuple[int, int] | None = None
    scale: float | None = None

    @classmethod
    def from_dict(cls, value: dict[str, object]) -> ProbeMonitorMatcher:
        name = value.get("name")
        refresh = value.get("refresh_rate_millihertz")
        position = _integer_pair(value.get("physical_position"), "physical_position")
        size = _integer_pair(value.get("physical_size"), "physical_size")
        scale = value.get("scale")
        if name is not None and not isinstance(name, str):
            raise ValueError("probe_monitor_matcher name must be a string")
        if refresh is not None and not isinstance(refresh, int):
            raise ValueError(
                "probe_monitor_matcher refresh_rate_millihertz must be an integer"
            )
        if scale is not None and not isinstance(scale, int | float):
            raise ValueError("probe_monitor_matcher scale must be numeric")
        if all(item is None for item in (name, refresh, position, size, scale)):
            raise ValueError("probe_monitor_matcher needs at least one field")
        return cls(
            name=name,
            refresh_rate_millihertz=refresh,
            physical_position=position,
            physical_size=size,
            scale=float(scale) if scale is not None else None,
        )

    def matches(self, environment: dict[str, str], index: int) -> bool:
        prefix = f"MONITOR_{index}_"
        checks = [
            self.name is None or environment.get(f"{prefix}NAME") == self.name,
            self.refresh_rate_millihertz is None
            or environment.get(f"{prefix}REFRESH_RATE_MILLIHERTZ")
            == str(self.refresh_rate_millihertz),
            self.physical_position is None
            or (
                environment.get(f"{prefix}POS_X") == str(self.physical_position[0])
                and environment.get(f"{prefix}POS_Y")
                == str(self.physical_position[1])
            ),
            self.physical_size is None
            or (
                environment.get(f"{prefix}WIDTH") == str(self.physical_size[0])
                and environment.get(f"{prefix}HEIGHT") == str(self.physical_size[1])
            ),
            self.scale is None
            or _optional_float(environment.get(f"{prefix}SCALE")) == self.scale,
        ]
        return all(checks)


@dataclass(frozen=True)
class HardwareProfile:
    name: str
    target_matcher: str
    inventory_name_field: str
    probe_monitor_matcher: ProbeMonitorMatcher
    power_off: HardwareCommand
    power_on: HardwareCommand
    inventory: HardwareCommand
    minimum_off_seconds: float
    minimum_on_seconds: float

    @classmethod
    def load(cls, path: Path) -> HardwareProfile:
        parsed = cast("object", json.loads(path.read_text()))
        if not isinstance(parsed, dict):
            raise ValueError("hardware profile root must be an object")
        raw = cast("dict[str, object]", parsed)
        name = raw.get("name")
        target_matcher = raw.get("target_matcher")
        inventory_name_field = raw.get("inventory_name_field", "_name")
        matcher_value = raw.get("probe_monitor_matcher")
        if not isinstance(name, str) or not name:
            raise ValueError("hardware profile needs a nonempty name")
        if not isinstance(target_matcher, str) or not target_matcher:
            raise ValueError("hardware profile needs a unique target_matcher")
        if not isinstance(inventory_name_field, str) or not inventory_name_field:
            raise ValueError("inventory_name_field must be a nonempty string")
        matcher_fields: dict[str, object]
        if isinstance(matcher_value, dict):
            matcher_fields = cast("dict[str, object]", matcher_value)
        else:
            matcher_fields = {"name": target_matcher}
        minimum_off_seconds = raw.get("minimum_off_seconds", 1)
        minimum_on_seconds = raw.get("minimum_on_seconds", 5)
        if not isinstance(minimum_off_seconds, int | float) or minimum_off_seconds < 0:
            raise ValueError("minimum_off_seconds must be nonnegative")
        if not isinstance(minimum_on_seconds, int | float) or minimum_on_seconds < 0:
            raise ValueError("minimum_on_seconds must be nonnegative")
        return cls(
            name=name,
            target_matcher=target_matcher,
            inventory_name_field=inventory_name_field,
            probe_monitor_matcher=ProbeMonitorMatcher.from_dict(matcher_fields),
            power_off=HardwareCommand.from_dict(_object(raw, "power_off")),
            power_on=HardwareCommand.from_dict(_object(raw, "power_on")),
            inventory=HardwareCommand.from_dict(_object(raw, "inventory")),
            minimum_off_seconds=float(minimum_off_seconds),
            minimum_on_seconds=float(minimum_on_seconds),
        )

    def inventory_match_count(self, inherited_environment: dict[str, str]) -> int:
        _, count = self.inventory_snapshot(inherited_environment)
        return count

    def inventory_snapshot(
        self, inherited_environment: dict[str, str]
    ) -> tuple[object, int]:
        completed = self.inventory.run(inherited_environment)
        value = cast("object", json.loads(completed.stdout))
        return (
            value,
            _matching_names(value, self.inventory_name_field, self.target_matcher),
        )


def _object(value: dict[str, object], key: str) -> dict[str, object]:
    child = value.get(key)
    if not isinstance(child, dict):
        raise ValueError(f"hardware profile {key} must be an object")
    return cast("dict[str, object]", child)


def _string_tuple(value: object, message: str) -> tuple[str, ...]:
    if not isinstance(value, list):
        raise ValueError(message)
    result: list[str] = []
    for item in cast("list[object]", value):
        if not isinstance(item, str):
            raise ValueError(message)
        result.append(item)
    return tuple(result)


def _integer_tuple(value: object, message: str) -> tuple[int, ...]:
    if not isinstance(value, list):
        raise ValueError(message)
    result: list[int] = []
    for item in cast("list[object]", value):
        if not isinstance(item, int):
            raise ValueError(message)
        result.append(item)
    return tuple(result)


def _string_pairs(value: object, message: str) -> tuple[tuple[str, str], ...]:
    if not isinstance(value, dict):
        raise ValueError(message)
    pairs: list[tuple[str, str]] = []
    for key, item in cast("dict[object, object]", value).items():
        if not isinstance(key, str) or not isinstance(item, str):
            raise ValueError(message)
        pairs.append((key, item))
    return tuple(sorted(pairs))


def _integer_pair(value: object, field_name: str) -> tuple[int, int] | None:
    if value is None:
        return None
    message = f"probe_monitor_matcher {field_name} must contain two integers"
    if not isinstance(value, list):
        raise ValueError(message)
    items = cast("list[object]", value)
    if len(items) != 2:
        raise ValueError(message)
    first, second = items[0], items[1]
    if not isinstance(first, int) or not isinstance(second, int):
        raise ValueError(message)
    return first, second


def _optional_float(value: str | None) -> float | None:
    try:
        return float(value) if value is not None else None
    except ValueError:
        return None


def _matching_names(value: object, name_field: str, target: str) -> int:
    if isinstance(value, dict):
        mapping = cast("dict[str, object]", value)
        own_match = int(mapping.get(name_field) == target)
        return own_match + sum(
            _matching_names(child, name_field, target) for child in mapping.values()
        )
    if isinstance(value, list):
        return sum(
            _matching_names(child, name_field, target)
            for child in cast("list[object]", value)
        )
    return 0
