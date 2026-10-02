//! 大きさを決める決まり。
//!
//! # 2 周する
//!
//! ```text
//! measure(制約) ──▶ 欲しい大きさ      ……子に希望を聞く（下から上へ）
//! arrange(矩形)  ──▶ 置き場所の確定   ……親が割り振る（上から下へ）
//! ```
//!
//! 1 周では決まりません。「中身に合わせて縮む箱」は子の希望を聞くまで
//! 大きさが決まらず、「親いっぱいに伸びる子」は親が決まるまで決まりません。
//! 両方を同時に満たすには、**聞いてから割り振る**の 2 周が要ります。
//!
//! # 制約は下向き、大きさは上向き
//!
//! 親は子に [`Constraints`]（この範囲で収めてくれ）を渡し、子は [`Size`]
//! （この大きさが欲しい）を返します。**子は自分の位置を知りません。**
//! 位置は [`crate::gui::Widget::arrange`] で親が教えます。
//!
//! これを守ると、同じウィジェットがどこに置かれても同じように測れます。
//! 位置を見て大きさを変えるウィジェットを許すと、親が子を測った結果で
//! 位置が変わり、その位置で大きさが変わる、という循環に入ります。
//!
//! # 上限なし
//!
//! [`Constraints::max`] には正の無限大が入ります。縦に無限に伸びる
//! スクロール領域の中身を測るときなどです。**無限の制約を返してはいけません。**
//! [`Widget::measure`](crate::gui::Widget::measure) が無限を返すと、
//! 親はそれを矩形に使えません。無限を受けたら「中身に必要なぶん」を返します。

use crate::gui::geometry::{Axis, Insets, Size};

/// 親が子に渡す範囲。
///
/// `min` は「これ以上は縮まないでくれ」、`max` は「これ以上は広がらないでくれ」。
/// 子が返す大きさは呼ぶ側が [`Constraints::constrain`] で丸めるので、
/// 守らない子がいても形は壊れません。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Constraints {
    pub min: Size,
    pub max: Size,
}

impl Default for Constraints {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

impl Constraints {
    /// 下限 0、上限なし。何でも好きな大きさを返してよい。
    pub const UNBOUNDED: Self = Self {
        min: Size::ZERO,
        max: Size {
            width: f32::INFINITY,
            height: f32::INFINITY,
        },
    };

    /// 下限も上限も 0。**必ず 0 になる。**
    pub const ZERO: Self = Self {
        min: Size::ZERO,
        max: Size::ZERO,
    };

    /// 上限と下限が同じ。子に選ぶ余地はない。
    pub fn tight(size: Size) -> Self {
        Self {
            min: size,
            max: size,
        }
    }

    /// 上限だけ。0 まで縮んでよい。
    pub fn loose(max: Size) -> Self {
        Self {
            min: Size::ZERO,
            max,
        }
    }

    pub fn new(min: Size, max: Size) -> Self {
        // 下限が上限を超えていたら上限に合わせる。逆転した制約は
        // どちらを信じても間違うので、広がらない側を優先する。
        Self {
            min: min.min(max),
            max,
        }
    }

    /// この範囲に丸める。
    pub fn constrain(self, size: Size) -> Size {
        Size::new(
            size.width.clamp(self.min.width, self.max.width),
            size.height.clamp(self.min.height, self.max.height),
        )
    }

    /// 余白のぶん狭めた制約。子の中身に渡す。
    ///
    /// 上限から余白を引き、下限も同じだけ引きます（0 で止まります）。
    pub fn deflate(self, insets: Insets) -> Self {
        Self::new(self.min.deflate(insets), self.max.deflate(insets))
    }

    /// 上限いっぱいの大きさ。**上限が無限の軸では下限を返す。**
    ///
    /// 「親いっぱいに広がる」を実装するときに使います。無限を返さないのは、
    /// 返した大きさがそのまま矩形になるからです。
    pub fn biggest(self) -> Size {
        Size::new(
            finite_or(self.max.width, self.min.width),
            finite_or(self.max.height, self.min.height),
        )
    }

    /// 下限ぴったりの大きさ。
    pub fn smallest(self) -> Size {
        self.min
    }

    /// 上限と下限が一致しているか。
    pub fn is_tight(self) -> bool {
        self.min == self.max
    }

    pub fn has_bounded_width(self) -> bool {
        self.max.width.is_finite()
    }

    pub fn has_bounded_height(self) -> bool {
        self.max.height.is_finite()
    }

    /// その軸の上限が有限か。
    pub fn has_bounded(self, axis: Axis) -> bool {
        match axis {
            Axis::Horizontal => self.has_bounded_width(),
            Axis::Vertical => self.has_bounded_height(),
        }
    }

    /// 軸に沿った上限だけ外す。縦に積む箱が中身を測るときなどに使う。
    pub fn unbound(self, axis: Axis) -> Self {
        let mut max = self.max;
        let mut min = self.min;

        match axis {
            Axis::Horizontal => {
                max.width = f32::INFINITY;
                min.width = 0.0;
            }
            Axis::Vertical => {
                max.height = f32::INFINITY;
                min.height = 0.0;
            }
        }

        Self { min, max }
    }

    /// 軸に沿った長さを固定する。
    pub fn tighten(self, axis: Axis, length: f32) -> Self {
        let clamped = match axis {
            Axis::Horizontal => length.clamp(self.min.width, self.max.width),
            Axis::Vertical => length.clamp(self.min.height, self.max.height),
        };

        let mut min = self.min;
        let mut max = self.max;

        match axis {
            Axis::Horizontal => {
                min.width = clamped;
                max.width = clamped;
            }
            Axis::Vertical => {
                min.height = clamped;
                max.height = clamped;
            }
        }

        Self { min, max }
    }
}

/// 長さの指定。
///
/// 並べる箱が子をどう扱うかを書くためのもので、**どう見えるか**とは別です。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub enum Length {
    /// 中身に必要なぶん。子に測らせる。
    #[default]
    Hug,
    /// ピクセル固定。
    Fixed(f32),
    /// 余りを分け合う。数は**重み**で、割合ではない。
    ///
    /// 2 つの子が `Grow(1.0)` と `Grow(3.0)` なら 1 : 3 に分かれます。
    /// 合計を 1 に揃える必要はありません。
    Grow(f32),
}

impl Length {
    /// 余りを分け合う重み。分け合わないなら 0。
    pub fn grow_weight(self) -> f32 {
        match self {
            Self::Grow(weight) if weight > 0.0 => weight,
            _ => 0.0,
        }
    }

    pub fn is_grow(self) -> bool {
        self.grow_weight() > 0.0
    }

    /// 測る前に決まる長さ。`Hug` と `Grow` は測らないと決まらないので `None`。
    pub fn fixed(self) -> Option<f32> {
        match self {
            Self::Fixed(length) => Some(length.max(0.0)),
            Self::Hug | Self::Grow(_) => None,
        }
    }
}

/// 測り直しを省くための覚え書き。
///
/// # なぜ要るのか
///
/// [`Widget::measure`](crate::gui::Widget::measure) は木の深さぶん入れ子に
/// 走るので、何もしていないフレームでも全部測ると無駄です。
/// **同じ制約で同じ中身なら答えは同じ**なので、前回の制約と答えを覚えておき、
/// 制約が変わっていなければそのまま返します。
///
/// 中身が変わったときは [`LayoutCache::invalidate`] で捨てます。
/// これを呼び忘れると古い大きさのまま出るので、ウィジェット側で状態を
/// 書き換えたら [`LayoutContext::request_layout`](crate::gui::LayoutContext)
/// を通してください（木が代わりに捨てます）。
#[derive(Clone, Copy)]
#[derive(Debug, Default)]
pub struct LayoutCache {
    last: Option<(Constraints, Size)>,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// その制約での答えを覚えていれば返す。
    pub fn get(&self, constraints: Constraints) -> Option<Size> {
        self.last
            .filter(|(cached, _)| *cached == constraints)
            .map(|(_, size)| size)
    }

    pub fn put(&mut self, constraints: Constraints, size: Size) {
        self.last = Some((constraints, size));
    }

    /// 覚えを捨てる。中身が変わったとき。
    pub fn invalidate(&mut self) {
        self.last = None;
    }

    pub fn is_valid(&self) -> bool {
        self.last.is_some()
    }
}

/// 無限なら代わりの値を返す。
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constrain_clamps_both_ways() {
        let constraints = Constraints::new(Size::new(10.0, 10.0), Size::new(100.0, 100.0));

        assert_eq!(constraints.constrain(Size::new(5.0, 500.0)), Size::new(10.0, 100.0));
        assert_eq!(constraints.constrain(Size::new(50.0, 50.0)), Size::new(50.0, 50.0));
    }

    #[test]
    fn biggest_falls_back_to_min_when_unbounded() {
        let constraints = Constraints {
            min: Size::new(20.0, 5.0),
            max: Size::new(f32::INFINITY, 80.0),
        };

        // 無限をそのまま矩形には使えないので、下限で止める。
        assert_eq!(constraints.biggest(), Size::new(20.0, 80.0));
    }

    #[test]
    fn reversed_constraints_collapse_to_max() {
        let constraints = Constraints::new(Size::new(50.0, 50.0), Size::new(10.0, 10.0));

        assert_eq!(constraints.min, Size::new(10.0, 10.0));
        assert!(constraints.is_tight());
    }

    #[test]
    fn deflate_shrinks_and_stops_at_zero() {
        let constraints = Constraints::loose(Size::new(100.0, 40.0));
        let inner = constraints.deflate(Insets::all(10.0));

        assert_eq!(inner.max, Size::new(80.0, 20.0));

        let squeezed = constraints.deflate(Insets::all(999.0));
        assert_eq!(squeezed.max, Size::ZERO);
    }

    #[test]
    fn unbound_only_touches_one_axis() {
        let constraints = Constraints::tight(Size::new(100.0, 40.0));
        let scrollable = constraints.unbound(Axis::Vertical);

        assert_eq!(scrollable.max.width, 100.0);
        assert!(scrollable.max.height.is_infinite());
        assert!(!scrollable.has_bounded(Axis::Vertical));
    }

    #[test]
    fn cache_only_answers_the_same_question() {
        let mut cache = LayoutCache::new();
        let asked = Constraints::loose(Size::new(100.0, 100.0));

        cache.put(asked, Size::new(30.0, 10.0));

        assert_eq!(cache.get(asked), Some(Size::new(30.0, 10.0)));
        assert_eq!(cache.get(Constraints::UNBOUNDED), None);

        cache.invalidate();
        assert_eq!(cache.get(asked), None);
    }

    #[test]
    fn grow_weights_are_relative() {
        assert_eq!(Length::Grow(3.0).grow_weight(), 3.0);
        assert_eq!(Length::Hug.grow_weight(), 0.0);
        // 0 以下の重みは分け合わない扱い。割り算で落ちないようにする。
        assert_eq!(Length::Grow(0.0).grow_weight(), 0.0);
        assert_eq!(Length::Fixed(12.0).fixed(), Some(12.0));
    }
}
