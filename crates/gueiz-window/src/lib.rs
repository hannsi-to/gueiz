//! 窓を開く。[`winit`] の薄い包み。
//!
//! # なぜ包むのか
//!
//! `winit` の `ApplicationHandler` は、窓・イベント・制御フローを
//! 呼ぶ側にまとめて預けます。**窓を複数持つと、その管理が
//! 呼ぶ側に散らかります。**
//!
//! ここでは [`window::Application`] が窓の一覧を持ち、呼ぶ側には
//! 「窓を作る」「主たる窓を決める」「終わる」だけを見せます。
//! 実装するのは [`window::ApplicationHandler`] の 1 つのメソッドだけです。
//!
//! # 使い方
//!
//! ```no_run
//! use gueiz_window::window::{
//!     Application, ApplicationHandler, ApplicationRunner, WindowDescriptor,
//! };
//!
//! struct MyApplication;
//!
//! impl ApplicationHandler for MyApplication {
//!     // 描き先を作れるようになったら呼ばれる。ここで窓を開く。
//!     fn can_create_surfaces(&mut self, application: &Application) {
//!         let _ = application.create_window(WindowDescriptor {
//!             title: String::from("gueiz"),
//!             width: 1280,
//!             height: 720,
//!             ..Default::default()
//!         });
//!     }
//! }
//!
//! ApplicationRunner::new().run_application(MyApplication).unwrap();
//! ```
//!
//! 動く例は `crates/gueiz/examples/gueiz_window_test.rs`（窓を 2 枚開く）。
//!
//! # 回し方
//!
//! [`window::ApplicationLoopType`] で決めます。既定は `Poll`。
//!
//! | | いつ回るか | 向いているもの |
//! |---|---|---|
//! | `Poll` | 止まらずに回り続ける | 動き続ける絵、ゲーム |
//! | `Wait` | 何か起きるまで眠る | 道具の画面。電池に優しい |
//! | `WaitUntil` | その時刻まで眠る | 決まった間隔で動かすもの |
//!
//! # 描画とのつなぎ
//!
//! このクレートは **GPU を触りません。** `wgpu` にも依存していません。
//! 描き先を作るのに要る生のハンドルは [`raw_window_handle`] から取れます。
//!
//! ```no_run
//! # use gueiz_window::raw_window_handle::HasWindowHandle;
//! # fn run(window: &impl HasWindowHandle) {
//! let handle = window.window_handle().expect("ハンドルが取れる");
//! # let _ = handle;
//! # }
//! ```

pub mod window;
pub mod error;

pub use winit::raw_window_handle;
