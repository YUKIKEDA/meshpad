#!/usr/bin/env python3
"""Synthetic unit-cube volume NAS via pyNastran (CHEXA or CTETRA)."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from nas_pynastran import write_hex, write_tet


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--cells", type=int, required=True, help="cells along each axis")
    p.add_argument("--elem", choices=("hex", "tet"), default="hex")
    p.add_argument("--size", type=int, choices=(8, 16), default=8)
    p.add_argument("--out", required=True)
    args = p.parse_args()
    out = Path(args.out)
    if args.elem == "hex":
        nnode, nelem = write_hex(args.cells, out, size=args.size)
    else:
        nnode, nelem = write_tet(args.cells, out, size=args.size)
    size_mb = out.stat().st_size / (1024 * 1024)
    print(f"{nnode} GRID, {nelem} {args.elem}, {size_mb:.2f} MB -> {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
