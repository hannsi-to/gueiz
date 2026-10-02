//! 子を並べる箱。**見た目は持ちません。**
//!
//! 2 つあります。
//!
//! | | 並べ方 | 使うところ |
//! |---|---|---|
//! | [`Stack`] | 軸に沿って順に | 縦並び・横並び・道具箱 |
//! | [`Floating`] | 子が言った座標に | 浮いた窓・吹き出し・つまみ |
//!
//! # 余りの分け方
//!
//! [`Stack`] は子ごとに [`Length`] を覚えます。
//!
//! ```text
//! Fixed(120)  ──▶ 120 px で固定
//! Hug         ──▶ 中身に必要なぶん
//! Grow(w)     ──▶ Fixed と Hug を置いたあとの余りを、重み w で分ける
//! ```
//!
//! 覚えていない子は [`Length::Hug`] です。**[`Stack`] に入れるだけで並びます**が、
//! 伸ばしたいものだけ [`Stack::set_length`] を呼んでください。
//!
//! 鍵で覚えているので、外から子を足しても消しても壊れません。

use fxhash::FxHashMap;

use crate::gui::geometry::{Align, Axis, Insets, Point, Rect, Size};
use crate::gui::id::WidgetId;
use crate::gui::layout::{Constraints, Length};
use crate::gui::widget::Behavior;
use crate::gui::{ArrangeContext, MeasureContext, Widget};

/// 軸と直交する向きの寄せ方。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub enum CrossAlign {
    #[default]
    Start,
    Center,
    End,
    /// 軸と直交する向きいっぱいに伸ばす。
    Stretch,
}

impl CrossAlign {
    fn align(self) -> Align {
        match self {
            Self::Start | Self::Stretch => Align::Start,
            Self::Center => Align::Center,
            Self::End => Align::End,
        }
    }
}

/// 軸に沿って順に並べる箱。
///
/// ```no_run
/// # use gueiz_2d::gui::container::Stack;
/// # use gueiz_2d::gui::geometry::{Axis, Insets};
/// # use gueiz_2d::gui::layout::Length;
/// # use gueiz_2d::gui::Gui;
/// # fn build(gui: &mut Gui) {
/// # fn some_widget() -> Box<dyn gueiz_2d::gui::Widget> { unimplemented!() }
/// let row = gui.set_root(Box::new(
///     Stack::new(Axis::Horizontal)
///         .gap(8.0)
///         .padding(Insets::all(16.0)),
/// ));
///
/// let left = gui.tree_mut().add_child(row, some_widget());
/// let filler = gui.tree_mut().add_child(row, some_widget());
///
/// // 真ん中だけ余りを取る。
/// if let Some(stack) = gui.tree_mut().get_as_mut::<Stack>(row) {
///     stack.set_length(filler, Length::Grow(1.0));
/// }
/// # let _ = left;
/// # }
/// ```
pub struct Stack {
    axis: Axis,
    gap: f32,
    padding: Insets,
    main_align: Align,
    cross_align: CrossAlign,
    lengths: FxHashMap<WidgetId, Length>,
    behavior: Behavior,
}

impl Stack {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            gap: 0.0,
            padding: Insets::ZERO,
            main_align: Align::Start,
            cross_align: CrossAlign::default(),
            lengths: FxHashMap::default(),
            behavior: Behavior::DECORATION,
        }
    }

    /// 縦に積む。
    pub fn column() -> Self {
        Self::new(Axis::Vertical)
    }

    /// 横に並べる。
    pub fn row() -> Self {
        Self::new(Axis::Horizontal)
    }

    /// 子と子のあいだ。
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// 内側の余白。
    pub fn padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    /// 余りが出たときに、並び全体をどこへ寄せるか。
    ///
    /// [`Length::Grow`] の子が 1 つでもあれば余りは出ないので、効きません。
    pub fn main_align(mut self, main_align: Align) -> Self {
        self.main_align = main_align;
        self
    }

    pub fn cross_align(mut self, cross_align: CrossAlign) -> Self {
        self.cross_align = cross_align;
        self
    }

    /// 入力の扱いを変える。既定は飾り（素通り）。
    ///
    /// 背景を押して閉じる、はみ出しを切り抜く、といったときに差し替えます。
    pub fn behavior(mut self, behavior: Behavior) -> Self {
        self.behavior = behavior;
        self
    }

    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// その子の軸に沿った長さを決める。
    pub fn set_length(&mut self, child: WidgetId, length: Length) -> &mut Self {
        self.lengths.insert(child, length);
        self
    }

    /// 覚えている長さ。覚えていなければ [`Length::Hug`]。
    pub fn length(&self, child: WidgetId) -> Length {
        self.lengths.get(&child).copied().unwrap_or(Length::Hug)
    }

    /// 覚えを捨てる。子を消したあとに呼ばなくても動きますが、
    /// 長く動かすものでは溜まるので捨てられるようにしてあります。
    pub fn forget(&mut self, child: WidgetId) -> &mut Self {
        self.lengths.remove(&child);
        self
    }

    /// 子 1 つを測るときの制約。
    ///
    /// **下限は渡しません。** 自分が画面いっぱいに広げられていても、
    /// それは子が広がる理由にはなりません。下限をそのまま流すと、
    /// 子が全員親の大きさになって並びが潰れます。
    /// 広げたいものは [`Length::Grow`] か [`CrossAlign::Stretch`] で
    /// **明示されたときだけ**固定します。
    fn child_constraints(&self, inner: Constraints, main: Option<f32>) -> Constraints {
        let cross = self.axis.cross();
        let loose = Constraints::loose(inner.max);

        // 軸に沿った側: 決まっていれば固定、決まっていなければ上限なしで聞く。
        let constraints = match main {
            Some(main) => loose.tighten(self.axis, main),
            None => loose.unbound(self.axis),
        };

        // 直交する側: 伸ばすなら固定、そうでなければ上限まで好きに。
        match self.cross_align {
            CrossAlign::Stretch if inner.has_bounded(cross) => {
                constraints.tighten(cross, inner.max.across(self.axis))
            }
            _ => constraints,
        }
    }
}

impl Widget for Stack {
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        let children = context.children();

        if children.is_empty() {
            return constraints.constrain(Size::ZERO.inflate(self.padding));
        }

        let inner = constraints.deflate(self.padding);
        let gaps = self.gap * (children.len() - 1) as f32;

        let mut used_main = 0.0_f32;
        let mut max_cross = 0.0_f32;
        let mut total_weight = 0.0_f32;

        // 1 周目: 伸びない子を測る。
        for child in &children {
            let length = self.length(*child);

            if let Some(weight) = Some(length.grow_weight()).filter(|w| *w > 0.0) {
                total_weight += weight;
                continue;
            }

            let size = context.measure_child(*child, self.child_constraints(inner, length.fixed()));

            used_main += size.along(self.axis);
            max_cross = max_cross.max(size.across(self.axis));
        }

        // 2 周目: 余りを分ける。軸の上限が無ければ余りは無いので、
        // 伸びる子も中身に合わせて測る。
        if total_weight > 0.0 {
            let available = if inner.has_bounded(self.axis) {
                (inner.max.along(self.axis) - gaps - used_main).max(0.0)
            } else {
                0.0
            };

            for child in &children {
                let weight = self.length(*child).grow_weight();

                if weight <= 0.0 {
                    continue;
                }

                let share = if inner.has_bounded(self.axis) {
                    Some(available * weight / total_weight)
                } else {
                    // 上限が無いので分けるものが無い。中身に任せる。
                    None
                };

                let size = context.measure_child(*child, self.child_constraints(inner, share));

                used_main += size.along(self.axis);
                max_cross = max_cross.max(size.across(self.axis));
            }
        }

        let content = Size::from_axis(self.axis, used_main + gaps, max_cross);

        constraints.constrain(content.inflate(self.padding))
    }

    fn arrange(&mut self, context: &mut ArrangeContext<'_>, bounds: Rect) {
        let children = context.children();

        if children.is_empty() {
            return;
        }

        let inner = bounds.deflate(self.padding);
        let gaps = self.gap * (children.len() - 1) as f32;

        // 測った結果を足し直す。測る周回と同じ数字になる。
        let used: f32 = children
            .iter()
            .map(|child| context.desired(*child).along(self.axis))
            .sum();

        let slack = (inner.size().along(self.axis) - used - gaps).max(0.0);
        let mut main = self.main_align.offset(slack + used + gaps, used + gaps);

        for child in &children {
            let desired = context.desired(*child);

            let cross_length = match self.cross_align {
                CrossAlign::Stretch => inner.size().across(self.axis),
                _ => desired.across(self.axis).min(inner.size().across(self.axis)),
            };

            let cross = self
                .cross_align
                .align()
                .offset(inner.size().across(self.axis), cross_length);

            let main_length = desired.along(self.axis);

            // 軸ごとに (x, y) へ組み直す。
            let (x, y, width, height) = match self.axis {
                Axis::Horizontal => (main, cross, main_length, cross_length),
                Axis::Vertical => (cross, main, cross_length, main_length),
            };

            context.place(
                *child,
                Rect::new(inner.x + x, inner.y + y, width, height),
            );

            main += main_length + self.gap;
        }
    }

    fn behavior(&self) -> Behavior {
        self.behavior
    }
}

/// 子を**子自身が言った座標**に置く箱。
///
/// 子が [`Widget::absolute_position`] を返さなければ `(0, 0)` に置きます。
/// 大きさは子に聞いたぶん。重なり順は足した順（あとが手前）。
///
/// 浮いた窓を並べる台として使います。
pub struct Floating {
    behavior: Behavior,
}

impl Default for Floating {
    fn default() -> Self {
        Self::new()
    }
}

impl Floating {
    pub fn new() -> Self {
        Self {
            behavior: Behavior::DECORATION,
        }
    }

    pub fn behavior(mut self, behavior: Behavior) -> Self {
        self.behavior = behavior;
        self
    }
}

impl Widget for Floating {
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        // 子は好きな大きさでよい。台は画面いっぱい。
        let loose = Constraints::loose(constraints.max);

        for child in context.children() {
            context.measure_child(child, loose);
        }

        constraints.biggest()
    }

    fn arrange(&mut self, context: &mut ArrangeContext<'_>, bounds: Rect) {
        for child in context.children() {
            let size = context.desired(child);

            // 子が位置を言っていればそこへ。言わなければ左上。
            let position = context
                .absolute_position(child)
                .unwrap_or(Point::ZERO);

            let _ = bounds;
            context.place(child, Rect::from_origin_size(position, size));
        }
    }

    fn behavior(&self) -> Behavior {
        self.behavior
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::context::NoTextMeasure;
    use crate::gui::Gui;

    /// 固定の大きさを言うだけの子。
    struct Fixed {
        size: Size,
        at: Option<Point>,
    }

    impl Fixed {
        fn boxed(width: f32, height: f32) -> Box<dyn Widget> {
            Box::new(Self {
                size: Size::new(width, height),
                at: None,
            })
        }

        fn at(width: f32, height: f32, x: f32, y: f32) -> Box<dyn Widget> {
            Box::new(Self {
                size: Size::new(width, height),
                at: Some(Point::new(x, y)),
            })
        }
    }

    impl Widget for Fixed {
        fn measure(&mut self, _: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
            constraints.constrain(self.size)
        }

        fn absolute_position(&self) -> Option<Point> {
            self.at
        }
    }

    fn laid_out(gui: &mut Gui, viewport: Size) {
        gui.layout(viewport, &NoTextMeasure);
    }

    #[test]
    fn a_column_stacks_with_gaps_and_padding() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(
            Stack::column().gap(10.0).padding(Insets::all(5.0)),
        ));

        let a = gui.tree_mut().add_child(root, Fixed::boxed(40.0, 20.0));
        let b = gui.tree_mut().add_child(root, Fixed::boxed(40.0, 30.0));

        laid_out(&mut gui, Size::new(200.0, 200.0));

        assert_eq!(gui.tree().bounds(a), Rect::new(5.0, 5.0, 40.0, 20.0));
        assert_eq!(gui.tree().bounds(b), Rect::new(5.0, 35.0, 40.0, 30.0));
    }

    #[test]
    fn grow_splits_the_leftover_by_weight() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(Stack::row()));

        let fixed = gui.tree_mut().add_child(root, Fixed::boxed(100.0, 10.0));
        let small = gui.tree_mut().add_child(root, Fixed::boxed(0.0, 10.0));
        let large = gui.tree_mut().add_child(root, Fixed::boxed(0.0, 10.0));

        let stack = gui.tree_mut().get_as_mut::<Stack>(root).unwrap();
        stack.set_length(small, Length::Grow(1.0));
        stack.set_length(large, Length::Grow(3.0));

        laid_out(&mut gui, Size::new(500.0, 50.0));

        // 余り 400 を 1 : 3 で分ける。
        assert_eq!(gui.tree().bounds(fixed).width, 100.0);
        assert_eq!(gui.tree().bounds(small).width, 100.0);
        assert_eq!(gui.tree().bounds(large).width, 300.0);
        assert_eq!(gui.tree().bounds(large).x, 200.0);
    }

    #[test]
    fn fixed_length_overrides_what_the_child_wants() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(Stack::row()));

        let child = gui.tree_mut().add_child(root, Fixed::boxed(500.0, 10.0));
        gui.tree_mut()
            .get_as_mut::<Stack>(root)
            .unwrap()
            .set_length(child, Length::Fixed(60.0));

        laid_out(&mut gui, Size::new(500.0, 50.0));

        assert_eq!(gui.tree().bounds(child).width, 60.0);
    }

    #[test]
    fn stretch_fills_the_cross_axis() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(
            Stack::column()
                .cross_align(CrossAlign::Stretch)
                .padding(Insets::symmetric(0.0, 10.0)),
        ));

        let child = gui.tree_mut().add_child(root, Fixed::boxed(5.0, 20.0));

        laid_out(&mut gui, Size::new(200.0, 100.0));

        // 左右 10 の余白を引いた 180 まで伸びる。
        assert_eq!(gui.tree().bounds(child).width, 180.0);
        assert_eq!(gui.tree().bounds(child).x, 10.0);
    }

    #[test]
    fn cross_align_centres_without_stretching() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(
            Stack::column().cross_align(CrossAlign::Center),
        ));

        let child = gui.tree_mut().add_child(root, Fixed::boxed(40.0, 20.0));

        laid_out(&mut gui, Size::new(200.0, 100.0));

        assert_eq!(gui.tree().bounds(child).x, 80.0);
        assert_eq!(gui.tree().bounds(child).width, 40.0);
    }

    #[test]
    fn main_align_moves_the_whole_run() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(Stack::row().main_align(Align::End)));

        let a = gui.tree_mut().add_child(root, Fixed::boxed(30.0, 10.0));
        let b = gui.tree_mut().add_child(root, Fixed::boxed(20.0, 10.0));

        laid_out(&mut gui, Size::new(100.0, 50.0));

        // 合計 50 を右端へ寄せる。
        assert_eq!(gui.tree().bounds(a).x, 50.0);
        assert_eq!(gui.tree().bounds(b).x, 80.0);
    }

    #[test]
    fn a_stack_hugs_its_content_when_unbounded() {
        let mut gui = Gui::new();

        // 縦に伸びる外側の中に、横並びを入れる。
        let root = gui.set_root(Box::new(Stack::column()));
        let row = gui
            .tree_mut()
            .add_child(root, Box::new(Stack::row().gap(4.0)));

        gui.tree_mut().add_child(row, Fixed::boxed(30.0, 12.0));
        gui.tree_mut().add_child(row, Fixed::boxed(20.0, 18.0));

        laid_out(&mut gui, Size::new(400.0, 400.0));

        let bounds = gui.tree().bounds(row);

        assert_eq!(bounds.width, 54.0, "30 + 4 + 20");
        assert_eq!(bounds.height, 18.0, "いちばん高い子に合わせる");
    }

    #[test]
    fn an_empty_stack_is_just_its_padding() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(
            Stack::column().padding(Insets::all(7.0)),
        ));

        // 根は画面いっぱいなので、中身の無さは子の側で見る。
        let inner = gui
            .tree_mut()
            .add_child(root, Box::new(Stack::row().padding(Insets::all(7.0))));

        laid_out(&mut gui, Size::new(200.0, 200.0));

        assert_eq!(gui.tree().bounds(inner).size(), Size::new(14.0, 14.0));
    }

    #[test]
    fn forgetting_a_length_falls_back_to_hug() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(Stack::row()));
        let child = gui.tree_mut().add_child(root, Fixed::boxed(25.0, 10.0));

        let stack = gui.tree_mut().get_as_mut::<Stack>(root).unwrap();
        stack.set_length(child, Length::Fixed(90.0));
        assert_eq!(stack.length(child), Length::Fixed(90.0));

        stack.forget(child);
        assert_eq!(stack.length(child), Length::Hug);

        gui.request_layout();
        laid_out(&mut gui, Size::new(300.0, 50.0));

        assert_eq!(gui.tree().bounds(child).width, 25.0);
    }

    #[test]
    fn floating_places_children_where_they_ask() {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(Floating::new()));

        let placed = gui
            .tree_mut()
            .add_child(root, Fixed::at(120.0, 80.0, 30.0, 40.0));
        let defaulted = gui.tree_mut().add_child(root, Fixed::boxed(10.0, 10.0));

        laid_out(&mut gui, Size::new(400.0, 300.0));

        assert_eq!(gui.tree().bounds(placed), Rect::new(30.0, 40.0, 120.0, 80.0));
        assert_eq!(gui.tree().bounds(defaulted), Rect::new(0.0, 0.0, 10.0, 10.0));
    }
}
