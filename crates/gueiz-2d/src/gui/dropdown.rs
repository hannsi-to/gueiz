use std::time::Instant;
use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::effect::Block;
use crate::gui::window_frame::{Gap, QuadType, ThemeColor};
use crate::gui::window_item::{built_point, replace_outline, within, Animation, CursorShape, EditKey, Pointer, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::{create_instance, Instance};
use crate::object::Object;
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::objects::triangle::Triangle;
use crate::paint_type::PaintType;
use crate::resource::{FontHandle, Resources};
use crate::text::{TextAlign, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

/// 一覧を、窓のほかの部品より手前に出す重なり順。右クリックのメニュー（1000）よりは奥。
const POPUP_Z: f32 = 500.0;
/// 閉じているあいだ、一覧を逃がしておく場所。
const OFFSCREEN: f32 = -1.0e6;
/// 縁取りの太さ。
const OUTLINE_WIDTH: f32 = 1.0;
/// ボタンと一覧のあいだ。
const LIST_GAP: f32 = 2.0;
/// 一覧の上下の内側の余白と、光らせる行の左右の寄せ。
const LIST_PADDING: f32 = 4.0;
const HIGHLIGHT_INSET: f32 = 2.0;
/// 字の左の余白。
const TEXT_PADDING: f32 = 8.0;
/// ▼ の幅。ボタンの右端から、高さぶんの場所のまん中に置く。
const ARROW_WIDTH: f32 = 8.0;
/// 既定で一度に見せる行の数。超えたらホイールで送る。
const DEFAULT_MAX_VISIBLE: usize = 8;
/// スクロールバーの太さと、右端からの寄せ。
const SCROLLBAR_WIDTH: f32 = 4.0;
const SCROLLBAR_INSET: f32 = 3.0;

// 図形の番号。登録する順（＝同じ重なり順の中での前後）でもある。
const BUTTON: usize = 0;
const BUTTON_OUTLINE: usize = 1;
const ARROW: usize = 2;
const LIST: usize = 3;
const LIST_OUTLINE: usize = 4;
const HIGHLIGHT: usize = 5;
const SCROLLBAR: usize = 6;
const PART_COUNT: usize = 7;
const PART_SUFFIXES: [&str; PART_COUNT] = ["", "-outline", "-arrow", "-list", "-list-outline", "-highlight", "-scrollbar"];

/// 項目から 1 つを選ぶ欄。Web の `<select>` と同じ仕組みです。
///
/// 選んでいる項目の字と ▼ を出したボタンで、押すと下に項目の一覧が開きます。
/// 指が乗った行を光らせ、押すとその項目を選んで閉じます。項目が多いときは
/// [`Dropdown::max_visible`] 行まで見せて、ホイールで送ります（右にスクロールバーが出る）。
/// 長い字は欄の中で削ります。
///
/// # キー
///
/// - 閉じているとき：↑← で前、↓→ で次の項目を選ぶ（ブラウザと同じく、開かずに選び変わる）。
///   Home・End で最初・最後。Enter か Space で開く。
/// - 開いているとき：↑↓ で光らせる行を動かし、PageUp・PageDown で 1 画面ぶん、Home・End で端へ。
///   Enter で光らせた項目を選んで閉じ、Esc は選ばずに閉じる。
/// - どちらでも、字を打つとその字で始まる項目へ飛ぶ（同じ字を続けると次の項目へ）。
///
/// 選んだ項目が変わったら [`Dropdown::on_change`] を呼びます（HTML の `input` と `change`）。
///
/// # 重なりと覆い
///
/// 一覧は窓のほかの部品より手前に開きます（重なり順 500）。窓の外へは出られず、
/// はみ出したぶんは窓の覆いで削られるので、窓の高さに余裕を持たせてください。
///
/// # 動かし方
///
/// 頂点を書き換えると全図形の積み直しになるので、頂点は組むときだけ作ります。
/// 光らせる行・スクロールバー・開け閉めは複製（[`Instance`]）の位置と倍率で映します。
/// 項目の字の形は組むときにすべて作っておくので、開いたり送ったりしても字の形は増えません。
pub struct Dropdown {
    /// 登録の名前のもと。
    name: String,
    width: f32,
    height: f32,
    quad_type: QuadType,
    gap: Gap,
    dropdown_font: DropdownFont,
    dropdown_theme: DropdownTheme,
    options: Vec<String>,
    selected: Option<usize>,
    max_visible: usize,
    on_change: Option<Box<dyn FnMut(usize, &str)>>,
    /// 図形の名前。番号は `BUTTON` などの定数。
    parts: Vec<Option<String>>,
    /// ボタンの字と、一覧の字。
    button_text: Option<String>,
    list_text: Option<String>,
    /// 組んだときの左上（余白を足す前）。
    built_at: [f32; 2],
    /// 窓を組んだ場所からのずれ。字を削る範囲（ワールド）に足す。
    offset: [f32; 2],
    /// 字の形に最後に積んだ、はみ出しを削る山（ボタン・一覧）。
    applied_clips: [Option<Block>; 2],
    scale_mode: ScaleMode,
    focused: bool,
    open: bool,
    /// 開いているあいだ、光らせている行（項目の番号）。
    highlighted: Option<usize>,
    /// 一覧のいちばん上に見せている項目の番号。
    scroll: usize,
    /// 字を書き直す要るか。字は書体が要るので animate で書く。
    button_dirty: bool,
    list_dirty: bool,
}

#[derive(Debug, Copy, Clone)]
pub struct DropdownFont {
    pub base_font: FontHandle,
    pub base_font_size: f32,
}

pub struct DropdownTheme {
    /// ボタンの地と縁取り。
    pub fill_color: ThemeColor,
    pub outline_color: ThemeColor,
    /// フォーカスがあるとき・開いているときの縁取り。
    pub focus_outline_color: ThemeColor,
    /// ボタンの字と一覧の字。
    pub text_color: ThemeColor,
    pub arrow_color: ThemeColor,
    /// 一覧の地と縁取り。
    pub list_fill_color: ThemeColor,
    pub list_outline_color: ThemeColor,
    /// 光らせている行の地と字。
    pub highlight_fill_color: ThemeColor,
    pub highlight_text_color: ThemeColor,
    pub scrollbar_color: ThemeColor,
}

impl Dropdown {
    pub fn new(register_name_string: String, width: f32, height: f32, quad_type: QuadType, gap: Gap, dropdown_font: DropdownFont, dropdown_theme: DropdownTheme) -> Self {
        Self {
            name: register_name_string,
            width,
            height,
            quad_type,
            gap,
            dropdown_font,
            dropdown_theme,
            options: Vec::new(),
            selected: None,
            max_visible: DEFAULT_MAX_VISIBLE,
            on_change: None,
            parts: vec![None; PART_COUNT],
            button_text: None,
            list_text: None,
            built_at: [0.0; 2],
            offset: [0.0; 2],
            applied_clips: [None; 2],
            scale_mode: ScaleMode::default(),
            focused: false,
            open: false,
            highlighted: None,
            scroll: 0,
            button_dirty: true,
            list_dirty: true,
        }
    }

    /// 項目。最初の項目を選んだ状態になる（HTML と同じ）。組んだ後には変えられない。
    pub fn options<S: Into<String>>(mut self, options: impl IntoIterator<Item = S>) -> Self {
        self.options = options.into_iter().map(Into::into).collect();
        self.selected = (!self.options.is_empty()).then_some(0);
        self
    }

    /// 初めに選ぶ項目の番号。項目の数を超えていれば何もしない。
    pub fn selected(mut self, index: usize) -> Self {
        if index < self.options.len() {
            self.selected = Some(index);
        }
        self
    }

    /// 一度に見せる行の数。既定は 8。
    pub fn max_visible(mut self, max_visible: usize) -> Self {
        self.max_visible = max_visible.max(1);
        self
    }

    /// 選んだ項目が変わったときに呼ぶ関数。項目の番号と字を渡す。
    pub fn on_change(mut self, on_change: impl FnMut(usize, &str) + 'static) -> Self {
        self.on_change = Some(Box::new(on_change));
        self
    }

    /// 選んでいる項目の番号。項目が無ければ `None`。
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// 選んでいる項目の字。
    pub fn selected_option(&self) -> Option<&str> {
        self.selected.and_then(|index| self.options.get(index)).map(String::as_str)
    }

    /// 一覧が開いているか。
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 項目を選ぶ。知らせは呼ばない（HTML と同じ）。変わったら `true`。
    /// 字は次の窓の [`update`](crate::gui::window_frame::WindowFrame::update) で書き直す。
    pub fn set_selected(&mut self, draw_manager: &mut DrawManager, index: usize) -> bool {
        if index >= self.options.len() || self.selected == Some(index) {
            return false;
        }

        self.selected = Some(index);
        self.button_dirty = true;
        self.list_dirty = true;
        self.place(draw_manager);
        true
    }

    // --- 選ぶ ---

    /// `index` を選び、変わったら知らせる。
    fn choose(&mut self, draw_manager: &mut DrawManager, index: usize) -> bool {
        if !self.set_selected(draw_manager, index) {
            return false;
        }

        if let Some(on_change) = self.on_change.as_mut() {
            on_change(index, &self.options[index]);
        }
        true
    }

    fn open(&mut self, draw_manager: &mut DrawManager) {
        if self.options.is_empty() {
            return;
        }

        self.open = true;
        self.highlighted = self.selected.or(Some(0));
        self.scroll_into_view();
        self.list_dirty = true;
        self.place(draw_manager);
    }

    fn close(&mut self, draw_manager: &mut DrawManager) -> bool {
        if !std::mem::take(&mut self.open) {
            return false;
        }

        self.highlighted = None;
        self.clear_text(self.list_text.as_deref(), draw_manager);
        self.place(draw_manager);
        true
    }

    /// 開いているあいだに光らせる行を `index` にし、見えるように送る。
    fn highlight(&mut self, draw_manager: &mut DrawManager, index: usize) -> bool {
        let index = index.min(self.options.len().saturating_sub(1));
        if self.highlighted == Some(index) {
            return false;
        }

        self.highlighted = Some(index);
        self.scroll_into_view();
        self.list_dirty = true;
        self.place(draw_manager);
        true
    }

    /// 光らせている行が見えるように、送る量を決める。
    fn scroll_into_view(&mut self) {
        let Some(index) = self.highlighted else {
            return;
        };
        let visible = self.visible_rows();

        if index < self.scroll {
            self.scroll = index;
        } else if index >= self.scroll + visible {
            self.scroll = index + 1 - visible;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn visible_rows(&self) -> usize {
        self.options.len().min(self.max_visible)
    }

    fn max_scroll(&self) -> usize {
        self.options.len().saturating_sub(self.visible_rows())
    }

    /// `from` の次から、`character` で始まる項目を探す（大文字と小文字は問わない）。
    fn find_by_initial(&self, character: char, from: Option<usize>) -> Option<usize> {
        let count = self.options.len();
        let start = from.map_or(0, |from| from + 1);
        let lower: String = character.to_lowercase().collect();

        (0..count)
            .map(|offset| (start + offset) % count)
            .find(|index| self.options[*index].to_lowercase().starts_with(&lower))
    }

    // --- 形 ---

    fn button_bounds(&self) -> [f32; 4] {
        let [x, y] = self.built_at;
        let (left, top) = (x + self.gap.x, y + self.gap.y);

        [left, top, left + self.width, top + self.height]
    }

    fn row_height(&self) -> f32 {
        self.height
    }

    fn list_bounds(&self) -> [f32; 4] {
        let [left, _, right, bottom] = self.button_bounds();
        let top = bottom + LIST_GAP;

        [left, top, right, top + LIST_PADDING * 2.0 + self.row_height() * self.visible_rows() as f32]
    }

    /// 項目が一覧に収まらず、スクロールバーを出すか。
    fn scrolls(&self) -> bool {
        self.options.len() > self.max_visible
    }

    /// 行の字と光らせる帯を置く右端。スクロールバーがあればその手前。
    fn rows_right(&self) -> f32 {
        let [_, _, right, _] = self.list_bounds();
        if self.scrolls() { right - SCROLLBAR_WIDTH - SCROLLBAR_INSET * 2.0 } else { right - HIGHLIGHT_INSET }
    }

    /// 一覧の中の `slot` 行めの上端。
    fn row_top(&self, slot: usize) -> f32 {
        self.list_bounds()[1] + LIST_PADDING + self.row_height() * slot as f32
    }

    /// ボタンの字を置く場所（▼ の手前まで）。
    fn button_text_bounds(&self) -> [f32; 4] {
        let [left, top, right, bottom] = self.button_bounds();
        [left + TEXT_PADDING, top, (right - self.height).max(left + TEXT_PADDING), bottom]
    }

    fn vertex(x: f32, y: f32) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0)
    }

    fn build_box(name: &str, [left, top, right, bottom]: [f32; 4], quad_type: QuadType, paint_type: PaintType) -> Object {
        match quad_type {
            QuadType::Rounded { radius } if radius > 0.0 => RectRounded::new(name)
                .paint_type(paint_type)
                .from(CornerType::Circle { radius: radius.min((right - left) / 2.0).min((bottom - top) / 2.0) }, Self::vertex(left, top))
                .to(Self::vertex(right, bottom))
                .end(),
            _ => Rect::new(name).paint_type(paint_type).from(Self::vertex(left, top)).to(Self::vertex(right, bottom)).end(),
        }
    }

    /// 図形 `part` の形（白で組む。色は複製で掛ける）。
    fn build(&self, part: usize, name: &str) -> Object {
        let button = self.button_bounds();
        let list = self.list_bounds();

        match part {
            BUTTON => Self::build_box(name, button, self.quad_type, PaintType::Fill),
            BUTTON_OUTLINE => Self::build_box(name, button, self.quad_type, PaintType::stroke(OUTLINE_WIDTH)),
            ARROW => {
                let [_, top, right, bottom] = button;
                let (center_x, center_y) = (right - self.height / 2.0, (top + bottom) / 2.0);
                let half = ARROW_WIDTH / 2.0;

                Triangle::new(name)
                    .paint_type(PaintType::Fill)
                    .vertex1(Self::vertex(center_x - half, center_y - half / 2.0))
                    .vertex2(Self::vertex(center_x + half, center_y - half / 2.0))
                    .vertex3(Self::vertex(center_x, center_y + half / 2.0))
                    .end()
            }
            LIST => Self::build_box(name, list, self.quad_type, PaintType::Fill),
            LIST_OUTLINE => Self::build_box(name, list, self.quad_type, PaintType::stroke(OUTLINE_WIDTH)),
            // 光らせる行。上端を 0 に組み、複製の位置で行へ置く。
            HIGHLIGHT => Self::build_box(name, [list[0] + HIGHLIGHT_INSET, 0.0, self.rows_right(), self.row_height()], QuadType::Rounded { radius: 3.0 }, PaintType::Fill),
            // スクロールバーのつまみ。高さ 1 に組み、複製の縦の倍率で伸ばす。
            _ => {
                let right = list[2] - SCROLLBAR_INSET;
                Rect::new(name)
                    .paint_type(PaintType::Fill)
                    .from(Self::vertex(right - SCROLLBAR_WIDTH, 0.0))
                    .to(Self::vertex(right, 1.0))
                    .end()
            }
        }
    }

    fn z(part: usize) -> f32 {
        if part <= ARROW { 0.0 } else { POPUP_Z }
    }

    /// 見た目を、複製の位置・倍率・色で映す。頂点には触らない。
    fn place(&self, draw_manager: &mut DrawManager) {
        let theme = &self.dropdown_theme;
        let hidden = [OFFSCREEN, OFFSCREEN];
        let shown = |at: [f32; 2]| if self.open { at } else { hidden };
        let outline = if self.focused || self.open { theme.focus_outline_color } else { theme.outline_color };

        let highlight_at = self
            .highlighted
            .filter(|index| (self.scroll..self.scroll + self.visible_rows()).contains(index))
            .map_or(hidden, |index| shown([0.0, self.row_top(index - self.scroll)]));

        // つまみは、見えている行の割合の長さで、送った量に合わせて下がる。
        let [_, list_top, _, list_bottom] = self.list_bounds();
        let track = (list_bottom - list_top - LIST_PADDING * 2.0).max(0.0);
        let count = self.options.len().max(1) as f32;
        let thumb_height = track * self.visible_rows() as f32 / count;
        let thumb_top = list_top + LIST_PADDING + track * self.scroll as f32 / count;
        let scrollbar_at = if self.scrolls() { shown([0.0, thumb_top]) } else { hidden };

        for (part, name) in self.parts.iter().enumerate() {
            let Some(object) = name.as_deref().and_then(|name| draw_manager.object_mut(name)) else {
                continue;
            };

            let (at, scale_y, color) = match part {
                BUTTON => ([0.0, 0.0], 1.0, theme.fill_color),
                BUTTON_OUTLINE => ([0.0, 0.0], 1.0, outline),
                ARROW => ([0.0, 0.0], 1.0, theme.arrow_color),
                LIST => (shown([0.0, 0.0]), 1.0, theme.list_fill_color),
                LIST_OUTLINE => (shown([0.0, 0.0]), 1.0, theme.list_outline_color),
                HIGHLIGHT => (highlight_at, 1.0, theme.highlight_fill_color),
                SCROLLBAR => (scrollbar_at, thumb_height, theme.scrollbar_color),
                _ => continue,
            };

            let instance = create_instance()
                .translate(at[0], at[1], 0.0)
                .scale(1.0, scale_y, 1.0)
                .color(color.r, color.g, color.b, color.a);
            // 同じなら触らない。引くと送り直しの印が付く。
            if object.instances().first() != Some(&instance) {
                set_first_instance(object, instance);
            }
        }
    }

    /// 字のはみ出しを削る。削る範囲はワールドなので、窓のずれを足す。字の形すべてに積む。
    fn apply_clips(&mut self, draw_manager: &mut DrawManager) {
        let [x, y] = self.offset;
        let clip = |[left, top, right, bottom]: [f32; 4]| Block::ClipRect {
            min: [left + x, top + y],
            max: [right + x, bottom + y],
            radius: 0.0,
            softness: 0.0,
            invert: false,
        };
        let [list_left, list_top, _, list_bottom] = self.list_bounds();
        let blocks = [
            (self.button_text.clone(), clip(self.button_text_bounds())),
            (self.list_text.clone(), clip([list_left + TEXT_PADDING / 2.0, list_top, self.rows_right() - TEXT_PADDING / 2.0, list_bottom])),
        ];

        for (index, (text, block)) in blocks.into_iter().enumerate() {
            let old = self.applied_clips[index];
            let Some(text) = text.filter(|_| old != Some(block)) else {
                continue;
            };

            let names: Vec<String> = draw_manager.group_names(&text).map(String::from).collect();
            for name in names {
                if let Some(object) = draw_manager.object_mut(&name) {
                    object.replace_effect(|effect| Some(*effect) == old, block);
                }
            }
            self.applied_clips[index] = Some(block);
        }
    }

    // --- 字 ---

    /// `renderer` に、`texts`（字・置き場所・色）を書く。書体が無ければ `false`。
    fn write(&self, draw_manager: &mut DrawManager, resources: &Resources, renderer: Option<&str>, texts: Vec<(String, [f32; 4], ThemeColor)>) -> bool {
        let (Some(name), Some(font)) = (renderer, resources.font(self.dropdown_font.base_font)) else {
            return false;
        };
        let size = self.dropdown_font.base_font_size;

        draw_manager.with_text_renderer(name, |text_renderer, draw_manager| {
            text_renderer.clear(draw_manager);

            // 登録は足していくので、行ごとに並べて書ける。
            for (text, [left, top, right, bottom], color) in texts {
                text_renderer.color(color.r, color.g, color.b, color.a);
                text_renderer.text_layout_data(TextLayoutData {
                    text,
                    style: TextStyle::new(size),
                    // 幅は字のぶん広く取って、寄せも折り返しもさせない。はみ出しは覆いで削る。
                    text_location: TextLocation::Area {
                        area: TextArea::new(left, top, (right - left).max(0.0) + size * 64.0, bottom - top).vertical(TextAlign::Center),
                    },
                });

                if let Err(error) = text_renderer.register_draw_manager(draw_manager, &font) {
                    log::warn!("failed to lay out the dropdown text: {error}");
                }
            }
        });
        true
    }

    fn write_button(&self, draw_manager: &mut DrawManager, resources: &Resources) -> bool {
        let text = self.selected_option().unwrap_or_default().to_string();
        self.write(draw_manager, resources, self.button_text.as_deref(), vec![(text, self.button_text_bounds(), self.dropdown_theme.text_color)])
    }

    /// 見えている行だけ書く。
    fn write_list(&self, draw_manager: &mut DrawManager, resources: &Resources) -> bool {
        let theme = &self.dropdown_theme;
        let [list_left, _, _, _] = self.list_bounds();
        let right = self.rows_right();
        let texts = (self.scroll..self.scroll + self.visible_rows())
            .enumerate()
            .map(|(slot, index)| {
                let top = self.row_top(slot);
                let color = if self.highlighted == Some(index) { theme.highlight_text_color } else { theme.text_color };
                (self.options[index].clone(), [list_left + TEXT_PADDING, top, right, top + self.row_height()], color)
            })
            .collect();

        self.write(draw_manager, resources, self.list_text.as_deref(), texts)
    }

    /// 字を消す（字の形は残り、複製だけが消える）。
    fn clear_text(&self, renderer: Option<&str>, draw_manager: &mut DrawManager) {
        if let Some(name) = renderer {
            draw_manager.with_text_renderer(name, |text_renderer, draw_manager| {
                text_renderer.clear(draw_manager);
            });
        }
    }

    // --- 指 ---

    /// 画素の位置を、組んだときの座標へ。動かないボタンを通す。
    fn local(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<[f32; 2]> {
        built_point(draw_manager, self.parts[BUTTON].as_deref(), x, y)
    }

    /// 枠（ボタンと、開いていれば一覧）の中か。
    fn inside(&self, local: [f32; 2]) -> bool {
        within(self.button_bounds(), 0.0, local) || (self.open && within(self.list_bounds(), 0.0, local))
    }

    /// 指の下で一番手前にあるのが、この部品の図形か。枠の外なら当てずに断る。
    fn hit(&self, draw_manager: &DrawManager, pointer: &Pointer) -> bool {
        if !self.local(draw_manager, pointer.x, pointer.y).is_some_and(|local| self.inside(local)) {
            return false;
        }
        let Some(pick) = pointer.pick(draw_manager) else {
            return false;
        };

        if self.parts.iter().any(|name| name.as_deref() == Some(pick.name.as_str())) {
            return true;
        }

        [&self.button_text, &self.list_text]
            .into_iter()
            .flatten()
            .any(|text| draw_manager.group_names(text).any(|name| name == pick.name))
    }

    /// 組んだときの座標の点にある項目。開いていて、一覧の行の上のときだけ。
    fn option_at(&self, [x, y]: [f32; 2]) -> Option<usize> {
        let [left, _, right, _] = self.list_bounds();
        if !self.open || x < left || x > right {
            return None;
        }

        let offset = y - self.row_top(0);
        let slot = (offset / self.row_height()).floor();
        (offset >= 0.0 && slot < self.visible_rows() as f32).then(|| self.scroll + slot as usize)
    }
}

impl WindowItem for Dropdown {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let counter = window_item_arguments.counter;
        let name = format!("{register_name}-{}-{counter}", self.name);

        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];
        self.scale_mode = window_item_arguments.scale_mode;

        let surface_size = window_item_arguments.surface_size;
        let design = window_item_arguments.design;
        let scale_mode = window_item_arguments.scale_mode;
        let resources = window_item_arguments.resources;
        let draw_manager = window_item_arguments.draw_manager;

        for part in 0..PART_COUNT {
            let mut object = self.build(part, &format!("{name}{}", PART_SUFFIXES[part]));
            object.camera_for(surface_size, design, scale_mode);
            object.instance(create_instance());
            object.z(Self::z(part));
            // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
            self.parts[part] = Some(draw_manager.register(object));
        }

        for (text_name, z) in [(format!("{name}-text"), 0.0), (format!("{name}-list-text"), POPUP_Z)] {
            let mut text_renderer = TextRenderer::new(&text_name);
            text_renderer.camera_for(surface_size, design, scale_mode);
            text_renderer.z(z);
            if let Some(replaced) = draw_manager.add_text_renderer(text_renderer) {
                log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
            }
        }
        self.button_text = Some(format!("{name}-text"));
        self.list_text = Some(format!("{name}-list-text"));

        // 項目の字の形を、いま両方に作っておく。開いたり選んだりしてから字の形が増えないように。
        let all: String = self.options.concat();
        let warm = vec![(all, self.button_text_bounds(), self.dropdown_theme.text_color)];
        let written = self.write(draw_manager, resources, self.button_text.as_deref(), warm.clone())
            && self.write(draw_manager, resources, self.list_text.as_deref(), warm);
        if written {
            self.clear_text(self.list_text.as_deref(), draw_manager);
            self.write_button(draw_manager, resources);
            self.button_dirty = false;
        } else {
            log::warn!("the dropdown font is not in the resources; the dropdown text is not drawn");
        }

        self.apply_clips(draw_manager);
        self.place(draw_manager);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];
        let draw_manager = window_item_arguments.draw_manager;

        // 窓の大きさが変わったときだけ。頂点を組み直す（全図形の積み直しになる）。
        for part in 0..PART_COUNT {
            let built = self.build(part, "");
            if let Some(object) = self.parts[part].as_deref().and_then(|name| draw_manager.object_mut(name)) {
                replace_outline(object, &built);
            }
        }

        self.button_dirty = true;
        self.list_dirty = true;
        self.apply_clips(draw_manager);
        self.place(draw_manager);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let mut owned: Vec<String> = self.parts.iter().flatten().cloned().collect();

        for text in [&self.button_text, &self.list_text].into_iter().flatten() {
            owned.extend(draw_manager.group_names(text).map(String::from));
        }

        owned
    }

    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        // 字の形は TextRenderer のカメラで登録される。窓は形のカメラしか貼り直さないので、ここで貼り直す。
        let scale_mode = self.scale_mode;
        for text in [&self.button_text, &self.list_text].into_iter().flatten() {
            draw_manager.with_text_renderer(text, |text_renderer, _| {
                text_renderer.camera_for(surface_size, design, scale_mode);
            });
        }
    }

    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        if !self.hit(draw_manager, &Pointer::new(x, y)) {
            // 一覧の外を押したら、選ばずに閉じる（押したものは後ろへ渡す）。
            self.close(draw_manager);
            return false;
        }
        let Some(local) = self.local(draw_manager, x, y) else {
            return false;
        };

        self.focused = true;

        if within(self.button_bounds(), 0.0, local) {
            if self.open {
                self.close(draw_manager);
            } else {
                self.open(draw_manager);
            }
            return true;
        }

        if let Some(index) = self.option_at(local) {
            self.choose(draw_manager, index);
            self.close(draw_manager);
        }
        self.place(draw_manager);
        true
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        if !self.open {
            return false;
        }
        let Some(local) = self.local(draw_manager, pointer.x, pointer.y) else {
            return false;
        };

        // 枠の中のときだけ当てる。行の上なら光らせる。
        match self.option_at(local).filter(|_| self.hit(draw_manager, pointer)) {
            Some(index) => self.highlight(draw_manager, index),
            None => false,
        }
    }

    fn mouse_wheel(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32, lines: [f32; 2]) -> bool {
        let over_list = self.open && self.local(draw_manager, x, y).is_some_and(|local| within(self.list_bounds(), 0.0, local));
        if !over_list || lines[1] == 0.0 || !self.scrolls() {
            return over_list;
        }

        // 上へ回すと前の項目が見える。
        let rows = lines[1].abs().ceil().max(1.0) as usize;
        let scroll = if lines[1] > 0.0 { self.scroll.saturating_sub(rows) } else { (self.scroll + rows).min(self.max_scroll()) };
        if scroll != self.scroll {
            self.scroll = scroll;
            self.list_dirty = true;
            self.place(draw_manager);
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
        self.close(draw_manager);
        self.place(draw_manager);
        true
    }

    fn wants_ime(&self) -> bool {
        false
    }

    fn text_input(&mut self, draw_manager: &mut DrawManager, _resources: &Resources, text: &str) -> bool {
        let Some(character) = text.chars().next() else {
            return false;
        };

        // Space は開く（開いていれば何もしない）。
        if character == ' ' {
            if !self.open {
                self.open(draw_manager);
                return true;
            }
            return false;
        }

        let from = if self.open { self.highlighted } else { self.selected };
        let Some(index) = self.find_by_initial(character, from) else {
            return false;
        };

        if self.open { self.highlight(draw_manager, index) } else { self.choose(draw_manager, index) }
    }

    fn key_pressed(&mut self, draw_manager: &mut DrawManager, _resources: &Resources, key: EditKey) -> bool {
        let last = self.options.len().saturating_sub(1);

        if !self.open {
            let selected = self.selected.unwrap_or(0);
            return match key {
                EditKey::Up | EditKey::Left => self.choose(draw_manager, selected.saturating_sub(1)),
                EditKey::Down | EditKey::Right => self.choose(draw_manager, (selected + 1).min(last)),
                EditKey::Home => self.choose(draw_manager, 0),
                EditKey::End => self.choose(draw_manager, last),
                EditKey::Enter => {
                    self.open(draw_manager);
                    self.open
                }
                EditKey::Escape => self.unfocus(draw_manager),
                _ => false,
            };
        }

        let highlighted = self.highlighted.unwrap_or(0);
        let page = self.visible_rows().saturating_sub(1).max(1);
        match key {
            EditKey::Up => self.highlight(draw_manager, highlighted.saturating_sub(1)),
            EditKey::Down => self.highlight(draw_manager, highlighted + 1),
            EditKey::PageUp => self.highlight(draw_manager, highlighted.saturating_sub(page)),
            EditKey::PageDown => self.highlight(draw_manager, highlighted + page),
            EditKey::Home => self.highlight(draw_manager, 0),
            EditKey::End => self.highlight(draw_manager, last),
            EditKey::Enter => {
                self.choose(draw_manager, highlighted);
                self.close(draw_manager)
            }
            EditKey::Escape | EditKey::Tab => self.close(draw_manager),
            _ => false,
        }
    }

    fn offset_changed(&mut self, draw_manager: &mut DrawManager, offset: [f32; 2]) {
        self.offset = offset;
        self.apply_clips(draw_manager);
    }

    fn animate(&mut self, draw_manager: &mut DrawManager, resources: &Resources, _now: Instant) -> Animation {
        let mut animation = Animation::default();

        // 字は書体が要るので、ここで書く。字の形は前もって作ってあるので、増えるのは複製だけ。
        if self.button_dirty {
            self.button_dirty = false;
            animation.relaid_out |= self.write_button(draw_manager, resources);
        }
        if self.list_dirty && self.open {
            self.list_dirty = false;
            animation.relaid_out |= self.write_list(draw_manager, resources);
        }

        animation
    }

    fn cursor_shape(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<CursorShape> {
        self.hit(draw_manager, pointer).then_some(CursorShape::Default)
    }
}

/// 図形の 1 つめの複製を差し替える（組んだときに 1 つ足してある）。
fn set_first_instance(object: &mut Object, instance: Instance) {
    match object.instance_mut(0) {
        Some(first) => *first = instance,
        None => {
            object.instance(instance);
        }
    }
}
