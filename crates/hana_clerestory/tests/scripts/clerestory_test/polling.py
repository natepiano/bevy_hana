"""Action-relative polling with testable clock and observation boundaries."""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass
from typing import Generic
from typing import Protocol
from typing import TypeVar


Observation = TypeVar("Observation")


class Clock(Protocol):
    def monotonic(self) -> float: ...

    def sleep(self, seconds: float) -> None: ...


class SystemClock:
    def monotonic(self) -> float:
        return time.monotonic()

    def sleep(self, seconds: float) -> None:
        time.sleep(seconds)


@dataclass(frozen=True)
class WaitResult(Generic[Observation]):
    expected: str
    elapsed_seconds: float
    first: Observation
    final: Observation
    observations: int


class WaitTimedOut(TimeoutError):
    def __init__(self, expected: str, elapsed_seconds: float, last: object) -> None:
        super().__init__(f"timed out after {elapsed_seconds:.3f}s waiting for {expected}")
        self.expected: str = expected
        self.elapsed_seconds: float = elapsed_seconds
        self.last: object = last


def poll_until(
    observe: Callable[[], Observation],
    satisfied: Callable[[Observation], bool],
    *,
    expected: str,
    timeout_seconds: float,
    interval_seconds: float,
    clock: Clock,
) -> WaitResult[Observation]:
    started = clock.monotonic()
    first = observe()
    last = first
    observations = 1
    while not satisfied(last):
        elapsed = clock.monotonic() - started
        if elapsed >= timeout_seconds:
            raise WaitTimedOut(expected, elapsed, last)
        clock.sleep(min(interval_seconds, timeout_seconds - elapsed))
        last = observe()
        observations += 1
    return WaitResult(
        expected=expected,
        elapsed_seconds=clock.monotonic() - started,
        first=first,
        final=last,
        observations=observations,
    )

