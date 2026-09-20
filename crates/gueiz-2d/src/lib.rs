//! 2D の描画。形を作って、並べて、描く。
//!
//! GPU を触る共通の層は [`gueiz_gpu`] にあり、ここからそのまま使えるように
//! 再輸出しています。どちらに何を置くかの基準は [`gueiz_gpu`] を参照。
//!
//! このクレートに残っているのは、**2D だと言っているもの**だけです。
//! 輪郭のテッセレーション、塗りと線の指定、輪郭を記録する `Object`、
//! そして 2D 用の `DrawManager`。

pub mod effect;
pub mod font;
pub mod format;
pub mod object;
pub mod paint_type;
pub mod resource;
pub mod tessellate;
pub mod text;

// 共通層。`crate::buffer` のような書き方がそのまま通る。
pub use gueiz_gpu::{
    atlas, binding, buffer, camera, error, math, msaa, pipeline_state, post, renderer, sprite,
    texture, vertex,
};

pub use gueiz_gpu;

pub use bytemuck;
pub use wgpu;
