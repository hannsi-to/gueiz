use std::collections::HashMap;
use std::time::{Duration, Instant};
use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::effect::Block;
use crate::font::Font;
use crate::gui::context_menu::{ContextMenuRequest, ContextMenuState, INPUT_TEXT_CONTEXT_MENU};
use crate::gui::input_text::{measure_stops, InputTextFont, InputTextTheme};
use crate::gui::text_edit::TextEdit;
use crate::gui::window_frame::{Gap, QuadType, ThemeColor};
use crate::gui::window_item::{replace_outline, within, Animation, CursorShape, EditKey, KeyModifiers, Pointer, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::{JointType, PaintType};
use crate::resource::Resources;
use crate::text::{layout, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

/// 縁取りの太さ。
const OUTLINE_WIDTH: f32 = 1.0;
/// 字を打つ位置に出す縦棒の太さ。
const CARET_WIDTH: f32 = 1.5;
/// この時間のうちに同じ場所をもう一度押したら、続けて押した（ダブルクリック）とみなす。
const MULTI_CLICK_TIME: Duration = Duration::from_millis(500);
/// 続けて押したとみなす、押した場所のずれ（画素）。
const MULTI_CLICK_DISTANCE: f32 = 4.0;
/// キャレットを点けている・消している長さ。
const BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// 引いたまま欄の外にいるあいだ、縦へ送る間隔。
const AUTO_SCROLL_INTERVAL: Duration = Duration::from_millis(16);
/// 引いたまま欄の外にいるときの送る速さ（1 秒あたり）。外へ離れるほど速くする。
const AUTO_SCROLL_SPEED: f32 = 80.0;
const AUTO_SCROLL_SPEED_PER_DISTANCE: f32 = 8.0;
/// ホイール 1 目盛りで送る行数。
const WHEEL_ROWS: f32 = 3.0;
/// 選んだ範囲が改行をまたぐとき、行の右端に足す幅（字の大きさに対する割合）。改行も選んでいると分かるように。
const NEWLINE_SELECTION_WIDTH: f32 = 0.3;
/// 縦のスクロールバーの太さと、欄の縁からの隙間。
const SCROLLBAR_WIDTH: f32 = 6.0;
const SCROLLBAR_MARGIN: f32 = 2.0;
/// スクロールバーのつまみの高さの下限。
const SCROLLBAR_MIN_THUMB: f32 = 16.0;

/// 複数行の文字を打ち込む欄。Web の `<textarea>` にあたる。
///
/// [`InputText`](crate::gui::input_text::InputText) と同じ操作（選ぶ・単語ごとに動く・元に戻す・
/// コピーと貼り付け・右クリックのメニュー・IME）に加えて、
///
/// - Enter で改行する。Ctrl+Enter（macOS は Option+Enter）で [`InputTextarea::on_submit`] の関数を呼ぶ。
/// - 欄の幅で折り返す。空白のあとで切り、空白の無い長い語は字の途中で切る。
/// - ↑↓ は見た目の行（折り返した行）を動き、行をまたいでも横の位置を保つ。
///   Home・End は見た目の行の先頭・末尾へ、Ctrl を押しながらなら全体の先頭・末尾へ。
/// - キャレットが見えるように縦に送る。ホイールでも送れる。押して引いたまま上下の外へ出ると、
///   外へ離れるほど速く送りながら選ぶ。
/// - 3 回続けて押すと、その 1 行（改行から改行まで）を選ぶ。
/// - PageUp・PageDown で 1 ページ（見えている行数から 1 行引いたぶん）ずつ動く。
/// - Tab で空白を入れる（[`InputTextarea::tab_width`]、既定は 4 つ）。
/// - 中身が欄より高いと、右端に縦のスクロールバーを出す。つまみを引くと送れ、
///   つまみの外を押すと 1 ページ送る。
/// - [`InputTextarea::wrap`] で折り返さないようにもできる。そのときは横にも送る
///   （キャレットを追う・Shift+ホイール・引いたまま左右の外へ）。
///
/// 書体と色は [`InputTextFont`] と [`InputTextTheme`] を使う。
///
/// # 速さ
///
/// 行の分け方は、文字が変わったときだけ出し直します。行ごとの字の幅は前の結果を
/// 覚えておき、同じ行なら測り直しません。置く字は欄に見えている行だけです。
pub struct InputTextarea {
    edit: TextEdit,
    placeholder: String,
    width: f32,
    height: f32,
    /// 内側の余白（上下左右）。
    padding: f32,
    quad_type: QuadType,
    gap: Gap,
    input_text_font: InputTextFont,
    input_text_theme: InputTextTheme,
    on_change: Option<Box<dyn FnMut(&str)>>,
    on_submit: Option<Box<dyn FnMut(&str)>>,
    context_menu: String,
    names: Names,
    scale_mode: ScaleMode,
    /// 組んだときの左上（余白を足す前）。
    built_at: [f32; 2],
    /// 窓を組んだ場所からのずれ。自前の覆いをワールドに置くのに使う。
    offset: [f32; 2],
    applied_clip: Option<Block>,
    focused: bool,
    modifiers: KeyModifiers,
    dragging: bool,
    /// 引いているあいだの指の位置（組んだときの座標）。
    drag_pointer: Option<[f32; 2]>,
    last_auto_scroll: Option<Instant>,
    last_click: Option<Click>,
    blink_start: Instant,
    caret_shown: bool,
    preedit: String,
    preedit_cursor: Option<usize>,
    /// ↑↓ で行をまたぐときに保つ横の位置。横に動いたり書き換えたりすると忘れる。
    goal_x: Option<f32>,
    /// 1 行の高さ。書体から測る（[`InputTextarea::create_object`] で決まる）。
    line_height: f32,
    /// 縦に送っている量。
    scroll: f32,
    /// 横に送っている量（折り返さないときだけ）。
    scroll_x: f32,
    /// ホイールで回した、まだ反映していない量（画素）。字を置き直すので書体のある animate で反映する。
    pending_scroll: f32,
    pending_scroll_x: f32,
    /// スクロールバーで決めた、まだ反映していない縦の送り（画素）。
    scroll_target: Option<f32>,
    /// スクロールバーのつまみを引いているあいだ、つまみの上端から指までの距離。
    scrollbar_drag: Option<f32>,
    /// 欄の幅で折り返すか。
    wrap: bool,
    /// 縦のスクロールバーを出すか（出すなら、右端にそのぶんの幅を空ける）。
    scrollbar: bool,
    /// Tab で入れる空白の数。
    tab_width: usize,
    /// 一番長い見た目の行の幅。折り返さないときの横の送りの上限に使う。
    content_width: f32,
    /// 行の分け方を出した文字（変換中の文字を差し込んだもの）。同じなら出し直さない。
    layout_text: String,
    /// 見た目の行。
    rows: Vec<Row>,
    /// 1 行（改行まで）ごとの字の幅。同じ行なら測り直さない。
    line_stops: HashMap<String, Vec<(usize, f32)>>,
    /// 字を最後に置いたときの、縦と横の送りと、案内の字を出していたか。
    rendered: Option<(f32, f32, bool)>,
}

/// 押したときの覚え。
#[derive(Debug, Copy, Clone)]
struct Click {
    at: Instant,
    position: [f32; 2],
    count: u32,
}

/// 登録した図形の名前。
#[derive(Default)]
struct Names {
    background: Option<String>,
    /// 選んだ範囲の帯。見えている行の数だけ前もって作っておき、使い回す。
    selections: Vec<String>,
    outline: Option<String>,
    /// 縦のスクロールバーの溝とつまみ。
    track: Option<String>,
    thumb: Option<String>,
    caret: Option<String>,
    text: Option<String>,
}

/// 見た目の 1 行。
#[derive(Debug, Clone, PartialEq)]
struct Row {
    /// 出している文字の中の範囲（バイト）。改行は含まない。
    start: usize,
    end: usize,
    /// 字の境目ごとの `(出している文字の中のバイト位置, 行の左端からの幅)`。両端も入る。
    stops: Vec<(usize, f32)>,
    /// 折り返しで終わる行か（改行や文字の終わりで終わる行は `false`）。
    wrapped: bool,
}

impl Row {
    /// 行の中の `index` の幅。行に無ければ `None`。
    fn x(&self, index: usize) -> Option<f32> {
        self.stops
            .binary_search_by_key(&index, |(position, _)| *position)
            .ok()
            .map(|found| self.stops[found].1)
    }

    /// 行の中で `x` に一番近い字の境目。折り返しで終わる行は、末尾（次の行の頭）を選ばない。
    fn nearest(&self, x: f32) -> usize {
        let usable = if self.wrapped && self.stops.len() > 1 { &self.stops[..self.stops.len() - 1] } else { &self.stops[..] };
        let right = usable.partition_point(|(_, stop)| *stop < x).min(usable.len() - 1);
        let left = right.saturating_sub(1);

        if (usable[left].1 - x).abs() <= (usable[right].1 - x).abs() { usable[left].0 } else { usable[right].0 }
    }
}

impl InputTextarea {
    pub fn new(register_name_string: String, placeholder: String, width: f32, height: f32, padding: f32, quad_type: QuadType, gap: Gap, input_text_font: InputTextFont, input_text_theme: InputTextTheme) -> Self {
        let mut edit = TextEdit::new();
        edit.set_multiline(true);

        Self {
            edit,
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
            context_menu: INPUT_TEXT_CONTEXT_MENU.to_string(),
            names: Names { background: Some(register_name_string), ..Names::default() },
            scale_mode: ScaleMode::default(),
            built_at: [0.0; 2],
            offset: [0.0; 2],
            applied_clip: None,
            focused: false,
            modifiers: KeyModifiers::default(),
            dragging: false,
            drag_pointer: None,
            last_auto_scroll: None,
            last_click: None,
            blink_start: Instant::now(),
            caret_shown: false,
            preedit: String::new(),
            preedit_cursor: None,
            goal_x: None,
            line_height: 0.0,
            scroll: 0.0,
            scroll_x: 0.0,
            pending_scroll: 0.0,
            pending_scroll_x: 0.0,
            scroll_target: None,
            scrollbar_drag: None,
            wrap: true,
            scrollbar: true,
            tab_width: 4,
            content_width: 0.0,
            layout_text: String::new(),
            rows: vec![Row { start: 0, end: 0, stops: vec![(0, 0.0)], wrapped: false }],
            line_stops: HashMap::new(),
            rendered: None,
        }
    }

    /// 文字が変わるたびに呼ぶ関数。
    pub fn on_change(mut self, on_change: impl FnMut(&str) + 'static) -> Self {
        self.on_change = Some(Box::new(on_change));
        self
    }

    /// Ctrl+Enter（macOS は Option+Enter）を押したときに呼ぶ関数。Enter だけなら改行する。
    pub fn on_submit(mut self, on_submit: impl FnMut(&str) + 'static) -> Self {
        self.on_submit = Some(Box::new(on_submit));
        self
    }

    /// 入れられる字の数の上限（改行も 1 字に数える）。
    pub fn max_length(mut self, max_length: usize) -> Self {
        self.edit.set_max_length(Some(max_length));
        self
    }

    /// 欄の幅で折り返すか（既定は折り返す）。折り返さなければ、横にも送る。
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// 縦のスクロールバーを出すか（既定は出す）。出すなら、右端にそのぶんの幅を空ける。
    pub fn scrollbar(mut self, scrollbar: bool) -> Self {
        self.scrollbar = scrollbar;
        self
    }

    /// Tab で入れる空白の数（既定は 4）。`0` なら Tab では何も入れない。
    pub fn tab_width(mut self, tab_width: usize) -> Self {
        self.tab_width = tab_width;
        self
    }

    /// 右クリックで開くメニューの名前。既定は [`INPUT_TEXT_CONTEXT_MENU`]。
    pub fn context_menu(mut self, name: impl Into<String>) -> Self {
        self.context_menu = name.into();
        self
    }

    /// いまの文字。変換中の文字は入りません。
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    // --- 寸法 ---

    /// 欄の左上と右下（組んだときの座標）。
    fn bounds(&self) -> [f32; 4] {
        let [x, y] = self.built_at;
        let (left, top) = (x + self.gap.x, y + self.gap.y);

        [left, top, left + self.width, top + self.height]
    }

    /// 字を置く範囲（余白の内側。スクロールバーを出すなら、そのぶん右を空ける）。
    fn inner(&self) -> [f32; 4] {
        let [left, top, right, bottom] = self.bounds();
        let gutter = if self.scrollbar { SCROLLBAR_WIDTH + SCROLLBAR_MARGIN } else { 0.0 };
        [left + self.padding, top + self.padding, right - self.padding - gutter, bottom - self.padding]
    }

    fn inner_width(&self) -> f32 {
        let [left, _, right, _] = self.inner();
        (right - left).max(0.0)
    }

    /// 字を削る範囲。字を置く範囲から余白の半分だけ外。キャレットと帯もこの中にだけ描く。
    fn clip_box(&self) -> [f32; 4] {
        let [left, top, right, bottom] = self.inner();
        let inset = self.padding * 0.5;
        [left - inset, top - inset, right + inset, bottom + inset]
    }

    /// 横の送りの上限（折り返すときは 0）。
    fn max_scroll_x(&self) -> f32 {
        if self.wrap { 0.0 } else { (self.content_width + CARET_WIDTH - self.inner_width()).max(0.0) }
    }

    /// 縦のスクロールバーの溝。出さないなら `None`。
    fn scrollbar_track(&self) -> Option<[f32; 4]> {
        let [_, top, right, bottom] = self.bounds();
        self.scrollbar.then(|| [right - SCROLLBAR_MARGIN - SCROLLBAR_WIDTH, top + SCROLLBAR_MARGIN, right - SCROLLBAR_MARGIN, bottom - SCROLLBAR_MARGIN])
    }

    /// 縦のスクロールバーのつまみ。中身が欄に収まっていれば `None`（出さない）。
    fn scrollbar_thumb(&self) -> Option<[f32; 4]> {
        let [left, top, right, bottom] = self.scrollbar_track()?;
        let max_scroll = self.max_scroll();
        if max_scroll <= 0.0 {
            return None;
        }

        let track_height = bottom - top;
        let content_height = self.rows.len() as f32 * self.line_height;
        let thumb_height = (track_height * self.inner_height() / content_height).clamp(SCROLLBAR_MIN_THUMB.min(track_height), track_height);
        let thumb_top = top + (track_height - thumb_height) * (self.scroll / max_scroll);

        Some([left, thumb_top, right, thumb_top + thumb_height])
    }

    /// つまみの上端を `thumb_top` に置いたときの縦の送り。
    fn scroll_for_thumb(&self, thumb_top: f32) -> f32 {
        let (Some([_, track_top, _, track_bottom]), Some([_, top, _, bottom])) = (self.scrollbar_track(), self.scrollbar_thumb()) else {
            return self.scroll;
        };
        let room = (track_bottom - track_top) - (bottom - top);
        if room <= 0.0 {
            return self.scroll;
        }

        ((thumb_top - track_top) / room * self.max_scroll()).clamp(0.0, self.max_scroll())
    }

    /// 1 ページで動く行数。見えている行数から 1 行引く（前のページの最後の行を残す）。
    fn page_rows(&self) -> usize {
        ((self.inner_height() / self.line_height.max(1.0)).floor() as usize).saturating_sub(1).max(1)
    }

    fn inner_height(&self) -> f32 {
        (self.height - self.padding * 2.0).max(0.0)
    }

    /// 1 行目の上端（組んだときの座標）。縦の送りも入る。
    fn text_top(&self) -> f32 {
        self.inner()[1] - self.scroll
    }

    fn max_scroll(&self) -> f32 {
        (self.rows.len() as f32 * self.line_height - self.inner_height()).max(0.0)
    }

    // --- 出す文字と位置 ---

    fn showing_placeholder(&self) -> bool {
        self.edit.text().is_empty() && self.preedit.is_empty()
    }

    /// 出す文字。変換中の文字は、字を打つ位置に差し込む。
    fn display_text(&self) -> String {
        if self.showing_placeholder() {
            return self.placeholder.clone();
        }

        let (text, cursor) = (self.edit.text(), self.edit.cursor());
        let mut display = String::with_capacity(text.len() + self.preedit.len());
        display.push_str(&text[..cursor]);
        display.push_str(&self.preedit);
        display.push_str(&text[cursor..]);
        display
    }

    /// 文字の中のバイト位置を、出している文字の中の位置へ。
    fn display_index(&self, index: usize) -> usize {
        if self.showing_placeholder() {
            0
        } else if index <= self.edit.cursor() {
            index
        } else {
            index + self.preedit.len()
        }
    }

    /// 出している文字の中の位置を、文字の中のバイト位置へ（変換中の文字の中なら、字を打つ位置）。
    fn text_index(&self, display_index: usize) -> usize {
        if self.showing_placeholder() {
            return 0;
        }

        let cursor = self.edit.cursor();
        if display_index <= cursor {
            display_index
        } else if display_index >= cursor + self.preedit.len() {
            display_index - self.preedit.len()
        } else {
            cursor
        }
    }

    /// 出している文字の中で、字を打つ位置。
    fn display_cursor(&self) -> usize {
        if self.showing_placeholder() {
            return 0;
        }

        self.edit.cursor() + self.preedit_cursor.unwrap_or(self.preedit.len()).min(self.preedit.len())
    }

    /// 出している文字の `index` がある見た目の行。折り返しの境目は、次の行の頭とみなす。
    fn row_of(&self, index: usize) -> usize {
        self.rows.partition_point(|row| row.start <= index).saturating_sub(1)
    }

    /// 出している文字の `index` の位置 `[x, 行]`（x は字を置く左端から）。
    fn position_of(&self, index: usize) -> (f32, usize) {
        let row = self.row_of(index);
        (self.rows[row].x(index).unwrap_or(0.0), row)
    }

    /// 縦棒の位置と大きさ（組んだときの座標）。
    fn caret_area_local(&self) -> [f32; 4] {
        let (x, row) = self.position_of(self.display_cursor());
        let height = (self.input_text_font.base_font_size * 1.2).min(self.line_height.max(1.0));
        let y = self.text_top() + row as f32 * self.line_height + (self.line_height - height) / 2.0;

        [self.inner()[0] + x - self.scroll_x, y, CARET_WIDTH, height]
    }

    /// 組んだときの座標の点に一番近い字の境目（出している文字の中）。
    fn display_index_at(&self, [x, y]: [f32; 2]) -> usize {
        let row = ((y - self.text_top()) / self.line_height.max(1.0)).floor();
        let row = (row.max(0.0) as usize).min(self.rows.len() - 1);

        self.rows[row].nearest(x - self.inner()[0] + self.scroll_x)
    }

    /// 画素の位置を、組んだときの座標へ。窓のずれもここで外れる。
    fn local_point(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<[f32; 2]> {
        let object = draw_manager.object(self.names.background.as_deref()?)?;
        let [world_x, world_y] = object.view_camera().screen_to_world(x, y)?;

        object.to_local(0, world_x, world_y)
    }

    /// 押した画素の位置に一番近い、文字の中の字の境目。
    fn cursor_at(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<usize> {
        let point = self.local_point(draw_manager, x, y)?;
        Some(self.text_index(self.display_index_at(point)))
    }

    // --- 並べる ---

    /// 文字が変わっていれば行を分け直し、キャレットが見えるように縦に送り（`follow` のとき）、
    /// 縦の送りか文字が変わっていれば字を置き直す。縁取り・帯・キャレットも組み直す。
    fn relayout(&mut self, draw_manager: &mut DrawManager, resources: &Resources, follow: bool) -> bool {
        let Some(font) = resources.font(self.input_text_font.base_font) else {
            log::warn!("the input text font is not in the resources; the text is not drawn");
            return false;
        };
        let style = self.text_style();

        let display = self.display_text();
        if display != self.layout_text {
            self.rows = self.break_rows(&font, &style, &display);
            self.layout_text = display;
            self.rendered = None;
        }

        if follow {
            let (x, row) = self.position_of(self.display_cursor());
            let top = row as f32 * self.line_height;
            if top < self.scroll {
                self.scroll = top;
            }
            if top + self.line_height > self.scroll + self.inner_height() {
                self.scroll = top + self.line_height - self.inner_height();
            }

            // 折り返さないときは、横にもキャレットを追う。
            if !self.wrap {
                if x < self.scroll_x {
                    self.scroll_x = x;
                }
                if x + CARET_WIDTH > self.scroll_x + self.inner_width() {
                    self.scroll_x = x + CARET_WIDTH - self.inner_width();
                }
            }
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        self.scroll_x = self.scroll_x.clamp(0.0, self.max_scroll_x());

        let rendered = (self.scroll, self.scroll_x, self.showing_placeholder());
        if self.rendered != Some(rendered) {
            self.render_text(draw_manager, &font, style);
            self.rendered = Some(rendered);
            self.apply_clip(draw_manager);
        }

        self.refresh_shapes(draw_manager);
        true
    }

    /// 字の並べ方。行の高さをそろえて、改行で区切った行がそのまま見た目の行になるように。
    fn text_style(&self) -> TextStyle {
        let mut style = TextStyle::new(self.input_text_font.base_font_size);
        if self.line_height > 0.0 {
            style.line_height = Some(self.line_height);
        }
        style
    }

    /// 文字を見た目の行に分ける。1 行（改行まで）ごとに字の幅を測り（前と同じ行なら覚えたものを使う）、
    /// 欄の幅で折り返す。
    fn break_rows(&mut self, font: &Font, style: &TextStyle, display: &str) -> Vec<Row> {
        let wrap_width = if self.wrap { self.inner_width() } else { f32::INFINITY };
        let mut cache = std::mem::take(&mut self.line_stops);
        let mut rows = Vec::new();
        let mut line_start = 0;

        for line in display.split('\n') {
            let stops = match cache.get(line) {
                Some(stops) => stops.clone(),
                None => measure_stops(font, line, style),
            };

            wrap_line(line, line_start, &stops, wrap_width, &mut rows);
            self.line_stops.insert(line.to_string(), stops);
            line_start += line.len() + 1;
        }

        cache.clear();
        self.content_width = rows.iter().map(|row: &Row| row.stops.last().map_or(0.0, |(_, x)| *x)).fold(0.0, f32::max);
        rows
    }

    /// 見えている行（と前後に 1 行）の字を置き直す（字の形を登録し直す）。
    fn render_text(&self, draw_manager: &mut DrawManager, font: &Font, style: TextStyle) {
        let line_height = self.line_height.max(1.0);
        let first = ((self.scroll / line_height).floor() as usize).saturating_sub(1);
        let last = (((self.scroll + self.inner_height()) / line_height).ceil() as usize + 1).min(self.rows.len());

        let text = self.rows[first.min(last)..last]
            .iter()
            .map(|row| &self.layout_text[row.start..row.end])
            .collect::<Vec<_>>()
            .join("\n");

        let theme = &self.input_text_theme;
        let color = if self.showing_placeholder() { &theme.placeholder_color } else { &theme.text_color };
        let left = self.inner()[0] - self.scroll_x;
        let top = self.text_top() + first as f32 * line_height;
        let layout_data = TextLayoutData {
            text,
            style,
            text_location: TextLocation::Area {
                // 行は自分で分けたので、寄せも折り返しもさせない。はみ出しは覆いで削る。
                area: TextArea::new(left, top, self.content_width + self.inner_width() + 1.0e4, line_height * (last - first.min(last) + 1) as f32),
            },
        };
        let color = [color.r, color.g, color.b, color.a];

        if let Some(name) = self.names.text.as_deref() {
            draw_manager.with_text_renderer(name, |text_renderer, draw_manager| {
                text_renderer.clear(draw_manager);
                text_renderer.color(color[0], color[1], color[2], color[3]);
                text_renderer.text_layout_data(layout_data);

                if let Err(error) = text_renderer.register_draw_manager(draw_manager, font) {
                    log::warn!("failed to lay out the textarea: {error}");
                }
            });
        }
    }

    /// 欄の外にはみ出した字を削る山を、字の形すべてに積む。削る範囲はワールドなので、窓のずれを足す。
    fn apply_clip(&mut self, draw_manager: &mut DrawManager) {
        let Some(text) = self.names.text.as_deref() else {
            return;
        };

        let [left, top, right, bottom] = self.clip_box();
        let [x, y] = self.offset;
        let block = Block::ClipRect {
            min: [left + x, top + y],
            max: [right + x, bottom + y],
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

    // --- 形 ---

    fn vertex(x: f32, y: f32, color: &ThemeColor) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a)
    }

    fn build_box(&self, name: &str, paint_type: PaintType, color: &ThemeColor) -> Object {
        let [left, top, right, bottom] = self.bounds();

        match self.quad_type {
            QuadType::Normal => Rect::new(name).paint_type(paint_type).from(Self::vertex(left, top, color)).to(Self::vertex(right, bottom, color)).end(),
            QuadType::Rounded { radius } => RectRounded::new(name)
                .paint_type(paint_type)
                .from(CornerType::Circle { radius }, Self::vertex(left, top, color))
                .to(Self::vertex(right, bottom, color))
                .end(),
        }
    }

    fn build_outline(&self, name: &str) -> Object {
        let theme = &self.input_text_theme;
        let color = if self.focused { &theme.focus_outline_color } else { &theme.outline_color };
        let stroke = PaintType::Stroke { line_width: OUTLINE_WIDTH, joint_type: JointType::None, strip: false, dash: None };

        self.build_box(name, stroke, color)
    }

    /// 四角。`visible` でなければ透明にする。
    fn build_rect(name: &str, [left, top, right, bottom]: [f32; 4], color: &ThemeColor, visible: bool) -> Object {
        let color = ThemeColor { a: if visible { color.a } else { 0.0 }, ..*color };

        Rect::new(name)
            .paint_type(PaintType::Fill)
            .from(Self::vertex(left, top, &color))
            .to(Self::vertex(right.max(left), bottom.max(top), &color))
            .end()
    }

    /// キャレット。点けていないか、字を削る範囲の外なら透明。
    fn build_caret(&self, name: &str) -> Object {
        let [x, y, width, height] = self.caret_area_local();
        let [left, top, right, bottom] = self.clip_box();
        let inside = x >= left && x + width <= right && y >= top && y + height <= bottom;

        Self::build_rect(name, [x, y, x + width, y + height], &self.input_text_theme.caret_color, self.caret_shown && inside)
    }

    /// 選んだ範囲の帯を、見えている行ごとに。帯の数（作っておいた数）を超える行は出さない。
    fn selection_rects(&self) -> Vec<[f32; 4]> {
        let Some(range) = self.edit.selection().filter(|_| self.focused && self.preedit.is_empty() && !self.showing_placeholder()) else {
            return Vec::new();
        };
        let (start, end) = (self.display_index(range.start), self.display_index(range.end));

        let inner_left = self.inner()[0] - self.scroll_x;
        let [left, top, right, bottom] = self.clip_box();
        let line_height = self.line_height.max(1.0);
        let newline_width = self.input_text_font.base_font_size * NEWLINE_SELECTION_WIDTH;

        let first = self.row_of(start);
        let last = self.row_of(end);
        let [_, caret_top, _, caret_height] = self.caret_area_local();
        let caret_offset = caret_top - (self.text_top() + self.row_of(self.display_cursor()) as f32 * line_height);

        (first..=last)
            .filter_map(|index| {
                let row = &self.rows[index];
                let from = row.x(start.max(row.start)).unwrap_or(0.0);
                let mut to = row.x(end.min(row.end)).unwrap_or(0.0);
                // 改行まで選んでいるなら、改行のぶんを少し足す。
                if end > row.end && !row.wrapped {
                    to += newline_width;
                }
                if to <= from {
                    return None;
                }

                let y = self.text_top() + index as f32 * line_height + caret_offset;
                let rect = [
                    (inner_left + from).max(left),
                    y.max(top),
                    (inner_left + to).min(right),
                    (y + caret_height).min(bottom),
                ];
                (rect[2] > rect[0] && rect[3] > rect[1]).then_some(rect)
            })
            .take(self.names.selections.len())
            .collect()
    }

    /// 縁取り・選んだ範囲・キャレットを、いまの状態で組み直す。字は触らないので書体は要らない。
    fn refresh_shapes(&self, draw_manager: &mut DrawManager) {
        if let Some(object) = self.names.outline.as_deref().and_then(|name| draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_outline(""));
        }

        let rects = self.selection_rects();
        let color = self.input_text_theme.selection_color;
        for (index, name) in self.names.selections.iter().enumerate() {
            if let Some(object) = draw_manager.object_mut(name) {
                let built = match rects.get(index) {
                    Some(rect) => Self::build_rect("", *rect, &color, true),
                    None => Self::build_rect("", [0.0; 4], &color, false),
                };
                replace_outline(object, &built);
            }
        }

        // スクロールバー。中身が欄に収まっていれば出さない。
        let thumb = self.scrollbar_thumb();
        let theme = &self.input_text_theme;
        let track_color = ThemeColor { a: theme.outline_color.a * 0.35, ..theme.outline_color };
        let bars = [
            (self.names.track.as_deref(), self.scrollbar_track().filter(|_| thumb.is_some()), track_color),
            (self.names.thumb.as_deref(), thumb, theme.outline_color),
        ];
        for (name, rect, color) in bars {
            if let Some(object) = name.and_then(|name| draw_manager.object_mut(name)) {
                let built = match rect {
                    Some(rect) => Self::build_bar("", rect, &color, true),
                    None => Self::build_bar("", [0.0; 4], &color, false),
                };
                replace_outline(object, &built);
            }
        }

        if let Some(object) = self.names.caret.as_deref().and_then(|name| draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_caret(""));
        }
    }

    /// スクロールバーの溝とつまみ。角を丸める。`visible` でなければ透明。
    fn build_bar(name: &str, [left, top, right, bottom]: [f32; 4], color: &ThemeColor, visible: bool) -> Object {
        let color = ThemeColor { a: if visible { color.a } else { 0.0 }, ..*color };
        let radius = ((right - left) / 2.0).min((bottom - top) / 2.0);
        if !visible || radius <= 0.0 {
            return Self::build_rect(name, [left, top, right, bottom], &color, visible);
        }

        RectRounded::new(name)
            .paint_type(PaintType::Fill)
            .from(CornerType::Circle { radius }, Self::vertex(left, top, &color))
            .to(Self::vertex(right, bottom, &color))
            .end()
    }

    /// 組んだときの座標の点が、出ているスクロールバーの溝の上か。
    fn on_scrollbar(&self, [x, y]: [f32; 2]) -> bool {
        self.scrollbar_thumb().is_some()
            && self.scrollbar_track().is_some_and(|[left, top, right, bottom]| (left..=right).contains(&x) && (top..=bottom).contains(&y))
    }

    // --- 書き換え・その他 ---

    fn changed(&mut self) {
        self.goal_x = None;
        if let Some(on_change) = self.on_change.as_mut() {
            on_change(self.edit.text());
        }
    }

    fn reset_blink(&mut self) {
        self.blink_start = Instant::now();
        self.caret_shown = self.focused;
    }

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
    ///
    /// [`DrawManager::pick`] は全図形をなめるので重い。指が動くたびに呼ばれるので、
    /// 枠（縁取りの太さを含む）の外なら座標だけで断る。欄の外へ送られて見えていない字にも当たらなくなる。
    fn hit(&self, draw_manager: &DrawManager, pointer: &Pointer) -> bool {
        let inside = self
            .local_point(draw_manager, pointer.x, pointer.y)
            .is_some_and(|point| within(self.bounds(), OUTLINE_WIDTH, point));
        if !inside {
            return false;
        }

        let Some(pick) = pointer.pick(draw_manager) else {
            return false;
        };

        let names = &self.names;
        let own = [names.background.as_ref(), names.outline.as_ref(), names.caret.as_ref(), names.track.as_ref(), names.thumb.as_ref()];
        if own.into_iter().flatten().chain(&names.selections).any(|name| *name == pick.name) {
            return true;
        }

        names.text.as_deref().is_some_and(|text| draw_manager.group_names(text).any(|name| name == pick.name))
    }

    /// ↑↓ で見た目の行を `delta` 行動く。端の行なら全体の先頭・末尾へ。
    fn move_rows(&mut self, delta: isize, extend: bool) {
        let cursor = self.display_index(self.edit.cursor());
        let (x, row) = self.position_of(cursor);
        let goal_x = *self.goal_x.get_or_insert(x);

        let target = row as isize + delta;
        let index = if target < 0 {
            0
        } else if target as usize >= self.rows.len() {
            self.layout_text.len()
        } else {
            self.rows[target as usize].nearest(goal_x)
        };

        let goal_x = self.goal_x;
        self.edit.move_to(self.text_index(index), extend);
        self.goal_x = goal_x;
    }

    /// 見た目の行の先頭・末尾へ。折り返しで終わる行の末尾は、最後の字の手前（次の行の頭にならないよう）。
    fn move_row_edge(&mut self, end: bool, extend: bool) {
        let row = &self.rows[self.row_of(self.display_index(self.edit.cursor()))];
        let index = if !end {
            row.start
        } else if row.wrapped {
            row.stops.iter().rev().nth(1).map_or(row.end, |(index, _)| *index)
        } else {
            row.end
        };

        self.edit.move_to(self.text_index(index), extend);
    }

    /// 引いたまま上下の外にいれば、縦に送りながら指の側の端まで選ぶ。送ったら `true`。
    fn auto_scroll(&mut self, draw_manager: &mut DrawManager, resources: &Resources, now: Instant) -> bool {
        let Some([pointer_x, pointer_y]) = self.drag_pointer.filter(|_| self.dragging) else {
            self.last_auto_scroll = None;
            return false;
        };

        let [inner_left, inner_top, inner_right, inner_bottom] = self.inner();
        let outside = |pointer: f32, low: f32, high: f32| {
            if pointer < low {
                pointer - low
            } else if pointer > high {
                pointer - high
            } else {
                0.0
            }
        };
        let over_y = outside(pointer_y, inner_top, inner_bottom);
        let over_x = if self.wrap { 0.0 } else { outside(pointer_x, inner_left, inner_right) };
        if over_x == 0.0 && over_y == 0.0 {
            self.last_auto_scroll = None;
            return false;
        }

        let elapsed = self
            .last_auto_scroll
            .map_or(AUTO_SCROLL_INTERVAL, |last| now.duration_since(last))
            .min(Duration::from_millis(100))
            .as_secs_f32();
        self.last_auto_scroll = Some(now);

        let step = |over: f32| over.signum() * (AUTO_SCROLL_SPEED + over.abs() * AUTO_SCROLL_SPEED_PER_DISTANCE) * elapsed;
        if over_y != 0.0 {
            self.scroll = (self.scroll + step(over_y)).clamp(0.0, self.max_scroll());
        }
        if over_x != 0.0 {
            self.scroll_x = (self.scroll_x + step(over_x)).clamp(0.0, self.max_scroll_x());
        }

        let point = [pointer_x.clamp(inner_left, inner_right), pointer_y.clamp(inner_top, inner_bottom - 0.5)];
        let index = self.text_index(self.display_index_at(point));
        self.edit.move_to(index, true);

        self.relayout(draw_manager, resources, false)
    }
}

/// 1 行（改行まで）を、幅 `wrap_width` で見た目の行に分けて `rows` に足す。
///
/// `stops` はその行の字の境目（行の中のバイト位置と、行の頭からの幅）。
/// 空白のあとで切り、空白の無い長い語は字の途中で切る。どの見た目の行も 1 字は入れる。
fn wrap_line(line: &str, line_start: usize, stops: &[(usize, f32)], wrap_width: f32, rows: &mut Vec<Row>) {
    let mut row_start = 0;

    loop {
        let start_x = stops[row_start].1;
        // この見た目の行に入りきる最後の境目。1 字は必ず入れる。
        let fits = stops[row_start + 1..]
            .iter()
            .take_while(|(_, x)| x - start_x <= wrap_width)
            .count();
        let last = row_start + fits.max(1).min(stops.len() - 1 - row_start);

        if last >= stops.len() - 1 {
            rows.push(make_row(line_start, &stops[row_start..], false));
            return;
        }

        // 入りきらない。空白のすぐ後ろで切れるなら、そこで切る。
        let break_at = (row_start + 1..=last)
            .rev()
            .find(|index| line[..stops[*index].0].ends_with(char::is_whitespace))
            .unwrap_or(last);

        rows.push(make_row(line_start, &stops[row_start..=break_at], true));
        row_start = break_at;
    }
}

fn make_row(line_start: usize, stops: &[(usize, f32)], wrapped: bool) -> Row {
    let start_x = stops[0].1;
    Row {
        start: line_start + stops[0].0,
        end: line_start + stops[stops.len() - 1].0,
        stops: stops.iter().map(|(index, x)| (line_start + index, x - start_x)).collect(),
        wrapped,
    }
}

/// 行の高さを測る。2 行並べて、行の上端の差を見る。
fn measure_line_height(font: &Font, style: &TextStyle) -> f32 {
    let placed = layout(font, "Ag\nAg", style);

    match placed.rows.as_slice() {
        [first, second, ..] => second.top - first.top,
        _ => style.size * 1.2,
    }
}

impl WindowItem for InputTextarea {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let counter = window_item_arguments.counter;
        let Some(local_name) = self.names.background.as_deref() else {
            return (0.0, 0.0);
        };
        let name = format!("{register_name}-{local_name}-{counter}");

        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];
        self.scale_mode = window_item_arguments.scale_mode;

        if let Some(font) = window_item_arguments.resources.font(self.input_text_font.base_font) {
            self.line_height = measure_line_height(&font, &TextStyle::new(self.input_text_font.base_font_size));
        } else {
            self.line_height = self.input_text_font.base_font_size * 1.2;
        }

        let surface_size = window_item_arguments.surface_size;
        let design = window_item_arguments.design;
        let scale_mode = window_item_arguments.scale_mode;
        let draw_manager = window_item_arguments.draw_manager;
        let register = |mut object: Object, draw_manager: &mut DrawManager| {
            object.camera_for(surface_size, design, scale_mode);
            object.instance(create_instance());
            draw_manager.register(object)
        };

        // 地、選んだ範囲の帯、縁取り、字、キャレットの順に重ねる。
        let background = register(self.build_box(&name, PaintType::Fill, &self.input_text_theme.fill_color), draw_manager);

        // 帯は、見えている行の数（と上下に 1 行ずつ）だけ作っておく。
        let visible_rows = (self.inner_height() / self.line_height.max(1.0)).ceil() as usize + 2;
        let color = self.input_text_theme.selection_color;
        let selections = (0..visible_rows)
            .map(|index| register(Self::build_rect(&format!("{background}-selection-{index}"), [0.0; 4], &color, false), draw_manager))
            .collect();

        let outline = register(self.build_outline(&format!("{background}-outline")), draw_manager);
        let track = register(Self::build_rect(&format!("{background}-track"), [0.0; 4], &color, false), draw_manager);
        let thumb = register(Self::build_rect(&format!("{background}-thumb"), [0.0; 4], &color, false), draw_manager);

        let text_name = format!("{background}-text");
        let mut text_renderer = TextRenderer::new(&text_name);
        text_renderer.camera_for(surface_size, design, scale_mode);
        if let Some(replaced) = draw_manager.add_text_renderer(text_renderer) {
            log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
        }

        let caret = register(self.build_caret(&format!("{background}-caret")), draw_manager);

        self.names = Names {
            background: Some(background),
            selections,
            outline: Some(outline),
            track: Some(track),
            thumb: Some(thumb),
            caret: Some(caret),
            text: Some(text_name),
        };

        self.relayout(draw_manager, window_item_arguments.resources, false);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];
        self.rendered = None;

        if let Some(object) = self.names.background.as_deref().and_then(|name| window_item_arguments.draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_box("", PaintType::Fill, &self.input_text_theme.fill_color));
        }

        self.relayout(window_item_arguments.draw_manager, window_item_arguments.resources, false);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let names = &self.names;
        let mut owned: Vec<String> = [&names.background, &names.outline, &names.track, &names.thumb, &names.caret].into_iter().flatten().cloned().collect();
        owned.extend(names.selections.iter().cloned());

        if let Some(text) = &names.text {
            owned.extend(draw_manager.group_names(text).map(String::from));
        }

        owned
    }

    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        let scale_mode = self.scale_mode;
        if let Some(text) = self.names.text.as_deref() {
            draw_manager.with_text_renderer(text, |text_renderer, _| {
                text_renderer.camera_for(surface_size, design, scale_mode);
            });
        }
    }

    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        if !self.hit(draw_manager, &Pointer::new(x, y)) {
            return false;
        }

        // スクロールバーの上なら、つまみを掴むか、1 ページ送る。字は選ばない。
        if let Some(point) = self.local_point(draw_manager, x, y).filter(|point| self.on_scrollbar(*point)) {
            match self.scrollbar_thumb() {
                Some([_, top, _, bottom]) if (top..=bottom).contains(&point[1]) => self.scrollbar_drag = Some(point[1] - top),
                Some([_, top, _, _]) => {
                    let page = self.page_rows() as f32 * self.line_height;
                    let delta = if point[1] < top { -page } else { page };
                    self.scroll_target = Some((self.scroll + delta).clamp(0.0, self.max_scroll()));
                }
                None => {}
            }
            return true;
        }

        self.focused = true;
        self.goal_x = None;
        let count = self.count_click(x, y);

        if self.preedit.is_empty() && !self.showing_placeholder() {
            if let Some(index) = self.cursor_at(draw_manager, x, y) {
                match count {
                    1 => {
                        self.edit.move_to(index, self.modifiers.shift);
                        self.dragging = true;
                        self.drag_pointer = self.local_point(draw_manager, x, y);
                    }
                    2 => self.edit.select_word_at(index),
                    _ => self.edit.select_line_at(index),
                }
            }
        }

        self.reset_blink();
        self.refresh_shapes(draw_manager);
        true
    }

    fn mouse_left_released(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        self.scrollbar_drag = None;
        self.drag_pointer = None;
        self.last_auto_scroll = None;
        std::mem::take(&mut self.dragging)
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        let (x, y) = (pointer.x, pointer.y);
        // つまみを引いている。指に合わせて送る（字を置き直すので animate で反映する）。
        if let Some(grab) = self.scrollbar_drag {
            let Some([_, pointer_y]) = self.local_point(draw_manager, x, y) else {
                return false;
            };
            let target = self.scroll_for_thumb(pointer_y - grab);
            if target == self.scroll {
                return false;
            }
            self.scroll_target = Some(target);
            return true;
        }

        if !self.dragging {
            return false;
        }

        // 選ぶのは欄の内側の端まで。その先は animate が縦に送りながら広げる。
        self.drag_pointer = self.local_point(draw_manager, x, y);
        let Some([pointer_x, pointer_y]) = self.drag_pointer else {
            return false;
        };
        let [inner_left, inner_top, inner_right, inner_bottom] = self.inner();
        let point = [pointer_x.clamp(inner_left, inner_right), pointer_y.clamp(inner_top, inner_bottom - 0.5)];
        let index = self.text_index(self.display_index_at(point));
        if index == self.edit.cursor() {
            return false;
        }

        self.edit.move_to(index, true);
        self.goal_x = None;
        self.reset_blink();
        self.refresh_shapes(draw_manager);
        true
    }

    fn mouse_wheel(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32, [lines_x, lines_y]: [f32; 2]) -> bool {
        if !self.hit(draw_manager, &Pointer::new(x, y)) {
            return false;
        }

        // 字を置き直すので、書体のある animate で反映する。横は折り返さないときだけ。
        self.pending_scroll -= lines_y * WHEEL_ROWS * self.line_height;
        if !self.wrap {
            self.pending_scroll_x -= lines_x * WHEEL_ROWS * self.line_height;
        }
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
        self.edit.selected_text().map(String::from)
    }

    fn context_menu(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> Option<ContextMenuRequest> {
        if !self.hit(draw_manager, &Pointer::new(x, y)) {
            return None;
        }

        self.focused = true;
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
                can_copy: true,
                editable: true,
                has_text: !self.edit.text().is_empty(),
            },
        })
    }

    fn animate(&mut self, draw_manager: &mut DrawManager, resources: &Resources, now: Instant) -> Animation {
        let mut animation = Animation::default();

        // ホイールやスクロールバーで送ったぶん。フォーカスが無くても送る。
        if self.pending_scroll != 0.0 || self.pending_scroll_x != 0.0 || self.scroll_target.is_some() {
            let target = self.scroll_target.take().unwrap_or(self.scroll);
            self.scroll = (target + std::mem::take(&mut self.pending_scroll)).clamp(0.0, self.max_scroll());
            self.scroll_x = (self.scroll_x + std::mem::take(&mut self.pending_scroll_x)).clamp(0.0, self.max_scroll_x());
            animation.relaid_out |= self.relayout(draw_manager, resources, false);
        }

        if !self.focused {
            return animation;
        }

        if self.auto_scroll(draw_manager, resources, now) {
            animation.relaid_out = true;
        }
        if self.last_auto_scroll.is_some() {
            animation.next = Some(now + AUTO_SCROLL_INTERVAL);
        }

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
        self.relayout(draw_manager, resources, true)
    }

    fn paste(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        if !self.preedit.is_empty() || !self.edit.paste(text) {
            return false;
        }

        self.changed();
        self.reset_blink();
        self.relayout(draw_manager, resources, true)
    }

    fn ime_preedit(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str, cursor: Option<usize>) -> bool {
        if self.preedit == text && self.preedit_cursor == cursor {
            return false;
        }

        if !text.is_empty() && self.edit.delete_selection() {
            self.changed();
        }

        self.preedit = text.to_string();
        self.preedit_cursor = cursor;

        self.reset_blink();
        self.relayout(draw_manager, resources, true)
    }

    fn key_pressed(&mut self, draw_manager: &mut DrawManager, resources: &Resources, key: EditKey) -> bool {
        if !self.preedit.is_empty() {
            return false;
        }

        let KeyModifiers { shift, word } = self.modifiers;
        let before = (self.edit.cursor(), self.edit.selection());
        // 横に動くと、↑↓ で保つ横の位置は忘れる。
        let keep_goal = matches!(key, EditKey::Up | EditKey::Down | EditKey::PageUp | EditKey::PageDown);

        let edited = match key {
            EditKey::Backspace => self.edit.delete_backward(word),
            EditKey::Delete => self.edit.delete_forward(word),
            EditKey::Undo => self.edit.undo(),
            EditKey::Redo => self.edit.redo(),
            EditKey::Enter if word => {
                if let Some(on_submit) = self.on_submit.as_mut() {
                    on_submit(self.edit.text());
                }
                return false;
            }
            EditKey::Enter => self.edit.insert("\n"),
            EditKey::Left => {
                self.edit.move_left(shift, word);
                false
            }
            EditKey::Right => {
                self.edit.move_right(shift, word);
                false
            }
            EditKey::Up => {
                self.move_rows(-1, shift);
                false
            }
            EditKey::Down => {
                self.move_rows(1, shift);
                false
            }
            // 1 ページぶん、見ているところとキャレットを一緒に動かす。
            EditKey::PageUp | EditKey::PageDown => {
                let rows = self.page_rows() as isize * if key == EditKey::PageUp { -1 } else { 1 };
                self.scroll = (self.scroll + rows as f32 * self.line_height).clamp(0.0, self.max_scroll());
                self.move_rows(rows, shift);
                false
            }
            EditKey::Tab if self.tab_width > 0 => self.edit.insert(&" ".repeat(self.tab_width)),
            EditKey::Tab => return false,
            EditKey::Home if word => {
                self.edit.move_home(shift);
                false
            }
            EditKey::End if word => {
                self.edit.move_end(shift);
                false
            }
            EditKey::Home => {
                self.move_row_edge(false, shift);
                false
            }
            EditKey::End => {
                self.move_row_edge(true, shift);
                false
            }
            EditKey::SelectAll => {
                self.edit.select_all();
                false
            }
            EditKey::Escape => return self.unfocus(draw_manager),
        };

        if !keep_goal {
            self.goal_x = None;
        }

        if edited {
            self.changed();
        } else if before == (self.edit.cursor(), self.edit.selection()) {
            return false;
        }

        self.reset_blink();
        self.relayout(draw_manager, resources, true)
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

    fn cursor_shape(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<CursorShape> {
        let (x, y) = (pointer.x, pointer.y);
        if !self.hit(draw_manager, pointer) {
            return None;
        }

        let on_scrollbar = self.scrollbar_drag.is_some() || self.local_point(draw_manager, x, y).is_some_and(|point| self.on_scrollbar(point));
        Some(if on_scrollbar { CursorShape::Default } else { CursorShape::Text })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 字の幅が 10 の、行の字の境目。
    fn stops(line: &str) -> Vec<(usize, f32)> {
        std::iter::once((0, 0.0))
            .chain(line.char_indices().enumerate().map(|(count, (index, character))| (index + character.len_utf8(), (count + 1) as f32 * 10.0)))
            .collect()
    }

    fn rows(line: &str, width: f32) -> Vec<(String, bool)> {
        let mut rows = Vec::new();
        wrap_line(line, 0, &stops(line), width, &mut rows);
        rows.into_iter().map(|row| (line[row.start..row.end].to_string(), row.wrapped)).collect()
    }

    #[test]
    fn wraps_after_spaces_and_breaks_long_words() {
        // 幅 60 には 6 字入る。
        assert_eq!(rows("hello world", 60.0), vec![("hello ".to_string(), true), ("world".to_string(), false)]);
        assert_eq!(rows("abcdefghij", 60.0), vec![("abcdef".to_string(), true), ("ghij".to_string(), false)]);
        assert_eq!(rows("ab", 60.0), vec![("ab".to_string(), false)]);
        assert_eq!(rows("", 60.0), vec![("".to_string(), false)]);
        // 1 字も入らない幅でも、1 字ずつは並べる。
        assert_eq!(rows("abc", 5.0).len(), 3);
    }

    #[test]
    fn wrapped_rows_do_not_offer_their_end() {
        let mut wrapped = Vec::new();
        wrap_line("hello world", 0, &stops("hello world"), 60.0, &mut wrapped);

        // 1 行目の右端を押しても、次の行の頭（6）ではなく、その手前の境目になる。
        assert_eq!(wrapped[0].nearest(1000.0), 5);
        // 最後の行は末尾まで選べる。
        assert_eq!(wrapped[1].nearest(1000.0), 11);
    }
}
