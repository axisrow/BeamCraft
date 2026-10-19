#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
# Contributor-original measurement summarizer; Python standard library only.
"""Aggregate raw window_bench JSON without hiding variation between process runs."""
import argparse
import gzip
import json
import statistics
from pathlib import Path


def describe(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values),
            "samples": values}


def summarize(root):
    result = {}
    for label in ("before", "after"):
        for fixture in ("native", "stress"):
            name = f"{label}-{fixture}"
            path = root / f"{name}.json"
            content = path.read_bytes() if path.exists() else gzip.decompress(
                path.with_suffix(".json.gz").read_bytes())
            data = json.loads(content)
            iterations = data["iterations"]
            phases = []
            for index, first in enumerate(iterations[0]["phases"]):
                metrics = {"index": index, "action": first["action"]}
                for key in ("ui_cpu_percent_one_core", "ui_cpu_s", "median_ui_rss_bytes",
                            "median_ui_pss_bytes", "peak_tree_rss_bytes", "peak_tree_pss_bytes"):
                    metrics[key] = describe([r["phases"][index][key] for r in iterations])
                phases.append(metrics)
            refresh_calls = []
            for run in iterations:
                calls = run["cli_calls"]
                expected = 4 if label == "before" else 2
                if len(calls) != expected or any(c["code"] for c in calls):
                    raise ValueError(f"{name}: incomplete/failed initial load or refresh: {calls}")
                refresh_calls.append(calls[expected // 2:])
            result[name] = {
                "binary_sha256": data["binary_sha256"], "repetitions": len(iterations),
                "warmup_s": data["warmup_after_cli_completion_s"], "phases": phases,
                "settle_refresh": data.get("settle_refresh", False),
                "refresh_cli_calls": describe([len(c) for c in refresh_calls]),
                "refresh_cli_child_cpu_s": describe([sum(c["children_user_s"] + c["children_system_s"] for c in calls)
                                                       for calls in refresh_calls]),
                "refresh_cli_wall_sum_s": describe([sum(c["wall_s"] for c in calls) for calls in refresh_calls]),
            }
            if data.get("settle_refresh"):
                result[name]["whole_refresh_peak_tree_rss_bytes"] = describe([
                    max(run["refresh_settle"]["peak_tree_rss_bytes"],
                        max(p["peak_tree_rss_bytes"] for p in run["phases"] if p["action"] == "refresh"))
                    for run in iterations])
                result[name]["refresh_settle_elapsed_s"] = describe([
                    run["refresh_settle"]["elapsed_s"] for run in iterations])
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    text = json.dumps(summarize(args.directory), indent=2) + "\n"
    if args.output:
        args.output.write_text(text)
    else:
        print(text, end="")
