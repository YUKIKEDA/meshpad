//! STL を三角形スープへ展開する。
//!
//! バイナリと ASCII を判別する。`mmap` は読み取りにだけ使い、バイナリの 50 バイトレコードを
//! GPU 頂点へ直接再解釈しない。法線はファイル値を捨て、描画時に面から復元する。

use std::borrow::Cow;
use std::path::Path;

use anyhow::{bail, Context, Result};
use glam::Vec3;
use memmap2::Mmap;

const HEADER: usize = 80;
const RECORD: usize = 50;

/// GPU へ渡す前の三角形スープ。
///
/// 位置だけを持つ。隣接頂点は共有せず、法線はシェーダ側で面ごとに出す。
/// `positions` はファイル座標から [`Self::origin`] を引いた値。
#[derive(Debug, Clone)]
pub struct TriangleSoup {
    /// 3 頂点で 1 三角形。描画空間（AABB 中心が原点）。
    pub positions: Vec<[f32; 3]>,
    /// 元ファイル空間での AABB 中心。
    pub origin: Vec3,
    /// AABB 外接球の半径。空に近いメッシュでも下限を持つ。
    pub radius: f32,
}

impl TriangleSoup {
    /// 三角形の枚数を返す。
    ///
    /// `positions` は 3 頂点ずつなので、長さを 3 で割った値。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(soup.triangle_count(), soup.positions.len() / 3);
    /// ```
    pub fn triangle_count(&self) -> usize {
        self.positions.len() / 3
    }
}

/// 複数の STL（バイナリまたは ASCII）をワールド座標のまま結合する。
///
/// 読めたファイルの頂点を連結したあと、全体 AABB で再中心化する。
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
    let mut warnings = Vec::new();
    for p in paths {
        let p = p.as_ref();
        match load_stl(p) {
            Ok(pos) => all.extend(pos),
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
    Ok((recenter(all), warnings))
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
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mmap = unsafe { Mmap::map(&file) }.with_context(|| format!("mmap {}", path.display()))?;
    parse_stl(&mmap)
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
    if looks_like_ascii(bytes) && !binary_size_matches(bytes) {
        parse_ascii_stl(bytes)
    } else {
        parse_binary_stl(bytes)
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
    if bytes.len() < HEADER + 4 {
        bail!("STL too small");
    }
    let count = u32::from_le_bytes(bytes[HEADER..HEADER + 4].try_into().unwrap()) as usize;
    let need = HEADER + 4 + count.saturating_mul(RECORD);
    if bytes.len() < need {
        bail!("truncated STL: need {need} bytes, have {}", bytes.len());
    }
    let mut positions = Vec::with_capacity(count.saturating_mul(3));
    let recs = &bytes[HEADER + 4..need];
    for rec in recs.chunks_exact(RECORD) {
        for v in 0..3 {
            let o = 12 + v * 12;
            let x = f32::from_le_bytes(rec[o..o + 4].try_into().unwrap());
            let y = f32::from_le_bytes(rec[o + 4..o + 8].try_into().unwrap());
            let z = f32::from_le_bytes(rec[o + 8..o + 12].try_into().unwrap());
            positions.push([x, y, z]);
        }
    }
    if positions.is_empty() {
        bail!("STL has zero triangles");
    }
    Ok(positions)
}

/// バイト列を ASCII STL として解釈する。
///
/// `facet` / `vertex` / `endfacet` を行単位で読む。キーワードは大文字小文字を問わない。
/// ファイル内の法線は捨て、`vertex` の座標だけを取る。頂点が 3 つ揃わない facet は飛ばす。
/// UTF-8 でなければ Latin-1 として読む。先頭の UTF-8 BOM は無視する。
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
    let text = ascii_stl_text(bytes);
    let mut positions = Vec::new();
    let mut verts: Vec<[f32; 3]> = Vec::with_capacity(3);

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let mut tokens = line.split_ascii_whitespace();
        let Some(kw) = tokens.next() else {
            continue;
        };
        if kw.eq_ignore_ascii_case("facet") {
            verts.clear();
        } else if kw.eq_ignore_ascii_case("vertex") {
            if let Some(v) = parse_xyz(tokens) {
                verts.push(v);
            } else {
                verts.clear();
            }
        } else if kw.eq_ignore_ascii_case("endfacet") {
            if verts.len() == 3 {
                positions.extend_from_slice(&verts);
            }
            verts.clear();
        }
    }
    if verts.len() == 3 {
        positions.extend_from_slice(&verts);
    }
    if positions.is_empty() {
        bail!("STL has zero triangles");
    }
    Ok(positions)
}

fn ascii_stl_text(bytes: &[u8]) -> Cow<'_, str> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => Cow::Owned(bytes.iter().map(|&b| b as char).collect()),
    }
}

fn parse_xyz<'a>(mut it: impl Iterator<Item = &'a str>) -> Option<[f32; 3]> {
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    let z = it.next()?.parse().ok()?;
    Some([x, y, z])
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

fn recenter(mut positions: Vec<[f32; 3]>) -> TriangleSoup {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for p in &positions {
        let v = Vec3::from_array(*p);
        min = min.min(v);
        max = max.max(v);
    }
    let origin = (min + max) * 0.5;
    for p in &mut positions {
        p[0] -= origin.x;
        p[1] -= origin.y;
        p[2] -= origin.z;
    }
    let radius = (max - min).length() * 0.5;
    TriangleSoup {
        positions,
        origin,
        radius: radius.max(1e-6),
    }
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
