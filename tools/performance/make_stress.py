#!/usr/bin/env python3
# Contributor-original synthetic closed polygon prisms, MPL-2.0.
"""Generate a BCMESH01/BCVIEW01 display stress fixture, not a native BIM project."""
import argparse
import hashlib
import json
import math
import struct
import uuid
from pathlib import Path


def pad8(data):
    return data + b"\0" * (-len(data) % 8)


def mesh(sides, radius, height):
    points = [
        (radius * math.cos(2 * math.pi * i / sides),
         radius * math.sin(2 * math.pi * i / sides), z)
        for z in (0.0, height) for i in range(sides)
    ]
    triangles, faces = [], []
    for i in range(sides):
        j = (i + 1) % sides
        triangles.extend([(i, j, j + sides), (i, j + sides, i + sides)])
        faces.extend([i, i])
    for i in range(1, sides - 1):
        triangles.extend([(0, i + 1, i), (sides, sides + i, sides + i + 1)])
        faces.extend([sides, sides + 1])
    header = b"BCMESH01" + struct.pack("<6I", 1, len(points), len(triangles), sides + 2, 0, 0)
    vertices = b"".join(struct.pack("<3d", *(0.0 if x == 0 else x for x in p)) for p in points)
    indices = pad8(b"".join(struct.pack("<3I", *t) for t in triangles))
    face_ids = pad8(b"".join(struct.pack("<H", f) for f in faces))
    return header + vertices + indices + face_ids, len(points), len(triangles)


def generate(output, count, sides, shared=False):
    if not 1 <= count <= 65536 or not 3 <= sides <= 1022:
        raise ValueError("fixture cap")
    triangle_count = 4 * sides - 4
    mesh_count = 1 if shared else count
    padded = lambda n: (n + 7) // 8 * 8
    blob_size = 32 + 48 * sides + padded(12 * triangle_count) + padded(2 * triangle_count)
    bundle_size = 88 + mesh_count * (4 + blob_size) + count * 49
    expanded = mesh_count * triangle_count * 72
    if (mesh_count * blob_size > 64 * 1024**2 or bundle_size > 70844504
            or count * triangle_count > 4194304 or expanded > 256 * 1024**2):
        raise ValueError("snapshot or display budget")
    output.mkdir(parents=True, exist_ok=True)
    blobs = [mesh(sides, 40 + i * 0.001, 10)[0] for i in range(1 if shared else count)]
    digests = [hashlib.sha256(blob).digest() for blob in blobs]
    revision = hashlib.sha256(b"BeamCraft original display-only prism stress v1").digest()
    generation = 1
    token = hashlib.sha256()
    token.update(b"BCVIEW-SNAPSHOT\0" + revision + struct.pack(">QQ", generation, count))
    records = []
    for i in range(count):
        identity = uuid.UUID(f"01990000-0000-7000-8000-{i:012x}").bytes
        placement = ((i % 32) * 120.0, (i // 32) * 120.0, 0.0)
        slot = 0 if shared else i
        token.update(identity + b"\0" + struct.pack(">3d", *placement) + digests[slot] * 2)
        records.append(identity + b"\0" + struct.pack("<3dII", *placement, slot, slot))
    bundle = (
        b"BCVIEW01" + revision + struct.pack("<Q", generation) + token.digest()
        + struct.pack("<II", count, len(blobs))
        + b"".join(struct.pack("<I", len(blob)) + blob for blob in blobs)
        + b"".join(records)
    )
    if len(bundle) > 70844504 or count * triangle_count > 4194304 or expanded > 256 * 1024**2:
        raise ValueError("snapshot or display budget")
    (output / "view.bin").write_bytes(bundle)
    definitions = {"outcome": "definitions", "definitions": []}
    (output / "definitions.json").write_text(json.dumps(definitions) + "\n")
    manifest = {
        "kind": "contributor-original display-only polygon-prism stress fixture",
        "not_native_store": True,
        "elements": count, "meshes": len(blobs), "sides": sides,
        "vertices_per_mesh": 2 * sides, "triangles_per_mesh": triangle_count,
        "projected_triangles": count * triangle_count,
        "raw_mesh_bytes": sum(map(len, blobs)), "expanded_vertex_bytes": expanded,
        "instance_bytes": count * 12, "bundle_bytes": len(bundle),
        "bundle_sha256": hashlib.sha256(bundle).hexdigest(),
        "snapshot_token": "vs1-" + token.hexdigest(),
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--sides", type=int, default=512)
    parser.add_argument("--shared", action="store_true")
    args = parser.parse_args()
    print(json.dumps(generate(args.output, args.count, args.sides, args.shared)))
