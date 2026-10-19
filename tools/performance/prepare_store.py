#!/usr/bin/env python3
# Contributor-original synthetic model through the public CLI, MPL-2.0.
import argparse
import hashlib
import json
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--cli", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--count", type=int, default=100)
args = parser.parse_args()
if not 1 <= args.count <= 1000:
    parser.error("--count must be 1..1000 for this development fixture")
root = args.output.resolve()
root.mkdir(parents=True)
log = []


def run(*parts):
    command = [str(args.cli), *map(str, parts), "--store", str(root)]
    reply = subprocess.run(command, capture_output=True, check=False, timeout=120)
    if reply.returncode:
        raise RuntimeError(f"{parts}: {reply.stdout!r} {reply.stderr!r}")
    result = json.loads(reply.stdout)
    log.append({"arguments": list(map(str, parts)), "outcome": result.get("outcome")})
    return result


for i in range(32):
    run("define-material", "--density", 7850 + i, "--grade", f"PerfMaterial{i:02}",
        "--name", f"PerfMaterial{i:02}")
    run("define-profile", "--width", 100 + i, "--height", 200 + i,
        "--designation", f"PerfProfile{i:02}", "--name", f"PerfProfile{i:02}")
plate = "01990000-0000-7000-8000-000000000000"
run("create-plate", "--width", 1000, "--length", 500, "--thickness", 10,
    "--new-id", plate, "--material", "PerfMaterial00")
for i in range(1, args.count):
    run("copy-element", "--id", plate,
        "--new-id", f"01990000-0000-7000-8000-{i:012x}",
        "--offset", f"{i % 10 * 1200},{i // 10 * 700},0")
bundle = subprocess.check_output([str(args.cli), "view-export", "--store", str(root), "--out", "-"])
manifest = {
    "kind": "contributor-original native store created through beamcraft CLI",
    "elements": args.count, "materials": 32, "profiles": 32,
    "bundle_sha256": hashlib.sha256(bundle).hexdigest(),
    "bundle_bytes": len(bundle), "commands": log,
}
(root.parent / (root.name + ".manifest.json")).write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps({k: v for k, v in manifest.items() if k != "commands"}))
