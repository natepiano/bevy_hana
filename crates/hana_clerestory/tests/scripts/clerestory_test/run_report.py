"""Crash-resilient progress journal and suite reports."""

from __future__ import annotations

import json
import os
import tempfile
from dataclasses import asdict
from dataclasses import dataclass
from datetime import UTC
from datetime import datetime
from pathlib import Path
from typing import final

from .case_result import CaseResult


@dataclass(frozen=True)
class ProgressEvent:
    timestamp: str
    kind: str
    message: str
    case_id: str | None = None


@final
class RunReport:
    def __init__(self, artifact_directory: Path, run_id: str, source_revision: str) -> None:
        self.artifact_directory = artifact_directory
        self.run_id = run_id
        self.source_revision = source_revision
        self.results: list[CaseResult] = []
        self.metadata: dict[str, object] = {}
        artifact_directory.mkdir(parents=True, exist_ok=False)
        self.journal_path = artifact_directory / "progress.jsonl"
        self.json_path = artifact_directory / "results.json"
        self.markdown_path = artifact_directory / "results.md"
        self.heartbeat_path = artifact_directory / "heartbeat"
        self.write_partial()

    def event(self, kind: str, message: str, case_id: str | None = None) -> None:
        event = ProgressEvent(
            timestamp=datetime.now(UTC).isoformat(),
            kind=kind,
            message=message,
            case_id=case_id,
        )
        with self.journal_path.open("a") as journal:
            _ = journal.write(json.dumps(asdict(event), sort_keys=True) + "\n")
            journal.flush()
            os.fsync(journal.fileno())
        self.heartbeat_path.touch()

    def append(self, result: CaseResult) -> None:
        self.results.append(result)
        self.write_partial()

    def update_metadata(self, **values: object) -> None:
        self.metadata.update(values)
        self.write_partial()

    def write_partial(self) -> None:
        payload = {
            "schema_version": 1,
            "run_id": self.run_id,
            "source_revision": self.source_revision,
            "metadata": self.metadata,
            "results": [result.to_dict() for result in self.results],
        }
        self._atomic_text(self.json_path, json.dumps(payload, indent=2, sort_keys=True) + "\n")
        self._atomic_text(self.markdown_path, self._markdown())

    def _markdown(self) -> str:
        lines = [
            "# Clerestory test results",
            "",
            f"Run: `{self.run_id}`  ",
            f"Source: `{self.source_revision}`",
            "",
            "| Case | Interaction | Evidence | Availability | Outcome | Cleanup |",
            "| --- | --- | --- | --- | --- | --- |",
        ]
        for result in self.results:
            lines.append(
                f"| `{result.case_id}` | {result.interaction.value} | {result.evidence.value} | "
                + f"{result.availability.value} | {result.outcome.value} | "
                + f"{_cleanup_label(result)} |"
            )
        lines.append("")
        return "\n".join(lines)

    @staticmethod
    def _atomic_text(path: Path, content: str) -> None:
        with tempfile.NamedTemporaryFile(
            mode="w",
            dir=path.parent,
            prefix=f".{path.name}.",
            delete=False,
        ) as temporary:
            _ = temporary.write(content)
            temporary.flush()
            os.fsync(temporary.fileno())
            temporary_path = Path(temporary.name)
        _ = temporary_path.replace(path)


def _cleanup_label(result: CaseResult) -> str:
    if result.cleanup is None:
        return "not applicable"
    return "passed" if result.cleanup.succeeded else "failed"
