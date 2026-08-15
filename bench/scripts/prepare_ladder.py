#!/usr/bin/env python3
"""Generate the STL ladder (stdlib) and NAS ladder (pyNastran)."""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mesh_io import read_ply, subdivide, write_stl_ascii, write_stl_binary
from nas_pynastran import write_hex, write_shell, write_tet

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "data"
DERIVED = DATA / "derived"
SIZES_FORMAT = (8, 16)


def _mb(path: Path) -> float:
    return path.stat().st_size / (1024 * 1024)


def _need(path: Path) -> Path:
    if not path.is_file():
        raise FileNotFoundError(f"missing source: {path}")
    return path


def _wipe_nas() -> None:
    for name in ("nas_surface", "nas_volume"):
        d = DERIVED / name
        if d.is_dir():
            shutil.rmtree(d)
        d.mkdir(parents=True)


ASCII_TAGS = frozenset({"bunny_res3", "bunny", "happy_res2", "happy"})


def emit_stl(tag: str, ply: Path, *, subdiv: int = 0, binary: bool = True, ascii: bool = False):
    print(f"read {ply} ...")
    mesh = read_ply(ply)
    if subdiv:
        print(f"  subdivide x{subdiv} ({mesh.nfaces} faces) ...")
        mesh = subdivide(mesh, subdiv)
    print(f"  {mesh.nverts} verts, {mesh.nfaces} faces")
    if binary:
        stl = DERIVED / "stl" / f"{tag}.stl"
        ntri = write_stl_binary(stl, mesh)
        print(f"  wrote {stl}  {ntri} tri  {_mb(stl):.2f} MB")
    if ascii:
        astl = DERIVED / "stl_ascii" / f"{tag}.stl"
        ntri = write_stl_ascii(astl, mesh)
        print(f"  wrote {astl}  {ntri} tri  {_mb(astl):.2f} MB")
    return mesh


def emit_shell(tag: str, mesh, sizes: tuple[int, ...]) -> None:
    for size in sizes:
        path = DERIVED / "nas_surface" / f"{tag}_small{size}.nas" if size == 8 else DERIVED / "nas_surface" / f"{tag}_large{size}.nas"
        print(f"  pyNastran shell size={size} -> {path.name} ...")
        write_shell(mesh, path, size=size)
        print(f"  wrote {path}  {_mb(path):.2f} MB")


def emit_volume(kind: str, cells: int, sizes: tuple[int, ...]) -> None:
    for size in sizes:
        tag = f"box_{kind}_c{cells}_small{size}" if size == 8 else f"box_{kind}_c{cells}_large{size}"
        path = DERIVED / "nas_volume" / f"{tag}.nas"
        print(f"volume {kind} c{cells} size={size} -> {path.name} ...")
        if kind == "hex":
            nnode, nelem = write_hex(cells, path, size=size)
        else:
            nnode, nelem = write_tet(cells, path, size=size)
        print(f"  {nnode} GRID, {nelem} elem, {_mb(path):.2f} MB")


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--tier", choices=("core", "all"), default="core")
    p.add_argument("--stl-only", action="store_true")
    p.add_argument("--ascii-only", action="store_true")
    p.add_argument("--nas-only", action="store_true")
    args = p.parse_args()
    if sum((args.stl_only, args.ascii_only, args.nas_only)) > 1:
        p.error("pick one of --stl-only / --ascii-only / --nas-only")

    DERIVED.mkdir(parents=True, exist_ok=True)
    (DERIVED / "stl").mkdir(exist_ok=True)
    (DERIVED / "stl_ascii").mkdir(exist_ok=True)

    bunny = _need(DATA / "bunny" / "bun_zipper.ply")
    bunny_s = _need(DATA / "bunny" / "bun_zipper_res3.ply")
    happy = _need(DATA / "happy_recon" / "happy_vrip.ply")
    happy2 = _need(DATA / "happy_recon" / "happy_vrip_res2.ply")

    do_stl = not args.nas_only and not args.ascii_only
    do_ascii = not args.nas_only
    do_nas = not args.stl_only and not args.ascii_only

    meshes = {}
    if do_stl or do_ascii or do_nas:
        for tag, src, subdiv in (
            ("bunny_res3", bunny_s, 0),
            ("bunny", bunny, 0),
            ("happy_res2", happy2, 0),
            ("happy", happy, 0),
            ("happy_subdiv1", happy, 1),
        ):
            want_ascii = do_ascii and tag in ASCII_TAGS
            if do_stl or want_ascii:
                if tag == "happy_subdiv1" and not do_stl:
                    continue
                meshes[tag] = emit_stl(
                    tag, src, subdiv=subdiv, binary=do_stl, ascii=want_ascii
                )
            elif do_nas and tag != "happy_subdiv1":
                print(f"read {src} for NAS ...")
                mesh = read_ply(src)
                meshes[tag] = mesh

    if do_stl and args.tier == "all":
        lucy = _need(DATA / "lucy" / "lucy.ply")
        emit_stl("lucy", lucy)

    if do_nas:
        _wipe_nas()
        emit_shell("bunny_res3", meshes["bunny_res3"], SIZES_FORMAT)
        emit_shell("bunny", meshes["bunny"], SIZES_FORMAT)
        emit_shell("happy_res2", meshes["happy_res2"], (8,))
        emit_shell("happy", meshes["happy"], (8,))
        emit_volume("hex", 20, SIZES_FORMAT)
        emit_volume("tet", 20, (8,))
        emit_volume("hex", 40, (8,))
        if args.tier == "all":
            emit_volume("tet", 50, (8,))

    print("done")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
