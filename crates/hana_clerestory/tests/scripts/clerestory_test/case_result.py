"""Versioned result records shared by restore and reconnect test cases."""

from __future__ import annotations

from dataclasses import asdict
from dataclasses import dataclass
from dataclasses import field
from enum import StrEnum
from pathlib import Path
from typing import Final
from typing import cast


RESULT_SCHEMA_VERSION: Final = 1


class Interaction(StrEnum):
    AUTOMATED = "automated"
    OPERATOR_ACTION = "operator-action"
    OPERATOR_JUDGMENT = "operator-judgment"


class Evidence(StrEnum):
    APPLICATION = "application"
    SYNTHETIC = "synthetic"
    PHYSICAL = "physical"


class Availability(StrEnum):
    AVAILABLE = "available"
    UNAVAILABLE = "unavailable"
    UNSUPPORTED = "unsupported"


class Outcome(StrEnum):
    NOT_RUN = "not-run"
    PASSED = "passed"
    FAILED = "failed"
    TIMED_OUT = "timed-out"
    INTERRUPTED = "interrupted"
    ABORTED = "aborted"
    HARNESS_ERROR = "harness-error"


@dataclass(frozen=True)
class AssertionResult:
    name: str
    passed: bool
    detail: str


@dataclass(frozen=True)
class CleanupResult:
    attempted: bool
    succeeded: bool
    detail: str


@dataclass
class CaseResult:
    case_id: str
    interaction: Interaction
    evidence: Evidence
    availability: Availability = Availability.AVAILABLE
    outcome: Outcome = Outcome.NOT_RUN
    availability_reason: str | None = None
    missing_capability: str | None = None
    applicable: bool = True
    assertions: list[AssertionResult] = field(default_factory=list)
    workaround_subruns: list[CaseResult] = field(default_factory=list)
    process_exit_code: int | None = None
    elapsed_seconds: float = 0.0
    artifacts: list[str] = field(default_factory=list)
    cleanup: CleanupResult | None = None
    schema_version: int = RESULT_SCHEMA_VERSION

    def add_artifact(self, path: Path) -> None:
        self.artifacts.append(str(path))

    def finish_from_assertions(self) -> None:
        if self.availability is not Availability.AVAILABLE or not self.applicable:
            self.outcome = Outcome.NOT_RUN
        elif self.assertions and all(assertion.passed for assertion in self.assertions):
            self.outcome = Outcome.PASSED
        else:
            self.outcome = Outcome.FAILED

    def to_dict(self) -> dict[str, object]:
        return asdict(self)

    @classmethod
    def from_dict(cls, value: dict[str, object]) -> CaseResult:
        assertions = [
            AssertionResult(
                name=str(assertion["name"]),
                passed=bool(assertion["passed"]),
                detail=str(assertion["detail"]),
            )
            for assertion in _object_list(value.get("assertions"), "assertions")
        ]
        subruns = [
            cls.from_dict(subrun)
            for subrun in _object_list(value.get("workaround_subruns"), "workaround_subruns")
        ]
        cleanup_value = value.get("cleanup")
        cleanup = None
        if isinstance(cleanup_value, dict):
            cleanup_fields = cast("dict[str, object]", cleanup_value)
            cleanup = CleanupResult(
                attempted=bool(cleanup_fields.get("attempted", False)),
                succeeded=bool(cleanup_fields.get("succeeded", False)),
                detail=str(cleanup_fields.get("detail", "")),
            )
        return cls(
            case_id=str(value["case_id"]),
            interaction=Interaction(str(value["interaction"])),
            evidence=Evidence(str(value["evidence"])),
            availability=Availability(str(value.get("availability", Availability.AVAILABLE))),
            outcome=Outcome(str(value.get("outcome", Outcome.NOT_RUN))),
            availability_reason=_optional_string(value.get("availability_reason")),
            missing_capability=_optional_string(value.get("missing_capability")),
            applicable=bool(value.get("applicable", True)),
            assertions=assertions,
            workaround_subruns=subruns,
            process_exit_code=_optional_integer(value.get("process_exit_code")),
            elapsed_seconds=_coerce_float(value.get("elapsed_seconds"), 0.0),
            artifacts=_string_list(value.get("artifacts")),
            cleanup=cleanup,
            schema_version=_coerce_integer(value.get("schema_version"), RESULT_SCHEMA_VERSION),
        )


def unavailable_case(
    case_id: str,
    interaction: Interaction,
    evidence: Evidence,
    reason: str,
    missing_capability: str,
) -> CaseResult:
    return CaseResult(
        case_id=case_id,
        interaction=interaction,
        evidence=evidence,
        availability=Availability.UNAVAILABLE,
        availability_reason=reason,
        missing_capability=missing_capability,
    )


def _object_list(value: object, field_name: str) -> list[dict[str, object]]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{field_name} must contain objects")
    objects: list[dict[str, object]] = []
    for item in cast("list[object]", value):
        if not isinstance(item, dict):
            raise ValueError(f"{field_name} must contain objects")
        objects.append(cast("dict[str, object]", item))
    return objects


def _string_list(value: object) -> list[str]:
    if not isinstance(value, list):
        return []
    return [str(item) for item in cast("list[object]", value)]


def _optional_string(value: object) -> str | None:
    return value if isinstance(value, str) else None


def _optional_integer(value: object) -> int | None:
    return value if isinstance(value, int) else None


def _coerce_float(value: object, default: float) -> float:
    return float(value) if isinstance(value, (int, float)) else default


def _coerce_integer(value: object, default: int) -> int:
    return int(value) if isinstance(value, int) else default
