# STL パース計測（2026-08-15）

C-lite の T_proxy とは別経路。パース・スープ化・GPU 載せを壁時計と CPU 割り当てで見る。
再現: `cargo bench --bench stl_parse`。データが無ければ先に `bench/README.md` のラダーを生成する。追加の STL パスは引数で足せる。無いファイルは黙って飛ばす。

下の表はこのマシン（Windows / DX12）の **1 回分**。ラボ精度ではない。比較とボトルネック特定用。

## 測り方

ハーネスは `benches/stl_parse.rs`（`harness = false`）。Criterion は使わない。信頼区間や外れ値検出は無い。プロファイルは `[profile.release]`（`lto = true`, `codegen-units = 1`）を `cargo bench` がそのまま使う。

### 1 ファイルあたりの手順

1. メタデータでファイル MB。mmap。計測 **外** で 1 回 `parse_stl` して三角形数を取る（この時点でページキャッシュに乗る）。
2. 三角形が 1000 万以上なら試行 3、それ以外は 5。ウォームアップは常に 1。
3. 列を **この順** で測る: `parse` → `soup` → `gpu` → `stl_io` →（条件付き）`stl_io_idx`。
4. 各列: ウォームアップ 1 回のあと、試行ごとに `Instant` でクロージャ全体の壁時計。結果は `black_box`。
5. **時間の中央値** と、**peak / allocated の中央値** は別々に取る。同じ試行の組ではない。

既定のファイル順はバイナリ（bunny → … → lucy）のあと ASCII。lucy の直後に ASCII happy の GPU が来る。

### 各列が実際に囲っている範囲

| 列           | クロージャ                                                                           | 計測に入る                                                                        | 入らない                                    |
| ------------ | ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------- | ------------------------------------------- |
| `parse`      | 既に持っている mmap に対する `parse_stl`                                             | CPU 展開・AABB・`Vec`                                                             | open / mmap / GPU                           |
| `soup`       | `load_paths(&[path])`                                                                | 製品ステータスバーと同じ（open + mmap + parse + AABB）。位置の CPU シフトはしない | GPU                                         |
| `gpu`        | 計測 **前** に 1 回作ったスープへ `SceneGpu::from_soup`。終わったら `drop`           | バッファ作成、mapped への原点引き書き、`unmap`                                    | パース、`queue.submit`、描画、`device.poll` |
| `stl_io`     | 同じ mmap を `Cursor<&[u8]>` 経由で `create_stl_reader` し、三角形を捨てながら数える | パーサ本体                                                                        | 自前のような頂点 `Vec`、溶接                |
| `stl_io_idx` | `read_stl`（溶接）。2M 三角形以上は省略                                              | 索引化まで                                                                        | —                                           |

製品の「開いて見える」はおおむね **`soup` + `gpu`**。足し算は別計測の中央値同士なので、同時に走った一回分ではない。ステータスバーは `soup` だけ。

`gpu` は頂点が `3 × 12 × 三角形` バイトでアダプタの `max_buffer_size` を超えると `-`。デバイスはプロセスで 1 個、DX12、アダプタ limits をそのまま要求する。

### 比較として妥当なこと / 妥当でないこと

**してよい**

- `parse` と `stl_io`: どちらも「すでに RAM にあるバイト列」のパーサ。自前パーサを書く根拠。
- 同じ列の前後比（Rayon 導入前と後、など）。ビルド条件が同じなら。
- `parse` / `soup` の peak が「頂点 `Vec` 1 本分か」。

**してはいけない**

- `soup` と `stl_io` を並べて「何倍速いか」。`soup` は I/O と mmap を含み、`stl_io` は含まない。
- `stl_io_idx` をパース対戦相手にする。溶接は別仕事。
- 表のミリ秒を **コールドな初回オープン** と読む。三角形数のための事前 `parse_stl` とウォームアップで、測っているのはページキャッシュが乗ったあとの再実行に近い。
- `gpu` の ms を「画面に出るまで」。`unmap` までで、GPU 完了も最初のフレームも待たない。
- GPU 列の peak MB を VRAM 使用量と読む（後述）。

### 割り当て

`#[global_allocator]` は **このベンチバイナリだけ**。`System` を包み、`alloc` / `alloc_zeroed` / `realloc` のサイズを Relaxed 加算する。アプリの exe には付けない。

区間に入る直前の `current` と累計 `allocated` をスナップショットし、`peak` をその `current` に戻す。終わったら差分:

- **peak extra MB**: 区間中の high-water − 開始時の `current`
- **allocated MB**: 区間中に要求したバイト合計（`dealloc` しても減らない）

開始時に `current` を 0 にはしない（先に生きているアロケーションの `dealloc` でアンダーフローするため）。

**見えないもの**: ファイル mmap、VRAM、ドライバが mapped バッファに付けた領域、スレッドスタック、ウォームアップ中に作った Rayon プール。したがって GPU 列の peak はほぼ 0 と出る。lucy の本物の GPU 常駐は頂点だけで約 963 MB。

### 既知の歪み

- **キャッシュ温め**: コールドディスクや大きな STL の初回ページフォルトは `soup` でも過小。lucy の 1.3 GB を初めて開く実時間は表より悪くなり得る。
- **Rayon プール**: 初回起動はウォームアップ側。製品の「その日最初の大きいファイル」には乗る。
- **GPU の非同期**: `drop(scene)` のあとに `poll` しない。次のファイルの `gpu` が前の破棄と重なり得る。ASCII happy の gpu がバイナリ happy より遅いのは、lucy の直後だから疑う。
- **小さいファイル**: 0.03ms 台はタイマー粒度とノイズが支配的。bunny_res3 で優劣を語らない。
- **1 マシン・1 実行**: 表はスナップショット。再実行で十数 % 動いても同じ結論なら十分、とする程度の精度。

## 結果（時間）

| file             |   MB |  tris |  parse |   soup |    gpu | stl_io | stl_io_idx |
| ---------------- | ---: | ----: | -----: | -----: | -----: | -----: | ---------: |
| bunny_res3 bin   | 0.18 |    4k | 0.03ms | 0.09ms | 0.03ms | 0.03ms |     0.27ms |
| bunny bin        | 3.31 |   69k | 0.83ms | 1.52ms | 0.25ms | 0.60ms |     7.53ms |
| happy_res2 bin   |   14 |  293k | 1.36ms | 2.64ms | 1.08ms | 2.83ms |     33.3ms |
| happy bin        |   52 | 1.09M | 4.80ms | 8.84ms | 12.2ms | 10.6ms |      147ms |
| happy_subdiv1    |  207 | 4.35M | 21.3ms | 33.3ms | 80.8ms | 43.6ms |          — |
| lucy             | 1338 | 28.1M |  168ms |  243ms |  782ms |  278ms |          — |
| bunny_res3 ASCII | 0.74 |    4k | 1.12ms | 1.33ms | 0.02ms | 8.47ms |     8.96ms |
| bunny ASCII      |   13 |   69k | 20.9ms | 24.0ms | 0.23ms |  153ms |      166ms |
| happy_res2 ASCII |   56 |  293k | 87.2ms | 99.2ms | 0.99ms |  659ms |      699ms |
| happy ASCII      |  207 | 1.09M |  303ms |  350ms | 44.3ms | 2359ms |     2584ms |

アダプタの `max_buffer_size` は 2048 MB。lucy 頂点バッファは約 963 MB で載る。

## 結果（割り当て）

peak と allocated は自前パスではほぼ一致（`Vec::with_capacity` で伸びない）。

| file          | parse / soup peak | stl_io allocated |
| ------------- | ----------------: | ---------------: |
| happy bin     |           37.3 MB |   ~0（列挙だけ） |
| happy_subdiv1 |            149 MB |               ~0 |
| lucy          |            963 MB |               ~0 |
| happy ASCII   |           41.4 MB |      **1158 MB** |

自前は「三角形数 × 12 バイト × 3」がほぼ全部。ASCII happy がバイナリより数 MB 多いのは容量見積もり `nbytes/60` の余り。

`stl_io` のバイナリ列挙は頂点 `Vec` を持たないので CPU 割り当ては小さい。ASCII は内部で巨大に積む。溶接 `stl_io_idx` は時間も割り当ても別物なので参考値。

**GPU 列の peak は常に 0.0 MB** と出る。mapped バッファはプロセスの `GlobalAlloc` を通らない。VRAM 側は lucy で約 963 MB（位置のみ、法線はシェーダ復元）。

## 実験と結論

比較のベース（AABB をパースに融合した直後、この三実験の前）は happy bin parse ~13ms / soup ~28ms、happy ASCII ~386ms、happy_subdiv1 parse ~51ms。`stl_io` 三角形列挙とバイナリ parse はほぼ同速だった。

### 1. ASCII のより速い float パーサ（`fast-float2`）

`str::parse::<f32>` は同じ fast-float 系なので、トークンを `&str` にしてから `FromStr` しても大差は出ない。効いたのは **空白区切りを挟まず `parse_partial` でバイト列から読む**こと。UTF-8 は行分割だけ `str::lines`（以前、純バイト走査のほうが遅かった）、非 UTF-8 は行のバイト走査。

happy ASCII: **~386ms → 303ms（約 1.3 倍）**。`stl_io` の 2.4s よりまだ約 8 倍速い。支配項は依然として小数パース（happy で約 327 万個の f32）。行スキャンや `Vec` 成長ではない。

これ以上を狙うなら SIMD トークナイズか、生成器側の固定幅前提（汎用 STL では難しい）。ASCII 1.0 の上限は happy 級で、lucy ASCII は作らない方針のままでよい。

### 2. lucy 級バイナリの展開 + AABB を Rayon

250k 三角形以上、16 384 三角形チャンク。出力は `with_capacity` + `set_len`（lucy の 963 MB をゼロ埋めしない）。AABB はチャンク内で取り、最後にマージ。

| ファイル |                   以前（直列） |    今 | `stl_io` |
| -------- | -----------------------------: | ----: | -------: |
| happy    |                          ~13ms | 4.8ms |     11ms |
| subdiv1  |                          ~51ms |  21ms |     44ms |
| lucy     | （未計測・比例なら ~300ms 超） | 168ms |    278ms |

**入れる価値あり。** 小さい bunny は閾値未満のまま直列。happy_res2（293k）でも `stl_io` より速い。初回のスレッドプール生成はウォームアップに含めた。製品の初回オープンでは大きいファイルの最初だけプール起動コストが乗る。

STL バイナリの並列は「固定長 50 バイトを分割」。NAS で予定している rayon（継続行を直列でつないでから行を並列）とは別。両方とも `rayon` 依存で足せる。

### 3. GPU アップロード

CPU スープはファイル座標のまま。AABB 中心は `origin` に残し、GPU バッファへの mapped 書き込みで引く。`create_buffer_init`（CPU 上でもう一回コピー）は使わない。大きいメッシュは書き込みも Rayon。

| 規模         | parse |  soup |       gpu | 製品に近い合計 soup+gpu |
| ------------ | ----: | ----: | --------: | ----------------------: |
| happy 1.1M   |   5ms |   9ms |      12ms |                   ~21ms |
| subdiv1 4.4M |  21ms |  33ms |      81ms |                  ~114ms |
| lucy 28M     | 168ms | 243ms | **782ms** |               **~1.0s** |

lucy では **パースより GPU 載せが支配的**。原点引きの加減算自体は 963 MB を触る帯域の中に埋もれる。シェーダで origin を引いても、頂点 963 MB を載せること自体は残る。次のレバーは「全三角形を一度に載せない」（C-lite の近景 200 万）。2M 三角形なら GPU は数十 ms 規模（happy のほぼ 2 倍）と見てよい。

ASCII happy の gpu 44ms は、同じ枚数のバイナリ happy 12ms と食い違う。lucy の直後で VRAM が荒れているノイズの可能性が高く、ASCII 固有コストではない。

## ボトルネック（今の設計で残るもの）

1. **バイナリ lucy 全載せ**: GPU への 1 GB 近い VERTEX。パースを倍速にしても「開けて回す」は載るまで待つ。
2. **ASCII**: 小数パース。自前でも happy で 0.3s。1.0 の想定上限なので許容。
3. **CPU 常駐**: スープがファイル座標の `Vec`（lucy 963 MB）＋ GPU 複製。表示だけなら載せたあと CPU を捨てられるが、再アップロードや寸法には必要。今は持ったまま。
4. **mmap**: `GlobalAlloc` に出ない。lucy はファイル 1.34 GB がアドレス空間に乗る（ページフォルトは `soup` 側に含まれる）。

溶接しない（スープのまま）判断は時間・割り当てとも正しい。`stl_io_idx` は happy で 12ms 対 147ms。

## コード側に残した定数

- 並列化閾値: 250 000 三角形（`src/stl.rs` / `src/gpu.rs`）
- バイナリチャンク: 16 384 三角形

NAS 未実装。ここの rayon は STL バイナリ展開と GPU の原点引きだけ。

## まだやっていないこと

- シェーダ側 origin（CPU の per-vertex 減算をやめる）。帯域律速なので効果は小さい見込み
- アップロード後に CPU `Vec` を捨てる
- lucy をプロキシだけ先に載せる（C-lite）
- GPU 時間の安定化（ファイル順・VRAM）
- RSS / 専用 VRAM カウンタ（`GlobalAlloc` では無理）
