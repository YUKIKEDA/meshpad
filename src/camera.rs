//! 直交投影の画面基準軌道カメラ。
//!
//! 拡大縮小は視線距離ではなく投影の高さ（[`Camera::half_height`]）を変える。
//! 回転はカメラの right/up まわりで、世界軸はロックしない。
//! ホイールはカーソル下の世界点が動かないように注視点をずらす。

use glam::{Mat3, Mat4, Quat, Vec2, Vec3};

const ROT_SENS: f32 = 0.008;
const FIT_PAD: f32 = 1.15;
/// 全体フィットに対するズームアウト上限。
const ZOOM_OUT_MAX: f32 = 2.0;
const ZOOM_IN_FRAC: f32 = 1e-4;

/// 直交投影の自由軌道カメラ。
///
/// 姿勢はクォータニオン。仰角クランプはせず、真上・真下を通れる。
/// 拡大は [`Self::half_height`] で行い、パースは使わない。
#[derive(Clone, Debug)]
pub struct Camera {
    /// 注視点。ロード直後の描画空間では原点。
    pub target: Vec3,
    /// カメラ空間（+X 右、+Y 上、−Z 前方）から世界へ。
    pub orientation: Quat,
    /// 注視点からの視線距離。直交スケールには使わず、near/far の配置に使う。
    pub distance: f32,
    /// 画面縦方向の直交ハーフサイズ。ズームの本体。
    pub half_height: f32,
    /// 最後にフィットした外接半径。ズーム上限と `F` に使う。
    pub fit_radius: f32,
    /// 世界の上向き。`false` なら +Z（既定）、`true` なら +Y。
    pub y_up: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            orientation: orientation_looking_from(Vec3::new(1.0, 1.0, 1.0), Vec3::Z),
            distance: 4.0,
            half_height: 1.0,
            fit_radius: 1.0,
            y_up: false,
        }
    }
}

/// 外接球が画面に収まる縦ハーフサイズ。
///
/// `aspect` は幅 / 高さ。縦長では横が足りない分だけ縦を広げる。
pub fn fitted_half_height(radius: f32, aspect: f32) -> f32 {
    let half = radius * FIT_PAD;
    let aspect = aspect.max(1e-4);
    half.max(half / aspect)
}

fn preferred_up(dir: Vec3) -> Vec3 {
    if dir.z.abs() > 0.9 {
        Vec3::Y
    } else {
        Vec3::Z
    }
}

/// 画面上向きにいちばん近い符号付き世界軸。
pub fn nearest_up_axis(screen_up: Vec3) -> Vec3 {
    let h = screen_up.normalize_or_zero();
    if h.length_squared() < 0.5 {
        return Vec3::Z;
    }
    let mut best = Vec3::Z;
    let mut best_mag = -1.0_f32;
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        let along = h.dot(axis);
        let mag = along.abs();
        if mag > best_mag + 1e-4 {
            best_mag = mag;
            best = if mag < 1e-6 {
                axis
            } else {
                axis * along.signum()
            };
        }
    }
    if best_mag < 1e-6 {
        Vec3::Z
    } else {
        best
    }
}

/// いま画面の上にあるキューブ面の軸。その軸まわりにヨーする。
///
/// +Y が上の面なら Y。視線と平行な面しか無いときは [`nearest_up_axis`]。
pub fn cube_snap_up(view_dir: Vec3, screen_up: Vec3) -> Vec3 {
    let view = view_dir.normalize_or_zero();
    let up = screen_up.normalize_or_zero();
    let mut best: Option<(f32, Vec3)> = None;
    for axis in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
        if view.dot(axis) <= 0.02 {
            continue;
        }
        let height = up.dot(axis);
        let better = best.map(|(h, _)| height > h + 1e-4).unwrap_or(true);
        if better {
            best = Some((height, axis));
        }
    }
    if let Some((_, axis)) = best {
        if view.cross(axis).length_squared() > 1e-4 {
            return axis;
        }
    }
    nearest_up_axis(up)
}

/// `dir` 方向から原点を見る姿勢。
///
/// ±Z（真上・真下）は世界 Y を画面上向きにする。それ以外は `hint_up` を画面垂直に保つ。
fn orientation_looking_from(dir: Vec3, hint_up: Vec3) -> Quat {
    let dir = dir.normalize_or_zero();
    if dir.length_squared() < 0.5 {
        return Quat::IDENTITY;
    }
    let hint = hint_up.normalize_or_zero();
    let mut up = if dir.z.abs() > 0.9 {
        preferred_up(dir)
    } else if hint.length_squared() > 0.5 && dir.cross(hint).length_squared() > 1e-4 {
        hint
    } else {
        preferred_up(dir)
    };
    let f = -dir;
    let mut s = f.cross(up);
    if s.length_squared() < 1e-8 {
        up = if up.dot(Vec3::Y).abs() > 0.9 {
            Vec3::X
        } else {
            Vec3::Y
        };
        s = f.cross(up);
    }
    let s = s.normalize_or_zero();
    let u = s.cross(f).normalize_or_zero();
    Quat::from_mat3(&Mat3::from_cols(s, u, dir))
}

impl Camera {
    /// 原点中心の球が画面に収まるよう注視点・距離・ハーフサイズを入れ直す。
    ///
    /// 向きは変えない。`aspect` は幅 / 高さ。呼び出し側は描画に使う実アスペクトを渡す。
    /// ズームアウト上限はこのフィットサイズの約 2 倍になる。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.fit(1.0, 0.5);
    /// assert!(cam.half_height > 1.15);
    /// ```
    pub fn fit(&mut self, radius: f32, aspect: f32) {
        self.target = Vec3::ZERO;
        self.fit_radius = radius.max(1e-6);
        self.distance = (self.fit_radius * 3.0).max(1e-3);
        self.half_height = fitted_half_height(self.fit_radius, aspect);
    }

    /// 指定方向から原点を見る姿勢に切り替え、続けて全体フィットする。
    ///
    /// ビューキューブの面・辺・頂点クリック用。`dir` はカメラが座る世界方向。
    /// 頂点は角の等角。画面上向きは、いま上にあるキューブ面の軸。
    /// 画面上の動きは [`CameraTween`] が短い時間で補間する。この関数自体は最終姿勢を即時に入れる。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.snap_and_fit(glam::Vec3::X, 1.0, 1.0);
    /// ```
    pub fn snap_and_fit(&mut self, dir: Vec3, radius: f32, aspect: f32) {
        self.look_from(dir);
        self.fit(radius, aspect);
    }

    /// 世界の上向き（[`Self::y_up`]）。
    pub fn world_up(&self) -> Vec3 {
        if self.y_up {
            Vec3::Y
        } else {
            Vec3::Z
        }
    }

    /// 指定方向から原点を見る。
    ///
    /// ズームと注視点は変えない。[`Self::world_up`] を画面上向きにする。
    /// 視線がそれと平行なときだけ、もう一方の軸を画面上向きにする。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.look_from(glam::Vec3::X);
    /// ```
    pub fn look_from(&mut self, dir: Vec3) {
        self.look_from_up(dir, self.world_up());
    }

    /// 視線方向は保ち、[`Self::world_up`] が画面上に来るよう姿勢だけ入れ直す。
    pub fn apply_world_up(&mut self) {
        let dir = self.eye_offset();
        self.look_from(dir);
    }

    /// 指定方向から原点を見る。`hint_up` を画面垂直に保つ。
    ///
    /// 視線と平行なときは [`look_from`] と同じフォールバック。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.look_from_up(glam::Vec3::X, glam::Vec3::Y);
    /// ```
    pub fn look_from_up(&mut self, dir: Vec3, hint_up: Vec3) {
        self.orientation = orientation_looking_from(dir, hint_up);
    }

    /// 画面ピクセルのドラッグ量で、カメラの up / right まわりに回す。
    ///
    /// 世界軸は固定しない。仰角クランプも無いので、ポールを越えて裏側へ行ける。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.rotate(glam::Vec2::new(10.0, 400.0));
    /// ```
    pub fn rotate(&mut self, delta_px: Vec2) {
        if delta_px.length_squared() < 1e-12 {
            return;
        }
        let right = (self.orientation * Vec3::X).normalize_or_zero();
        let up = (self.orientation * Vec3::Y).normalize_or_zero();
        let q = Quat::from_axis_angle(up, -delta_px.x * ROT_SENS)
            * Quat::from_axis_angle(right, -delta_px.y * ROT_SENS);
        self.orientation = (q * self.orientation).normalize();
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
    /// 結果のハーフサイズは [`Self::zoom_limits`] に収める。
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut cam = Camera::default();
    /// cam.zoom_at(0.9, glam::Vec2::ZERO, 16.0 / 9.0);
    /// ```
    pub fn zoom_at(&mut self, factor: f32, cursor_ndc: Vec2, aspect: f32) {
        let before = self.world_on_screen(cursor_ndc, aspect);
        self.apply_zoom_factor(factor, aspect);
        let after = self.world_on_screen(cursor_ndc, aspect);
        self.target += before - after;
    }

    /// カーソル位置が無いときの等比ズーム。
    pub fn apply_zoom_factor(&mut self, factor: f32, aspect: f32) {
        let (zmin, zmax) = self.zoom_limits(aspect);
        self.half_height = (self.half_height * factor).clamp(zmin, zmax);
    }

    /// 現在アスペクトでのズーム下限・上限。
    ///
    /// 上限はフィットサイズの 2 倍。下限は半径のごく一部で、三角形まで寄れる。
    pub fn zoom_limits(&self, aspect: f32) -> (f32, f32) {
        let fit = fitted_half_height(self.fit_radius, aspect);
        let zmax = (fit * ZOOM_OUT_MAX).max(fit);
        let zmin = (self.fit_radius * ZOOM_IN_FRAC).max(1e-6);
        (zmin.min(zmax), zmax)
    }

    /// ビュー行列、直交射影、視線オフセットの単位ベクトルを返す。
    ///
    /// カメラの up は姿勢の +Y。`aspect` は幅 / 高さ。
    pub fn view_proj(&self, aspect: f32) -> (Mat4, Mat4, Vec3) {
        let offset = self.eye_offset();
        let eye = self.target + offset;
        let up = (self.orientation * Vec3::Y).normalize_or_zero();
        let view = Mat4::look_at_rh(eye, self.target, up);
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

    /// 注視点からカメラへ向かうオフセット。
    pub fn eye_offset(&self) -> Vec3 {
        self.orientation * (Vec3::Z * self.distance)
    }

    fn camera_axes(&self) -> (Vec3, Vec3) {
        let right = (self.orientation * Vec3::X).normalize_or_zero();
        let up = (self.orientation * Vec3::Y).normalize_or_zero();
        (right, up)
    }

    fn world_on_screen(&self, ndc: Vec2, aspect: f32) -> Vec3 {
        let (right, up) = self.camera_axes();
        let half_h = self.half_height;
        let half_w = half_h * aspect.max(1e-4);
        self.target + right * (ndc.x * half_w) + up * (ndc.y * half_h)
    }
}

const TWEEN_SEC: f32 = 0.28;

fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
    }
}

/// 姿勢・注視点・ズームを短く補間する。
///
/// ビューキューブと `F` の切り替えで、向きが一瞬で飛ばないようにする。
/// 所要時間は約 0.28 秒。ease-in-out。クォータニオンは最短経路。
///
/// # Examples
///
/// ```ignore
/// let mut tween = CameraTween::toward(&from, &goal);
/// while tween.tick(&mut cam, 1.0 / 60.0) {}
/// ```
pub struct CameraTween {
    from_ori: Quat,
    from_target: Vec3,
    from_half: f32,
    from_dist: f32,
    to_ori: Quat,
    to_target: Vec3,
    to_half: f32,
    to_dist: f32,
    to_fit_radius: f32,
    elapsed: f32,
}

impl CameraTween {
    /// `from` から `to` へ向かう補間を始める。
    ///
    /// 姿勢もズームもほぼ同じなら `None`（即時適用で足りる）。
    pub fn toward(from: &Camera, to: &Camera) -> Option<Self> {
        let mut to_ori = to.orientation;
        if from.orientation.dot(to_ori) < 0.0 {
            to_ori = -to_ori;
        }
        let same_ori = from.orientation.dot(to_ori).abs() > 0.9997;
        let same_zoom = (from.half_height - to.half_height).abs() < to.half_height * 1e-3 + 1e-6;
        let same_target = (from.target - to.target).length() < to.fit_radius * 1e-4 + 1e-5;
        if same_ori && same_zoom && same_target {
            return None;
        }
        Some(Self {
            from_ori: from.orientation,
            from_target: from.target,
            from_half: from.half_height,
            from_dist: from.distance,
            to_ori,
            to_target: to.target,
            to_half: to.half_height,
            to_dist: to.distance,
            to_fit_radius: to.fit_radius,
            elapsed: 0.0,
        })
    }

    /// `dt` 秒進める。まだ動いていれば `true`。
    ///
    /// 終わったら `cam` は `to` と一致する。途中でも `fit_radius` はゴール側にしてズーム上限を保つ。
    pub fn tick(&mut self, cam: &mut Camera, dt: f32) -> bool {
        self.elapsed += dt.max(0.0);
        let u = (self.elapsed / TWEEN_SEC).clamp(0.0, 1.0);
        let e = ease_in_out_cubic(u);
        cam.orientation = self.from_ori.slerp(self.to_ori, e);
        cam.target = self.from_target.lerp(self.to_target, e);
        cam.half_height = self.from_half + (self.to_half - self.from_half) * e;
        cam.distance = self.from_dist + (self.to_dist - self.from_dist) * e;
        cam.fit_radius = self.to_fit_radius;
        u < 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers_radius(cam: &Camera, radius: f32, aspect: f32) -> bool {
        let pad = radius * FIT_PAD;
        let half_h = cam.half_height;
        let half_w = half_h * aspect;
        half_h + 1e-5 >= pad && half_w + 1e-5 >= pad
    }

    #[test]
    fn fit_landscape_uses_vertical_extent() {
        let mut cam = Camera::default();
        let aspect = 16.0 / 9.0;
        cam.fit(10.0, aspect);
        assert!((cam.half_height - 11.5).abs() < 1e-5);
        assert!(covers_radius(&cam, 10.0, aspect));
    }

    #[test]
    fn fit_portrait_enlarges_half_height() {
        let mut cam = Camera::default();
        let aspect = 0.5;
        cam.fit(10.0, aspect);
        assert!(cam.half_height > 11.5);
        assert!((cam.half_height * aspect - 11.5).abs() < 1e-5);
        assert!(covers_radius(&cam, 10.0, aspect));
    }

    #[test]
    fn fit_keeps_orientation() {
        let mut cam = Camera::default();
        let before = cam.orientation;
        cam.fit(2.0, 1.0);
        assert!((cam.orientation.dot(before) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rotate_can_pass_the_pole() {
        let mut cam = Camera::default();
        cam.rotate(Vec2::new(0.0, 200.0));
        let d1 = cam.eye_offset().normalize();
        cam.rotate(Vec2::new(0.0, 200.0));
        let d2 = cam.eye_offset().normalize();
        assert!(d1.is_finite() && d2.is_finite());
        assert!(
            (d1 - d2).length() > 0.1,
            "仰角クランプがあると2回目の縦ドラッグが止まる"
        );
    }

    #[test]
    fn look_from_x_places_camera_on_x() {
        let mut cam = Camera::default();
        cam.distance = 1.0;
        cam.look_from(Vec3::X);
        let dir = cam.eye_offset().normalize();
        assert!((dir - Vec3::X).length() < 1e-4);
    }

    #[test]
    fn y_up_look_from_x_aligns_y_with_screen_up() {
        let mut cam = Camera::default();
        cam.distance = 1.0;
        cam.look_from(Vec3::X);
        assert!((cam.orientation * Vec3::Y).dot(Vec3::Z) > 0.99);
        cam.y_up = true;
        cam.apply_world_up();
        assert!((cam.eye_offset().normalize() - Vec3::X).length() < 1e-3);
        assert!((cam.orientation * Vec3::Y).dot(Vec3::Y) > 0.99);
    }

    #[test]
    fn zoom_out_stops_near_twice_fit() {
        let mut cam = Camera::default();
        cam.fit(10.0, 1.0);
        let fit = cam.half_height;
        cam.apply_zoom_factor(100.0, 1.0);
        let (_, zmax) = cam.zoom_limits(1.0);
        assert!((cam.half_height - zmax).abs() < 1e-4);
        assert!((zmax - fit * ZOOM_OUT_MAX).abs() < 1e-4);
    }

    #[test]
    fn snap_and_fit_orients_and_covers() {
        let mut cam = Camera::default();
        cam.snap_and_fit(Vec3::Z, 4.0, 1.0);
        let dir = cam.eye_offset().normalize();
        assert!((dir - Vec3::Z).length() < 1e-3);
        assert!(covers_radius(&cam, 4.0, 1.0));
        assert!(cam.target.length() < 1e-5);
    }

    #[test]
    fn y_up_top_view_vertex_is_iso_keeping_y_unrolled() {
        let mut cam = Camera::default();
        cam.look_from(Vec3::Z);
        let up = cube_snap_up(cam.eye_offset(), cam.orientation * Vec3::Y);
        assert!((up - Vec3::Y).length() < 1e-4);
        let dir = Vec3::new(1.0, 1.0, 1.0).normalize();
        assert!(dir.x > 0.4 && dir.y > 0.4 && dir.z > 0.4);
        cam.look_from_up(dir, up);
        let eye = cam.eye_offset().normalize();
        assert!(
            eye.x > 0.4 && eye.y > 0.4 && eye.z > 0.4,
            "iso elevation; eye={eye:?}"
        );
        let right = (cam.orientation * Vec3::X).normalize();
        assert!(
            right.dot(Vec3::Y).abs() < 1e-3,
            "Y-up iso should not roll; right={right:?}"
        );
    }

    #[test]
    fn z_up_side_view_vertex_is_iso_keeping_z_unrolled() {
        let mut cam = Camera::default();
        cam.look_from_up(Vec3::X, Vec3::Z);
        let up = cube_snap_up(cam.eye_offset(), cam.orientation * Vec3::Y);
        assert!((up - Vec3::Z).length() < 1e-4);
        let dir = Vec3::new(1.0, 1.0, 1.0).normalize();
        cam.look_from_up(dir, up);
        let eye = cam.eye_offset().normalize();
        assert!(
            eye.x > 0.4 && eye.y > 0.4 && eye.z > 0.4,
            "iso elevation; eye={eye:?}"
        );
        let right = (cam.orientation * Vec3::X).normalize();
        assert!(
            right.dot(Vec3::Z).abs() < 1e-3,
            "Z-up iso should keep Z screen-vertical; right={right:?}"
        );
    }

    #[test]
    fn yz_edge_vertex_keeps_y_face_as_roof() {
        let mut cam = Camera::default();
        cam.look_from_up(Vec3::new(0.0, 1.0, -1.0), Vec3::Z);
        let up = cube_snap_up(cam.eye_offset(), cam.orientation * Vec3::Y);
        assert!(up.dot(Vec3::Y) > 0.99, "top visible face is +Y; up={up:?}");
        cam.look_from_up(Vec3::new(-1.0, 1.0, -1.0).normalize(), up);
        let eye = cam.eye_offset().normalize();
        assert!(
            eye.x < -0.4 && eye.y > 0.4 && eye.z < -0.4,
            "iso; eye={eye:?}"
        );
        let right = (cam.orientation * Vec3::X).normalize();
        assert!(
            right.dot(Vec3::Y).abs() < 1e-3,
            "Y-up iso must not roll toward Z; right={right:?}"
        );
    }

    #[test]
    fn yz_edge_yx_ridge_stays_y_up_stacked() {
        let mut cam = Camera::default();
        cam.look_from_up(Vec3::new(0.0, 1.0, -1.0), Vec3::Z);
        let up = cube_snap_up(cam.eye_offset(), cam.orientation * Vec3::Y);
        cam.look_from_up(Vec3::new(-1.0, 1.0, 0.0), up);
        let right = (cam.orientation * Vec3::X).normalize();
        assert!(
            right.dot(Vec3::Z).abs() > 0.9,
            "YX edge with Y-up is stacked; Z is screen-horizontal; right={right:?}"
        );
        let screen_up = (cam.orientation * Vec3::Y).normalize();
        assert!(
            screen_up.dot(Vec3::new(1.0, 1.0, 0.0).normalize()) > 0.9
                || screen_up.dot(Vec3::Y) > 0.5,
            "Y stays toward screen-up; up={screen_up:?}"
        );
    }

    #[test]
    fn snap_from_x_face_keeps_z_as_screen_up() {
        let mut cam = Camera::default();
        cam.orientation = Quat::from_mat3(&Mat3::from_cols(Vec3::Y, Vec3::Z, Vec3::X));
        cam.look_from(Vec3::new(1.0, 0.0, -1.0));
        let dir = cam.eye_offset().normalize();
        assert!(
            dir.y.abs() < 1e-3,
            "ZX edge view should sit in the ZX plane"
        );
        let right = (cam.orientation * Vec3::X).normalize();
        assert!(
            right.dot(Vec3::Z).abs() < 1e-3,
            "ZX edge should be screen-horizontal; right={right:?}"
        );
    }

    #[test]
    fn zx_edge_view_has_level_horizon() {
        let mut cam = Camera::default();
        cam.look_from(Vec3::new(-1.0, 0.0, -1.0));
        let up = (cam.orientation * Vec3::Y).normalize();
        assert!(up.dot(Vec3::Z) > 0.7);
        let right = (cam.orientation * Vec3::X).normalize();
        assert!(right.dot(Vec3::Z).abs() < 1e-3);
        assert!(right.y.abs() > 0.7, "looking in ZX, screen-right is ±Y");
    }

    #[test]
    fn tween_reaches_snap_pose() {
        let from = Camera::default();
        let mut goal = from.clone();
        goal.snap_and_fit(Vec3::X, 3.0, 1.0);
        let mut tween = CameraTween::toward(&from, &goal).expect("needs motion");
        let mut cam = from;
        while tween.tick(&mut cam, 1.0 / 60.0) {}
        assert!((cam.eye_offset().normalize() - Vec3::X).length() < 1e-3);
        assert!((cam.half_height - goal.half_height).abs() < 1e-4);
        assert!(cam.target.length() < 1e-5);
    }

    #[test]
    fn tween_skips_when_already_there() {
        let cam = Camera::default();
        assert!(CameraTween::toward(&cam, &cam).is_none());
    }
}
