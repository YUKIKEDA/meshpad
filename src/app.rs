//! eframe 上の Meshpad ウィンドウ。
//!
//! 起動引数・ドロップ・「ファイル → 開く」のファイル列は、いずれも新しいシーンになる。
//! `F` で全体フィット。左下のビューキューブで向き＋ズームを揃える。

use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui::{self, Color32, PointerButton, RichText, Sense};
use glam::Vec2;

use crate::camera::{Camera, CameraTween};
use crate::gpu::{Renderer, SceneGpu};
use crate::load;
use crate::open;
use crate::view_cube;

/// Meshpad のウィンドウ本体。
///
/// 細いファイルメニューとステータス、ビュー全面の 3D を持つ。
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

impl eframe::App for MeshpadApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let title = match &self.title_file {
            Some(f) => format!("Meshpad - {f}"),
            None => "Meshpad".into(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let mut want_open =
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O));

        egui::TopBottomPanel::top("bar")
            .exact_height(28.0)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(22, 22, 24))
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Open...    Ctrl+O").clicked() {
                            ui.close();
                            want_open = true;
                        }
                    });
                    if !self.status.is_empty() {
                        ui.separator();
                        ui.label(RichText::new(&self.status).color(Color32::from_gray(160)));
                    }
                    for w in &self.warnings {
                        ui.label(RichText::new(w).color(Color32::from_rgb(220, 160, 80)));
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
                                let mut goal = self.camera.clone();
                                goal.snap_and_fit(hit.dir, scene.radius, aspect);
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
