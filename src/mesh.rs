//! 描画用の三角形スープ。形式によらずここに集める。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use glam::Vec3;

/// バックグラウンド読み込みの進捗と取り消し。
pub(crate) struct LoadProbe {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl LoadProbe {
    pub fn new(total: u64) -> Self {
        Self {
            done: AtomicU64::new(0),
            total: AtomicU64::new(total.max(1)),
            cancel: AtomicBool::new(false),
        }
    }

    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed).max(1);
        let done = self.done.load(Ordering::Relaxed);
        (done as f32 / total as f32).clamp(0.0, 1.0)
    }

    pub fn report(&self, done: u64) {
        self.done.store(done, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// GPU へ渡す前の三角形スープ。
///
/// 位置だけを持つ。隣接頂点は共有せず、法線はシェーダ側で面ごとに出す。
/// `positions` はファイル座標のまま。[`Self::origin`] は AABB 中心。シェーダが頂点から引く。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_fraction_clamps() {
        let p = LoadProbe::new(100);
        assert_eq!(p.fraction(), 0.0);
        p.report(40);
        assert!((p.fraction() - 0.4).abs() < 1e-5);
        p.report(200);
        assert_eq!(p.fraction(), 1.0);
    }
}
