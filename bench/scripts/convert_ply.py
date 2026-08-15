#!/usr/bin/env python3
"""Convert one PLY mesh to binary STL and/or pyNastran bulk NAS."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mesh_io import read_ply, subdivide, write_stl_binary
from nas_pynastran import write_shell


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("ply")
    p.add_argument("--stl")
    p.add_argument("--nas")
    p.add_argument("--nas-size", type=int, choices=(8, 16), default=8)
    p.add_argument("--subdiv", type=int, default=0)
    p.add_argument("--tile", default="1,1,1", help="Nx,Ny,Nz copies (STL only)")
    args = p.parse_args()
    if not args.stl and not args.nas:
        p.error("specify --stl and/or --nas")
    mesh = read_ply(Path(args.ply))
    if args.subdiv:
        mesh = subdivide(mesh, args.subdiv)
    print(f"{mesh.nverts} verts, {mesh.nfaces} faces")
    if args.stl:
        nx, ny, nz = (int(x) for x in args.tile.split(","))
        n = write_stl_binary(Path(args.stl), mesh, copies=(nx, ny, nz))
        print(f"STL {n} triangles -> {args.stl}")
    if args.nas:
        if args.tile != "1,1,1":
            p.error("--tile is STL-only")
        write_shell(mesh, Path(args.nas), size=args.nas_size)
        print(f"NAS size={args.nas_size} -> {args.nas}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
