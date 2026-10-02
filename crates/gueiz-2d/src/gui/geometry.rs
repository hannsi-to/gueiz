//! 画面の上の位置と大きさ。
//!
//! # 座標系
//!
//! 原点は**左上**、x は右、y は**下**向き。単位は論理ピクセル。
//!
//! このクレートの [`Camera`](crate::camera) は自由に組めますが、GUI は
//! 「左上が原点で下が正」に固定します。UI の寸法はデザイン側が
//! 「上から 16 px」のように考えるもので、数学の向きに合わせても誰も得をしません。
//!
//! 画面の倍率（HiDPI）はここでは扱いません。[`crate::gui::Gui`] が
//! 入口で物理ピクセルを論理ピクセルに直すので、ウィジェットは
//! **常に論理ピクセルだけ**を見ます。
//!
//! # 空の矩形
//!
//! 幅か高さが 0 以下の [`Rect`] は**空**です。空の矩形は何も含まず
//! （[`Rect::contains`] が常に `false`）、重なりもしません。
//! `width` を負にして「左向きの矩形」を表すようなことはしません。

use std::ops::{Add, AddAssign, Sub, SubAssign};

/// 点。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// 2 点の距離。
    pub fn distance(self, other: Self) -> f32 {
        (self - other).length()
    }

    /// 原点からの距離。
    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    pub fn to_array(self) -> [f32; 2] {
        [self.x, self.y]
    }

    /// 有限でない成分（NaN / 無限）を持たないか。
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Point {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Point {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl AddAssign for Point {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl SubAssign for Point {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl From<[f32; 2]> for Point {
    fn from(value: [f32; 2]) -> Self {
        Self::new(value[0], value[1])
    }
}

/// 大きさ。**負にはならない。**
///
/// [`Size::new`] は負を 0 に丸めます。負の大きさを持ち回ると、
/// レイアウトの引き算で符号が反転して原因の分からない形になります。
///
/// 正の無限大は通します。[`Constraints`](crate::gui::layout::Constraints) が
/// 「上限なし」をこれで表すためです。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };

    /// 負は 0 に、NaN も 0 に丸める。無限大はそのまま。
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: sanitize(width),
            height: sanitize(height),
        }
    }

    /// 縦横おなじ。
    pub fn splat(side: f32) -> Self {
        Self::new(side, side)
    }

    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// 軸に沿った長さ。
    pub fn along(self, axis: Axis) -> f32 {
        match axis {
            Axis::Horizontal => self.width,
            Axis::Vertical => self.height,
        }
    }

    /// 軸と直交する長さ。
    pub fn across(self, axis: Axis) -> f32 {
        self.along(axis.cross())
    }

    /// 軸ごとに長さを組む。`main` が `axis` に沿う側。
    pub fn from_axis(axis: Axis, main: f32, cross: f32) -> Self {
        match axis {
            Axis::Horizontal => Self::new(main, cross),
            Axis::Vertical => Self::new(cross, main),
        }
    }

    /// 両方を包む大きさ。
    pub fn max(self, other: Self) -> Self {
        Self::new(
            self.width.max(other.width),
            self.height.max(other.height),
        )
    }

    /// 両方に収まる大きさ。
    pub fn min(self, other: Self) -> Self {
        Self::new(
            self.width.min(other.width),
            self.height.min(other.height),
        )
    }

    /// 余白を足した大きさ。
    pub fn inflate(self, insets: Insets) -> Self {
        Self::new(
            self.width + insets.horizontal(),
            self.height + insets.vertical(),
        )
    }

    /// 余白を引いた大きさ。引きすぎたら 0 で止まる。
    pub fn deflate(self, insets: Insets) -> Self {
        Self::new(
            self.width - insets.horizontal(),
            self.height - insets.vertical(),
        )
    }

    pub fn to_array(self) -> [f32; 2] {
        [self.width, self.height]
    }
}

/// 位置と大きさ。`x` / `y` は**左上**。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width: sanitize(width),
            height: sanitize(height),
        }
    }

    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::new(origin.x, origin.y, size.width, size.height)
    }

    /// 左上と右下から組む。順序が逆でも入れ替えて受け取る。
    pub fn from_corners(a: Point, b: Point) -> Self {
        Self::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        )
    }

    pub fn left(self) -> f32 {
        self.x
    }

    pub fn top(self) -> f32 {
        self.y
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub fn origin(self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn center(self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// その点を含むか。**右端と下端は含まない**（`x <= p.x < right`）。
    ///
    /// 端を含めると、隣り合う矩形の境目で両方が当たります。
    /// 当たり判定はどちらか 1 つに決まらないと使えません。
    ///
    /// ```
    /// # use gueiz_2d::gui::geometry::{Point, Rect};
    /// let left = Rect::new(0.0, 0.0, 10.0, 10.0);
    /// let right = Rect::new(10.0, 0.0, 10.0, 10.0);
    ///
    /// assert!(!left.contains(Point::new(10.0, 5.0)), "境目は右の矩形のもの");
    /// assert!(right.contains(Point::new(10.0, 5.0)));
    /// ```
    pub fn contains(self, point: Point) -> bool {
        !self.is_empty()
            && point.x >= self.x
            && point.x < self.right()
            && point.y >= self.y
            && point.y < self.bottom()
    }

    /// 重なっている部分。重なっていなければ [`Rect::ZERO`]。
    pub fn intersect(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());

        if right <= x || bottom <= y {
            return Self::ZERO;
        }

        Self::new(x, y, right - x, bottom - y)
    }

    /// 少しでも重なっているか。
    pub fn overlaps(self, other: Self) -> bool {
        !self.intersect(other).is_empty()
    }

    /// 両方を包むいちばん小さい矩形。片方が空ならもう片方。
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }

        if other.is_empty() {
            return self;
        }

        Self::from_corners(
            Point::new(self.x.min(other.x), self.y.min(other.y)),
            Point::new(self.right().max(other.right()), self.bottom().max(other.bottom())),
        )
    }

    /// 内側へ縮める。縮めすぎたら幅・高さが 0 で止まる。
    pub fn deflate(self, insets: Insets) -> Self {
        Self::new(
            self.x + insets.left,
            self.y + insets.top,
            self.width - insets.horizontal(),
            self.height - insets.vertical(),
        )
    }

    /// 外側へ広げる。
    pub fn inflate(self, insets: Insets) -> Self {
        Self::new(
            self.x - insets.left,
            self.y - insets.top,
            self.width + insets.horizontal(),
            self.height + insets.vertical(),
        )
    }

    pub fn translate(self, offset: Point) -> Self {
        Self::new(self.x + offset.x, self.y + offset.y, self.width, self.height)
    }

    /// この矩形の中に `size` を置く。寄せ方は `align`。
    pub fn align_size(self, size: Size, align: Alignment) -> Self {
        let x = self.x + align.horizontal.offset(self.width, size.width);
        let y = self.y + align.vertical.offset(self.height, size.height);

        Self::new(x, y, size.width, size.height)
    }

    /// `[左, 上, 右, 下]`。[`crate::effect::Block::ClipRect`] に渡す形。
    pub fn to_min_max(self) -> ([f32; 2], [f32; 2]) {
        ([self.x, self.y], [self.right(), self.bottom()])
    }
}

/// 四辺の余白。内側（padding）にも外側（margin）にも使う。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub const ZERO: Self = Self {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    /// 四辺おなじ。
    pub const fn all(amount: f32) -> Self {
        Self {
            top: amount,
            right: amount,
            bottom: amount,
            left: amount,
        }
    }

    /// 縦と横で別。
    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// 左右の合計。
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }

    /// 上下の合計。
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }

    /// 軸に沿った合計。
    pub fn along(self, axis: Axis) -> f32 {
        match axis {
            Axis::Horizontal => self.horizontal(),
            Axis::Vertical => self.vertical(),
        }
    }

    /// 軸に沿った手前側（横なら左、縦なら上）。
    pub fn leading(self, axis: Axis) -> f32 {
        match axis {
            Axis::Horizontal => self.left,
            Axis::Vertical => self.top,
        }
    }
}

/// 並べる向き。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug, Default)]
pub enum Axis {
    #[default]
    Horizontal,
    Vertical,
}

impl Axis {
    pub fn cross(self) -> Self {
        match self {
            Self::Horizontal => Self::Vertical,
            Self::Vertical => Self::Horizontal,
        }
    }
}

/// 1 軸の寄せ方。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

impl Align {
    /// `available` の中に `used` を置くときの手前からの距離。
    pub fn offset(self, available: f32, used: f32) -> f32 {
        let slack = available - used;

        match self {
            Self::Start => 0.0,
            Self::Center => slack / 2.0,
            Self::End => slack,
        }
    }
}

/// 縦横の寄せ方。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug, Default)]
pub struct Alignment {
    pub horizontal: Align,
    pub vertical: Align,
}

impl Alignment {
    pub const TOP_LEFT: Self = Self::new(Align::Start, Align::Start);
    pub const TOP_CENTER: Self = Self::new(Align::Center, Align::Start);
    pub const TOP_RIGHT: Self = Self::new(Align::End, Align::Start);
    pub const CENTER_LEFT: Self = Self::new(Align::Start, Align::Center);
    pub const CENTER: Self = Self::new(Align::Center, Align::Center);
    pub const CENTER_RIGHT: Self = Self::new(Align::End, Align::Center);
    pub const BOTTOM_LEFT: Self = Self::new(Align::Start, Align::End);
    pub const BOTTOM_CENTER: Self = Self::new(Align::Center, Align::End);
    pub const BOTTOM_RIGHT: Self = Self::new(Align::End, Align::End);

    pub const fn new(horizontal: Align, vertical: Align) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }
}

/// 四隅の丸み。ピクセル。
///
/// 丸めの形そのものはデザインの話なので、ここは**入れ物だけ**です。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub const ZERO: Self = Self::all(0.0);

    pub const fn all(radius: f32) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    pub const fn new(
        top_left: f32,
        top_right: f32,
        bottom_right: f32,
        bottom_left: f32,
    ) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    pub fn is_zero(self) -> bool {
        self.max() <= 0.0
    }

    /// いちばん大きい丸み。
    pub fn max(self) -> f32 {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }

    /// 矩形に収まるよう頭打ちにする。辺の半分を超える丸みは隣とぶつかる。
    pub fn clamp_to(self, size: Size) -> Self {
        let limit = (size.width.min(size.height) / 2.0).max(0.0);

        Self::new(
            self.top_left.clamp(0.0, limit),
            self.top_right.clamp(0.0, limit),
            self.bottom_right.clamp(0.0, limit),
            self.bottom_left.clamp(0.0, limit),
        )
    }
}

/// 負と NaN を 0 に丸める。**正の無限大は通す。**
///
/// 無限大は [`Constraints`](crate::gui::layout::Constraints) が
/// 「上限なし」を表すのに使うので、潰してはいけない。
fn sanitize(value: f32) -> f32 {
    if value.is_nan() || value < 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_size_collapses_to_zero() {
        assert_eq!(Size::new(-5.0, 10.0), Size::new(0.0, 10.0));
        assert_eq!(Rect::new(0.0, 0.0, -1.0, -1.0), Rect::ZERO);
        assert!(Size::new(f32::NAN, 1.0).width == 0.0);
        // 上限なしを潰してはいけない。
        assert!(Size::new(f32::INFINITY, 1.0).width.is_infinite());
    }

    #[test]
    fn deflate_stops_at_zero() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        let squeezed = rect.deflate(Insets::all(20.0));

        assert!(squeezed.is_empty());
        assert_eq!(squeezed.width, 0.0);
    }

    #[test]
    fn intersect_of_disjoint_is_empty() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(20.0, 0.0, 10.0, 10.0);

        assert!(a.intersect(b).is_empty());
        assert!(!a.overlaps(b));
        assert_eq!(a.union(b), Rect::new(0.0, 0.0, 30.0, 10.0));
    }

    #[test]
    fn align_places_inside() {
        let area = Rect::new(0.0, 0.0, 100.0, 50.0);
        let placed = area.align_size(Size::new(20.0, 10.0), Alignment::CENTER);

        assert_eq!(placed, Rect::new(40.0, 20.0, 20.0, 10.0));
    }

    #[test]
    fn corners_clamp_to_half_the_shorter_side() {
        let clamped = Corners::all(50.0).clamp_to(Size::new(40.0, 20.0));

        assert_eq!(clamped.max(), 10.0);
    }
}
