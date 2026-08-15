//! eframe 上の Meshpad ウィンドウ。
//!
//! 起動引数・ドロップ・「ファイル → 開く」のファイル列は、いずれも新しいシーンになる。
//! `F` で全体フィット。左下のビューキューブで向き＋ズームを揃える。

use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{self, Color32, PointerButton, RichText, Sense, Stroke, ViewportCommand};
use glam::Vec2;

use crate::camera::{Camera, CameraTween};
use crate::gpu::{Renderer, SceneGpu};
use crate::icon;
use crate::load;
use crate::open;
use crate::view_cube;

/// Meshpad のウィンドウ本体。
///
/// ウィンドウ枠に File メニュー、下のステータス、ビュー全面の 3D を持つ。
pub struct MeshpadApp {
    renderer: Renderer,
    scene: Option<SceneGpu>,
    camera: Camera,
    /// 次フレームの実ビューポートで [`Camera::fit`] する半径。
    pending_fit: Option<f32>,
    tween: Option<CameraTween>,
    status: String,
    warnings: Vec<String>,
    title_file: Option<String>,
    title_icon: egui::TextureHandle,
}

impl MeshpadApp {
    /// wgpu レンダラを初期化し、パスがあればメッシュを載せる。
    ///
    /// `initial_paths` が空なら空シーンのまま起動する。ダイアログは出さない。
    ///
    /// # Panics
    ///
    /// eframe が wgpu バックエンド無しで作られたとき。本アプリは DX12 前提。
    pub fn new(cc: &eframe::CreationContext<'_>, initial_paths: Vec<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .expect("Meshpad requires the wgpu renderer");
        let mut renderer = Renderer::new(&rs.device);
        {
            let mut wgpu_renderer = rs.renderer.write();
            renderer.sync_egui_tex(&rs.device, &mut wgpu_renderer);
        }
        let mut app = Self {
            renderer,
            scene: None,
            camera: Camera::default(),
            pending_fit: None,
            tween: None,
            status: String::new(),
            warnings: Vec::new(),
            title_file: None,
            title_icon: cc.egui_ctx.load_texture(
                "meshpad_icon",
                icon::title_color_image(),
                egui::TextureOptions::NEAREST,
            ),
        };
        if !initial_paths.is_empty() {
            app.open_paths(&rs.device, &initial_paths);
        }
        app
    }

    fn open_paths(&mut self, device: &eframe::egui_wgpu::wgpu::Device, paths: &[PathBuf]) {
        let (files, mut warnings) = open::expand_open_inputs(paths);
        self.tween = None;

        if files.is_empty() {
            self.scene = None;
            self.pending_fit = None;
            self.title_file = None;
            self.warnings = warnings;
            self.status = "no mesh could be opened".into();
            return;
        }

        let started = Instant::now();
        match load::load_paths(&files) {
            Ok((soup, load_warnings)) => {
                let load_ms = started.elapsed().as_secs_f64() * 1000.0;
                warnings.extend(load_warnings);
                self.warnings = warnings;
                self.scene = Some(SceneGpu::from_soup(device, &soup));
                self.pending_fit = Some(soup.radius);
                self.title_file = files.first().map(|p| file_label(p, files.len()));
                self.status = format_load_status(soup.triangle_count(), load_ms);
            }
            Err(e) => {
                self.scene = None;
                self.pending_fit = None;
                self.title_file = None;
                self.warnings = warnings;
                self.status = e.to_string();
            }
        }
    }

    fn try_open_dialog(&mut self, frame: &eframe::Frame) {
        let picked = rfd::FileDialog::new()
            .set_title("Open")
            .add_filter("Mesh", &["stl", "nas", "nastran"])
            .add_filter("STL", &["stl"])
            .add_filter("NAS", &["nas", "nastran"])
            .pick_files();
        if let Some(files) = picked {
            self.open_if_device(frame, &files);
        }
    }

    fn open_if_device(&mut self, frame: &eframe::Frame, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        if let Some(rs) = frame.wgpu_render_state() {
            self.open_paths(&rs.device, paths);
        }
    }
}

// mmap・パース・AABB までの時間。GPU アップロードは含めない。
fn format_load_status(triangles: usize, load_ms: f64) -> String {
    if load_ms < 10.0 {
        format!("{triangles} triangles  {load_ms:.1}ms")
    } else {
        format!("{triangles} triangles  {load_ms:.0}ms")
    }
}

fn file_label(path: &Path, count: usize) -> String {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("mesh")
        .to_string();
    if count > 1 {
        format!("{name} +{}", count - 1)
    } else {
        name
    }
}

fn dropped_paths(ctx: &egui::Context) -> Vec<PathBuf> {
    ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.clone())
            .collect()
    })
}

fn hover_dropping(ctx: &egui::Context) -> bool {
    ctx.input(|i| !i.raw.hovered_files.is_empty())
}

const TITLE_H: f32 = 32.0;
const RESIZE_PAD: f32 = 5.0;
const CAPTION_W: f32 = 46.0;

fn title_bar(ctx: &egui::Context, want_open: &mut bool, title_icon: &egui::TextureHandle) {
    egui::TopBottomPanel::top("title")
        .exact_height(TITLE_H)
        .frame(
            egui::Frame::NONE
                .fill(Color32::from_rgb(22, 22, 24))
                .inner_margin(egui::Margin::ZERO),
        )
        .show(ctx, |ui| {
            let bar = ui.max_rect();
            let btn_band = bar.with_min_x(bar.right() - CAPTION_W * 3.0);
            let drag_rect = bar
                .with_min_x(bar.left() + 210.0)
                .with_max_x(btn_band.left());
            let drag = ui.interact(
                drag_rect,
                egui::Id::new("title_drag"),
                Sense::click_and_drag(),
            );
            if drag.double_clicked() {
                let maxed = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(ViewportCommand::Maximized(!maxed));
            }
            if drag.drag_started_by(PointerButton::Primary) {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }

            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(bar)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.add_space(10.0);
                    paint_app_mark(ui, title_icon);
                    ui.add_space(6.0);
                    ui.label(RichText::new("Meshpad").color(Color32::from_gray(210)));
                    ui.add_space(6.0);
                    ui.menu_button("File", |ui| {
                        if ui.button("Open...    Ctrl+O").clicked() {
                            ui.close();
                            *want_open = true;
                        }
                    });
                },
            );

            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(bar)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    window_buttons(ui);
                },
            );
        });
}

fn paint_app_mark(ui: &mut egui::Ui, icon: &egui::TextureHandle) {
    ui.add(
        egui::Image::new(icon)
            .fit_to_exact_size(egui::vec2(16.0, 16.0))
            .sense(Sense::hover()),
    );
}

fn window_buttons(ui: &mut egui::Ui) {
    let h = TITLE_H;
    let w = CAPTION_W;
    let close = caption_btn(ui, CaptionIcon::Close, w, h, Color32::from_rgb(196, 43, 28));
    if close.clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
    let maxed = ui.input(|i| i.viewport().maximized.unwrap_or(false));
    let max_kind = if maxed {
        CaptionIcon::Restore
    } else {
        CaptionIcon::Maximize
    };
    let maximize = caption_btn(ui, max_kind, w, h, Color32::from_rgb(52, 52, 54));
    if maximize.clicked() {
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maxed));
    }
    let minimize = caption_btn(
        ui,
        CaptionIcon::Minimize,
        w,
        h,
        Color32::from_rgb(52, 52, 54),
    );
    if minimize.clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

#[derive(Clone, Copy)]
enum CaptionIcon {
    Minimize,
    Maximize,
    Restore,
    Close,
}

fn caption_btn(
    ui: &mut egui::Ui,
    kind: CaptionIcon,
    w: f32,
    h: f32,
    hover: Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(w, h), Sense::click());
    let close = matches!(kind, CaptionIcon::Close);
    let fill = if response.hovered() {
        hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    let fg = if response.hovered() && close {
        Color32::WHITE
    } else {
        Color32::from_gray(220)
    };
    paint_caption_icon(ui.painter(), rect, kind, fg);
    response
}

fn paint_caption_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    kind: CaptionIcon,
    color: Color32,
) {
    let ppp = painter.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let c = egui::pos2(snap(rect.center().x), snap(rect.center().y));
    let stroke = Stroke::new(1.0_f32, color);
    match kind {
        CaptionIcon::Minimize => {
            let w = 10.0;
            painter.line_segment(
                [
                    egui::pos2(snap(c.x - w * 0.5), c.y),
                    egui::pos2(snap(c.x + w * 0.5), c.y),
                ],
                stroke,
            );
        }
        CaptionIcon::Maximize => {
            let s = 10.0;
            let r = egui::Rect::from_min_size(
                egui::pos2(snap(c.x - s * 0.5), snap(c.y - s * 0.5)),
                egui::vec2(s, s),
            );
            painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
        }
        CaptionIcon::Restore => {
            let s = 8.0;
            let back = egui::Rect::from_min_size(
                egui::pos2(snap(c.x - 2.0), snap(c.y - 5.0)),
                egui::vec2(s, s),
            );
            let front = egui::Rect::from_min_size(
                egui::pos2(snap(c.x - 5.0), snap(c.y - 2.0)),
                egui::vec2(s, s),
            );
            painter.rect_stroke(back, 0.0, stroke, egui::StrokeKind::Inside);
            painter.rect_filled(front, 0.0, Color32::from_rgb(22, 22, 24));
            painter.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Inside);
        }
        CaptionIcon::Close => {
            let s = 5.0;
            painter.line_segment(
                [
                    egui::pos2(snap(c.x - s), snap(c.y - s)),
                    egui::pos2(snap(c.x + s), snap(c.y + s)),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(snap(c.x + s), snap(c.y - s)),
                    egui::pos2(snap(c.x - s), snap(c.y + s)),
                ],
                stroke,
            );
        }
    }
}

fn window_resize_borders(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.pointer_latest_pos() else {
        return;
    };
    let screen = ctx.screen_rect();
    let l = pos.x - screen.left() <= RESIZE_PAD;
    let r = screen.right() - pos.x <= RESIZE_PAD;
    let t = pos.y - screen.top() <= RESIZE_PAD;
    let b = screen.bottom() - pos.y <= RESIZE_PAD;
    let dir = match (l, r, t, b) {
        (true, false, true, false) => Some(egui::viewport::ResizeDirection::NorthWest),
        (false, true, true, false) => Some(egui::viewport::ResizeDirection::NorthEast),
        (true, false, false, true) => Some(egui::viewport::ResizeDirection::SouthWest),
        (false, true, false, true) => Some(egui::viewport::ResizeDirection::SouthEast),
        (true, false, false, false) => Some(egui::viewport::ResizeDirection::West),
        (false, true, false, false) => Some(egui::viewport::ResizeDirection::East),
        (false, false, true, false) => Some(egui::viewport::ResizeDirection::North),
        (false, false, false, true) => Some(egui::viewport::ResizeDirection::South),
        _ => None,
    };
    let Some(dir) = dir else {
        return;
    };
    ctx.set_cursor_icon(match dir {
        egui::viewport::ResizeDirection::North | egui::viewport::ResizeDirection::South => {
            egui::CursorIcon::ResizeVertical
        }
        egui::viewport::ResizeDirection::East | egui::viewport::ResizeDirection::West => {
            egui::CursorIcon::ResizeHorizontal
        }
        egui::viewport::ResizeDirection::NorthEast | egui::viewport::ResizeDirection::SouthWest => {
            egui::CursorIcon::ResizeNeSw
        }
        egui::viewport::ResizeDirection::NorthWest | egui::viewport::ResizeDirection::SouthEast => {
            egui::CursorIcon::ResizeNwSe
        }
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}

impl eframe::App for MeshpadApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("Meshpad".into()));
        window_resize_borders(ctx);

        let mut want_open =
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O));
        title_bar(ctx, &mut want_open, &self.title_icon);

        egui::TopBottomPanel::bottom("status")
            .exact_height(22.0)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(18, 18, 20))
                    .inner_margin(egui::Margin::symmetric(10, 0)),
            )
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let mut need_sep = false;
                    if let Some(name) = &self.title_file {
                        ui.label(RichText::new(name).color(Color32::from_gray(200)));
                        need_sep = true;
                    }
                    if !self.status.is_empty() {
                        if need_sep {
                            ui.separator();
                        }
                        ui.label(RichText::new(&self.status).color(Color32::from_gray(160)));
                        need_sep = true;
                    }
                    for w in &self.warnings {
                        if need_sep {
                            ui.separator();
                        }
                        ui.label(RichText::new(w).color(Color32::from_rgb(220, 160, 80)));
                        need_sep = true;
                    }
                });
            });

        if want_open {
            self.try_open_dialog(frame);
        } else {
            let dropped = dropped_paths(ctx);
            if !dropped.is_empty() {
                self.open_if_device(frame, &dropped);
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(26, 26, 28)))
            .show(ctx, |ui| {
                let avail = ui.available_size();
                let (rect, response) = ui.allocate_exact_size(avail, Sense::click_and_drag());
                let aspect = (rect.width() / rect.height().max(1.0)).max(0.05);
                if let Some(radius) = self.pending_fit.take() {
                    self.camera.fit(radius, aspect);
                    self.tween = None;
                }
                if let Some(scene) = &self.scene {
                    self.camera.distance = self.camera.distance.max(scene.radius * 3.0);
                }

                if ui.input(|i| i.key_pressed(egui::Key::F)) {
                    if let Some(scene) = &self.scene {
                        let mut goal = self.camera.clone();
                        goal.fit(scene.radius, aspect);
                        self.tween = CameraTween::toward(&self.camera, &goal);
                        if self.tween.is_none() {
                            self.camera = goal;
                        }
                    }
                }

                let orbiting = response.dragged_by(PointerButton::Primary);
                let panning = response.dragged_by(PointerButton::Secondary)
                    || response.dragged_by(PointerButton::Middle);
                if orbiting {
                    self.tween = None;
                    self.camera
                        .rotate(Vec2::new(response.drag_delta().x, response.drag_delta().y));
                }
                if panning {
                    self.tween = None;
                    self.camera.pan(
                        Vec2::new(response.drag_delta().x, response.drag_delta().y),
                        Vec2::new(rect.width(), rect.height()),
                    );
                }
                if response.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 0.0 {
                        self.tween = None;
                        let factor = (0.0015 * -scroll).exp();
                        if let Some(pos) = response.hover_pos() {
                            let ndc = Vec2::new(
                                ((pos.x - rect.left()) / rect.width()) * 2.0 - 1.0,
                                1.0 - ((pos.y - rect.top()) / rect.height()) * 2.0,
                            );
                            self.camera.zoom_at(factor, ndc, aspect);
                        } else {
                            self.camera.apply_zoom_factor(factor, aspect);
                        }
                    }
                }

                if self.scene.is_some() && response.clicked() {
                    let (view, _, _) = self.camera.view_proj(aspect);
                    if let Some(pos) = response.interact_pointer_pos() {
                        if let Some(hit) = view_cube::pick(view, rect, pos) {
                            if let Some(scene) = &self.scene {
                                let screen_up = self.camera.orientation * glam::Vec3::Y;
                                let view_dir = self.camera.eye_offset();
                                let mut goal = self.camera.clone();
                                let dir = hit.snap_dir(view_dir, screen_up);
                                let up = hit.snap_up(view_dir, screen_up);
                                goal.look_from_up(dir, up);
                                goal.fit(scene.radius, aspect);
                                self.tween = CameraTween::toward(&self.camera, &goal);
                                if self.tween.is_none() {
                                    self.camera = goal;
                                }
                            }
                        }
                    }
                }

                if let Some(tw) = self.tween.as_mut() {
                    let dt = ctx.input(|i| i.stable_dt);
                    if tw.tick(&mut self.camera, dt) {
                        ctx.request_repaint();
                    } else {
                        self.tween = None;
                    }
                }

                if let Some(rs) = frame.wgpu_render_state() {
                    let w = rect.width().max(1.0) as u32;
                    let h = rect.height().max(1.0) as u32;
                    self.renderer.resize(&rs.device, w, h);
                    {
                        let mut wr = rs.renderer.write();
                        self.renderer.sync_egui_tex(&rs.device, &mut wr);
                    }
                    self.renderer.render(
                        &rs.device,
                        &rs.queue,
                        &self.camera,
                        aspect,
                        self.scene.as_ref(),
                    );
                    if let Some(id) = self.renderer.egui_tex {
                        ui.painter().image(
                            id,
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                }

                let dropping = hover_dropping(ctx);
                if self.scene.is_none() {
                    if !dropping {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Drop a file, or File -> Open",
                            egui::FontId::proportional(16.0),
                            Color32::from_gray(130),
                        );
                    }
                } else if !dropping {
                    let (view, _, _) = self.camera.view_proj(aspect);
                    view_cube::paint(ui, rect, view);
                }

                if dropping {
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(20, 40, 70, 180),
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Drop to open (replaces the current scene)",
                        egui::FontId::proportional(18.0),
                        Color32::from_gray(235),
                    );
                }
            });
    }
}
