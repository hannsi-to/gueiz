//! 文字列を並べて、図形として描けるようにする。
//!
//! # 同じ文字は形を 1 つだけ持つ
//!
//! `Hello` の `l` は 2 回出ますが、形は 1 つです。[`TextRenderer`] は
//! **グリフごとに 1 つだけ [`Object`](crate::object::Object) を登録**し、出てくるたびに
//! インスタンスを足します。
//!
//! これはこのクレートの GPU-driven 描画にそのまま乗ります。
//! 文字が何千文字あっても、形の数は**使った文字の種類の数**まで、
//! ドローは `multi_draw_indirect` 1 回のままです。
//!
//! # 書式
//!
//! 色・太字・斜体・下線などは [`crate::format`] のコードを文字列に埋めて
//! 指定します。[`TextRenderer::write`] は書式を**読みません**。
//! 読ませたいときだけ [`crate::format::Formatted::parse`] を通して
//! [`TextRenderer::write_formatted`] に渡します。
//!
//! # 並べ方
//!
//! [`layout`] と [`layout_formatted`] は純粋な関数で、GPU を触りません。
//! 送り幅・カーニング・改行・書式を見て、各グリフの置き場所を決めます。
//!
//! 対応しているのは**横書きの単純な並び**までです。合字、
//! アラビア語のような連結、縦書き、双方向はやりません。
//! そこまで要るならシェーピングエンジンが別に要ります。
//!
//! # 太字・縁取り・影の作り方
//!
//! 別の書体を持っていないので、**同じ形をずらして重ねて**作っています。
//!
//! - 太字: 上下左右に少しずらした複製を足す
//! - 縁取り: 輪を描くようにずらした複製を、字の後ろに敷く
//! - 影: 1 つだけずらした複製を、いちばん後ろに敷く
//!
//! 形は共有されたままなのでドローは増えません。増えるのはインスタンスだけです。
//! 半透明の字だと重なった部分が濃くなりますが、これは重ねて作る以上避けられません。

use std::ops::Range;

use fxhash::FxHashMap;
use ttf_parser::GlyphId;

use crate::camera::Camera;
use crate::error::Gueiz2DError;
use crate::font::{DEFAULT_TOLERANCE, Font};
use crate::format::{Formatted, LineDecoration, ResetTarget, Shadow, TextFormat, Token};
use crate::object::draw_manager::DrawManager;
use crate::object::{self, instance};
use crate::paint_type::PaintType;
use crate::vertex::Vertex;

/// 出鱈目な字に置き換えるときの候補。
///
/// 送り幅は元の字のものを使うので、ここが狭くても並びは崩れません。
const OBFUSCATION_POOL: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789#$%&@?!";

/// 斜体の傾きを丸める細かさ。1 em あたりの段数。
///
/// 丸めるのは、形の置き場の鍵に `f32` をそのまま使えないから。
/// 段が細かすぎると、ほんの少し違う傾きごとに形が増える。
const SKEW_STEPS: f32 = 256.0;

/// 縁取りを何点で囲むか。
const OUTLINE_POINTS: usize = 8;

/// 丸い端をいくつの辺で作るか。
const CAP_SEGMENTS: usize = 24;

/// 文字の並べ方。
///
/// 文字列の途中で変えたいものは [`crate::format`] のコードで指定します。
/// ここに置くのは**文字列全体の既定値**です。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct TextStyle {
    /// 1 em の大きさ。ピクセル。
    pub size: f32,
    /// 行送り。`None` ならフォントの推奨値。
    pub line_height: Option<f32>,
    /// 字間の追加。ピクセル。
    pub letter_spacing: f32,
    /// カーニングを使うか。
    pub kerning: bool,
    /// 曲線を折れ線に開くときの許容誤差（em に対する割合）。
    pub tolerance: f32,
    /// 太字にしたときの太らせ量。em に対する割合。
    pub bold_weight: f32,
    /// 縁取りの太さ。em に対する割合。
    pub outline_width: f32,
    /// 出鱈目な字を選ぶときの種。
    ///
    /// **毎フレーム変えるとちらつきます。**それが
    /// [`TextFormat::Obfuscated`] の狙いです。動かしたくなければ固定します。
    pub obfuscation_seed: u64,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 32.0,
            line_height: None,
            letter_spacing: 0.0,
            kerning: true,
            tolerance: DEFAULT_TOLERANCE,
            bold_weight: 0.03,
            outline_width: 0.05,
            obfuscation_seed: 0,
        }
    }
}

impl TextStyle {
    pub fn new(size: f32) -> Self {
        Self {
            size,
            ..Default::default()
        }
    }
}

// --- 並べた結果 ---

/// 1 文字ぶんに効いている書式。[`crate::format::TextFormat`] を積み終えた形。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct GlyphStyle {
    /// この字の大きさ。ピクセル。
    pub size: f32,
    /// 上書きされた色。`None` なら呼ぶ側が決めた色を使う。
    pub color: Option<[f32; 4]>,
    /// 不透明度に掛ける値。[`TextFormat::Ghost`] で下がる。
    pub alpha_scale: f32,
    /// 傾き。0 なら立っている。
    pub italic: f32,
    pub bold: bool,
    /// 箱で隠されている。**字は描かれない。**
    pub hidden: bool,
    pub shadow: Option<Shadow>,
    pub outline: Option<[f32; 4]>,
}

impl Default for GlyphStyle {
    fn default() -> Self {
        Self {
            size: 32.0,
            color: None,
            alpha_scale: 1.0,
            italic: 0.0,
            bold: false,
            hidden: false,
            shadow: None,
            outline: None,
        }
    }
}

/// 置き場所の決まったグリフ 1 つ。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct PlacedGlyph {
    /// **描く**グリフ。[`TextFormat::Obfuscated`] なら差し替わっている。
    pub glyph: GlyphId,
    /// 左端。ピクセル。
    pub x: f32,
    /// **ベースライン**の位置。ピクセル。
    pub y: f32,
    /// 次の字までの送り幅。ピクセル。隠し箱の幅にも使う。
    pub advance: f32,
    /// 何行目か。
    pub row: usize,
    pub style: GlyphStyle,
}

/// 字に添える線の位置。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug)]
pub enum LinePosition {
    Underline,
    Strikethrough,
    Overline,
}

/// 置き場所の決まった線 1 本ぶん（[`LineDecoration::lines`] 本まとめて）。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct PlacedLine {
    pub position: LinePosition,
    /// 左端。ピクセル。
    pub x: f32,
    /// 長さ。ピクセル。
    pub width: f32,
    /// 線の高さ。ピクセル。ベースラインではなく**線そのもの**の位置。
    pub y: f32,
    /// 引いている字の大きさ。太さの基準。
    pub size: f32,
    pub decoration: LineDecoration,
    pub color: Option<[f32; 4]>,
    pub alpha_scale: f32,
}

impl PlacedLine {
    /// 線の太さ。ピクセル。
    pub fn thickness(&self) -> f32 {
        self.decoration.line_width * self.size
    }
}

/// **実際に塗られる**範囲。
///
/// # なぜ送り幅と別に要るのか
///
/// 送り幅は「次の字をどこに置くか」でしかありません。**はみ出すものは
/// そこに入っていない**ので、送り幅だけを見て場所を決めると、
/// はみ出したぶんが切れます。
///
/// はみ出すのは 3 つ。
///
/// - **斜体**: 上が右に、下が左に出る（`傾き x アセンダ` ぶん）
/// - **影**: ずらした向きに丸ごと出る
/// - **縁取り・太字**: 四方に出る
///
/// 右端に寄せる、背景の箱を敷く、画面からはみ出していないか見る。
/// こういうときは [`TextLayout::width`] ではなくこちらを見ます。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct InkBounds {
    pub left: f32,
    pub right: f32,
    /// 上端。**下向きが正**なので、ふつうは負。
    pub top: f32,
    pub bottom: f32,
}

impl InkBounds {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }

    /// 描く場所へ動かした範囲。
    ///
    /// [`TextLayout`] の座標は**左上を原点**にしているので、
    /// [`TextRenderer::write`] に渡した `(x, y)` を足すと画面の位置になる。
    ///
    /// ```
    /// # use gueiz_2d::text::InkBounds;
    /// let ink = InkBounds { left: -3.0, right: 100.0, top: 0.0, bottom: 40.0 };
    /// let on_screen = ink.translated(50.0, 20.0);
    ///
    /// assert_eq!(on_screen.left, 47.0);
    /// assert_eq!(on_screen.bottom, 60.0);
    /// ```
    pub fn translated(self, x: f32, y: f32) -> Self {
        Self {
            left: self.left + x,
            right: self.right + x,
            top: self.top + y,
            bottom: self.bottom + y,
        }
    }

    /// 両方を囲む範囲。
    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            right: self.right.max(other.right),
            top: self.top.min(other.top),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// y を動かす。ベースラインが決まってから足す。
    fn offset_y(self, amount: f32) -> Self {
        Self {
            top: self.top + amount,
            bottom: self.bottom + amount,
            ..self
        }
    }
}

/// 1 行ぶんの寸法。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct TextRow {
    /// 行の上端。ピクセル。
    pub top: f32,
    /// ベースライン。ピクセル。
    pub baseline: f32,
    /// 上端から下端まで。ピクセル。
    pub height: f32,
    /// この行の**送り幅**。ピクセル。次の字を置く位置であって、
    /// 塗られる範囲ではない。はみ出すぶんは [`TextRow::ink`]。
    pub width: f32,
    /// この行で実際に塗られる範囲。
    pub ink: InkBounds,
}

/// 並べた結果。
#[derive(Clone)]
#[derive(Debug, Default)]
pub struct TextLayout {
    pub glyphs: Vec<PlacedGlyph>,
    /// 下線・打消し線・上線。
    pub decorations: Vec<PlacedLine>,
    /// 行ごとの寸法。空行も 1 行として入る。
    pub rows: Vec<TextRow>,
    /// いちばん長い行の**送り幅**。ピクセル。
    ///
    /// 斜体・影・縁取りは**ここに入りません**。塗られる範囲は
    /// [`TextLayout::ink`] を見ます。
    pub width: f32,
    /// 全体の高さ。ピクセル。こちらも送り幅と同じで、
    /// はみ出すぶんは入らない。
    pub height: f32,
    /// 1 行目のベースラインの位置。ピクセル。
    pub first_baseline: f32,
    /// 実際に塗られる範囲。はみ出すものを全部含む。
    pub ink: InkBounds,
}

/// 文字列が使う大きさ。置き場所は含まない。
///
/// # 幅と高さが 2 組あるのはなぜか
///
/// **送り幅の箱**（[`TextSize::width`] / [`TextSize::height`]）は、
/// 次の字をどこに置くかで決まる箱です。行を並べる、字送りを合わせる、
/// 表の桁をそろえる。こういうときはこちらを使います。
///
/// **塗られる箱**（[`TextSize::ink`]）は、実際に色が乗る範囲です。
/// 斜体・影・縁取り・太字・丸い端は送り幅の外に出るので、
/// **右端に寄せる・背景を敷く・画面に収まるか見る**ときはこちらです。
///
/// 何も付いていない字なら 2 つは一致します。
///
/// ```no_run
/// # use gueiz_2d::font::Font;
/// # use gueiz_2d::text::{measure, TextStyle};
/// # fn run(font: &Font) {
/// let size = measure(font, "Hello", &TextStyle::new(32.0));
///
/// println!("送り幅 {} x {}", size.width, size.height);
/// println!("塗る幅 {} x {}", size.ink_width(), size.ink_height());
/// # }
/// ```
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct TextSize {
    /// いちばん長い行の送り幅。ピクセル。
    pub width: f32,
    /// 上端から最後の行の下端まで。ピクセル。
    pub height: f32,
    /// 実際に塗られる範囲。**左上を原点とした相対位置。**
    /// 画面の位置は [`InkBounds::translated`] で出す。
    pub ink: InkBounds,
    /// 行数。空行も数える。
    pub rows: usize,
    /// 1 行目のベースラインが上端からどれだけ下か。ピクセル。
    pub first_baseline: f32,
}

impl TextSize {
    /// 塗られる範囲の幅。
    pub fn ink_width(&self) -> f32 {
        self.ink.width()
    }

    /// 塗られる範囲の高さ。
    pub fn ink_height(&self) -> f32 {
        self.ink.height()
    }

    /// 送り幅の箱。`[幅, 高さ]`。
    pub fn to_array(&self) -> [f32; 2] {
        [self.width, self.height]
    }
}

impl TextLayout {
    /// 大きさだけを取り出す。
    pub fn size(&self) -> TextSize {
        TextSize {
            width: self.width,
            height: self.height,
            ink: self.ink,
            rows: self.rows.len(),
            first_baseline: self.first_baseline,
        }
    }

    /// 中身を空にする。確保した領域は残すので、詰め直しても割り当てが起きない。
    pub fn clear(&mut self) {
        self.glyphs.clear();
        self.decorations.clear();
        self.rows.clear();
        self.width = 0.0;
        self.height = 0.0;
        self.first_baseline = 0.0;
        self.ink = InkBounds::default();
    }

    /// ある行に載っているグリフの範囲。
    pub fn row_glyphs(&self, row: usize) -> Range<usize> {
        let start = self
            .glyphs
            .iter()
            .position(|glyph| glyph.row == row)
            .unwrap_or(self.glyphs.len());
        let end = start
            + self.glyphs[start..]
                .iter()
                .take_while(|glyph| glyph.row == row)
                .count();

        start..end
    }
}

// --- 並べる ---

/// 並べるのに要るフォントの情報だけを切り出したもの。
///
/// [`Font`] が実装します。外に出していないのは、これが**並べ方を試すための
/// 継ぎ目**だからです。作り物を差し込めば、フォントを読まなくても
/// 行の高さや線の切れ目を検算できます。
trait Metrics {
    fn ascender(&self) -> f32;
    fn descender(&self) -> f32;
    fn line_height(&self) -> f32;
    fn glyph(&self, character: char) -> Option<GlyphId>;
    fn advance(&self, glyph: GlyphId) -> f32;
    fn kerning(&self, left: GlyphId, right: GlyphId) -> f32;
    fn underline_position(&self) -> f32;
    fn strikeout_position(&self) -> f32;
    fn overline_position(&self) -> f32;
}

impl Metrics for Font<'_> {
    fn ascender(&self) -> f32 {
        Font::ascender(self)
    }

    fn descender(&self) -> f32 {
        Font::descender(self)
    }

    fn line_height(&self) -> f32 {
        Font::line_height(self)
    }

    fn glyph(&self, character: char) -> Option<GlyphId> {
        Font::glyph(self, character)
    }

    fn advance(&self, glyph: GlyphId) -> f32 {
        Font::advance(self, glyph)
    }

    fn kerning(&self, left: GlyphId, right: GlyphId) -> f32 {
        Font::kerning(self, left, right)
    }

    fn underline_position(&self) -> f32 {
        Font::underline_position(self)
    }

    fn strikeout_position(&self) -> f32 {
        Font::strikeout_position(self)
    }

    fn overline_position(&self) -> f32 {
        Font::overline_position(self)
    }
}


/// 書式を読まずに並べる。`\n` だけが改行になる。
///
/// 原点は**左上**で、1 行目のベースラインは `first_baseline` だけ下がります。
///
/// ```no_run
/// # use gueiz_2d::font::Font;
/// # use gueiz_2d::text::{layout, TextStyle};
/// # fn run(font: &Font) {
/// let placed = layout(font, "Hello\nWorld", &TextStyle::new(48.0));
///
/// println!("{} 文字、幅 {} px", placed.glyphs.len(), placed.width);
/// # }
/// ```
pub fn layout(font: &Font, text: &str, style: &TextStyle) -> TextLayout {
    layout_formatted(font, &Formatted::plain(text), style)
}

/// 書式込みで並べる。
///
/// ```no_run
/// # use gueiz_2d::font::Font;
/// # use gueiz_2d::format::Formatted;
/// # use gueiz_2d::text::{layout_formatted, TextStyle};
/// # fn run(font: &Font) -> Result<(), Box<dyn std::error::Error>> {
/// let formatted = Formatted::parse("§[red]赤§[/]§[size 64]大きい")?;
/// let placed = layout_formatted(font, &formatted, &TextStyle::new(32.0));
///
/// // 行の高さは、その行でいちばん大きい字が決める。
/// println!("{} px", placed.height);
/// # Ok(())
/// # }
/// ```
pub fn layout_formatted(
    font: &Font,
    formatted: &Formatted,
    style: &TextStyle,
) -> TextLayout {
    let mut out = TextLayout::default();
    layout_with(font, formatted, style, &mut out);

    out
}

/// すでにある [`TextLayout`] に詰め直す。
///
/// 中身は先に空にします。**確保した領域は残る**ので、毎フレーム並べ直す
/// ときに 1 つ持ち回せば割り当てが起きません。
///
/// ```no_run
/// # use gueiz_2d::font::Font;
/// # use gueiz_2d::format::Formatted;
/// # use gueiz_2d::text::{layout_into, TextLayout, TextStyle};
/// # fn run(font: &Font) {
/// let mut reused = TextLayout::default();
///
/// for frame in 0..60 {
///     let text = format!("{frame} 枚目");
///     // 2 周目からは割り当てが起きない。
///     layout_into(font, &Formatted::plain(&text), &TextStyle::new(24.0), &mut reused);
/// }
/// # }
/// ```
pub fn layout_into(
    font: &Font,
    formatted: &Formatted,
    style: &TextStyle,
    out: &mut TextLayout,
) {
    layout_with(font, formatted, style, out);
}

/// 文字列が使う大きさ。書式は読まない。**GPU は触らない。**
///
/// 中で 1 度並べるので、[`layout`] と同じだけの手間が掛かります。
/// 並べた結果も要るなら、[`layout`] を呼んで
/// [`TextLayout::size`] を取るほうが 1 度で済みます。
///
/// ```no_run
/// # use gueiz_2d::font::Font;
/// # use gueiz_2d::text::{measure, TextStyle};
/// # fn run(font: &Font) {
/// let size = measure(font, "Hello\nWorld", &TextStyle::new(32.0));
///
/// // 右下に寄せる。塗られる範囲で測らないと、影や斜体がはみ出す。
/// let x = 800.0 - size.ink.right;
/// let y = 600.0 - size.ink.bottom;
/// # let _ = (x, y);
/// # }
/// ```
pub fn measure(font: &Font, text: &str, style: &TextStyle) -> TextSize {
    measure_formatted(font, &Formatted::plain(text), style)
}

/// 書式込みで、文字列が使う大きさ。
///
/// ```no_run
/// # use gueiz_2d::font::Font;
/// # use gueiz_2d::format::Formatted;
/// # use gueiz_2d::text::{measure_formatted, TextStyle};
/// # fn run(font: &Font) -> Result<(), Box<dyn std::error::Error>> {
/// let formatted = Formatted::parse("§[italic 0.3]§[shadow]傾けて影も")?;
/// let size = measure_formatted(font, &formatted, &TextStyle::new(32.0));
///
/// // 送り幅は傾きも影も知らない。塗られる幅のほうが広い。
/// assert!(size.ink_width() > size.width);
/// # Ok(())
/// # }
/// ```
pub fn measure_formatted(
    font: &Font,
    formatted: &Formatted,
    style: &TextStyle,
) -> TextSize {
    // 並べた中身は捨てるが、大きさは並べてみないと分からない。
    // 行の高さがその行でいちばん大きい字で決まるため。
    let mut out = TextLayout::default();
    layout_with(font, formatted, style, &mut out);

    out.size()
}

fn layout_with<M: Metrics>(
    font: &M,
    formatted: &Formatted,
    style: &TextStyle,
    out: &mut TextLayout,
) {
    out.clear();

    let mut context = LayoutContext::new(font, style);

    for token in formatted.tokens() {
        match token {
            Token::Format(format) => context.apply(*format, out),
            Token::Text(text) => {
                for character in text.chars() {
                    context.character(character, out);
                }
            }
        }
    }

    context.finish(out);
}

/// 積み上がった書式。どれも「指定されていない」が既定。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
struct FormatState {
    color: Option<[f32; 4]>,
    size: Option<f32>,
    ghost: f32,
    italic: f32,
    bold: bool,
    obfuscated: bool,
    hidden: bool,
    shadow: Option<Shadow>,
    outline: Option<[f32; 4]>,
    underline: Option<LineDecoration>,
    strikethrough: Option<LineDecoration>,
    overline: Option<LineDecoration>,
    space_x: Option<f32>,
    space_y: Option<f32>,
}

impl FormatState {
    /// 状態だけで決まる書式を積む。
    ///
    /// 行やスタックに関わるもの（[`TextFormat::NewLine`]、
    /// [`TextFormat::SkipPush`]、[`TextFormat::SkipPop`]）はここでは扱えないので、
    /// そのまま返して呼ぶ側に任せる。
    fn apply(&mut self, format: TextFormat) -> Option<TextFormat> {
        match format {
            TextFormat::Color(color) => self.color = Some(color),
            TextFormat::DefaultColor => self.color = None,
            TextFormat::Obfuscated => self.obfuscated = true,
            TextFormat::Bold => self.bold = true,
            TextFormat::FontSize(size) => self.size = Some(size),
            TextFormat::Strikethrough(line) => self.strikethrough = Some(line),
            TextFormat::Underline(line) => self.underline = Some(line),
            TextFormat::Overline(line) => self.overline = Some(line),
            TextFormat::Italic(value) => self.italic = value,
            TextFormat::Ghost(value) => self.ghost = value.clamp(0.0, 1.0),
            TextFormat::HideBox => self.hidden = true,
            TextFormat::Shadow(shadow) => self.shadow = Some(shadow),
            TextFormat::Outline(color) => self.outline = Some(color),
            TextFormat::SpaceX(value) => self.space_x = Some(value),
            TextFormat::SpaceY(value) => self.space_y = Some(value),
            TextFormat::Reset(target) => self.reset(target),

            other => return Some(other),
        }

        None
    }

    fn reset(&mut self, target: ResetTarget) {
        match target {
            ResetTarget::All => *self = Self::default(),
            ResetTarget::Color => self.color = None,
            ResetTarget::Obfuscated => self.obfuscated = false,
            ResetTarget::Bold => self.bold = false,
            ResetTarget::FontSize => self.size = None,
            ResetTarget::Strikethrough => self.strikethrough = None,
            ResetTarget::Underline => self.underline = None,
            ResetTarget::Overline => self.overline = None,
            ResetTarget::Italic => self.italic = 0.0,
            ResetTarget::Ghost => self.ghost = 0.0,
            ResetTarget::HideBox => self.hidden = false,
            ResetTarget::Shadow => self.shadow = None,
            ResetTarget::Outline => self.outline = None,
            ResetTarget::SpaceX => self.space_x = None,
            ResetTarget::SpaceY => self.space_y = None,
        }
    }
}

/// 積み上げた書式と、[`TextFormat::SkipPush`] で退避したぶん。
///
/// 並べる処理から切り離してあるので、フォントが無くても試せる。
#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
struct FormatStack {
    state: FormatState,
    skipped: Vec<FormatState>,
}

impl FormatStack {
    /// 書式を 1 つ積む。**改行なら `true`。**改行だけは行の処理が要るので、
    /// ここでは何もせず呼ぶ側に知らせる。
    fn apply(&mut self, format: TextFormat) -> bool {
        let Some(rest) = self.state.apply(format) else {
            return false;
        };

        match rest {
            TextFormat::NewLine => return true,

            // 退避して素に戻す。`SkipPop` で戻す。
            TextFormat::SkipPush => {
                self.skipped.push(self.state);
                self.state = FormatState::default();
            }

            // 退避が無ければ何もしない。落とすほどのことではない。
            TextFormat::SkipPop => {
                if let Some(previous) = self.skipped.pop() {
                    self.state = previous;
                }
            }

            unexpected => unreachable!("{:?} は FormatState が引き受けるはず", unexpected),
        }

        false
    }
}

/// 線 1 続きぶんの見た目。ここが変わったら線を切る。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
struct RunStyle {
    decoration: LineDecoration,
    color: Option<[f32; 4]>,
    alpha_scale: f32,
    size: f32,
}

const LINE_POSITIONS: [LinePosition; 3] = [
    LinePosition::Underline,
    LinePosition::Strikethrough,
    LinePosition::Overline,
];

struct LayoutContext<'a, M: Metrics> {
    font: &'a M,
    style: &'a TextStyle,
    formats: FormatStack,

    pen_x: f32,
    previous: Option<GlyphId>,

    row_index: usize,
    row_top: f32,
    row_ascent: f32,
    row_descent: f32,
    row_gap: f32,
    row_start_glyph: usize,
    row_start_decoration: usize,
    /// この行で塗られる範囲。**y はベースラインからの相対**。
    /// ベースラインは行が閉じるまで決まらないので、あとから足す。
    row_ink: Option<InkBounds>,

    /// 開いている線。[`LINE_POSITIONS`] と同じ並び。
    open: [Option<(RunStyle, f32)>; 3],

    /// 何文字目か。出鱈目な字を選ぶ種に混ぜる。
    character_index: u64,
}

impl<'a, M: Metrics> LayoutContext<'a, M> {
    fn new(font: &'a M, style: &'a TextStyle) -> Self {
        Self {
            font,
            style,
            formats: FormatStack::default(),
            pen_x: 0.0,
            previous: None,
            row_index: 0,
            row_top: 0.0,
            row_ascent: 0.0,
            row_descent: 0.0,
            row_gap: 0.0,
            row_start_glyph: 0,
            row_start_decoration: 0,
            row_ink: None,
            open: [None, None, None],
            character_index: 0,
        }
    }

    fn state(&self) -> &FormatState {
        &self.formats.state
    }

    fn size(&self) -> f32 {
        self.state().size.unwrap_or(self.style.size)
    }

    fn alpha_scale(&self) -> f32 {
        1.0 - self.state().ghost
    }

    fn apply(&mut self, format: TextFormat, out: &mut TextLayout) {
        if self.formats.apply(format) {
            self.break_row(out);
        }
    }

    fn character(&mut self, character: char, out: &mut TextLayout) {
        if character == '\n' {
            self.break_row(out);
            return;
        }

        let index = self.character_index;
        self.character_index += 1;

        let Some(glyph) = self.font.glyph(character) else {
            // 持っていない文字は詰めずに飛ばす。豆腐を出すほうが親切だが、
            // それは呼ぶ側が代替フォントで決めること。
            self.previous = None;
            return;
        };

        let size = self.size();

        // 送り幅は**元のグリフ**で決める。出鱈目な字に置き換えても
        // 並びが動かないのは、ここで元のほうを見ているから。
        if let Some(left) = self.previous.filter(|_| self.style.kerning) {
            self.pen_x += self.font.kerning(left, glyph) * size;
        }

        self.sync_decorations(out);

        // 字間は書式が指定していればそちらが勝つ。積むのではなく置き換える。
        let spacing = self.state().space_x.unwrap_or(self.style.letter_spacing);
        let mut advance = self.font.advance(glyph) * size + spacing;

        if self.state().bold {
            // 太らせたぶんだけ広げないと、字がくっつく。
            advance += self.style.bold_weight * size;
        }

        let drawn = if self.state().obfuscated {
            self.scramble(glyph, index)
        } else {
            glyph
        };

        out.glyphs.push(PlacedGlyph {
            glyph: drawn,
            x: self.pen_x,
            // ベースラインは行が閉じるまで決まらない。閉じるときに足す。
            y: 0.0,
            advance,
            row: self.row_index,
            style: GlyphStyle {
                size,
                color: self.state().color,
                alpha_scale: self.alpha_scale(),
                italic: self.state().italic,
                bold: self.state().bold,
                hidden: self.state().hidden,
                shadow: self.state().shadow,
                outline: self.state().outline,
            },
        });

        self.grow_row(size);
        self.note_glyph_ink(size, advance);
        self.pen_x += advance;
        self.previous = Some(glyph);
    }

    /// いま置いた字が、送り幅からどれだけはみ出すかを足し込む。
    ///
    /// ここを忘れると、斜体の上や影が箱の外に出たまま気づけません。
    fn note_glyph_ink(&mut self, size: f32, advance: f32) {
        let ascent = self.font.ascender() * size;
        let descent = self.font.descender() * size;

        let mut left = 0.0_f32;
        let mut right = 0.0_f32;
        let mut up = 0.0_f32;
        let mut down = 0.0_f32;

        // 隠し箱は送り幅ぴったりの四角。傾けも縁取りも掛からない。
        if !self.state().hidden {
            // 傾けると、上端は右に、下端は左にずれる（傾きが負なら逆）。
            let top_shift = self.state().italic * ascent;
            let bottom_shift = -self.state().italic * descent;

            left = top_shift.min(bottom_shift).min(0.0);
            right = top_shift.max(bottom_shift).max(0.0);

            if self.state().bold {
                let weight = self.style.bold_weight * size;

                left -= weight;
                right += weight;
                up -= weight;
                down += weight;
            }

            if self.state().outline.is_some() {
                let radius = self.style.outline_width * size;

                left -= radius;
                right += radius;
                up -= radius;
                down += radius;
            }

            if let Some(shadow) = self.state().shadow {
                let [dx, dy] = shadow.offset();

                // 影は丸ごとずれた複製。元の位置にも字があるので、
                // 広がるのはずれた向きだけ。
                left = left.min(dx * size);
                right = right.max(dx * size);
                up = up.min(dy * size);
                down = down.max(dy * size);
            }
        }

        self.extend_ink(InkBounds {
            left: self.pen_x + left,
            right: self.pen_x + advance + right,
            top: -ascent + up,
            bottom: descent + down,
        });
    }

    fn extend_ink(&mut self, bounds: InkBounds) {
        self.row_ink = Some(match self.row_ink {
            Some(existing) => existing.union(bounds),
            None => bounds,
        });
    }

    /// 元のグリフの代わりに出す、出鱈目なグリフ。
    ///
    /// フォントが候補を持っていなければ元のまま。
    fn scramble(&self, glyph: GlyphId, index: u64) -> GlyphId {
        let pick = scramble_bits(self.style.obfuscation_seed, index) as usize
            % OBFUSCATION_POOL.len();

        self.font
            .glyph(OBFUSCATION_POOL[pick] as char)
            .unwrap_or(glyph)
    }

    /// この行の高さを、載せた字に合わせて広げる。
    fn grow_row(&mut self, size: f32) {
        let font = self.font;

        self.row_ascent = self.row_ascent.max(font.ascender() * size);
        self.row_descent = self.row_descent.max(font.descender() * size);
        self.row_gap = self.row_gap.max(
            (font.line_height() - font.ascender() - font.descender()) * size,
        );
    }

    /// 開いている線と、いま効いている書式を突き合わせる。
    ///
    /// 見た目が変わったところで線を切って、次を開く。
    fn sync_decorations(&mut self, out: &mut TextLayout) {
        let desired = [
            self.state().underline.map(|line| self.run_style(line)),
            self.state().strikethrough.map(|line| self.run_style(line)),
            self.state().overline.map(|line| self.run_style(line)),
        ];

        for (slot, wanted) in desired.into_iter().enumerate() {
            let unchanged = match (&self.open[slot], &wanted) {
                (Some((open, _)), Some(wanted)) => open == wanted,
                (None, None) => true,
                _ => false,
            };

            if unchanged {
                continue;
            }

            self.close_run(slot, out);

            if let Some(wanted) = wanted {
                self.open[slot] = Some((wanted, self.pen_x));
            }
        }
    }

    fn run_style(&self, decoration: LineDecoration) -> RunStyle {
        RunStyle {
            decoration,
            color: self.state().color,
            alpha_scale: self.alpha_scale(),
            size: self.size(),
        }
    }

    fn close_run(&mut self, slot: usize, out: &mut TextLayout) {
        let Some((run, start_x)) = self.open[slot].take() else {
            return;
        };

        let width = self.pen_x - start_x;

        // 長さ 0 の線は描いても見えない。書式を付けてすぐ外した跡。
        if width <= 0.0 || run.decoration.lines == 0 {
            return;
        }

        let position = LINE_POSITIONS[slot];
        let offset = match position {
            LinePosition::Underline => self.font.underline_position(),
            LinePosition::Strikethrough => self.font.strikeout_position(),
            LinePosition::Overline => self.font.overline_position(),
        };

        let y = offset * run.size;
        let thickness = run.decoration.line_width * run.size;
        // 何本あっても、まとまりの中心が本来の高さに来る。端から端まで。
        let spread = (run.decoration.lines - 1) as f32 * thickness + thickness / 2.0;
        let cap = thickness / 2.0;

        self.extend_ink(InkBounds {
            left: start_x - if run.decoration.joint_type.caps_start() { cap } else { 0.0 },
            right: self.pen_x + if run.decoration.joint_type.caps_end() { cap } else { 0.0 },
            top: y - spread,
            bottom: y + spread,
        });

        out.decorations.push(PlacedLine {
            position,
            x: start_x,
            width,
            // ベースラインは行が閉じるときに足す。
            y,
            size: run.size,
            decoration: run.decoration,
            color: run.color,
            alpha_scale: run.alpha_scale,
        });
    }

    /// 行を閉じて、次の行に移る。
    fn break_row(&mut self, out: &mut TextLayout) {
        for slot in 0..LINE_POSITIONS.len() {
            self.close_run(slot, out);
        }

        // 字が 1 つも無い行でも、1 行ぶんの高さは取る。
        if out.glyphs.len() == self.row_start_glyph {
            self.grow_row(self.size());
        }

        let baseline = self.row_top + self.row_ascent;

        for glyph in &mut out.glyphs[self.row_start_glyph..] {
            glyph.y += baseline;
        }

        for line in &mut out.decorations[self.row_start_decoration..] {
            line.y += baseline;
        }

        let height = self.row_ascent + self.row_descent;

        // 塗られる範囲もベースラインを足して絶対の位置にする。
        let ink = self
            .row_ink
            .take()
            .unwrap_or_default()
            .offset_y(baseline);

        out.rows.push(TextRow {
            top: self.row_top,
            baseline,
            height,
            width: self.pen_x,
            ink,
        });

        out.width = out.width.max(self.pen_x);
        out.ink = if out.rows.len() == 1 { ink } else { out.ink.union(ink) };

        // 行送り。書式の指定がいちばん強く、次が [`TextStyle::line_height`]、
        // どちらも無ければフォントの推奨値。
        self.row_top += self
            .state()
            .space_y
            .or(self.style.line_height)
            .unwrap_or(height + self.row_gap);
        self.row_index += 1;
        self.row_start_glyph = out.glyphs.len();
        self.row_start_decoration = out.decorations.len();
        self.row_ascent = 0.0;
        self.row_descent = 0.0;
        self.row_gap = 0.0;
        self.pen_x = 0.0;
        self.previous = None;
    }

    fn finish(mut self, out: &mut TextLayout) {
        // 最後の行も閉じる。閉じないと高さもベースラインも入らない。
        self.break_row(out);

        // 最後の行の下端が全体の高さ。行送りを足すと、下に余白が付く。
        out.height = out
            .rows
            .last()
            .map_or(0.0, |row| row.top + row.height);
        out.first_baseline = out.rows.first().map_or(0.0, |row| row.baseline);
    }
}

/// 種と番号から、偏りの少ないビット列を作る。splitmix64。
fn scramble_bits(seed: u64, index: u64) -> u64 {
    let mut value = seed.wrapping_add(index.wrapping_mul(0x9e37_79b9_7f4a_7c15));

    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);

    value ^ (value >> 31)
}

// --- 描く ---

/// 何を、どの重なり順で描くか。
///
/// 重なり順は [`Object`](crate::object::Object) ごとにしか持てないので、
/// 段ごとに別の形を登録する。形が同じでも、`z` が違えば別の `Object` が要る。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug)]
enum Layer {
    Shadow,
    Outline,
    Glyph,
    Decoration,
    HideBox,
}

impl Layer {
    /// 呼ぶ側が決めた `z` からのずらし幅。
    ///
    /// 細かく刻んであるので、他の図形との前後関係はほとんど変わらない。
    fn z_offset(self) -> f32 {
        match self {
            Self::Shadow => -0.003,
            Self::Outline => -0.002,
            Self::Glyph => 0.0,
            Self::Decoration => 0.001,
            Self::HideBox => 0.002,
        }
    }
}

/// 登録した形を引く鍵。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug)]
enum ShapeKey {
    /// 1 文字ぶんの輪郭。傾きは形に焼き込むので鍵に入る。
    Glyph {
        glyph: GlyphId,
        /// 傾きを [`SKEW_STEPS`] で丸めた値。
        skew: i32,
        layer: Layer,
    },
    /// 0..1 の正方形。隠し箱と線の胴に使い回す。
    Square(Layer),
    /// 直径 1 の円。丸い端に使う。
    Disc(Layer),
}

/// 並べた文字を [`DrawManager`] に載せる。
///
/// グリフごとに 1 つだけ形を登録し、出てくるたびにインスタンスを足します。
/// 同じ [`TextRenderer`] を使い回せば、形は共有されます。
///
/// # 毎フレーム書き直すとき
///
/// [`TextRenderer::write`] はインスタンスを**足します**。
/// 毎フレーム呼ぶなら、先に [`TextRenderer::clear`] でインスタンスを
/// 空にしてください。形は残るので、作り直しは起きません。
pub struct TextRenderer {
    /// 形の鍵 → 登録した名前。**名前を持つのは、引くたびに
    /// 作り直さないため。** 貸し出しは `&str` で返すので、
    /// 1 文字ごとに確保が起きることはない。
    shapes: FxHashMap<ShapeKey, Option<String>>,
    camera: Camera,
    color: [f32; 4],
    z: f32,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    pub fn new() -> Self {
        Self {
            shapes: FxHashMap::default(),
            camera: Camera::default(),
            color: [1.0; 4],
            z: 0.0,
        }
    }

    /// 文字を写すカメラ。**形を作る前に決めること。**
    ///
    /// グリフは出てきた順に登録されるので、後から変えても
    /// すでに登録したぶんには効かない。
    pub fn camera(&mut self, camera: Camera) -> &mut Self {
        self.camera = camera;
        self
    }

    /// 登録済みの形。カメラを貼り直したいときなどに。
    pub fn shape_ids(&self) -> impl Iterator<Item = &str> {
        self.shapes.values().filter_map(|name| name.as_deref())
    }

    /// 字の色。[`TextFormat::Color`] で上書きされていない字に効く。
    pub fn color(&mut self, red: f32, green: f32, blue: f32, alpha: f32) -> &mut Self {
        self.color = [red, green, blue, alpha];
        self
    }

    /// 重なり順。大きいほど手前。影と縁取りはこれより少し奥に入る。
    ///
    /// **形を作る前に決めること。**カメラと同じで、後から変えても
    /// 登録済みのぶんには効かない。
    pub fn z(&mut self, z: f32) -> &mut Self {
        self.z = z;
        self
    }

    /// 登録済みの形の数。
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    /// 置いたインスタンスを全部消す。形は残す。
    ///
    /// 毎フレーム書き直すときは、[`TextRenderer::write`] の前にこれを呼びます。
    pub fn clear(&mut self, draw_manager: &mut DrawManager) -> &mut Self {
        for name in self.shapes.values().flatten() {
            if let Some(object) = draw_manager.object_mut(name) {
                object.clear_instances();
            }
        }

        self
    }

    /// 文字列を `(x, y)` に置く。`y` は**上端**。書式は読まない。
    ///
    /// ```no_run
    /// # use gueiz_2d::font::Font;
    /// # use gueiz_2d::object::DrawManager;
    /// # use gueiz_2d::text::{TextRenderer, TextStyle};
    /// # fn run(font: &Font, draw_manager: &mut DrawManager) -> Result<(), gueiz_2d::error::Gueiz2DError> {
    /// let mut text = TextRenderer::new();
    /// text.color(1.0, 1.0, 1.0, 1.0);
    /// text.write(draw_manager, font, "Hello", &TextStyle::new(48.0), 100.0, 100.0)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn write(
        &mut self,
        draw_manager: &mut DrawManager,
        font: &Font,
        text: &str,
        style: &TextStyle,
        x: f32,
        y: f32,
    ) -> Result<TextLayout, Gueiz2DError> {
        self.write_formatted(draw_manager, font, &Formatted::plain(text), style, x, y)
    }

    /// 書式を読んで置く。
    ///
    /// ```no_run
    /// # use gueiz_2d::font::Font;
    /// # use gueiz_2d::format::Formatted;
    /// # use gueiz_2d::object::DrawManager;
    /// # use gueiz_2d::text::{TextRenderer, TextStyle};
    /// # fn run(font: &Font, draw_manager: &mut DrawManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let formatted = Formatted::parse("§[gold]§[bold]勝利§[/]§[ln]§[underline]次へ")?;
    ///
    /// let mut text = TextRenderer::new();
    /// text.write_formatted(draw_manager, font, &formatted, &TextStyle::new(48.0), 16.0, 16.0)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn write_formatted(
        &mut self,
        draw_manager: &mut DrawManager,
        font: &Font,
        formatted: &Formatted,
        style: &TextStyle,
        x: f32,
        y: f32,
    ) -> Result<TextLayout, Gueiz2DError> {
        let placed = layout_formatted(font, formatted, style);

        for glyph in &placed.glyphs {
            self.place_glyph(draw_manager, font, style, glyph, x, y)?;
        }

        for line in &placed.decorations {
            self.place_line(draw_manager, line, x, y)?;
        }

        Ok(placed)
    }

    /// 1 文字ぶんを置く。影・縁取り・太字の複製もここで足す。
    fn place_glyph(
        &mut self,
        draw_manager: &mut DrawManager,
        font: &Font,
        style: &TextStyle,
        glyph: &PlacedGlyph,
        origin_x: f32,
        origin_y: f32,
    ) -> Result<(), Gueiz2DError> {
        let size = glyph.style.size;
        let left = origin_x + glyph.x;
        let baseline = origin_y + glyph.y;

        let mut color = glyph.style.color.unwrap_or(self.color);
        color[3] *= glyph.style.alpha_scale;

        if glyph.style.hidden {
            // 隠すのだから字は置かない。箱を薄くしても中身は読めない。
            let top = baseline - font.ascender() * size;
            let height = (font.ascender() + font.descender()) * size;

            return self.place_square(
                draw_manager,
                Layer::HideBox,
                left,
                top,
                glyph.advance,
                height,
                color,
            );
        }

        let skew = quantize_skew(glyph.style.italic);

        // 影がいちばん奥、その上に縁取り、いちばん手前が字。
        if let Some(shadow) = glyph.style.shadow {
            let [dx, dy] = shadow.offset();
            // 影の色は字の色を落としたもの。別々に持たせるほどではない。
            let shadow_color = [color[0] * 0.25, color[1] * 0.25, color[2] * 0.25, color[3]];

            self.place_copy(
                draw_manager,
                font,
                style,
                glyph.glyph,
                skew,
                Layer::Shadow,
                left + dx * size,
                baseline + dy * size,
                size,
                shadow_color,
            )?;
        }

        if let Some(mut outline_color) = glyph.style.outline {
            outline_color[3] *= glyph.style.alpha_scale;
            let radius = style.outline_width * size;

            for point in 0..OUTLINE_POINTS {
                let angle = std::f32::consts::TAU * point as f32 / OUTLINE_POINTS as f32;

                self.place_copy(
                    draw_manager,
                    font,
                    style,
                    glyph.glyph,
                    skew,
                    Layer::Outline,
                    left + radius * angle.cos(),
                    baseline + radius * angle.sin(),
                    size,
                    outline_color,
                )?;
            }
        }

        self.place_copy(
            draw_manager,
            font,
            style,
            glyph.glyph,
            skew,
            Layer::Glyph,
            left,
            baseline,
            size,
            color,
        )?;

        if glyph.style.bold {
            // 別の書体が無いので、少しずらした複製で太らせる。
            let weight = style.bold_weight * size;

            for (dx, dy) in [(weight, 0.0), (-weight, 0.0), (0.0, weight), (0.0, -weight)] {
                self.place_copy(
                    draw_manager,
                    font,
                    style,
                    glyph.glyph,
                    skew,
                    Layer::Glyph,
                    left + dx,
                    baseline + dy,
                    size,
                    color,
                )?;
            }
        }

        Ok(())
    }

    /// 同じグリフをもう 1 つ置く。
    #[allow(clippy::too_many_arguments)]
    fn place_copy(
        &mut self,
        draw_manager: &mut DrawManager,
        font: &Font,
        style: &TextStyle,
        glyph: GlyphId,
        skew: i32,
        layer: Layer,
        x: f32,
        y: f32,
        size: f32,
        color: [f32; 4],
    ) -> Result<(), Gueiz2DError> {
        let key = ShapeKey::Glyph { glyph, skew, layer };

        let Some(id) = self.glyph_shape(draw_manager, font, style, key)? else {
            // 空白のように形を持たないグリフ。送り幅だけで済んでいる。
            return Ok(());
        };

        let Some(object) = draw_manager.object_mut(id) else {
            return Ok(());
        };

        // 形は em 単位で登録してあるので、大きさはここで掛ける。
        object.instance(
            instance::create_instance()
                .translate(x, y, 0.0)
                .scale(size, size, 1.0)
                .color(color[0], color[1], color[2], color[3]),
        );

        Ok(())
    }

    /// 線 1 続きぶんを置く。[`LineDecoration::lines`] 本ぶん重ねる。
    fn place_line(
        &mut self,
        draw_manager: &mut DrawManager,
        line: &PlacedLine,
        origin_x: f32,
        origin_y: f32,
    ) -> Result<(), Gueiz2DError> {
        let mut color = line.color.unwrap_or(self.color);
        color[3] *= line.alpha_scale;

        let thickness = line.thickness();
        let left = origin_x + line.x;

        for index in 0..line.decoration.lines {
            // 何本あっても、まとまりの中心が本来の高さに来るようにする。
            let offset = (index as f32 - (line.decoration.lines as f32 - 1.0) / 2.0)
                * thickness
                * 2.0;
            let center = origin_y + line.y + offset;

            self.place_square(
                draw_manager,
                Layer::Decoration,
                left,
                center - thickness / 2.0,
                line.width,
                thickness,
                color,
            )?;

            // まっすぐな線なので角は無い。効くのは端だけ。
            if line.decoration.joint_type.caps_start() {
                self.place_disc(draw_manager, left, center, thickness, color)?;
            }

            if line.decoration.joint_type.caps_end() {
                self.place_disc(
                    draw_manager,
                    left + line.width,
                    center,
                    thickness,
                    color,
                )?;
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn place_square(
        &mut self,
        draw_manager: &mut DrawManager,
        layer: Layer,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    ) -> Result<(), Gueiz2DError> {
        let id = self.unit_shape(draw_manager, ShapeKey::Square(layer))?;

        let Some(object) = draw_manager.object_mut(id) else {
            return Ok(());
        };

        object.instance(
            instance::create_instance()
                .translate(x, y, 0.0)
                .scale(width, height, 1.0)
                .color(color[0], color[1], color[2], color[3]),
        );

        Ok(())
    }

    fn place_disc(
        &mut self,
        draw_manager: &mut DrawManager,
        x: f32,
        y: f32,
        diameter: f32,
        color: [f32; 4],
    ) -> Result<(), Gueiz2DError> {
        let id = self.unit_shape(draw_manager, ShapeKey::Disc(Layer::Decoration))?;

        let Some(object) = draw_manager.object_mut(id) else {
            return Ok(());
        };

        object.instance(
            instance::create_instance()
                .translate(x, y, 0.0)
                .scale(diameter, diameter, 1.0)
                .color(color[0], color[1], color[2], color[3]),
        );

        Ok(())
    }

    /// そのグリフの形。無ければ作って登録する。
    ///
    /// 形は **em 単位**（1.0 = 1 em）で登録する。大きさはインスタンス側で
    /// 掛けるので、同じ文字を別の大きさで出しても形は 1 つで済む。
    /// 傾きだけは形に焼き込む。インスタンスの変換に歪みが無いため。
    fn glyph_shape(
        &mut self,
        draw_manager: &mut DrawManager,
        font: &Font,
        style: &TextStyle,
        key: ShapeKey,
    ) -> Result<Option<&str>, Gueiz2DError> {
        // 先に「無ければ作る」を済ませてから引く。`get` した結果を
        // 返しつつ同じ `self` に入れる書き方は、借用が重なって通らない。
        if !self.shapes.contains_key(&key) {
            self.build_glyph_shape(draw_manager, font, style, key)?;
        }

        Ok(self
            .shapes
            .get(&key)
            .expect("いま入れた")
            .as_deref())
    }

    /// グリフの形を組んで登録する。[`TextRenderer::glyph_shape`] からだけ呼ぶ。
    fn build_glyph_shape(
        &mut self,
        draw_manager: &mut DrawManager,
        font: &Font,
        style: &TextStyle,
        key: ShapeKey,
    ) -> Result<(), Gueiz2DError> {
        let ShapeKey::Glyph { glyph, skew, layer } = key else {
            unreachable!("glyph_shape はグリフの鍵しか受け取らない");
        };

        let Some(outline) = font.outline(glyph, style.tolerance) else {
            self.shapes.insert(key, None);
            return Ok(());
        };

        // 名前は形の鍵から組む。登録先は名前をひとつに保つので、
        // ここで重ねると付け替えられて、当てにできなくなる。
        let mut object =
            object::create_object(&format!("Glyph {} skew{} {:?}", glyph.0, skew, layer));

        let skew = skew as f32 / SKEW_STEPS;
        object.begin(PaintType::Fill);

        for (index, point) in outline.points.iter().enumerate() {
            // 輪郭の切れ目で新しい輪郭を始める。外周か穴かは
            // テッセレータが包含関係で決めるので、ここでは区別しない。
            if index > 0 && outline.contour_starts.contains(&index) {
                object.begin_hole();
            }

            // 傾ける。y は下向きなので、字の上（y が負）ほど右へ寄る。
            object.put_vertex(Vertex::new_position_color(
                point[0] - skew * point[1],
                point[1],
                0.0,
                1.0,
                1.0,
                1.0,
                1.0,
            ));
        }

        object.end();
        object.camera(self.camera);
        object.z(self.z + layer.z_offset());

        let name = draw_manager.register(object);
        self.shapes.insert(key, Some(name));

        Ok(())
    }

    /// 正方形か円。大きさはインスタンスで決まるので、段ごとに 1 つで足りる。
    fn unit_shape(
        &mut self,
        draw_manager: &mut DrawManager,
        key: ShapeKey,
    ) -> Result<&str, Gueiz2DError> {
        if !self.shapes.contains_key(&key) {
            self.build_unit_shape(draw_manager, key)?;
        }

        Ok(self
            .shapes
            .get(&key)
            .expect("いま入れた")
            .as_deref()
            .expect("正方形と円は必ず形を持つ"))
    }

    /// 正方形か円を組んで登録する。[`TextRenderer::unit_shape`] からだけ呼ぶ。
    fn build_unit_shape(
        &mut self,
        draw_manager: &mut DrawManager,
        key: ShapeKey,
    ) -> Result<(), Gueiz2DError> {

        let (name, layer, points): (&str, Layer, Vec<[f32; 2]>) = match key {
            ShapeKey::Square(layer) => (
                "TextSquare",
                layer,
                vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            ),

            ShapeKey::Disc(layer) => (
                "TextDisc",
                layer,
                (0..CAP_SEGMENTS)
                    .map(|step| {
                        let angle =
                            std::f32::consts::TAU * step as f32 / CAP_SEGMENTS as f32;
                        [0.5 * angle.cos(), 0.5 * angle.sin()]
                    })
                    .collect(),
            ),

            ShapeKey::Glyph { .. } => unreachable!("unit_shape はグリフを受け取らない"),
        };

        // 段ごとに別の図形なので、名前にも段を入れる。
        let mut object = object::create_object(&format!("{name} {layer:?}"));
        object.begin(PaintType::Fill);

        for point in points {
            object.put_vertex(Vertex::new_position_color(
                point[0], point[1], 0.0, 1.0, 1.0, 1.0, 1.0,
            ));
        }

        object.end();
        object.camera(self.camera);
        object.z(self.z + layer.z_offset());

        let name = draw_manager.register(object);
        self.shapes.insert(key, Some(name));

        Ok(())
    }
}

/// 傾きを形の鍵に使える整数に丸める。
fn quantize_skew(italic: f32) -> i32 {
    (italic * SKEW_STEPS).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{DEFAULT_LINE_WIDTH, NamedColor};
    use crate::paint_type::JointType;

    /// フォントを読まずに並べ方を試すための、作り物の指標。
    ///
    /// どの字も同じ送り幅にしてあるので、期待値を手で書ける。
    /// タブだけは**形を持たない字**ということにして、飛ばされる側も試す。
    struct FakeFont;

    impl FakeFont {
        const ADVANCE: f32 = 0.5;
        const ASCENDER: f32 = 0.8;
        const DESCENDER: f32 = 0.2;
        const LINE_HEIGHT: f32 = 1.2;
        /// 1 行ぶんの高さ。
        const BODY: f32 = Self::ASCENDER + Self::DESCENDER;
    }

    impl Metrics for FakeFont {
        fn ascender(&self) -> f32 {
            Self::ASCENDER
        }

        fn descender(&self) -> f32 {
            Self::DESCENDER
        }

        fn line_height(&self) -> f32 {
            Self::LINE_HEIGHT
        }

        fn glyph(&self, character: char) -> Option<GlyphId> {
            (character != '\t').then_some(GlyphId(character as u16))
        }

        fn advance(&self, _glyph: GlyphId) -> f32 {
            Self::ADVANCE
        }

        fn kerning(&self, _left: GlyphId, _right: GlyphId) -> f32 {
            0.0
        }

        fn underline_position(&self) -> f32 {
            0.1
        }

        fn strikeout_position(&self) -> f32 {
            -0.26
        }

        fn overline_position(&self) -> f32 {
            -Self::ASCENDER
        }
    }

    /// 作り物の指標で並べる。
    fn place(text: &str, style: &TextStyle) -> TextLayout {
        place_tokens(&Formatted::plain(text), style)
    }

    /// 書式を読んでから並べる。
    fn place_formatted(text: &str, style: &TextStyle) -> TextLayout {
        place_tokens(&Formatted::parse(text).expect("読める"), style)
    }

    fn place_tokens(formatted: &Formatted, style: &TextStyle) -> TextLayout {
        let mut out = TextLayout::default();
        layout_with(&FakeFont, formatted, style, &mut out);

        out
    }

    /// 書式だけを積んで、結果の状態を見る。
    ///
    /// 並べる処理と同じ [`FormatStack`] を通すので、写しではなく本物を試している。
    fn state_after(codes: &str) -> FormatState {
        let formatted = Formatted::parse(codes).expect("読める");
        let mut stack = FormatStack::default();

        for token in formatted.tokens() {
            if let Token::Format(format) = token {
                stack.apply(*format);
            }
        }

        stack.state
    }

    #[test]
    fn the_default_style_is_sane() {
        let style = TextStyle::default();

        assert!(style.size > 0.0);
        assert!(style.kerning);
        assert_eq!(style.line_height, None, "既定はフォントの推奨値");
        assert_eq!(style.tolerance, DEFAULT_TOLERANCE);
        assert!(style.bold_weight > 0.0 && style.outline_width > 0.0);
    }

    #[test]
    fn a_style_keeps_the_size_it_was_given() {
        assert_eq!(TextStyle::new(48.0).size, 48.0);
    }

    /// 幅は「送り幅 + 字間」の積み上げ。
    #[test]
    fn the_width_adds_up_the_advances() {
        let mut style = TextStyle::new(32.0);

        assert_eq!(place("abcde", &style).width, 80.0);

        // 字間を足すと文字数ぶん伸びる。
        style.letter_spacing = 2.0;
        assert_eq!(place("abcde", &style).width, 90.0);
    }

    /// 高さは「行送り x (行数 - 1) + 1 行ぶんの高さ」。
    /// 行数を掛けてしまうと、最後の行の下に余白が付く。
    #[test]
    fn the_height_counts_the_gaps_not_the_lines() {
        let style = TextStyle::new(32.0);
        let one = FakeFont::BODY * 32.0;

        assert_eq!(place("ab", &style).height, one);
        assert_eq!(place("ab\ncd", &style).height, one + FakeFont::LINE_HEIGHT * 32.0);
    }

    /// 形を持たない字は飛ばすが、その前後は詰まらない。
    #[test]
    fn glyphless_characters_are_skipped() {
        let placed = place("a\tb", &TextStyle::new(32.0));

        assert_eq!(placed.glyphs.len(), 2);
        assert_eq!(placed.width, 32.0, "送り幅 2 文字ぶん");
    }

    /// `§[ln]` は `\n` と同じ働き。
    #[test]
    fn the_newline_format_breaks_a_row() {
        let style = TextStyle::new(32.0);

        assert_eq!(
            place_formatted("ab§[ln]cd", &style).rows.len(),
            place("ab\ncd", &style).rows.len(),
        );
        assert_eq!(place_formatted("ab§[ln]cd", &style).height, place("ab\ncd", &style).height);
    }

    // --- 大きさが混ざった行 ---

    /// 行の高さは、その行でいちばん大きい字が決める。
    /// 小さいほうに合わせると、大きい字が上にはみ出す。
    #[test]
    fn a_row_is_as_tall_as_its_largest_glyph() {
        let style = TextStyle::new(20.0);
        let mixed = place_formatted("a§[size 60]b", &style);

        assert_eq!(mixed.rows.len(), 1);
        assert_eq!(mixed.height, FakeFont::BODY * 60.0);
        assert_eq!(mixed.first_baseline, FakeFont::ASCENDER * 60.0);
    }

    /// 大きさが混ざっても、同じ行の字はベースラインが揃う。
    #[test]
    fn mixed_sizes_share_one_baseline() {
        let placed = place_formatted("a§[size 60]b§[/size]c", &TextStyle::new(20.0));

        let baselines: Vec<f32> = placed.glyphs.iter().map(|glyph| glyph.y).collect();

        assert_eq!(baselines.len(), 3);
        assert!(
            baselines.windows(2).all(|pair| pair[0] == pair[1]),
            "{:?}",
            baselines,
        );
    }

    /// 2 行目の上端は、1 行目の高さぶん下がる。
    #[test]
    fn a_tall_row_pushes_the_next_one_down() {
        let small = place_formatted("a§[ln]b", &TextStyle::new(20.0));
        let tall = place_formatted("§[size 60]a§[/size]§[ln]b", &TextStyle::new(20.0));

        assert!(tall.rows[1].top > small.rows[1].top);
    }

    /// 字が 1 つも無い行も 1 行ぶんの高さを取る。詰めると空行が消える。
    #[test]
    fn an_empty_row_still_takes_height() {
        let style = TextStyle::new(32.0);

        let with_gap = place("a\n\nb", &style);
        let without = place("a\nb", &style);

        assert_eq!(with_gap.rows.len(), 3);
        assert!(with_gap.height > without.height);
    }

    // --- 字間と行間 ---

    /// `spacex` は [`TextStyle::letter_spacing`] を**置き換える**。
    #[test]
    fn space_x_overrides_the_style() {
        let mut style = TextStyle::new(32.0);
        style.letter_spacing = 10.0;

        // 3 文字とも 10 px 広い。
        assert_eq!(place("abc", &style).width, 3.0 * (16.0 + 10.0));
        // 2 文字目から 0 px に。
        assert_eq!(
            place_formatted("a§[spacex 0]bc", &style).width,
            16.0 + 10.0 + 16.0 + 16.0,
        );
    }

    /// `spacey` は行送りそのもの。
    #[test]
    fn space_y_sets_the_line_pitch() {
        let placed = place_formatted("§[spacey 100]a§[ln]b", &TextStyle::new(32.0));

        assert_eq!(placed.rows[1].top, 100.0);
    }

    // --- 線 ---

    /// 下線は、書式が効いている範囲だけに引かれる。
    #[test]
    fn a_line_covers_only_the_run_it_was_asked_for() {
        let placed = place_formatted("ab§[underline]cd§[/underline]ef", &TextStyle::new(32.0));

        assert_eq!(placed.decorations.len(), 1);

        let line = placed.decorations[0];
        assert_eq!(line.position, LinePosition::Underline);
        assert_eq!(line.x, 32.0, "`c` の左端から");
        assert_eq!(line.width, 32.0, "`cd` の 2 文字ぶん");
    }

    /// 3 種類は別々に数える。
    #[test]
    fn the_three_line_positions_are_independent() {
        let placed =
            place_formatted("§[underline]§[strike]§[overline]ab", &TextStyle::new(32.0));

        let mut positions: Vec<LinePosition> =
            placed.decorations.iter().map(|line| line.position).collect();
        positions.sort_by_key(|position| format!("{:?}", position));

        assert_eq!(
            positions,
            vec![
                LinePosition::Overline,
                LinePosition::Strikethrough,
                LinePosition::Underline,
            ],
        );
    }

    /// 高さは位置ごとに違う。同じなら 3 本が重なってしまう。
    #[test]
    fn each_line_position_sits_at_its_own_height() {
        let placed =
            place_formatted("§[underline]§[strike]§[overline]ab", &TextStyle::new(32.0));

        let at = |position| {
            placed
                .decorations
                .iter()
                .find(|line| line.position == position)
                .expect("引かれている")
                .y
        };

        assert!(at(LinePosition::Overline) < at(LinePosition::Strikethrough));
        assert!(at(LinePosition::Strikethrough) < at(LinePosition::Underline));
    }

    /// 行をまたぐ線は行ごとに切れる。切らないと行間を横切る。
    #[test]
    fn a_line_is_cut_at_the_row_break() {
        let placed = place_formatted("§[underline]ab§[ln]cd", &TextStyle::new(32.0));

        assert_eq!(placed.decorations.len(), 2);
        assert!(placed.decorations[0].y < placed.decorations[1].y);
        assert_eq!(placed.decorations[1].x, 0.0, "2 行目は左端から");
    }

    /// 途中で色や太さが変われば、線も切れる。
    #[test]
    fn a_line_is_cut_when_it_changes() {
        let placed = place_formatted(
            "§[underline]ab§[underline 0.2]cd",
            &TextStyle::new(32.0),
        );

        assert_eq!(placed.decorations.len(), 2);
        assert_eq!(placed.decorations[0].decoration.line_width, DEFAULT_LINE_WIDTH);
        assert_eq!(placed.decorations[1].decoration.line_width, 0.2);
    }

    /// 付けてすぐ外した線は引かない。長さ 0 の線は描いても見えない。
    #[test]
    fn a_zero_length_line_is_dropped() {
        let placed = place_formatted("a§[underline]§[/underline]b", &TextStyle::new(32.0));

        assert!(placed.decorations.is_empty());
    }

    // --- 出鱈目な字 ---

    /// 置き換えても送り幅は元のまま。並びが動かないこと。
    #[test]
    fn obfuscation_keeps_the_layout_still() {
        let mut style = TextStyle::new(32.0);
        let plain = place("abcd", &style);

        style.obfuscation_seed = 12345;
        let scrambled = place_formatted("§[obfuscated]abcd", &style);

        assert_eq!(scrambled.width, plain.width);

        let plain_x: Vec<f32> = plain.glyphs.iter().map(|glyph| glyph.x).collect();
        let scrambled_x: Vec<f32> = scrambled.glyphs.iter().map(|glyph| glyph.x).collect();

        assert_eq!(plain_x, scrambled_x);
    }

    /// 種を変えれば出る字が変わる。変わらなければ「出鱈目」にならない。
    #[test]
    fn a_new_seed_gives_new_characters() {
        let mut style = TextStyle::new(32.0);
        let glyphs = |style: &TextStyle| -> Vec<GlyphId> {
            place_formatted("§[obfuscated]abcdefghij", style)
                .glyphs
                .iter()
                .map(|glyph| glyph.glyph)
                .collect()
        };

        style.obfuscation_seed = 1;
        let first = glyphs(&style);

        style.obfuscation_seed = 1;
        assert_eq!(glyphs(&style), first, "同じ種なら同じ");

        style.obfuscation_seed = 2;
        assert_ne!(glyphs(&style), first, "種が違えば変わる");
    }

    // --- 書式が残る ---

    /// 並べた結果に書式が付いてくること。描く側はこれだけを見る。
    #[test]
    fn the_resolved_style_rides_along_with_each_glyph() {
        let placed = place_formatted(
            "§[red]§[bold]§[ghost 0.25]§[italic 0.3]§[outline #000]a",
            &TextStyle::new(32.0),
        );

        let style = placed.glyphs[0].style;

        assert_eq!(style.color, Some(NamedColor::Red.rgba()));
        assert!(style.bold);
        assert_eq!(style.alpha_scale, 0.75);
        assert_eq!(style.italic, 0.3);
        assert_eq!(style.outline, Some([0.0, 0.0, 0.0, 1.0]));
    }

    /// 隠した字も並びには残る。幅が変わると周りがずれる。
    #[test]
    fn a_hidden_glyph_still_takes_its_place() {
        let style = TextStyle::new(32.0);
        let hidden = place_formatted("§[hidebox]abc", &style);

        assert_eq!(hidden.width, place("abc", &style).width);
        assert!(hidden.glyphs.iter().all(|glyph| glyph.style.hidden));
        assert!(hidden.glyphs.iter().all(|glyph| glyph.advance == 16.0));
    }

    /// 行ごとのグリフを引ける。行ぞろえを自分で計算するときに使う。
    #[test]
    fn glyphs_can_be_looked_up_by_row() {
        let placed = place("ab\ncde\n\nf", &TextStyle::new(32.0));

        assert_eq!(placed.rows.len(), 4);
        assert_eq!(placed.row_glyphs(0).len(), 2);
        assert_eq!(placed.row_glyphs(1).len(), 3);
        assert_eq!(placed.row_glyphs(2).len(), 0, "空行");
        assert_eq!(placed.row_glyphs(3).len(), 1);
    }

    // --- 大きさを測る ---

    /// 測った大きさと、並べた結果の大きさが一致すること。
    /// ずれたら、測ってから置いた図がずれる。
    #[test]
    fn measuring_agrees_with_laying_out() {
        let style = TextStyle::new(32.0);
        let placed = place_formatted("§[italic 0.3]ab§[ln]§[/]cde", &style);
        let size = placed.size();

        assert_eq!(size.width, placed.width);
        assert_eq!(size.height, placed.height);
        assert_eq!(size.ink, placed.ink);
        assert_eq!(size.rows, placed.rows.len());
        assert_eq!(size.first_baseline, placed.first_baseline);
    }

    /// 何も付いていなければ、送り幅の箱と塗られる箱は同じ大きさ。
    #[test]
    fn a_plain_size_has_one_answer() {
        let size = place("abcde", &TextStyle::new(32.0)).size();

        assert_eq!(size.width, 80.0);
        assert_eq!(size.ink_width(), size.width);
        assert_eq!(size.ink_height(), size.height);
        assert_eq!(size.to_array(), [size.width, size.height]);
    }

    /// 斜体・影が付けば、塗られる箱のほうが広い。
    /// ここが同じに戻ったら、測っても切れるようになっている。
    #[test]
    fn decorated_text_paints_wider_than_it_advances() {
        let style = TextStyle::new(32.0);

        for codes in ["§[italic 0.4]abc", "§[shadow 0.3 0]abc", "§[outline #000]abc"] {
            let size = place_formatted(codes, &style).size();

            assert!(
                size.ink_width() > size.width,
                "{codes}: 塗る幅 {} <= 送り幅 {}",
                size.ink_width(),
                size.width,
            );
        }
    }

    #[test]
    fn a_multi_row_size_counts_every_row() {
        let size = place("a\nbb\n\nc", &TextStyle::new(32.0)).size();

        assert_eq!(size.rows, 4, "空行も 1 行");
        assert_eq!(size.width, 32.0, "いちばん長い行は 2 文字");
        assert_eq!(size.first_baseline, FakeFont::ASCENDER * 32.0);
    }

    /// 塗られる範囲を描く場所へ動かせること。寄せるときに使う。
    #[test]
    fn the_painted_box_can_be_moved_to_where_it_is_drawn() {
        let size = place_formatted("§[italic 0.4]abc", &TextStyle::new(32.0)).size();
        let on_screen = size.ink.translated(100.0, 50.0);

        assert_eq!(on_screen.left, size.ink.left + 100.0);
        assert_eq!(on_screen.right, size.ink.right + 100.0);
        assert_eq!(on_screen.top, size.ink.top + 50.0);
        assert_eq!(on_screen.bottom, size.ink.bottom + 50.0);
        assert_eq!(on_screen.width(), size.ink_width());
    }

    /// 詰め直しても前の中身が残らないこと。残ると幅がじわじわ伸びる。
    #[test]
    fn refilling_a_layout_does_not_keep_the_old_contents() {
        let style = TextStyle::new(32.0);
        let mut reused = TextLayout::default();

        layout_with(&FakeFont, &Formatted::plain("abcdefgh"), &style, &mut reused);
        let long = reused.size();

        layout_with(&FakeFont, &Formatted::plain("ab"), &style, &mut reused);

        assert_eq!(reused.glyphs.len(), 2);
        assert_eq!(reused.rows.len(), 1);
        assert!(reused.width < long.width, "幅が残っている");
        assert_eq!(reused.size(), place("ab", &style).size());
    }

    /// 詰め直しは確保し直さない。毎フレーム測るときに効く。
    #[test]
    fn refilling_reuses_the_space_it_already_has() {
        let style = TextStyle::new(32.0);
        let mut reused = TextLayout::default();

        layout_with(&FakeFont, &Formatted::plain("abcdefghij"), &style, &mut reused);
        let capacity = reused.glyphs.capacity();

        layout_with(&FakeFont, &Formatted::plain("klmnopqrst"), &style, &mut reused);

        assert_eq!(reused.glyphs.capacity(), capacity);
    }

    // --- 塗られる範囲 ---

    /// 何もはみ出さなければ、塗られる範囲は送り幅の箱と同じ。
    #[test]
    fn plain_text_paints_exactly_its_advance_box() {
        let placed = place("abc", &TextStyle::new(32.0));

        // 並べた結果の原点は左上なので、はみ出しが無ければ 0 から始まる。
        assert_eq!(placed.ink.left, 0.0);
        assert_eq!(placed.ink.right, placed.width);
        assert_eq!(placed.ink.top, 0.0);
        assert_eq!(placed.ink.bottom, placed.height);
    }

    /// 斜体は上が右にはみ出す。送り幅に入らないので、
    /// ここを見ないと右端で切れる。
    #[test]
    fn italic_leans_past_the_advance_box() {
        let style = TextStyle::new(32.0);
        let upright = place("abc", &style);
        let slanted = place_formatted("§[italic 0.4]abc", &style);

        assert_eq!(slanted.width, upright.width, "送り幅は変わらない");
        assert!(
            slanted.ink.right > upright.ink.right,
            "上が右に出るはず: {} <= {}",
            slanted.ink.right,
            upright.ink.right,
        );
        // 傾き x アセンダ ぶん。
        assert!(
            (slanted.ink.right - upright.ink.right - 0.4 * FakeFont::ASCENDER * 32.0).abs()
                < 0.01,
        );
        // 下は左に出る。
        assert!(slanted.ink.left < 0.0);
    }

    /// 傾きが逆なら、はみ出す向きも逆。
    #[test]
    fn a_backwards_lean_overhangs_the_other_way() {
        let style = TextStyle::new(32.0);
        let forward = place_formatted("§[italic 0.4]abc", &style);
        let backward = place_formatted("§[italic -0.4]abc", &style);

        assert!(backward.ink.left < forward.ink.left);
        assert!(backward.ink.right < forward.ink.right);
    }

    /// 影はずらした向きにだけ広がる。元の位置にも字があるので、
    /// 反対側は広がらない。
    #[test]
    fn a_shadow_only_grows_the_side_it_falls_on() {
        let style = TextStyle::new(32.0);
        let plain = place("abc", &style);
        // 角度 0 は真右。
        let shadow = place_formatted("§[shadow 0.3 0]abc", &style);

        assert_eq!(shadow.ink.left, plain.ink.left, "左には広がらない");
        assert!((shadow.ink.right - plain.ink.right - 0.3 * 32.0).abs() < 0.01);
    }

    /// 縁取りは四方に広がる。
    #[test]
    fn an_outline_grows_every_side() {
        let mut style = TextStyle::new(32.0);
        style.outline_width = 0.05;

        let plain = place("abc", &style);
        let outlined = place_formatted("§[outline #000]abc", &style);
        let radius = 0.05 * 32.0;

        assert!((outlined.ink.left - (plain.ink.left - radius)).abs() < 0.01);
        assert!((outlined.ink.right - (plain.ink.right + radius)).abs() < 0.01);
        assert!((outlined.ink.top - (plain.ink.top - radius)).abs() < 0.01);
        assert!((outlined.ink.bottom - (plain.ink.bottom + radius)).abs() < 0.01);
    }

    /// 下線は字の下に出る。塗られる範囲がそのぶん下に伸びること。
    #[test]
    fn a_line_grows_the_painted_box() {
        let style = TextStyle::new(32.0);
        let plain = place("abc", &style);
        let underlined = place_formatted("§[underline 0.1 miter 3]abc", &style);

        assert!(underlined.ink.bottom > plain.ink.bottom);
    }

    /// 丸い端は線の外に出る。端を閉じないなら出ない。
    #[test]
    fn round_caps_stick_out_but_square_ones_do_not() {
        let style = TextStyle::new(32.0);
        let square = place_formatted("§[underline 0.1 miter 1]abc", &style);
        let round = place_formatted("§[underline 0.1 round_start_end 1]abc", &style);

        assert!(round.ink.left < square.ink.left);
        assert!(round.ink.right > square.ink.right);
    }

    /// 複数行なら、全部の行を囲む。
    #[test]
    fn the_painted_box_covers_every_row() {
        let placed = place_formatted("§[italic 0.4]a§[ln]§[/]bbbb", &TextStyle::new(32.0));

        assert_eq!(placed.rows.len(), 2);
        assert!(placed.ink.left <= placed.rows[0].ink.left.min(placed.rows[1].ink.left));
        assert!(placed.ink.right >= placed.rows[0].ink.right.max(placed.rows[1].ink.right));
        assert!(placed.ink.bottom >= placed.rows[1].ink.bottom);
    }

    /// 行ごとの範囲はベースラインぶん下がっている。
    #[test]
    fn each_rows_box_sits_at_its_own_baseline() {
        let placed = place("a\nb", &TextStyle::new(32.0));

        assert!(placed.rows[1].ink.top > placed.rows[0].ink.top);
        assert_eq!(
            placed.rows[0].ink.top,
            placed.rows[0].baseline - FakeFont::ASCENDER * 32.0,
        );
    }

    /// 太字は送り幅も広がる。広げないと字がくっつく。
    #[test]
    fn bold_widens_the_advance() {
        let style = TextStyle::new(32.0);

        assert!(place_formatted("§[bold]abc", &style).width > place("abc", &style).width);
    }

    #[test]
    fn a_fresh_renderer_has_no_shapes() {
        let renderer = TextRenderer::new();

        assert_eq!(renderer.shape_count(), 0);
    }

    #[test]
    fn the_colour_and_depth_are_remembered() {
        let mut renderer = TextRenderer::new();
        renderer.color(1.0, 0.0, 0.0, 0.5).z(3.0);

        assert_eq!(renderer.color, [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(renderer.z, 3.0);
    }

    // --- 書式の積み上げ ---

    #[test]
    fn formats_stack_up() {
        let state = state_after("§[red]§[bold]§[size 48]§[italic 0.3]");

        assert_eq!(state.color, Some(NamedColor::Red.rgba()));
        assert!(state.bold);
        assert_eq!(state.size, Some(48.0));
        assert_eq!(state.italic, 0.3);
    }

    /// `/名前` はその書式だけを戻す。他は積まれたまま。
    #[test]
    fn a_targeted_reset_leaves_the_rest_alone() {
        let state = state_after("§[red]§[bold]§[/bold]");

        assert_eq!(state.color, Some(NamedColor::Red.rgba()), "色は残る");
        assert!(!state.bold, "太字だけ戻る");
    }

    #[test]
    fn a_full_reset_clears_everything() {
        let state = state_after("§[red]§[bold]§[underline]§[shadow]§[/]");

        assert_eq!(state, FormatState::default());
    }

    /// `DefaultColor` は「色を指定しない」に戻す。黒にするのではない。
    #[test]
    fn default_colour_goes_back_to_the_callers_colour() {
        let state = state_after("§[red]§[defaultcolor]");

        assert_eq!(state.color, None);
    }

    /// 退避して素に戻り、戻すと積み上げが復活する。
    #[test]
    fn skip_push_and_pop_save_the_stack() {
        let state = state_after("§[red]§[bold]§[skippush]");
        assert_eq!(state, FormatState::default(), "退避中は素のまま");

        let state = state_after("§[red]§[bold]§[skippush]§[italic]§[skippop]");
        assert_eq!(state.color, Some(NamedColor::Red.rgba()));
        assert!(state.bold);
        assert_eq!(state.italic, 0.0, "退避中に積んだぶんは捨てる");
    }

    /// 退避していないのに戻しても落ちない。
    #[test]
    fn an_unmatched_pop_is_harmless() {
        let state = state_after("§[red]§[skippop]");

        assert_eq!(state.color, Some(NamedColor::Red.rgba()));
    }

    #[test]
    fn skip_can_nest() {
        let state = state_after(
            "§[red]§[skippush]§[bold]§[skippush]§[italic]§[skippop]§[skippop]",
        );

        assert_eq!(state.color, Some(NamedColor::Red.rgba()));
        assert!(!state.bold);
    }

    // --- 出鱈目な字 ---

    /// 同じ種なら同じ結果。並べ直しても暴れない。
    #[test]
    fn scrambling_is_repeatable() {
        let first = scramble_bits(7, 3);

        assert_eq!(first, scramble_bits(7, 3));
        assert_ne!(first, scramble_bits(8, 3), "種が違えば変わる");
        assert_ne!(first, scramble_bits(7, 4), "位置が違えば変わる");
    }

    /// 候補は広く散らばること。偏ると「出鱈目」に見えない。
    #[test]
    fn scrambling_covers_the_pool() {
        let mut seen = vec![false; OBFUSCATION_POOL.len()];

        for index in 0..10_000 {
            seen[scramble_bits(1, index) as usize % OBFUSCATION_POOL.len()] = true;
        }

        assert!(seen.iter().all(|hit| *hit), "使われない候補がある");
    }

    // --- 形の鍵 ---

    /// 傾きが違えば別の形、同じなら同じ形。丸めが効いていること。
    #[test]
    fn the_skew_is_rounded_into_the_key() {
        assert_eq!(quantize_skew(0.0), 0);
        assert_eq!(quantize_skew(0.2), quantize_skew(0.2 + 0.0001));
        assert_ne!(quantize_skew(0.2), quantize_skew(0.3));
    }

    /// 段ごとに別の形が要る。`z` が [`Object`] にしか持てないため。
    #[test]
    fn each_layer_is_a_separate_shape() {
        let glyph = GlyphId(1);

        assert_ne!(
            ShapeKey::Glyph { glyph, skew: 0, layer: Layer::Glyph },
            ShapeKey::Glyph { glyph, skew: 0, layer: Layer::Shadow },
        );
        assert_eq!(
            ShapeKey::Glyph { glyph, skew: 0, layer: Layer::Glyph },
            ShapeKey::Glyph { glyph, skew: 0, layer: Layer::Glyph },
        );
    }

    /// 重なり順は「影 → 縁取り → 字 → 線 → 隠し箱」。
    #[test]
    fn the_layers_are_ordered_back_to_front() {
        let order = [
            Layer::Shadow,
            Layer::Outline,
            Layer::Glyph,
            Layer::Decoration,
            Layer::HideBox,
        ];

        for pair in order.windows(2) {
            assert!(
                pair[0].z_offset() < pair[1].z_offset(),
                "{:?} は {:?} より奥のはず",
                pair[0],
                pair[1],
            );
        }
    }

    /// ずらし幅は小さいこと。大きいと他の図形との前後が入れ替わる。
    #[test]
    fn the_layer_offsets_are_small() {
        for layer in [Layer::Shadow, Layer::Outline, Layer::Decoration, Layer::HideBox] {
            assert!(layer.z_offset().abs() < 0.01, "{:?}", layer);
        }
    }

    // --- 線 ---

    #[test]
    fn a_lines_thickness_follows_the_size() {
        let line = PlacedLine {
            position: LinePosition::Underline,
            x: 0.0,
            width: 100.0,
            y: 0.0,
            size: 40.0,
            decoration: LineDecoration::new(0.05, JointType::Miter, 1),
            color: None,
            alpha_scale: 1.0,
        };

        assert_eq!(line.thickness(), 2.0);
    }

    /// まっすぐな線に角は無いので、端を閉じる指定だけが効く。
    #[test]
    fn only_the_cap_part_of_a_joint_matters() {
        assert!(!JointType::Round.caps_start() && !JointType::Round.caps_end());
        assert!(JointType::RoundStartEnd.caps_start() && JointType::RoundStartEnd.caps_end());
        assert!(JointType::RoundStart.caps_start() && !JointType::RoundStart.caps_end());
    }
}
