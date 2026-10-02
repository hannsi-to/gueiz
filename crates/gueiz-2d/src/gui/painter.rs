//! 描くものを**記録する**。GPU は触りません。
//!
//! # なぜその場で描かないのか
//!
//! ウィジェットが [`DrawManager`](crate::draw_manager::DrawManager) を直に
//! 触ると、次の 3 つが同時に壊れます。
//!
//! 1. **ウィジェットが GPU を知ることになる。** 見た目を書くたびに
//!    [`Object`](crate::object::Object) の寿命と名前の管理が付いてきます。
//! 2. **順番が決まらない。** 奥から手前へ描く順は木を辿った順で決まるので、
//!    ウィジェット自身には分かりません。
//! 3. **差分が取れない。** 何が変わったかを知るには、前のフレームに
//!    何を描いたかが残っていなければなりません。
//!
//! そこで [`Painter`] は [`DisplayList`] に**命令を並べるだけ**にして、
//! GPU へ渡す仕事は [`crate::gui::render`] に預けます。
//! 並んだ順がそのまま奥から手前の順です。
//!
//! # 座標はウィジェットの左上から
//!
//! [`Painter`] は自分が誰のものかを知っていて、受け取った座標に
//! そのウィジェットの画面上の位置を足します。**ウィジェットは
//! 自分がどこに置かれたか気にせず、`(0, 0)` を自分の左上として書けます。**
//!
//! # 切り抜きは積んだ結果を焼き付ける
//!
//! [`Painter::with_clip`] の中で記録した命令には、そのときの切り抜きが
//! 1 つずつ写し取られます。[`crate::gui::render`] が積み直す必要はありません。
//!
//! 重ねた切り抜きは**矩形だけを掛け合わせ、丸みは内側のものを使います**。
//! 丸い矩形どうしの共通部分は丸い矩形になりませんが、UI でそこまで必要に
//! なることはまずありません。

use crate::gui::color::Color;
use crate::gui::geometry::{Align, Corners, Point, Rect, Size};
use crate::gui::id::WidgetId;

/// 切り抜く範囲。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct ClipRegion {
    pub rect: Rect,
    pub corners: Corners,
}

impl ClipRegion {
    pub fn new(rect: Rect, corners: Corners) -> Self {
        Self { rect, corners }
    }

    /// 重ねる。矩形は掛け合わせ、丸みは内側（`other`）のものを使う。
    pub fn intersect(self, other: Self) -> Self {
        Self {
            rect: self.rect.intersect(other.rect),
            corners: other.corners,
        }
    }

    /// 何も残らないか。
    pub fn is_empty(self) -> bool {
        self.rect.is_empty()
    }
}

/// 文字をどう置くか。**書体の選び方や色はここではありません。**
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct TextLayoutOptions {
    /// 1 em の大きさ。ピクセル。
    pub size: f32,
    /// 横の寄せ。
    pub horizontal: Align,
    /// 縦の寄せ。矩形の高さに対して。
    pub vertical: Align,
    /// 矩形の幅で折り返すか。
    pub wrap: bool,
}

impl Default for TextLayoutOptions {
    fn default() -> Self {
        Self {
            size: 16.0,
            horizontal: Align::Start,
            vertical: Align::Center,
            wrap: false,
        }
    }
}

impl TextLayoutOptions {
    pub fn new(size: f32) -> Self {
        Self {
            size,
            ..Default::default()
        }
    }
}

/// 絵の差し場所。
///
/// [`crate::atlas::TextureRegion`] をそのまま持たないのは、
/// 絵を積む側（[`crate::resource`]）と描く側を同じフレームで
/// 揃えなくて済むようにするためです。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct ImageSource {
    /// 配列テクスチャの層。
    pub page: u32,
    /// 切り出し範囲。`[u, v, 幅, 高さ]`、いずれも 0..1。
    pub uv_rect: [f32; 4],
}

impl ImageSource {
    /// 層まるごと。
    pub fn whole_page(page: u32) -> Self {
        Self {
            page,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

impl From<crate::atlas::TextureRegion> for ImageSource {
    fn from(value: crate::atlas::TextureRegion) -> Self {
        Self {
            page: value.page,
            uv_rect: value.uv_rect,
        }
    }
}

/// 描くもの 1 つ。座標は**画面の左上を原点とした論理ピクセル**。
#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum Primitive {
    /// 塗った矩形。`corners` が 0 でなければ角が丸い。
    Rect {
        rect: Rect,
        corners: Corners,
        color: Color,
    },
    /// 枠線だけ。線は矩形の**内側**に引く。
    Border {
        rect: Rect,
        corners: Corners,
        width: f32,
        color: Color,
    },
    /// 矩形に内接する楕円。
    Ellipse { rect: Rect, color: Color },
    /// 折れ線。`closed` なら最後と最初をつなぐ。
    Polyline {
        points: Vec<Point>,
        width: f32,
        color: Color,
        closed: bool,
    },
    /// 文字。`rect` の中に [`TextLayoutOptions`] に従って置く。
    Text {
        rect: Rect,
        text: String,
        options: TextLayoutOptions,
        color: Color,
    },
    /// 絵。
    Image {
        rect: Rect,
        source: ImageSource,
        /// 絵の色に掛ける。白なら素のまま。
        tint: Color,
    },
}

impl Primitive {
    /// 塗られる範囲。[`Primitive::Border`] は内側に引くので矩形そのまま。
    pub fn bounds(&self) -> Rect {
        match self {
            Self::Rect { rect, .. }
            | Self::Border { rect, .. }
            | Self::Ellipse { rect, .. }
            | Self::Text { rect, .. }
            | Self::Image { rect, .. } => *rect,

            Self::Polyline {
                points, width, ..
            } => {
                let width = *width;
                let half = width / 2.0;

                points
                    .iter()
                    .fold(None::<Rect>, |bounds, point| {
                        let dot = Rect::new(
                            point.x - half,
                            point.y - half,
                            width,
                            width,
                        );

                        Some(match bounds {
                            Some(bounds) => bounds.union(dot),
                            None => dot,
                        })
                    })
                    .unwrap_or(Rect::ZERO)
            }
        }
    }

    /// 見えないか。空の矩形か透明なら描かなくてよい。
    pub fn is_invisible(&self) -> bool {
        match self {
            Self::Rect { rect, color, .. } | Self::Ellipse { rect, color } => {
                rect.is_empty() || color.is_invisible()
            }

            Self::Border {
                rect,
                width,
                color,
                ..
            } => rect.is_empty() || *width <= 0.0 || color.is_invisible(),

            Self::Polyline {
                points,
                width,
                color,
                ..
            } => points.len() < 2 || *width <= 0.0 || color.is_invisible(),

            Self::Text {
                rect,
                text,
                color,
                options,
            } => rect.is_empty() || text.is_empty() || color.is_invisible() || options.size <= 0.0,

            Self::Image { rect, tint, .. } => rect.is_empty() || tint.is_invisible(),
        }
    }
}

/// 記録された命令 1 つ。
#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct DrawCommand {
    /// 誰が出したか。[`crate::gui::render`] がこれで図形を使い回す。
    pub owner: WidgetId,
    /// 記録した時点の切り抜き。`None` なら切り抜かない。
    pub clip: Option<ClipRegion>,
    pub primitive: Primitive,
}

/// 1 フレームぶんの命令。**並んだ順が奥から手前。**
#[derive(Clone)]
#[derive(Debug, Default)]
pub struct DisplayList {
    commands: Vec<DrawCommand>,
}

impl DisplayList {
    pub fn new() -> Self {
        Self::default()
    }

    /// 中身を空にする。確保した領域は残すので、毎フレーム詰め直しても
    /// 割り当てが起きない。
    pub fn clear(&mut self) {
        self.commands.clear();
    }

    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// そのウィジェットが出した命令。[`crate::gui::render`] の差分取りに使う。
    pub fn commands_of(&self, owner: WidgetId) -> impl Iterator<Item = &DrawCommand> {
        self.commands
            .iter()
            .filter(move |command| command.owner == owner)
    }

    /// そのウィジェット用の記録器を作る。
    ///
    /// `origin` はウィジェットの**画面上の**左上。`clip` は親から受け継いだ
    /// 切り抜き。[`crate::gui::Gui`] が描画の周回で呼びます。
    pub fn painter(
        &mut self,
        owner: WidgetId,
        origin: Point,
        clip: Option<ClipRegion>,
    ) -> Painter<'_> {
        Painter {
            list: self,
            owner,
            origin,
            clip,
        }
    }

    fn push(&mut self, command: DrawCommand) {
        // 見えないものは入れない。後ろの段で弾くより、ここで落とすほうが安い。
        if command.primitive.is_invisible() {
            return;
        }

        // 切り抜きの外に出きっているものも入れない。
        //
        // **これが無いと、長い一覧が丸ごと記録されます。** GPU は切り抜いて
        // くれるので絵は正しく出ますが、画面に 10 行しか見えない 1000 行の
        // 一覧で、1000 行ぶんの図形を作って毎フレーム送ることになります。
        if let Some(clip) = command.clip
            && (clip.is_empty() || !command.primitive.bounds().overlaps(clip.rect))
        {
            return;
        }

        self.commands.push(command);
    }
}

/// 命令を並べる道具。
///
/// 座標は**ウィジェットの左上を原点**として渡してください。
/// 画面上の位置へは [`Painter`] が直します。
pub struct Painter<'a> {
    list: &'a mut DisplayList,
    owner: WidgetId,
    origin: Point,
    clip: Option<ClipRegion>,
}

impl Painter<'_> {
    /// このウィジェットの画面上の左上。
    pub fn origin(&self) -> Point {
        self.origin
    }

    /// いま効いている切り抜き。画面座標。
    pub fn clip(&self) -> Option<ClipRegion> {
        self.clip
    }

    /// 誰として記録しているか。
    pub fn owner(&self) -> WidgetId {
        self.owner
    }

    /// 局所座標を画面座標に直す。
    pub fn to_screen(&self, point: Point) -> Point {
        point + self.origin
    }

    /// 局所矩形を画面矩形に直す。
    pub fn to_screen_rect(&self, rect: Rect) -> Rect {
        rect.translate(self.origin)
    }

    /// 塗った矩形。
    pub fn rect(&mut self, rect: Rect, color: Color) {
        self.rounded_rect(rect, Corners::ZERO, color);
    }

    /// 角の丸い塗った矩形。
    pub fn rounded_rect(&mut self, rect: Rect, corners: Corners, color: Color) {
        let rect = self.to_screen_rect(rect);

        self.emit(Primitive::Rect {
            rect,
            corners: corners.clamp_to(rect.size()),
            color,
        });
    }

    /// 枠線。線は矩形の内側に引く。
    pub fn border(&mut self, rect: Rect, corners: Corners, width: f32, color: Color) {
        let rect = self.to_screen_rect(rect);

        self.emit(Primitive::Border {
            rect,
            corners: corners.clamp_to(rect.size()),
            width,
            color,
        });
    }

    /// 矩形に内接する楕円。真円にしたいなら正方形を渡す。
    pub fn ellipse(&mut self, rect: Rect, color: Color) {
        let rect = self.to_screen_rect(rect);
        self.emit(Primitive::Ellipse { rect, color });
    }

    /// 中心と半径で円。
    pub fn circle(&mut self, center: Point, radius: f32, color: Color) {
        self.ellipse(
            Rect::from_origin_size(
                Point::new(center.x - radius, center.y - radius),
                Size::splat(radius * 2.0),
            ),
            color,
        );
    }

    /// 1 本の線。
    pub fn line(&mut self, from: Point, to: Point, width: f32, color: Color) {
        self.polyline(&[from, to], width, color, false);
    }

    /// 折れ線。
    pub fn polyline(&mut self, points: &[Point], width: f32, color: Color, closed: bool) {
        let points = points.iter().map(|point| self.to_screen(*point)).collect();

        self.emit(Primitive::Polyline {
            points,
            width,
            color,
            closed,
        });
    }

    /// 文字。`rect` の中に収める。
    pub fn text(
        &mut self,
        rect: Rect,
        text: impl Into<String>,
        options: TextLayoutOptions,
        color: Color,
    ) {
        let rect = self.to_screen_rect(rect);

        self.emit(Primitive::Text {
            rect,
            text: text.into(),
            options,
            color,
        });
    }

    /// 絵。
    pub fn image(&mut self, rect: Rect, source: ImageSource, tint: Color) {
        let rect = self.to_screen_rect(rect);

        self.emit(Primitive::Image {
            rect,
            source,
            tint,
        });
    }

    /// `rect` の外へ出たぶんを切り落として記録する。
    ///
    /// 入れ子にすると掛け合わされます。中身が何も見えなくなった時点で
    /// 記録は捨てられるので、スクロール領域の外の行を描いても無駄になりません。
    ///
    /// ```no_run
    /// # use gueiz_2d::gui::color::Color;
    /// # use gueiz_2d::gui::geometry::{Corners, Rect};
    /// # use gueiz_2d::gui::painter::Painter;
    /// # fn paint(painter: &mut Painter) {
    /// let view = Rect::new(0.0, 0.0, 200.0, 100.0);
    ///
    /// painter.with_clip(view, Corners::all(8.0), |painter| {
    ///     // はみ出すぶんは勝手に落ちる。
    ///     painter.rect(Rect::new(0.0, -50.0, 200.0, 400.0), Color::WHITE);
    /// });
    /// # }
    /// ```
    pub fn with_clip(
        &mut self,
        rect: Rect,
        corners: Corners,
        paint: impl FnOnce(&mut Painter<'_>),
    ) {
        let rect = self.to_screen_rect(rect);
        let region = ClipRegion::new(rect, corners.clamp_to(rect.size()));

        let clip = match self.clip {
            Some(outer) => outer.intersect(region),
            None => region,
        };

        let mut inner = Painter {
            list: self.list,
            owner: self.owner,
            origin: self.origin,
            clip: Some(clip),
        };

        paint(&mut inner);
    }

    /// 原点をずらして記録する。スクロールの送り量を当てるときなど。
    pub fn with_offset(&mut self, offset: Point, paint: impl FnOnce(&mut Painter<'_>)) {
        let mut inner = Painter {
            list: self.list,
            owner: self.owner,
            origin: self.origin + offset,
            clip: self.clip,
        };

        paint(&mut inner);
    }

    fn emit(&mut self, primitive: Primitive) {
        self.list.push(DrawCommand {
            owner: self.owner,
            clip: self.clip,
            primitive,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> DisplayList {
        DisplayList::new()
    }

    #[test]
    fn local_coordinates_become_screen_coordinates() {
        let mut list = list();
        let mut painter = list.painter(WidgetId::NONE, Point::new(100.0, 50.0), None);

        painter.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE);

        let Primitive::Rect { rect, .. } = &list.commands()[0].primitive else {
            panic!("expected a rect");
        };

        assert_eq!(*rect, Rect::new(100.0, 50.0, 10.0, 10.0));
    }

    #[test]
    fn invisible_commands_are_dropped() {
        let mut list = list();
        let mut painter = list.painter(WidgetId::NONE, Point::ZERO, None);

        painter.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::TRANSPARENT);
        painter.rect(Rect::ZERO, Color::WHITE);
        painter.line(Point::ZERO, Point::new(10.0, 0.0), 0.0, Color::WHITE);
        painter.text(Rect::new(0.0, 0.0, 10.0, 10.0), "", TextLayoutOptions::new(16.0), Color::WHITE);

        assert!(list.is_empty(), "{:?}", list.commands());
    }

    #[test]
    fn clip_is_baked_into_each_command() {
        let mut list = list();
        let mut painter = list.painter(WidgetId::NONE, Point::ZERO, None);

        painter.with_clip(Rect::new(0.0, 0.0, 50.0, 50.0), Corners::ZERO, |painter| {
            painter.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE);
        });
        painter.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE);

        assert_eq!(list.len(), 2);
        assert!(list.commands()[0].clip.is_some(), "切り抜きの中");
        assert!(list.commands()[1].clip.is_none(), "抜けたら戻る");
    }

    #[test]
    fn nested_clips_multiply() {
        let mut list = list();
        let mut painter = list.painter(WidgetId::NONE, Point::ZERO, None);

        painter.with_clip(Rect::new(0.0, 0.0, 100.0, 100.0), Corners::ZERO, |painter| {
            painter.with_clip(Rect::new(50.0, 50.0, 100.0, 100.0), Corners::ZERO, |painter| {
                painter.rect(Rect::new(0.0, 0.0, 200.0, 200.0), Color::WHITE);
            });
        });

        let clip = list.commands()[0].clip.expect("clipped");
        assert_eq!(clip.rect, Rect::new(50.0, 50.0, 50.0, 50.0));
    }

    #[test]
    fn commands_outside_the_clip_are_dropped() {
        let mut list = list();
        let mut painter = list.painter(WidgetId::NONE, Point::ZERO, None);

        painter.with_clip(Rect::new(0.0, 0.0, 100.0, 100.0), Corners::ZERO, |painter| {
            // 枠の中。残る。
            painter.rect(Rect::new(10.0, 10.0, 20.0, 20.0), Color::WHITE);
            // 半分だけ出ている。残る。
            painter.rect(Rect::new(90.0, 10.0, 40.0, 20.0), Color::WHITE);
            // 完全に外。落ちる。
            painter.rect(Rect::new(0.0, 500.0, 20.0, 20.0), Color::WHITE);
            painter.rect(Rect::new(-300.0, 0.0, 20.0, 20.0), Color::WHITE);
        });

        assert_eq!(list.len(), 2, "{:?}", list.commands());
    }

    #[test]
    fn without_a_clip_nothing_is_culled() {
        let mut list = list();
        let mut painter = list.painter(WidgetId::NONE, Point::ZERO, None);

        // 切り抜いていないなら、画面の外も記録する。どこまでが画面かは
        // `Painter` の知るところではない。
        painter.rect(Rect::new(0.0, 9999.0, 10.0, 10.0), Color::WHITE);

        assert_eq!(list.len(), 1);
    }

    #[test]
    fn fully_clipped_commands_are_dropped() {
        let mut list = list();
        let mut painter = list.painter(WidgetId::NONE, Point::ZERO, None);

        painter.with_clip(Rect::new(0.0, 0.0, 10.0, 10.0), Corners::ZERO, |painter| {
            painter.with_clip(Rect::new(500.0, 500.0, 10.0, 10.0), Corners::ZERO, |painter| {
                painter.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE);
            });
        });

        assert!(list.is_empty(), "重ならない切り抜きは何も残さない");
    }

    #[test]
    fn offset_shifts_without_clipping() {
        let mut list = list();
        let mut painter = list.painter(WidgetId::NONE, Point::new(10.0, 10.0), None);

        painter.with_offset(Point::new(0.0, -30.0), |painter| {
            painter.rect(Rect::new(0.0, 0.0, 5.0, 5.0), Color::WHITE);
        });

        assert_eq!(list.commands()[0].primitive.bounds(), Rect::new(10.0, -20.0, 5.0, 5.0));
    }

    #[test]
    fn polyline_bounds_include_the_line_width() {
        let primitive = Primitive::Polyline {
            points: vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)],
            width: 4.0,
            color: Color::WHITE,
            closed: false,
        };

        assert_eq!(primitive.bounds(), Rect::new(-2.0, -2.0, 14.0, 4.0));
    }
}
