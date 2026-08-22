#!/usr/bin/env python3
"""Compare two benchmark result directories and produce a markdown report."""

import argparse
import csv
import glob
import re
import sys
from pathlib import Path
from typing import TypedDict


class ScenarioRow(TypedDict):
    scenario: str
    frames: float
    avg_ms: float
    median_ms: float
    p95_ms: float
    p99_ms: float
    min_ms: float
    max_ms: float
    avg_fps: float


class OutlineCost(TypedDict):
    off_median: float
    on_median: float
    total_cost: float
    per_entity_cost: float | None
    entity_count: int


NUMERIC_COLS: list[str] = [
    "frames",
    "avg_ms",
    "median_ms",
    "p95_ms",
    "p99_ms",
    "min_ms",
    "max_ms",
    "avg_fps",
]

PER_ENTITY_THRESHOLD = 100
TARGET_FRAME_MS = 16.67
EXTREME_THRESHOLD = 50000


def filter_extreme(data: dict[str, ScenarioRow]) -> dict[str, ScenarioRow]:
    """Remove scenarios at or above the extreme entity threshold."""
    return {
        name: row
        for name, row in data.items()
        if (parsed := parse_scenario_name(name)) is None or parsed[0] < EXTREME_THRESHOLD
    }


def load_and_average(directory: Path) -> tuple[dict[str, ScenarioRow], int]:
    """Load all CSVs from a directory and average numeric columns per scenario."""
    csv_files = sorted(glob.glob(str(directory / "*.csv")))
    if not csv_files:
        print(f"Error: no CSV files found in {directory}", file=sys.stderr)
        sys.exit(1)

    # scenario -> column -> list of values
    accumulator: dict[str, dict[str, list[float]]] = {}

    for filepath in csv_files:
        with open(filepath, newline="") as f:
            reader = csv.DictReader(f)
            for raw_row in reader:
                scenario = raw_row["scenario"]
                if scenario not in accumulator:
                    accumulator[scenario] = {col: [] for col in NUMERIC_COLS}
                for col in NUMERIC_COLS:
                    accumulator[scenario][col].append(float(raw_row[col]))

    averaged: dict[str, ScenarioRow] = {}
    for scenario, cols in accumulator.items():
        row: ScenarioRow = {"scenario": scenario}  # pyright: ignore[reportAssignmentType]
        for col in NUMERIC_COLS:
            values = cols[col]
            row[col] = sum(values) / len(values)
        averaged[scenario] = row

    return averaged, len(csv_files)


def parse_scenario_name(name: str) -> tuple[int, str] | None:
    """Extract entity count and on/off state from scenario name like 'Entities100 on'."""
    m = re.match(r"Entities(\d+)\s+(on|off)", name)
    if m:
        return int(m.group(1)), m.group(2)
    return None


def compute_outline_costs(data: dict[str, ScenarioRow]) -> dict[int, OutlineCost]:
    """For each entity count, compute outline cost = on_median - off_median."""
    # Group by entity count
    by_count: dict[int, dict[str, float]] = {}
    for scenario, row in data.items():
        parsed = parse_scenario_name(scenario)
        if parsed is None:
            continue
        count, state = parsed
        if count not in by_count:
            by_count[count] = {}
        by_count[count][state] = row["median_ms"]

    costs: dict[int, OutlineCost] = {}
    for count in sorted(by_count.keys()):
        states = by_count[count]
        if "on" not in states or "off" not in states:
            continue
        off_median = states["off"]
        on_median = states["on"]
        total_cost = on_median - off_median
        per_entity: float | None = None
        if count >= PER_ENTITY_THRESHOLD:
            per_entity = total_cost / count
        costs[count] = OutlineCost(
            off_median=off_median,
            on_median=on_median,
            total_cost=total_cost,
            per_entity_cost=per_entity,
            entity_count=count,
        )
    return costs


def estimate_60fps_budget(costs: dict[int, OutlineCost]) -> int | None:
    """Estimate max outlined entities at 60 FPS using on_median / entity_count from highest count."""
    best_count: int | None = None
    for count in costs:
        if best_count is None or count > best_count:
            best_count = count
    if best_count is None:
        return None

    cost = costs[best_count]
    on_median = cost["on_median"]
    if on_median <= 0:
        return None

    cost_per_entity = on_median / best_count
    return int(TARGET_FRAME_MS / cost_per_entity)


def fmt_delta(new: float, old: float) -> str:
    """Format absolute delta with +/- and percentage."""
    diff = new - old
    if old != 0:
        pct = (diff / old) * 100
    else:
        pct = 0.0
    sign = "+" if diff >= 0 else ""
    return f"{sign}{diff:.2f} ({sign}{pct:.1f}%)"


def print_run_info(
    name_a: str,
    count_a: int,
    name_b: str,
    count_b: int,
) -> None:
    """Print run info section."""
    print("## Run Info\n")
    print("| | Directory | CSV Files |")
    print("|---|---|---|")
    print(f"| **A (new)** | `{name_a}` | {count_a} |")
    print(f"| **B (baseline)** | `{name_b}` | {count_b} |")
    print()


def format_results_row(row: ScenarioRow) -> str:
    """Format a single scenario row for the results table."""
    scenario = row["scenario"]
    avg = row["avg_ms"]
    median = row["median_ms"]
    p95 = row["p95_ms"]
    p99 = row["p99_ms"]
    mn = row["min_ms"]
    mx = row["max_ms"]
    fps = row["avg_fps"]
    return f"| {scenario} | {avg:.2f} | {median:.2f} | {p95:.2f} | {p99:.2f} | {mn:.2f} | {mx:.2f} | {fps:.0f} |"


def print_results_table(label: str, data: dict[str, ScenarioRow]) -> None:
    """Print a full stats table for one dataset."""
    print(f"## Individual Results — {label}\n")
    header = "| Scenario | Avg ms | Median ms | P95 ms | P99 ms | Min ms | Max ms | Avg FPS |"
    print(header)
    print("|---|---|---|---|---|---|---|---|")
    for scenario in sorted(data.keys(), key=scenario_sort_key):
        print(format_results_row(data[scenario]))
    print()


def scenario_sort_key(name: str) -> tuple[int, int]:
    """Sort scenarios by entity count then off < on."""
    parsed = parse_scenario_name(name)
    if parsed is None:
        return (0, 0)
    count, state = parsed
    return (count, 0 if state == "off" else 1)


def format_comparison_row(
    scenario: str,
    row_a: ScenarioRow,
    row_b: ScenarioRow,
) -> str:
    """Format a comparison row with deltas."""
    delta_ms = fmt_delta(row_a["median_ms"], row_b["median_ms"])
    delta_fps = fmt_delta(row_a["avg_fps"], row_b["avg_fps"])
    a_med = row_a["median_ms"]
    b_med = row_b["median_ms"]
    a_fps = row_a["avg_fps"]
    b_fps = row_b["avg_fps"]
    return f"| {scenario} | {a_med:.2f} | {b_med:.2f} | {delta_ms} | {a_fps:.0f} | {b_fps:.0f} | {delta_fps} |"


def print_comparison_table(
    data_a: dict[str, ScenarioRow],
    data_b: dict[str, ScenarioRow],
) -> None:
    """Print comparison table with deltas."""
    print("## Comparison (A vs B)\n")
    header = "| Scenario | A Median ms | B Median ms | Delta ms | A FPS | B FPS | Delta FPS |"
    print(header)
    print("|---|---|---|---|---|---|---|")

    all_scenarios = sorted(
        set(data_a.keys()) | set(data_b.keys()), key=scenario_sort_key
    )
    for scenario in all_scenarios:
        row_a = data_a.get(scenario)
        row_b = data_b.get(scenario)
        if row_a is None or row_b is None:
            a_med = f"{row_a['median_ms']:.2f}" if row_a else "N/A"
            b_med = f"{row_b['median_ms']:.2f}" if row_b else "N/A"
            a_fps = f"{row_a['avg_fps']:.0f}" if row_a else "N/A"
            b_fps = f"{row_b['avg_fps']:.0f}" if row_b else "N/A"
            print(f"| {scenario} | {a_med} | {b_med} | N/A | {a_fps} | {b_fps} | N/A |")
        else:
            print(format_comparison_row(scenario, row_a, row_b))
    print()


def format_outline_cost_row(
    count: int,
    ca: OutlineCost | None,
    cb: OutlineCost | None,
) -> str:
    """Format a single outline cost row."""
    a_total = f"{ca['total_cost']:.2f}" if ca else "N/A"
    b_total = f"{cb['total_cost']:.2f}" if cb else "N/A"

    if ca and cb:
        delta_total = fmt_delta(ca["total_cost"], cb["total_cost"])
    else:
        delta_total = "N/A"

    a_per = (
        f"{ca['per_entity_cost']:.4f}"
        if ca and ca["per_entity_cost"] is not None
        else "—"
    )
    b_per = (
        f"{cb['per_entity_cost']:.4f}"
        if cb and cb["per_entity_cost"] is not None
        else "—"
    )

    if (
        ca
        and cb
        and ca["per_entity_cost"] is not None
        and cb["per_entity_cost"] is not None
    ):
        delta_per = fmt_delta(ca["per_entity_cost"], cb["per_entity_cost"])
    else:
        delta_per = "—"

    return f"| {count} | {a_total} | {b_total} | {delta_total} | {a_per} | {b_per} | {delta_per} |"


def print_outline_cost_analysis(
    costs_a: dict[int, OutlineCost],
    costs_b: dict[int, OutlineCost],
) -> None:
    """Print outline cost analysis with deltas."""
    print("## Outline Cost Analysis\n")
    header = "| Entities | A Cost ms | B Cost ms | Delta | A Per-Entity | B Per-Entity | Delta |"
    print(header)
    print("|---|---|---|---|---|---|---|")

    all_counts = sorted(set(costs_a.keys()) | set(costs_b.keys()))
    for count in all_counts:
        ca = costs_a.get(count)
        cb = costs_b.get(count)
        print(format_outline_cost_row(count, ca, cb))
    print()


def print_budget_line(
    label: str,
    costs: dict[int, OutlineCost],
    budget: int | None,
) -> None:
    """Print a single budget line for one dataset."""
    if budget is None:
        print(f"- **{label}**: insufficient data")
        return

    best_count = max(
        (c for c, v in costs.items() if v["per_entity_cost"] is not None),
        default=None,
    )
    if best_count is None:
        print(f"- **{label}**: insufficient data")
        return

    cost = costs[best_count]
    on_median = cost["on_median"]
    cost_per_entity = on_median / best_count
    print(f"- **{label}**: ~**{budget:,}** outlined entities (on_median: {on_median:.2f} ms, per-entity: {cost_per_entity:.4f} ms, from {best_count}-entity scenario)")


def print_budget_section(
    costs_a: dict[int, OutlineCost],
    costs_b: dict[int, OutlineCost],
) -> None:
    """Print 60 FPS budget estimate."""
    budget_a = estimate_60fps_budget(costs_a)
    budget_b = estimate_60fps_budget(costs_b)

    print("## 60 FPS Budget Estimate\n")
    print(f"Target frame time: {TARGET_FRAME_MS:.2f} ms\n")

    print_budget_line("A", costs_a, budget_a)
    print_budget_line("B", costs_b, budget_b)

    if budget_a is not None and budget_b is not None:
        diff = budget_a - budget_b
        sign = "+" if diff >= 0 else ""
        print(f"\nDelta: {sign}{diff:,} entities")
    print()


def print_summary(
    data_a: dict[str, ScenarioRow],
    data_b: dict[str, ScenarioRow],
) -> None:
    """Print auto-generated summary."""
    print("## Summary\n")

    # Find biggest movers by median_ms percentage change
    movers: list[tuple[str, float, float]] = []
    for scenario in sorted(
        set(data_a.keys()) & set(data_b.keys()), key=scenario_sort_key
    ):
        a_med = data_a[scenario]["median_ms"]
        b_med = data_b[scenario]["median_ms"]
        if b_med != 0:
            pct = ((a_med - b_med) / b_med) * 100
            movers.append((scenario, a_med - b_med, pct))

    if not movers:
        print("No comparable scenarios found.\n")
        return

    # Overall direction
    avg_pct = sum(m[2] for m in movers) / len(movers)
    if abs(avg_pct) < 0.5:
        print("**Overall**: No significant change (avg delta < 0.5%).\n")
    elif avg_pct > 0:
        print(f"**Overall**: A is **slower** by {avg_pct:.1f}% on average.\n")
    else:
        print(f"**Overall**: A is **faster** by {abs(avg_pct):.1f}% on average.\n")

    # Biggest movers (top 3 by absolute percentage)
    movers_sorted = sorted(movers, key=lambda m: abs(m[2]), reverse=True)
    print("**Biggest movers** (by median ms % change):\n")
    for scenario, delta_ms, pct in movers_sorted[:3]:
        sign = "+" if delta_ms >= 0 else ""
        direction = "slower" if delta_ms > 0 else "faster"
        print(f"- {scenario}: {sign}{delta_ms:.2f} ms ({sign}{pct:.1f}%) — {direction}")
    print()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Compare two benchmark result directories and produce a markdown report.",
    )
    _ = parser.add_argument("dir_a", help="New results directory")
    _ = parser.add_argument("dir_b", help="Baseline results directory")
    _ = parser.add_argument(
        "--extreme",
        action="store_true",
        help=f"Include extreme-scale scenarios ({EXTREME_THRESHOLD}+ entities)",
    )
    if len(sys.argv) == 1:
        parser.print_help(sys.stderr)
        sys.exit(1)

    args = parser.parse_args()

    dir_a = Path(args.dir_a)  # pyright: ignore[reportAny]
    dir_b = Path(args.dir_b)  # pyright: ignore[reportAny]
    include_extreme: bool = args.extreme  # pyright: ignore[reportAny]

    if not dir_a.is_dir():
        print(f"Error: {dir_a} is not a directory", file=sys.stderr)
        sys.exit(1)
    if not dir_b.is_dir():
        print(f"Error: {dir_b} is not a directory", file=sys.stderr)
        sys.exit(1)

    data_a, count_a = load_and_average(dir_a)
    data_b, count_b = load_and_average(dir_b)

    if not include_extreme:
        data_a = filter_extreme(data_a)
        data_b = filter_extreme(data_b)

    costs_a = compute_outline_costs(data_a)
    costs_b = compute_outline_costs(data_b)

    print("# Benchmark Comparison Report\n")
    print_run_info(str(dir_a), count_a, str(dir_b), count_b)
    print_results_table("A (new)", data_a)
    print_results_table("B (baseline)", data_b)
    print_comparison_table(data_a, data_b)
    print_outline_cost_analysis(costs_a, costs_b)
    print_budget_section(costs_a, costs_b)
    print_summary(data_a, data_b)


if __name__ == "__main__":
    main()
