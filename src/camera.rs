//! Z-up の直交ターンテーブルカメラ。
//!
//! 拡大縮小は視線距離ではなく投影の高さ（[`Camera::half_height`]）を変える。
//! ホイールはカーソル下の世界点が動かないように注視点をずらす。

use glam::{Mat4, Vec2, Vec3};

const ELEV_MAX: f32 = 89.0_f32.to_radians();

/// 直交投影のターンテーブルカメラ。
///
/// 方位は世界 Z まわり。仰角をクランプして up 軸が裏返らないようにする。
/// 拡大は [`Self::half_height`] で行い、パースは使わない。
#[derive(Clone, Debug)]
pub struct Camera {
    /// 注視点。ロード直後の描画空間では原点。
    pub target: Vec3,
    /// 世界 Z まわりの方位角（ラジアン）。
    pub azimuth: f32,
    /// XY 平面からの仰角（ラジアン）。
    pub elevation: f32,
    /// 注視点からの視線距離。直交スケールには使わず、near/far の配置に使う。
    pub distance: f32,
    /// 画面縦方向の直交ハーフサイズ。ズームの本体。
    pub half_height: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            azimuth: 45_f32.to_radians(),
            elevation: 35_f32.to_radians(),
            distance: 4.0,
            half_height: 1.0,
        }
    }
}

impl Camera {
    /// 原点中心の球が画面に収まるよう注視点・距離・ハーフサイズを入れ直す。
    ///
    /// `aspect` は幅 / 高さ。横長なら縦方向の半径、縦長なら縦を基準に拡げる。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.fit(1.0, 16.0 / 9.0);
    /// assert!(cam.target.length() < 1e-5);
    /// ```
    pub fn fit(&mut self, radius: f32, aspect: f32) {
        self.target = Vec3::ZERO;
        self.distance = (radius * 3.0).max(1e-3);
        let half = radius * 1.15;
        self.half_height = if aspect > 1.0 { half } else { half / aspect.max(1e-4) };
    }

    /// 画面ピクセルのドラッグ量で方位と仰角を更新する。
    ///
    /// 仰角はおよそ ±89° にクランプし、世界 Z が裏返らないようにする。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.rotate(glam::Vec2::new(10.0, 0.0));
    /// ```
    pub fn rotate(&mut self, delta_px: Vec2) {
        self.azimuth -= delta_px.x * 0.008;
        self.elevation = (self.elevation + delta_px.y * 0.008).clamp(-ELEV_MAX, ELEV_MAX);
    }

    /// 画面ピクセルのドラッグ量で注視点をカメラの右・上方向へ動かす。
    ///
    /// 移動量は現在の [`Self::half_height`] とビューポート高さから世界単位へ換算する。
    /// 高さが 1px 以下なら何もしない。
    pub fn pan(&mut self, delta_px: Vec2, viewport: Vec2) {
        if viewport.y <= 1.0 {
            return;
        }
        let (right, up) = self.camera_axes();
        let world_per_px = (self.half_height * 2.0) / viewport.y;
        self.target += (-delta_px.x * right + delta_px.y * up) * world_per_px;
    }

    /// 直交スケールを `factor` 倍し、カーソル下の世界点が動かないよう注視点をずらす。
    ///
    /// `cursor_ndc` は OpenGL 系 NDC（各成分 [-1, 1]、Y は上向き）。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.zoom_at(0.9, glam::Vec2::ZERO, 16.0 / 9.0);
    /// ```
    pub fn zoom_at(&mut self, factor: f32, cursor_ndc: Vec2, aspect: f32) {
        let before = self.world_on_screen(cursor_ndc, aspect);
        self.half_height = (self.half_height * factor).clamp(1e-6, 1e12);
        let after = self.world_on_screen(cursor_ndc, aspect);
        self.target += before - after;
    }

    /// ビュー行列、直交射影、視線オフセットの単位ベクトルを返す。
    ///
    /// カメラの up は世界 Z。`aspect` は幅 / 高さ。
    pub fn view_proj(&self, aspect: f32) -> (Mat4, Mat4, Vec3) {
        let offset = self.eye_offset();
        let eye = self.target + offset;
        let view = Mat4::look_at_rh(eye, self.target, Vec3::Z);
        let half_h = self.half_height;
        let half_w = half_h * aspect.max(1e-4);
        let near = (self.distance * 0.01).max(1e-4);
        let far = (self.distance * 20.0).max(near + 1.0);
        let proj = Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, near, far);
        (view, proj, offset.normalize_or_zero())
    }

    /// カメラからシーンへ向かう単位ベクトルを返す。
    ///
    /// フラットシェードのヘッドライトとして使い、カメラに追従させる。
    pub fn light_dir(&self) -> Vec3 {
        -self.eye_offset().normalize_or_zero()
    }

    fn eye_offset(&self) -> Vec3 {
        let ce = self.elevation.cos();
        Vec3::new(
            ce * self.azimuth.cos(),
            ce * self.azimuth.sin(),
            self.elevation.sin(),
        ) * self.distance
    }

    fn camera_axes(&self) -> (Vec3, Vec3) {
        let offset = self.eye_offset();
        let fwd = (-offset).normalize_or_zero();
        let mut right = fwd.cross(Vec3::Z);
        if right.length_squared() < 1e-8 {
            right = fwd.cross(Vec3::Y);
        }
        let right = right.normalize_or_zero();
        let up = right.cross(fwd).normalize_or_zero();
        (right, up)
    }

    fn world_on_screen(&self, ndc: Vec2, aspect: f32) -> Vec3 {
        let (right, up) = self.camera_axes();
        let half_h = self.half_height;
        let half_w = half_h * aspect.max(1e-4);
        self.target + right * (ndc.x * half_w) + up * (ndc.y * half_h)
    }
}
