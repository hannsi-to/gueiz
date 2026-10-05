# gueiz

Rust + [wgpu](https://wgpu.rs/) の描画エンジン。**図形が何種類あっても、ドローコールは 1 回**。

形の登録も、複製の配置も、カリングも GPU 側で解きます。CPU がやるのは
「変わったものだけを送る」ことだけです。

```rust
let mut triangle = object::create_object("Triangle");

triangle.begin(PaintType::Fill);
triangle.put_vertex(Vertex::new_position_color(  0.0,   0.0, 0.0, 1.0, 0.0, 0.0, 1.0));
triangle.put_vertex(Vertex::new_position_color(100.0,   0.0, 0.0, 1.0, 0.0, 0.0, 1.0));
triangle.put_vertex(Vertex::new_position_color(100.0, 100.0, 0.0, 1.0, 0.0, 0.0, 1.0));
triangle.end();

triangle.camera(Camera::orthographic_2d(1280.0, 720.0));
triangle.instance(instance::create_instance().translate(40.0, 40.0, 0.0));

draw_manager.register(triangle);
```

<sub>このコードは
[`crates/gueiz/src/lib.rs`](crates/gueiz/src/lib.rs) にも置いてあり、
`cargo test` でコンパイルを確かめています。古くなったら落ちます。</sub>

> **開発中です。** API は予告なく変わります。`gueiz-sound` はまだ中身がありません。

## 使う

crates.io にはまだ出していません。**git から直接引いてください。**

```toml
[dependencies]
gueiz = { git = "https://github.com/hannsi-to/gueiz.git" }
```

```rust
use gueiz::two_d::camera::Camera;
use gueiz::two_d::object::{self, instance};
use gueiz::two_d::paint_type::PaintType;
use gueiz::two_d::vertex::Vertex;
```

| 入口 | 中身 |
|---|---|
| `gueiz::two_d` | 平面の図形・文字・エフェクト |
| `gueiz::three_d` | 立体のメッシュ |
| `gueiz::window` | 窓を開く |
| `gueiz::gpu` | 2D と 3D で共通の土台 |

### 版を固定する

**既定のままだと既定ブランチを追いかけます。** API がまだ動いているので、
仕事で使うなら固定してください。

```toml
gueiz = { git = "https://github.com/hannsi-to/gueiz.git", tag = "v0.1.0" }
gueiz = { git = "https://github.com/hannsi-to/gueiz.git", rev = "80738f7" }
```

固定しない場合でも、`Cargo.lock` があるうちは勝手には動きません。
上げたいときは `cargo update -p gueiz` です。

## 動かしてみる

Rust 2024 edition（1.85 以降）が要ります。

```bash
cargo run -p gueiz --example gueiz2d_test
```

窓が開いて、画面いっぱいの四角形が出ます。窓を引っぱっても追従します。

バックエンドを選ぶこともできます。

```bash
cargo run -p gueiz --example gueiz2d_test -- dx12
```

## クレートの地図

```
gueiz-core     依存なし。DirtyFlag のような小物
gueiz-gpu      2D と 3D で共通の、GPU を触る層
gueiz-2d       ├─ 2D の描画（テッセレーション・書式つき文字・エフェクト）
gueiz-3d       └─ 3D の描画（メッシュ・深度・ライティング）
gueiz-window   winit の薄い包み。複数ウィンドウ
gueiz-sound    未着手
gueiz          全部をまとめる窓口 + 例
```

置き場所の基準は 1 つだけです。**そのコードが「次元」を口にするか。**
ヒープもテクスチャもサーフェスも 2D か 3D かを知る必要がないので `gueiz-gpu`、
テッセレーションやライティングは知る必要があるので `gueiz-2d` / `gueiz-3d`。

詳しくは [docs/architecture.md](docs/architecture.md)。

## できること

### 2D（`gueiz-2d`）

- **塗りと線** — 穴・離れた輪郭・自己交差しない任意の多角形を耳刈り取りで三角形に開く
- **文字** — TrueType の輪郭をそのまま図形にする。同じ字は形を 1 つだけ持つ
- **書式** — 色・太字・斜体・下線・影・縁取りなどを文字列に埋め込む（[`format`](crates/gueiz-2d/src/format.rs)）
- **エフェクト** — 段（形 / 変換 / 色）ごとに積む VFX Graph 風の仕組み。WGSL を自前で書ける
- **スプライト** — 大きさの違う絵を 1 枚に詰め込むアトラス。Z による重なり順
- **実行中の編集** — 頂点の追加・削除・変更がその場で GPU まで届く
- **当たり判定** — 点が図形の中にあるか。描いた三角形で見るので、線も穴も見たとおり

### 3D（`gueiz-3d`）

- 頂点バッファ + 索引、深度バッファ、錐台カリング、平行光源

### 共通（`gueiz-gpu`）

- 境界タグ方式のフリーリストで確保する GPU ヒープ
- マルチサンプル、ポストプロセス、生成的ハンドルのリソース置き場
- 窓の大きさへの合わせ方の切り替え（[`ScaleMode`](crates/gueiz-gpu/src/camera.rs)）

## 例

**ウィンドウを開かないもの**は、描いた画素を読み戻して自分で確かめ、
合っていなければ落ちます。CI にそのまま置けます。
`-- --window` を付けると、結果を窓に出して見比べられます。

```bash
cargo run -p gueiz --example text_format_test            # 確かめるだけ
cargo run -p gueiz --example text_format_test -- --window # 目で見る
```

| 例 | 見るもの |
|---|---|
| `text_test` | 文字の輪郭・穴・離れた輪郭 |
| `text_format_test` | 文字列に埋めた書式（色・太字・斜体・線・影…） |
| `scale_mode_test` | 窓の大きさへの合わせ方（絶対 / 相対） |
| `sprite_test` | スプライトの絵と Z の重なり順 |
| `atlas_test` | 大きさ違いの絵の詰め込み・にじませ・縮小段 |
| `effect_test` | 図形ごとのエフェクト |
| `post_test` | 画面全体のエフェクト |
| `resource_test` | フォント・絵・自前エフェクトの置き場 |
| `edit_test` | 実行中の頂点の足し引き |
| `dirty_test` | 変わっていない複製を送り直さない経路 |
| `cull_test` | GPU カリング |
| `mesh3d_test` | 3D の描画 |
| `hit_test` | 当たり判定が描いた画素と一致するか |
| `buffer_heap_test` | GPU ヒープ |

**ウィンドウを開くもの**

| 例 | 中身 |
|---|---|
| `gueiz2d_test` | いちばん短い絵。画面いっぱいの四角形 1 つ |
| `gueiz_window_test` | `gueiz-window` の複数ウィンドウ |
| `wgpu_test` / `winit_test` | 依存ライブラリだけの最小構成 |

**測るもの**

```bash
cargo run --release -p gueiz --example bench_draw   # 描画の各段の時間
cargo run --release -p gueiz --example bench_fill   # 塗りの時間
```

`--release` を付けないと桁が変わります。

## 開発

```bash
cargo test --workspace         # 435 件。GPU は要らない
cargo clippy --workspace --all-targets
cargo doc --workspace --open   # API ドキュメント（日本語）
```

単体テストは **GPU を触りません。** 行列・テッセレーション・文字の並べ方・
書式の解釈といった、計算だけで確かめられるところを見ています。
GPU まで届いているかは例のほうが見ます。この 2 段構えなので、
`cargo test` はどこでも通り、例はグラフィックスのある機械で走らせます。

コードのドキュメントは日本語です。`cargo doc` で読めます。

## ライセンス

未定。
