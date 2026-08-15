//! Nastran bulk（`.nas` / `.nastran`）から外皮の三角形スープを作る。
//!
//! 対象カードは `GRID` / `CTRIA3` / `CQUAD4` / `CTETRA` / `CHEXA`。`cp ≠ 0` の節点は捨てて件数を警告する。
//! `CORD*` は読まない。未知カードは飛ばして種類と件数を出す。体積の内部面は出さない。
//!
//! 小さいファイルはこのマイルストーンでは点群を出さず、外皮まで同期で載せる。
//! 点群先行は走査を裏に回す C-lite 側。

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use glam::Vec3;
use memmap2::Mmap;
use rayon::prelude::*;

use crate::mesh::ParsedMesh;

/// これ以上の論理カードはパースを Rayon で分割する。
const PAR_CARDS: usize = 10_000;

/// 1 ファイルを mmap して NAS 外皮へ展開する。
///
/// 警告（未知カード、`cp ≠ 0`、欠ける GRID）はファイルを捨てずに返す。
///
/// # Errors
///
/// 開けない、マップできない、または外皮三角形が 1 枚も無いとき。
pub(crate) fn load_nas(path: &Path) -> Result<(ParsedMesh, Vec<String>)> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mmap = unsafe { Mmap::map(&file) }.with_context(|| format!("mmap {}", path.display()))?;
    parse_nas(&mmap)
}

/// バイト列を bulk として解釈し、外皮三角形を返す。
///
/// `BEGIN BULK` は無くてよい。継続行は先頭 8 桁が空、`+`、`*`。
///
/// # Errors
///
/// 外皮三角形が 1 枚も無いとき。
pub(crate) fn parse_nas(bytes: &[u8]) -> Result<(ParsedMesh, Vec<String>)> {
    let cards = logical_cards(bytes);
    let parsed: Vec<ParsedCard> = if cards.len() >= PAR_CARDS {
        cards.par_iter().map(parse_card).collect()
    } else {
        cards.iter().map(parse_card).collect()
    };
    assemble(parsed)
}

struct LogicalCard {
    name: String,
    fields: Vec<String>,
}

enum ParsedCard {
    Grid { id: u32, cp: u32, xyz: [f32; 3] },
    Ctria3([u32; 3]),
    Cquad4([u32; 4]),
    Ctetra([u32; 4]),
    Chexa([u32; 8]),
    Unknown(String),
    Skip,
}

fn logical_cards(bytes: &[u8]) -> Vec<LogicalCard> {
    let mut out = Vec::new();
    let mut cur: Option<LogicalCard> = None;
    let mut i = 0;
    while i < bytes.len() {
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
        let (name, wide, fields) = line_fields(line);
        if is_cont(&name) {
            if let Some(card) = cur.as_mut() {
                let more = if card.name.ends_with('*') || wide {
                    fields_wide(line)
                } else {
                    fields
                };
                card.fields.extend(more);
            }
            continue;
        }
        if let Some(card) = cur.take() {
            out.push(strip_star(card));
        }
        if ignore_name(&name) {
            continue;
        }
        cur = Some(LogicalCard { name, fields });
    }
    if let Some(card) = cur.take() {
        out.push(strip_star(card));
    }
    out
}

fn strip_star(mut card: LogicalCard) -> LogicalCard {
    if let Some(trimmed) = card.name.strip_suffix('*') {
        card.name = trimmed.to_string();
    }
    card
}

fn is_cont(name: &str) -> bool {
    name.is_empty() || name.starts_with('+') || name == "*" || name.starts_with('*')
}

fn ignore_name(name: &str) -> bool {
    matches!(
        name,
        "ENDDATA" | "BEGIN" | "CEND" | "SOL" | "TIME" | "PARAM" | "INCLUDE" | "ID" | "ASSIGN"
    )
}

fn line_fields(line: &[u8]) -> (String, bool, Vec<String>) {
    if line.contains(&b',') {
        let text = String::from_utf8_lossy(line);
        let mut parts = text.split(',').map(|s| s.trim().to_string());
        let name = parts.next().unwrap_or_default();
        return (name, false, parts.collect());
    }
    let name = field_text(&line[..line.len().min(8)]);
    let wide = name.ends_with('*') || name == "*";
    let fields = if wide {
        fields_wide(line)
    } else {
        fields_small(line)
    };
    (name, wide, fields)
}

fn fields_small(line: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut off = 8;
    while off < line.len() && off < 80 {
        let end = (off + 8).min(line.len());
        out.push(field_text(&line[off..end]));
        off = end;
    }
    out
}

fn fields_wide(line: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut off = 8;
    while off < line.len() && off < 72 {
        let end = (off + 16).min(line.len());
        out.push(field_text(&line[off..end]));
        off = end;
    }
    out
}

fn field_text(s: &[u8]) -> String {
    String::from_utf8_lossy(trim_ascii(s)).into_owned()
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

fn parse_card(card: &LogicalCard) -> ParsedCard {
    match card.name.to_ascii_uppercase().as_str() {
        "GRID" => parse_grid(&card.fields),
        "CTRIA3" => parse_nids(&card.fields, 3)
            .map(ParsedCard::Ctria3)
            .unwrap_or(ParsedCard::Skip),
        "CQUAD4" => parse_nids(&card.fields, 4)
            .map(ParsedCard::Cquad4)
            .unwrap_or(ParsedCard::Skip),
        "CTETRA" => parse_nids(&card.fields, 4)
            .map(ParsedCard::Ctetra)
            .unwrap_or(ParsedCard::Skip),
        "CHEXA" => parse_nids(&card.fields, 8)
            .map(ParsedCard::Chexa)
            .unwrap_or(ParsedCard::Skip),
        "PSOLID" | "PSHELL" | "MAT1" | "MAT2" | "SPC" | "SPC1" | "LOAD" | "FORCE" | "MOMENT"
        | "MPC" | "EIGR" | "EIGRL" => ParsedCard::Skip,
        _ => ParsedCard::Unknown(card.name.clone()),
    }
}

fn parse_grid(fields: &[String]) -> ParsedCard {
    let Some(id) = parse_u32(fields.first().map(|s| s.as_str()).unwrap_or("")) else {
        return ParsedCard::Skip;
    };
    let cp = fields
        .get(1)
        .and_then(|s| if s.is_empty() { Some(0) } else { parse_u32(s) })
        .unwrap_or(0);
    let Some(x) = fields.get(2).and_then(|s| parse_nas_f32(s)) else {
        return ParsedCard::Skip;
    };
    let Some(y) = fields.get(3).and_then(|s| parse_nas_f32(s)) else {
        return ParsedCard::Skip;
    };
    let Some(z) = fields.get(4).and_then(|s| parse_nas_f32(s)) else {
        return ParsedCard::Skip;
    };
    ParsedCard::Grid {
        id,
        cp,
        xyz: [x, y, z],
    }
}

fn parse_nids<const N: usize>(fields: &[String], need: usize) -> Option<[u32; N]> {
    debug_assert_eq!(N, need);
    let mut ids = [0u32; N];
    // EID, PID, then nodes
    let nodes = fields.get(2..)?;
    if nodes.len() < need {
        return None;
    }
    for (i, f) in nodes.iter().take(need).enumerate() {
        ids[i] = parse_u32(f)?;
        if ids[i] == 0 {
            return None;
        }
    }
    Some(ids)
}

fn parse_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse().ok()
}

fn parse_nas_f32(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let b = t.as_bytes();
    let mut e_at = None;
    for i in 1..b.len() {
        if b[i] == b'+' || b[i] == b'-' {
            let p = b[i - 1];
            if p != b'e' && p != b'E' && (p.is_ascii_digit() || p == b'.') {
                e_at = Some(i);
                break;
            }
        }
    }
    if let Some(i) = e_at {
        let mut tmp = String::with_capacity(t.len() + 1);
        tmp.push_str(&t[..i]);
        tmp.push('e');
        tmp.push_str(&t[i..]);
        return fast_float2::parse::<f32, _>(tmp.as_str()).ok();
    }
    fast_float2::parse::<f32, _>(t).ok()
}

fn assemble(cards: Vec<ParsedCard>) -> Result<(ParsedMesh, Vec<String>)> {
    let mut grids: HashMap<u32, [f32; 3]> = HashMap::new();
    let mut skipped_cp = 0u32;
    let mut unknown: HashMap<String, u32> = HashMap::new();
    let mut missing = 0u32;
    let mut tri_faces: HashMap<[u32; 3], FaceHit> = HashMap::new();
    let mut quad_faces: HashMap<[u32; 4], FaceHit> = HashMap::new();

    for card in cards {
        match card {
            ParsedCard::Grid { id, cp, xyz } => {
                if cp != 0 {
                    skipped_cp += 1;
                    continue;
                }
                grids.insert(id, xyz);
            }
            ParsedCard::Ctria3(n) => add_tri(&mut tri_faces, n),
            ParsedCard::Cquad4(n) => add_quad(&mut quad_faces, n),
            ParsedCard::Ctetra(n) => {
                add_tri(&mut tri_faces, [n[0], n[1], n[2]]);
                add_tri(&mut tri_faces, [n[0], n[3], n[1]]);
                add_tri(&mut tri_faces, [n[1], n[3], n[2]]);
                add_tri(&mut tri_faces, [n[2], n[3], n[0]]);
            }
            ParsedCard::Chexa(n) => {
                add_quad(&mut quad_faces, [n[0], n[1], n[2], n[3]]);
                add_quad(&mut quad_faces, [n[4], n[7], n[6], n[5]]);
                add_quad(&mut quad_faces, [n[0], n[4], n[5], n[1]]);
                add_quad(&mut quad_faces, [n[1], n[5], n[6], n[2]]);
                add_quad(&mut quad_faces, [n[2], n[6], n[7], n[3]]);
                add_quad(&mut quad_faces, [n[3], n[7], n[4], n[0]]);
            }
            ParsedCard::Unknown(name) => {
                *unknown.entry(name).or_insert(0) += 1;
            }
            ParsedCard::Skip => {}
        }
    }

    let mut mesh = ParsedMesh::empty();
    for hit in tri_faces.into_values() {
        if hit.count != 1 || hit.wind.len() < 3 {
            continue;
        }
        push_tri(
            &mut mesh,
            &grids,
            [hit.wind[0], hit.wind[1], hit.wind[2]],
            &mut missing,
        );
    }
    for hit in quad_faces.into_values() {
        if hit.count != 1 || hit.wind.len() < 4 {
            continue;
        }
        let a = hit.wind[0];
        let b = hit.wind[1];
        let c = hit.wind[2];
        let d = hit.wind[3];
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
        bail!("NAS has no surface triangles");
    }
    Ok((mesh, warnings))
}

struct FaceHit {
    wind: Vec<u32>,
    count: u32,
}

fn add_tri(map: &mut HashMap<[u32; 3], FaceHit>, n: [u32; 3]) {
    let mut key = n;
    key.sort_unstable();
    map.entry(key)
        .and_modify(|h| h.count += 1)
        .or_insert(FaceHit {
            wind: n.to_vec(),
            count: 1,
        });
}

fn add_quad(map: &mut HashMap<[u32; 4], FaceHit>, n: [u32; 4]) {
    let mut key = n;
    key.sort_unstable();
    map.entry(key)
        .and_modify(|h| h.count += 1)
        .or_insert(FaceHit {
            wind: n.to_vec(),
            count: 1,
        });
}

fn push_tri(
    mesh: &mut ParsedMesh,
    grids: &HashMap<u32, [f32; 3]>,
    ids: [u32; 3],
    missing: &mut u32,
) {
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
}
