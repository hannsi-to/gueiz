//! UI を組む基盤。
//!
//! **見た目は決めません。** 色・丸み・余白の値はすべて
//! [`theme::Theme`] に預けてあり、ここにあるのは
//! 「木をどう持つか」「どう測るか」「入力をどこへ届けるか」
//! 「描くものをどう GPU へ渡すか」だけです。
//!
//! # 全体の形
//!
//! ```text
//!                        ┌──────────────┐
//!   InputEvent ─────────▶│              │
//!   (呼ぶ側が詰め替える)   │     Gui      │
//!                        │  ┌────────┐  │
//!   TextMeasure ────────▶│  │ Widget │  │──▶ DisplayList ──▶ GuiRenderer ──▶ DrawManager
//!   (測る周回だけ借りる)   │  │  Tree  │  │     (命令の列)      (差分を取る)      (GPU)
//!                        │  └────────┘  │
//!   Theme ──────────────▶│              │──▶ Action（押された、値が動いた）
//!                        └──────────────┘
//! ```
//!
//! | 層 | 役 |
//! |---|---|
//! | [`geometry`] [`color`] | 位置・大きさ・色。ここだけは誰でも使う |
//! | [`event`] | 入力の型。**プラットフォームを知らない** |
//! | [`id`] [`tree`] | ウィジェットの入れ物と、それを指す鍵 |
//! | [`layout`] | 測り方の決まり（制約と 2 周） |
//! | [`widget`] | ウィジェットが満たす約束 |
//! | [`painter`] | 描くものの記録 |
//! | [`theme`] | 見た目の値を引く表 |
//! | [`Gui`] | 上を全部回す人 |
//! | [`render`] | 記録を [`DrawManager`](crate::draw_manager::DrawManager) に移す |
//! | [`container`] [`window_frame`] | 並べる箱と、浮いた窓 |
//! | [`button`] | 押せるもの |
//! | [`scroll_area`] | はみ出した中身を送って見せる箱 |
//! | [`text_field`] | 1 行の入力欄 |
//! | [`dropdown`] | 畳んだ一覧から 1 つ選ぶもの |
//!
//! # 決めた 5 つのこと
//!
//! ## 1. 木は残す（即時モードにしない）
//!
//! 毎フレーム UI を組み直す作り（即時モード）にすると、
//! **[`DrawManager`](crate::draw_manager::DrawManager) の取り柄が消えます。**
//! あれは「書き換えられた図形だけ送り直す」ことで、動かない図形を
//! 最初のフレーム以降 0 円にする仕組みです。毎フレーム組み直せば、
//! 毎フレーム全部が「書き換えられた図形」になります。
//!
//! 木を残す代わりに、状態をどこに置くかを決める必要があります。
//! → **触られている印は木が持ち（[`widget::WidgetState`]）、
//! 中身はウィジェットが持つ。**
//!
//! ## 2. 親が子を所有せず、平らな枠に並べる
//!
//! 下書きの `items: Vec<Box<dyn ItemTrait>>` をやめた理由は
//! [`tree`] の冒頭に書いてあります。短く言えば、
//!
//! - 親を `&mut` で握ったまま子へ降りられない（レイアウトが書けない）
//! - 「いま押されているもの」をフレームをまたいで指せない
//! - 外のコードから特定のウィジェットを掴めない
//!
//! の 3 つです。[`id::WidgetId`] は世代付きなので、消えたものを指した
//! 古い鍵が別のウィジェットに化けることがありません。
//!
//! ## 3. 測るのは 2 周、位置は子に見せない
//!
//! [`layout`] のとおり、`measure`（下から上）→ `arrange`（上から下）です。
//! **どの周回でもウィジェットは自分の画面上の位置を知りません。**
//! 位置を足すのは [`painter::Painter`] と [`ArrangeContext`] です。
//! これを守ると、同じウィジェットをどこに置いても同じように測れます。
//!
//! ## 4. ウィジェットは GPU を触らない
//!
//! 描くものは [`painter::DisplayList`] に**並べるだけ**で、
//! [`render::GuiRenderer`] が図形に移します。こうしてあるので、
//!
//! - 奥から手前の順は木を辿った順で勝手に決まる
//! - 前のフレームと見比べて、変わったものだけ積み直せる
//! - ウィジェットの試験に GPU が要らない（このモジュールの試験は全部そう）
//!
//! ## 5. 結果はコールバックではなく溜める
//!
//! 「押されたら呼ぶ」を閉包で渡すと、閉包の中から UI を触るために
//! `&mut` が二重に要って必ず詰まります。代わりに
//! [`EventContext::emit`] で [`widget::Action`] を積み、
//! 呼ぶ側が [`Gui::drain_actions`] でまとめて受け取ります。
//!
//! # 1 フレーム
//!
//! ```no_run
//! # use gueiz_2d::camera::Camera;
//! # use gueiz_2d::draw_manager::DrawManager;
//! # use gueiz_2d::font::Font;
//! # use gueiz_2d::gui::context::FontMeasure;
//! # use gueiz_2d::gui::event::InputEvent;
//! # use gueiz_2d::gui::geometry::Size;
//! # use gueiz_2d::gui::render::GuiRenderer;
//! # use gueiz_2d::gui::widget::ActionKind;
//! # use gueiz_2d::gui::Gui;
//! # fn frame(
//! #     gui: &mut Gui,
//! #     renderer: &mut GuiRenderer,
//! #     draw_manager: &mut DrawManager,
//! #     font: &Font,
//! #     events: Vec<InputEvent>,
//! #     viewport: Size,
//! # ) {
//! for event in events {
//!     gui.handle_input(event);
//! }
//!
//! // 書体は測る周回のあいだだけ借りる。`Gui` は書体を持たない。
//! gui.layout(viewport, &FontMeasure::new(font));
//!
//! let list = gui.paint();
//! let camera = Camera::orthographic_2d(viewport.width, viewport.height);
//! renderer.sync(draw_manager, list, camera, Some(font));
//!
//! for action in gui.drain_actions() {
//!     if let ActionKind::Clicked { .. } = action.kind {
//!         println!("{} が押された", action.widget);
//!     }
//! }
//! # }
//! ```
//!
//! **最初のフレームだけ、入力を流す前に [`Gui::layout`] を 1 度呼んでください。**
//! 当たり判定は前のフレームの置き場所を使うので、1 度も測っていないと
//! どこにも当たりません。
//!
//! # ここに無いもの
//!
//! 基盤だけなので、部品はほとんどありません。[`container::Stack`] と
//! [`container::Floating`]、それに [`window_frame::WindowFrame`] だけです。
//! ボタン・入力欄・つまみ・一覧は、[`Widget`] を実装して足してください。
//! 必要なものは揃っています。
//!
//! - 押された: [`widget::WidgetState::armed`] と [`widget::ActionKind::Clicked`]
//! - 引っぱる: [`widget::Behavior::captures_pointer`] と
//!   [`event::Event::PointerMove`] の `delta`
//! - 鍵盤: [`widget::Behavior::focusable`] と [`event::Event::Text`]
//! - はみ出しを隠す: [`widget::Behavior::clips_children`]
//! - 送り: [`event::ScrollDelta`] と [`painter::Painter::with_offset`]

pub mod button;
pub mod color;
pub mod container;
pub mod context;
pub mod dropdown;
pub mod event;
pub mod geometry;
pub mod id;
pub mod layout;
pub mod painter;
pub mod render;
pub mod scroll_area;
pub mod text_field;
pub mod theme;
pub mod tree;
pub mod widget;
pub mod window_frame;

mod driver;

pub use context::{ArrangeContext, EventContext, MeasureContext, PaintContext};
pub use driver::Gui;
pub use widget::Widget;
