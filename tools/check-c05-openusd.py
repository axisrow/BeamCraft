# SPDX-License-Identifier: MPL-2.0
# SPDX-FileCopyrightText: 2026 Apibim
"""Additional C05 acceptance using an installed official OpenUSD SDK, without BeamCraft imports."""
import json
import math
import sys
from pathlib import Path

from pxr import Usd, UsdGeom, UsdUtils


def check(directory):
    expected = json.loads((directory / "expected.json").read_text())
    assert len(expected) == 2001
    assert len({entry["id"] for entry in expected}) == 2001
    stage = Usd.Stage.Open(str(directory / "root.usda"))
    assert stage
    assert UsdGeom.GetStageUpAxis(stage) == "Z"
    assert UsdGeom.GetStageMetersPerUnit(stage) == 0.001
    parts = {
        str(prim.GetAttribute("bim:id").Get()): prim
        for prim in stage.Traverse()
        if prim.GetAttribute("bim:elementKind").Get() == "plate"
    }
    assert len(parts) == 2001
    paths = set()
    for entry in expected:
        prim = parts[entry["id"]]
        assert str(prim.GetPath()) not in paths
        paths.add(str(prim.GetPath()))
        assert prim.GetAttribute("bim:name").Get() == entry["name"]
        assert list(prim.GetAttribute("xformOp:translate").Get()) == entry["translation"]
        assert prim.GetAttribute("steel:plate:thicknessMm").Get() == entry["thickness"]
        meshes = [UsdGeom.Mesh(p) for p in Usd.PrimRange(prim) if p.IsA(UsdGeom.Mesh)]
        assert len(meshes) == 1
        mesh = meshes[0]
        points = mesh.GetPointsAttr().Get()
        counts = mesh.GetFaceVertexCountsAttr().Get()
        indices = mesh.GetFaceVertexIndicesAttr().Get()
        offset, volume = 0, 0.0
        for count in counts:
            face = indices[offset:offset + count]
            assert len(face) == count and count >= 3
            a = points[face[0]]
            for n in range(1, count - 1):
                b, c = points[face[n]], points[face[n + 1]]
                volume += (
                    a[0] * (b[1] * c[2] - b[2] * c[1])
                    + a[1] * (b[2] * c[0] - b[0] * c[2])
                    + a[2] * (b[0] * c[1] - b[1] * c[0])
                ) / 6.0
            offset += count
        assert offset == len(indices)
        assert math.isclose(abs(volume), entry["cli_volume_mm3"], rel_tol=1e-6)
        assert mesh.GetPrim().GetAttribute("bim:volumeMm3").Get() == 600.0
    checker = UsdUtils.ComplianceChecker(arkit=False, skipARKitRootLayerCheck=True)
    checker.CheckCompliance(str(directory / "root.usda"))
    assert not checker.GetErrors(), checker.GetErrors()
    assert not checker.GetFailedChecks(), checker.GetFailedChecks()
    assert not checker.GetWarnings(), checker.GetWarnings()
    print(json.dumps({"reader": "OpenUSD", "version": Usd.GetVersion(), "parts": len(parts), "volume_each_mm3": 600, "compliance": "passed"}))


if __name__ == "__main__":
    check(Path(sys.argv[1]))
