//! 押せるもの。
//!
//! # 「押された」を出すのは誰か
//!
//! [`Gui`](crate::gui::Gui) です。押して、**同じウィジェットの上で**放すと、
//! [`ActionKind::Clicked`] が自動で積まれます。だから [`Button`] は
//! ポインタについては何も出しません。出すと二重になります。
//!
//! 出すのは**鍵盤で押されたとき**だけです。`Space` と `Enter` は
//! 入力としては [`Event::KeyPressed`] で来るだけで、押下として扱うかは
//! ウィジェットが決めることなので、[`Gui`](crate::gui::Gui) は判断しません。
//!
//! ```text
//! ポインタ: 押す ──▶ 放す（上で）──▶ Gui が Clicked を積む
//! 鍵盤:     Space を押す ──▶ Button が Clicked を積む
//! ```
//!
//! # 見た目は全部 [`Theme`] から
//!
//! [`Button`] は色も丸みも持ちません。[`Role`]（既定は [`Role::CONTROL`]）で
//! 「自分は押せるものだ」とだけ言い、値は表から引きます。
//!
//! | [`StateKey`] | いつ |
//! |---|---|
//! | `Normal` | 何もしていない |
//! | `Hovered` | ポインタが乗っている |
//! | `Pressed` | 押されている（鍵盤で押されているあいだも） |
//! | `Focused` | 焦点があり、乗ってもいない |
//! | `Disabled` | 無効 |
//!
//! 登録していない状態は `Normal` に落ちるので、**全部埋める必要はありません。**
//!
//! ## 焦点の輪
//!
//! `Hovered` のほうが `Focused` より強いので（[`StateKey`] の優先順）、
//! 乗っているあいだ `Focused` の型は出てきません。それでも輪を出したいのが
//! ふつうなので、[`Button`] は**「`(役, Focused)` が表に登録されていれば、
//! その枠線を上から重ねて引く」**ようにしています。
//!
//! つまり輪が要るなら `(役, Focused)` に枠線付きの [`Style`] を登録し、
//! 要らないなら登録しないでください。[`Button`] の側に輪の有無はありません。
//!
//! ```no_run
//! # use gueiz_2d::gui::color::Color;
//! # use gueiz_2d::gui::theme::{Role, StateKey, Style, Theme};
//! # let mut theme = Theme::new();
//! // 焦点のときだけ、2 px の輪を重ねる。
//! theme.set(
//!     Role::CONTROL,
//!     StateKey::Focused,
//!     Style::BARE.border(Color::hex(0x88ccff), 2.0),
//! );
//! ```
//!
//! # 使い方
//!
//! ```no_run
//! # use gueiz_2d::gui::button::Button;
//! # use gueiz_2d::gui::id::Tag;
//! # use gueiz_2d::gui::widget::ActionKind;
//! # use gueiz_2d::gui::Gui;
//! # fn build(gui: &mut Gui, parent: gueiz_2d::gui::id::WidgetId) {
//! const OK: Tag = Tag::new("ok");
//!
//! let ok = gui.tree_mut().add_child(parent, Box::new(Button::new("決定")));
//! gui.tree_mut().set_tag(ok, OK);
//!
//! // 毎フレーム。
//! for action in gui.drain_actions() {
//!     if action.is(OK) && matches!(action.kind, ActionKind::Clicked { .. }) {
//!         println!("決定が押された");
//!     }
//! }
//! # }
//! ```
//!
//! # 書体が字を持っていないとき
//!
//! [`crate::text`] は**持っていない文字を黙って飛ばします**（豆腐は出しません）。
//! `arial.ttf` で `やめる` と書くと字が 1 つも出ず、幅は余白だけになります。
//!
//! 日本語の札を使うなら、仮名と漢字を持つ書体を渡してください。
//! 幅が崩れるのを避けたいだけなら [`Button::min_width`] で下限を敷けます。
//!
//! [`Event::KeyPressed`]: crate::gui::event::Event::KeyPressed
//! [`Theme`]: crate::gui::theme::Theme
//! [`Style`]: crate::gui::theme::Style

use crate::gui::event::{Event, EventResult, Key, NamedKey, PointerButton};
use crate::gui::geometry::{Align, Rect, Size};
use crate::gui::layout::Constraints;
use crate::gui::painter::{Painter, TextLayoutOptions};
use crate::gui::theme::{Role, StateKey};
use crate::gui::widget::{ActionKind, Behavior, WidgetState};
use crate::gui::{ArrangeContext, EventContext, MeasureContext, PaintContext, Widget};

/// 押せるもの。
pub struct Button {
    label: String,
    role: Role,

    horizontal: Align,
    /// 中身の最小の幅。`None` なら字の幅のまま。
    min_width: Option<f32>,
    /// 最小の高さ。`None` なら [`Metrics::control_height`](crate::gui::theme::Metrics) から。
    min_height: Option<f32>,
    /// 字を矩形の幅で折り返すか。
    wrap: bool,

    /// 鍵盤で押されているあいだ立つ。
    ///
    /// [`WidgetState::pressed`] はポインタでしか動かないので、
    /// 鍵盤の押下は自分で覚える必要がある。
    key_held: bool,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            role: Role::CONTROL,
            horizontal: Align::Center,
            min_width: None,
            min_height: None,
            wrap: false,
            key_held: false,
        }
    }

    /// 見た目を引く役。危険な操作だけ色を変えたいときなどに
    /// [`Role::custom`] を渡します。
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// 字の横の寄せ。既定は中央。
    pub fn align(mut self, horizontal: Align) -> Self {
        self.horizontal = horizontal;
        self
    }

    /// 最小の幅。並べたボタンの幅を揃えるときに。
    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = Some(min_width.max(0.0));
        self
    }

    /// 最小の高さ。既定は
    /// [`Metrics::control_height`](crate::gui::theme::Metrics)。
    pub fn min_height(mut self, min_height: f32) -> Self {
        self.min_height = Some(min_height.max(0.0));
        self
    }

    /// 字を折り返す。2 行になるぶん高くなります。
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// 札を書き換える。
    ///
    /// **幅が変わるので、呼んだら
    /// [`WidgetTree::request_layout`](crate::gui::tree::WidgetTree::request_layout)
    /// も呼んでください。**
    ///
    /// ```no_run
    /// # use gueiz_2d::gui::button::Button;
    /// # use gueiz_2d::gui::id::WidgetId;
    /// # use gueiz_2d::gui::Gui;
    /// # fn rename(gui: &mut Gui, id: WidgetId) {
    /// if let Some(button) = gui.tree_mut().get_as_mut::<Button>(id) {
    ///     button.set_label("やめる");
    /// }
    ///
    /// gui.tree_mut().request_layout(id);
    /// # }
    /// ```
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// 鍵盤で押されているか。
    pub fn is_key_held(&self) -> bool {
        self.key_held
    }

    /// 見た目を引くときの状態。鍵盤の押下もここで混ぜる。
    fn visual_state(&self, state: WidgetState) -> WidgetState {
        WidgetState {
            pressed: state.pressed || self.key_held,
            ..state
        }
    }

    /// 字の置き方。
    fn text_options(&self, text_size: f32) -> TextLayoutOptions {
        TextLayoutOptions {
            size: text_size,
            horizontal: self.horizontal,
            vertical: Align::Center,
            wrap: self.wrap,
        }
    }

    /// その鍵で押したことになるか。
    ///
    /// `Space` と `Enter` だけ。矢印や Tab は移動に使うので取りません。
    fn activates(key: &Key) -> bool {
        key.is_named(NamedKey::Space) || key.is_named(NamedKey::Enter)
    }
}

impl Widget for Button {
    fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        // 余白と字の大きさは、いまの状態の型から取る。押したときだけ
        // 余白が変わる作りでも、測りが追いつく。
        let style = context
            .theme()
            .style(self.role, self.visual_state(context.state()));

        let padding = style.padding;
        let inner = constraints.deflate(padding);

        // 折り返すときだけ幅の上限を渡す。上限が無ければ折り返せない。
        let max_width = inner
            .max
            .width
            .is_finite()
            .then_some(inner.max.width);

        let text = context.measure_text(
            &self.label,
            self.text_options(style.text_size),
            max_width.filter(|_| self.wrap),
        );

        let min_height = self
            .min_height
            .unwrap_or(context.metrics().control_height);

        let width = text.width.max(self.min_width.unwrap_or(0.0)) + padding.horizontal();
        let height = (text.height + padding.vertical()).max(min_height);

        constraints.constrain(Size::new(width, height))
    }

    /// 子は取りません。札しか中身が無いので、置くものがない。
    fn arrange(&mut self, context: &mut ArrangeContext<'_>, _bounds: Rect) {
        if context.child_count() > 0 {
            log::warn!(
                "Button has {} children; they will not be placed. \
                 use a Stack if you need to put things inside",
                context.child_count()
            );
        }
    }

    fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        let state = self.visual_state(context.state());
        let style = context.theme().style(self.role, state);
        let frame = context.rect();

        painter.rounded_rect(frame, style.corners, style.background);

        if style.border_width > 0.0 {
            painter.border(frame, style.corners, style.border_width, style.border);
        }

        if !self.label.is_empty() {
            painter.text(
                frame.deflate(style.padding),
                self.label.as_str(),
                self.text_options(style.text_size),
                style.foreground,
            );
        }

        // 焦点の輪。`Hovered` のほうが強いので、型を引くと出てこない。
        // 表に `(役, Focused)` が入っているときだけ、その枠線を重ねる。
        if !state.focused || state.disabled {
            return;
        }

        let Some(focus) = context.theme().style_exact(self.role, StateKey::Focused) else {
            return;
        };

        if focus.border_width > 0.0 && !focus.border.is_invisible() {
            painter.border(frame, style.corners, focus.border_width, focus.border);
        }
    }

    fn on_event(&mut self, context: &mut EventContext<'_>, event: &Event) -> EventResult {
        match event {
            // 押下そのものは `Gui` が `Clicked` にしてくれる。
            // 取っておくのは、後ろのパネルに抜けないようにするため。
            Event::PointerPressed { .. } | Event::PointerReleased { .. } => EventResult::Consumed,

            Event::KeyPressed { key, repeat } if Self::activates(key) => {
                // 押しっぱなしの連射では撃たない。ボタンは 1 回押して 1 回。
                if *repeat {
                    return EventResult::Consumed;
                }

                self.key_held = true;
                context.emit(ActionKind::Clicked {
                    button: PointerButton::Primary,
                });

                EventResult::Consumed
            }

            Event::KeyReleased { key } if Self::activates(key) => {
                self.key_held = false;
                EventResult::Consumed
            }

            // 焦点が外れたら、押していた見た目を戻す。
            // 押したまま Tab で移ると、押された形のまま残ってしまう。
            Event::FocusLost => {
                self.key_held = false;
                EventResult::Ignored
            }

            _ => EventResult::Ignored,
        }
    }

    fn behavior(&self) -> Behavior {
        Behavior::CONTROL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::color::Color;
    use crate::gui::container::Stack;
    use crate::gui::context::TextMeasure;
    use crate::gui::event::InputEvent;
    use crate::gui::geometry::{Corners, Insets, Point};
    use crate::gui::id::{Tag, WidgetId};
    use crate::gui::painter::Primitive;
    use crate::gui::theme::{Style, Theme};
    use crate::gui::Gui;

    /// 字 1 つを `size * 0.5` 幅、行は `size` 高さとして測る。
    struct Monospace;

    impl TextMeasure for Monospace {
        fn measure_text(
            &self,
            text: &str,
            options: TextLayoutOptions,
            max_width: Option<f32>,
        ) -> Size {
            let advance = options.size * 0.5;
            let columns = text.chars().count() as f32;

            let per_line = match max_width.filter(|_| options.wrap) {
                Some(max_width) => (max_width / advance).floor().max(1.0),
                None => columns.max(1.0),
            };

            let lines = (columns / per_line).ceil().max(1.0);

            Size::new(columns.min(per_line) * advance, lines * options.size)
        }

        fn line_height(&self, text_size: f32) -> f32 {
            text_size
        }
    }

    fn theme() -> Theme {
        let mut theme = Theme::new();

        theme.set(
            Role::CONTROL,
            StateKey::Normal,
            Style::BARE
                .background(Color::hex(0x334455))
                .foreground(Color::WHITE)
                .text_size(16.0)
                .padding(Insets::symmetric(4.0, 12.0))
                .corners(Corners::all(4.0)),
        );
        theme.set(
            Role::CONTROL,
            StateKey::Hovered,
            Style::BARE
                .background(Color::hex(0x445566))
                .foreground(Color::WHITE)
                .text_size(16.0)
                .padding(Insets::symmetric(4.0, 12.0)),
        );
        theme.set(
            Role::CONTROL,
            StateKey::Pressed,
            Style::BARE
                .background(Color::hex(0x223344))
                .foreground(Color::WHITE)
                .text_size(16.0)
                .padding(Insets::symmetric(4.0, 12.0)),
        );

        theme
    }

    /// ボタン 1 つを縦並びの中に置いた `Gui`。
    fn with_button(button: Button) -> (Gui, WidgetId) {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let root = gui.set_root(Box::new(Stack::column()));
        let id = gui.tree_mut().add_child(root, Box::new(button));

        (gui, id)
    }

    fn laid_out(gui: &mut Gui) {
        gui.layout(Size::new(400.0, 300.0), &Monospace);
    }

    fn background_of(gui: &mut Gui, id: WidgetId) -> Color {
        let list = gui.paint();

        let command = list
            .commands()
            .iter()
            .find(|command| command.owner == id)
            .expect("ボタンは必ず背景を描く");

        match command.primitive {
            Primitive::Rect { color, .. } => color,
            ref other => panic!("最初は塗った矩形のはず: {other:?}"),
        }
    }

    fn click(gui: &mut Gui, at: Point) {
        gui.handle_input(InputEvent::PointerMoved { position: at });
        gui.handle_input(InputEvent::PointerPressed {
            position: at,
            button: PointerButton::Primary,
        });
        gui.handle_input(InputEvent::PointerReleased {
            position: at,
            button: PointerButton::Primary,
        });
    }

    #[test]
    fn it_sizes_itself_to_the_label_plus_padding() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);

        // 字 2 つ × 8 px + 左右 12 px = 40。
        // 高さは字 16 + 上下 4 = 24 だが、control_height 32 で止まる。
        assert_eq!(gui.tree().bounds(id).size(), Size::new(40.0, 32.0));
    }

    #[test]
    fn min_width_pads_the_narrow_ones() {
        let (mut gui, id) = with_button(Button::new("はい").min_width(100.0));
        laid_out(&mut gui);

        assert_eq!(gui.tree().bounds(id).width, 124.0, "100 + 左右 12");
    }

    #[test]
    fn wrapping_makes_it_taller() {
        // 画面（400）から余白（24）を引いた 376 px に収まらない長さにする。
        // 字 1 つ 8 px なので 50 字で 400 px。
        let label = "a".repeat(50);

        let (mut gui, single) =
            with_button(Button::new(label.clone()).min_height(0.0));
        laid_out(&mut gui);
        let single_height = gui.tree().bounds(single).height;

        let (mut gui, wrapped) =
            with_button(Button::new(label).wrap(true).min_height(0.0));
        laid_out(&mut gui);
        let wrapped_height = gui.tree().bounds(wrapped).height;

        assert_eq!(single_height, 24.0, "1 行（字 16 + 上下 4）");
        assert_eq!(wrapped_height, 40.0, "2 行になって 16 px ぶん高い");
        assert!(wrapped_height > single_height);
    }

    #[test]
    fn a_pointer_click_emits_exactly_one_action() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);

        click(&mut gui, Point::new(10.0, 10.0));

        let actions = gui.drain_actions();

        assert_eq!(actions.len(), 1, "{actions:?}");
        assert_eq!(actions[0].widget, id);
        assert!(matches!(
            actions[0].kind,
            ActionKind::Clicked {
                button: PointerButton::Primary
            }
        ));
    }

    #[test]
    fn releasing_outside_emits_nothing() {
        let (mut gui, _) = with_button(Button::new("OK"));
        laid_out(&mut gui);

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
        });
        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(300.0, 250.0),
        });
        gui.handle_input(InputEvent::PointerReleased {
            position: Point::new(300.0, 250.0),
            button: PointerButton::Primary,
        });

        assert!(gui.drain_actions().is_empty(), "外で放したらやめた扱い");
    }

    #[test]
    fn space_and_enter_press_the_focused_button() {
        for key in [NamedKey::Space, NamedKey::Enter] {
            let (mut gui, id) = with_button(Button::new("OK"));
            laid_out(&mut gui);

            gui.set_focus(id);

            gui.handle_input(InputEvent::KeyPressed {
                key: Key::Named(key),
                repeat: false,
            });

            let actions = gui.drain_actions();
            assert_eq!(actions.len(), 1, "{key:?}: {actions:?}");

            // 押しているあいだは押された見た目。
            assert!(gui.tree().get_as::<Button>(id).unwrap().is_key_held());

            gui.handle_input(InputEvent::KeyReleased {
                key: Key::Named(key),
            });
            assert!(!gui.tree().get_as::<Button>(id).unwrap().is_key_held());
        }
    }

    #[test]
    fn a_held_key_does_not_repeat() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);
        gui.set_focus(id);

        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Named(NamedKey::Space),
            repeat: false,
        });

        for _ in 0..5 {
            gui.handle_input(InputEvent::KeyPressed {
                key: Key::Named(NamedKey::Space),
                repeat: true,
            });
        }

        assert_eq!(gui.drain_actions().len(), 1, "1 回押して 1 回");
    }

    #[test]
    fn other_keys_pass_through_to_the_parent() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);
        gui.set_focus(id);

        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Named(NamedKey::Tab),
            repeat: false,
        });
        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Character('a'),
            repeat: false,
        });

        assert!(gui.drain_actions().is_empty(), "Tab と文字は押下ではない");
    }

    #[test]
    fn losing_focus_clears_the_key_press() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);
        gui.set_focus(id);

        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Named(NamedKey::Space),
            repeat: false,
        });
        assert!(gui.tree().get_as::<Button>(id).unwrap().is_key_held());

        // 押したまま焦点を外す。押された形で固まらない。
        gui.set_focus(WidgetId::NONE);

        assert!(!gui.tree().get_as::<Button>(id).unwrap().is_key_held());
    }

    #[test]
    fn the_background_follows_the_state() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);

        let idle = background_of(&mut gui, id);

        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(10.0, 10.0),
        });
        let hovered = background_of(&mut gui, id);

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
        });
        let pressed = background_of(&mut gui, id);

        assert_ne!(idle, hovered);
        assert_ne!(hovered, pressed);
        assert_eq!(idle, Color::hex(0x334455));
        assert_eq!(pressed, Color::hex(0x223344));
    }

    #[test]
    fn a_keyboard_press_looks_pressed_too() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);

        let idle = background_of(&mut gui, id);
        gui.set_focus(id);

        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Named(NamedKey::Space),
            repeat: false,
        });

        assert_eq!(
            background_of(&mut gui, id),
            Color::hex(0x223344),
            "鍵盤で押しても押された型になる"
        );
        assert_ne!(idle, background_of(&mut gui, id));
    }

    #[test]
    fn a_disabled_button_takes_nothing() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);

        gui.tree_mut().set_disabled(id, true);

        click(&mut gui, Point::new(10.0, 10.0));

        assert!(gui.drain_actions().is_empty());
        assert_eq!(gui.hit_test(Point::new(10.0, 10.0)), WidgetId::NONE);
        assert!(!gui.tree().state(id).hovered);

        // 焦点も取れない。
        gui.set_focus(id);
        assert_eq!(gui.focused(), WidgetId::NONE);
    }

    #[test]
    fn the_focus_ring_appears_only_when_the_theme_asks_for_it() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);
        gui.set_focus(id);

        // 表に `(CONTROL, Focused)` が無い。輪は出ない。
        let without = gui.paint().commands_of(id).count();

        gui.theme_mut().set(
            Role::CONTROL,
            StateKey::Focused,
            Style::BARE.border(Color::hex(0x88ccff), 2.0),
        );

        let with = gui.paint().commands_of(id).count();

        assert_eq!(with, without + 1, "枠線 1 本ぶん増える");
    }

    #[test]
    fn buttons_line_up_in_a_row() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let root = gui.set_root(Box::new(Stack::row().gap(8.0)));
        let cancel = gui
            .tree_mut()
            .add_child(root, Box::new(Button::new("やめる")));
        let ok = gui.tree_mut().add_child(root, Box::new(Button::new("OK")));

        laid_out(&mut gui);

        let left = gui.tree().bounds(cancel);
        let right = gui.tree().bounds(ok);

        assert_eq!(right.x, left.right() + 8.0, "間隔ぶん空く");
        assert_eq!(left.height, right.height, "高さは揃う");
    }

    #[test]
    fn a_tag_identifies_which_button_was_pressed() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let ok_tag = Tag::new("ok");
        let cancel_tag = Tag::new("cancel");

        let root = gui.set_root(Box::new(Stack::column()));
        let ok = gui.tree_mut().add_child(root, Box::new(Button::new("OK")));
        let cancel = gui
            .tree_mut()
            .add_child(root, Box::new(Button::new("やめる")));

        gui.tree_mut().set_tag(ok, ok_tag);
        gui.tree_mut().set_tag(cancel, cancel_tag);

        laid_out(&mut gui);

        // 2 つめ（やめる）を押す。
        let at = gui.tree().bounds(cancel).center();
        click(&mut gui, at);

        let actions = gui.drain_actions();

        assert_eq!(actions.len(), 1);
        assert!(actions[0].is(cancel_tag));
        assert!(!actions[0].is(ok_tag));
    }

    #[test]
    fn changing_the_label_changes_the_width() {
        let (mut gui, id) = with_button(Button::new("OK"));
        laid_out(&mut gui);

        let narrow = gui.tree().bounds(id).width;

        gui.tree_mut()
            .get_as_mut::<Button>(id)
            .unwrap()
            .set_label("もっと長い札");
        gui.tree_mut().request_layout(id);

        laid_out(&mut gui);

        assert!(
            gui.tree().bounds(id).width > narrow,
            "{} > {narrow}",
            gui.tree().bounds(id).width
        );
    }

    #[test]
    fn tab_moves_between_buttons() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        let root = gui.set_root(Box::new(Stack::column()));
        let first = gui.tree_mut().add_child(root, Box::new(Button::new("1")));
        let second = gui.tree_mut().add_child(root, Box::new(Button::new("2")));

        laid_out(&mut gui);

        gui.focus_next();
        assert_eq!(gui.focused(), first);

        gui.focus_next();
        assert_eq!(gui.focused(), second);

        gui.focus_next();
        assert_eq!(gui.focused(), first, "最後まで行ったら戻る");
    }

    #[test]
    fn the_click_does_not_leak_to_the_panel_behind() {
        let mut gui = Gui::new();
        gui.set_theme(theme());

        // 背景のパネルは入力を取る。ボタンが取らないと、ここまで上がってくる。
        let root = gui.set_root(Box::new(
            Stack::column().behavior(Behavior::SURFACE),
        ));
        let button = gui.tree_mut().add_child(root, Box::new(Button::new("OK")));

        laid_out(&mut gui);

        let at = gui.tree().bounds(button).center();
        click(&mut gui, at);

        let actions = gui.drain_actions();

        assert_eq!(actions.len(), 1, "パネルの分は出ない: {actions:?}");
        assert_eq!(actions[0].widget, button);
    }
}
