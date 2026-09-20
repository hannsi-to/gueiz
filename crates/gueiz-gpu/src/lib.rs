//! 2D と 3D で共通の、GPU を触る層。
//!
//! ここに置くかどうかの基準は 1 つだけです。**そのコードが「次元」を口にするか。**
//!
//! | 何をしているか | 次元 | 置き場所 |
//! |---|---|---|
//! | 確保する・束ねる・提示する | 言わない | ここ |
//! | できあがった画像を加工する | 言わない | ここ |
//! | 形を作る | 言う | `gueiz-2d` / `gueiz-3d` |
//! | 光を当てる・奥行きを解く | 言う | `gueiz-3d` |
//!
//! [`buffer`] のヒープも [`post`] のポストも [`renderer`] のサーフェス管理も、
//! 2D か 3D かを知る必要がありません。逆にテッセレーションやライティングは
//! 知る必要があるので、ここには置きません。
//!
//! # ここに無いもの
//!
//! **描画の本体（`DrawManager`）はここにありません。** 2D と 3D で
//! 構造は似ていますが、カリング（円 vs 錐台）・深度（無し vs 有り）・
//! 並べ替え（奥から vs 手前から）の違いが本質的で、パラメータで吸収できません。
//! 実物が 2 つ揃ってから、本当に同じだった部分だけを引き上げます。

pub mod atlas;
pub mod binding;
pub mod buffer;
pub mod camera;
pub mod error;
pub mod instance;
pub mod math;
pub mod msaa;
pub mod pipeline_state;
pub mod pool;
pub mod post;
pub mod renderer;
pub mod resource;
pub mod sprite;
pub mod texture;
pub mod vertex;

pub use bytemuck;
pub use wgpu;
