# SPDX-License-Identifier: MPL-2.0
"""Prepare an original synthetic model and launch the native UI for a visual repro."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("case", choices=["selection", "publish", "clipping"])
parser.add_argument("--bin-dir", type=Path, default=Path(__file__).resolve().parents[2] / "target" / "debug")
parser.add_argument("--prepare-only", action="store_true")
args = parser.parse_args()
root = Path(tempfile.mkdtemp(prefix="beamcraft-visual-repro-"))
store = root / "published"
store.mkdir()
cli = args.bin_dir.resolve() / "beamcraft"
ui = args.bin_dir.resolve() / "beamcraft-ui"

def command(*values):
    reply = subprocess.run([str(cli), *map(str, values)], capture_output=True, text=True, check=True)
    return json.loads(reply.stdout)

command("define-profile", "--store", store, "--width", "80", "--height", "160", "--designation", "RECT-80x160")
if args.case in {"selection", "publish"}:
    command("define-profile", "--store", store, "--family", "i_shape", "--h", "200", "--b", "100", "--tw", "6", "--tf", "10", "--r", "0", "--designation", "SYNTHETIC-I200")
if args.case == "selection":
    command("create-member", "--store", store, "--start", "0,0,0", "--end", "1000,0,0", "--profile", "RECT-80x160", "--name", "SelectableRectangle")
    command("create-member", "--store", store, "--start", "0,400,0", "--end", "1000,400,0", "--profile", "SYNTHETIC-I200", "--name", "IBeam")
    command("create-plate", "--store", store, "--width", "300", "--length", "300", "--thickness", "10", "--hole-round", "150,150,100", "--translation", "300,800,0", "--name", "PlateWithHole")
if args.case == "clipping":
    command("create-member", "--store", store, "--start", "12345.678,23456.789,34567.891", "--end", "12645.678,23756.789,34867.891", "--profile", "RECT-80x160", "--name", "LongCoordinates")
workspace = root / "workspace"
command("workspace-create", "--store", store, "--workspace-dir", workspace)
launch = [str(ui), "--workspace", str(workspace), "--published", str(store)]
print("Synthetic repro directory:", root, flush=True)
print("UI argument vector:", json.dumps(launch), flush=True)
if args.case == "publish":
    print("Create > Member: Start 0,400,0; End 1000,400,0; Profile SYNTHETIC-I200; Name IBeam. Preview > Commit; scroll to Publish.", flush=True)
if not args.prepare_only:
    subprocess.run(launch, check=True)
