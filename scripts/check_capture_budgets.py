#!/usr/bin/env python3
"""Check isolated Rust projection counts and timing against a committed report."""

import json
import math
import sys


METRICS = {
    "allocations_per_capture": ("Rust allocations", "max"),
    "rust_bytes_per_capture": ("requested Rust bytes", "max"),
    "isolated_projector_capture": ("ms", "p95"),
}


def check(actual, baseline):
    for report in (actual, baseline):
        if report.get("schema") != 1 or report.get("sample_count", 0) < 1:
            raise ValueError("invalid schema or sample count")
        for name, (unit, _) in METRICS.items():
            metric = report["metrics"][name]
            if metric["unit"] != unit or metric["samples"] != report["sample_count"]:
                raise ValueError(f"{name}: incompatible unit or sample count")
            for field in ("median", "p95", "max", "budget"):
                value = metric[field]
                if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
                    raise ValueError(f"{name}: invalid {field}")
    for name, (_, statistic) in METRICS.items():
        value = actual["metrics"][name][statistic]
        budget = baseline["metrics"][name]["budget"]
        if value > budget:
            raise ValueError(f"{name}: budget exceeded ({value} > {budget})")
        print(f"{name}: {statistic} {value} <= {budget}")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3:
            raise ValueError("usage: check_capture_budgets.py ACTUAL.json BUDGET.json")
        with open(sys.argv[1], encoding="utf-8") as actual_file:
            actual = json.load(actual_file)
        with open(sys.argv[2], encoding="utf-8") as budget_file:
            baseline = json.load(budget_file)
        check(actual, baseline)
    except (ValueError, KeyError, TypeError, OSError) as error:
        sys.exit(str(error))
