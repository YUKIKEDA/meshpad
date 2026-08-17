//! Nastran bulk（`.nas` / `.nastran`）から外皮の三角形スープを作る。
//!
//! 対象カードは `GRID` / `CTRIA3` / `CQUAD4` / `CTETRA` / `CHEXA`。`cp ≠ 0` の節点は捨てて件数を警告する。
//! `CORD*` は読まない。未知カードは飛ばして種類と件数を出す。体積の内部面は出さない。
//! フィールドは mmap 上のスライスから数値化する。シェル要素は面の重複カウントをしない。
//! カード列挙の中間ベクタは持たず、走査中に GRID 表と要素節点だけ残す。
//! 表は Fx ハッシュ。体積面の巻きは置換番号、初回挿入で容量を予約する。
//!
//! 1.0 は点群を出さず、外皮まで読んでから載せる。

use std::path::Path;

use anyhow::{bail, Context, Result};
use glam::Vec3;
use memmap2::Mmap;
use rustc_hash::FxHashMap;

use crate::mesh::{bounds_to_soup, LoadProbe, ParsedMesh, TriangleSoup};

const MAX_FIELDS: usize = 16;
const PROG_STEP: usize = 1 << 20;

type Map<K, V> = FxHashMap<K, V>;

fn map_with_cap<K, V>(cap: usize) -> Map<K, V> {
    Map::with_capacity_and_hasher(cap, Default::default())
}

/// 1 ファイルを mmap して NAS 外皮へ展開する。
///
/// 警告（未知カード、`cp ≠ 0`、欠ける GRID）はファイルを捨てずに返す。
///
/// # Errors
///
/// 開けない、マップできない、または外皮三角形が 1 枚も無いとき。
pub(crate) fn load_nas_at(
    path: &Path,
    probe: Option<&LoadProbe>,
    base: u64,
) -> Result<(ParsedMesh, Vec<String>)> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mmap = unsafe { Mmap::map(&file) }.with_context(|| format!("mmap {}", path.display()))?;
    parse_nas_mesh(&mmap, probe, base)
}

/// バイト列を bulk として解釈し、外皮の三角形スープを返す。
///
/// `BEGIN BULK` は無くてよい。継続行は先頭 8 桁が空、`+`、`*`。
/// 位置はファイル座標のまま（中心化しない）。
///
/// # Errors
///
/// 外皮三角形が 1 枚も無いとき。診断（`cp != 0`、未知カードなど）があればエラー文に含める。
///
/// # Examples
///
/// ```ignore
/// let (soup, warnings) = meshpad::nas::parse_nas(bytes)?;
/// let _ = (soup.triangle_count(), warnings.len());
/// ```
pub fn parse_nas(bytes: &[u8]) -> Result<(TriangleSoup, Vec<String>)> {
    let (mesh, warnings) = parse_nas_mesh(bytes, None, 0)?;
    Ok((bounds_to_soup(mesh.positions, mesh.min, mesh.max), warnings))
}

fn parse_nas_mesh(
    bytes: &[u8],
    probe: Option<&LoadProbe>,
    base: u64,
) -> Result<(ParsedMesh, Vec<String>)> {
    finish(scan(bytes, probe, base), probe)
}

struct Card<'a> {
    name: &'a [u8],
    wide: bool,
    n: usize,
    fields: [&'a [u8]; MAX_FIELDS],
}

impl<'a> Card<'a> {
    fn new() -> Self {
        Self {
            name: b"",
            wide: false,
            n: 0,
            fields: [&b""[..]; MAX_FIELDS],
        }
    }

    fn clear(&mut self) {
        *self = Self::new();
    }

    fn push(&mut self, field: &'a [u8]) {
        if self.n < MAX_FIELDS {
            self.fields[self.n] = field;
            self.n += 1;
        }
    }

    fn get(&self, i: usize) -> &'a [u8] {
        if i < self.n {
            self.fields[i]
        } else {
            b""
        }
    }
}

struct Acc {
    est_faces: usize,
    reserved_tri: bool,
    reserved_quad: bool,
    grids: Map<u32, [f32; 3]>,
    shells: Vec<[u32; 3]>,
    tri_faces: Map<[u32; 3], FaceHit>,
    quad_faces: Map<[u32; 4], FaceHit>,
    skipped_cp: u32,
    unknown: Map<String, u32>,
}

fn scan(bytes: &[u8], probe: Option<&LoadProbe>, base: u64) -> Acc {
    let est = bytes.len() / 48 + 8;
    let mut acc = Acc {
        est_faces: bytes.len() / 50 + 8,
        reserved_tri: false,
        reserved_quad: false,
        grids: map_with_cap(est / 2 + 8),
        shells: Vec::with_capacity(est / 2 + 8),
        tri_faces: Map::default(),
        quad_faces: Map::default(),
        skipped_cp: 0,
        unknown: Map::default(),
    };
    let mut cur = Card::new();
    let mut i = 0;
    let mut last = 0usize;
    while i < bytes.len() {
        if i.saturating_sub(last) >= PROG_STEP {
            if probe.is_some_and(LoadProbe::is_cancelled) {
                break;
            }
            if let Some(p) = probe {
                p.report(base.saturating_add(i as u64));
            }
            last = i;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        let mut line = &bytes[start..i];
        if i < bytes.len() {
            i += 1;
        }
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if let Some(p) = line.iter().position(|&b| b == b'$') {
            line = &line[..p];
        }
        if trim_ascii(line).is_empty() {
            continue;
        }
        let csv = line.contains(&b',');
        let name = if csv {
            trim_ascii(line.split(|&b| b == b',').next().unwrap_or(b""))
        } else {
            trim_ascii(&line[..line.len().min(8)])
        };
        if is_cont(name) {
            if !cur.name.is_empty() {
                let wide = cur.wide || name.ends_with(b"*") || name == b"*";
                append_fields(&mut cur, line, csv, wide);
            }
            continue;
        }
        flush_card(&mut cur, &mut acc);
        if ignore_name(name) {
            continue;
        }
        cur.name = name;
        let wide = name.ends_with(b"*");
        cur.wide = wide;
        append_fields(&mut cur, line, csv, wide);
    }
    flush_card(&mut cur, &mut acc);
    if let Some(p) = probe {
        p.report(base.saturating_add(bytes.len() as u64));
    }
    acc
}

fn flush_card(card: &mut Card<'_>, acc: &mut Acc) {
    if card.name.is_empty() {
        return;
    }
    ingest_card(card, acc);
    card.clear();
}

fn append_fields<'a>(card: &mut Card<'a>, line: &'a [u8], csv: bool, wide: bool) {
    if csv {
        let mut parts = line.split(|&b| b == b',');
        let _name = parts.next();
        for part in parts {
            card.push(trim_ascii(part));
        }
        return;
    }
    let width = if wide { 16 } else { 8 };
    let limit = if wide { 72 } else { 80 };
    let mut off = 8;
    while off < line.len() && off < limit && card.n < MAX_FIELDS {
        let end = (off + width).min(line.len()).min(limit);
        card.push(trim_ascii(&line[off..end]));
        off = end;
    }
}

fn is_cont(name: &[u8]) -> bool {
    name.is_empty() || name[0] == b'+' || name[0] == b'*'
}

fn ignore_name(name: &[u8]) -> bool {
    let n = strip_star(name);
    eq_ci(n, b"ENDDATA")
        || eq_ci(n, b"BEGIN")
        || eq_ci(n, b"CEND")
        || eq_ci(n, b"SOL")
        || eq_ci(n, b"TIME")
        || eq_ci(n, b"PARAM")
        || eq_ci(n, b"INCLUDE")
        || eq_ci(n, b"ID")
        || eq_ci(n, b"ASSIGN")
}

fn strip_star(name: &[u8]) -> &[u8] {
    name.strip_suffix(b"*").unwrap_or(name)
}

fn eq_ci(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn trim_ascii(s: &[u8]) -> &[u8] {
    let start = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(start);
    &s[start..end]
}

fn ingest_card(card: &Card<'_>, acc: &mut Acc) {
    let name = strip_star(card.name);
    if eq_ci(name, b"GRID") {
        ingest_grid(card, acc);
        return;
    }
    if eq_ci(name, b"CTRIA3") {
        if let Some(ids) = parse_nids(card, 3) {
            acc.shells.push(ids);
        }
        return;
    }
    if eq_ci(name, b"CQUAD4") {
        if let Some(n) = parse_nids::<4>(card, 4) {
            acc.shells.push([n[0], n[1], n[2]]);
            acc.shells.push([n[0], n[2], n[3]]);
        }
        return;
    }
    if eq_ci(name, b"CTETRA") {
        if let Some(n) = parse_nids::<4>(card, 4) {
            add_tri(acc, [n[0], n[1], n[2]]);
            add_tri(acc, [n[0], n[3], n[1]]);
            add_tri(acc, [n[1], n[3], n[2]]);
            add_tri(acc, [n[2], n[3], n[0]]);
        }
        return;
    }
    if eq_ci(name, b"CHEXA") {
        if let Some(n) = parse_nids::<8>(card, 8) {
            add_quad(acc, [n[0], n[1], n[2], n[3]]);
            add_quad(acc, [n[4], n[7], n[6], n[5]]);
            add_quad(acc, [n[0], n[4], n[5], n[1]]);
            add_quad(acc, [n[1], n[5], n[6], n[2]]);
            add_quad(acc, [n[2], n[6], n[7], n[3]]);
            add_quad(acc, [n[3], n[7], n[4], n[0]]);
        }
        return;
    }
    if eq_ci(name, b"PSOLID")
        || eq_ci(name, b"PSHELL")
        || eq_ci(name, b"MAT1")
        || eq_ci(name, b"MAT2")
        || eq_ci(name, b"SPC")
        || eq_ci(name, b"SPC1")
        || eq_ci(name, b"LOAD")
        || eq_ci(name, b"FORCE")
        || eq_ci(name, b"MOMENT")
        || eq_ci(name, b"MPC")
        || eq_ci(name, b"EIGR")
        || eq_ci(name, b"EIGRL")
    {
        return;
    }
    *acc.unknown
        .entry(String::from_utf8_lossy(name).into_owned())
        .or_insert(0) += 1;
}

fn ingest_grid(card: &Card<'_>, acc: &mut Acc) {
    let Some(id) = parse_u32(card.get(0)) else {
        return;
    };
    let cp = if card.get(1).is_empty() {
        0
    } else {
        parse_u32(card.get(1)).unwrap_or(0)
    };
    let Some(x) = parse_nas_f32(card.get(2)) else {
        return;
    };
    let Some(y) = parse_nas_f32(card.get(3)) else {
        return;
    };
    let Some(z) = parse_nas_f32(card.get(4)) else {
        return;
    };
    if cp != 0 {
        acc.skipped_cp += 1;
        return;
    }
    acc.grids.insert(id, [x, y, z]);
}

fn parse_nids<const N: usize>(card: &Card<'_>, need: usize) -> Option<[u32; N]> {
    debug_assert_eq!(N, need);
    if card.n < 2 + need {
        return None;
    }
    let mut ids = [0u32; N];
    for (i, slot) in ids.iter_mut().enumerate() {
        let id = parse_u32(card.get(2 + i))?;
        if id == 0 {
            return None;
        }
        *slot = id;
    }
    Some(ids)
}

fn parse_u32(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut n = 0u32;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

fn parse_nas_f32(s: &[u8]) -> Option<f32> {
    if s.is_empty() {
        return None;
    }
    let mut e_at = None;
    for i in 1..s.len() {
        if s[i] == b'+' || s[i] == b'-' {
            let p = s[i - 1];
            if p != b'e' && p != b'E' && (p.is_ascii_digit() || p == b'.') {
                e_at = Some(i);
                break;
            }
        }
    }
    if let Some(i) = e_at {
        let mut tmp = [0u8; 48];
        if i + 1 + (s.len() - i) > tmp.len() {
            return None;
        }
        tmp[..i].copy_from_slice(&s[..i]);
        tmp[i] = b'e';
        tmp[i + 1..i + 1 + s.len() - i].copy_from_slice(&s[i..]);
        return fast_float2::parse::<f32, _>(&tmp[..s.len() + 1]).ok();
    }
    fast_float2::parse::<f32, _>(s).ok()
}

fn finish(acc: Acc, probe: Option<&LoadProbe>) -> Result<(ParsedMesh, Vec<String>)> {
    if probe.is_some_and(LoadProbe::is_cancelled) {
        bail!("cancelled");
    }
    let Acc {
        grids,
        shells,
        tri_faces,
        quad_faces,
        skipped_cp,
        unknown,
        ..
    } = acc;
    let mut missing = 0u32;
    let mut mesh = ParsedMesh::empty();
    mesh.positions
        .reserve(shells.len() * 3 + tri_faces.len() * 3 + quad_faces.len() * 6);
    for ids in shells {
        push_tri(&mut mesh, &grids, ids, &mut missing);
    }
    for (key, hit) in tri_faces {
        if hit.count != 1 {
            continue;
        }
        push_tri(&mut mesh, &grids, unrank_perm(&key, hit.perm), &mut missing);
    }
    for (key, hit) in quad_faces {
        if hit.count != 1 {
            continue;
        }
        let [a, b, c, d] = unrank_perm(&key, hit.perm);
        push_tri(&mut mesh, &grids, [a, b, c], &mut missing);
        push_tri(&mut mesh, &grids, [a, c, d], &mut missing);
    }

    let mut warnings = Vec::new();
    if skipped_cp > 0 {
        warnings.push(format!("skipped GRID with cp != 0: {skipped_cp}"));
    }
    if missing > 0 {
        warnings.push(format!("missing GRID refs: {missing}"));
    }
    if !unknown.is_empty() {
        let mut kinds: Vec<_> = unknown.into_iter().collect();
        kinds.sort_by(|a, b| a.0.cmp(&b.0));
        let detail = kinds
            .into_iter()
            .map(|(n, c)| format!("{n} ({c})"))
            .collect::<Vec<_>>()
            .join(", ");
        warnings.push(format!("unknown cards: {detail}"));
    }
    if mesh.positions.is_empty() {
        if warnings.is_empty() {
            bail!("NAS has no surface triangles");
        }
        bail!("NAS has no surface triangles ({})", warnings.join("; "));
    }
    Ok((mesh, warnings))
}

struct FaceHit {
    perm: u8,
    count: u8,
}

fn fact(n: usize) -> u8 {
    match n {
        0 | 1 => 1,
        2 => 2,
        3 => 6,
        4 => 24,
        _ => 1,
    }
}

fn rank_perm<const N: usize>(sorted: &[u32; N], orig: &[u32; N]) -> u8 {
    debug_assert!(N <= 4);
    let mut used = [false; 4];
    let mut rank = 0u8;
    for i in 0..N {
        let mut chosen = None;
        for j in 0..N {
            if !used[j] && sorted[j] == orig[i] {
                chosen = Some(j);
                break;
            }
        }
        let Some(j) = chosen else {
            return 0;
        };
        let mut smaller = 0u8;
        for k in 0..j {
            if !used[k] {
                smaller += 1;
            }
        }
        used[j] = true;
        rank += smaller * fact(N - 1 - i);
    }
    rank
}

fn unrank_perm<const N: usize>(sorted: &[u32; N], mut rank: u8) -> [u32; N] {
    debug_assert!(N <= 4);
    let mut avail = [true; 4];
    let mut out = [0u32; N];
    for i in 0..N {
        let f = fact(N - 1 - i);
        let mut skip = if f == 0 { 0 } else { rank / f };
        rank = if f == 0 { 0 } else { rank % f };
        for j in 0..N {
            if !avail[j] {
                continue;
            }
            if skip == 0 {
                avail[j] = false;
                out[i] = sorted[j];
                break;
            }
            skip -= 1;
        }
    }
    out
}

fn add_tri(acc: &mut Acc, n: [u32; 3]) {
    if !acc.reserved_tri {
        acc.reserved_tri = true;
        acc.tri_faces.reserve(acc.est_faces);
    }
    let mut key = n;
    key.sort_unstable();
    let perm = rank_perm(&key, &n);
    acc.tri_faces
        .entry(key)
        .and_modify(|h| {
            if h.count < 2 {
                h.count += 1;
            }
        })
        .or_insert(FaceHit { perm, count: 1 });
}

fn add_quad(acc: &mut Acc, n: [u32; 4]) {
    if !acc.reserved_quad {
        acc.reserved_quad = true;
        acc.quad_faces.reserve(acc.est_faces);
    }
    let mut key = n;
    key.sort_unstable();
    let perm = rank_perm(&key, &n);
    acc.quad_faces
        .entry(key)
        .and_modify(|h| {
            if h.count < 2 {
                h.count += 1;
            }
        })
        .or_insert(FaceHit { perm, count: 1 });
}

fn push_tri(mesh: &mut ParsedMesh, grids: &Map<u32, [f32; 3]>, ids: [u32; 3], missing: &mut u32) {
    let Some(a) = grids.get(&ids[0]) else {
        *missing += 1;
        return;
    };
    let Some(b) = grids.get(&ids[1]) else {
        *missing += 1;
        return;
    };
    let Some(c) = grids.get(&ids[2]) else {
        *missing += 1;
        return;
    };
    for v in [*a, *b, *c] {
        let t = Vec3::from_array(v);
        mesh.min = mesh.min.min(t);
        mesh.max = mesh.max.max(t);
        mesh.positions.push(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri_ascii() -> &'static str {
        "GRID           1              0.      0.      0.\n\
         GRID           2              1.      0.      0.\n\
         GRID           3              0.      1.      0.\n\
         CTRIA3         1       1       1       2       3\n\
         PSHELL         1       1     .001\n\
         MAT1           1  2.1+11              .3\n\
         ENDDATA\n"
    }

    #[test]
    fn face_perm_roundtrip() {
        let orig3 = [10u32, 3, 7];
        let mut key3 = orig3;
        key3.sort_unstable();
        assert_eq!(unrank_perm(&key3, rank_perm(&key3, &orig3)), orig3);
        let orig4 = [9u32, 1, 4, 2];
        let mut key4 = orig4;
        key4.sort_unstable();
        assert_eq!(unrank_perm(&key4, rank_perm(&key4, &orig4)), orig4);
    }

    #[test]
    fn parse_one_triangle_and_skip_props() {
        let (mesh, warnings) = parse_nas(tri_ascii().as_bytes()).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.positions[1], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn nastran_exponent_and_jammed_fields() {
        let src = "\
GRID           1        -6.001-3 .130398.0178986\n\
GRID           2              1.      0.      0.\n\
GRID           3              0.      1.      0.\n\
CTRIA3         1       1       1       2       3\n";
        let (mesh, _) = parse_nas(src.as_bytes()).unwrap();
        assert!((mesh.positions[0][0] + 0.006001).abs() < 1e-6);
    }

    #[test]
    fn grid_star_two_lines() {
        let src = "\
GRID*                  1                              0.              1.\n\
*                     2.                                                \n\
GRID*                  2                              1.              1.\n\
*                     2.                                                \n\
GRID*                  3                              0.              2.\n\
*                     2.                                                \n\
CTRIA3         1       1       1       2       3\n";
        let (mesh, w) = parse_nas(src.as_bytes()).unwrap();
        assert!(w.is_empty());
        assert_eq!(mesh.positions[0], [0.0, 1.0, 2.0]);
    }

    #[test]
    fn chexa_skin_has_twelve_triangles() {
        // 継続行の先頭空白は `\n\` だと落ちるので concat で列を保つ。
        let src = concat!(
            "GRID           1              0.      0.      0.\n",
            "GRID           2              1.      0.      0.\n",
            "GRID           3              1.      1.      0.\n",
            "GRID           4              0.      1.      0.\n",
            "GRID           5              0.      0.      1.\n",
            "GRID           6              1.      0.      1.\n",
            "GRID           7              1.      1.      1.\n",
            "GRID           8              0.      1.      1.\n",
            "CHEXA          1       1       1       2       3       4       5       6\n",
            "               7       8\n",
        );
        let (mesh, w) = parse_nas(src.as_bytes()).unwrap();
        assert!(w.is_empty());
        assert_eq!(mesh.positions.len() / 3, 12);
    }

    #[test]
    fn two_hexes_drop_shared_face() {
        let bulk = concat!(
            "GRID           1              0.      0.      0.\n",
            "GRID           2              1.      0.      0.\n",
            "GRID           3              1.      1.      0.\n",
            "GRID           4              0.      1.      0.\n",
            "GRID           5              0.      0.      1.\n",
            "GRID           6              1.      0.      1.\n",
            "GRID           7              1.      1.      1.\n",
            "GRID           8              0.      1.      1.\n",
            "GRID           9              0.      0.      2.\n",
            "GRID          10              1.      0.      2.\n",
            "GRID          11              1.      1.      2.\n",
            "GRID          12              0.      1.      2.\n",
            "CHEXA          1       1       1       2       3       4       5       6\n",
            "               7       8\n",
            "CHEXA          2       1       5       6       7       8       9      10\n",
            "              11      12\n",
        );
        let (mesh, _) = parse_nas(bulk.as_bytes()).unwrap();
        assert_eq!(mesh.positions.len() / 3, 20);
    }

    #[test]
    fn skip_cp_and_unknown() {
        let src = "\
GRID           1              0.      0.      0.\n\
GRID           2              1.      0.      0.\n\
GRID           3              0.      1.      0.\n\
GRID           4       2      9.      9.      9.\n\
CTRIA3         1       1       1       2       3\n\
CORD2R         1\n";
        let (mesh, warnings) = parse_nas(src.as_bytes()).unwrap();
        assert_eq!(mesh.positions.len(), 3);
        assert!(warnings.iter().any(|w| w.contains("cp")));
        assert!(warnings.iter().any(|w| w.contains("CORD2R")));
    }

    #[test]
    fn empty_skin_keeps_diagnostic_warnings() {
        let src = "\
GRID           1       2      0.      0.      0.\n\
CORD2R         1\n";
        let err = parse_nas(src.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("no surface triangles"));
        assert!(err.contains("cp != 0"));
        assert!(err.contains("CORD2R"));
    }

    #[test]
    fn free_field_grid() {
        let src =
            "GRID,1,,0.0,0.0,0.0\nGRID,2,,1.0,0.0,0.0\nGRID,3,,0.0,1.0,0.0\nCTRIA3,1,1,1,2,3\n";
        let (mesh, _) = parse_nas(src.as_bytes()).unwrap();
        assert_eq!(mesh.positions.len(), 3);
    }

    #[test]
    fn ctetra_skin_has_four_triangles() {
        let src = concat!(
            "GRID           1              0.      0.      0.\n",
            "GRID           2              1.      0.      0.\n",
            "GRID           3              0.      1.      0.\n",
            "GRID           4              0.      0.      1.\n",
            "CTETRA         1       1       1       2       3       4\n",
        );
        let (mesh, w) = parse_nas(src.as_bytes()).unwrap();
        assert!(w.is_empty());
        assert_eq!(mesh.positions.len() / 3, 4);
    }

    #[test]
    fn cquad4_becomes_two_triangles() {
        let src = "\
GRID           1              0.      0.      0.\n\
GRID           2              1.      0.      0.\n\
GRID           3              1.      1.      0.\n\
GRID           4              0.      1.      0.\n\
CQUAD4         1       1       1       2       3       4\n";
        let (mesh, w) = parse_nas(src.as_bytes()).unwrap();
        assert!(w.is_empty());
        assert_eq!(mesh.positions.len() / 3, 2);
    }

    #[test]
    fn testdata_warnings_nas_has_diagnostics() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/warnings.nas");
        let bytes = std::fs::read(&path).unwrap();
        let (mesh, warnings) = parse_nas(&bytes).unwrap();
        assert_eq!(mesh.positions.len(), 3);
        assert!(warnings.iter().any(|w| w.contains("cp")));
        assert!(warnings.iter().any(|w| w.contains("CORD2R")));
    }

    #[test]
    fn testdata_warnings_long_covers_all_kinds() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/warnings_long.nas");
        let bytes = std::fs::read(&path).unwrap();
        let (mesh, warnings) = parse_nas(&bytes).unwrap();
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(warnings.len(), 3);
        assert!(warnings
            .iter()
            .any(|w| w.contains("skipped GRID") && w.contains("30")));
        assert!(warnings.iter().any(|w| w.contains("missing GRID")));
        let unknown = warnings
            .iter()
            .find(|w| w.contains("unknown cards"))
            .expect("unknown cards");
        assert!(unknown.len() > 200);
        assert!(unknown.matches(',').count() >= 20);
    }

    #[test]
    fn testdata_warnings_fail_has_no_triangles() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/warnings_fail.nas");
        let bytes = std::fs::read(&path).unwrap();
        let err = parse_nas(&bytes).unwrap_err().to_string();
        assert!(err.contains("no surface triangles"));
        assert!(err.contains("cp != 0"));
        assert!(err.contains("CORD2R"));
    }
}
