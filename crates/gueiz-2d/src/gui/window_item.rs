use std::time::Instant;
use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use crate::draw_manager::DrawManager;
use crate::gui::context_menu::ContextMenuRequest;
use crate::object::Object;
use crate::resource::Resources;

/// 組み直した形の頂点を、登録済みの図形に入れ直す。
///
/// 図形を登録し直すと [`DrawManager`] に名前が溜まるので、頂点だけを差し替えます。
/// カメラ・複製・ずらし・効果は `object` に残ったままです。
pub(crate) fn replace_outline(object: &mut Object, built: &Object) {
    object.begin(built.paint_type());
    for vertex in built.vertices() {
        object.put_vertex(*vertex);
    }
    object.end();
}

pub struct WindowItemArguments1<'a> {
    pub draw_manager: &'a mut DrawManager,
    pub resources: &'a Resources,
    pub register_name: &'a String,
    pub counter: i32,
    pub item_x: f32,
    pub item_y: f32,
    pub width: f32,
    pub surface_size: SurfaceSize,
    pub scale_mode: ScaleMode,
    pub design: [f32; 2],
}


pub struct WindowItemArguments2<'a> {
    pub draw_manager: &'a mut DrawManager,
    pub resources: &'a Resources,
    pub register_name: &'a String,
    pub counter: i32,
    pub item_x: f32,
    pub item_y: f32,
    pub width: f32,
    pub scale_mode: ScaleMode,
}

/// 窓の中に置く部品。
///
/// # 座標
///
/// `item_x` / `item_y` は**窓を組んだときの左上を基準にした**座標です。窓を動かしても
/// 変わりません。窓を動かしたぶんは、窓が [`WindowItem::object_names`] の図形すべてに
/// [`Object::translate`](crate::object::Object::translate) で掛けます。部品の側で
/// 置き場所をずらす必要はありません。
///
/// # 窓が部品の図形に代わりにすること
///
/// [`WindowItem::object_names`] で返した図形には、窓が次のことをします。
///
/// - 窓を動かしたら、同じだけずらす
/// - 描き先の大きさが変わったら、カメラを貼り直す
/// - 窓の当たり判定で、窓自身の図形として扱う（縁の取っ手を、別の窓とまちがえない）
///
/// 窓の大きさが変わったときの組み直しだけは、部品ごとに形が違うので
/// [`WindowItem::rebuild`] で部品にしてもらいます。
pub trait WindowItem {
    /// 図形を組む。返すのは部品の `(幅, 高さ)`。次の部品はこの高さぶん下に置かれる。
    ///
    /// `width` は部品に使える幅（窓の幅から左右の余白を引いたもの）です。
    /// [`WindowItem::rebuild`] に渡す幅と同じ決め方なので、寄せや折り返しを最初から合わせられます。
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32);

    /// 窓の大きさが変わった。**登録済みの図形の中で**組み直す。返すのは新しい `(幅, 高さ)`。
    ///
    /// `width` と座標の決め方は [`WindowItem::create_object`] と同じです。
    ///
    /// 図形を登録し直さないでください。[`DrawManager`] には消す手立てが無いので、
    /// 名前が溜まります。頂点を入れ直すなら
    /// [`Object::begin`](crate::object::Object::begin) から
    /// [`Object::end`](crate::object::Object::end) で、
    /// 字なら [`TextRenderer::clear`](crate::text::TextRenderer::clear) してから書き直します。
    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32);

    /// この部品が登録した図形の名前。字の形
    /// （[`TextRenderer::shape_ids`](crate::text::TextRenderer::shape_ids)）も入れてください。
    ///
    /// ここに無い図形は、窓を動かしても付いてきません。
    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String>;

    /// 描き先の大きさが変わった。カメラは窓が貼り直すので、それ以外にすることがあれば。
    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {

    }

    /// 左を押した。窓の中で、手前がこの窓のときだけ届きます。**取ったら `true`。**
    ///
    /// 取ったら、後ろの部品にも窓の帯にも渡りません。
    /// 見た目を変えるなら、ここで図形を書き換えて構いません。
    fn mouse_left_pressed(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    fn mouse_right_pressed(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    /// 左を放した。**どこで放しても、すべての部品に届きます**（押したまま外へ出たのを戻せるように）。
    /// 何か変わったら `true`。
    fn mouse_left_released(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    fn mouse_right_released(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    /// 指が動いた。窓を掴んでいる・縁を引いているあいだは届きません。
    /// 見た目が変わったら `true`（呼ぶ側が描き直します）。
    fn mouse_moved(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    // --- キーボード ---
    //
    // キーと文字は、フォーカスを持っている部品（`has_focus` が `true`）にだけ届きます。
    // 図形を書き換えて字の形が増えたら `true` を返してください。窓が名前を集め直し、
    // 新しい図形にもずらしと覆いを掛けます。

    /// キーと文字を受け取りたいか。押されてフォーカスを取ったら `true` を返すようにする。
    fn has_focus(&self) -> bool {
        false
    }

    /// フォーカスを手放す。ほかの部品や窓の外が押されたときに窓が呼ぶ。見た目が変わったら `true`。
    fn unfocus(&mut self, _draw_manager: &mut DrawManager) -> bool {
        false
    }

    /// 文字が打たれた（IME で確定した文字も）。
    fn text_input(&mut self, _draw_manager: &mut DrawManager, _resources: &Resources, _text: &str) -> bool {
        false
    }

    /// IME で変換中の文字が変わった。`cursor` は変換中の文字の中のバイト位置。空なら変換が終わった。
    fn ime_preedit(&mut self, _draw_manager: &mut DrawManager, _resources: &Resources, _text: &str, _cursor: Option<usize>) -> bool {
        false
    }

    /// 文字を書き換えるキーが押された。Shift や Ctrl は [`WindowItem::modifiers_changed`] で届いたものを見る。
    fn key_pressed(&mut self, _draw_manager: &mut DrawManager, _resources: &Resources, _key: EditKey) -> bool {
        false
    }

    /// Shift や Ctrl の押し具合が変わった。フォーカスが無くても、すべての部品に届く
    /// （Shift を押しながらのクリックで範囲を広げるのにも使うため）。
    fn modifiers_changed(&mut self, _modifiers: KeyModifiers) {}

    /// 選んでいる文字。コピーと切り取りに使う。
    fn selected_text(&self) -> Option<String> {
        None
    }

    /// クリップボードから貼り付ける。打った文字と違い、1 回で元に戻せるようにする。
    fn paste(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        self.text_input(draw_manager, resources, text)
    }

    /// 窓を組んだ場所からのずれが変わった。部品が自分で覆いを掛けるときなど、
    /// ワールドでの位置が要るときに使う。
    fn offset_changed(&mut self, _draw_manager: &mut DrawManager, _offset: [f32; 2]) {}

    /// 字を打つ位置（`[x, y, 幅, 高さ]`、組んだときの座標）。IME の候補を出す場所に使う。
    fn caret_area(&self) -> Option<[f32; 4]> {
        None
    }

    /// IME（日本語入力など）を使いたいか。パスワード欄のように、変換させたくない部品は `false` にする。
    fn wants_ime(&self) -> bool {
        self.has_focus()
    }

    /// 右クリックされた。この部品の上なら、どのメニューをどんなようすで開くかを返す（窓が開く）。
    fn context_menu(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> Option<ContextMenuRequest> {
        None
    }

    /// 時間で見た目を変える（キャレットの点滅、引いたままの自動スクロールなど）。
    ///
    /// 窓の [`update`](crate::gui::window_frame::WindowFrame::update) から、描く前に毎回呼ばれます。
    /// 次に呼んでほしい時刻を返すと、呼ぶ側はその時刻に描き直しを頼みます。
    fn animate(&mut self, _draw_manager: &mut DrawManager, _resources: &Resources, _now: Instant) -> Animation {
        Animation::default()
    }

    /// その画素の位置で出したい指の形。この部品の上でなければ `None`。
    ///
    /// 窓の中で、手前がこの窓のときだけ聞かれます。
    fn cursor_shape(&self, _draw_manager: &DrawManager, _x: f32, _y: f32) -> Option<CursorShape> {
        None
    }
}

/// 出したい指の形。
///
/// gui は winit を知らないので、形だけをここで決めます。
/// 実際の窓に映すのは、つなぐ側（`gueiz::winit_gui` など）の仕事です。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub enum CursorShape {
    #[default]
    Default,
    /// 押せるもの（ボタンなど）の上。
    Pointer,
    /// 文字を打ち込む欄の上。
    Text,
    /// 窓を掴んで動かしているあいだ。
    Move,
    /// 左右の縁。
    ResizeHorizontal,
    /// 上下の縁。
    ResizeVertical,
    /// 左上と右下の角。
    ResizeNwse,
    /// 右上と左下の角。
    ResizeNesw,
}

/// 文字を書き換えるキー。窓の外の入力（winit など）から、呼ぶ側が直して渡す。
///
/// Shift（範囲を広げる）と Ctrl（単語ごと）は [`KeyModifiers`] で別に届きます。
/// コピー・切り取り・貼り付けはクリップボードが要るので、ここには無く、
/// 窓の [`selected_text`](crate::gui::window_frame::WindowFrame::selected_text) などを使います。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum EditKey {
    Backspace,
    Delete,
    Left,
    Right,
    /// 1 行なので、先頭へ。
    Up,
    /// 1 行なので、末尾へ。
    Down,
    Home,
    End,
    Enter,
    Escape,
    /// すべて選ぶ（Ctrl+A）。
    SelectAll,
    /// 元に戻す（Ctrl+Z）。
    Undo,
    /// やり直す（Ctrl+Y・Ctrl+Shift+Z）。
    Redo,
}

/// 時間で見た目を変えた結果。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Animation {
    /// 字を並べ直した（字の形が増えたかもしれない）。窓が名前を集め直す。
    pub relaid_out: bool,
    /// 次に呼んでほしい時刻。もう動かすものが無ければ `None`。
    pub next: Option<Instant>,
}

/// 押しているキーのうち、文字の書き換え方を変えるもの。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub struct KeyModifiers {
    /// 選んでいる範囲を広げる（Shift）。
    pub shift: bool,
    /// 単語ごとに動かす・消す（Windows と Linux は Ctrl、macOS は Option）。
    pub word: bool,
}
