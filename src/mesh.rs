//! 描画用の三角形スープ。形式によらずここに集める。

use glam::Vec3;

/// GPU へ渡す前の三角形スープ。
///
/// 位置だけを持つ。隣接頂点は共有せず、法線はシェーダ側で面ごとに出す。
/// `positions` はファイル座標のまま。[`Self::origin`] は AABB 中心。GPU へ載せるときに引く。
#[derive(Debug, Clone)]
pub struct TriangleSoup {
    /// 3 頂点で 1 三角形。ファイル座標（中心化しない）。
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
    pub fn triangle_count(&self) -> usize {
        self.positions.len() / 3
    }
}

#[derive(Debug)]
pub(crate) struct ParsedMesh {
    pub positions: Vec<[f32; 3]>,
    pub min: Vec3,
    pub max: Vec3,
}

impl ParsedMesh {
    pub fn empty() -> Self {
        Self {
            positions: Vec::new(),
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
        }
    }

    pub fn absorb(&mut self, other: ParsedMesh) {
        if other.positions.is_empty() {
            return;
        }
        if self.positions.is_empty() {
            *self = other;
            return;
        }
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        self.positions.extend(other.positions);
    }
}

pub(crate) fn bounds_to_soup(positions: Vec<[f32; 3]>, min: Vec3, max: Vec3) -> TriangleSoup {
    let origin = (min + max) * 0.5;
    let radius = (max - min).length() * 0.5;
    TriangleSoup {
        positions,
        origin,
        radius: radius.max(1e-6),
    }
}
