//! 右クリックで出すメニュー。
//!
//! # 作り方
//!
//! 項目は [`ContextMenuLabel`]（押せる項目）と [`ContextMenuSlicer`]（区切り線）です。
//! **1 つの項目に要るものは、すべてその項目に書きます。** 出す字、右の列の字
//! （ショートカットなど）、アイコン、右に開くメニュー、選べる条件、押したときにすること。
//! ほかのところを書き換える必要はありません。
//!
//! ```ignore
//! let mut case = ContextMenuLabels::new("Case");
//! case.add_context_menu_item(
//!     ContextMenuLabel::new("大文字に")
//!         .enabled(|state| state.has_selection && state.editable)
//!         .on_event(|event| {
//!             if let Some(text) = event.selected_text() {
//!                 event.replace_selection(&text.to_uppercase());
//!             }
//!         }),
//! );
//!
//! let mut menu = ContextMenuLabels::new(INPUT_TEXT_CONTEXT_MENU);
//! menu.add_context_menu_item(ContextMenuLabel::cut("切り取り"))
//!     .add_context_menu_item(ContextMenuLabel::copy("コピー"))
//!     .add_context_menu_item(ContextMenuLabel::paste("貼り付け"))
//!     .add_context_menu_item(ContextMenuSlicer { color })
//!     .add_context_menu_item(ContextMenuLabel::new("大文字・小文字").sub_menu(case));
//!
//! window_frame.add_context_menu_labels(menu);
//! ```
//!
//! メニューには名前があり、部品は右クリックされたときに
//! [`ContextMenuRequest`] でどのメニューを開くかを窓に伝えます。
//! [`InputText`](crate::gui::input_text::InputText) は [`INPUT_TEXT_CONTEXT_MENU`] を開きます。
//! その名前のメニューを足していなければ、窓が [`ContextMenuLabels::text_input`] を足します。
//!
//! # 押したときにすること
//!
//! [`ContextMenuLabel::on_event`] の関数は、[`ContextMenuEvent`] を受け取ります。
//! 窓・[`DrawManager`]・[`Resources`]・クリップボードはここから使います
//! （関数の中に借りておくことは、Rust の借用の決まりでできないので）。
//!
//! # 見た目
//!
//! [`ContextMenuStyle`]（書体・色・形）を窓の
//! [`set_context_menu_style`](crate::gui::window_frame::WindowFrame::set_context_menu_style) に渡します。
//! 1 つの項目は左から、アイコン・字・右の列の字・右に開く印の順に並びます。
//! 右の列の字は、列の中で左に寄せます。
//!
//! # 描き方
//!
//! [`DrawManager`] には図形を消す手立てが無いので、開くたびに作ると名前が溜まります。
//! そこで窓を組むときに、足されたメニュー（右に開くものも）をすべて 1 回だけ作り、
//! 閉じているあいだは画面のずっと外へ [`Object::translate`] で逃がしておきます。
//! 開くときは動かすだけなので、書体も要りません。
//!
//! 窓の部品より手前に出すため、重なり順（z）を大きくし、右に開くメニューはさらに手前にします。
//! 部品ではないので、窓の覆いで削られることもありません。

use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::effect::Block;
use crate::font::Font;
use crate::gui::clipboard::Clipboard;
use crate::gui::window_frame::{QuadType, ThemeColor, WindowFrame};
use crate::gui::window_item::{replace_outline, EditKey};
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::lines::{CurveRepresentationType, Lines};
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::{JointType, PaintType};
use crate::resource::{FontHandle, Resources, TextureHandle};
use crate::text::{measure, TextAlign, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

/// [`InputText`](crate::gui::input_text::InputText) が開くメニューの名前。
pub const INPUT_TEXT_CONTEXT_MENU: &str = "TextInputContextMenu";

/// 窓の部品より手前に出す重なり順。右に開くメニューは、深さぶん足す。
const MENU_Z: f32 = 1000.0;
/// 閉じているあいだに逃がしておく場所。
const OFFSCREEN: f32 = -1.0e6;
/// 右に開くメニューがある項目の印。
const SUB_MENU_MARK: &str = "\u{203A}";

// --- 定義 ---

/// 押したときにする関数。
pub type ContextMenuHandler = Box<dyn FnMut(&mut ContextMenuEvent)>;
/// 選べるかを決める関数。
pub type ContextMenuCondition = Box<dyn Fn(&ContextMenuState) -> bool>;

/// 名前の付いたメニュー。項目を上から順に持つ。
pub struct ContextMenuLabels {
    name: String,
    items: Vec<ContextMenuItem>,
}

impl ContextMenuLabels {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), items: Vec::new() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn items(&self) -> &[ContextMenuItem] {
        &self.items
    }

    /// 項目を下に足す。[`ContextMenuLabel`] も [`ContextMenuSlicer`] も渡せる。
    pub fn add_context_menu_item(&mut self, item: impl Into<ContextMenuItem>) -> &mut Self {
        self.items.push(item.into());
        self
    }

    /// 文字を打ち込む欄の、いつものメニュー（切り取り・コピー・貼り付け・すべて選択）。英語。
    pub fn text_input() -> Self {
        let mut menu = Self::new(INPUT_TEXT_CONTEXT_MENU);
        menu.add_context_menu_item(ContextMenuLabel::cut("Cut"))
            .add_context_menu_item(ContextMenuLabel::copy("Copy"))
            .add_context_menu_item(ContextMenuLabel::paste("Paste"))
            .add_context_menu_item(ContextMenuLabel::select_all("Select All"));
        menu
    }
}

/// メニューの 1 項目。
pub enum ContextMenuItem {
    Label(ContextMenuLabel),
    Slicer(ContextMenuSlicer),
}

impl From<ContextMenuLabel> for ContextMenuItem {
    fn from(label: ContextMenuLabel) -> Self {
        Self::Label(label)
    }
}

impl From<ContextMenuSlicer> for ContextMenuItem {
    fn from(slicer: ContextMenuSlicer) -> Self {
        Self::Slicer(slicer)
    }
}

/// 押せる項目。
///
/// フィールドを直に書いても、[`ContextMenuLabel::new`] から続けて書いても構いません。
/// 直に書くときは `..Default::default()` で残りを埋められます。
#[derive(Default)]
pub struct ContextMenuLabel {
    /// 出す字。
    pub label: String,
    /// 右の列に出す字（ショートカットなど）。列の中で左に寄せる。
    pub sub_title: Option<String>,
    /// 字の左に出す絵。
    pub icon: Option<TextureHandle>,
    /// 指を乗せると右に開くメニュー。あれば押しても `on_event` は呼ばない。
    pub sub_context_menu_labels: Option<ContextMenuLabels>,
    /// 選べるか。`None` ならいつでも選べる。
    pub enabled: Option<ContextMenuCondition>,
    /// 選べないときにどう見せるか。`None` ならメニューの
    /// [`ContextMenuLayout::when_disabled`] に従う。
    pub when_disabled: Option<ContextMenuDisabled>,
    /// 押したときにすること。
    pub on_event: Option<ContextMenuHandler>,
}

impl ContextMenuLabel {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), ..Self::default() }
    }

    pub fn sub_title(mut self, sub_title: impl Into<String>) -> Self {
        self.sub_title = Some(sub_title.into());
        self
    }

    pub fn icon(mut self, icon: TextureHandle) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn sub_menu(mut self, labels: ContextMenuLabels) -> Self {
        self.sub_context_menu_labels = Some(labels);
        self
    }

    pub fn enabled(mut self, condition: impl Fn(&ContextMenuState) -> bool + 'static) -> Self {
        self.enabled = Some(Box::new(condition));
        self
    }

    /// 選べないときにどう見せるか。
    pub fn when_disabled(mut self, when_disabled: ContextMenuDisabled) -> Self {
        self.when_disabled = Some(when_disabled);
        self
    }

    /// 選べないときは消す（[`ContextMenuDisabled::Hide`]）。
    pub fn hide_when_disabled(self) -> Self {
        self.when_disabled(ContextMenuDisabled::Hide)
    }

    pub fn on_event(mut self, handler: impl FnMut(&mut ContextMenuEvent) + 'static) -> Self {
        self.on_event = Some(Box::new(handler));
        self
    }

    /// 切り取り。選んでいて、コピーでき、書き換えられるときだけ選べる。
    pub fn cut(label: impl Into<String>) -> Self {
        Self::new(label)
            .sub_title(shortcut("X"))
            .enabled(|state| state.has_selection && state.can_copy && state.editable)
            .on_event(|event| {
                event.cut();
            })
    }

    /// コピー。選んでいて、コピーできるときだけ選べる。
    pub fn copy(label: impl Into<String>) -> Self {
        Self::new(label)
            .sub_title(shortcut("C"))
            .enabled(|state| state.has_selection && state.can_copy)
            .on_event(|event| {
                event.copy();
            })
    }

    /// 貼り付け。書き換えられるときだけ選べる。
    pub fn paste(label: impl Into<String>) -> Self {
        Self::new(label)
            .sub_title(shortcut("V"))
            .enabled(|state| state.editable)
            .on_event(|event| {
                event.paste();
            })
    }

    /// すべて選択。字があるときだけ選べる。
    pub fn select_all(label: impl Into<String>) -> Self {
        Self::new(label)
            .sub_title(shortcut("A"))
            .enabled(|state| state.has_text)
            .on_event(|event| {
                event.select_all();
            })
    }
}

/// ショートカットの書き方。macOS は Cmd、ほかは Ctrl。
fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("Cmd+{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// 選べない項目をどう見せるか。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub enum ContextMenuDisabled {
    /// 字と絵を薄くして（[`ContextMenuTheme::disabled_alpha`]）、押せなくする。
    #[default]
    Dim,
    /// 消す。下の項目は詰めて並べ、メニューも縮める。
    ///
    /// 消した結果、区切り線が先頭・末尾に来たり 2 本続いたりしたら、その区切り線も消します。
    /// 見える項目が 1 つも無ければ、メニューは開きません。
    Hide,
}

/// 区切り線。
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ContextMenuSlicer {
    pub color: ThemeColor,
}

/// メニューを開いたときの部品のようす。項目が選べるかを決めるのに使う。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub struct ContextMenuState {
    /// 何か選んでいる。
    pub has_selection: bool,
    /// 選んだものを外へ出してよい（パスワード欄では `false`）。
    pub can_copy: bool,
    /// 書き換えられる。
    pub editable: bool,
    /// 字が 1 つでもある。
    pub has_text: bool,
}

/// 右クリックされた部品から窓への頼み。どのメニューを、どんなようすで開くか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuRequest {
    /// [`ContextMenuLabels::name`]。
    pub menu: String,
    pub state: ContextMenuState,
}

/// 項目を押したときに、[`ContextMenuLabel::on_event`] の関数が受け取るもの。
pub struct ContextMenuEvent<'a> {
    pub window_frame: &'a mut WindowFrame,
    pub draw_manager: &'a mut DrawManager,
    pub resources: &'a Resources,
    pub clipboard: &'a mut dyn Clipboard,
}

impl ContextMenuEvent<'_> {
    /// フォーカスを持っている部品で選んでいる文字を切り取る。
    pub fn cut(&mut self) -> bool {
        self.window_frame.cut_to_clipboard(self.draw_manager, self.resources, self.clipboard)
    }

    /// 選んでいる文字をコピーする。
    pub fn copy(&mut self) -> bool {
        self.window_frame.copy_to_clipboard(self.clipboard)
    }

    /// クリップボードの文字を貼り付ける。
    pub fn paste(&mut self) -> bool {
        self.window_frame.paste_from_clipboard(self.draw_manager, self.resources, self.clipboard)
    }

    /// すべて選ぶ。
    pub fn select_all(&mut self) -> bool {
        self.window_frame.key_pressed(self.draw_manager, self.resources, EditKey::SelectAll)
    }

    /// 選んでいる文字。
    pub fn selected_text(&self) -> Option<String> {
        self.window_frame.selected_text()
    }

    /// 選んでいる範囲を `text` に置き換える（選んでいなければ字を打つ位置に入れる）。
    pub fn replace_selection(&mut self, text: &str) -> bool {
        self.window_frame.paste(self.draw_manager, self.resources, text)
    }
}

// --- 見た目 ---

/// メニューの見た目。書体・色・形をまとめたもの。
pub struct ContextMenuStyle {
    pub font: ContextMenuFont,
    pub theme: ContextMenuTheme,
    pub layout: ContextMenuLayout,
}

/// メニューの字の書体。
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ContextMenuFont {
    pub base_font: FontHandle,
    pub base_font_size: f32,
}

/// メニューの色。
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ContextMenuTheme {
    pub fill_color: ThemeColor,
    pub outline_color: ThemeColor,
    pub text_color: ThemeColor,
    /// 右の列の字（ショートカットなど）と、右に開く印の色。
    pub sub_title_color: ThemeColor,
    /// 指が上にある項目の地。
    pub highlight_color: ThemeColor,
    /// 影の色。[`ContextMenuLayout::shadow_offset`] が `None` なら使わない。
    pub shadow_color: ThemeColor,
    /// 選べない項目の字の濃さ（`0..1`）。字の色に掛けます。
    pub disabled_alpha: f32,
}

/// メニューの形と寸法。[`Default`] から変えたいところだけ書き換えてください。
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ContextMenuLayout {
    /// 地と縁取りの形。[`QuadType::Rounded`] で角を丸める。
    pub quad_type: QuadType,
    /// 字の左右（`[0]`）と、項目の並びの上下（`[1]`）の余白。
    pub padding: [f32; 2],
    /// 1 項目の高さ。`None` なら書体の大きさの 1.9 倍。
    pub item_height: Option<f32>,
    /// 区切り線 1 本ぶんの高さ。線はそのまん中に引く。
    pub slicer_height: f32,
    /// 字と右の列のあいだ。
    pub column_gap: f32,
    /// アイコンの大きさ。`None` なら 1 項目の高さの 0.6 倍。
    pub icon_size: Option<f32>,
    /// 幅の下限。字がこれより長ければ広がる。
    pub min_width: f32,
    /// 縁取りの太さ。`0` で縁取りなし。
    pub outline_width: f32,
    /// 指が上にある項目の帯を、左右にどれだけ内側へ寄せるか。
    pub highlight_inset: f32,
    /// 指が上にある項目の帯の角の丸み。
    pub highlight_radius: f32,
    /// 影のずれ。`None` で影なし。
    pub shadow_offset: Option<[f32; 2]>,
    /// 選べない項目をどう見せるか（項目で決めていなければ）。
    pub when_disabled: ContextMenuDisabled,
}

impl Default for ContextMenuLayout {
    fn default() -> Self {
        Self {
            quad_type: QuadType::Normal,
            padding: [12.0, 4.0],
            item_height: None,
            slicer_height: 9.0,
            column_gap: 24.0,
            icon_size: None,
            min_width: 120.0,
            outline_width: 1.0,
            highlight_inset: 0.0,
            highlight_radius: 0.0,
            shadow_offset: None,
            when_disabled: ContextMenuDisabled::Dim,
        }
    }
}

// --- 描く ---

/// 作るときに使う、見た目から出した値。
#[derive(Copy, Clone)]
struct Params {
    theme: ContextMenuTheme,
    layout: ContextMenuLayout,
    text_style: TextStyle,
    font_size: f32,
    item_height: f32,
    icon_size: f32,
    surface_size: SurfaceSize,
    design: [f32; 2],
    scale_mode: ScaleMode,
}

/// 作ったメニュー 1 枚。
struct MenuView {
    name: String,
    z: f32,
    width: f32,
    /// 地と縁取りの角の丸み。開くたびに高さを合わせて組み直すのに使う。
    radius: f32,
    shadow: Option<String>,
    background: String,
    highlight: String,
    outline: Option<String>,
    entries: Vec<EntryView>,
}

/// 作った項目 1 つ。
struct EntryView {
    /// メニューの上端からの位置。
    top: f32,
    height: f32,
    /// 絵など、字でない図形。
    objects: Vec<String>,
    /// 字（[`TextRenderer`] の名前）。形は `group_names` で引く。
    texts: Vec<String>,
    /// 押せる項目なら、その定義（右に開くメニューは抜いてある）。区切り線なら `None`。
    label: Option<ContextMenuLabel>,
    /// 右に開くメニュー。
    sub_menu: Option<usize>,
}

/// 開いているメニュー 1 枚。
#[derive(Debug, Clone)]
struct Opened {
    menu: usize,
    /// 左上（ワールド）。
    origin: [f32; 2],
    /// 指が上にある項目。
    hovered: Option<usize>,
    /// 開いたときの、項目ごとの上端（メニューの左上から）。消した項目は `None`。
    tops: Vec<Option<f32>>,
    /// 開いたときの高さ。消した項目のぶん縮む。
    height: f32,
}

pub(crate) struct ContextMenu {
    style: Option<ContextMenuStyle>,
    /// 足されたメニュー。[`ContextMenu::create`] で作ると空になる。
    definitions: Vec<ContextMenuLabels>,
    /// 作ったメニュー。右に開くものも入る。
    menus: Vec<MenuView>,
    /// 名前で開けるメニュー（足されたもの）と、その番号。
    roots: Vec<(String, usize)>,
    /// 開いているメニュー。後ろほど深い（右に開いたもの）。
    stack: Vec<Opened>,
    /// 開いたときの部品のようす。
    state: ContextMenuState,
    /// 押された項目（メニューの番号・項目の番号）。窓が呼ぶ側の手で関数を呼ぶ。
    pending: Option<(usize, usize)>,
}

impl ContextMenu {
    pub(crate) fn new() -> Self {
        Self {
            style: None,
            definitions: Vec::new(),
            menus: Vec::new(),
            roots: Vec::new(),
            stack: Vec::new(),
            state: ContextMenuState::default(),
            pending: None,
        }
    }

    /// 見た目を変える。[`ContextMenu::create`] より前に。
    pub(crate) fn set_style(&mut self, style: ContextMenuStyle) {
        self.style = Some(style);
    }

    /// 見た目がまだ決まっていなければ、これにする（窓のテーマから作ったもの）。
    pub(crate) fn style_or(&mut self, fallback: impl FnOnce() -> ContextMenuStyle) {
        if self.style.is_none() {
            self.style = Some(fallback());
        }
    }

    /// メニューを足す。同じ名前があれば差し替える。[`ContextMenu::create`] より前に。
    pub(crate) fn add_labels(&mut self, labels: ContextMenuLabels) {
        self.definitions.retain(|definition| definition.name != labels.name);
        self.definitions.push(labels);
    }

    /// その名前のメニューがあるか。
    pub(crate) fn has_menu(&self, name: &str) -> bool {
        self.definitions.iter().any(|definition| definition.name == name)
            || self.roots.iter().any(|(root, _)| root == name)
    }

    fn padding(&self) -> [f32; 2] {
        self.style.as_ref().map_or(ContextMenuLayout::default().padding, |style| style.layout.padding)
    }

    /// 足されたメニューを、右に開くものも含めてすべて作り、閉じた状態にしておく。
    /// 窓を組むときに 1 回だけ呼ぶ。見た目が決まっていないか、書体が無ければ何も作らない。
    pub(crate) fn create(
        &mut self,
        draw_manager: &mut DrawManager,
        resources: &Resources,
        base_name: &str,
        surface_size: SurfaceSize,
        design: [f32; 2],
        scale_mode: ScaleMode,
    ) {
        let Some(style) = self.style.as_ref() else {
            return;
        };
        let Some(font) = resources.font(style.font.base_font) else {
            log::warn!("the context menu font is not in the resources; the menu is not shown");
            return;
        };

        let item_height = style.layout.item_height.unwrap_or((style.font.base_font_size * 1.9).round());
        let params = Params {
            theme: style.theme,
            layout: style.layout,
            text_style: TextStyle::new(style.font.base_font_size),
            font_size: style.font.base_font_size,
            item_height,
            icon_size: style.layout.icon_size.unwrap_or(item_height * 0.6),
            surface_size,
            design,
            scale_mode,
        };

        for labels in std::mem::take(&mut self.definitions) {
            let name = labels.name.clone();
            let index = self.build(draw_manager, resources, &font, &params, base_name, labels, 0);
            self.roots.push((name, index));
        }

        self.close(draw_manager);
    }

    /// メニュー 1 枚を作る。右に開くメニューは先に作る。返すのは作ったメニューの番号。
    #[allow(clippy::too_many_arguments)]
    fn build(
        &mut self,
        draw_manager: &mut DrawManager,
        resources: &Resources,
        font: &Font,
        params: &Params,
        base_name: &str,
        labels: ContextMenuLabels,
        depth: usize,
    ) -> usize {
        let layout = params.layout;
        let theme = params.theme;
        let [padding_x, padding_y] = layout.padding;
        let z = MENU_Z + depth as f32;
        let measure_width = |text: &str| measure(font, text, &params.text_style).width;

        // 右に開くメニューを先に作り、項目から抜いておく。
        let mut items: Vec<(ContextMenuItem, Option<usize>)> = Vec::new();
        for item in labels.items {
            match item {
                ContextMenuItem::Label(mut label) => {
                    let sub_menu = label
                        .sub_context_menu_labels
                        .take()
                        .map(|sub| self.build(draw_manager, resources, font, params, base_name, sub, depth + 1));
                    items.push((ContextMenuItem::Label(label), sub_menu));
                }
                slicer => items.push((slicer, None)),
            }
        }

        // 列の幅を測る。左から、アイコン・字・右の列・右に開く印。
        let label_items = || items.iter().filter_map(|(item, sub)| match item {
            ContextMenuItem::Label(label) => Some((label, sub)),
            ContextMenuItem::Slicer(_) => None,
        });
        let label_width = label_items().map(|(label, _)| measure_width(&label.label)).fold(0.0, f32::max);
        let sub_title_width = label_items()
            .filter_map(|(label, _)| label.sub_title.as_deref())
            .map(measure_width)
            .fold(None, |widest: Option<f32>, width| Some(widest.map_or(width, |widest| widest.max(width))));
        let has_icon = label_items().any(|(label, _)| label.icon.is_some());
        let has_sub_menu = label_items().any(|(_, sub)| sub.is_some());
        let mark_width = measure_width(SUB_MENU_MARK);

        let icon_column = if has_icon { params.icon_size + padding_x * 0.5 } else { 0.0 };
        let label_x = padding_x + icon_column;
        let sub_title_x = label_x + label_width + layout.column_gap;
        let content_width = icon_column
            + label_width
            + sub_title_width.map_or(0.0, |width| layout.column_gap + width)
            + if has_sub_menu { layout.column_gap * 0.5 + mark_width } else { 0.0 };
        let width = (content_width + padding_x * 2.0).max(layout.min_width);
        let mark_x = width - padding_x - mark_width;

        let tops: Vec<f32> = items
            .iter()
            .scan(padding_y, |top, (item, _)| {
                let current = *top;
                *top += match item {
                    ContextMenuItem::Label(_) => params.item_height,
                    ContextMenuItem::Slicer(_) => layout.slicer_height,
                };
                Some(current)
            })
            .collect();
        let height = tops.last().map_or(padding_y, |last| {
            last + match items.last().map(|(item, _)| item) {
                Some(ContextMenuItem::Slicer(_)) => layout.slicer_height,
                _ => params.item_height,
            }
        }) + padding_y;

        let index = self.menus.len();
        let base = format!("{base_name}-menu-{index}");
        let radius = layout.quad_type.radius();

        // 影、地、指の下の帯、縁取り、項目の順に重ねる。
        let shadow = layout.shadow_offset.map(|_| {
            register(draw_manager, params, z, rect(&format!("{base}-shadow"), PaintType::Fill, [0.0, 0.0], [width, height], radius, &theme.shadow_color))
        });
        let background = register(draw_manager, params, z, rect(&base, PaintType::Fill, [0.0, 0.0], [width, height], radius, &theme.fill_color));
        let highlight = register(
            draw_manager,
            params,
            z,
            rect(
                &format!("{base}-highlight"),
                PaintType::Fill,
                [layout.highlight_inset, 0.0],
                [(width - layout.highlight_inset * 2.0).max(0.0), params.item_height],
                layout.highlight_radius,
                &theme.highlight_color,
            ),
        );
        let outline = (layout.outline_width > 0.0).then(|| {
            let stroke = PaintType::Stroke { line_width: layout.outline_width, joint_type: JointType::None, strip: false, dash: None };
            register(draw_manager, params, z, rect(&format!("{base}-outline"), stroke, [0.0, 0.0], [width, height], radius, &theme.outline_color))
        });

        let mut entries = Vec::with_capacity(items.len());
        for (entry_index, ((item, sub_menu), top)) in items.into_iter().zip(tops).enumerate() {
            let entry_base = format!("{base}-{entry_index}");

            match item {
                ContextMenuItem::Slicer(slicer) => {
                    let y = top + layout.slicer_height / 2.0;
                    let line = Lines::new(&format!("{entry_base}-slicer"))
                        .paint_type(PaintType::Stroke { line_width: 1.0, joint_type: JointType::None, strip: false, dash: None })
                        .curve_type(CurveRepresentationType::Normal)
                        .point(vertex(padding_x * 0.5, y, &slicer.color))
                        .last_point(vertex(width - padding_x * 0.5, y, &slicer.color))
                        .end();

                    entries.push(EntryView {
                        top,
                        height: layout.slicer_height,
                        objects: vec![register(draw_manager, params, z, line)],
                        texts: Vec::new(),
                        label: None,
                        sub_menu: None,
                    });
                }
                ContextMenuItem::Label(label) => {
                    let mut objects = Vec::new();
                    let mut texts = Vec::new();
                    let row = [top, params.item_height];

                    if let Some(handle) = label.icon {
                        let size = params.icon_size;
                        let white = ThemeColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
                        let mut icon = rect(
                            &format!("{entry_base}-icon"),
                            PaintType::Fill,
                            [padding_x, top + (params.item_height - size) / 2.0],
                            [size, size],
                            0.0,
                            &white,
                        );
                        match resources.texture(handle) {
                            Some(texture) => {
                                icon.sprite_texture(texture);
                            }
                            None => log::warn!("the icon of '{}' is not committed; it is drawn without one", label.label),
                        }
                        objects.push(register(draw_manager, params, z, icon));
                    }

                    texts.push(text(draw_manager, font, params, z, &format!("{entry_base}-label"), &label.label, label_x, row, &theme.text_color));
                    if let Some(sub_title) = label.sub_title.as_deref() {
                        texts.push(text(draw_manager, font, params, z, &format!("{entry_base}-sub_title"), sub_title, sub_title_x, row, &theme.sub_title_color));
                    }
                    if sub_menu.is_some() {
                        texts.push(text(draw_manager, font, params, z, &format!("{entry_base}-mark"), SUB_MENU_MARK, mark_x, row, &theme.sub_title_color));
                    }

                    entries.push(EntryView {
                        top,
                        height: params.item_height,
                        objects,
                        texts,
                        label: Some(label),
                        sub_menu,
                    });
                }
            }
        }

        self.menus.push(MenuView {
            name: labels.name,
            z,
            width,
            radius,
            shadow,
            background,
            highlight,
            outline,
            entries,
        });

        index
    }

    pub(crate) fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    /// 名前のメニューを、指の位置（ワールド）に開く。
    /// そのメニューが無いか、見える項目が 1 つも無ければ何もせず `false`。
    pub(crate) fn open(&mut self, draw_manager: &mut DrawManager, name: &str, at: [f32; 2], state: ContextMenuState) -> bool {
        self.close(draw_manager);

        let Some(&(_, menu)) = self.roots.iter().find(|(root, _)| root == name) else {
            log::debug!("no context menu named '{name}'");
            return false;
        };

        self.state = state;
        self.push(draw_manager, menu, at)
    }

    /// メニューを 1 枚開いて、いちばん上に重ねる。見える項目が無ければ開かず `false`。
    fn push(&mut self, draw_manager: &mut DrawManager, menu: usize, origin: [f32; 2]) -> bool {
        let (tops, height) = self.arrange(menu);
        if tops.iter().zip(&self.menus[menu].entries).all(|(top, entry)| top.is_none() || entry.label.is_none()) {
            return false;
        }

        self.reshape(draw_manager, menu, height);
        self.place(draw_manager, menu, origin, Some(&tops));
        self.stack.push(Opened { menu, origin, hovered: None, tops, height });
        self.tint(draw_manager, menu);
        self.place_highlight(draw_manager, self.stack.len() - 1);
        true
    }

    /// 開くときの並び。消す項目を抜いて詰め、項目ごとの上端と、メニューの高さを出す。
    fn arrange(&self, menu: usize) -> (Vec<Option<f32>>, f32) {
        let view = &self.menus[menu];
        let [_, padding_y] = self.padding();

        let mut visible: Vec<bool> = view.entries.iter().map(|entry| !self.hidden(entry)).collect();

        // 区切り線は、見える項目のあいだにあるものだけ残す。
        // 先頭（まだ項目が無い）と、続けて 2 本目と、末尾（後ろに項目が無い）は消す。
        let mut seen_label = false;
        let mut waiting_slicer: Option<usize> = None;
        for (index, entry) in view.entries.iter().enumerate() {
            if entry.label.is_some() {
                if visible[index] {
                    seen_label = true;
                    waiting_slicer = None;
                }
            } else if !seen_label || waiting_slicer.is_some() {
                visible[index] = false;
            } else {
                waiting_slicer = Some(index);
            }
        }
        if let Some(index) = waiting_slicer {
            visible[index] = false;
        }

        let mut top = padding_y;
        let tops = view
            .entries
            .iter()
            .zip(visible)
            .map(|(entry, visible)| {
                visible.then(|| {
                    let current = top;
                    top += entry.height;
                    current
                })
            })
            .collect();

        (tops, top + padding_y)
    }

    /// 選べないので消す項目か。
    fn hidden(&self, entry: &EntryView) -> bool {
        let Some(label) = entry.label.as_ref() else {
            return false;
        };
        let default = self.style.as_ref().map_or(ContextMenuDisabled::Dim, |style| style.layout.when_disabled);

        label.when_disabled.unwrap_or(default) == ContextMenuDisabled::Hide && !self.enabled(entry)
    }

    /// 地・縁取り・影を、`height` の高さに組み直す（消した項目のぶん縮める）。
    fn reshape(&self, draw_manager: &mut DrawManager, menu: usize, height: f32) {
        let Some(style) = self.style.as_ref() else {
            return;
        };
        let view = &self.menus[menu];
        let size = [view.width, height];
        let theme = style.theme;
        let stroke = PaintType::Stroke { line_width: style.layout.outline_width, joint_type: JointType::None, strip: false, dash: None };

        let shapes = [
            (Some(&view.background), PaintType::Fill, theme.fill_color),
            (view.outline.as_ref(), stroke, theme.outline_color),
            (view.shadow.as_ref(), PaintType::Fill, theme.shadow_color),
        ];
        for (name, paint_type, color) in shapes {
            if let Some(object) = name.and_then(|name| draw_manager.object_mut(name)) {
                replace_outline(object, &rect("", paint_type, [0.0, 0.0], size, view.radius, &color));
            }
        }
    }

    /// すべて閉じる。
    pub(crate) fn close(&mut self, draw_manager: &mut DrawManager) {
        self.close_above(draw_manager, None);
    }

    /// `level` より深いメニューを閉じる。`None` ならすべて。
    fn close_above(&mut self, draw_manager: &mut DrawManager, level: Option<usize>) {
        let keep = level.map_or(0, |level| level + 1);

        while self.stack.len() > keep {
            let Some(opened) = self.stack.pop() else {
                break;
            };
            self.place(draw_manager, opened.menu, [OFFSCREEN, OFFSCREEN], None);
            self.translate(draw_manager, &self.menus[opened.menu].highlight, [OFFSCREEN, OFFSCREEN], self.menus[opened.menu].z);
        }

        // 閉じきったら、まだ何も開いていない状態に戻す（窓を組んだ直後にも呼ばれる）。
        if self.stack.is_empty() {
            for menu in 0..self.menus.len() {
                self.place(draw_manager, menu, [OFFSCREEN, OFFSCREEN], None);
                self.translate(draw_manager, &self.menus[menu].highlight, [OFFSCREEN, OFFSCREEN], self.menus[menu].z);
            }
        }
    }

    fn translate(&self, draw_manager: &mut DrawManager, name: &str, [x, y]: [f32; 2], z: f32) {
        if let Some(object) = draw_manager.object_mut(name) {
            object.translate(x, y, z);
        }
    }

    /// 項目 `entry` の図形の名前（絵と字の形）。
    fn entry_shapes(&self, draw_manager: &DrawManager, entry: &EntryView) -> Vec<String> {
        let mut names = entry.objects.clone();
        for text in &entry.texts {
            names.extend(draw_manager.group_names(text).map(String::from));
        }
        names
    }

    /// メニューの影・地・縁取り・項目を `at` へ動かす。影はずらして置く。帯は別に動かす。
    ///
    /// `tops` は開いたときの並び（[`ContextMenu::arrange`]）。項目は作ったときの位置から
    /// そこへずらし、`None` の項目（消したもの）と、`tops` が無いとき（閉じる）は外へ逃がす。
    fn place(&self, draw_manager: &mut DrawManager, menu: usize, at: [f32; 2], tops: Option<&[Option<f32>]>) {
        let view = &self.menus[menu];
        let offscreen = [OFFSCREEN, OFFSCREEN];
        let at = if tops.is_some() { at } else { offscreen };

        for name in std::iter::once(&view.background).chain(view.outline.as_ref()) {
            self.translate(draw_manager, name, at, view.z);
        }

        for (index, entry) in view.entries.iter().enumerate() {
            let top = tops.and_then(|tops| tops.get(index).copied().flatten());
            let entry_at = top.map_or(offscreen, |top| [at[0], at[1] + top - entry.top]);

            for name in self.entry_shapes(draw_manager, entry) {
                self.translate(draw_manager, &name, entry_at, view.z);
            }
        }

        if let Some(shadow) = &view.shadow {
            let [x, y] = self.style.as_ref().and_then(|style| style.layout.shadow_offset).unwrap_or_default();
            self.translate(draw_manager, shadow, [at[0] + x, at[1] + y], view.z);
        }
    }

    fn enabled(&self, entry: &EntryView) -> bool {
        entry
            .label
            .as_ref()
            .is_some_and(|label| label.enabled.as_ref().map_or(true, |condition| condition(&self.state)))
    }

    /// 選べない項目の字と絵を薄くする。
    fn tint(&self, draw_manager: &mut DrawManager, menu: usize) {
        let disabled_alpha = self.style.as_ref().map_or(0.35, |style| style.theme.disabled_alpha);

        for entry in self.menus[menu].entries.iter().filter(|entry| entry.label.is_some()) {
            let alpha = if self.enabled(entry) { 1.0 } else { disabled_alpha };
            let tint = Block::Tint { color: [1.0, 1.0, 1.0, alpha] };

            for name in self.entry_shapes(draw_manager, entry) {
                if let Some(object) = draw_manager.object_mut(&name) {
                    object.replace_effect(|effect| matches!(effect, Block::Tint { .. }), tint);
                }
            }
        }
    }

    /// `level` のメニューの帯を、指が上にある項目へ。選べない項目と区切り線なら外へ逃がす。
    fn place_highlight(&self, draw_manager: &mut DrawManager, level: usize) {
        let Some(opened) = self.stack.get(level) else {
            return;
        };
        let view = &self.menus[opened.menu];

        let at = opened
            .hovered
            .filter(|index| view.entries.get(*index).is_some_and(|entry| self.enabled(entry)))
            .and_then(|index| opened.tops.get(index).copied().flatten())
            .map(|top| [opened.origin[0], opened.origin[1] + top]);

        self.translate(draw_manager, &view.highlight, at.unwrap_or([OFFSCREEN, OFFSCREEN]), view.z);
    }

    /// 画素の位置が、どのメニューのどこか。いちばん手前（深い）メニューから見る。
    /// どのメニューの上でもなければ `None`。項目の上でなければ（余白）、項目は `None`。
    fn locate(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<(usize, Option<usize>)> {
        for (level, opened) in self.stack.iter().enumerate().rev() {
            let view = &self.menus[opened.menu];
            let Some(object) = draw_manager.object(&view.background) else {
                continue;
            };
            let Some([world_x, world_y]) = object.view_camera().screen_to_world(x, y) else {
                continue;
            };
            let [local_x, local_y] = [world_x - opened.origin[0], world_y - opened.origin[1]];

            if (0.0..=view.width).contains(&local_x) && (0.0..=opened.height).contains(&local_y) {
                let entry = view.entries.iter().zip(&opened.tops).position(|(entry, top)| {
                    top.is_some_and(|top| (top..top + entry.height).contains(&local_y))
                });
                return Some((level, entry));
            }
        }

        None
    }

    /// 指がメニューの上にあるか。
    pub(crate) fn contains(&self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        self.locate(draw_manager, x, y).is_some()
    }

    /// 指が動いた。帯を動かし、右に開くメニューがある項目なら開く。何か変わったら `true`。
    pub(crate) fn hover(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        // どのメニューの上でもなければ、開いているものはそのまま。
        let Some((level, entry)) = self.locate(draw_manager, x, y) else {
            return false;
        };
        if self.stack[level].hovered == entry {
            return false;
        }

        // 別の項目へ移ったので、そこから右に開いていたメニューは閉じる。
        self.close_above(draw_manager, Some(level));
        self.stack[level].hovered = entry;
        self.place_highlight(draw_manager, level);

        let opened = &self.stack[level];
        let view = &self.menus[opened.menu];
        let origin = opened.origin;
        let sub_menu = entry
            .filter(|index| view.entries.get(*index).is_some_and(|entry| self.enabled(entry)))
            .and_then(|index| Some((view.entries[index].sub_menu?, opened.tops.get(index).copied().flatten()?)));

        if let Some((sub_menu, top)) = sub_menu {
            // 項目の右に、項目の上端と中身の上端がそろうように開く。
            let origin = [origin[0] + view.width, origin[1] + top - self.padding()[1]];
            self.push(draw_manager, sub_menu, origin);
        }

        true
    }

    /// 押した。押したのがメニューの上なら `true`（取った）。
    ///
    /// 選べる項目を押したら、その項目を覚えて（右に開くメニューがあれば開いて）、
    /// すべて閉じる。関数は窓が呼ぶ側の手で呼ぶ。
    pub(crate) fn press(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let Some((level, Some(index))) = self.locate(draw_manager, x, y) else {
            return self.locate(draw_manager, x, y).is_some();
        };

        let menu = self.stack[level].menu;
        let entry = &self.menus[menu].entries[index];
        if !self.enabled(entry) {
            return true;
        }

        if entry.sub_menu.is_some() {
            // 指を乗せたときに開いているはず。押して開くこともできるようにしておく。
            if self.stack.len() == level + 1 {
                self.stack[level].hovered = None;
                self.hover(draw_manager, x, y);
            }
            return true;
        }

        self.pending = Some((menu, index));
        self.close(draw_manager);
        true
    }

    /// 押された項目を受け取る。
    pub(crate) fn take_pending(&mut self) -> Option<(usize, usize)> {
        self.pending.take()
    }

    /// 項目の関数を借り出す。呼び終えたら [`ContextMenu::put_handler`] で戻す。
    pub(crate) fn take_handler(&mut self, menu: usize, entry: usize) -> Option<ContextMenuHandler> {
        self.menus.get_mut(menu)?.entries.get_mut(entry)?.label.as_mut()?.on_event.take()
    }

    pub(crate) fn put_handler(&mut self, menu: usize, entry: usize, handler: ContextMenuHandler) {
        if let Some(label) = self.menus.get_mut(menu).and_then(|view| view.entries.get_mut(entry)).and_then(|entry| entry.label.as_mut()) {
            label.on_event = Some(handler);
        }
    }

    /// 描き先の大きさが変わった。カメラを貼り直して閉じる。
    pub(crate) fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2], scale_mode: ScaleMode) {
        for view in &self.menus {
            let mut names: Vec<String> = [Some(&view.background), Some(&view.highlight), view.shadow.as_ref(), view.outline.as_ref()]
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            for entry in &view.entries {
                names.extend(self.entry_shapes(draw_manager, entry));

                for text in &entry.texts {
                    draw_manager.with_text_renderer(text, |text, _| {
                        text.camera_for(surface_size, design, scale_mode);
                    });
                }
            }

            for name in names {
                if let Some(object) = draw_manager.object_mut(&name) {
                    object.camera_for(surface_size, design, scale_mode);
                }
            }
        }

        self.close(draw_manager);
    }

    /// メニューの名前（右に開くものも含め、作った順）。テストや調べもの用。
    #[allow(dead_code)]
    pub(crate) fn menu_names(&self) -> impl Iterator<Item = &str> {
        self.menus.iter().map(|view| view.name.as_str())
    }
}

fn vertex(x: f32, y: f32, color: &ThemeColor) -> Vertex {
    Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a)
}

/// `at` から `size` の四角。`radius` が正なら角を丸める。
fn rect(name: &str, paint_type: PaintType, at: [f32; 2], size: [f32; 2], radius: f32, color: &ThemeColor) -> Object {
    let (from, to) = (vertex(at[0], at[1], color), vertex(at[0] + size[0], at[1] + size[1], color));

    if radius > 0.0 {
        RectRounded::new(name).paint_type(paint_type).from(CornerType::Circle { radius }, from).to(to).end()
    } else {
        Rect::new(name).paint_type(paint_type).from(from).to(to).end()
    }
}

/// 図形を登録し、閉じた状態（画面の外）に置く。返すのは登録した名前。
fn register(draw_manager: &mut DrawManager, params: &Params, z: f32, mut object: Object) -> String {
    object.camera_for(params.surface_size, params.design, params.scale_mode);
    object.instance(create_instance());
    object.translate(OFFSCREEN, OFFSCREEN, z);
    draw_manager.register(object)
}

/// 項目の字を 1 つ置く。`row` は `[上端, 高さ]`。返すのは [`TextRenderer`] の名前。
#[allow(clippy::too_many_arguments)]
fn text(
    draw_manager: &mut DrawManager,
    font: &Font,
    params: &Params,
    z: f32,
    name: &str,
    content: &str,
    x: f32,
    [top, height]: [f32; 2],
    color: &ThemeColor,
) -> String {
    let mut text = TextRenderer::new(name);
    text.camera_for(params.surface_size, params.design, params.scale_mode);
    text.color(color.r, color.g, color.b, color.a);
    text.z(z);
    text.text_layout_data(TextLayoutData {
        text: content.to_string(),
        style: params.text_style,
        text_location: TextLocation::Area {
            area: TextArea::new(x, top, measure(font, content, &params.text_style).width + params.font_size, height)
                .vertical(TextAlign::Center),
        },
    });

    if let Err(error) = text.register_draw_manager(draw_manager, font) {
        log::warn!("failed to lay out the context menu: {error}");
    }
    if let Some(replaced) = draw_manager.add_text_renderer(text) {
        log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
    }

    name.to_string()
}
