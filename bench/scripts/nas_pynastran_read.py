"""Time pyNastran BDF.read_bdf. Prints: <median_ms> <nodes> <elements>

Import and BDF() setup are outside the timer. Usage:

    python nas_pynastran_read.py <path.nas> [runs]
"""

from __future__ import annotations

import sys
import time


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: nas_pynastran_read.py <path> [runs]", file=sys.stderr)
        return 2
    path = sys.argv[1]
    runs = int(sys.argv[2]) if len(sys.argv) > 2 else 5
    try:
        from pyNastran.bdf.bdf import BDF
    except ImportError as e:
        print(f"pyNastran missing: {e}", file=sys.stderr)
        return 1

    def once() -> tuple[float, int, int]:
        model = BDF(debug=False)
        t = time.perf_counter()
        model.read_bdf(path, punch=True)
        ms = (time.perf_counter() - t) * 1000.0
        return ms, len(model.nodes), len(model.elements)

    once()
    samples = []
    nodes = 0
    elems = 0
    for _ in range(runs):
        ms, nodes, elems = once()
        samples.append(ms)
    samples.sort()
    print(f"{samples[len(samples) // 2]:.4f} {nodes} {elems}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
