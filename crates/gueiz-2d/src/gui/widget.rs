//! ウィジェットが満たす約束。
//!
//! # 1 つのウィジェットがやること
//!
//! | 周回 | 何をするか | 座標 |
//! |---|---|---|
//! | [`Widget::measure`] | 制約を見て**欲しい大きさ**を返す | 持たない |
//! | [`Widget::arrange`] | 子に置き場所を割り振る | 自分の左上から |
//! | [`Widget::paint`] | 描くものを記録する | 自分の左上から |
//! | [`Widget::on_event`] | 入力に応える | 自分の左上から |
//!
//! **どの周回でも自分の画面上の位置は出てきません。** 位置を見ないと
//! 書けないものは、置かれる場所が変わるたびに違う結果になります。
//! 位置を足すのは [`Painter`](crate::gui::painter::Painter) と
//! [`ArrangeContext`](crate::gui::ArrangeContext) の仕事です。
//!
//! # 状態は木が持つ
//!
//! 「乗っている」「押されている」「焦点がある」は [`WidgetState`] に入り、
//! **[`Gui`](crate::gui::Gui) が書き換えます。** ウィジェットは読むだけです。
//!
//! 自分で持とうとすると、ポインタが別のウィジェットへ移ったときに
//! 「離れた」を自分で検出しなければならず、取りこぼします。
//! 誰の上にいるかを知っているのは木だけです。
//!
//! # 結果は溜めて渡す
//!
//! 「押された」をコールベックで返すと、`&mut` が二重に要って
//! 借用で詰まります。代わりに [`EventContext::emit`](crate::gui::EventContext::emit)
//! で [`Action`] を積み、呼ぶ側が [`Gui::drain_actions`](crate::gui::Gui::drain_actions)
//! でまとめて受け取ります。

use std::any::Any;

use crate::gui::event::{Event, EventResult, PointerButton};
use crate::gui::geometry::{Corners, Point, Rect, Size};
use crate::gui::id::{Tag, WidgetId};
use crate::gui::layout::Constraints;
use crate::gui::painter::Painter;
use crate::gui::{ArrangeContext, EventContext, MeasureContext, PaintContext};

/// 画面に出るもの 1 つ。
///
/// 既定の実装だけで「中身の無い箱」として成り立ちます。
/// 必要な周回だけ上書きしてください。
pub trait Widget: Any {
    /// 診断に出す名前。既定は型の名前。
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// 制約の中で欲しい大きさを返す。
    ///
    /// **無限を返してはいけません。** 制約の上限が無限なら
    /// [`Constraints::biggest`] ではなく「中身に必要なぶん」を返します。
    ///
    /// 既定は子をすべて同じ制約で測り、いちばん大きいものに合わせます。
    /// 子が無ければ下限。
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        let mut size = constraints.smallest();

        for child in context.children() {
            size = size.max(context.measure_child(child, constraints));
        }

        constraints.constrain(size)
    }

    /// 子に置き場所を割り振る。`bounds` は**自分の左上を原点とした**自分の矩形
    /// （つまり `x` と `y` は常に 0）。
    ///
    /// 既定は子を全員まるごと重ねて置きます。
    fn arrange(&mut self, context: &mut ArrangeContext<'_>, bounds: Rect) {
        for child in context.children() {
            context.place(child, bounds);
        }
    }

    /// 描くものを記録する。子は [`Gui`](crate::gui::Gui) が続けて描きます。
    ///
    /// 既定は何も描きません（中身だけが見える箱）。
    fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let _ = (context, painter);
    }

    /// 子を描いた**あと**に描く。枠線や光沢を中身の上に乗せるとき。
    ///
    /// 既定は何も描きません。
    fn paint_foreground(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let _ = (context, painter);
    }

    /// 自分の矩形の外に、切り抜きを突き抜けて描く。
    ///
    /// [`Behavior::overlay`] が立っているときだけ呼ばれます。呼ばれるのは
    /// **木を全部描き終えたあと**で、切り抜きは効かず、どの図形よりも手前です。
    ///
    /// 座標は [`Widget::paint`] と同じ**自分の左上から**なので、
    /// 自分の下に一覧を出すなら `y = 自分の高さ` から描きます。
    /// 画面からはみ出さないかは自分で見てください
    /// （[`ArrangeContext::viewport`](crate::gui::ArrangeContext::viewport) と
    /// [`ArrangeContext::screen_origin`](crate::gui::ArrangeContext::screen_origin)）。
    ///
    /// 既定は何も描きません。
    fn paint_overlay(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let _ = (context, painter);
    }

    /// 子を切り抜くときの角の丸み。
    ///
    /// [`Behavior::clips_children`] が立っているときだけ見ます。
    /// 丸いパネルの中でスクロールさせるときに、角から中身がはみ出さなくなります。
    fn clip_corners(&self) -> Corners {
        Corners::ZERO
    }

    /// 入力に応える。
    ///
    /// 受け取ったら [`EventResult::Consumed`] を返してください。
    /// 返さなければ親へ上がります。
    fn on_event(&mut self, context: &mut EventContext<'_>, event: &Event) -> EventResult {
        let _ = (context, event);
        EventResult::Ignored
    }

    /// 当たり判定。`local` は自分の左上からの座標、`size` は自分の大きさ。
    ///
    /// 既定は矩形まるごと。丸いつまみのように形で絞りたいときに上書きします。
    fn hit_test(&self, local: Point, size: Size) -> bool {
        Rect::from_origin_size(Point::ZERO, size).contains(local)
    }

    /// 置き場所を自分で決めるなら、その左上。
    ///
    /// 既定は `None`（親に任せる）。返すと
    /// [`Floating`](crate::gui::container::Floating) がそこへ置きます。
    /// **流れに沿って並べる親（[`Stack`](crate::gui::container::Stack)）は見ません。**
    /// 並びの中で勝手に動かれると、隣との間隔が合わなくなるからです。
    fn absolute_position(&self) -> Option<Point> {
        None
    }

    /// このウィジェットの扱い。
    fn behavior(&self) -> Behavior {
        Behavior::default()
    }
}

/// [`Widget`] を具体型に戻すための補助。
///
/// ```no_run
/// # use gueiz_2d::gui::id::WidgetId;
/// # use gueiz_2d::gui::tree::WidgetTree;
/// # struct Slider { value: f32 }
/// # impl gueiz_2d::gui::Widget for Slider {}
/// # fn run(tree: &mut WidgetTree, id: WidgetId) {
/// if let Some(slider) = tree.get_as_mut::<Slider>(id) {
///     slider.value = 0.5;
/// }
/// # }
/// ```
impl dyn Widget {
    pub fn downcast_ref<W: Widget>(&self) -> Option<&W> {
        (self as &dyn Any).downcast_ref::<W>()
    }

    pub fn downcast_mut<W: Widget>(&mut self) -> Option<&mut W> {
        (self as &mut dyn Any).downcast_mut::<W>()
    }
}

/// そのウィジェットをどう扱うか。**見た目ではなく扱い**の指定です。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub struct Behavior {
    /// ポインタを受け取るか。
    ///
    /// `false` だと当たり判定で**素通り**し、後ろのウィジェットに当たります。
    /// 飾りだけの枠や文字はこちらにします。
    pub interactive: bool,
    /// 焦点を受け取れるか。鍵盤の入力は焦点のあるウィジェットにだけ届きます。
    pub focusable: bool,
    /// 子を自分の矩形で切り抜くか。
    ///
    /// 立てると、子が描いたもののうち自分の外へ出たぶんが落ちます。
    /// スクロール領域はこれを立てます。
    pub clips_children: bool,
    /// 押したまま外へ出てもポインタを掴み続けるか。
    ///
    /// つまみやスクロール棒はこれを立てます。立てないと、少し外れた瞬間に
    /// ドラッグが切れます。
    pub captures_pointer: bool,
    /// 自分の矩形の外に、**切り抜きを突き抜けて**描くものを持つか。
    ///
    /// 立てると [`Widget::paint_overlay`] が最後にもう 1 周呼ばれ、
    /// **親の切り抜きが効かない状態で、どの図形よりも手前に**描かれます。
    /// 当たり判定もいちばん先に通ります。
    ///
    /// 開いた一覧・吹き出し・つまみの値表示のように、
    /// **自分より大きいものを一時的に出す**ときに立ててください。
    ///
    /// # 立てても自分の見た目は浮きません
    ///
    /// 浮くのは [`Widget::paint_overlay`] で描いたぶんだけです。
    /// [`Widget::paint`] は変わらず親の切り抜きを受けます。
    /// 畳んだ見た目は囲みの中に収まり、開いた一覧だけが飛び出す、
    /// という形になります。
    ///
    /// # 持ち主が見えなくなったら浮きません
    ///
    /// 親の切り抜きから自分が外れると、浮かせるぶんも描かれず、
    /// 当たりも取りません。送って画面から出た一覧が宙に残らないようにです。
    pub overlay: bool,
}

impl Behavior {
    /// 飾り。入力に関わらない。
    pub const DECORATION: Self = Self {
        interactive: false,
        focusable: false,
        clips_children: false,
        captures_pointer: false,
        overlay: false,
    };

    /// 押せるもの。掴みも張る。
    pub const CONTROL: Self = Self {
        interactive: true,
        focusable: true,
        clips_children: false,
        captures_pointer: true,
        overlay: false,
    };

    /// 入力を受けるが焦点は取らない。背景のパネルなど。
    pub const SURFACE: Self = Self {
        interactive: true,
        focusable: false,
        clips_children: false,
        captures_pointer: false,
        overlay: false,
    };

    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    pub fn clips_children(mut self, clips_children: bool) -> Self {
        self.clips_children = clips_children;
        self
    }

    pub fn captures_pointer(mut self, captures_pointer: bool) -> Self {
        self.captures_pointer = captures_pointer;
        self
    }

    pub fn overlay(mut self, overlay: bool) -> Self {
        self.overlay = overlay;
        self
    }
}

/// [`Gui`](crate::gui::Gui) が書き換える状態。ウィジェットは**読むだけ**。
///
/// デザイン側はこの 4 つを見て色や影を決めます。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug, Default)]
pub struct WidgetState {
    /// ポインタが上に乗っている。
    pub hovered: bool,
    /// 押されている。**上に乗っているかは別**です。
    ///
    /// 押したまま外へずらすと `hovered` は false、`pressed` は true のままです。
    /// この組み合わせが「放したらやめる」の見た目になります。
    pub pressed: bool,
    /// 焦点がある。鍵盤の入力はここに届く。
    pub focused: bool,
    /// 無効。入力を受け取らず、当たり判定も素通りする。
    pub disabled: bool,
}

impl WidgetState {
    /// 押されていて、かつ上に乗っている。「いま放せば決まる」状態。
    pub fn armed(self) -> bool {
        self.pressed && self.hovered
    }

    /// 何かしら触られているか。
    pub fn is_idle(self) -> bool {
        !self.hovered && !self.pressed && !self.focused
    }
}

/// ウィジェットが呼ぶ側へ伝えること。
///
/// [`Gui::drain_actions`](crate::gui::Gui::drain_actions) で取り出します。
#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Action {
    /// 誰が出したか。
    pub widget: WidgetId,
    /// 名前が付いていれば。付け方は
    /// [`WidgetTree::set_tag`](crate::gui::tree::WidgetTree::set_tag)。
    pub tag: Option<Tag>,
    pub kind: ActionKind,
}

impl Action {
    /// その札のものか。
    pub fn is(&self, tag: Tag) -> bool {
        self.tag == Some(tag)
    }
}

/// 何が起きたか。
///
/// 細かい意味はウィジェットごとに違うので、**最小限しか決めていません。**
/// 足りなければ [`ActionKind::Custom`] に自分の番号を入れてください。
#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum ActionKind {
    /// 押して、上で放された。
    Clicked { button: PointerButton },
    /// 値が動いた。つまみを引いているあいだ毎フレーム出ます。
    ValueChanged,
    /// 確定した。入力欄で Enter、など。
    Submitted,
    /// 取り消された。Escape、など。
    Cancelled,
    /// 呼ぶ側が決める番号。
    Custom(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Empty;
    impl Widget for Empty {}

    #[test]
    fn default_behavior_is_inert() {
        // 既定は何もしない飾り。入力に関わるものは明示して立てる。
        assert_eq!(Empty.behavior(), Behavior::DECORATION);
        assert_eq!(Behavior::default(), Behavior::DECORATION);
    }

    #[test]
    fn the_preset_behaviors_differ_where_it_matters() {
        assert_ne!(Behavior::CONTROL, Behavior::DECORATION);
        assert_ne!(Behavior::CONTROL, Behavior::SURFACE);

        // SURFACE は入力を受けるが焦点は取らない。
        assert_eq!(
            Behavior::SURFACE,
            Behavior::DECORATION.interactive(true)
        );
        // CONTROL は SURFACE に焦点と掴みを足したもの。
        assert_eq!(
            Behavior::CONTROL,
            Behavior::SURFACE.focusable(true).captures_pointer(true)
        );
    }

    #[test]
    fn overlay_is_off_by_default() {
        assert!(!Empty.behavior().overlay);
        assert!(Behavior::CONTROL.overlay(true).overlay);
        assert_ne!(Behavior::CONTROL, Behavior::CONTROL.overlay(true));
    }

    #[test]
    fn armed_needs_both_pressed_and_hovered() {
        let dragged_away = WidgetState {
            pressed: true,
            hovered: false,
            ..Default::default()
        };

        assert!(!dragged_away.armed(), "外へずらしたら決まらない");
        assert!(
            WidgetState {
                pressed: true,
                hovered: true,
                ..Default::default()
            }
            .armed()
        );
    }

    #[test]
    fn hit_test_excludes_the_far_edge() {
        let size = Size::new(10.0, 10.0);

        assert!(Empty.hit_test(Point::new(0.0, 0.0), size));
        assert!(!Empty.hit_test(Point::new(10.0, 5.0), size));
        assert!(!Empty.hit_test(Point::new(-1.0, 5.0), size));
    }

    #[test]
    fn downcasting_recovers_the_concrete_widget() {
        let widget: Box<dyn Widget> = Box::new(Empty);

        assert!(widget.downcast_ref::<Empty>().is_some());
    }
}
