//! 浮いた窓。題名の帯を掴んで動かせます。
//!
//! # 元の下書きからの変更点
//!
//! | 下書き | いま | なぜ |
//! |---|---|---|
//! | `theme: Theme` を持つ | [`Role`] を持つ | 窓ごとに色の表を複製すると、配色を替えるのに全部の窓を触ることになる。表は [`Gui`](crate::gui::Gui) が 1 つ持ち、窓は「自分は何の役か」だけ言う |
//! | `items: Vec<Box<dyn ItemTrait>>` | 木の子 | 子を自分で持つと、親を触りながら子へ降りられない。[`crate::gui::tree`] の最初の節を参照 |
//! | `add_item(item)` | [`WidgetTree::add_child`](crate::gui::tree::WidgetTree::add_child) | 同じ |
//! | `x, y, width, height` | [`Point`] と [`Size`] | 負の大きさが入らなくなる |
//!
//! # 置き場所は親が決める、位置は自分が言う
//!
//! [`WindowFrame`] は [`Widget::absolute_position`] で自分の左上を言います。
//! これを見てくれるのは [`Floating`](crate::gui::container::Floating) だけなので、
//! **浮かせたい窓は `Floating` の子にしてください。**
//! [`Stack`](crate::gui::container::Stack) の子にすると、位置は無視されて
//! 並びに従います（固定の板として使う分にはこれで困りません）。
//!
//! ```no_run
//! # use gueiz_2d::gui::container::Floating;
//! # use gueiz_2d::gui::geometry::{Point, Size};
//! # use gueiz_2d::gui::theme::Role;
//! # use gueiz_2d::gui::window_frame::WindowFrame;
//! # use gueiz_2d::gui::Gui;
//! # fn build(gui: &mut Gui) {
//! # fn some_widget() -> Box<dyn gueiz_2d::gui::Widget> { unimplemented!() }
//! let desktop = gui.set_root(Box::new(Floating::new()));
//!
//! let frame = gui.tree_mut().add_child(
//!     desktop,
//!     Box::new(
//!         WindowFrame::new(Point::new(40.0, 40.0), Size::new(320.0, 240.0))
//!             .title("設定")
//!             .movable(true),
//!     ),
//! );
//!
//! // 下書きの `add_item` はこれ。
//! let first = gui.tree_mut().add_child(frame, some_widget());
//! # let _ = (first, Role::SURFACE);
//! # }
//! ```

use crate::gui::event::{Event, EventResult, PointerButton};
use crate::gui::geometry::{Align, Corners, Insets, Point, Rect, Size};
use crate::gui::layout::Constraints;
use crate::gui::painter::{Painter, TextLayoutOptions};
use crate::gui::theme::Role;
use crate::gui::widget::Behavior;
use crate::gui::{ArrangeContext, EventContext, MeasureContext, PaintContext, Widget};

/// 浮いた窓。
pub struct WindowFrame {
    position: Point,
    size: Size,

    role: Role,
    title_role: Role,

    title: Option<String>,
    /// 題名の帯の高さ。0 なら帯なし。
    title_bar_height: f32,

    /// 中身の周りの余白。`None` なら [`Metrics::spacing`](crate::gui::theme::Metrics) から。
    padding: Option<Insets>,
    gap: f32,

    movable: bool,
    /// 掴んでいるあいだ立つ。
    dragging: bool,
}

impl WindowFrame {
    pub fn new(position: Point, size: Size) -> Self {
        Self {
            position,
            size,
            role: Role::SURFACE,
            title_role: Role::ACCENT,
            title: None,
            title_bar_height: 28.0,
            padding: None,
            gap: 0.0,
            movable: false,
            dragging: false,
        }
    }

    /// `x, y, width, height` から。下書きの形に合わせた入口。
    pub fn at(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self::new(Point::new(x, y), Size::new(width, height))
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// 題名の帯を消す。掴んで動かせなくなります。
    pub fn without_title_bar(mut self) -> Self {
        self.title_bar_height = 0.0;
        self
    }

    pub fn title_bar_height(mut self, height: f32) -> Self {
        self.title_bar_height = height.max(0.0);
        self
    }

    /// 窓そのものの役。
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// 題名の帯の役。
    pub fn title_role(mut self, title_role: Role) -> Self {
        self.title_role = title_role;
        self
    }

    pub fn padding(mut self, padding: Insets) -> Self {
        self.padding = Some(padding);
        self
    }

    /// 中身の子と子のあいだ。
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// 題名の帯を掴んで動かせるようにする。
    pub fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }

    pub fn position(&self) -> Point {
        self.position
    }

    /// 動かす。**呼んだら
    /// [`WidgetTree::request_layout`](crate::gui::tree::WidgetTree::request_layout)
    /// も呼んでください。**
    pub fn set_position(&mut self, position: Point) {
        self.position = position;
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// 大きさを変える。測り直しが要ります。
    pub fn set_size(&mut self, size: Size) {
        self.size = size;
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// 題名の帯。帯が無ければ空。
    pub fn title_bar(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.width, self.title_bar_height)
    }

    /// 中身が入る範囲。帯の下、余白の内側。
    pub fn content_area(&self, padding: Insets) -> Rect {
        Rect::new(
            0.0,
            self.title_bar_height,
            self.size.width,
            (self.size.height - self.title_bar_height).max(0.0),
        )
        .deflate(padding)
    }

    fn resolved_padding(&self, spacing: f32) -> Insets {
        self.padding.unwrap_or(Insets::all(spacing))
    }
}

impl Widget for WindowFrame {
    /// 言われた大きさのまま。中身がはみ出しても窓は広がりません。
    ///
    /// 窓は「この大きさの枠」であって「中身に合わせる袋」ではないので、
    /// 中身で大きさが動くと、字が 1 文字増えるたびに窓が動いて落ち着きません。
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        let padding = self.resolved_padding(context.metrics().spacing);
        let content = self.content_area(padding).size();

        // 中身には「ここに収めてくれ」とだけ言う。答えは使わないが、
        // 測らないと子の `desired` が埋まらず、置けない。
        //
        // **縦の上限を外してはいけません。** 窓の丈は決まっているので、
        // 中身に「いくらでも伸びてよい」と言う理由がありません。外すと
        // [`ScrollArea`](crate::gui::scroll_area::ScrollArea) を入れたときに
        // 「はみ出す」という状態が成立せず、**送れない箱**になります
        // （中身の丈そのままの大きさになり、窓で切り取られるだけ）。
        //
        // 上限を渡しても、中身が溢れる形（長い [`Stack`](crate::gui::container::Stack)）は
        // 自分の矩形の外へ子を置くので、これまでどおり窓で切り取られます。
        let inner = Constraints::loose(content);

        for child in context.children() {
            context.measure_child(child, inner);
        }

        constraints.constrain(self.size)
    }

    fn arrange(&mut self, context: &mut ArrangeContext<'_>, bounds: Rect) {
        let padding = self.resolved_padding(context.metrics().spacing);
        let area = self.content_area(padding).intersect(bounds);

        let mut y = area.y;

        // 中身は縦に積む。並べ方を細かく決めたいなら、子に
        // `Stack` を 1 つ入れてそこへ足してください。
        for child in context.children() {
            let height = context.desired(child).height;

            context.place(child, Rect::new(area.x, y, area.width, height));
            y += height + self.gap;
        }
    }

    fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let style = context.style(self.role);
        let frame = context.rect();

        painter.rounded_rect(frame, style.corners, style.background);

        if self.title_bar_height <= 0.0 {
            return;
        }

        let title_style = context.style(self.title_role);
        let bar = self.title_bar();

        // 帯は上の角だけ丸める。下は中身と地続き。
        let bar_corners = Corners::new(
            style.corners.top_left,
            style.corners.top_right,
            0.0,
            0.0,
        );

        painter.rounded_rect(bar, bar_corners, title_style.background);

        if let Some(title) = &self.title {
            let text_area = bar.deflate(Insets::symmetric(0.0, title_style.padding.left.max(8.0)));

            painter.text(
                text_area,
                title.as_str(),
                TextLayoutOptions {
                    size: title_style.text_size,
                    horizontal: Align::Start,
                    vertical: Align::Center,
                    wrap: false,
                },
                title_style.foreground,
            );
        }
    }

    /// 枠線は中身の上に乗せる。先に描くと、中身の縁で隠れる。
    fn paint_foreground(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let style = context.style(self.role);

        if style.border_width <= 0.0 || style.border.is_invisible() {
            return;
        }

        painter.border(
            context.rect(),
            style.corners,
            style.border_width,
            style.border,
        );
    }

    fn on_event(&mut self, context: &mut EventContext<'_>, event: &Event) -> EventResult {
        match event {
            Event::PointerPressed {
                position,
                button: PointerButton::Primary,
            } => {
                // 帯の上でだけ掴む。中身の上で掴むと、中のつまみが動かせない。
                if !self.movable || !self.title_bar().contains(*position) {
                    // 窓の中を押したことは受け取る。押し抜けて後ろの窓が前に
                    // 出てしまうのを防ぐため。
                    return EventResult::Consumed;
                }

                self.dragging = true;
                context.capture_pointer();

                EventResult::Consumed
            }

            Event::PointerMove { delta, .. } if self.dragging => {
                // 動いた量で足す。画面座標から引くと、親がずれているときに飛ぶ。
                self.position += *delta;
                context.request_layout();

                EventResult::Consumed
            }

            Event::PointerReleased { .. } if self.dragging => {
                self.dragging = false;
                context.release_pointer();

                EventResult::Consumed
            }

            // 窓の上の入力は窓で止める。後ろの窓には渡さない。
            event if event.is_pointer() => EventResult::Consumed,

            _ => EventResult::Ignored,
        }
    }

    fn absolute_position(&self) -> Option<Point> {
        Some(self.position)
    }

    fn clip_corners(&self) -> Corners {
        // 中身が角から出ないように。丸みは見た目の表に従う。
        Corners::ZERO
    }

    fn behavior(&self) -> Behavior {
        Behavior::SURFACE
            .captures_pointer(self.movable)
            // 中身が窓からはみ出して見えないようにする。
            .clips_children(true)
    }
}

/// 下書きにあった空の `Theme` の行き先。
///
/// 色の表は [`crate::gui::theme::Theme`] に移り、[`Gui`](crate::gui::Gui) が
/// 1 つ持ちます。窓は [`WindowFrame::role`] で「自分は何の役か」を言うだけです。
///
/// この型は目印として残してあります。使うものではありません。
#[deprecated(note = "色の表は gui::theme::Theme にあります。WindowFrame::role を使ってください")]
pub struct Theme;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::color::Color;
    use crate::gui::container::Floating;
    use crate::gui::context::NoTextMeasure;
    use crate::gui::event::InputEvent;
    use crate::gui::theme::{StateKey, Style, Theme as GuiTheme};
    use crate::gui::Gui;

    struct Item(Size);

    impl Widget for Item {
        fn measure(&mut self, _: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
            constraints.constrain(self.0)
        }
    }

    fn desktop() -> (Gui, crate::gui::id::WidgetId) {
        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(Floating::new()));

        (gui, root)
    }

    fn laid_out(gui: &mut Gui) {
        gui.layout(Size::new(800.0, 600.0), &NoTextMeasure);
    }

    #[test]
    fn a_frame_sits_where_it_says() {
        let (mut gui, root) = desktop();

        let frame = gui.tree_mut().add_child(
            root,
            Box::new(WindowFrame::at(40.0, 60.0, 320.0, 240.0)),
        );

        laid_out(&mut gui);

        assert_eq!(
            gui.tree().bounds(frame),
            Rect::new(40.0, 60.0, 320.0, 240.0)
        );
    }

    #[test]
    fn content_starts_below_the_title_bar() {
        let (mut gui, root) = desktop();

        let frame = gui.tree_mut().add_child(
            root,
            Box::new(
                WindowFrame::at(0.0, 0.0, 200.0, 200.0)
                    .title_bar_height(30.0)
                    .padding(Insets::all(10.0)),
            ),
        );
        let item = gui
            .tree_mut()
            .add_child(frame, Box::new(Item(Size::new(50.0, 20.0))));

        laid_out(&mut gui);

        // 帯 30 + 余白 10 = y 40、x は余白ぶんの 10。
        let bounds = gui.tree().bounds(item);
        assert_eq!(bounds.y, 40.0);
        assert_eq!(bounds.x, 10.0);
        assert_eq!(bounds.width, 180.0, "横は中身の幅いっぱい");
    }

    #[test]
    fn items_stack_with_the_gap() {
        let (mut gui, root) = desktop();

        let frame = gui.tree_mut().add_child(
            root,
            Box::new(
                WindowFrame::at(0.0, 0.0, 200.0, 300.0)
                    .without_title_bar()
                    .padding(Insets::ZERO)
                    .gap(6.0),
            ),
        );

        let first = gui
            .tree_mut()
            .add_child(frame, Box::new(Item(Size::new(50.0, 20.0))));
        let second = gui
            .tree_mut()
            .add_child(frame, Box::new(Item(Size::new(50.0, 40.0))));

        laid_out(&mut gui);

        assert_eq!(gui.tree().bounds(first).y, 0.0);
        assert_eq!(gui.tree().bounds(second).y, 26.0, "20 + 6");
    }

    /// **窓に入れた送り箱が送れること。**
    ///
    /// 中身を縦の上限なしで測ると、送り箱は「はみ出していない」と判断して
    /// 中身の丈そのままになります。窓で切り取られるので中身は見えなくなるのに、
    /// 送ることはできない、という形になっていました。
    #[test]
    fn a_scroll_area_inside_a_frame_can_actually_scroll() {
        use crate::gui::scroll_area::ScrollArea;

        let (mut gui, root) = desktop();

        let frame = gui.tree_mut().add_child(
            root,
            Box::new(
                WindowFrame::at(0.0, 0.0, 200.0, 100.0)
                    .without_title_bar()
                    .padding(Insets::ZERO),
            ),
        );
        let area = gui
            .tree_mut()
            .add_child(frame, Box::new(ScrollArea::vertical()));
        gui.tree_mut()
            .add_child(area, Box::new(Item(Size::new(50.0, 500.0))));

        laid_out(&mut gui);

        let scroll = gui.tree().get_as::<ScrollArea>(area).expect("ScrollArea");

        assert_eq!(
            gui.tree().bounds(area).height,
            100.0,
            "送り箱は窓の丈まで",
        );
        assert_eq!(scroll.content_length(), 500.0);
        assert_eq!(scroll.max_offset(), 400.0, "送れる");
    }

    #[test]
    fn dragging_the_title_bar_moves_the_frame() {
        let (mut gui, root) = desktop();

        let frame = gui.tree_mut().add_child(
            root,
            Box::new(
                WindowFrame::at(100.0, 100.0, 200.0, 150.0)
                    .title_bar_height(24.0)
                    .movable(true),
            ),
        );

        laid_out(&mut gui);

        // 帯の上を押す。
        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(150.0, 110.0),
            button: PointerButton::Primary,
        });

        assert!(gui.tree().get_as::<WindowFrame>(frame).unwrap().is_dragging());
        assert_eq!(gui.pointer_capture(), frame);

        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(200.0, 160.0),
        });

        laid_out(&mut gui);

        assert_eq!(
            gui.tree().bounds(frame).origin(),
            Point::new(150.0, 150.0),
            "動いた量ぶん移る"
        );

        gui.handle_input(InputEvent::PointerReleased {
            position: Point::new(200.0, 160.0),
            button: PointerButton::Primary,
        });

        assert!(!gui.tree().get_as::<WindowFrame>(frame).unwrap().is_dragging());
        assert_eq!(gui.pointer_capture(), crate::gui::id::WidgetId::NONE);
    }

    #[test]
    fn pressing_the_body_does_not_drag() {
        let (mut gui, root) = desktop();

        let frame = gui.tree_mut().add_child(
            root,
            Box::new(
                WindowFrame::at(0.0, 0.0, 200.0, 200.0)
                    .title_bar_height(24.0)
                    .movable(true),
            ),
        );

        laid_out(&mut gui);

        // 帯より下を押す。
        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(100.0, 100.0),
            button: PointerButton::Primary,
        });

        assert!(!gui.tree().get_as::<WindowFrame>(frame).unwrap().is_dragging());

        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(300.0, 300.0),
        });
        laid_out(&mut gui);

        assert_eq!(gui.tree().bounds(frame).origin(), Point::ZERO, "動かない");
    }

    #[test]
    fn a_frame_swallows_pointer_events() {
        let (mut gui, root) = desktop();

        let under = gui.tree_mut().add_child(
            root,
            Box::new(WindowFrame::at(0.0, 0.0, 300.0, 300.0)),
        );
        let over = gui.tree_mut().add_child(
            root,
            Box::new(WindowFrame::at(50.0, 50.0, 100.0, 100.0)),
        );

        laid_out(&mut gui);

        // 重なっているところは手前の窓のもの。
        assert_eq!(gui.hit_test(Point::new(100.0, 100.0)), over);
        assert_eq!(gui.hit_test(Point::new(250.0, 250.0)), under);
    }

    #[test]
    fn the_frame_paints_background_then_bar_then_border() {
        let (mut gui, root) = desktop();

        let mut theme = GuiTheme::new();
        theme.set(
            Role::SURFACE,
            StateKey::Normal,
            Style::BARE
                .background(Color::hex(0x202020))
                .border(Color::WHITE, 1.0)
                .corners(Corners::all(6.0)),
        );
        theme.set(
            Role::ACCENT,
            StateKey::Normal,
            Style::BARE.background(Color::hex(0x404040)),
        );
        gui.set_theme(theme);

        gui.tree_mut().add_child(
            root,
            Box::new(WindowFrame::at(10.0, 10.0, 200.0, 100.0).title_bar_height(20.0)),
        );

        laid_out(&mut gui);
        let list = gui.paint();

        assert_eq!(list.len(), 3, "{:?}", list.commands());

        // 背景 → 帯 → 枠線（枠線は子より後ろの周回で出る）。
        let bounds: Vec<_> = list
            .commands()
            .iter()
            .map(|command| command.primitive.bounds())
            .collect();

        assert_eq!(bounds[0], Rect::new(10.0, 10.0, 200.0, 100.0));
        assert_eq!(bounds[1], Rect::new(10.0, 10.0, 200.0, 20.0));
        assert_eq!(bounds[2], Rect::new(10.0, 10.0, 200.0, 100.0));
    }

    #[test]
    fn children_are_clipped_to_the_frame() {
        let (mut gui, root) = desktop();

        let frame = gui.tree_mut().add_child(
            root,
            Box::new(
                WindowFrame::at(0.0, 0.0, 100.0, 60.0)
                    .without_title_bar()
                    .padding(Insets::ZERO),
            ),
        );

        struct Big;
        impl Widget for Big {
            fn measure(&mut self, _: &mut MeasureContext<'_>, _: Constraints) -> Size {
                Size::new(1000.0, 1000.0)
            }
            fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
                painter.rect(context.rect(), Color::WHITE);
            }
        }

        gui.tree_mut().add_child(frame, Box::new(Big));
        laid_out(&mut gui);

        let list = gui.paint();
        let child = list
            .commands()
            .iter()
            .find(|command| command.clip.is_some())
            .expect("子は切り抜かれる");

        assert_eq!(child.clip.unwrap().rect, Rect::new(0.0, 0.0, 100.0, 60.0));
    }
}
