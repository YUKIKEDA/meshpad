//! STL を三角形スープへ展開する。
//!
//! バイナリと ASCII を判別する。`mmap` は読み取りにだけ使い、バイナリの 50 バイトレコードを
//! GPU 頂点へ直接再解釈しない。大きいバイナリは展開と AABB を Rayon で分割する。
//! ASCII の数値は `fast-float2` の部分パース。法線はファイル値を捨て、描画時に面から復元する。

use std::path::Path;

use anyhow::{bail, Context, Result};
use glam::Vec3;
use memmap2::Mmap;
use rayon::prelude::*;

use crate::mesh::{bounds_to_soup, LoadProbe, ParsedMesh};

const HEADER: usize = 80;
const RECORD: usize = 50;
/// これ以上の三角形は展開と AABB を Rayon で分割する。
const PAR_TRIANGLES: usize = 250_000;
pub(crate) const PAR_TRI_CHUNK: usize = 16_384;
const PROG_STEP: usize = 1 << 20;

pub use crate::mesh::TriangleSoup;

/// 複数の STL（バイナリまたは ASCII）をワールド座標のまま結合する。
///
/// 読めたファイルの頂点をファイル座標のまま連結し、全体 AABB の中心を [`TriangleSoup::origin`] に残す。
/// 失敗したパスは捨てず、警告文字列のベクタに残す。
///
/// # Errors
///
/// 三角形が 1 枚も得られないとき（全部失敗、または空）。
///
/// # Examples
///
/// ```ignore
/// let (soup, warnings) = meshpad::stl::load_paths(&["a.stl", "b.stl"])?;
/// let _ = (soup.triangle_count(), warnings.len());
/// ```
pub fn load_paths(paths: &[impl AsRef<Path>]) -> Result<(TriangleSoup, Vec<String>)> {
    let mut all = Vec::new();
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let mut warnings = Vec::new();
    for p in paths {
        let p = p.as_ref();
        match load_parsed(p) {
            Ok(mesh) => {
                if all.is_empty() {
                    min = mesh.min;
                    max = mesh.max;
                    all = mesh.positions;
                } else {
                    min = min.min(mesh.min);
                    max = max.max(mesh.max);
                    all.extend(mesh.positions);
                }
            }
            Err(e) => warnings.push(format!("{}: {e}", p.display())),
        }
    }
    if all.is_empty() {
        let detail = if warnings.is_empty() {
            "no triangles".into()
        } else {
            warnings.join("; ")
        };
        bail!("{detail}");
    }
    Ok((bounds_to_soup(all, min, max), warnings))
}

/// 1 ファイルを mmap して STL として読む。
///
/// バイナリと ASCII はバイト列から判別する。得られた位置はファイル座標のまま（中心化しない）。
/// 結合・シフトは [`load_paths`] 側。
///
/// # Errors
///
/// 開けない、マップできない、または [`parse_stl`] が失敗したとき。
pub fn load_stl(path: &Path) -> Result<Vec<[f32; 3]>> {
    Ok(load_parsed(path)?.positions)
}

pub(crate) fn load_parsed(path: &Path) -> Result<ParsedMesh> {
    load_parsed_at(path, None, 0)
}

pub(crate) fn load_parsed_at(
    path: &Path,
    probe: Option<&LoadProbe>,
    base: u64,
) -> Result<ParsedMesh> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mmap = unsafe { Mmap::map(&file) }.with_context(|| format!("mmap {}", path.display()))?;
    parse_parsed(&mmap, probe, base)
}

/// バイト列を STL として解釈する。
///
/// 長さが `84 + 50 * 枚数` と一致すればバイナリ。一致せず ASCII らしいときだけテキストとして読む。
/// ヘッダが `solid` で始まるバイナリは、長さが合えばバイナリのまま扱う。
///
/// # Errors
///
/// 短すぎる、切れている、ASCII として三角形が取れない、または 0 枚のとき。
///
/// # Examples
///
/// ```ignore
/// let positions = meshpad::stl::parse_stl(&bytes)?;
/// assert_eq!(positions.len() % 3, 0);
/// ```
pub fn parse_stl(bytes: &[u8]) -> Result<Vec<[f32; 3]>> {
    Ok(parse_parsed(bytes, None, 0)?.positions)
}

fn parse_parsed(bytes: &[u8], probe: Option<&LoadProbe>, base: u64) -> Result<ParsedMesh> {
    if cancelled(probe) {
        bail!("cancelled");
    }
    if looks_like_ascii(bytes) && !binary_size_matches(bytes) {
        parse_ascii_mesh(bytes, probe, base)
    } else {
        parse_binary_mesh(bytes, probe, base)
    }
}

/// バイト列をバイナリ STL として解釈する。
///
/// ヘッダ 80 バイトと little-endian の枚数のあと、1 三角形 50 バイト（法線 12、頂点 36、属性 2）とみなして位置だけを展開する。
///
/// # Errors
///
/// 短すぎる、切れている、または三角形が 0 枚のとき。
///
/// # Examples
///
/// ```ignore
/// let positions = meshpad::stl::parse_binary_stl(&bytes)?;
/// assert_eq!(positions.len() % 3, 0);
/// ```
pub fn parse_binary_stl(bytes: &[u8]) -> Result<Vec<[f32; 3]>> {
    Ok(parse_binary_mesh(bytes, None, 0)?.positions)
}

fn parse_binary_mesh(bytes: &[u8], probe: Option<&LoadProbe>, base: u64) -> Result<ParsedMesh> {
    if cancelled(probe) {
        bail!("cancelled");
    }
    if bytes.len() < HEADER + 4 {
        bail!("STL too small");
    }
    let count = u32::from_le_bytes(bytes[HEADER..HEADER + 4].try_into().unwrap()) as usize;
    let need = HEADER + 4 + count.saturating_mul(RECORD);
    if bytes.len() < need {
        bail!("truncated STL: need {need} bytes, have {}", bytes.len());
    }
    if count == 0 {
        bail!("STL has zero triangles");
    }
    let recs = &bytes[HEADER + 4..need];
    let nvert = count * 3;
    let mut positions = Vec::with_capacity(nvert);
    // SAFETY: `unpack_tris` が全要素を書く。`[f32; 3]` は drop しない。
    unsafe {
        positions.set_len(nvert);
    }
    let (min, max) = if count >= PAR_TRIANGLES {
        unpack_tris_par(recs, &mut positions, probe, base)?
    } else {
        unpack_tris(recs, &mut positions, probe, Some(base))?
    };
    if cancelled(probe) {
        bail!("cancelled");
    }
    report(probe, base.saturating_add(bytes.len() as u64));
    Ok(ParsedMesh {
        positions,
        min,
        max,
    })
}

fn unpack_tris(
    recs: &[u8],
    out: &mut [[f32; 3]],
    probe: Option<&LoadProbe>,
    progress_base: Option<u64>,
) -> Result<(Vec3, Vec3)> {
    debug_assert_eq!(recs.len() / RECORD, out.len() / 3);
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for (i, rec) in recs.chunks_exact(RECORD).enumerate() {
        if i % PAR_TRI_CHUNK == 0 {
            if cancelled(probe) {
                bail!("cancelled");
            }
            if let Some(base) = progress_base {
                report(probe, base.saturating_add((i * RECORD) as u64));
            }
        }
        for v in 0..3 {
            let o = 12 + v * 12;
            let x = f32::from_le_bytes(rec[o..o + 4].try_into().unwrap());
            let y = f32::from_le_bytes(rec[o + 4..o + 8].try_into().unwrap());
            let z = f32::from_le_bytes(rec[o + 8..o + 12].try_into().unwrap());
            let t = Vec3::new(x, y, z);
            min = min.min(t);
            max = max.max(t);
            out[i * 3 + v] = [x, y, z];
        }
    }
    Ok((min, max))
}

fn unpack_tris_par(
    recs: &[u8],
    out: &mut [[f32; 3]],
    probe: Option<&LoadProbe>,
    base: u64,
) -> Result<(Vec3, Vec3)> {
    if cancelled(probe) {
        bail!("cancelled");
    }
    let rec_stride = RECORD * PAR_TRI_CHUNK;
    let vert_stride = 3 * PAR_TRI_CHUNK;
    report(probe, base);
    let bounds: Result<Vec<(Vec3, Vec3)>, anyhow::Error> = recs
        .par_chunks(rec_stride)
        .zip(out.par_chunks_mut(vert_stride))
        .map(|(rec_chunk, vert_chunk)| {
            let b = unpack_tris(rec_chunk, vert_chunk, probe, None)?;
            if let Some(p) = probe {
                p.done
                    .fetch_add(rec_chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
            }
            Ok::<_, anyhow::Error>(b)
        })
        .collect();
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for (a, b) in bounds? {
        min = min.min(a);
        max = max.max(b);
    }
    Ok((min, max))
}

/// バイト列を ASCII STL として解釈する。
///
/// `facet` / `vertex` / `endfacet` を行単位で読む。キーワードは大文字小文字を問わない。
/// ファイル内の法線は捨て、`vertex` の座標だけを取る。頂点が 3 つ揃わない facet は飛ばす。
/// 先頭の UTF-8 BOM は無視する。UTF-8 なら行分割だけ `str::lines`、数値は `fast-float2` の部分パース。
/// 非 UTF-8 は行をバイト走査する。
///
/// # Errors
///
/// 三角形が 1 枚も取れないとき。
///
/// # Examples
///
/// ```ignore
/// let positions = meshpad::stl::parse_ascii_stl(b"solid x\n...")?;
/// assert_eq!(positions.len(), 3);
/// ```
pub fn parse_ascii_stl(bytes: &[u8]) -> Result<Vec<[f32; 3]>> {
    Ok(parse_ascii_mesh(bytes, None, 0)?.positions)
}

fn parse_ascii_mesh(bytes: &[u8], probe: Option<&LoadProbe>, base: u64) -> Result<ParsedMesh> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => parse_ascii_text(text, bytes.len(), probe, base),
        Err(_) => parse_ascii_bytes(bytes, probe, base),
    }
}

fn parse_ascii_text(
    text: &str,
    nbytes: usize,
    probe: Option<&LoadProbe>,
    base: u64,
) -> Result<ParsedMesh> {
    let mut acc = AsciiAcc::new(nbytes);
    let mut seen = 0usize;
    let mut last = 0usize;
    for raw in text.lines() {
        seen += raw.len() + 1;
        if seen.saturating_sub(last) >= PROG_STEP {
            if cancelled(probe) {
                bail!("cancelled");
            }
            report(probe, base.saturating_add(seen as u64));
            last = seen;
        }
        let line = trim_ascii(raw.as_bytes());
        if !line.is_empty() {
            acc.ingest(line);
        }
    }
    if cancelled(probe) {
        bail!("cancelled");
    }
    report(probe, base.saturating_add(nbytes as u64));
    acc.finish()
}

fn parse_ascii_bytes(bytes: &[u8], probe: Option<&LoadProbe>, base: u64) -> Result<ParsedMesh> {
    let mut acc = AsciiAcc::new(bytes.len());
    let mut i = 0;
    let mut last = 0usize;
    while i < bytes.len() {
        if i.saturating_sub(last) >= PROG_STEP {
            if cancelled(probe) {
                bail!("cancelled");
            }
            report(probe, base.saturating_add(i as u64));
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
        line = trim_ascii(line);
        if !line.is_empty() {
            acc.ingest(line);
        }
    }
    if cancelled(probe) {
        bail!("cancelled");
    }
    report(probe, base.saturating_add(bytes.len() as u64));
    acc.finish()
}

struct AsciiAcc {
    positions: Vec<[f32; 3]>,
    min: Vec3,
    max: Vec3,
    facet: [[f32; 3]; 3],
    nvert: u8,
}

impl AsciiAcc {
    fn new(nbytes: usize) -> Self {
        Self {
            positions: Vec::with_capacity((nbytes / 60).max(3)),
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
            facet: [[0f32; 3]; 3],
            nvert: 0,
        }
    }

    fn ingest(&mut self, line: &[u8]) {
        if keyword_at(line, b"vertex") {
            match parse_xyz_bytes(skip_ascii_ws(&line[6..])) {
                Some(v) if self.nvert < 3 => {
                    self.facet[self.nvert as usize] = v;
                    self.nvert += 1;
                }
                _ => self.nvert = 0,
            }
        } else if keyword_at(line, b"facet") {
            self.nvert = 0;
        } else if keyword_at(line, b"endfacet") {
            if self.nvert == 3 {
                self.push_facet();
            }
            self.nvert = 0;
        }
    }

    fn push_facet(&mut self) {
        for v in self.facet {
            let t = Vec3::from_array(v);
            self.min = self.min.min(t);
            self.max = self.max.max(t);
            self.positions.push(v);
        }
    }

    fn finish(mut self) -> Result<ParsedMesh> {
        if self.nvert == 3 {
            self.push_facet();
        }
        if self.positions.is_empty() {
            bail!("STL has zero triangles");
        }
        Ok(ParsedMesh {
            positions: self.positions,
            min: self.min,
            max: self.max,
        })
    }
}

fn trim_ascii(s: &[u8]) -> &[u8] {
    skip_ascii_ws(s).trim_ascii_end()
}

fn skip_ascii_ws(s: &[u8]) -> &[u8] {
    let n = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    &s[n..]
}

fn keyword_at(line: &[u8], kw: &[u8]) -> bool {
    line.len() >= kw.len()
        && line[..kw.len()].eq_ignore_ascii_case(kw)
        && (line.len() == kw.len() || line[kw.len()].is_ascii_whitespace())
}

fn parse_xyz_bytes(s: &[u8]) -> Option<[f32; 3]> {
    let (x, s) = parse_f32_token(s)?;
    let (y, s) = parse_f32_token(skip_ascii_ws(s))?;
    let (z, _) = parse_f32_token(skip_ascii_ws(s))?;
    Some([x, y, z])
}

fn parse_f32_token(s: &[u8]) -> Option<(f32, &[u8])> {
    let (n, used) = fast_float2::parse_partial::<f32, _>(s).ok()?;
    if used == 0 {
        return None;
    }
    Some((n, &s[used..]))
}

fn declared_binary_len(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < HEADER + 4 {
        return None;
    }
    let count = u32::from_le_bytes(bytes[HEADER..HEADER + 4].try_into().unwrap()) as usize;
    Some(HEADER + 4 + count.saturating_mul(RECORD))
}

fn binary_size_matches(bytes: &[u8]) -> bool {
    declared_binary_len(bytes).is_some_and(|n| n == bytes.len())
}

fn looks_like_ascii(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let head = &bytes[..bytes.len().min(80)];
    let Ok(s) = std::str::from_utf8(head) else {
        return false;
    };
    let t = s.trim_start().to_ascii_lowercase();
    t.starts_with("solid")
        && !bytes
            .get(HEADER..)
            .unwrap_or(&[])
            .iter()
            .take(64)
            .any(|&b| b == 0)
}

fn report(probe: Option<&LoadProbe>, done: u64) {
    if let Some(p) = probe {
        p.report(done);
    }
}

fn cancelled(probe: Option<&LoadProbe>) -> bool {
    probe.is_some_and(LoadProbe::is_cancelled)
}

#[cfg(test)]
fn recenter(positions: Vec<[f32; 3]>) -> TriangleSoup {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for p in &positions {
        let v = Vec3::from_array(*p);
        min = min.min(v);
        max = max.max(v);
    }
    bounds_to_soup(positions, min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn one_tri_stl() -> Vec<u8> {
        let mut b = vec![0u8; HEADER + 4 + RECORD];
        b[HEADER..HEADER + 4].copy_from_slice(&1u32.to_le_bytes());
        let verts = [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        for (i, v) in verts.iter().enumerate() {
            let o = HEADER + 4 + 12 + i * 12;
            b[o..o + 4].copy_from_slice(&v[0].to_le_bytes());
            b[o + 4..o + 8].copy_from_slice(&v[1].to_le_bytes());
            b[o + 8..o + 12].copy_from_slice(&v[2].to_le_bytes());
        }
        b
    }

    fn one_tri_ascii() -> &'static str {
        "solid tri\n\
         facet normal 0 0 1\n\
         outer loop\n\
         vertex 0 0 0\n\
         vertex 1 0 0\n\
         vertex 0 1 0\n\
         endloop\n\
         endfacet\n\
         endsolid tri\n"
    }

    fn with_temp(test: impl FnOnce(&Path)) {
        let dir = std::env::temp_dir().join(format!(
            "meshpad-stl-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(&dir)));
        let _ = fs::remove_dir_all(&dir);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    #[test]
    fn parse_one_triangle() {
        let soup = recenter(parse_binary_stl(&one_tri_stl()).unwrap());
        assert_eq!(soup.triangle_count(), 1);
        assert!((soup.origin - Vec3::new(0.5, 0.5, 0.0)).length() < 1e-5);
    }

    #[test]
    fn reject_truncated() {
        assert!(parse_binary_stl(&[0u8; 10]).is_err());
    }

    #[test]
    fn unpack_tris_stops_when_cancelled() {
        let recs = vec![0u8; RECORD];
        let mut out = vec![[0.0f32; 3]; 3];
        let probe = LoadProbe::new(1);
        probe.cancel();
        let err = unpack_tris(&recs, &mut out, Some(&probe), Some(0)).unwrap_err();
        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn unpack_tris_par_stops_when_cancelled() {
        let recs = vec![0u8; RECORD * PAR_TRI_CHUNK];
        let mut out = vec![[0.0f32; 3]; 3 * PAR_TRI_CHUNK];
        let probe = LoadProbe::new(1);
        probe.cancel();
        let err = unpack_tris_par(&recs, &mut out, Some(&probe), 0).unwrap_err();
        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn parse_ascii_one_triangle() {
        let pos = parse_ascii_stl(one_tri_ascii().as_bytes()).unwrap();
        assert_eq!(pos, vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let soup = recenter(pos);
        assert_eq!(soup.triangle_count(), 1);
        assert!((soup.origin - Vec3::new(0.5, 0.5, 0.0)).length() < 1e-5);
    }

    #[test]
    fn parse_ascii_is_case_insensitive() {
        let src = "SOLID TRI\r\n\
             FACET NORMAL 0 0 1\r\n\
             OUTER LOOP\r\n\
             VERTEX 0 0 0\r\n\
             VERTEX 1 0 0\r\n\
             VERTEX 0 1 0\r\n\
             ENDLOOP\r\n\
             ENDFACET\r\n\
             ENDSOLID TRI\r\n";
        assert_eq!(parse_ascii_stl(src.as_bytes()).unwrap().len(), 3);
    }

    #[test]
    fn parse_ascii_scientific() {
        let src = "solid s\nfacet normal 0 0 0\nouter loop\n\
             vertex 1.0e-1 2.0E+1 3e0\n\
             vertex 0 0 0\n\
             vertex 0 1 0\n\
             endloop\nendfacet\nendsolid\n";
        let pos = parse_ascii_stl(src.as_bytes()).unwrap();
        assert!((pos[0][0] - 0.1).abs() < 1e-6);
        assert!((pos[0][1] - 20.0).abs() < 1e-6);
        assert!((pos[0][2] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn parse_ascii_skips_incomplete_facet() {
        let src = "solid s\n\
             facet normal 0 0 1\n outer loop\n vertex 0 0 0\n vertex 1 0 0\n endloop\n endfacet\n\
             facet normal 0 0 1\n outer loop\n\
             vertex 0 0 0\n vertex 1 0 0\n vertex 0 1 0\n\
             endloop\n endfacet\n endsolid\n";
        let pos = parse_ascii_stl(src.as_bytes()).unwrap();
        assert_eq!(pos.len(), 3);
        assert_eq!(pos[2], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn parse_ascii_rejects_empty() {
        let err = parse_ascii_stl(b"solid empty\nendsolid empty\n").unwrap_err();
        assert!(err.to_string().contains("zero triangles"));
    }

    #[test]
    fn parse_ascii_strips_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(one_tri_ascii().as_bytes());
        assert_eq!(parse_ascii_stl(&bytes).unwrap().len(), 3);
    }

    #[test]
    fn parse_stl_dispatches_ascii() {
        let pos = parse_stl(one_tri_ascii().as_bytes()).unwrap();
        assert_eq!(pos.len(), 3);
    }

    #[test]
    fn parse_stl_dispatches_ascii_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(one_tri_ascii().as_bytes());
        assert_eq!(parse_stl(&bytes).unwrap().len(), 3);
    }

    #[test]
    fn parse_stl_keeps_binary_with_solid_header() {
        let mut bytes = one_tri_stl();
        bytes[..5].copy_from_slice(b"solid");
        let binary = parse_binary_stl(&bytes).unwrap();
        let dispatched = parse_stl(&bytes).unwrap();
        assert_eq!(dispatched, binary);
    }

    #[test]
    fn load_paths_mixes_ascii_and_binary() {
        with_temp(|dir| {
            let bin = dir.join("a.stl");
            let asc = dir.join("b.stl");
            fs::write(&bin, one_tri_stl()).unwrap();
            fs::write(&asc, one_tri_ascii()).unwrap();
            let (soup, warnings) = load_paths(&[bin, asc]).unwrap();
            assert!(warnings.is_empty());
            assert_eq!(soup.triangle_count(), 2);
        });
    }
}
