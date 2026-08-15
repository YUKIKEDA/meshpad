"""Minimal PLY reader and STL writer. Stdlib only."""

from __future__ import annotations

import struct
from array import array
from pathlib import Path

_UCHAR_INT_FACE = struct.Struct("<Biii")
_UCHAR_INT_FACE_BE = struct.Struct(">Biii")
_F3_LE = struct.Struct("<fff")
_F3_BE = struct.Struct(">fff")
_STL_TRI = struct.Struct("<12fH")


class Mesh:
    __slots__ = ("indices", "xs", "ys", "zs")

    def __init__(self, xs: array, ys: array, zs: array, indices: array):
        self.xs = xs
        self.ys = ys
        self.zs = zs
        self.indices = indices  # flat triples, typecode "I" or "L"

    @property
    def nverts(self) -> int:
        return len(self.xs)

    @property
    def nfaces(self) -> int:
        return len(self.indices) // 3


def read_ply(path: Path) -> Mesh:
    with path.open("rb") as f:
        header_lines: list[str] = []
        while True:
            line = f.readline()
            if not line:
                raise ValueError(f"{path}: no PLY end_header")
            text = line.decode("ascii", errors="replace").strip()
            header_lines.append(text)
            if text == "end_header":
                break
        meta = _parse_ply_header(header_lines)
        if meta["format"] == "ascii":
            rest = f.read().decode("ascii", errors="replace")
            return _read_ply_ascii(rest, meta)
        return _read_ply_binary(f, meta)


def _parse_ply_header(lines: list[str]) -> dict:
    if not lines or lines[0] != "ply":
        raise ValueError("not a PLY file")
    fmt = None
    verts = faces = None
    for line in lines[1:]:
        if line.startswith(("comment", "obj_info")):
            continue
        if line.startswith("format "):
            fmt = line.split()[1]
            continue
        if line.startswith("element vertex "):
            verts = int(line.split()[-1])
            continue
        if line.startswith("element face "):
            faces = int(line.split()[-1])
            continue
    if fmt not in {"ascii", "binary_little_endian", "binary_big_endian"}:
        raise ValueError(f"unsupported PLY format: {fmt}")
    if verts is None or faces is None:
        raise ValueError("PLY header missing vertex/face counts")
    names: list[str] = []
    types: list[str] = []
    in_v = False
    for line in lines:
        if line.startswith("element vertex "):
            in_v = True
            names, types = [], []
            continue
        if in_v and line.startswith("element "):
            break
        if in_v and line.startswith("property "):
            parts = line.split()
            types.append(parts[1])
            names.append(parts[-1])
    if names[:3] != ["x", "y", "z"]:
        raise ValueError(f"expected first vertex props x,y,z; got {names[:3]}")
    vert_prop_types = types
    return {
        "format": fmt,
        "nverts": verts,
        "nfaces": faces,
        "vert_prop_types": vert_prop_types,
    }


def _vert_prop_size(types: list[str]) -> int:
    sizes = {
        "char": 1,
        "int8": 1,
        "uchar": 1,
        "uint8": 1,
        "short": 2,
        "int16": 2,
        "ushort": 2,
        "uint16": 2,
        "int": 4,
        "int32": 4,
        "uint": 4,
        "uint32": 4,
        "float": 4,
        "float32": 4,
        "double": 8,
        "float64": 8,
    }
    try:
        return sum(sizes[t] for t in types)
    except KeyError as e:
        raise ValueError(f"unsupported PLY vertex property {e}") from e


def _read_ply_ascii(body: str, meta: dict) -> Mesh:
    nverts = meta["nverts"]
    nfaces = meta["nfaces"]
    xs, ys, zs = array("f"), array("f"), array("f")
    indices = array("I")
    tokens = body.split()
    nprop = len(meta["vert_prop_types"])
    i = 0
    for _ in range(nverts):
        nums = [float(tokens[i + k]) for k in range(nprop)]
        i += nprop
        xs.append(nums[0])
        ys.append(nums[1])
        zs.append(nums[2])
    for _ in range(nfaces):
        n = int(tokens[i])
        i += 1
        if n < 3:
            i += n
            continue
        corners = [int(tokens[i + k]) for k in range(n)]
        i += n
        for k in range(1, n - 1):
            indices.append(corners[0])
            indices.append(corners[k])
            indices.append(corners[k + 1])
    return Mesh(xs, ys, zs, indices)


def _read_ply_binary(f, meta: dict) -> Mesh:
    be = meta["format"] == "binary_big_endian"
    nverts = meta["nverts"]
    nfaces = meta["nfaces"]
    stride = _vert_prop_size(meta["vert_prop_types"])
    f3 = _F3_BE if be else _F3_LE
    extra = stride - 12
    xs, ys, zs = array("f"), array("f"), array("f")
    buf = f.read(nverts * stride)
    if len(buf) != nverts * stride:
        raise ValueError("truncated PLY vertex data")
    off = 0
    for _ in range(nverts):
        x, y, z = f3.unpack_from(buf, off)
        xs.append(x)
        ys.append(y)
        zs.append(z)
        off += stride
        if extra:
            pass  # already included in stride
    face_st = _UCHAR_INT_FACE_BE if be else _UCHAR_INT_FACE
    indices = array("I")
    # Fast path: all triangles packed as uchar + 3 int
    packed = nfaces * 13
    rest = f.read()
    if len(rest) == packed:
        for i in range(nfaces):
            n, a, b, c = face_st.unpack_from(rest, i * 13)
            if n != 3:
                raise ValueError(f"expected triangle, got n={n}")
            indices.append(a)
            indices.append(b)
            indices.append(c)
        return Mesh(xs, ys, zs, indices)
    # Slow path: mixed polygons
    pos = 0
    unpack_n = struct.Struct(">B" if be else "<B")
    unpack_i = struct.Struct(">i" if be else "<i")
    for _ in range(nfaces):
        (n,) = unpack_n.unpack_from(rest, pos)
        pos += 1
        corners: list[int] = []
        for _k in range(n):
            (idx,) = unpack_i.unpack_from(rest, pos)
            pos += 4
            corners.append(idx)
        if n < 3:
            continue
        for k in range(1, n - 1):
            indices.append(corners[0])
            indices.append(corners[k])
            indices.append(corners[k + 1])
    return Mesh(xs, ys, zs, indices)


def subdivide(mesh: Mesh, times: int) -> Mesh:
    out = mesh
    for _ in range(times):
        out = _subdivide_once(out)
    return out


def _subdivide_once(mesh: Mesh) -> Mesh:
    xs, ys, zs = array("f", mesh.xs), array("f", mesh.ys), array("f", mesh.zs)
    cache: dict[tuple[int, int], int] = {}
    new_idx = array("I")

    def midpoint(a: int, b: int) -> int:
        key = (a, b) if a < b else (b, a)
        hit = cache.get(key)
        if hit is not None:
            return hit
        i = len(xs)
        xs.append((mesh.xs[a] + mesh.xs[b]) * 0.5)
        ys.append((mesh.ys[a] + mesh.ys[b]) * 0.5)
        zs.append((mesh.zs[a] + mesh.zs[b]) * 0.5)
        cache[key] = i
        return i

    it = mesh.indices
    for i in range(0, len(it), 3):
        a, b, c = it[i], it[i + 1], it[i + 2]
        ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
        new_idx.extend((a, ab, ca, b, bc, ab, c, ca, bc, ab, bc, ca))
    return Mesh(xs, ys, zs, new_idx)


def _copies_and_pitch(
    mesh: Mesh,
    copies: tuple[int, int, int],
    pitch: tuple[float, float, float] | None,
) -> tuple[int, int, int, int, tuple[float, float, float]]:
    nx, ny, nz = copies
    ntri = mesh.nfaces * nx * ny * nz
    if pitch is None:
        span = (
            (max(mesh.xs) - min(mesh.xs)) if mesh.nverts else 1.0,
            (max(mesh.ys) - min(mesh.ys)) if mesh.nverts else 1.0,
            (max(mesh.zs) - min(mesh.zs)) if mesh.nverts else 1.0,
        )
        pitch = (span[0] * 1.05 or 1.0, span[1] * 1.05 or 1.0, span[2] * 1.05 or 1.0)
    return nx, ny, nz, ntri, pitch


def write_stl_binary(
    path: Path,
    mesh: Mesh,
    *,
    copies: tuple[int, int, int] = (1, 1, 1),
    pitch: tuple[float, float, float] | None = None,
) -> int:
    nx, ny, nz, ntri, pitch = _copies_and_pitch(mesh, copies, pitch)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("wb") as f:
        header = b"meshpad bench stl".ljust(80, b"\0")
        f.write(header)
        f.write(struct.pack("<I", ntri))
        it = mesh.indices
        px, py, pz = pitch
        for iz in range(nz):
            for iy in range(ny):
                for ix in range(nx):
                    dx, dy, dz = ix * px, iy * py, iz * pz
                    for i in range(0, len(it), 3):
                        a, b, c = it[i], it[i + 1], it[i + 2]
                        f.write(
                            _STL_TRI.pack(
                                0.0,
                                0.0,
                                0.0,
                                mesh.xs[a] + dx,
                                mesh.ys[a] + dy,
                                mesh.zs[a] + dz,
                                mesh.xs[b] + dx,
                                mesh.ys[b] + dy,
                                mesh.zs[b] + dz,
                                mesh.xs[c] + dx,
                                mesh.ys[c] + dy,
                                mesh.zs[c] + dz,
                                0,
                            )
                        )
    return ntri


def write_stl_ascii(
    path: Path,
    mesh: Mesh,
    *,
    copies: tuple[int, int, int] = (1, 1, 1),
    pitch: tuple[float, float, float] | None = None,
) -> int:
    nx, ny, nz, ntri, pitch = _copies_and_pitch(mesh, copies, pitch)
    path.parent.mkdir(parents=True, exist_ok=True)
    name = path.stem.encode("ascii", "replace").decode("ascii") or "mesh"
    it = mesh.indices
    px, py, pz = pitch
    buf: list[str] = []
    with path.open("w", encoding="ascii", newline="\n") as f:
        f.write(f"solid {name}\n")
        for iz in range(nz):
            for iy in range(ny):
                for ix in range(nx):
                    dx, dy, dz = ix * px, iy * py, iz * pz
                    for i in range(0, len(it), 3):
                        a, b, c = it[i], it[i + 1], it[i + 2]
                        buf.append("  facet normal 0 0 0\n")
                        buf.append("    outer loop\n")
                        for idx in (a, b, c):
                            buf.append(
                                f"      vertex {mesh.xs[idx] + dx:.8g} {mesh.ys[idx] + dy:.8g} {mesh.zs[idx] + dz:.8g}\n"
                            )
                        buf.append("    endloop\n")
                        buf.append("  endfacet\n")
                        if len(buf) >= 4096:
                            f.writelines(buf)
                            buf.clear()
        if buf:
            f.writelines(buf)
        f.write(f"endsolid {name}\n")
    return ntri
