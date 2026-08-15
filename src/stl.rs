//! バイナリ STL を三角形スープへ展開する。
//!
//! `mmap` は読み取りにだけ使い、1 三角形 50 バイトのレコードを GPU 頂点へ直接再解釈しない。
//! 法線は描画時に面から復元する。

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

/// 複数のバイナリ STL をワールド座標のまま結合する。
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
/// let (soup, warnings) = meshpad::stl::load_binary_paths(&["a.stl", "b.stl"])?;
/// let _ = (soup.triangle_count(), warnings.len());
/// ```
pub fn load_binary_paths(paths: &[impl AsRef<Path>]) -> Result<(TriangleSoup, Vec<String>)> {
    let mut all = Vec::new();
    let mut warnings = Vec::new();
    for p in paths {
        let p = p.as_ref();
        match load_binary(p) {
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

/// 1 ファイルを mmap してバイナリ STL として読む。
///
/// 得られた位置はファイル座標のまま（中心化しない）。結合・シフトは [`load_binary_paths`] 側。
///
/// # Errors
///
/// 開けない、マップできない、または [`parse_binary_stl`] が失敗したとき。
pub fn load_binary(path: &Path) -> Result<Vec<[f32; 3]>> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mmap = unsafe { Mmap::map(&file) }.with_context(|| format!("mmap {}", path.display()))?;
    parse_binary_stl(&mmap)
}

/// バイト列をバイナリ STL として解釈する。
///
/// ヘッダ 80 バイトと little-endian の枚数のあと、1 三角形 50 バイト（法線 12、頂点 36、属性 2）とみなして位置だけを展開する。
/// バイト数が `84 + 50 * 枚数` と一致しないとき、ASCII と判定できればその旨で失敗する。
///
/// # Errors
///
/// 短すぎる、切れている、ASCII、または三角形が 0 枚のとき。
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
    if bytes.len() != need {
        if looks_like_ascii(bytes) {
            bail!("ASCII STL is not in this milestone");
        }
        if bytes.len() < need {
            bail!("truncated STL: need {need} bytes, have {}", bytes.len());
        }
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

fn looks_like_ascii(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(80)];
    let Ok(s) = std::str::from_utf8(head) else {
        return false;
    };
    let t = s.trim_start().to_ascii_lowercase();
    t.starts_with("solid") && !bytes[HEADER..].iter().take(64).any(|&b| b == 0)
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
}
