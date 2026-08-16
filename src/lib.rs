//! Meshpad のライブラリ面。ウィンドウ本体はバイナリ、パース比較はここから呼ぶ。
//!
//! 製品の約束はリポジトリの `.dev/project.md`。モジュールの繋がりは `.dev/architecture.md`。
//!
//! パス列は [`open`] でファイルに展開し、[`load`] が STL/NAS を [`mesh::TriangleSoup`] に結合する。
//! [`gpu::SceneGpu`] が頂点チャンクを載せ、[`app`] が読み込み状態とカメラ操作を回す。

#![warn(missing_docs)]

/// ウィンドウ、開く状態機械、入力。
pub mod app;
/// 直交カメラと短い姿勢補間。
pub mod camera;
/// GPU チャンクとオフスクリーン描画。
pub mod gpu;
/// ウィンドウ／タイトル用アイコン。
pub mod icon;
/// STL と NAS を一つのスープへ。
pub mod load;
/// 三角形スープと読み込みプローブ。
pub mod mesh;
/// Nastran bulk の外皮。
pub mod nas;
/// CLI・ドロップ・ダイアログのパス展開。
pub mod open;
/// STL（バイナリ / ASCII）。
pub mod stl;
/// 画面隅のビューキューブ。
pub mod view_cube;
