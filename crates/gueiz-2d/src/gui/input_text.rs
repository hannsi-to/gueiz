use std::time::{Duration, Instant};
use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::effect::Block;
use crate::gui::window_frame::{Gap, QuadType, ThemeColor};
use crate::gui::text_edit::TextEdit;
use crate::gui::context_menu::{ContextMenuRequest, ContextMenuState, INPUT_TEXT_CONTEXT_MENU};
use crate::gui::window_item::{replace_outline, Animation, CursorShape, EditKey, KeyModifiers, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::{JointType, PaintType};
use crate::resource::{FontHandle, Resources};
use crate::font::Font;
use crate::text::{layout, measure, TextAlign, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

/// 縁取りの太さ。
const OUTLINE_WIDTH: f32 = 1.0;
/// 字を打つ位置に出す縦棒の太さ。
const CARET_WIDTH: f32 = 1.5;
/// この時間のうちに同じ場所をもう一度押したら、続けて押した（ダブルクリック）とみなす。
const MULTI_CLICK_TIME: Duration = Duration::from_millis(500);
/// 続けて押したとみなす、押した場所のずれ（画素）。
const MULTI_CLICK_DISTANCE: f32 = 4.0;
/// キャレットを点けている・消している長さ（OS の既定に近い値）。
const BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// 引いたまま欄の外にいるあいだ、横へ送る間隔。
const AUTO_SCROLL_INTERVAL: Duration = Duration::from_millis(16);
/// 引いたまま欄の外にいるときの送る速さ（1 秒あたり）。外へ離れるほど速くする。
const AUTO_SCROLL_SPEED: f32 = 80.0;
const AUTO_SCROLL_SPEED_PER_DISTANCE: f32 = 8.0;
/// パスワード欄で、字の代わりに出す字。
const MASK: char = '\u{2022}';

/// 1 行の文字を打ち込む欄。
///
/// 押すとフォーカスを取り、キーと文字を受け取ります。よそを押すと手放します。
/// 文字は窓の [`WindowFrame::text_input`](crate::gui::window_frame::WindowFrame::text_input) などから届きます。
///
/// 書き換えの中身は [`TextEdit`] が持ちます。Web の `<input type="text">` と同じように、
///
/// - Shift+←→・Home・End で範囲を選び、Ctrl+←→ で単語ごとに動きます
///   （macOS は Option）。↑↓ は先頭・末尾へ。
/// - Backspace・Delete は選んだ範囲を消し、Ctrl を押しながらなら単語ごと消します。
/// - 選んでいるときに打つと置き換えます。Ctrl+A ですべて選びます。
/// - Ctrl+Z で元に戻し、Ctrl+Y・Ctrl+Shift+Z でやり直します。続けて打った字はまとめて戻ります。
/// - 押して引くと範囲を選び、Shift を押しながら押すと広げます。
///   2 回続けて押すと単語を、3 回続けて押すとすべてを選びます。
/// - コピー・切り取り・貼り付けは、窓の
///   [`selected_text`](crate::gui::window_frame::WindowFrame::selected_text)・
///   [`cut`](crate::gui::window_frame::WindowFrame::cut)・
///   [`paste`](crate::gui::window_frame::WindowFrame::paste) から（クリップボードは呼ぶ側が持つ）。
///
/// ほかに、
///
/// - 文字が欄より長くなったら、字を打つ位置が見えるように横へずらします。
///   はみ出したぶんは欄の内側で削ります。
/// - IME で変換中の文字も、確定前から欄の中に出します。
/// - 空のあいだは `placeholder` を薄い色で出します。
/// - フォーカスがあるあいだ、キャレットを点滅させます（打ったり動かしたりすると点けた状態からやり直す）。
/// - 押して引いたまま欄の左右の外へ出ると、外へ離れるほど速く横へ送りながら選びます。
/// - 右クリックで、切り取り・コピー・貼り付け・すべて選択のメニューを出します。
/// - [`InputText::password`] で、字を伏せたパスワード欄にできます。
///   コピー・切り取りはできず、IME も使いません。
///
/// 点滅と自動スクロールは時間で動くので、呼ぶ側は窓の
/// [`next_update`](crate::gui::window_frame::WindowFrame::next_update) の時刻に描き直しを頼んでください。
pub struct InputText {
    edit: TextEdit,
    placeholder: String,
    width: f32,
    height: f32,
    /// 左右の内側の余白。
    padding: f32,
    quad_type: QuadType,
    gap: Gap,
    input_text_font: InputTextFont,
    input_text_theme: InputTextTheme,
    on_change: Option<Box<dyn FnMut(&str)>>,
    on_submit: Option<Box<dyn FnMut(&str)>>,
    pub(crate) input_text_register_name: InputTextRegisterName,
    scale_mode: ScaleMode,
    /// 組んだときの左上（余白を足す前）。
    built_at: [f32; 2],
    /// 窓を組んだ場所からのずれ。自前の覆いをワールドに置くのに使う。
    offset: [f32; 2],
    focused: bool,
    /// いま押している Shift や Ctrl。
    modifiers: KeyModifiers,
    /// 押して引いて、範囲を選んでいるあいだ。
    dragging: bool,
    /// 最後に押したとき。続けて押した回数を数える。
    last_click: Option<Click>,
    /// 字を伏せるか。
    password: bool,
    /// 右クリックで開くメニューの名前。
    context_menu: String,
    /// 点滅を数え始めたとき。打ったり動かしたりするたびに今にする。
    blink_start: Instant,
    /// いまキャレットを点けているか。
    caret_shown: bool,
    /// 引いているあいだの指の位置（組んだときの座標の x）。欄の外にいれば横へ送る。
    drag_pointer: Option<f32>,
    /// 最後に横へ送ったとき。送る量を経った時間から出す。
    last_auto_scroll: Option<Instant>,
    /// IME で変換中の文字。確定するまで `text` には入れない。
    preedit: String,
    /// 変換中の文字の中の、字を打つ位置（バイト）。
    preedit_cursor: Option<usize>,
    /// 出している文字の、字の境目ごとの `(バイト位置, 先頭からの幅)`。押した場所から位置を出すのに使う。
    stops: Vec<(usize, f32)>,
    /// `stops` を測った文字。同じなら測り直さない。
    stops_text: String,
    /// 字を最後に並べたときの、横のずらしと、案内の字を出していたか。
    /// 文字も含めて同じなら、字の形は登録し直さない（キャレットと選択の帯だけ動かす）。
    rendered: Option<(f32, bool)>,
    /// 横へずらしている量。
    scroll: f32,
    /// 字の形に最後に積んだ、欄の外を削る山。
    applied_clip: Option<Block>,
}

/// 押したときの覚え。
#[derive(Debug, Copy, Clone)]
struct Click {
    at: Instant,
    position: [f32; 2],
    /// 続けて何回押したか。
    count: u32,
}

pub struct InputTextRegisterName {
    background: Option<String>,
    selection: Option<String>,
    outline: Option<String>,
    caret: Option<String>,
    text: Option<String>,
}

pub struct InputTextFont {
    pub base_font: FontHandle,
    pub base_font_size: f32,
}

pub struct InputTextTheme {
    pub fill_color: ThemeColor,
    pub outline_color: ThemeColor,
    /// フォーカスを持っているときの縁取り。
    pub focus_outline_color: ThemeColor,
    pub text_color: ThemeColor,
    pub placeholder_color: ThemeColor,
    pub caret_color: ThemeColor,
    /// 選んでいる範囲の地の色。
    pub selection_color: ThemeColor,
}

impl InputText {
    pub fn new(register_name_string: String, placeholder: String, width: f32, height: f32, padding: f32, quad_type: QuadType, gap: Gap, input_text_font: InputTextFont, input_text_theme: InputTextTheme) -> Self {
        Self {
            edit: TextEdit::new(),
            placeholder,
            width,
            height,
            padding,
            quad_type,
            gap,
            input_text_font,
            input_text_theme,
            on_change: None,
            on_submit: None,
            input_text_register_name: InputTextRegisterName {
                background: Some(register_name_string),
                selection: None,
                outline: None,
                caret: None,
                text: None,
            },
            scale_mode: ScaleMode::default(),
            built_at: [0.0; 2],
            offset: [0.0; 2],
            focused: false,
            modifiers: KeyModifiers::default(),
            dragging: false,
            last_click: None,
            password: false,
            context_menu: INPUT_TEXT_CONTEXT_MENU.to_string(),
            blink_start: Instant::now(),
            caret_shown: false,
            drag_pointer: None,
            last_auto_scroll: None,
            preedit: String::new(),
            preedit_cursor: None,
            stops: vec![(0, 0.0)],
            stops_text: String::new(),
            rendered: None,
            scroll: 0.0,
            applied_clip: None,
        }
    }

    /// 文字が変わるたびに呼ぶ関数。
    pub fn on_change(mut self, on_change: impl FnMut(&str) + 'static) -> Self {
        self.on_change = Some(Box::new(on_change));
        self
    }

    /// Enter を押したときに呼ぶ関数。
    pub fn on_submit(mut self, on_submit: impl FnMut(&str) + 'static) -> Self {
        self.on_submit = Some(Box::new(on_submit));
        self
    }

    /// 字を伏せたパスワード欄にする。コピー・切り取りはできず、IME も使わない。
    pub fn password(mut self) -> Self {
        self.password = true;
        self
    }

    /// 右クリックで開くメニューの名前。既定は [`INPUT_TEXT_CONTEXT_MENU`]。
    /// メニューは窓の [`add_context_menu_labels`](crate::gui::window_frame::WindowFrame::add_context_menu_labels) で足す。
    pub fn context_menu(mut self, name: impl Into<String>) -> Self {
        self.context_menu = name.into();
        self
    }

    /// 入れられる字の数の上限。
    pub fn max_length(mut self, max_length: usize) -> Self {
        self.edit.set_max_length(Some(max_length));
        self
    }

    /// いまの文字。変換中の文字は入りません。
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    // --- 形 ---

    /// 欄の左上と右下（組んだときの座標）。
    fn bounds(&self) -> [f32; 4] {
        let [x, y] = self.built_at;
        let (left, top) = (x + self.gap.x, y + self.gap.y);

        [left, top, left + self.width, top + self.height]
    }

    fn vertex(x: f32, y: f32, color: &ThemeColor) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a)
    }

    /// 地か縁取りの四角を組む。組み直しで頂点だけが欲しいときは、名前に `""` を渡します。
    fn build_box(&self, name: &str, paint_type: PaintType, color: &ThemeColor) -> Object {
        let [left, top, right, bottom] = self.bounds();

        match self.quad_type {
            QuadType::Normal => {
                Rect::new(name)
                    .paint_type(paint_type)
                    .from(Self::vertex(left, top, color))
                    .to(Self::vertex(right, bottom, color))
                    .end()
            }
            QuadType::Rounded { radius } => {
                RectRounded::new(name)
                    .paint_type(paint_type)
                    .from(CornerType::Circle { radius }, Self::vertex(left, top, color))
                    .to(Self::vertex(right, bottom, color))
                    .end()
            }
        }
    }

    fn build_background(&self, name: &str) -> Object {
        self.build_box(name, PaintType::Fill, &self.input_text_theme.fill_color)
    }

    fn build_outline(&self, name: &str) -> Object {
        let theme = &self.input_text_theme;
        let color = if self.focused { &theme.focus_outline_color } else { &theme.outline_color };
        let stroke = PaintType::Stroke { line_width: OUTLINE_WIDTH, joint_type: JointType::None, strip: false, dash: None };

        self.build_box(name, stroke, color)
    }

    /// 字を打つ位置の縦棒。フォーカスが無ければ透明にする（当たり判定は残るが、欄の中なので困らない）。
    ///
    /// 欄の内側（字を削る範囲）から外れているときも描かない。横の送りが追いつく前の
    /// 一瞬に、欄の外へキャレットが出ないように。
    fn build_caret(&self, name: &str) -> Object {
        let [x, y, width, height] = self.caret_area_local();
        let [left, _, right, _] = self.bounds();
        let inset = self.padding * 0.5;
        let inside = x >= left + inset && x + width <= right - inset;

        let caret_color = &self.input_text_theme.caret_color;
        let color = ThemeColor {
            r: caret_color.r,
            g: caret_color.g,
            b: caret_color.b,
            a: if self.caret_shown && inside { caret_color.a } else { 0.0 },
        };

        Rect::new(name)
            .paint_type(PaintType::Fill)
            .from(Self::vertex(x, y, &color))
            .to(Self::vertex(x + width, y + height, &color))
            .end()
    }

    /// 選んでいる範囲の地。フォーカスが無いか、何も選んでいなければ透明にする。
    ///
    /// 欄の内側からはみ出さないよう、端で切ります（字と違って覆いを掛けないので）。
    fn build_selection(&self, name: &str) -> Object {
        let [left, _, right, _] = self.bounds();
        let inset = self.padding * 0.5;
        let [_, top, _, height] = self.caret_area_local();

        let range = self.edit.selection().filter(|_| self.focused && self.preedit.is_empty());
        let (start, end) = match range {
            Some(range) => {
                let x = |index: usize| self.text_left() + self.stop_x(self.display_index(index));
                (x(range.start).max(left + inset), x(range.end).min(right - inset))
            }
            None => (left, left),
        };

        let selection_color = &self.input_text_theme.selection_color;
        let color = ThemeColor {
            r: selection_color.r,
            g: selection_color.g,
            b: selection_color.b,
            a: if end > start { selection_color.a } else { 0.0 },
        };

        Rect::new(name)
            .paint_type(PaintType::Fill)
            .from(Self::vertex(start, top, &color))
            .to(Self::vertex(end.max(start), top + height, &color))
            .end()
    }

    /// 縁取り・選んでいる範囲・縦棒を、いまの状態で組み直す。字は触らないので、書体は要らない。
    fn refresh_shapes(&self, draw_manager: &mut DrawManager) {
        let names = &self.input_text_register_name;

        if let Some(object) = names.selection.as_deref().and_then(|name| draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_selection(""));
        }

        if let Some(object) = names.outline.as_deref().and_then(|name| draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_outline(""));
        }
        if let Some(object) = names.caret.as_deref().and_then(|name| draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_caret(""));
        }
    }

    // --- 字 ---

    /// 字を置き始める左端（組んだときの座標）。横へずらしたぶんも入る。
    fn text_left(&self) -> f32 {
        self.bounds()[0] + self.padding - self.scroll
    }

    /// 空で変換中の文字も無ければ、薄い字の案内を出す。
    fn showing_placeholder(&self) -> bool {
        self.edit.text().is_empty() && self.preedit.is_empty()
    }

    /// 出す文字。変換中の文字は、字を打つ位置に差し込む。パスワード欄なら伏せる。
    fn display_text(&self) -> String {
        if self.showing_placeholder() {
            return self.placeholder.clone();
        }
        if self.password {
            return self.edit.text().chars().map(|_| MASK).collect();
        }

        let (text, cursor) = (self.edit.text(), self.edit.cursor());
        let mut display = String::with_capacity(text.len() + self.preedit.len());
        display.push_str(&text[..cursor]);
        display.push_str(&self.preedit);
        display.push_str(&text[cursor..]);
        display
    }

    /// 出している文字の中で、字を打つ位置（バイト）。
    fn display_cursor(&self) -> usize {
        if self.showing_placeholder() {
            return 0;
        }

        if self.password {
            return self.display_index(self.edit.cursor());
        }

        self.edit.cursor() + self.preedit_cursor.unwrap_or(self.preedit.len()).min(self.preedit.len())
    }

    /// 文字の中のバイト位置を、出している文字の中の位置へ（変換中の文字の手前まで）。
    /// パスワード欄では 1 字が伏せ字 1 字になるので、字の数から出す。
    fn display_index(&self, index: usize) -> usize {
        if self.password {
            self.edit.text()[..index].chars().count() * MASK.len_utf8()
        } else {
            index
        }
    }

    /// 出している文字の中の位置を、文字の中のバイト位置へ。[`InputText::display_index`] の逆。
    fn text_index(&self, display_index: usize) -> usize {
        if !self.password {
            return display_index;
        }

        let text = self.edit.text();
        text.char_indices()
            .nth(display_index / MASK.len_utf8())
            .map_or(text.len(), |(index, _)| index)
    }

    /// キャレットを点けて、点滅を今から数え直す。打ったり動かしたりしたときに呼ぶ。
    fn reset_blink(&mut self) {
        self.blink_start = Instant::now();
        self.caret_shown = self.focused;
    }

    /// 出している文字の `index` バイト目の、字の先頭からの幅。
    fn stop_x(&self, index: usize) -> f32 {
        // バイト位置の順に並んでいるので、二分探索で引ける。
        self.stops
            .binary_search_by_key(&index, |(position, _)| *position)
            .map_or(0.0, |found| self.stops[found].1)
    }

    /// 字を打つ位置の、字の先頭からの幅。
    fn caret_x(&self) -> f32 {
        self.stop_x(self.display_cursor())
    }

    /// 縦棒の位置と大きさ（組んだときの座標）。
    fn caret_area_local(&self) -> [f32; 4] {
        let [_, top, _, bottom] = self.bounds();
        let height = (self.input_text_font.base_font_size * 1.2).min(bottom - top);
        let y = top + (bottom - top - height) / 2.0;

        [self.text_left() + self.caret_x(), y, CARET_WIDTH, height]
    }

    /// 字を並べ直し、縦棒が見えるように横のずらしを決め、縁取りと縦棒も組み直す。
    ///
    /// 書体が無ければ何もせず `false`。
    fn relayout(&mut self, draw_manager: &mut DrawManager, resources: &Resources) -> bool {
        let Some(font) = resources.font(self.input_text_font.base_font) else {
            log::warn!("the input text font is not in the resources; the text is not drawn");
            return false;
        };

        let style = TextStyle::new(self.input_text_font.base_font_size);
        let display = self.display_text();

        // 字の境目ごとに、先頭からの幅を測っておく。押した場所から位置を出すのにも使う。
        // 文字が変わったときだけ。キャレットを動かすだけなら測り直さない。
        if display != self.stops_text {
            self.stops = update_stops(&self.stops_text, &self.stops, &display, &font, &style);
            self.stops_text = display;
            self.rendered = None;
        }

        // 縦棒が欄の内側に来るように、横のずらしを決める。
        let inner_width = (self.width - self.padding * 2.0).max(0.0);
        let caret_x = self.caret_x();
        let total = self.stops.last().map_or(0.0, |(_, x)| *x);

        if caret_x - self.scroll > inner_width {
            self.scroll = caret_x - inner_width;
        }
        if caret_x < self.scroll {
            self.scroll = caret_x;
        }
        // 消して短くなったら、右に空きを作らないよう戻す。
        self.scroll = self.scroll.min((total - inner_width).max(0.0)).max(0.0);

        // 文字・横のずらし・色が前と同じなら、字はそのままでよい。
        let rendered = (self.scroll, self.showing_placeholder());
        if self.rendered != Some(rendered) {
            self.render_text(draw_manager, &font, style);
            self.rendered = Some(rendered);
            self.apply_clip(draw_manager);
        }

        self.refresh_shapes(draw_manager);

        true
    }

    /// 字を、いまの横のずらしで並べ直す（字の形を登録し直す）。
    ///
    /// 置くのは欄に見えている字（と前後に少し）だけ。長い文字でも、横へ送るたびに
    /// 全部を置き直さずに済む。はみ出したぶんは覆いで削る。
    fn render_text(&self, draw_manager: &mut DrawManager, font: &Font, style: TextStyle) {
        let [_, top, _, bottom] = self.bounds();
        let theme = &self.input_text_theme;
        let color = if self.showing_placeholder() { &theme.placeholder_color } else { &theme.text_color };
        let color = [color.r, color.g, color.b, color.a];

        // 見えている範囲。端で字が途切れないよう、字の大きさ 2 つぶん広く取る。
        let margin = self.input_text_font.base_font_size * 2.0;
        let inner_width = (self.width - self.padding * 2.0).max(0.0);
        let (left, right) = (self.scroll - margin, self.scroll + inner_width + margin);

        // 境目は左から順に並んでいるので、二分探索で範囲を出す。
        let first = self.stops.partition_point(|(_, x)| *x <= left).saturating_sub(1);
        let last = self.stops.partition_point(|(_, x)| *x < right).min(self.stops.len() - 1);
        let (start, start_x) = self.stops[first];
        let (end, end_x) = self.stops[last.max(first)];

        let layout_data = TextLayoutData {
            text: self.stops_text[start..end].to_string(),
            style,
            text_location: TextLocation::Area {
                // 幅は文字ぶん取って、寄せも折り返しもさせない。
                area: TextArea::new(self.text_left() + start_x, top, end_x - start_x + self.input_text_font.base_font_size, bottom - top)
                    .vertical(TextAlign::Center),
            },
        };

        if let Some(name) = self.input_text_register_name.text.as_deref() {
            draw_manager.with_text_renderer(name, |text_renderer, draw_manager| {
                text_renderer.clear(draw_manager);
                text_renderer.color(color[0], color[1], color[2], color[3]);
                text_renderer.text_layout_data(layout_data);

                if let Err(error) = text_renderer.register_draw_manager(draw_manager, font) {
                    log::warn!("failed to lay out the input text: {error}");
                }
            });
        }
    }

    /// 欄の外にはみ出した字を削る山を、字の形すべてに積む。
    ///
    /// 削る範囲はワールドなので、窓のずれを足して置きます。
    fn apply_clip(&mut self, draw_manager: &mut DrawManager) {
        let Some(text) = self.input_text_register_name.text.as_deref() else {
            return;
        };

        let [left, top, right, bottom] = self.bounds();
        let inset = self.padding * 0.5;
        let [x, y] = self.offset;
        let block = Block::ClipRect {
            min: [left + inset + x, top + y],
            max: [right - inset + x, bottom + y],
            radius: 0.0,
            softness: 0.0,
            invert: false,
        };
        let old = self.applied_clip;

        let names: Vec<String> = draw_manager.group_names(text).map(String::from).collect();
        for name in names {
            if let Some(object) = draw_manager.object_mut(&name) {
                object.replace_effect(|effect| Some(*effect) == old, block);
            }
        }

        self.applied_clip = Some(block);
    }

    // --- 書き換え ---

    /// 文字が変わったことを知らせる。
    fn changed(&mut self) {
        if let Some(on_change) = self.on_change.as_mut() {
            on_change(self.edit.text());
        }
    }

    /// 続けて何回押したか。前に押したときから時間が経つか、場所がずれたら 1 に戻る。
    fn count_click(&mut self, x: f32, y: f32) -> u32 {
        let now = Instant::now();
        let count = match self.last_click {
            Some(click)
                if now.duration_since(click.at) <= MULTI_CLICK_TIME
                    && (click.position[0] - x).abs() <= MULTI_CLICK_DISTANCE
                    && (click.position[1] - y).abs() <= MULTI_CLICK_DISTANCE =>
            {
                click.count + 1
            }
            _ => 1,
        };

        self.last_click = Some(Click { at: now, position: [x, y], count });
        count
    }

    /// 指の下で一番手前にあるのが、この欄の図形か。
    fn hit(&self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        let Some(pick) = draw_manager.pick(x, y) else {
            return false;
        };

        let names = &self.input_text_register_name;
        let own = [names.background.as_deref(), names.selection.as_deref(), names.outline.as_deref(), names.caret.as_deref()];
        if own.into_iter().flatten().any(|name| name == pick.name) {
            return true;
        }

        names
            .text
            .as_deref()
            .is_some_and(|text| draw_manager.group_names(text).any(|name| name == pick.name))
    }

    /// 画素の位置を、組んだときの座標の x へ。窓のずれもここで外れる。
    fn local_x(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<f32> {
        let object = draw_manager.object(self.input_text_register_name.background.as_deref()?)?;
        let [world_x, world_y] = object.view_camera().screen_to_world(x, y)?;
        let [local_x, _] = object.to_local(0, world_x, world_y)?;

        Some(local_x)
    }

    /// 組んだときの座標の x に一番近い、字の境目（文字の中のバイト位置）。
    fn index_at_local(&self, local_x: f32) -> usize {
        let target = local_x - self.text_left();

        // 境目は左から順に並んでいるので、指の右にある最初の境目と、その 1 つ手前を比べる。
        let right = self.stops.partition_point(|(_, x)| *x < target).min(self.stops.len() - 1);
        let left = right.saturating_sub(1);
        let nearest = if (self.stops[left].1 - target).abs() <= (self.stops[right].1 - target).abs() { left } else { right };
        let display_index = self.stops[nearest].0;

        self.text_index(display_index)
    }

    /// 押した画素の位置に一番近い、字の境目。
    fn cursor_at(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<usize> {
        Some(self.index_at_local(self.local_x(draw_manager, x, y)?))
    }

    /// 引いたまま欄の外にいれば、横へ送りながら指の側の端まで選ぶ。送ったら `true`。
    fn auto_scroll(&mut self, draw_manager: &mut DrawManager, resources: &Resources, now: Instant) -> bool {
        let Some(pointer) = self.drag_pointer.filter(|_| self.dragging) else {
            self.last_auto_scroll = None;
            return false;
        };

        let [left, _, right, _] = self.bounds();
        let (inner_left, inner_right) = (left + self.padding, right - self.padding);
        let over = if pointer < inner_left {
            pointer - inner_left
        } else if pointer > inner_right {
            pointer - inner_right
        } else {
            self.last_auto_scroll = None;
            return false;
        };

        // 初めの 1 回は時間が測れないので、間隔ぶん経ったとみなす。
        let elapsed = self
            .last_auto_scroll
            .map_or(AUTO_SCROLL_INTERVAL, |last| now.duration_since(last))
            .min(Duration::from_millis(100))
            .as_secs_f32();
        self.last_auto_scroll = Some(now);

        let total = self.stops.last().map_or(0.0, |(_, x)| *x);
        let inner_width = (self.width - self.padding * 2.0).max(0.0);
        let max_scroll = (total - inner_width).max(0.0);
        let speed = AUTO_SCROLL_SPEED + over.abs() * AUTO_SCROLL_SPEED_PER_DISTANCE;
        let scroll = (self.scroll + over.signum() * speed * elapsed).clamp(0.0, max_scroll);

        // 端まで送りきっていても、指の側の端まで選ぶのは続ける。
        self.scroll = scroll;
        let index = self.index_at_local(pointer.clamp(inner_left, inner_right));
        self.edit.move_to(index, true);

        self.relayout(draw_manager, resources)
    }
}

impl WindowItem for InputText {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let counter = window_item_arguments.counter;
        let Some(local_name) = self.input_text_register_name.background.as_deref() else {
            return (0.0, 0.0);
        };
        let name = format!("{register_name}-{local_name}-{counter}");

        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];
        self.scale_mode = window_item_arguments.scale_mode;

        let surface_size = window_item_arguments.surface_size;
        let design = window_item_arguments.design;
        let scale_mode = window_item_arguments.scale_mode;
        let draw_manager = window_item_arguments.draw_manager;
        let register = |mut object: Object, draw_manager: &mut DrawManager| {
            object.camera_for(surface_size, design, scale_mode);
            object.instance(create_instance());
            // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
            draw_manager.register(object)
        };

        // 地、選んだ範囲、縁取り、字、縦棒の順に重ねる。
        let background = register(self.build_background(&name), draw_manager);
        let selection = register(self.build_selection(&format!("{background}-selection")), draw_manager);
        let outline = register(self.build_outline(&format!("{background}-outline")), draw_manager);

        let text_name = format!("{background}-text");
        let mut text_renderer = TextRenderer::new(&text_name);
        text_renderer.camera_for(surface_size, design, scale_mode);
        if let Some(replaced) = draw_manager.add_text_renderer(text_renderer) {
            log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
        }

        let caret = register(self.build_caret(&format!("{background}-caret")), draw_manager);

        self.input_text_register_name = InputTextRegisterName {
            background: Some(background),
            selection: Some(selection),
            outline: Some(outline),
            caret: Some(caret),
            text: Some(text_name),
        };

        self.relayout(draw_manager, window_item_arguments.resources);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.rendered = None;
        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];

        if let Some(object) = self
            .input_text_register_name
            .background
            .as_deref()
            .and_then(|name| window_item_arguments.draw_manager.object_mut(name))
        {
            replace_outline(object, &self.build_background(""));
        }

        self.relayout(window_item_arguments.draw_manager, window_item_arguments.resources);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let names = &self.input_text_register_name;
        let mut owned: Vec<String> = [&names.background, &names.selection, &names.outline, &names.caret]
            .into_iter()
            .flatten()
            .cloned()
            .collect();

        if let Some(text) = &names.text {
            owned.extend(draw_manager.group_names(text).map(String::from));
        }

        owned
    }

    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        // 打つたびに増える字の形は、TextRenderer のカメラで登録される。窓は形のカメラしか
        // 貼り直さないので、これから増える形のためにここで貼り直す。
        let scale_mode = self.scale_mode;
        if let Some(text) = self.input_text_register_name.text.as_deref() {
            draw_manager.with_text_renderer(text, |text_renderer, _| {
                text_renderer.camera_for(surface_size, design, scale_mode);
            });
        }
    }

    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        if !self.hit(draw_manager, x, y) {
            return false;
        }

        self.focused = true;
        let count = self.count_click(x, y);

        // 変換中は、確定するまで位置を動かさない。
        if self.preedit.is_empty() && !self.showing_placeholder() {
            if let Some(index) = self.cursor_at(draw_manager, x, y) {
                match count {
                    1 => {
                        self.edit.move_to(index, self.modifiers.shift);
                        self.dragging = true;
                        self.drag_pointer = self.local_x(draw_manager, x, y);
                    }
                    // パスワード欄は単語の区切りを見せないよう、すべて選ぶ（ブラウザと同じ）。
                    2 if !self.password => self.edit.select_word_at(index),
                    _ => self.edit.select_all(),
                }
            }
        }

        self.reset_blink();
        self.refresh_shapes(draw_manager);
        true
    }

    fn mouse_left_released(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        self.drag_pointer = None;
        self.last_auto_scroll = None;
        std::mem::take(&mut self.dragging)
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        if !self.dragging {
            return false;
        }

        // 選ぶのは欄の内側の端まで。その先は animate が横へ送りながら広げる（ブラウザと同じ）。
        // 指の下の字まで一気に選ぶと、横の送りが追いつかず、キャレットが欄の外に出てしまう。
        self.drag_pointer = self.local_x(draw_manager, x, y);
        let [left, _, right, _] = self.bounds();
        let (inner_left, inner_right) = (left + self.padding, right - self.padding);
        let Some(index) = self.drag_pointer.map(|pointer| self.index_at_local(pointer.clamp(inner_left, inner_right))) else {
            return false;
        };
        if index == self.edit.cursor() {
            return false;
        }

        self.edit.move_to(index, true);
        self.reset_blink();
        self.refresh_shapes(draw_manager);
        true
    }

    fn has_focus(&self) -> bool {
        self.focused
    }

    fn unfocus(&mut self, draw_manager: &mut DrawManager) -> bool {
        if !self.focused {
            return false;
        }

        self.focused = false;
        self.dragging = false;
        self.caret_shown = false;
        self.refresh_shapes(draw_manager);
        true
    }

    fn modifiers_changed(&mut self, modifiers: KeyModifiers) {
        self.modifiers = modifiers;
    }

    fn selected_text(&self) -> Option<String> {
        // パスワードは外へ出さない。
        if self.password {
            return None;
        }

        self.edit.selected_text().map(String::from)
    }

    fn wants_ime(&self) -> bool {
        self.focused && !self.password
    }

    fn context_menu(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> Option<ContextMenuRequest> {
        if !self.hit(draw_manager, x, y) {
            return None;
        }

        self.focused = true;

        // 選んだ範囲の外を押したら、そこへキャレットを動かす（中ならそのまま。ブラウザと同じ）。
        if self.preedit.is_empty() && !self.showing_placeholder() {
            if let Some(index) = self.cursor_at(draw_manager, x, y) {
                let inside = self.edit.selection().is_some_and(|range| range.contains(&index) || range.end == index);
                if !inside {
                    self.edit.move_to(index, false);
                }
            }
        }

        self.reset_blink();
        self.refresh_shapes(draw_manager);

        Some(ContextMenuRequest {
            menu: self.context_menu.clone(),
            state: ContextMenuState {
                has_selection: self.edit.selection().is_some(),
                can_copy: !self.password,
                editable: true,
                has_text: !self.edit.text().is_empty(),
            },
        })
    }

    fn animate(&mut self, draw_manager: &mut DrawManager, resources: &Resources, now: Instant) -> Animation {
        if !self.focused {
            return Animation::default();
        }

        let mut animation = Animation::default();

        if self.auto_scroll(draw_manager, resources, now) {
            animation.relaid_out = true;
        }
        if self.last_auto_scroll.is_some() {
            animation.next = Some(now + AUTO_SCROLL_INTERVAL);
        }

        // 引いているあいだは点けたまま。
        let elapsed = now.saturating_duration_since(self.blink_start);
        let phase = (elapsed.as_millis() / BLINK_INTERVAL.as_millis()) as u32;
        let shown = self.dragging || phase % 2 == 0;

        if shown != self.caret_shown {
            self.caret_shown = shown;
            self.refresh_shapes(draw_manager);
        }

        let next_blink = self.blink_start + BLINK_INTERVAL * (phase + 1);
        animation.next = Some(animation.next.map_or(next_blink, |next| next.min(next_blink)));

        animation
    }

    fn text_input(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        self.preedit.clear();
        self.preedit_cursor = None;

        if !self.edit.insert(text) {
            return false;
        }

        self.changed();
        self.reset_blink();
        self.relayout(draw_manager, resources)
    }

    fn paste(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        if !self.preedit.is_empty() || !self.edit.paste(text) {
            return false;
        }

        self.changed();
        self.reset_blink();
        self.relayout(draw_manager, resources)
    }

    fn ime_preedit(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str, cursor: Option<usize>) -> bool {
        // パスワード欄では変換しない（IME も点けない）。
        if self.password || (self.preedit == text && self.preedit_cursor == cursor) {
            return false;
        }

        // 選んでいるところで変換を始めたら、選んだ範囲は変換する字に置き換わる。
        if !text.is_empty() && self.edit.delete_selection() {
            self.changed();
        }

        self.preedit = text.to_string();
        self.preedit_cursor = cursor;

        self.reset_blink();
        self.relayout(draw_manager, resources)
    }

    fn key_pressed(&mut self, draw_manager: &mut DrawManager, resources: &Resources, key: EditKey) -> bool {
        // 変換中のキーは IME のもの。
        if !self.preedit.is_empty() {
            return false;
        }

        let KeyModifiers { shift, word } = self.modifiers;
        let before = (self.edit.cursor(), self.edit.selection());

        let edited = match key {
            EditKey::Backspace => self.edit.delete_backward(word),
            EditKey::Delete => self.edit.delete_forward(word),
            EditKey::Undo => self.edit.undo(),
            EditKey::Redo => self.edit.redo(),
            EditKey::Left => {
                self.edit.move_left(shift, word);
                false
            }
            EditKey::Right => {
                self.edit.move_right(shift, word);
                false
            }
            EditKey::Home | EditKey::Up | EditKey::PageUp => {
                self.edit.move_home(shift);
                false
            }
            EditKey::End | EditKey::Down | EditKey::PageDown => {
                self.edit.move_end(shift);
                false
            }
            EditKey::SelectAll => {
                self.edit.select_all();
                false
            }
            EditKey::Enter => {
                if let Some(on_submit) = self.on_submit.as_mut() {
                    on_submit(self.edit.text());
                }
                return false;
            }
            EditKey::Escape => return self.unfocus(draw_manager),
            // 1 行の欄では字下げしない（Web では次の欄へ移る）。
            EditKey::Tab => return false,
        };

        if edited {
            self.changed();
        } else if before == (self.edit.cursor(), self.edit.selection()) {
            return false;
        }

        self.reset_blink();
        self.relayout(draw_manager, resources)
    }

    fn offset_changed(&mut self, draw_manager: &mut DrawManager, offset: [f32; 2]) {
        if self.offset == offset && self.applied_clip.is_some() {
            return;
        }

        self.offset = offset;
        self.apply_clip(draw_manager);
    }

    fn caret_area(&self) -> Option<[f32; 4]> {
        self.focused.then(|| self.caret_area_local())
    }

    fn cursor_shape(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<CursorShape> {
        self.hit(draw_manager, x, y).then_some(CursorShape::Text)
    }
}

/// 字の境目ごとの `(バイト位置, 先頭からの幅)` を、1 回並べて出す。
///
/// 並べると 1 字が 1 つのグリフになるので、グリフの右端がその字の後ろの境目です。
/// 書体に無い字は飛ばされて数が合わなくなるので、そのときだけ境目ごとに測ります（遅い）。
pub(crate) fn measure_stops(font: &Font, text: &str, style: &TextStyle) -> Vec<(usize, f32)> {
    let placed = layout(font, text, style);

    if placed.glyphs.len() == text.chars().count() {
        std::iter::once((0, 0.0))
            .chain(
                text.char_indices()
                    .zip(&placed.glyphs)
                    .map(|((index, character), glyph)| (index + character.len_utf8(), glyph.x + glyph.advance)),
            )
            .collect()
    } else {
        text.char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .map(|index| (index, if index == 0 { 0.0 } else { measure(font, &text[..index], style).width }))
            .collect()
    }
}

/// 文字が `old_text` から `new_text` に変わった。境目の幅を、変わったところだけ測り直して出す。
///
/// 並べたときの字の位置は、隣どうしの詰め（カーニング）を除けば足し算で決まります。
/// そこで、変わったところの前後 1 字ずつだけを並べ直し、手前はそのまま、後ろは
/// ずれたぶんだけ足します。1 字打つたびに長い文字を丸ごと並べ直さずに済みます。
fn update_stops(old_text: &str, old_stops: &[(usize, f32)], new_text: &str, font: &Font, style: &TextStyle) -> Vec<(usize, f32)> {
    let stop_at = |stops: &[(usize, f32)], index: usize| {
        stops.binary_search_by_key(&index, |(position, _)| *position).map(|found| stops[found].1).ok()
    };

    // 前と後ろで、同じままの字の長さ（バイト、字の境目にそろえる）。
    let prefix = old_text
        .char_indices()
        .zip(new_text.chars())
        .take_while(|((_, old), new)| old == new)
        .last()
        .map_or(0, |((index, character), _)| index + character.len_utf8());
    let longest_suffix = old_text.len().min(new_text.len()) - prefix;
    let suffix = old_text[prefix..]
        .chars()
        .rev()
        .zip(new_text[prefix..].chars().rev())
        .take_while(|(old, new)| old == new)
        .map(|(character, _)| character.len_utf8())
        .scan(0, |total, length| {
            *total += length;
            Some(*total)
        })
        .take_while(|total| *total <= longest_suffix)
        .last()
        .unwrap_or(0);

    // 詰めを拾うため、前は同じままの最後の 1 字から、後ろは同じままの最初の 1 字まで並べる。
    let start = old_text[..prefix].char_indices().next_back().map_or(0, |(index, _)| index);
    let old_end = old_text[old_text.len() - suffix..].chars().next().map_or(old_text.len(), |character| old_text.len() - suffix + character.len_utf8());
    let new_end = old_end + new_text.len() - old_text.len();

    let (Some(old_prefix_x), Some(old_end_x)) = (stop_at(old_stops, prefix), stop_at(old_stops, old_end)) else {
        // 前の測り方が見当たらなければ（初めてなど）、全部測る。
        return measure_stops(font, new_text, style);
    };

    let local = measure_stops(font, &new_text[start..new_end], style);
    // 並べ直した字の位置を、全体の中の位置へ。同じままの最後の字の右端で合わせる。
    let offset = old_prefix_x - local.iter().find(|(index, _)| start + index == prefix).map_or(0.0, |(_, x)| *x);
    let new_end_x = local.last().map_or(0.0, |(_, x)| *x) + offset;
    let shift = new_end_x - old_end_x;

    old_stops
        .iter()
        .copied()
        .take_while(|(index, _)| *index <= prefix)
        .chain(
            local
                .iter()
                .map(|(index, x)| (start + index, x + offset))
                .filter(|(index, _)| *index > prefix),
        )
        .chain(
            old_stops
                .iter()
                .filter(|(index, _)| *index > old_end)
                .map(|(index, x)| (index + new_text.len() - old_text.len(), x + shift)),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テストを動かす機械にある書体。無ければ `None`（テストは何もせず終わる）。
    fn resources() -> Option<(Resources, FontHandle)> {
        let mut resources = Resources::new();
        let handle = [
            "C:/Windows/Fonts/YuGothM.ttc",
            "C:/Windows/Fonts/arial.ttf",
            "/System/Library/Fonts/Avenir Next.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
        .into_iter()
        .find_map(|path| resources.load_font_file("font", path).ok())?;

        Some((resources, handle))
    }

    #[test]
    fn updating_stops_matches_measuring_everything() {
        let Some((resources, handle)) = resources() else {
            eprintln!("skipped: no font");
            return;
        };
        let font = resources.font(handle).expect("書体を読める");
        let style = TextStyle::new(14.0);

        let edits = [
            ("", "hello"),
            ("hello world", "hello brave world"),
            ("hello world", "hello"),
            ("hello world", "world"),
            ("AVAVAV", "AVXAVAV"),
            ("To Wa", "ToWa"),
            ("日本語のテキスト", "日本語テキスト"),
            ("abc", "xyz"),
            ("aaaa", "aaaaa"),
            ("placeholder", "a"),
        ];

        for (old, new) in edits {
            let old_stops = measure_stops(&font, old, &style);
            let updated = update_stops(old, &old_stops, new, &font, &style);
            let measured = measure_stops(&font, new, &style);

            assert_eq!(updated.len(), measured.len(), "{old:?} -> {new:?}");
            for ((index, x), (expected_index, expected_x)) in updated.iter().zip(&measured) {
                assert_eq!(index, expected_index, "{old:?} -> {new:?}");
                assert!((x - expected_x).abs() < 1e-3, "{old:?} -> {new:?}: {x} != {expected_x} at {index}");
            }
        }
    }
}
