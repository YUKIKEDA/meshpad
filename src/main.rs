//! Meshpad は STL/NAS 向けの軽量 3D ビューアです。
//!
//! メモ帳のように単体 exe で開き、形をすぐ確認することを目的にする。
//! STL（バイナリと ASCII）と NAS を CLI・ドロップ・「ファイル → 開く」から読み、直交投影の自由軌道で回せる。
//!
//! # Examples
//!
//! ```text
//! cargo run -- bench/data/derived/stl/bunny.stl
//! ```
//!
//! 引数なしなら空ウィンドウを出す。フォルダを渡せば直下のメッシュだけを載せる。

#![warn(missing_docs)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu::wgpu::{
    Backends, DeviceDescriptor, Features, InstanceDescriptor, MemoryHints, Trace,
};
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use meshpad::app::MeshpadApp;
use meshpad::icon;

fn main() -> eframe::Result<()> {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();

    let wgpu_options = WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(WgpuSetupCreateNew {
            instance_descriptor: InstanceDescriptor {
                backends: Backends::DX12,
                ..Default::default()
            },
            // eframe 既定の max_buffer_size は 256MB。lucy 級の全載せはアダプタ上限が要る。
            device_descriptor: Arc::new(|adapter| {
                let mut limits = adapter.limits();
                limits.max_texture_dimension_2d = limits.max_texture_dimension_2d.max(8192);
                DeviceDescriptor {
                    label: Some("meshpad"),
                    required_features: Features::default(),
                    required_limits: limits,
                    memory_hints: MemoryHints::Performance,
                    trace: Trace::Off,
                }
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0])
            .with_title("Meshpad")
            .with_decorations(false)
            .with_resizable(true)
            .with_icon(icon::viewport_icon())
            .with_drag_and_drop(true),
        wgpu_options,
        // メッシュの深度は `Renderer` のオフスクリーン側。egui のメインパスには不要。
        depth_buffer: 0,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Meshpad",
        options,
        Box::new(move |cc| Ok(Box::new(MeshpadApp::new(cc, paths)))),
    )
}
