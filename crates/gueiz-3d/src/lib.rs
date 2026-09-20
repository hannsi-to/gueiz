//! 3D の描画。メッシュを並べて、奥行きを解いて、光を当てる。
//!
//! GPU を触る共通の層は [`gueiz_gpu`] にあり、ここからそのまま使えるように
//! 再輸出しています。ヒープ・テクスチャ・サーフェス・ポストは 2D と同じものです。
//!
//! # 2D と本当に違ったところ
//!
//! 素直に書いてみた結果、[`draw_manager`] の中で 4 点が違いました。
//! どれもパラメータでは吸収できず、共通化すると両方が歪みます。
//!
//! | | 2D | 3D |
//! |---|---|---|
//! | 頂点の渡し方 | ストレージバッファ + 番号 | **頂点バッファ + 索引** |
//! | カリング | 境界円 × クリップ矩形 | **境界球 × 錐台 6 面（世界空間）** |
//! | 奥行き | 深度なし。z は並べ替えの鍵 | **深度バッファ** |
//! | 行列 | `camera * object` を畳む | **世界行列とカメラを分ける** |
//!
//! 逆に、**骨組みはそのまま同じでした**。形のプール、複製の素データ、
//! コンピュートで行列を組んで詰める、`atomicAdd` で数える、
//! インダイレクトで 1 回投げる。ここは引き上げる価値があります。

pub mod camera;
pub mod draw_manager;
pub mod mesh;
pub mod object;

pub use camera::Camera3d;
pub use draw_manager::{DrawManager3d, DrawManager3dDescriptor};
pub use mesh::{Mesh, Vertex3d};
pub use object::{Instance3d, Object3d, create_instance, create_object};

// 共通層。`gueiz_3d::buffer` のような書き方がそのまま通る。
pub use gueiz_gpu::{
    binding, buffer, error, math, msaa, pipeline_state, post, renderer, sprite, texture,
};

pub use gueiz_gpu;

pub use bytemuck;
pub use wgpu;
