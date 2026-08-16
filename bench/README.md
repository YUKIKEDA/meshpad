# Meshpad ベンチマーク方針

実装に入る前の計測用データと、測るものの契約。Stanford の PLY は **ソース** であり、1.0 の入力形式ではない。
変換結果と数 GB 級は git に載せない（`.gitignore`）。再生成はスクリプト。

出典: [Stanford 3D Scanning Repository](http://graphics.stanford.edu/data/3Dscanrep/)（bunny / happy Buddha / lucy）。

## 測るもの（1.0 の成功条件に合わせる）

比較の主軸は **ファイル MB ではなく三角形数（STL）／節点数＋要素数（NAS）**。
ASCII NAS は同じ幾何でも STL の数倍〜十数倍のバイトになる。

| 指標       | 意味                               | 合格の見方                               |
| ---------- | ---------------------------------- | ---------------------------------------- |
| T_proxy    | bbox＋粗いプロキシが出て回せるまで | 「メモ帳」の体感。初回走査完了を待たない |
| T_index    | 空間索引が TEMP に載るまで         | 2 回目オープンの前提                     |
| T_open2    | 同じファイルの再オープン           | TEMP キャッシュが効いていること          |
| T_grid     | NAS で点群が出るまで               | STL には無い                             |
| T_skin     | NAS 外皮がプロキシを置き換えるまで | 内部面がチラつかないこととセット         |
| RSS / VRAM | プロセスと GPU 常駐                | 近景はシーン全体で約 200 万三角形        |
| FPS        | プロキシのみ、および近景ロード後   | 操作中に落ちても表示は維持               |
| T_refine   | 直交ズーム後に近景セルが載るまで   | 拡大に意味があることの計測               |

日々の開発は smoke（tiny/small）でよい。C-lite の判定は **happy_subdiv1 と lucy** が必須。負荷試験マイルストーンで定数 200 万を動かす。

### パース比較（STL）

C-lite ハーネスとは別に、パース・スープ化・GPU 載せを `stl_io` と並べて測る。時間に加え、ベンチ専用アロケータで CPU 割り当てのピークと合計も出す（VRAM / mmap は含まれない）。

```text
cargo bench --bench stl_parse
```

結果・測り方・妥当性の限界は [stl_parse.md](stl_parse.md)。C-lite の T_proxy 以降はまだ後段。

### パース比較（NAS）

STL と同じハーネス形で、表面（`GRID`+`CTRIA3`）と体積（`CHEXA` / `CTETRA` の外皮）を測る。基準列は `bench/.venv` の pyNastran `read_bdf`（フル BDF。外皮は取らない）。

```text
cargo bench --bench nas_parse
```

結果・測り方は [nas_parse.md](nas_parse.md)。点群先行（T_grid）と TEMP 索引はまだ後段。

## 手元のソース（PLY）

| ファイル                          | 形式      | 頂点       | 三角形     | おおよそ |
| --------------------------------- | --------- | ---------- | ---------- | -------- |
| `bunny/bun_zipper_res4.ply`       | ASCII     | 453        | 948        | 0.03 MB  |
| `bunny/bun_zipper_res3.ply`       | ASCII     | 1,889      | 3,851      | 0.14 MB  |
| `bunny/bun_zipper_res2.ply`       | ASCII     | 8,171      | 16,301     | 0.63 MB  |
| `bunny/bun_zipper.ply`            | ASCII     | 35,947     | 69,451     | 2.9 MB   |
| `happy_recon/happy_vrip_res4.ply` | ASCII     | 7,108      | 15,536     | 0.5 MB   |
| `happy_recon/happy_vrip_res3.ply` | ASCII     | 32,328     | 67,240     | 2.3 MB   |
| `happy_recon/happy_vrip_res2.ply` | ASCII     | 144,647    | 293,232    | 10 MB    |
| `happy_recon/happy_vrip.ply`      | ASCII     | 543,652    | 1,087,716  | 41 MB    |
| `lucy/lucy.ply`                   | binary BE | 14,027,872 | 28,055,742 | 508 MB   |

10 倍刻みの **PLY** は揃っている。バイナリ STL にするとサイズが変わる（50 バイト×三角形＋84）。

- happy 全解像度 → 約 **54 MB STL**（「数百 MB STL」ではない）
- lucy → 約 **1.40 GB STL**（数 GB 帯の下限。別途スキャンデータを探す必要はない）
- 中間の数百 MB STL は **happy を 1 回細分**（約 435 万三角形 → 約 **217 MB**）で埋める

ASCII STL は bunny/happy だけで足りる。`happy_subdiv1` と lucy の ASCII は数百 MB〜数 GB のテキストになるので 1.0 では作らない。

## ラダー（生成物）

出力先: `bench/data/derived/`（git 対象外）。

### STL（パーサ・C-lite）

バイナリは `stl/`。ASCII は同じ幾何を `stl_ascii/` に置く（フォルダドロップで混ざらないように分ける）。

| 段       | 生成物                  | 三角形（目安） | バイナリ       | ASCII                | 何を見るか                               |
| -------- | ----------------------- | -------------- | -------------- | -------------------- | ---------------------------------------- |
| smoke    | `bunny_res3.stl`        | 4k             | `stl/` <1 MB   | `stl_ascii/` ~0.7 MB | 起動・回帰、ASCII パーサ                 |
| small    | `bunny.stl`             | 69k            | `stl/` ~3.5 MB | `stl_ascii/` ~13 MB  | チャンク 1 個の日常                      |
| medium   | `happy_res2.stl`        | 293k           | `stl/` ~15 MB  | `stl_ascii/` ~56 MB  | 同上                                     |
| large    | `happy.stl`             | 1.09M          | `stl/` ~54 MB  | `stl_ascii/` ~207 MB | 全載せ経路。ASCII の上限付近             |
| heavy    | `stl/happy_subdiv1.stl` | ~4.4M          | ~220 MB        | 作らない             | 上限超過・C-lite 前半                    |
| huge     | `stl/lucy.stl`          | 28.1M          | ~1.40 GB       | 作らない             | 初回走査中操作、TEMP、近景 200 万        |
| optional | `--tile 2,1,1` on lucy  | 56M            | ~2.8 GB        | 作らない             | 数 GB 上限。ディスクに余裕があるときだけ |

### NAS は 2 系統（混ぜて評価しない）

**NAS はすべて pyNastran が書く。** 自前のカンマ出力は仕様テストにならないので残さない。
`PSHELL` / `PSOLID` / `MAT1`（指数 `2.1+11`）も実ファイルどおり混ざる。ビューアは未知カードとして飛ばす。

| 系統   | 作り方                              | 見るもの                                |
| ------ | ----------------------------------- | --------------------------------------- |
| 表面   | PLY → pyNastran `GRID`+`CTRIA3`     | フィールド形式、点群→面                 |
| 体積   | 格子 → pyNastran `CHEXA` / `CTETRA` | 外皮抽出。small-field の `CHEXA` 継続行 |
| 実 CAE | `bench/data/local/`                 | 本番の揺れ                              |

ファイル名: `*_small8.nas` は 8 桁フィールド、`*_large16.nas` は `GRID*` の 16 桁（ロング）。同じ幾何を両方置くのは bunny 級と `box_hex_c20`。happy 以上は small8 のみ（オブジェクト数が重い）。

lucy の表面 NAS と 100³ CHEXA は作らない。STL 側でサイズを見る。

| 生成                                      | 形式         | 役割                           |
| ----------------------------------------- | ------------ | ------------------------------ |
| `bunny_res3_small8.nas` / `_large16.nas`  | 両フィールド | パーサ回帰                     |
| `bunny_small8.nas` / `_large16.nas`       | 両フィールド | 同上・日常サイズ               |
| `happy_res2_small8.nas`                   | small8       | 表面の中規模                   |
| `happy_small8.nas`                        | small8       | 表面の大規模（~100 万 CTRIA3） |
| `box_hex_c20_small8.nas` / `_large16.nas` | 両フィールド | CHEXA 継続                     |
| `box_tet_c20_small8.nas`                  | small8       | tet 外皮                       |
| `box_hex_c40_small8.nas`                  | small8       | 体積の中規模                   |

実ファイルは `bench/data/local/`。git 対象外。

## 作り方

STL 変換は標準ライブラリのみ。NAS を仕様どおり書くときは **pyNastran を `bench/.venv` に入れる**（グローバルに入れない）。

```text
python -m venv bench/.venv
bench\.venv\Scripts\python -m pip install -r bench/requirements.txt

bench\.venv\Scripts\python bench/scripts/prepare_ladder.py --nas-only
bench\.venv\Scripts\python bench/scripts/prepare_ladder.py --stl-only --tier all
bench\.venv\Scripts\python bench/scripts/prepare_ladder.py --ascii-only
```

PLY が無いとき: 上の Stanford のページから `bunny` / `happy_recon` / `lucy` を `bench/data/<name>/` に置く。

## 複数ファイル

組み立て確認用: derived の STL をそのまま複数渡せばよい（ワールド座標のまま結合）。
フォルダ直下ドロップの確認は `derived/stl/`（バイナリ）または `derived/stl_ascii/` を使う。再帰は 1.0 に無いのでサブフォルダを切らない。混ぜると ASCII とバイナリが同じシーンになる。

## git

追跡する: `bench/scripts/`、`bench/requirements.txt`、`bench/**/README*`、`bench/stl_parse.md`、`bench/nas_parse.md`、このファイル。
追跡しない: `*.ply` `*.stl` `*.nas`、`derived/`、`local/`、`.venv/`。
