//! Meshpad は STL/NAS 向けの軽量 3D ビューアです。
//!
//! メモ帳のように単体 exe で開き、形をすぐ確認することを目的にする。
//! いまの実行ファイルは STL（バイナリと ASCII）を CLI・ドロップ・「ファイル → 開く」から読み、直交投影の自由軌道で回せる。
//!
//! # Examples
//!
//! ```text
//! cargo run -- bench/data/derived/stl/bunny.stl
//! ```
//!
//! 引数なしなら空ウィンドウを出す。フォルダを渡せば直下のメッシュだけを載せる。

#![warn(missing_docs)]

mod app;
mod camera;
mod gpu;
mod open;
mod stl;
mod view_cube;

use std::path::PathBuf;

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use eframe::egui_wgpu::wgpu::{Backends, InstanceDescriptor};

fn main() -> eframe::Result<()> {
    let paths: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect();

    let wgpu_options = WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(WgpuSetupCreateNew {
            instance_descriptor: InstanceDescriptor {
                backends: Backends::DX12,
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("Meshpad")
            .with_drag_and_drop(true),
        wgpu_options,
        depth_buffer: 0,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Meshpad",
        options,
        Box::new(move |cc| Ok(Box::new(app::MeshpadApp::new(cc, paths)))),
    )
}
