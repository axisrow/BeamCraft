#!/usr/bin/env python3
# Contributor-original performance harness, MPL-2.0.
"""Measure the same native UI event sequence before and after an optimization."""
import argparse
import hashlib
import json
import os
import platform
import statistics
import subprocess
import time
from pathlib import Path

import psutil


def xdo(*args):
    return subprocess.check_output(["xdotool", *map(str, args)], text=True).strip()


def readings(pid):
    process = psutil.Process(pid)
    cpu = process.cpu_times()
    memory = process.memory_full_info()
    rss = memory.rss
    pss = getattr(memory, "pss", 0)
    for child in process.children(recursive=True):
        try:
            mem = child.memory_full_info()
            rss += mem.rss
            pss += getattr(mem, "pss", 0)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass
    return {
        "ui_cpu_s": cpu.user + cpu.system,
        "ui_and_reaped_children_cpu_s": cpu.user + cpu.system + cpu.children_user + cpu.children_system,
        "ui_rss_bytes": memory.rss,
        "ui_pss_bytes": getattr(memory, "pss", 0),
        "tree_rss_bytes": rss, "tree_pss_bytes": pss,
    }


def phase(process, window, action, duration):
    start = time.monotonic()
    first = readings(process.pid)
    samples = [dict(monotonic=start, **first)]
    next_action = start
    index = 0
    try:
        if action == "orbit":
            xdo("mousemove", "--window", window, 900, 450)
            xdo("mousedown", 1)
        while time.monotonic() - start < duration:
            now = time.monotonic()
            if now >= next_action:
                if action == "orbit":
                    xdo("mousemove", "--window", window, 900 + (index % 16) * 6, 450 + (index % 5) * 4)
                elif action == "resize":
                    xdo("windowsize", window, 1250 + (index % 8) * 20, 800 + (index % 4) * 15)
                elif action == "refresh" and index == 0:
                    xdo("key", "--window", window, "r")
                next_action = now + 0.1
                index += 1
            samples.append(dict(monotonic=now, **readings(process.pid)))
            time.sleep(0.05)
        last = readings(process.pid)
    finally:
        if action == "orbit":
            xdo("mouseup", 1)
    elapsed = time.monotonic() - start
    return {
        "action": action, "elapsed_s": elapsed,
        "ui_cpu_s": last["ui_cpu_s"] - first["ui_cpu_s"],
        "ui_cpu_percent_one_core": (last["ui_cpu_s"] - first["ui_cpu_s"]) / elapsed * 100,
        "ui_and_reaped_children_cpu_s": last["ui_and_reaped_children_cpu_s"] - first["ui_and_reaped_children_cpu_s"],
        "median_ui_rss_bytes": statistics.median(s["ui_rss_bytes"] for s in samples),
        "median_ui_pss_bytes": statistics.median(s["ui_pss_bytes"] for s in samples),
        "peak_tree_pss_bytes": max(s["tree_pss_bytes"] for s in samples),
        "peak_tree_rss_bytes": max(s["tree_rss_bytes"] for s in samples),
        "samples": samples,
    }


def settle_refresh(process, trace, expected_calls):
    """Observe the whole asynchronous refresh, then ten seconds for upload/render to settle."""
    start = time.monotonic()
    ready_at = None
    samples = []
    while time.monotonic() - start < 120:
        if process.poll() is not None:
            raise RuntimeError("UI exited during refresh")
        samples.append(dict(monotonic=time.monotonic(), **readings(process.pid)))
        calls = [json.loads(line) for line in trace.read_text().splitlines() if line]
        if any(c["code"] != 0 for c in calls):
            raise RuntimeError("CLI failed during refresh")
        if len(calls) >= expected_calls and not psutil.Process(process.pid).children():
            ready_at = ready_at or time.monotonic()
            if time.monotonic() - ready_at >= 10:
                return {"elapsed_s": time.monotonic() - start, "samples": samples,
                        "peak_tree_rss_bytes": max(s["tree_rss_bytes"] for s in samples),
                        "peak_tree_pss_bytes": max(s["tree_pss_bytes"] for s in samples)}
        else:
            ready_at = None
        time.sleep(0.05)
    raise RuntimeError("asynchronous refresh did not settle within 120s")


def run(args):
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    results = {
        "label": args.label, "binary": str(args.ui), "store": str(args.store),
        "binary_sha256": hashlib.sha256(args.ui.read_bytes()).hexdigest(),
        "fixture": str(args.fixture) if args.fixture else None,
        "platform": platform.platform(), "cpu_percent_convention": "100% is one fully occupied logical CPU",
        "warmup_after_cli_completion_s": 10, "iterations": [],
        "settle_refresh": args.settle_refresh,
    }
    for repetition in range(args.repetitions):
        trace = output.with_suffix(f".{repetition}.cli.jsonl")
        trace.write_text("")
        env = dict(os.environ, DISPLAY=":99", BEAMCRAFT_PERF_CLI=str(args.cli),
                   BEAMCRAFT_PERF_TRACE=str(trace))
        if args.fixture:
            env["BEAMCRAFT_PERF_FIXTURE"] = str(args.fixture)
        else:
            env.pop("BEAMCRAFT_PERF_FIXTURE", None)
        log = output.with_suffix(f".{repetition}.ui.log").open("wb")
        process = subprocess.Popen([str(args.ui), "--store", str(args.store),
                                    "--cli", str(args.wrapper), *args.extra], env=env, stdout=log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 120
            window = None
            ready_at = None
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(f"UI exited ({process.returncode}); see {log.name}")
                try:
                    window = subprocess.check_output(
                        ["xdotool", "search", "--onlyvisible", "--pid", str(process.pid)],
                        env=env, text=True, stderr=subprocess.DEVNULL).splitlines()[0]
                except (subprocess.CalledProcessError, IndexError):
                    time.sleep(0.1)
                    continue
                calls = [json.loads(line) for line in trace.read_text().splitlines() if line]
                if calls and all(c["code"] == 0 for c in calls) and not psutil.Process(process.pid).children():
                    ready_at = ready_at or time.monotonic()
                    if time.monotonic() - ready_at >= 10:
                        break
                else:
                    ready_at = None
                time.sleep(0.1)
            else:
                raise RuntimeError("UI did not finish its successful load within 120s")
            os.environ["DISPLAY"] = ":99"
            xdo("windowfocus", window)
            initial_calls = len([line for line in trace.read_text().splitlines() if line])
            iteration = {"repetition": repetition, "phases": []}
            for action in args.actions:
                iteration["phases"].append(phase(process, window, action, args.seconds))
                if action == "refresh" and args.settle_refresh:
                    iteration["refresh_settle"] = settle_refresh(process, trace, initial_calls * 2)
            iteration["cli_calls"] = [json.loads(line) for line in trace.read_text().splitlines() if line]
            results["iterations"].append(iteration)
            output.write_text(json.dumps(results, indent=2) + "\n")
            print(json.dumps({"label": args.label, "repetition": repetition,
                              "phases": [{k: v for k, v in p.items() if k != "samples"}
                                         for p in iteration["phases"]]}), flush=True)
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            log.close()
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--ui", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--store", type=Path, required=True)
    parser.add_argument("--wrapper", type=Path, default=Path("/measure/cli_wrapper.py"))
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--seconds", type=float, default=3.0)
    parser.add_argument("--settle-refresh", action="store_true")
    parser.add_argument("--actions", nargs="+", choices=["idle", "orbit", "refresh", "resize"],
                        default=["idle", "orbit", "idle", "refresh", "resize", "idle"])
    parser.add_argument("--extra", nargs=argparse.REMAINDER, default=[])
    args = parser.parse_args()
    if not 1 <= args.repetitions <= 20 or not 1 <= args.seconds <= 60 or len(args.actions) > 12:
        parser.error("use 1..20 repetitions, 1..60 seconds and at most 12 phases")
    run(args)
