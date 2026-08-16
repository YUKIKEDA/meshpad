# Meshpad

[![CI](https://github.com/YUKIKEDA/meshpad/actions/workflows/ci.yml/badge.svg)](https://github.com/YUKIKEDA/meshpad/actions/workflows/ci.yml)

Windows 向けの軽量 STL / NAS ビューア。単一の exe で開き、形を確認して回せます。

計測・メッシュ診断はしません。Windows 3D ビューアーの「見て終わり」置換が目的です。

## 入手

[Releases](https://github.com/YUKIKEDA/meshpad/releases) から `meshpad.exe` を置いて使います。インストーラはありません。

要件:

- Windows 10 / 11（x64）
- Direct3D 12 が使える GPU

Visual C++ 再頒布パッケージは不要です。

## 使い方

```text
meshpad
meshpad model.stl
meshpad part_a.stl part_b.nas
meshpad mesh_folder
```

引数なしは空ウィンドウです。「ファイル → 開く」かドロップでも開けます。渡したファイル列がその場のシーンになります（確認ダイアログなし）。読み込み中に開き直すと、前のジョブは破棄されます。

| 操作 | 内容 |
| --- | --- |
| 左ドラッグ | 軌道 |
| 右ドラッグ / 中ドラッグ | パン |
| ホイール | カーソル位置へ直交ズーム |
| `F` | 全体フィット |
| `Y` | Y-up / Z-up |
| `Ctrl+O` | 開く（シーン置き換え） |
| `F1` | 操作一覧 |
| ビューキューブ | 向きをスナップしてフィット |

## 対応形式

| 形式 | 内容 |
| --- | --- |
| STL | バイナリと ASCII。色・マテリアルは無し |
| NAS / Nastran bulk | `GRID` / `CTRIA3` / `CQUAD4` / `CTETRA` / `CHEXA`。体積は外皮のみ |

ワールド座標のまま結合します。未知の拡張子やカードは読み飛ばして警告します。フォルダは直下の `.stl` / `.nas` / `.nastran` だけです。

既定の世界 up は Z。投影は直交。裏面も描きます。

巨大ファイルは裏で全三角形を展開し、チャンクに分けて GPU へ載せます。完了まで操作は止まり、失敗するとシーンは空になります。

## ビルド

Rust（stable、MSVC）と Windows SDK が必要です。

```text
cargo build --release
```

成果物は `target/release/meshpad.exe` です。`.cargo/config.toml` で CRT を静的リンクします。

```text
cargo test --lib
cargo fmt --all
```

計測データとハーネスは [`bench/README.md`](bench/README.md) です。巨大 STL/NAS は git に入れていません。

開発者向けの全体設計は [`.dev/architecture.md`](.dev/architecture.md)。製品契約（1.0 の範囲）は [`.dev/project.md`](.dev/project.md)。

## リリース

`Cargo.toml` の `version` を上げてタグを推します。

```text
git tag v1.0.0
git push origin v1.0.0
```

[Release](https://github.com/YUKIKEDA/meshpad/actions/workflows/release.yml) workflow が Windows の release ビルドを作り、GitHub Release に `meshpad.exe` を載せます。タグ（`v1.0.0`）と `Cargo.toml` の version は一致させてください。
