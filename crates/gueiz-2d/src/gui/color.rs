use std::f32::consts::FRAC_PI_2;
use std::time::Instant;
use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::effect::{srgb_to_linear, Block, Gradient};
use crate::gui::window_frame::{Gap, ThemeColor};
use crate::gui::window_item::{built_point, replace_outline, within, Animation, CursorShape, EditKey, KeyModifiers, Pointer, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::{create_instance, Instance};
use crate::object::Object;
use crate::objects::circle::Circle;
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::PaintType;
use crate::resource::{FontHandle, Resources};
use crate::text::{measure, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

/// パレットを、窓のほかの部品より手前に出す重なり順。右クリックのメニュー（1000）よりは奥。
const POPUP_Z: f32 = 500.0;
/// 閉じているあいだ、パレットを逃がしておく場所。
const OFFSCREEN: f32 = -1.0e6;
/// 色見本の縁取りの太さと、縁取りから色までの隙間。
const SWATCH_OUTLINE_WIDTH: f32 = 1.0;
const SWATCH_INSET: f32 = 3.0;
const SWATCH_RADIUS: f32 = 4.0;
/// 色見本とパレットのあいだ。
const POPUP_GAP: f32 = 4.0;
/// パレットの既定の幅。
const DEFAULT_POPUP_WIDTH: f32 = 220.0;
/// パレットの内側の余白と、中の並びの間。
const POPUP_PADDING: f32 = 10.0;
const POPUP_SPACING: f32 = 10.0;
const POPUP_RADIUS: f32 = 6.0;
/// 彩度×明度の面の、幅に対する高さ。
const SV_ASPECT: f32 = 0.65;
const SV_RADIUS: f32 = 3.0;
/// 色相の帯と透明度の帯の高さと、2 本のあいだ。
const BAR_HEIGHT: f32 = 12.0;
const BAR_SPACING: f32 = 8.0;
/// 帯は、上下にこれだけ外れて押しても当たる。
const BAR_REACH: f32 = 4.0;
/// R・G・B・A の欄。上に名前、下に数を打つ箱。
const FIELD_COUNT: usize = 4;
const FIELD_LABELS: [&str; FIELD_COUNT] = ["R", "G", "B", "A"];
const FIELD_LABEL_HEIGHT: f32 = 16.0;
const FIELD_HEIGHT: f32 = 24.0;
const FIELD_SPACING: f32 = 8.0;
const FIELD_RADIUS: f32 = 3.0;
/// 欄の名前の字の大きさ（数の字に対する割合）。
const LABEL_FONT_RATIO: f32 = 0.85;
/// 欄に打てる桁の数。
const FIELD_DIGITS: usize = 3;
/// 欄の、字を打つ位置の縦棒。
const CARET_WIDTH: f32 = 1.5;
const CARET_INSET: f32 = 5.0;
/// 全部選んでいる欄の地。フォーカスの縁取りの色を、この濃さで敷く。
const SELECTION_ALPHA: f32 = 0.4;
/// 面の上の印（丸）と、帯の上の印（縦長の角丸）。
const SV_MARKER_RADIUS: f32 = 6.0;
const BAR_MARKER_WIDTH: f32 = 6.0;
const MARKER_WIDTH: f32 = 2.0;
const CIRCLE_SEGMENTS: usize = 32;
/// 透けているのが分かるよう、透明度の帯と色見本の後ろに敷く市松模様。
const CHECKER_CELL: f32 = 6.0;
const CHECKER_LIGHT: ThemeColor = ThemeColor { r: 0.85, g: 0.85, b: 0.85, a: 1.0 };
const CHECKER_DARK: ThemeColor = ThemeColor { r: 0.6, g: 0.6, b: 0.6, a: 1.0 };
/// キーで動かす量。彩度と明度は 1%、色相は 10 度。
const KEY_STEP: f32 = 0.01;
const HUE_KEY_STEP: f32 = 10.0 / 360.0;
/// 字の形を前もって作っておく字。欄の名前と数に出てくる字はこれで全部。
const LABEL_GLYPHS: &str = "RGBA";
const VALUE_GLYPHS: &str = "0123456789";
/// 形は白で組み、色は複製の色で掛ける。
const WHITE: ThemeColor = ThemeColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

// 図形の番号。登録する順（＝同じ重なり順の中での前後）でもある。
const SWATCH_OUTLINE: usize = 0;
const SWATCH_CHECKER_BASE: usize = 1;
const SWATCH_CHECKER: usize = 2;
const SWATCH_FILL: usize = 3;
const PANEL: usize = 4;
const PANEL_OUTLINE: usize = 5;
const SV_BASE: usize = 6;
const SV_WHITE: usize = 7;
const SV_BLACK: usize = 8;
const HUE_BAR: usize = 9;
const ALPHA_CHECKER_BASE: usize = 10;
const ALPHA_CHECKER: usize = 11;
const ALPHA_BAR: usize = 12;
/// R・G・B・A の欄の地（4 つ）と縁取り（4 つ）。
const FIELD_FILL: usize = 13;
const FIELD_OUTLINE: usize = FIELD_FILL + FIELD_COUNT;
const CARET: usize = FIELD_OUTLINE + FIELD_COUNT;
const SV_MARKER_SHADOW: usize = CARET + 1;
const SV_MARKER: usize = CARET + 2;
const HUE_MARKER_SHADOW: usize = CARET + 3;
const HUE_MARKER: usize = CARET + 4;
const ALPHA_MARKER_SHADOW: usize = CARET + 5;
const ALPHA_MARKER: usize = CARET + 6;
const PART_COUNT: usize = CARET + 7;
/// 名前の後ろに付ける字。番号の順。
const PART_SUFFIXES: [&str; PART_COUNT] = [
    "",
    "-swatch-checker-base",
    "-swatch-checker",
    "-swatch",
    "-panel",
    "-panel-outline",
    "-sv",
    "-sv-white",
    "-sv-black",
    "-hue",
    "-alpha-checker-base",
    "-alpha-checker",
    "-alpha",
    "-field-r",
    "-field-g",
    "-field-b",
    "-field-a",
    "-field-r-outline",
    "-field-g-outline",
    "-field-b-outline",
    "-field-a-outline",
    "-caret",
    "-sv-marker-shadow",
    "-sv-marker",
    "-hue-marker-shadow",
    "-hue-marker",
    "-alpha-marker-shadow",
    "-alpha-marker",
];

/// 8 ビットの RGBA の色。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 透明度。0 で透明、255 で不透明。
    pub a: u8,
}

impl Default for Rgba {
    /// 不透明な黒（HTML の `<input type="color">` の既定と同じ色）。
    fn default() -> Self {
        Self::new(0, 0, 0, 255)
    }
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `#rrggbb`（不透明）か `#rrggbbaa` を読む。大文字と小文字は問わない。形が違えば `None`。
    pub fn from_hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }

        let channel = |at: usize| digits.get(at..at + 2).map_or(Some(255), |pair| u8::from_str_radix(pair, 16).ok());
        Some(Self { r: channel(0)?, g: channel(2)?, b: channel(4)?, a: channel(6)? })
    }

    /// `#rrggbb`（不透明のとき）か `#rrggbbaa`（小文字）。
    pub fn to_hex(self) -> String {
        let rgb = format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b);
        if self.a == 255 { rgb } else { format!("{rgb}{:02x}", self.a) }
    }

    /// 0 から 1 の色にする。
    pub fn to_theme_color(self) -> ThemeColor {
        let [r, g, b, a] = [self.r, self.g, self.b, self.a].map(|channel| channel as f32 / 255.0);
        ThemeColor { r, g, b, a }
    }

    fn channel(self, field: usize) -> u8 {
        [self.r, self.g, self.b, self.a][field]
    }
}

/// 色相・彩度・明度（どれも 0 から 1）。灰色や黒でも色相を覚えておけるよう、中ではこれで持つ。
#[derive(Debug, Copy, Clone, PartialEq)]
struct Hsv {
    h: f32,
    s: f32,
    v: f32,
}

/// 色を選ぶ欄。Web の `<input type="color">` と同じ仕組みで、選ぶ画面は独自のもの。透明度も選べます。
///
/// 色見本のボタンを押すと、下にパレットが開きます。パレットには
///
/// - 彩度（横）×明度（縦）の面
/// - 色相の帯と、透明度（A）の帯
/// - R・G・B・A を 0 から 255 で打ち込む欄
///
/// があり、面と帯は押して引けます。透けているのが分かるよう、透明度の帯と色見本の後ろには
/// 市松模様を敷きます。
///
/// # キー
///
/// - 開いているあいだ：←→ で彩度、↑↓ で明度、PageUp・PageDown で色相。Tab で R の欄へ。
/// - 欄を押すか Tab で入ると、数が全部選ばれた状態になり、打つと置き換わります（打った値はすぐ色に出る）。
///   ↑↓ で 1 ずつ、Tab・Shift+Tab で次・前の欄、Enter で決めて欄を出ます。255 を超える数は 255 にします。
/// - Enter（欄の外）かパレットの外を押すと決めて閉じ、Esc は開く前の色に戻して閉じます。
///   閉じていてフォーカスがあるときは Enter で開きます。
///
/// 色が変わるたびに [`Color::on_input`] を、閉じたときに開く前と色が違えば
/// [`Color::on_change`] を呼びます（HTML の `input` と `change` の出来事）。
///
/// # 重なりと覆い
///
/// パレットは窓のほかの部品より手前に開きます（重なり順 500）。窓の外へは出られず、
/// はみ出したぶんは窓の覆いで削られるので、窓の高さに余裕を持たせてください。
///
/// # 動かし方
///
/// 頂点を書き換えると全図形の積み直しになるので、頂点は組むときだけ作ります。
/// 印の位置・色見本の色・面の色相・透明度の帯の色は複製（[`Instance`]）の位置と色で映し、
/// パレットの開け閉めも複製の位置（閉じたら画面の外）で済ませます。
/// 欄の字は出てくる字の形をすべて前もって作っておくので、色を変えても字の形は増えません。
///
/// 面の明るさの段は、GPU が線形の色で混ぜても sRGB で計算した色とそろうよう、
/// 段を細かく置いたグラデーションで近づけています。
pub struct Color {
    /// 登録の名前のもと。
    name: String,
    width: f32,
    height: f32,
    gap: Gap,
    popup_width: f32,
    color_font: ColorFont,
    color_theme: ColorTheme,
    hsv: Hsv,
    alpha: u8,
    on_input: Option<Box<dyn FnMut(Rgba)>>,
    on_change: Option<Box<dyn FnMut(Rgba)>>,
    /// 図形の名前。番号は `SWATCH_OUTLINE` などの定数。
    parts: Vec<Option<String>>,
    text: Option<String>,
    /// 組んだときの左上（余白を足す前）。
    built_at: [f32; 2],
    /// 窓を組んだ場所からのずれ。市松模様を削る範囲（ワールド）に足す。
    offset: [f32; 2],
    /// 市松模様に最後に積んだ、角を丸く削る山（色見本・透明度の帯）。
    applied_clips: [Option<Block>; 2],
    scale_mode: ScaleMode,
    focused: bool,
    shift: bool,
    /// 開いたときの色。Esc で戻す・閉じるときに `change` を知らせるかを決める。
    opened_with: Option<(Hsv, u8)>,
    dragging: Option<Drag>,
    editing: Option<Editing>,
    /// 字を打つ位置の縦棒の左上。字の幅は書体で測るので、animate で決める。
    caret_at: Option<[f32; 2]>,
    /// 欄の字を書き直す要るか。字は書体が要るので animate で書く。
    text_dirty: bool,
}

/// 引いているもの。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Drag {
    SaturationValue,
    Hue,
    Alpha,
}

/// 打ち込んでいる欄。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Editing {
    /// 0 から 3（R・G・B・A）。
    field: usize,
    text: String,
    /// 全部選んでいる（次に打つ字で置き換える）。
    replace: bool,
}

/// パレットの中の並び。どれも組んだときの座標の `[左, 上, 右, 下]`。
struct Layout {
    panel: [f32; 4],
    sv: [f32; 4],
    hue: [f32; 4],
    alpha: [f32; 4],
    labels: [[f32; 4]; FIELD_COUNT],
    fields: [[f32; 4]; FIELD_COUNT],
}

pub struct ColorFont {
    pub base_font: FontHandle,
    pub base_font_size: f32,
}

pub struct ColorTheme {
    /// 色見本の縁取り。
    pub swatch_outline_color: ThemeColor,
    /// フォーカスがあるとき・開いているときの、色見本の縁取り。打ち込んでいる欄の縁取りにも使う。
    pub focus_outline_color: ThemeColor,
    /// パレットの地と縁取り。
    pub panel_fill_color: ThemeColor,
    pub panel_outline_color: ThemeColor,
    /// 面と帯の上の印。見やすいよう、外側に影の色で縁を付ける。
    pub marker_color: ThemeColor,
    pub marker_shadow_color: ThemeColor,
    /// R・G・B・A の欄の地と縁取り。
    pub field_fill_color: ThemeColor,
    pub field_outline_color: ThemeColor,
    /// 欄の上の名前（R・G・B・A）の字。
    pub label_color: ThemeColor,
    /// 欄の数の字と、字を打つ位置の縦棒。
    pub text_color: ThemeColor,
}

impl Color {
    pub fn new(register_name_string: String, width: f32, height: f32, gap: Gap, color_font: ColorFont, color_theme: ColorTheme) -> Self {
        Self {
            name: register_name_string,
            width,
            height,
            gap,
            popup_width: DEFAULT_POPUP_WIDTH,
            color_font,
            color_theme,
            hsv: Hsv { h: 0.0, s: 0.0, v: 0.0 },
            alpha: 255,
            on_input: None,
            on_change: None,
            parts: vec![None; PART_COUNT],
            text: None,
            built_at: [0.0; 2],
            offset: [0.0; 2],
            applied_clips: [None; 2],
            scale_mode: ScaleMode::default(),
            focused: false,
            shift: false,
            opened_with: None,
            dragging: None,
            editing: None,
            caret_at: None,
            text_dirty: true,
        }
    }

    /// 初めの色。既定は不透明な黒。
    pub fn value(mut self, value: Rgba) -> Self {
        self.hsv = rgb_to_hsv([value.r, value.g, value.b], self.hsv);
        self.alpha = value.a;
        self
    }

    /// パレットの幅。既定は 220。
    pub fn popup_width(mut self, popup_width: f32) -> Self {
        let fields = FIELD_COUNT as f32;
        self.popup_width = popup_width.max(POPUP_PADDING * 2.0 + fields * 24.0 + (fields - 1.0) * FIELD_SPACING);
        self
    }

    /// 色が変わるたびに呼ぶ関数（HTML の `input`）。
    pub fn on_input(mut self, on_input: impl FnMut(Rgba) + 'static) -> Self {
        self.on_input = Some(Box::new(on_input));
        self
    }

    /// 閉じたときに、開く前と色が違えば呼ぶ関数（HTML の `change`）。
    pub fn on_change(mut self, on_change: impl FnMut(Rgba) + 'static) -> Self {
        self.on_change = Some(Box::new(on_change));
        self
    }

    /// いまの色。
    pub fn get_value(&self) -> Rgba {
        let [r, g, b] = hsv_to_rgb(self.hsv);
        Rgba { r, g, b, a: self.alpha }
    }

    /// パレットが開いているか。
    pub fn is_open(&self) -> bool {
        self.opened_with.is_some()
    }

    /// 色を差し替える。知らせは呼ばない（HTML と同じ）。変わったら `true`。
    pub fn set_value(&mut self, draw_manager: &mut DrawManager, value: Rgba) -> bool {
        let before = self.get_value();
        self.hsv = rgb_to_hsv([value.r, value.g, value.b], self.hsv);
        self.alpha = value.a;
        self.text_dirty = true;
        self.place(draw_manager);

        before != value
    }

    // --- 値 ---

    /// 色を `hsv` と `alpha` にして映す。RGBA が変わったら `input` を知らせる。見た目が変わったら `true`。
    fn set_color(&mut self, draw_manager: &mut DrawManager, hsv: Hsv, alpha: u8) -> bool {
        let hsv = Hsv { h: hsv.h.clamp(0.0, 1.0), s: hsv.s.clamp(0.0, 1.0), v: hsv.v.clamp(0.0, 1.0) };
        if hsv == self.hsv && alpha == self.alpha {
            return false;
        }

        let before = self.get_value();
        self.hsv = hsv;
        self.alpha = alpha;
        self.place(draw_manager);

        let value = self.get_value();
        if value != before {
            self.text_dirty = true;
            if let Some(on_input) = self.on_input.as_mut() {
                on_input(value);
            }
        }
        true
    }

    fn set_hsv(&mut self, draw_manager: &mut DrawManager, hsv: Hsv) -> bool {
        self.set_color(draw_manager, hsv, self.alpha)
    }

    /// R・G・B・A の 1 つを `value` にする。
    fn set_channel(&mut self, draw_manager: &mut DrawManager, field: usize, value: u8) -> bool {
        let mut rgba = self.get_value();
        match field {
            0 => rgba.r = value,
            1 => rgba.g = value,
            2 => rgba.b = value,
            _ => rgba.a = value,
        }

        let hsv = rgb_to_hsv([rgba.r, rgba.g, rgba.b], self.hsv);
        self.set_color(draw_manager, hsv, rgba.a)
    }

    fn open(&mut self, draw_manager: &mut DrawManager) {
        self.opened_with = Some((self.hsv, self.alpha));
        self.text_dirty = true;
        self.place(draw_manager);
    }

    /// パレットを閉じる。`revert` なら開く前の色に戻す。開く前と色が違えば `change` を知らせる。
    fn close(&mut self, draw_manager: &mut DrawManager, revert: bool) -> bool {
        let Some((hsv, alpha)) = self.opened_with else {
            return false;
        };

        if revert {
            self.editing = None;
        } else {
            self.commit_field(draw_manager);
        }
        self.opened_with = None;
        self.dragging = None;
        if revert {
            self.hsv = hsv;
            self.alpha = alpha;
        }
        self.place(draw_manager);
        self.clear_text(draw_manager);

        let [r, g, b] = hsv_to_rgb(hsv);
        let value = self.get_value();
        if value != (Rgba { r, g, b, a: alpha }) {
            if let Some(on_change) = self.on_change.as_mut() {
                on_change(value);
            }
        }
        true
    }

    // --- 欄 ---

    /// `field` の欄に入る。数は全部選んだ状態にする。
    fn start_editing(&mut self, draw_manager: &mut DrawManager, field: usize) {
        self.commit_field(draw_manager);
        self.editing = Some(Editing { field, text: self.get_value().channel(field).to_string(), replace: true });
        self.text_dirty = true;
        self.place(draw_manager);
    }

    /// 打っている欄の数を決めて、欄を出る。空なら 0、255 を超えたら 255。
    fn commit_field(&mut self, draw_manager: &mut DrawManager) {
        let Some(editing) = self.editing.take() else {
            return;
        };

        self.set_channel(draw_manager, editing.field, parse_channel(&editing.text));
        self.text_dirty = true;
        self.place(draw_manager);
    }

    /// 打っている欄の字を書き換え、数として読めればすぐ色に出す。
    fn edit_field(&mut self, draw_manager: &mut DrawManager, edit: impl FnOnce(&mut Editing)) -> bool {
        let Some(editing) = self.editing.as_mut() else {
            return false;
        };

        edit(editing);
        let (field, text) = (editing.field, editing.text.clone());
        self.text_dirty = true;
        if !text.is_empty() {
            self.set_channel(draw_manager, field, parse_channel(&text));
        }
        self.place(draw_manager);
        true
    }

    /// 打っている欄の数を `delta` 動かす。
    fn step_field(&mut self, draw_manager: &mut DrawManager, delta: i32) -> bool {
        let Some(field) = self.editing.as_ref().map(|editing| editing.field) else {
            return false;
        };
        let value = (self.get_value().channel(field) as i32 + delta).clamp(0, 255) as u8;

        self.edit_field(draw_manager, |editing| {
            editing.text = value.to_string();
            editing.replace = false;
        })
    }

    // --- 形 ---

    /// 色見本の左上と右下（組んだときの座標）。
    fn swatch_bounds(&self) -> [f32; 4] {
        let [x, y] = self.built_at;
        let (left, top) = (x + self.gap.x, y + self.gap.y);

        [left, top, left + self.width, top + self.height]
    }

    /// 色見本の、縁取りの内側の色を塗るところ。
    fn swatch_fill_bounds(&self) -> [f32; 4] {
        let [left, top, right, bottom] = self.swatch_bounds();
        let inset = SWATCH_INSET.min((right - left) / 2.0).min((bottom - top) / 2.0);

        [left + inset, top + inset, right - inset, bottom - inset]
    }

    fn swatch_fill_radius(&self) -> f32 {
        (SWATCH_RADIUS - SWATCH_INSET * 0.5).max(0.0)
    }

    /// パレットの中の並び。色見本のすぐ下から。
    fn layout(&self) -> Layout {
        let [left, _, _, bottom] = self.swatch_bounds();
        let panel_top = bottom + POPUP_GAP;
        let inner_left = left + POPUP_PADDING;
        let inner_width = self.popup_width - POPUP_PADDING * 2.0;
        let inner_right = inner_left + inner_width;

        let sv_top = panel_top + POPUP_PADDING;
        let sv = [inner_left, sv_top, inner_right, sv_top + inner_width * SV_ASPECT];
        let hue_top = sv[3] + POPUP_SPACING;
        let hue = [inner_left, hue_top, inner_right, hue_top + BAR_HEIGHT];
        let alpha_top = hue[3] + BAR_SPACING;
        let alpha = [inner_left, alpha_top, inner_right, alpha_top + BAR_HEIGHT];

        let label_top = alpha[3] + POPUP_SPACING;
        let field_top = label_top + FIELD_LABEL_HEIGHT;
        let fields_count = FIELD_COUNT as f32;
        let field_width = (inner_width - FIELD_SPACING * (fields_count - 1.0)) / fields_count;
        let column = |index: usize| inner_left + (field_width + FIELD_SPACING) * index as f32;
        let labels = std::array::from_fn(|index| [column(index), label_top, column(index) + field_width, field_top]);
        let fields = std::array::from_fn(|index| [column(index), field_top, column(index) + field_width, field_top + FIELD_HEIGHT]);

        let panel = [left, panel_top, left + self.popup_width, field_top + FIELD_HEIGHT + POPUP_PADDING];

        Layout { panel, sv, hue, alpha, labels, fields }
    }

    fn vertex(x: f32, y: f32) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, WHITE.r, WHITE.g, WHITE.b, WHITE.a)
    }

    fn build_rounded(name: &str, [left, top, right, bottom]: [f32; 4], radius: f32, paint_type: PaintType) -> Object {
        let radius = radius.min((right - left) / 2.0).min((bottom - top) / 2.0);

        if radius > 0.0 {
            RectRounded::new(name)
                .paint_type(paint_type)
                .from(CornerType::Circle { radius }, Self::vertex(left, top))
                .to(Self::vertex(right, bottom))
                .end()
        } else {
            Rect::new(name).paint_type(paint_type).from(Self::vertex(left, top)).to(Self::vertex(right, bottom)).end()
        }
    }

    fn build_circle(name: &str, radius: f32, paint_type: PaintType) -> Object {
        Circle::new(name)
            .paint_type(paint_type)
            .vertex_center(Self::vertex(0.0, 0.0))
            .radius(radius)
            .segment_count(CIRCLE_SEGMENTS)
            .end_circle_option()
            .end()
    }

    /// 市松模様の暗いマス。1 つの図形に、マスを 1 つずつ輪郭として入れ、非ゼロ巻き数で塗る。
    fn build_checker(name: &str, [left, top, right, bottom]: [f32; 4]) -> Object {
        let mut object = crate::object::create_object(name);
        object.begin(PaintType::FillNonZero);

        let mut first = true;
        let (columns, rows) = (((right - left) / CHECKER_CELL).ceil() as usize, ((bottom - top) / CHECKER_CELL).ceil() as usize);
        for row in 0..rows {
            for column in (0..columns).filter(|column| (column + row) % 2 == 1) {
                let x0 = left + column as f32 * CHECKER_CELL;
                let y0 = top + row as f32 * CHECKER_CELL;
                let (x1, y1) = ((x0 + CHECKER_CELL).min(right), (y0 + CHECKER_CELL).min(bottom));

                if !first {
                    object.begin_hole();
                }
                first = false;
                for [x, y] in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
                    object.put_vertex(Self::vertex(x, y));
                }
            }
        }

        object.end();
        object
    }

    /// 帯の上の印。原点を中心にした縦長の角丸。
    fn build_bar_marker(name: &str, grow: f32, paint_type: PaintType) -> Object {
        let (half_width, half_height) = (BAR_MARKER_WIDTH / 2.0 + grow, BAR_HEIGHT / 2.0 + 2.0 + grow);
        Self::build_rounded(name, [-half_width, -half_height, half_width, half_height], half_width, paint_type)
    }

    /// 図形 `part` の形（頂点だけ。色は複製で、グラデーションは組むときに足す）。
    fn build(&self, part: usize, name: &str) -> Object {
        let layout = self.layout();

        match part {
            SWATCH_OUTLINE => Self::build_rounded(name, self.swatch_bounds(), SWATCH_RADIUS, PaintType::stroke(SWATCH_OUTLINE_WIDTH)),
            SWATCH_CHECKER_BASE | SWATCH_FILL => Self::build_rounded(name, self.swatch_fill_bounds(), self.swatch_fill_radius(), PaintType::Fill),
            SWATCH_CHECKER => Self::build_checker(name, self.swatch_fill_bounds()),
            PANEL => Self::build_rounded(name, layout.panel, POPUP_RADIUS, PaintType::Fill),
            PANEL_OUTLINE => Self::build_rounded(name, layout.panel, POPUP_RADIUS, PaintType::stroke(1.0)),
            SV_BASE | SV_WHITE | SV_BLACK => Self::build_rounded(name, layout.sv, SV_RADIUS, PaintType::Fill),
            HUE_BAR => Self::build_rounded(name, layout.hue, BAR_HEIGHT / 2.0, PaintType::Fill),
            ALPHA_CHECKER_BASE | ALPHA_BAR => Self::build_rounded(name, layout.alpha, BAR_HEIGHT / 2.0, PaintType::Fill),
            ALPHA_CHECKER => Self::build_checker(name, layout.alpha),
            part if (FIELD_FILL..FIELD_OUTLINE).contains(&part) => {
                Self::build_rounded(name, layout.fields[part - FIELD_FILL], FIELD_RADIUS, PaintType::Fill)
            }
            part if (FIELD_OUTLINE..CARET).contains(&part) => {
                Self::build_rounded(name, layout.fields[part - FIELD_OUTLINE], FIELD_RADIUS, PaintType::stroke(1.0))
            }
            CARET => Rect::new(name)
                .paint_type(PaintType::Fill)
                .from(Self::vertex(0.0, 0.0))
                .to(Self::vertex(CARET_WIDTH, FIELD_HEIGHT - CARET_INSET * 2.0))
                .end(),
            SV_MARKER_SHADOW => Self::build_circle(name, SV_MARKER_RADIUS + 1.0, PaintType::stroke(1.0)),
            SV_MARKER => Self::build_circle(name, SV_MARKER_RADIUS - 0.5, PaintType::stroke(MARKER_WIDTH)),
            HUE_MARKER_SHADOW | ALPHA_MARKER_SHADOW => Self::build_bar_marker(name, 1.0, PaintType::stroke(1.0)),
            HUE_MARKER | ALPHA_MARKER => Self::build_bar_marker(name, 0.0, PaintType::stroke(MARKER_WIDTH)),
            _ => unreachable!("no part {part}"),
        }
    }

    /// 組むときに足す、図形 `part` のグラデーション。
    fn gradient(part: usize) -> Option<Gradient> {
        match part {
            // 白の重ね。左（彩度 0）で白、右（彩度 1）で透明。透明度の混ぜは線形の色で行われるので、
            // sRGB で `(1 - s)` 白を混ぜたのとそろうよう、透明度を `srgb_to_linear(1 - s)` にした段を細かく置く。
            SV_WHITE => Some((0..8).fold(Gradient::linear(0.0), |gradient, index| {
                let position = index as f32 / 7.0;
                gradient.stop(position, [1.0, 1.0, 1.0, linear_alpha(1.0 - position)])
            })),
            // 黒の重ね。上（明度 1）で透明、下（明度 0）で黒。線形で `(1 - a)` 倍に暗くなるので、
            // sRGB で `v` 倍にしたのとそろうよう、透明度を `1 - srgb_to_linear(v)` にした段を細かく置く。
            SV_BLACK => Some((0..8).fold(Gradient::linear(FRAC_PI_2), |gradient, index| {
                let position = index as f32 / 7.0;
                gradient.stop(position, [0.0, 0.0, 0.0, 1.0 - linear_alpha(1.0 - position)])
            })),
            // 色相の帯。赤・黄・緑・水色・青・紫・赤。
            HUE_BAR => Some((0..7).fold(Gradient::linear(0.0), |gradient, index| {
                let position = index as f32 / 6.0;
                let [r, g, b] = hsv_to_rgb(Hsv { h: position, s: 1.0, v: 1.0 }).map(|channel| channel as f32 / 255.0);
                gradient.stop(position, [r, g, b, 1.0])
            })),
            // 透明度の帯。左で透明、右で不透明。色は複製の色で掛ける。
            ALPHA_BAR => Some(Gradient::linear(0.0).stop(0.0, [1.0, 1.0, 1.0, 0.0]).stop(1.0, [1.0, 1.0, 1.0, 1.0])),
            _ => None,
        }
    }

    fn z(part: usize) -> f32 {
        if part <= SWATCH_FILL { 0.0 } else { POPUP_Z }
    }

    /// 値と見た目を、複製の位置と色で映す。頂点には触らない。
    fn place(&self, draw_manager: &mut DrawManager) {
        let theme = &self.color_theme;
        let layout = self.layout();
        let open = self.is_open();
        let shown = |at: [f32; 2]| if open { at } else { [OFFSCREEN, OFFSCREEN] };
        let value = self.get_value().to_theme_color();
        let opaque = ThemeColor { a: 1.0, ..value };
        let hue = Rgba::from(hsv_to_rgb(Hsv { h: self.hsv.h, s: 1.0, v: 1.0 })).to_theme_color();
        let outline = if self.focused || open { theme.focus_outline_color } else { theme.swatch_outline_color };
        let selection = ThemeColor { a: theme.focus_outline_color.a * SELECTION_ALPHA, ..theme.focus_outline_color };

        let [sv_left, sv_top, sv_right, sv_bottom] = layout.sv;
        let sv_marker = [sv_left + (sv_right - sv_left) * self.hsv.s, sv_top + (sv_bottom - sv_top) * (1.0 - self.hsv.v)];
        let bar_marker = |[left, top, right, bottom]: [f32; 4], ratio: f32| [left + (right - left) * ratio, (top + bottom) / 2.0];
        let hue_marker = bar_marker(layout.hue, self.hsv.h);
        let alpha_marker = bar_marker(layout.alpha, self.alpha as f32 / 255.0);

        for (part, name) in self.parts.iter().enumerate() {
            let Some(object) = name.as_deref().and_then(|name| draw_manager.object_mut(name)) else {
                continue;
            };

            let (at, color) = match part {
                SWATCH_OUTLINE => ([0.0, 0.0], outline),
                SWATCH_CHECKER_BASE => ([0.0, 0.0], CHECKER_LIGHT),
                SWATCH_CHECKER => ([0.0, 0.0], CHECKER_DARK),
                SWATCH_FILL => ([0.0, 0.0], value),
                PANEL => (shown([0.0, 0.0]), theme.panel_fill_color),
                PANEL_OUTLINE => (shown([0.0, 0.0]), theme.panel_outline_color),
                SV_BASE => (shown([0.0, 0.0]), hue),
                SV_WHITE | SV_BLACK | HUE_BAR => (shown([0.0, 0.0]), WHITE),
                ALPHA_CHECKER_BASE => (shown([0.0, 0.0]), CHECKER_LIGHT),
                ALPHA_CHECKER => (shown([0.0, 0.0]), CHECKER_DARK),
                // 透明度の帯は、いまの色を透明から不透明へ。
                ALPHA_BAR => (shown([0.0, 0.0]), opaque),
                part if (FIELD_FILL..FIELD_OUTLINE).contains(&part) => {
                    let selected = self.editing.as_ref().is_some_and(|editing| editing.field == part - FIELD_FILL && editing.replace);
                    (shown([0.0, 0.0]), if selected { selection } else { theme.field_fill_color })
                }
                part if (FIELD_OUTLINE..CARET).contains(&part) => {
                    let editing = self.editing.as_ref().is_some_and(|editing| editing.field == part - FIELD_OUTLINE);
                    (shown([0.0, 0.0]), if editing { theme.focus_outline_color } else { theme.field_outline_color })
                }
                CARET => {
                    let typing = self.editing.as_ref().is_some_and(|editing| !editing.replace);
                    let at = self.caret_at.filter(|_| typing && open).unwrap_or([OFFSCREEN, OFFSCREEN]);
                    (at, theme.text_color)
                }
                SV_MARKER_SHADOW => (shown(sv_marker), theme.marker_shadow_color),
                SV_MARKER => (shown(sv_marker), theme.marker_color),
                HUE_MARKER_SHADOW => (shown(hue_marker), theme.marker_shadow_color),
                HUE_MARKER => (shown(hue_marker), theme.marker_color),
                ALPHA_MARKER_SHADOW => (shown(alpha_marker), theme.marker_shadow_color),
                ALPHA_MARKER => (shown(alpha_marker), theme.marker_color),
                _ => continue,
            };

            let instance = create_instance().translate(at[0], at[1], 0.0).color(color.r, color.g, color.b, color.a);
            // 同じなら触らない。引くと送り直しの印が付く。
            if object.instances().first() != Some(&instance) {
                set_first_instance(object, instance);
            }
        }
    }

    /// 市松模様の角を、上に重ねる色と同じ丸みで削る。削る範囲はワールドなので、窓のずれを足す。
    fn apply_clips(&mut self, draw_manager: &mut DrawManager) {
        let [x, y] = self.offset;
        let clip = |[left, top, right, bottom]: [f32; 4], radius: f32| Block::ClipRect {
            min: [left + x, top + y],
            max: [right + x, bottom + y],
            radius,
            softness: 0.0,
            invert: false,
        };
        let blocks = [
            (SWATCH_CHECKER, clip(self.swatch_fill_bounds(), self.swatch_fill_radius())),
            (ALPHA_CHECKER, clip(self.layout().alpha, BAR_HEIGHT / 2.0)),
        ];

        for (index, (part, block)) in blocks.into_iter().enumerate() {
            let old = self.applied_clips[index];
            if old == Some(block) {
                continue;
            }
            if let Some(object) = self.parts[part].as_deref().and_then(|name| draw_manager.object_mut(name)) {
                object.replace_effect(|effect| Some(*effect) == old, block);
            }
            self.applied_clips[index] = Some(block);
        }
    }

    // --- 字 ---

    /// 欄の名前と数を書く。打っている欄は打っている字を出し、字を打つ位置も決める。
    fn write_text(&mut self, draw_manager: &mut DrawManager, resources: &Resources) -> bool {
        let values: [String; FIELD_COUNT] = std::array::from_fn(|field| match &self.editing {
            Some(editing) if editing.field == field => editing.text.clone(),
            _ => self.get_value().channel(field).to_string(),
        });
        let labels = FIELD_LABELS.map(String::from);
        let written = self.write(draw_manager, resources, &labels, &values);

        // 字を打つ位置は、打っている数の右端（数は欄のまん中に寄せてある）。
        self.caret_at = match (&self.editing, resources.font(self.color_font.base_font)) {
            (Some(editing), Some(font)) => {
                let width = measure(&font, &editing.text, &TextStyle::new(self.color_font.base_font_size)).width;
                let [left, top, right, _] = self.layout().fields[editing.field];
                Some([(left + right) / 2.0 + width / 2.0 + 1.0, top + CARET_INSET])
            }
            _ => None,
        };
        self.place(draw_manager);

        written
    }

    /// `labels` と `values` を欄の上と中に書く。書体が無ければ `false`。
    fn write(&self, draw_manager: &mut DrawManager, resources: &Resources, labels: &[String], values: &[String]) -> bool {
        let (Some(name), Some(font)) = (self.text.as_deref(), resources.font(self.color_font.base_font)) else {
            return false;
        };
        let layout = self.layout();
        let theme = &self.color_theme;
        let size = self.color_font.base_font_size;
        let area = |[left, top, right, bottom]: [f32; 4]| TextLocation::Area { area: TextArea::new(left, top, right - left, bottom - top).centered() };

        let mut texts = Vec::new();
        for (index, label) in labels.iter().enumerate() {
            texts.push((label.clone(), size * LABEL_FONT_RATIO, area(layout.labels[index]), theme.label_color));
        }
        for (index, value) in values.iter().enumerate() {
            texts.push((value.clone(), size, area(layout.fields[index]), theme.text_color));
        }

        draw_manager.with_text_renderer(name, |text_renderer, draw_manager| {
            text_renderer.clear(draw_manager);

            // 登録は足していくので、欄ごとに並べて書ける。
            for (text, size, text_location, color) in texts {
                text_renderer.color(color.r, color.g, color.b, color.a);
                text_renderer.text_layout_data(TextLayoutData { text, style: TextStyle::new(size), text_location });

                if let Err(error) = text_renderer.register_draw_manager(draw_manager, &font) {
                    log::warn!("failed to lay out the color text: {error}");
                }
            }
        });
        true
    }

    /// 字を消す（字の形は残り、複製だけが消える）。
    fn clear_text(&self, draw_manager: &mut DrawManager) {
        if let Some(name) = self.text.as_deref() {
            draw_manager.with_text_renderer(name, |text_renderer, draw_manager| {
                text_renderer.clear(draw_manager);
            });
        }
    }

    // --- 指 ---

    /// 画素の位置を、組んだときの座標へ。動かない色見本の縁取りを通す。
    fn local(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<[f32; 2]> {
        built_point(draw_manager, self.parts[SWATCH_OUTLINE].as_deref(), x, y)
    }

    /// 枠（色見本と、開いていればパレット）の中か。印がはみ出すぶんも含める。
    fn inside(&self, local: [f32; 2]) -> bool {
        within(self.swatch_bounds(), 0.0, local) || (self.is_open() && within(self.layout().panel, SV_MARKER_RADIUS, local))
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

        self.text.as_deref().is_some_and(|text| draw_manager.group_names(text).any(|name| name == pick.name))
    }

    /// 組んだときの座標の点にある欄。
    fn field_at(&self, local: [f32; 2]) -> Option<usize> {
        let layout = self.layout();
        (0..FIELD_COUNT).find(|field| within(layout.fields[*field], 0.0, local))
    }

    /// 組んだときの座標の点から、引いているものの値を決める。
    fn drag_to(&mut self, draw_manager: &mut DrawManager, drag: Drag, [x, y]: [f32; 2]) -> bool {
        let layout = self.layout();

        match drag {
            Drag::SaturationValue => {
                let [left, top, right, bottom] = layout.sv;
                let hsv = Hsv { s: ratio(x, left, right), v: 1.0 - ratio(y, top, bottom), ..self.hsv };
                self.set_hsv(draw_manager, hsv)
            }
            Drag::Hue => {
                let [left, _, right, _] = layout.hue;
                let hsv = Hsv { h: ratio(x, left, right), ..self.hsv };
                self.set_hsv(draw_manager, hsv)
            }
            Drag::Alpha => {
                let [left, _, right, _] = layout.alpha;
                let alpha = (ratio(x, left, right) * 255.0).round() as u8;
                self.set_color(draw_manager, self.hsv, alpha)
            }
        }
    }
}

impl From<[u8; 3]> for Rgba {
    /// 不透明な色。
    fn from([r, g, b]: [u8; 3]) -> Self {
        Self::new(r, g, b, 255)
    }
}

impl WindowItem for Color {
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
            if let Some(gradient) = Self::gradient(part) {
                object.effect(Block::GradientStops { gradient });
            }
            object.camera_for(surface_size, design, scale_mode);
            object.instance(create_instance());
            object.z(Self::z(part));
            // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
            self.parts[part] = Some(draw_manager.register(object));
        }

        let text_name = format!("{name}-text");
        let mut text_renderer = TextRenderer::new(&text_name);
        text_renderer.camera_for(surface_size, design, scale_mode);
        text_renderer.z(POPUP_Z);
        if let Some(replaced) = draw_manager.add_text_renderer(text_renderer) {
            log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
        }
        self.text = Some(text_name);

        // 欄に出てくる字の形を、いま作っておく。開いてから字の形が増えないように。
        if self.write(draw_manager, resources, &[LABEL_GLYPHS.to_string()], &[VALUE_GLYPHS.to_string()]) {
            self.clear_text(draw_manager);
        } else {
            log::warn!("the color font is not in the resources; the color text is not drawn");
        }

        self.apply_clips(draw_manager);
        self.place(draw_manager);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];
        let draw_manager = window_item_arguments.draw_manager;

        // 窓の大きさが変わったときだけ。頂点を組み直す（全図形の積み直しになる）。グラデーションは残る。
        for part in 0..PART_COUNT {
            let built = self.build(part, "");
            if let Some(object) = self.parts[part].as_deref().and_then(|name| draw_manager.object_mut(name)) {
                replace_outline(object, &built);
            }
        }

        self.text_dirty = true;
        self.apply_clips(draw_manager);
        self.place(draw_manager);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let mut owned: Vec<String> = self.parts.iter().flatten().cloned().collect();

        if let Some(text) = &self.text {
            owned.extend(draw_manager.group_names(text).map(String::from));
        }

        owned
    }

    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        // 字の形は TextRenderer のカメラで登録される。窓は形のカメラしか貼り直さないので、ここで貼り直す。
        let scale_mode = self.scale_mode;
        if let Some(text) = self.text.as_deref() {
            draw_manager.with_text_renderer(text, |text_renderer, _| {
                text_renderer.camera_for(surface_size, design, scale_mode);
            });
        }
    }

    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let pointer = Pointer::new(x, y);

        if !self.hit(draw_manager, &pointer) {
            // パレットの外を押したら、決めて閉じる（押したものは後ろへ渡す）。
            self.close(draw_manager, false);
            return false;
        }
        let Some(local) = self.local(draw_manager, x, y) else {
            return false;
        };

        self.focused = true;

        if within(self.swatch_bounds(), 0.0, local) {
            if self.is_open() {
                self.close(draw_manager, false);
            } else {
                self.open(draw_manager);
            }
            return true;
        }

        if let Some(field) = self.field_at(local) {
            self.start_editing(draw_manager, field);
            return true;
        }

        // 欄の外を押したら、打っていた欄は決める。
        self.commit_field(draw_manager);

        let layout = self.layout();
        let reach = |[left, top, right, bottom]: [f32; 4]| [left, top - BAR_REACH, right, bottom + BAR_REACH];
        let drag = if within(layout.sv, 0.0, local) {
            Some(Drag::SaturationValue)
        } else if within(reach(layout.hue), 0.0, local) {
            Some(Drag::Hue)
        } else if within(reach(layout.alpha), 0.0, local) {
            Some(Drag::Alpha)
        } else {
            // パレットの地を押しただけ。
            None
        };

        if let Some(drag) = drag {
            self.dragging = Some(drag);
            self.drag_to(draw_manager, drag, local);
        }
        self.place(draw_manager);
        true
    }

    fn mouse_left_released(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        self.dragging.take().is_some()
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        // 引いているあいだは、パレットの外へ出ても端で止めて付いてくる。
        let Some(drag) = self.dragging else {
            return false;
        };
        let Some(local) = self.local(draw_manager, pointer.x, pointer.y) else {
            return false;
        };

        self.drag_to(draw_manager, drag, local)
    }

    fn has_focus(&self) -> bool {
        self.focused
    }

    fn unfocus(&mut self, draw_manager: &mut DrawManager) -> bool {
        if !self.focused {
            return false;
        }

        self.focused = false;
        self.close(draw_manager, false);
        self.place(draw_manager);
        true
    }

    fn modifiers_changed(&mut self, modifiers: KeyModifiers) {
        self.shift = modifiers.shift;
    }

    fn wants_ime(&self) -> bool {
        false
    }

    fn text_input(&mut self, draw_manager: &mut DrawManager, _resources: &Resources, text: &str) -> bool {
        let digits: String = text.chars().filter(char::is_ascii_digit).collect();
        if digits.is_empty() || self.editing.is_none() {
            return false;
        }

        self.edit_field(draw_manager, |editing| {
            for digit in digits.chars() {
                if editing.replace {
                    editing.text.clear();
                    editing.replace = false;
                }
                if editing.text.len() < FIELD_DIGITS {
                    editing.text.push(digit);
                }
            }
        })
    }

    fn key_pressed(&mut self, draw_manager: &mut DrawManager, _resources: &Resources, key: EditKey) -> bool {
        if !self.is_open() {
            return match key {
                EditKey::Enter => {
                    self.open(draw_manager);
                    true
                }
                EditKey::Escape => self.unfocus(draw_manager),
                _ => false,
            };
        }

        let next_field = |field: usize, shift: bool| if shift { (field + FIELD_COUNT - 1) % FIELD_COUNT } else { (field + 1) % FIELD_COUNT };

        if let Some(field) = self.editing.as_ref().map(|editing| editing.field) {
            return match key {
                EditKey::Up => self.step_field(draw_manager, 1),
                EditKey::Down => self.step_field(draw_manager, -1),
                EditKey::Backspace => self.edit_field(draw_manager, |editing| {
                    if std::mem::take(&mut editing.replace) {
                        editing.text.clear();
                    } else {
                        editing.text.pop();
                    }
                }),
                EditKey::Delete => self.edit_field(draw_manager, |editing| {
                    if std::mem::take(&mut editing.replace) {
                        editing.text.clear();
                    }
                }),
                EditKey::SelectAll => self.edit_field(draw_manager, |editing| editing.replace = true),
                EditKey::Left | EditKey::Right | EditKey::Home | EditKey::End => {
                    self.edit_field(draw_manager, |editing| editing.replace = false)
                }
                EditKey::Tab => {
                    self.start_editing(draw_manager, next_field(field, self.shift));
                    true
                }
                EditKey::Enter => {
                    self.commit_field(draw_manager);
                    true
                }
                EditKey::Escape => self.close(draw_manager, true),
                _ => false,
            };
        }

        let hsv = self.hsv;
        let wrap = |hue: f32| hue.rem_euclid(1.0);
        match key {
            EditKey::Left => self.set_hsv(draw_manager, Hsv { s: hsv.s - KEY_STEP, ..hsv }),
            EditKey::Right => self.set_hsv(draw_manager, Hsv { s: hsv.s + KEY_STEP, ..hsv }),
            EditKey::Up => self.set_hsv(draw_manager, Hsv { v: hsv.v + KEY_STEP, ..hsv }),
            EditKey::Down => self.set_hsv(draw_manager, Hsv { v: hsv.v - KEY_STEP, ..hsv }),
            EditKey::PageUp => self.set_hsv(draw_manager, Hsv { h: wrap(hsv.h + HUE_KEY_STEP), ..hsv }),
            EditKey::PageDown => self.set_hsv(draw_manager, Hsv { h: wrap(hsv.h - HUE_KEY_STEP), ..hsv }),
            EditKey::Tab => {
                self.start_editing(draw_manager, if self.shift { FIELD_COUNT - 1 } else { 0 });
                true
            }
            EditKey::Enter => self.close(draw_manager, false),
            EditKey::Escape => self.close(draw_manager, true),
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
        if self.text_dirty && self.is_open() {
            self.text_dirty = false;
            animation.relaid_out = self.write_text(draw_manager, resources);
        }

        animation
    }

    fn cursor_shape(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<CursorShape> {
        // 引いているあいだは当てるまでもない。
        if self.dragging.is_some() {
            return Some(CursorShape::Default);
        }
        if !self.hit(draw_manager, pointer) {
            return None;
        }

        let local = self.local(draw_manager, pointer.x, pointer.y)?;
        Some(if within(self.swatch_bounds(), 0.0, local) {
            CursorShape::Pointer
        } else if self.is_open() && self.field_at(local).is_some() {
            CursorShape::Text
        } else {
            CursorShape::Default
        })
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

/// `value` の、`start` から `end` のうちの割合（0 から 1）。
fn ratio(value: f32, start: f32, end: f32) -> f32 {
    if end > start { ((value - start) / (end - start)).clamp(0.0, 1.0) } else { 0.0 }
}

/// 欄の字を 0 から 255 の数へ。空なら 0、255 を超えたら 255。
fn parse_channel(text: &str) -> u8 {
    text.parse::<u32>().map_or(0, |value| value.min(255) as u8)
}

/// sRGB の明るさ `value`（0 から 1）を、線形の明るさへ。透明度の段に使う。
fn linear_alpha(value: f32) -> f32 {
    srgb_to_linear([value, 0.0, 0.0, 1.0])[0]
}

fn hsv_to_rgb(Hsv { h, s, v }: Hsv) -> [u8; 3] {
    let sector = (h.rem_euclid(1.0) * 6.0).min(6.0 - f32::EPSILON);
    let index = sector.floor();
    let fraction = sector - index;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * fraction), v * (1.0 - s * (1.0 - fraction)));

    let (r, g, b) = match index as u32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };

    [r, g, b].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// RGB を HSV へ。灰色（彩度 0）では色相を、黒（明度 0）では色相と彩度を `previous` のまま残す。
fn rgb_to_hsv(rgb: [u8; 3], previous: Hsv) -> Hsv {
    let [r, g, b] = rgb.map(|channel| channel as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let v = max;
    if max == 0.0 {
        return Hsv { v, ..previous };
    }

    let s = delta / max;
    if delta == 0.0 {
        return Hsv { h: previous.h, s, v };
    }

    let h = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } / 6.0;

    Hsv { h, s, v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_reads_six_and_eight_digits() {
        assert_eq!(Rgba::from_hex("#ff8000"), Some(Rgba::new(255, 128, 0, 255)), "6 桁は不透明");
        assert_eq!(Rgba::from_hex("#FF800080"), Some(Rgba::new(255, 128, 0, 128)), "8 桁は透明度付き・大文字も読む");
        assert_eq!(Rgba::new(255, 128, 0, 255).to_hex(), "#ff8000", "不透明なら 6 桁");
        assert_eq!(Rgba::new(255, 128, 0, 128).to_hex(), "#ff800080");
        assert_eq!(Rgba::default(), Rgba::new(0, 0, 0, 255), "既定は不透明な黒");

        for text in ["ff8000", "#ff800", "#ff80000", "#ff8000800", "#gg0000", "#ff 000", "", "#"] {
            assert_eq!(Rgba::from_hex(text), None, "{text:?}");
        }
    }

    #[test]
    fn hsv_and_rgb_convert_both_ways() {
        let zero = Hsv { h: 0.0, s: 0.0, v: 0.0 };
        for rgb in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 128, 0], [12, 200, 99], [128, 128, 128], [255, 255, 255]] {
            assert_eq!(hsv_to_rgb(rgb_to_hsv(rgb, zero)), rgb, "{rgb:?}");
        }

        assert_eq!(hsv_to_rgb(Hsv { h: 1.0, s: 1.0, v: 1.0 }), [255, 0, 0], "色相の右端は赤に戻る");
        assert_eq!(hsv_to_rgb(Hsv { h: 1.0 / 6.0, s: 1.0, v: 1.0 }), [255, 255, 0]);
    }

    #[test]
    fn gray_and_black_keep_the_previous_hue() {
        let previous = Hsv { h: 0.4, s: 0.7, v: 0.9 };

        let gray = rgb_to_hsv([100, 100, 100], previous);
        assert_eq!(gray.h, 0.4, "灰色は色相を残す");
        assert_eq!(gray.s, 0.0);

        let black = rgb_to_hsv([0, 0, 0], previous);
        assert_eq!((black.h, black.s, black.v), (0.4, 0.7, 0.0), "黒は色相と彩度を残す");
    }

    #[test]
    fn channels_parse_and_clamp() {
        assert_eq!(parse_channel(""), 0);
        assert_eq!(parse_channel("42"), 42);
        assert_eq!(parse_channel("999"), 255);
    }

    #[test]
    fn overlay_alphas_match_srgb_mixing_at_the_ends() {
        assert_eq!(linear_alpha(0.0), 0.0);
        assert!((linear_alpha(1.0) - 1.0).abs() < 1e-6);
        // sRGB の 0.5 は線形でおよそ 0.214。
        assert!((linear_alpha(0.5) - 0.214).abs() < 0.001);
    }
}
