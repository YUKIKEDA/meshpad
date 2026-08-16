# NAS パース計測（2026-08-16）

1.0 の全載せ経路。カード分割・外皮抽出・GPU 載せを壁時計と CPU 割り当てで見る。C-lite の T_grid / T_skin は後段。
基準列 `pynas` は `bench/.venv` の pyNastran `BDF.read_bdf`（フル BDF。外皮抽出はしない）。
再現: `cargo bench --bench nas_parse`。データが無ければ先に `bench/README.md` の `--nas-only` を生成する。追加の NAS パスは引数で足せる。無いファイルは黙って飛ばす。venv が無ければ `pynas` は `-`。

下の表はこのマシン（Windows / DX12）の **1 回分**。ラボ精度ではない。比較とボトルネック特定用。

## 測り方

ハーネスは `bench/nas_parse.rs`（`harness = false`）。Criterion は使わない。プロファイルは `[profile.release]`（`lto = true`, `codegen-units = 1`）を `cargo bench` がそのまま使う。測り方の細部（ウォームアップ、中央値、`GlobalAlloc`）は [stl_parse.md](stl_parse.md) と同じ。`pynas` だけは外部プロセスなので割り当て表に出さない。

### 1 ファイルあたりの手順

1. メタデータでファイル MB。mmap。計測 **外** で 1 回 `parse_nas` して外皮三角形数を取る（この時点でページキャッシュに乗る）。
2. 外皮が 1000 万三角形以上なら試行 3、それ以外は 5。ウォームアップは常に 1。ラダーの最大は happy 約 109 万なので常に 5。
3. 列を **この順** で測る: `parse` → `soup` → `gpu` → `pynas`。
4. `parse` / `soup` / `gpu`: ウォームアップ 1 回のあと、試行ごとに `Instant` でクロージャ全体の壁時計。結果は `black_box`。
5. `pynas`: `bench/scripts/nas_pynastran_read.py` を一度起動。Python 内で import / `BDF()` のあと `read_bdf(..., punch=True)` だけを測る。40 MB 超は試行 3、それ以外は 5。インタプリタ起動は中央値に入らない。
6. **時間の中央値** と、**peak / allocated の中央値** は別々に取る。同じ試行の組ではない。

既定のファイル順は表面（bunny → happy）のあと体積。small8 と large16 を隣に置く。

### 各列が実際に囲っている範囲

| 列      | クロージャ                                                                 | 計測に入る                                                                  | 入らない                                                              |
| ------- | -------------------------------------------------------------------------- | --------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `parse` | 既に持っている mmap に対する `nas::parse_nas`                              | 行走査、スライスからの数値化、GRID 表、シェル直出し、体積の面 HashMap、展開 | open / mmap / GPU                                                     |
| `soup`  | `load::load_paths(&[path])`                                                | 製品ステータスバーと同じ（open + mmap + parse + AABB）                      | GPU                                                                   |
| `gpu`   | 計測 **前** に 1 回作ったスープへ `SceneGpu::from_soup`。終わったら `drop` | バッファ作成、mapped への原点引き書き、`unmap`                              | パース、`queue.submit`、描画                                          |
| `pynas` | Python 内の `BDF.read_bdf`                                                 | カードオブジェクトと節点・要素の構築                                        | 外皮抽出、`cross_reference`、インタプリタ起動、こちらの `GlobalAlloc` |

製品の「開いて見える」はおおむね **`soup` + `gpu`**。点群先行（T_grid）はこのマイルストーンでは測らない。`parse_nas` は外皮まで一気に出す。

`gpu` は頂点が `3 × 12 × 三角形` バイトでアダプタの `max_buffer_size` を超えると `-`。デバイスはプロセスで 1 個、DX12。

### 比較として妥当なこと / 妥当でないこと

**してよい**

- 同じ列の前後比（フィールド表現を変える、HashMap を変える、など）。
- `parse` と `soup` がほぼ同じであること（I/O が温まったあとの追加は小さい）。
- 表面 happy と STL ASCII happy（同じ 109 万三角形）の `parse` 比。
- `parse` と `pynas`: 「自前パーサを書く根拠」。pyNastran は仕事が多いので、こちらが速いこと自体は期待どおり。逆に遅いと問題。
- 体積の `parse` を **外皮三角形数ではなく要素数／全画面** で読む。

**してはいけない**

- `gpu` の ms を「画面に出るまで」と読む。
- GPU 列の peak MB を VRAM と読む。
- 表を **コールドな初回オープン** と読む。
- `pynas` と `parse` を「同じ仕事の何倍」とだけ書いて終わりにする。pyNastran は PSHELL / MAT1 までオブジェクト化する。
- `skin_tris` だけで体積 NAS のコストを語る。

## 結果（時間）

| file                |   MB | skin_tris |  parse |   soup |    gpu |  pynas |
| ------------------- | ---: | --------: | -----: | -----: | -----: | -----: |
| bunny_res3 small8   | 0.27 |        4k | 0.73ms | 0.85ms | 0.02ms | 62.6ms |
| bunny_res3 large16  | 0.45 |        4k | 1.00ms | 1.16ms | 0.02ms | 64.5ms |
| bunny small8        | 5.03 |       69k | 13.2ms | 15.3ms | 0.21ms |  1.36s |
| bunny large16       | 8.39 |       69k | 20.3ms | 23.8ms | 0.23ms |  1.43s |
| happy_res2 small8   |   21 |      293k | 54.7ms | 63.1ms | 0.87ms |  5.83s |
| happy small8        |   78 |     1.09M |  218ms |  233ms | 10.3ms |  21.9s |
| box_hex_c20 small8  | 1.20 |      4.8k | 3.36ms | 3.71ms | 0.02ms |  217ms |
| box_hex_c20 large16 | 2.07 |      4.8k | 5.08ms | 5.90ms | 0.03ms |  237ms |
| box_tet_c20 small8  | 3.10 |      4.8k | 9.60ms | 10.7ms | 0.03ms |  645ms |
| box_hex_c40 small8  | 9.39 |     19.2k | 29.0ms | 30.9ms | 0.07ms |  1.86s |

アダプタの `max_buffer_size` は 2048 MB。happy の頂点バッファは約 37 MB で載る。

同じ 109 万三角形の STL ASCII happy は parse 303ms。NAS happy は **218ms** で、テキストとして並ぶ。`pynas` の 22s はフル BDF なので対戦相手としては上限側。

`skin_tris` は PLY の三角形数と一致する（bunny_res3 = 3851、happy_res2 = 293232）。以前はシェルも面 HashMap に入れ、同じ 3 節点キーを内部面扱いで落としていた。

## 結果（割り当て）

| file              | parse / soup peak | allocated |
| ----------------- | ----------------: | --------: |
| bunny small8      |            4.7 MB |       4.7 |
| happy_res2 small8 |             20 MB |        20 |
| happy small8      |             74 MB |        74 |
| box_hex_c20       |            2.8 MB |       2.8 |
| box_tet_c20       |            7.0 MB |       7.0 |
| box_hex_c40       |             22 MB |        22 |

happy の出力スープは約 37 MB。peak 74 MB は GRID 表 + シェル節点 ID + 頂点 `Vec` が同時にいる分。カード列挙の中間ベクタはもう無い。allocated と peak が一致するのは、再ハッシュで捨てる中間表がほぼ無くなったため。

**GPU 列の peak は常に 0.0 MB**。VRAM 側は happy で約 37 MB。`pynas` の RSS はこちらのアロケータに出ない。

## 実験

比較のベース（フィールドを全部 `String`、シェルも面 HashMap、カード並列）は happy parse 1.57s / peak 680 MB、happy_res2 384ms、bunny 87ms。

### 1. mmap スライスから数値化 + シェル直出し

論理カードの `Vec<String>` をやめ、行の `&[u8]` から `fast-float2` / 整数を読む。`CTRIA3` / `CQUAD4` は表面なので HashMap を通さず節点 ID を積む。体積の `CTETRA` / `CHEXA` だけ共有面を数える。`FaceHit` の巻きは固定配列。

| ファイル    | 以前 parse |    今 | peak 以前 | peak 今 |
| ----------- | ---------: | ----: | --------: | ------: |
| bunny       |       87ms |  16ms |     43 MB |  6.5 MB |
| happy_res2  |      384ms |  70ms |    177 MB |   27 MB |
| happy       |      1.57s | 277ms |    680 MB |  101 MB |
| box_hex_c40 |      158ms |  45ms |     80 MB |   26 MB |

**入れる価値あり。** カード並列（Rayon、1 万枚以上）はこの段では外した。数値化が軽くなったあと、継続行の直列走査のほうが単純で、happy でも十分速い。

### 2. pyNastran を列にする

`read_bdf` は happy で 22s。自前（当時 277ms、今 218ms）は仕事が違う。並べる意味は「自前を書く根拠」と回帰（突然 10 倍遅くなったら気付ける）で、倍速自慢ではない。

### 3. `ParsedCard` 全件ベクタをやめる

走査中に GRID 表・シェル節点・体積面へ直接積む。節点が要素の後に来てもよいので、三角形座標は全 GRID のあとで解決する。

| ファイル    | 以前 parse |    今 | peak 以前 | peak 今 |
| ----------- | ---------: | ----: | --------: | ------: |
| bunny       |       16ms |  13ms |    6.5 MB |  4.7 MB |
| happy_res2  |       70ms |  55ms |     27 MB |   20 MB |
| happy       |      277ms | 218ms |    101 MB |   74 MB |

**入れる価値あり。** happy の 101→74 MB は中間カード列挙が消えた分。37 MB のスープまでは、GRID 表と節点 ID を展開中も持つ必要がある。

### 4. 体積の面 HashMap（容量予約とキー圧縮）

初回の体積面で `bytes.len() / 50` を予約する（再ハッシュの 2 倍ピークを避ける）。面の同一性はソート済み節点 ID、巻きは置換番号 1 バイト。`u128` にパックすると整列パディングで **悪化**したので、キーは `[u32; 3]` / `[u32; 4]` のままにした。

| ファイル    | 以前 parse |    今 | peak 以前 | peak 今 |
| ----------- | ---------: | ----: | --------: | ------: |
| box_hex_c20 |      5.7ms | 3.4ms |    3.3 MB |  2.8 MB |
| box_tet_c20 |       17ms | 9.6ms |    9.6 MB |  7.0 MB |
| box_hex_c40 |       45ms |  29ms |     26 MB |   22 MB |

**入れる価値あり。** 外皮 19k でも中の面は約 20 万エントリあるので、予約と値の縮小が効く。

### 5. FxHasher

GRID 表と面表を標準 SipHash から `rustc-hash` の Fx にする。入力は自前ファイルなのでハッシュ DOS は見ない。表 3・4 と同じ走査に入っている。happy の GRID ルックアップと体積の面ヒットの両方に効く。

## いま残るボトルネック

1. **happy の CPU**: まだ 0.22s。ASCII STL happy と同規模。残りは行走査と GRID 表。
2. **体積の面表**: box_hex_c40 は 29ms / 22 MB。コストは外皮枚数ではなく要素数／全画面で決まる。これ以上はファイル形式か空間分割。
3. **GPU** は happy でも 10ms。今はいじらない。

## コード側に残した定数

- カードあたりフィールド上限: 16（`src/nas.rs`）
- 体積面の初回予約: ファイルバイト / 50
- `pynas` 試行: 40 MB 超は 3、それ以外は `parse` と同じ 5

## まだやっていないこと

- 走査を列で分ける（T_grid / T_skin の近似）は後段
- 実 CAE（`bench/data/local/`）
- C-lite の走査中プロキシ（1.0 の外）
