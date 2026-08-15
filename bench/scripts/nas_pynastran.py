"""Write Nastran bulk via pyNastran (independent of Meshpad's parser)."""

from __future__ import annotations

from pathlib import Path

from mesh_io import Mesh


def _bdf():
    try:
        from pyNastran.bdf.bdf import BDF
    except ImportError as e:
        raise SystemExit(
            "pyNastran is required. Use bench/.venv:\n"
            r"  bench\.venv\Scripts\python -m pip install -r bench/requirements.txt"
        ) from e
    model = BDF(debug=False)
    model.add_mat1(1, 2.1e11, None, 0.3)
    return model


def write_bdf(model, path: Path, *, size: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    kwargs: dict = {
        "size": size,
        "is_double": False,
        "enddata": True,
        "write_header": True,
        "close": True,
    }
    if size == 16:
        kwargs["nodes_size"] = 16
        kwargs["elements_size"] = 16
    model.write_bdf(str(path), **kwargs)


def write_shell(mesh: Mesh, path: Path, *, size: int) -> None:
    model = _bdf()
    model.add_pshell(1, 1, 0.001)
    xs, ys, zs = mesh.xs, mesh.ys, mesh.zs
    for i in range(mesh.nverts):
        model.add_grid(i + 1, [float(xs[i]), float(ys[i]), float(zs[i])])
    it = mesh.indices
    eid = 1
    for i in range(0, len(it), 3):
        model.add_ctria3(
            eid, 1, [int(it[i]) + 1, int(it[i + 1]) + 1, int(it[i + 2]) + 1]
        )
        eid += 1
    write_bdf(model, path, size=size)


def _hex_nodes(cells: int):
    n = cells
    def nid(i: int, j: int, k: int) -> int:
        return 1 + i + (n + 1) * (j + (n + 1) * k)

    nnode = (n + 1) ** 3
    step = 1.0 / n if n else 1.0
    xyz: list[tuple[int, list[float]]] = []
    for k in range(n + 1):
        z = k * step
        for j in range(n + 1):
            y = j * step
            for i in range(n + 1):
                xyz.append((nid(i, j, k), [i * step, y, z]))
    hexes: list[tuple[int, list[int]]] = []
    eid = 1
    for k in range(n):
        for j in range(n):
            for i in range(n):
                hexes.append(
                    (
                        eid,
                        [
                            nid(i, j, k),
                            nid(i + 1, j, k),
                            nid(i + 1, j + 1, k),
                            nid(i, j + 1, k),
                            nid(i, j, k + 1),
                            nid(i + 1, j, k + 1),
                            nid(i + 1, j + 1, k + 1),
                            nid(i, j + 1, k + 1),
                        ],
                    )
                )
                eid += 1
    return nnode, xyz, hexes


def write_hex(cells: int, path: Path, *, size: int) -> tuple[int, int]:
    nnode, xyz, hexes = _hex_nodes(cells)
    model = _bdf()
    model.add_psolid(1, 1)
    for nid, pos in xyz:
        model.add_grid(nid, pos)
    for eid, nids in hexes:
        model.add_chexa(eid, 1, nids)
    write_bdf(model, path, size=size)
    return nnode, len(hexes)


def write_tet(cells: int, path: Path, *, size: int) -> tuple[int, int]:
    nnode, xyz, hexes = _hex_nodes(cells)
    tets = (
        (0, 1, 2, 6),
        (0, 2, 3, 6),
        (0, 1, 5, 6),
        (0, 5, 4, 6),
        (0, 3, 7, 6),
        (0, 7, 4, 6),
    )
    model = _bdf()
    model.add_psolid(1, 1)
    for nid, pos in xyz:
        model.add_grid(nid, pos)
    eid = 1
    for _hid, nids in hexes:
        for a, b, c, d in tets:
            model.add_ctetra(eid, 1, [nids[a], nids[b], nids[c], nids[d]])
            eid += 1
    write_bdf(model, path, size=size)
    return nnode, eid - 1
