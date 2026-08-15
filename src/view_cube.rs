//! 画面隅のビューキューブ。
//!
//! 面は軸直交、辺は二等分、頂点は等角。上向きはいま上にある面の軸。
//! 描画は egui の 2D、回転はシーンのビュー行列の線形部分だけを使う。

use crate::camera::cube_snap_up;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use glam::{Mat4, Vec3};

/// 等角見たときの六角形の外接半径（頂点円を除く）。
const HEX_RADIUS: f32 = 40.0;
const CORNER_R: f32 = 4.0;
const CORNER_R_HOVER: f32 = 5.5;
const PAD: f32 = 6.0;
const CORNER_HIT: f32 = 11.0;
const EDGE_HIT: f32 = 8.0;

/// ビューキューブ上で選んだ面・辺・頂点。
///
/// `dir` は注視点からカメラが座る位置へ向かう。面は軸方向、辺は二等分、頂点は等角。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubePick {
    /// 注視点からカメラへ向かう単位ベクトル。
    pub dir: Vec3,
    kind: PickKind,
}

impl CubePick {
    /// カメラが座る方向。頂点は角の等角（仰角を落とさない）。
    pub fn snap_dir(self, _view_dir: Vec3, _screen_up: Vec3) -> Vec3 {
        self.dir
    }

    /// スナップ後に保つ上向き。いま画面の上にあるキューブ面の軸。
    pub fn snap_up(self, view_dir: Vec3, screen_up: Vec3) -> Vec3 {
        cube_snap_up(view_dir, screen_up)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickKind {
    Face,
    Edge,
    Corner,
}

fn cube_scale() -> f32 {
    HEX_RADIUS / std::f32::consts::SQRT_2
}

fn max_vertex_radius() -> f32 {
    cube_scale() * 3.0_f32.sqrt()
}

fn draw_extent() -> f32 {
    max_vertex_radius() + CORNER_R_HOVER + PAD
}

fn gizmo_center(viewport: Rect) -> Pos2 {
    let r = draw_extent();
    viewport.left_bottom() + egui::vec2(r, -r)
}

/// クリック判定用の矩形。
///
/// 等角六角形と頂点円が収まる大きさ。一辺は 44px を超える。
pub fn interact_rect(viewport: Rect) -> Rect {
    Rect::from_center_size(gizmo_center(viewport), Vec2::splat(draw_extent() * 2.0))
}

fn vert(i: usize) -> Vec3 {
    Vec3::new(
        if i & 1 != 0 { 1.0 } else { -1.0 },
        if i & 2 != 0 { 1.0 } else { -1.0 },
        if i & 4 != 0 { 1.0 } else { -1.0 },
    )
}

fn project(view: Mat4, viewport: Rect, p: Vec3) -> (Pos2, f32) {
    let v = view.transform_vector3(p);
    let c = gizmo_center(viewport);
    let s = cube_scale();
    (c + egui::vec2(v.x * s, -v.y * s), v.z)
}

fn faces() -> [(Vec3, [Vec3; 4], &'static str, Color32); 6] {
    [
        (
            Vec3::X,
            [
                Vec3::new(1.0, -1.0, -1.0),
                Vec3::new(1.0, 1.0, -1.0),
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(1.0, -1.0, 1.0),
            ],
            "+X",
            Color32::from_rgb(176, 72, 72),
        ),
        (
            -Vec3::X,
            [
                Vec3::new(-1.0, -1.0, 1.0),
                Vec3::new(-1.0, 1.0, 1.0),
                Vec3::new(-1.0, 1.0, -1.0),
                Vec3::new(-1.0, -1.0, -1.0),
            ],
            "-X",
            Color32::from_rgb(120, 48, 48),
        ),
        (
            Vec3::Y,
            [
                Vec3::new(-1.0, 1.0, -1.0),
                Vec3::new(-1.0, 1.0, 1.0),
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(1.0, 1.0, -1.0),
            ],
            "+Y",
            Color32::from_rgb(64, 148, 80),
        ),
        (
            -Vec3::Y,
            [
                Vec3::new(-1.0, -1.0, 1.0),
                Vec3::new(-1.0, -1.0, -1.0),
                Vec3::new(1.0, -1.0, -1.0),
                Vec3::new(1.0, -1.0, 1.0),
            ],
            "-Y",
            Color32::from_rgb(44, 96, 52),
        ),
        (
            Vec3::Z,
            [
                Vec3::new(-1.0, -1.0, 1.0),
                Vec3::new(1.0, -1.0, 1.0),
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(-1.0, 1.0, 1.0),
            ],
            "+Z",
            Color32::from_rgb(72, 104, 188),
        ),
        (
            -Vec3::Z,
            [
                Vec3::new(-1.0, 1.0, -1.0),
                Vec3::new(1.0, 1.0, -1.0),
                Vec3::new(1.0, -1.0, -1.0),
                Vec3::new(-1.0, -1.0, -1.0),
            ],
            "-Z",
            Color32::from_rgb(48, 64, 120),
        ),
    ]
}

fn point_in_tri(p: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let sign = |p1: Pos2, p2: Pos2, p3: Pos2| {
        (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
    };
    let b1 = sign(p, a, b) < 0.0;
    let b2 = sign(p, b, c) < 0.0;
    let b3 = sign(p, c, a) < 0.0;
    b1 == b2 && b2 == b3
}

fn point_in_quad(p: Pos2, q: [Pos2; 4]) -> bool {
    point_in_tri(p, q[0], q[1], q[2]) || point_in_tri(p, q[0], q[2], q[3])
}

fn dist_to_seg(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() < 1e-8 {
        0.0
    } else {
        ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
    };
    (a + ab * t - p).length()
}

fn facing_camera(view: Mat4, normal: Vec3) -> bool {
    // 回転のみのビューでは、カメラ側（look_at の eye 側）が +Z に来る。
    view.transform_vector3(normal).z > 0.02
}

fn axis_normal(axis: usize, positive: bool) -> Vec3 {
    let s = if positive { 1.0 } else { -1.0 };
    let mut n = Vec3::ZERO;
    n[axis] = s;
    n
}

fn vert_has_front_face(view: Mat4, i: usize) -> bool {
    (0..3).any(|axis| facing_camera(view, axis_normal(axis, i & (1 << axis) != 0)))
}

fn edge_has_front_face(view: Mat4, i: usize, j: usize) -> bool {
    (0..3)
        .filter(|axis| (i ^ j) & (1 << axis) == 0)
        .any(|axis| facing_camera(view, axis_normal(axis, i & (1 << axis) != 0)))
}

fn mix_hover(c: Color32) -> Color32 {
    Color32::from_rgb(
        c.r().saturating_add(48),
        c.g().saturating_add(48),
        c.b().saturating_add(48),
    )
}

/// ポインタ位置の面・辺・頂点を返す。
///
/// 優先順位は頂点、辺、面。矩形の外なら `None`。
///
/// # Examples
///
/// ```ignore
/// let hit = view_cube::pick(view, viewport, pointer);
/// ```
pub fn pick(view: Mat4, viewport: Rect, pointer: Pos2) -> Option<CubePick> {
    if !interact_rect(viewport).contains(pointer) {
        return None;
    }

    let mut best_corner: Option<(f32, f32, Vec3)> = None;
    for i in 0..8 {
        if !vert_has_front_face(view, i) {
            continue;
        }
        let d = vert(i);
        let (p, z) = project(view, viewport, d);
        let dist = p.distance(pointer);
        if dist <= CORNER_HIT {
            let key = (dist, -z);
            let better = best_corner
                .map(|(bd, bnegz, _)| key < (bd, bnegz))
                .unwrap_or(true);
            if better {
                best_corner = Some((key.0, key.1, d.normalize()));
            }
        }
    }
    if let Some((_, _, dir)) = best_corner {
        return Some(CubePick {
            dir,
            kind: PickKind::Corner,
        });
    }

    let mut best_edge: Option<(f32, f32, Vec3)> = None;
    for i in 0..8 {
        for bit in [1_usize, 2, 4] {
            let j = i ^ bit;
            if j <= i {
                continue;
            }
            if !edge_has_front_face(view, i, j) {
                continue;
            }
            let a = vert(i);
            let b = vert(j);
            let (pa, za) = project(view, viewport, a);
            let (pb, zb) = project(view, viewport, b);
            let dist = dist_to_seg(pointer, pa, pb);
            if dist <= EDGE_HIT {
                let z = (za + zb) * 0.5;
                let key = (dist, -z);
                let better = best_edge
                    .map(|(bd, bnegz, _)| key < (bd, bnegz))
                    .unwrap_or(true);
                if better {
                    best_edge = Some((key.0, key.1, (a + b).normalize()));
                }
            }
        }
    }
    if let Some((_, _, dir)) = best_edge {
        return Some(CubePick {
            dir,
            kind: PickKind::Edge,
        });
    }

    let mut best_face: Option<(f32, Vec3)> = None;
    for (n, corners, _, _) in faces() {
        if !facing_camera(view, n) {
            continue;
        }
        let pts = corners.map(|c| project(view, viewport, c).0);
        if point_in_quad(pointer, pts) {
            let z = corners
                .iter()
                .map(|c| project(view, viewport, *c).1)
                .sum::<f32>()
                / 4.0;
            if best_face.map(|(bz, _)| z > bz).unwrap_or(true) {
                best_face = Some((z, n));
            }
        }
    }
    best_face.map(|(_, dir)| CubePick {
        dir,
        kind: PickKind::Face,
    })
}

/// ビューキューブを描き、ホバー中の要素を返す。
///
/// クリック処理は呼び出し側。ホバーは強調とカーソル用で、操作の本体はクリック。
///
/// # Examples
///
/// ```ignore
/// let hover = view_cube::paint(ui, rect, view);
/// ```
pub fn paint(ui: &egui::Ui, viewport: Rect, view: Mat4) -> Option<CubePick> {
    let hover = ui
        .input(|i| i.pointer.hover_pos())
        .and_then(|p| pick(view, viewport, p));
    if hover.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    draw(ui, viewport, view, hover);
    hover
}

fn draw(ui: &egui::Ui, viewport: Rect, view: Mat4, hover: Option<CubePick>) {
    let painter = ui.painter();
    let mut faces_front: Vec<(f32, Vec3, [Pos2; 4], &'static str, Color32)> = faces()
        .into_iter()
        .filter(|(n, _, _, _)| facing_camera(view, *n))
        .map(|(n, corners, label, color)| {
            let z = corners
                .iter()
                .map(|c| project(view, viewport, *c).1)
                .sum::<f32>()
                / 4.0;
            let pts = corners.map(|c| project(view, viewport, c).0);
            (z, n, pts, label, color)
        })
        .collect();
    faces_front.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    for (_, n, pts, _, color) in &faces_front {
        let hovered = hover
            .map(|h| h.kind == PickKind::Face && h.dir.dot(*n) > 0.99)
            .unwrap_or(false);
        let fill = if hovered { mix_hover(*color) } else { *color };
        painter.add(egui::Shape::convex_polygon(
            pts.to_vec(),
            fill,
            Stroke::NONE,
        ));
    }

    let mut edges: Vec<(f32, Pos2, Pos2, bool)> = Vec::new();
    for i in 0..8 {
        for bit in [1_usize, 2, 4] {
            let j = i ^ bit;
            if j <= i || !edge_has_front_face(view, i, j) {
                continue;
            }
            let a = vert(i);
            let b = vert(j);
            let (pa, za) = project(view, viewport, a);
            let (pb, zb) = project(view, viewport, b);
            let hovered = hover
                .map(|h| h.kind == PickKind::Edge && (a + b).normalize().dot(h.dir) > 0.99)
                .unwrap_or(false);
            edges.push(((za + zb) * 0.5, pa, pb, hovered));
        }
    }
    edges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    for (_, pa, pb, hovered) in edges {
        let stroke = if hovered {
            Stroke::new(2.5_f32, Color32::from_gray(230))
        } else {
            Stroke::new(1.35_f32, Color32::from_gray(22))
        };
        painter.line_segment([pa, pb], stroke);
    }

    let mut corners: Vec<(f32, Pos2, bool)> = Vec::new();
    for i in 0..8 {
        if !vert_has_front_face(view, i) {
            continue;
        }
        let d = vert(i);
        let (p, z) = project(view, viewport, d);
        let hovered = hover
            .map(|h| h.kind == PickKind::Corner && d.normalize().dot(h.dir) > 0.99)
            .unwrap_or(false);
        corners.push((z, p, hovered));
    }
    corners.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    for (_, p, hovered) in corners {
        let r = if hovered { CORNER_R_HOVER } else { CORNER_R };
        let fill = if hovered {
            Color32::from_gray(240)
        } else {
            Color32::from_gray(220)
        };
        painter.circle_filled(p, r, fill);
        painter.circle_stroke(p, r, Stroke::new(1.0_f32, Color32::from_gray(30)));
    }

    for (_, _, pts, label, _) in &faces_front {
        let c = Pos2::new(
            pts.iter().map(|p| p.x).sum::<f32>() / 4.0,
            pts.iter().map(|p| p.y).sum::<f32>() / 4.0,
        );
        painter.text(
            c,
            egui::Align2::CENTER_CENTER,
            *label,
            egui::FontId::proportional(11.0),
            Color32::from_gray(235),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top_view() -> Mat4 {
        Mat4::look_at_rh(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y)
    }

    fn vp() -> Rect {
        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(400.0, 400.0))
    }

    #[test]
    fn center_of_top_view_picks_plus_z() {
        let view = top_view();
        let viewport = vp();
        let hit = pick(view, viewport, gizmo_center(viewport)).expect("cube center");
        assert!((hit.dir - Vec3::Z).length() < 1e-4);
        assert_eq!(hit.kind, PickKind::Face);
    }

    #[test]
    fn plus_x_plus_y_vertex_picks_corner() {
        let view = top_view();
        let viewport = vp();
        let (p, _) = project(view, viewport, Vec3::new(1.0, 1.0, 1.0));
        let hit = pick(view, viewport, p).expect("corner");
        assert_eq!(hit.kind, PickKind::Corner);
        assert!(hit.dir.x > 0.4 && hit.dir.y > 0.4 && hit.dir.z > 0.4);
        let snap = hit.snap_dir(Vec3::Z, Vec3::Y);
        assert!(snap.x > 0.4 && snap.y > 0.4 && snap.z > 0.4);
    }

    #[test]
    fn outside_interact_rect_is_none() {
        let view = top_view();
        let viewport = vp();
        assert!(pick(view, viewport, Pos2::new(390.0, 10.0)).is_none());
    }

    #[test]
    fn isometric_verts_fit_inside_interact_rect() {
        let view = Mat4::look_at_rh(Vec3::new(1.0, 1.0, 1.0), Vec3::ZERO, Vec3::Z);
        let viewport = vp();
        let inner = interact_rect(viewport).shrink(CORNER_R_HOVER);
        for i in 0..8 {
            let (p, _) = project(view, viewport, vert(i));
            assert!(inner.contains(p), "vertex {i} at {p:?} is clipped");
        }
    }
}
