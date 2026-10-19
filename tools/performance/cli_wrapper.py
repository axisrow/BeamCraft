#!/usr/bin/env python3
# Contributor-original performance harness, MPL-2.0.
"""Trace subprocess work or serve an explicitly labelled display-only fixture."""
import json
import os
import resource
import subprocess
import sys
import time
from pathlib import Path

args = sys.argv[1:]
started = time.monotonic()
fixture = os.environ.get("BEAMCRAFT_PERF_FIXTURE")
if fixture:
    root = Path(fixture)
    def read_capped(path, cap):
        with path.open("rb") as file:
            data = file.read(cap + 1)
        if not data or len(data) > cap:
            raise ValueError("display-fixture payload exceeds its cap")
        return data

    if args and args[0] == "view-export":
        snapshot = read_capped(root / "view.bin", 70844504)
        if "--include-definitions" in args:
            metadata = read_capped(root / "definitions.json", 1024**2)
            # Match the documented opt-in refresh envelope; the UI still validates its contents.
            import struct
            output = b"BCREFR01" + struct.pack("<QQ", len(snapshot), len(metadata)) + snapshot + metadata
        else:
            output = snapshot
        sys.stdout.buffer.write(output)
        code = 0
    elif args and args[0] == "definitions":
        sys.stdout.buffer.write(read_capped(root / "definitions.json", 1024**2))
        code = 0
    else:
        print(json.dumps({"outcome": "rejected", "code": "INVALID_ARGUMENT"}))
        code = 2
else:
    code = subprocess.call([os.environ["BEAMCRAFT_PERF_CLI"], *args])
usage = resource.getrusage(resource.RUSAGE_CHILDREN)
record = {
    "args": args, "pid": os.getpid(), "code": code,
    "started_monotonic": started, "finished_monotonic": time.monotonic(),
    "wall_s": time.monotonic() - started,
    "children_user_s": usage.ru_utime, "children_system_s": usage.ru_stime,
    "children_peak_rss_kib": usage.ru_maxrss,
}
trace = os.environ.get("BEAMCRAFT_PERF_TRACE")
if trace:
    data = (json.dumps(record) + "\n").encode()
    fd = os.open(trace, os.O_APPEND | os.O_CREAT | os.O_WRONLY, 0o600)
    try:
        os.write(fd, data)
    finally:
        os.close(fd)
raise SystemExit(code)
