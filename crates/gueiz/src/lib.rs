//! gueiz グラフィックエンジンの窓口。
//!
//! 個々のクレートを 1 つずつ書かなくて済むように、まとめて再輸出しています。
//!
//! ```toml
//! [dependencies]
//! gueiz = { path = "crates/gueiz" }
//! ```
//!
//! # 三角形を 1 つ
//!
//! README に載せているものと同じです。ここに置いてあるので、
//! **API が変われば `cargo test` で落ちます。**
//!
//! ```no_run
//! use gueiz::two_d::camera::Camera;
//! use gueiz::two_d::object::{self, DrawManager, instance};
//! use gueiz::two_d::paint_type::PaintType;
//! use gueiz::two_d::vertex::Vertex;
//!
//! # fn run(draw_manager: &mut DrawManager) {
//! let mut triangle = object::create_object("Triangle");
//!
//! triangle.begin(PaintType::Fill);
//! triangle.put_vertex(Vertex::new_position_color(  0.0,   0.0, 0.0, 1.0, 0.0, 0.0, 1.0));
//! triangle.put_vertex(Vertex::new_position_color(100.0,   0.0, 0.0, 1.0, 0.0, 0.0, 1.0));
//! triangle.put_vertex(Vertex::new_position_color(100.0, 100.0, 0.0, 1.0, 0.0, 0.0, 1.0));
//! triangle.end();
//!
//! triangle.camera(Camera::orthographic_2d(1280.0, 720.0));
//! triangle.instance(instance::create_instance().translate(40.0, 40.0, 0.0));
//!
//! draw_manager.register(triangle);
//! # }
//! ```
//!
//! # どれを使うか
//!
//! | 作るもの | 見るところ |
//! |---|---|
//! | 平面の図形・文字・UI | [`two_d`] |
//! | 立体のメッシュ | [`three_d`] |
//! | 窓を開く | [`window`] |
//! | 上の 2 つに共通の土台 | [`gpu`] |
//!
//! [`two_d`] と [`three_d`] は [`gpu`] の中身をそのまま再輸出しているので、
//! `gueiz::two_d::buffer` のようにどちらからでも書けます。
//! どこに何を置くかの基準は [`gpu`] を参照。
//!
//! # 例
//!
//! 使い方は `crates/gueiz/examples/` にあります。**ウィンドウを開かない例**は、
//! 描いた画素を読み戻して自分で確かめるので、そのまま動く手本になります。
//!
//! ```sh
//! cargo run -p gueiz --example gueiz_2d_test               # 総合デモ（窓が開く）
//! cargo run -p gueiz --example text_format_test            # 文字の書式
//! cargo run -p gueiz --example text_format_test -- --window # 結果を目で見る
//! ```

/// 平面の描画。形を作って、並べて、描く。
pub use gueiz_2d as two_d;

/// 立体の描画。メッシュを並べて、奥行きを解いて、光を当てる。
pub use gueiz_3d as three_d;

/// 2D と 3D で共通の、GPU を触る層。
pub use gueiz_2d::gueiz_gpu as gpu;

/// 窓を開く。winit の薄い包み。
pub use gueiz_window as window;

pub use wgpu;
