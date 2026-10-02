//! 木を回す人。
//!
//! # 1 フレームの流れ
//!
//! ```text
//! handle_input を入力のぶんだけ ──▶ layout ──▶ paint ──▶ drain_actions
//! ```
//!
//! ```no_run
//! # use gueiz_2d::gui::context::{NoTextMeasure, TextMeasure};
//! # use gueiz_2d::gui::event::InputEvent;
//! # use gueiz_2d::gui::geometry::Size;
//! # use gueiz_2d::gui::Gui;
//! # fn frame(gui: &mut Gui, events: Vec<InputEvent>, viewport: Size) {
//! let text: &dyn TextMeasure = &NoTextMeasure;
//!
//! for event in events {
//!     gui.handle_input(event);
//! }
//!
//! gui.layout(viewport, text);
//! let display_list = gui.paint();
//! # let _ = display_list;
//!
//! for action in gui.drain_actions() {
//!     println!("{action:?}");
//! }
//! # }
//! ```
//!
//! # 入力は 1 フレーム遅れた形を見る
//!
//! [`Gui::handle_input`] は**前のフレームで決まった置き場所**で当たりを取ります。
//! 入力のたびに測り直すと、1 フレームに何十回も木を測ることになるからです。
//!
//! 実害が出るのは「押した瞬間に並びが変わって、押したつもりの場所に別のものが
//! 来る」場合だけで、そのときは次のフレームで正しくなります。
//!
//! ただし**最初のフレームだけは置き場所がありません。** 入力を流す前に
//! [`Gui::layout`] を 1 度呼んでください。呼ばないと、最初の 1 フレームの
//! 入力がどこにも当たりません。
//!
//! # 消えたものを指した鍵は捨てる
//!
//! 焦点・掴み・乗っている印はフレームをまたいで鍵で覚えています。
//! 指す先が消えていることがあるので、[`Gui::layout`] の頭で捨てます。
//! これがないと、消えたウィジェットに鍵盤の入力が吸われ続けます。

use crate::gui::context::{ArrangeContext, EventContext, MeasureContext, PaintContext, TextMeasure};
use crate::gui::event::{Event, EventResult, InputEvent, Modifiers, PointerButton, ScrollDelta};
use crate::gui::geometry::{Point, Rect, Size};
use crate::gui::id::WidgetId;
use crate::gui::layout::Constraints;
use crate::gui::painter::{ClipRegion, DisplayList};
use crate::gui::theme::Theme;
use crate::gui::tree::WidgetTree;
use crate::gui::widget::{Action, ActionKind, Widget, WidgetState};

/// UI 全体。木・見た目の表・入力の覚えを持ちます。
pub struct Gui {
    tree: WidgetTree,
    theme: Theme,
    display_list: DisplayList,
    /// 今フレームで浮かせるもの。毎フレーム詰め直す。
    overlays: Vec<WidgetId>,

    viewport: Size,
    scale_factor: f32,

    pointer: Option<Point>,
    modifiers: Modifiers,

    /// いまポインタが乗っているもの。
    hovered: WidgetId,
    /// 鍵盤の入力が届くもの。
    focused: WidgetId,
    /// 放すまで入力を独り占めするもの。
    capture: WidgetId,
    /// 押しているボタンと、押した先。
    pressed: Option<(PointerButton, WidgetId)>,

    actions: Vec<Action>,
}

impl Default for Gui {
    fn default() -> Self {
        Self::new()
    }
}

impl Gui {
    pub fn new() -> Self {
        Self {
            tree: WidgetTree::new(),
            theme: Theme::new(),
            display_list: DisplayList::new(),
            overlays: Vec::new(),
            viewport: Size::ZERO,
            scale_factor: 1.0,
            pointer: None,
            modifiers: Modifiers::empty(),
            hovered: WidgetId::NONE,
            focused: WidgetId::NONE,
            capture: WidgetId::NONE,
            pressed: None,
            actions: Vec::new(),
        }
    }

    // --- 持ち物 ---

    pub fn tree(&self) -> &WidgetTree {
        &self.tree
    }

    pub fn tree_mut(&mut self) -> &mut WidgetTree {
        &mut self.tree
    }

    /// 根を据える。
    pub fn set_root(&mut self, widget: Box<dyn Widget>) -> WidgetId {
        // 古い木ごと消えるので、覚えていた鍵も捨てる。
        self.hovered = WidgetId::NONE;
        self.focused = WidgetId::NONE;
        self.capture = WidgetId::NONE;
        self.pressed = None;

        self.tree.set_root(widget)
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// 見た目の表を差し替える。**余白や文字の大きさが変わるので測り直します。**
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;

        let root = self.tree.root();
        if root.is_some() {
            self.tree.request_layout(root);
        }
    }

    /// 表を直に書き換える。寸法に関わるものを触ったら
    /// [`Gui::request_layout`] も呼んでください。
    pub fn theme_mut(&mut self) -> &mut Theme {
        &mut self.theme
    }

    /// 木全体を測り直す。
    pub fn request_layout(&mut self) {
        let root = self.tree.root();

        if root.is_some() {
            self.tree.request_layout(root);
        }
    }

    /// 直近に [`Gui::layout`] へ渡された大きさ。
    pub fn viewport(&self) -> Size {
        self.viewport
    }

    /// 画面の倍率。**[`Gui`] は使いません。**
    ///
    /// 入力も寸法も論理ピクセルで受け取る約束なので、変換は呼ぶ側の仕事です。
    /// ここに置いてあるのは、描く側（[`crate::gui::render`]）が
    /// 物理ピクセルへ直すのに要るからです。
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        if scale_factor > 0.0 {
            self.scale_factor = scale_factor;
        }
    }

    /// 物理ピクセルを論理ピクセルに直す。入力を詰め替えるときの補助。
    pub fn to_logical(&self, physical: Point) -> Point {
        Point::new(physical.x / self.scale_factor, physical.y / self.scale_factor)
    }

    // --- 入力の覚え ---

    pub fn pointer(&self) -> Option<Point> {
        self.pointer
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn hovered(&self) -> WidgetId {
        self.hovered
    }

    pub fn focused(&self) -> WidgetId {
        self.focused
    }

    pub fn pointer_capture(&self) -> WidgetId {
        self.capture
    }

    /// 焦点を移す。
    ///
    /// 焦点を取れないもの・無効なものを渡すと**焦点は外れます**。
    pub fn set_focus(&mut self, id: WidgetId) {
        let id = if self.is_focusable(id) { id } else { WidgetId::NONE };

        if self.focused == id {
            return;
        }

        let old = self.focused;
        self.focused = id;

        if old.is_some() {
            self.edit_state(old, |state| state.focused = false);
            self.send(old, &Event::FocusLost);
        }

        if id.is_some() {
            self.edit_state(id, |state| state.focused = true);
            self.send(id, &Event::FocusGained);
        }
    }

    /// 次に焦点を取れるものへ移す。Tab の実装に。
    ///
    /// 木を前から辿った順で、いまの焦点より後ろにある最初のものへ移します。
    /// 末尾まで行ったら先頭へ戻ります。
    pub fn focus_next(&mut self) {
        self.move_focus(true);
    }

    /// 1 つ前の焦点へ。Shift+Tab の実装に。
    pub fn focus_previous(&mut self) {
        self.move_focus(false);
    }

    pub fn set_pointer_capture(&mut self, id: WidgetId) {
        self.capture = if self.tree.is_valid(id) { id } else { WidgetId::NONE };
    }

    // --- 呼ぶ側への伝達 ---

    pub(crate) fn push_action(&mut self, widget: WidgetId, kind: ActionKind) {
        let tag = self.tree.get(widget).and_then(|node| node.tag());

        self.actions.push(Action {
            widget,
            tag,
            kind,
        });
    }

    /// 溜まった伝達を取り出して空にする。
    ///
    /// **毎フレーム呼んでください。** 呼ばないと溜まり続けます。
    pub fn drain_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// 溜まっている伝達を見るだけ。
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    // --- 当たり判定 ---

    /// その点のいちばん手前の、入力を受け取るウィジェット。
    ///
    /// 手前から見ていき、最初に当たったものを返します。
    /// 入力を受け取らないもの（[`Behavior::interactive`] が false）は
    /// **素通りして後ろに当たります**。
    ///
    /// [`Behavior::interactive`]: crate::gui::widget::Behavior::interactive
    pub fn hit_test(&self, point: Point) -> WidgetId {
        // **浮かせているものが先。** 切り抜きを突き抜けていちばん手前に
        // 描かれているので、当たりもそこから取らないと噛み合わない。
        let found = self.hit_overlay(self.tree.root(), point, None);

        if found.is_some() {
            return found;
        }

        self.hit_node(self.tree.root(), point, false)
    }

    /// 浮かせているものだけを、手前から探す。
    ///
    /// `clip` は持ち主に効いている切り抜き。持ち主がそこから外れていれば、
    /// 浮かせるぶんも無いものとして扱う。
    fn hit_overlay(&self, id: WidgetId, point: Point, clip: Option<Rect>) -> WidgetId {
        let Some(node) = self.tree.get(id) else {
            return WidgetId::NONE;
        };

        let bounds = node.bounds();
        let behavior = node.behavior();

        let inner_clip = if behavior.clips_children {
            Some(match clip {
                Some(outer) => outer.intersect(bounds),
                None => bounds,
            })
        } else {
            clip
        };

        // 子を後ろから。あとに置いたものが手前。
        for child in node.children().iter().rev() {
            let found = self.hit_overlay(*child, point, inner_clip);

            if found.is_some() {
                return found;
            }
        }

        if !behavior.overlay || !behavior.interactive || node.state().disabled {
            return WidgetId::NONE;
        }

        // 持ち主が切り抜きから外れていれば、浮いてもいない。
        if clip.is_some_and(|clip| !clip.overlaps(bounds)) {
            return WidgetId::NONE;
        }

        let local = point - bounds.origin();

        match node.widget() {
            Some(widget) if widget.hit_test(local, bounds.size()) => id,
            _ => WidgetId::NONE,
        }
    }

    /// `in_overlay` が立っているあいだは、浮かせる印を見ない。
    /// 浮かせたものの中身を普通に当てるために使う。
    fn hit_node(&self, id: WidgetId, point: Point, in_overlay: bool) -> WidgetId {
        let Some(node) = self.tree.get(id) else {
            return WidgetId::NONE;
        };

        let bounds = node.bounds();
        let behavior = node.behavior();

        // 切り抜いているなら、外へ出た時点でこの枝は終わり。
        // 切り抜いていない枝は、子が親の外へ出ていても当たる。
        if behavior.clips_children && !bounds.contains(point) {
            return WidgetId::NONE;
        }

        // 子は後ろから。あとに描いたものがいちばん手前。
        for child in node.children().iter().rev() {
            let found = self.hit_node(*child, point, in_overlay);

            if found.is_some() {
                return found;
            }
        }

        if !behavior.interactive || node.state().disabled {
            return WidgetId::NONE;
        }

        // 浮かせるものは `hit_overlay` が先に見ている。ここで二度見ない。
        if behavior.overlay && !in_overlay {
            return WidgetId::NONE;
        }

        let local = point - bounds.origin();

        match node.widget() {
            Some(widget) if widget.hit_test(local, bounds.size()) => id,
            // 周回中で抜けている。形が聞けないので矩形で見る。
            None if bounds.contains(point) => id,
            _ => WidgetId::NONE,
        }
    }

    // --- 入力 ---

    /// 入力を 1 つ流す。
    pub fn handle_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers,

            InputEvent::PointerMoved { position } => self.pointer_moved(position),

            InputEvent::PointerPressed { position, button } => {
                self.pointer_pressed(position, button)
            }

            InputEvent::PointerReleased { position, button } => {
                self.pointer_released(position, button)
            }

            InputEvent::PointerLeft => self.pointer_left(),

            InputEvent::Scrolled { position, delta } => self.scrolled(position, delta),

            InputEvent::KeyPressed { key, repeat } => {
                let target = self.keyboard_target();
                self.dispatch(target, &Event::KeyPressed { key, repeat });
            }

            InputEvent::KeyReleased { key } => {
                let target = self.keyboard_target();
                self.dispatch(target, &Event::KeyReleased { key });
            }

            InputEvent::Text(text) => {
                let target = self.keyboard_target();
                self.dispatch(target, &Event::Text(text));
            }
        }
    }

    fn pointer_moved(&mut self, position: Point) {
        let delta = match self.pointer {
            Some(previous) => position - previous,
            // 窓に入った最初の 1 回は動いた量が分からない。
            None => Point::ZERO,
        };

        self.pointer = Some(position);

        let under = self.hit_test(position);

        // 掴んでいるあいだは、掴んだ相手の上にいるときだけ「乗っている」。
        // そうしないと、引いている途中で通りかかったものが光る。
        let hovered = if self.capture.is_some() {
            if under == self.capture {
                self.capture
            } else {
                WidgetId::NONE
            }
        } else {
            under
        };

        self.update_hover(hovered);

        let target = if self.capture.is_some() {
            self.capture
        } else {
            under
        };

        if target.is_some() {
            self.dispatch(target, &Event::PointerMove { position, delta });
        }
    }

    fn pointer_pressed(&mut self, position: Point, button: PointerButton) {
        self.pointer = Some(position);

        let target = self.hit_test(position);
        self.update_hover(target);

        // 焦点を取れるものを押したらそこへ。取れないものを押したら外す。
        // 「どこか空いた場所を押して入力欄を確定する」がこれで成り立つ。
        let focus = self.focus_candidate(target);
        self.set_focus(focus);

        if target.is_none() {
            return;
        }

        self.pressed = Some((button, target));
        self.edit_state(target, |state| state.pressed = true);

        if self
            .tree
            .get(target)
            .is_some_and(|node| node.behavior().captures_pointer)
        {
            self.set_pointer_capture(target);
        }

        self.dispatch(target, &Event::PointerPressed { position, button });
    }

    fn pointer_released(&mut self, position: Point, button: PointerButton) {
        self.pointer = Some(position);

        let under = self.hit_test(position);

        let Some((pressed_button, pressed_on)) = self.pressed else {
            // 押した覚えが無い。窓の外で押して中で放した、など。
            // 押下の対として扱わず、当たった先へそのまま渡す。
            if under.is_some() {
                self.dispatch(
                    under,
                    &Event::PointerReleased {
                        position,
                        button,
                        inside: true,
                    },
                );
            }

            self.set_pointer_capture(WidgetId::NONE);
            return;
        };

        if pressed_button != button {
            // 押しているのと別のボタン。対にしない。
            if under.is_some() {
                self.dispatch(
                    under,
                    &Event::PointerReleased {
                        position,
                        button,
                        inside: true,
                    },
                );
            }

            return;
        }

        self.pressed = None;
        self.edit_state(pressed_on, |state| state.pressed = false);

        // 押した相手の上で放したか。外へずらして放すのは「やめた」。
        let inside = under == pressed_on;

        self.dispatch(
            pressed_on,
            &Event::PointerReleased {
                position,
                button,
                inside,
            },
        );

        // 押して、上で放した。これを「押された」として伝える。
        // 要らないウィジェットは受け取ったあと無視すればよい。
        if inside {
            self.push_action(pressed_on, ActionKind::Clicked { button });
        }

        self.set_pointer_capture(WidgetId::NONE);
        self.update_hover(under);
    }

    fn pointer_left(&mut self) {
        self.pointer = None;
        self.update_hover(WidgetId::NONE);

        // 掴みと押下は切らない。窓の外まで引いて戻ってくることがある。
    }

    fn scrolled(&mut self, position: Point, delta: ScrollDelta) {
        self.pointer = Some(position);

        let target = if self.capture.is_some() {
            self.capture
        } else {
            self.hit_test(position)
        };

        if target.is_some() {
            self.dispatch(target, &Event::Scrolled { position, delta });
        }
    }

    /// 鍵盤の宛先。焦点が無ければ根へ送って、全体のショートカットを拾わせる。
    fn keyboard_target(&self) -> WidgetId {
        if self.focused.is_some() {
            self.focused
        } else {
            self.tree.root()
        }
    }

    fn update_hover(&mut self, next: WidgetId) {
        if self.hovered == next {
            return;
        }

        let previous = self.hovered;
        self.hovered = next;

        if previous.is_some() {
            self.edit_state(previous, |state| state.hovered = false);
            self.send(previous, &Event::PointerExit);
        }

        if next.is_some() {
            self.edit_state(next, |state| state.hovered = true);

            let position = self.pointer.unwrap_or(Point::ZERO);
            self.send(next, &Event::PointerEnter { position });
        }
    }

    /// 宛先から根へ、受け取られるまで順に渡す。
    ///
    /// `event` の座標は**画面座標**で渡してください。
    /// 各ウィジェットの手前で局所座標に直します。
    fn dispatch(&mut self, target: WidgetId, event: &Event) -> EventResult {
        let mut current = target;

        while current.is_some() {
            if self.send(current, event).is_consumed() {
                return EventResult::Consumed;
            }

            current = self.tree.parent(current);
        }

        EventResult::Ignored
    }

    /// 1 つに渡す。上へは上げない。
    fn send(&mut self, id: WidgetId, event: &Event) -> EventResult {
        let Some(node) = self.tree.get(id) else {
            return EventResult::Ignored;
        };

        // 無効なものは入力を受け取らない。焦点や掴みが残っていても届かせない。
        if node.state().disabled {
            return EventResult::Ignored;
        }

        let origin = node.origin();
        let local = rebase(event, origin);

        let Some(mut widget) = self.tree.take_widget(id) else {
            // 自分の周回の中から自分へ送ろうとしている。無限に回るので止める。
            log::debug!("event sent to {id} while it is in flight; dropped");
            return EventResult::Ignored;
        };

        let result = {
            let mut context = EventContext { gui: self, id };
            widget.on_event(&mut context, &local)
        };

        self.tree.put_widget(id, widget);

        result
    }

    // --- レイアウト ---

    /// 必要なら測り直して置き直す。
    ///
    /// `viewport` が前と同じで、どこにも印が立っていなければ**何もしません**。
    pub fn layout(&mut self, viewport: Size, text: &dyn TextMeasure) {
        self.prune();

        let root = self.tree.root();

        if root.is_none() {
            return;
        }

        if self.viewport != viewport {
            self.viewport = viewport;
            self.tree.request_layout(root);
        }

        if !self.tree.needs_layout() {
            return;
        }

        // 根は画面いっぱい。選ぶ余地を与えない。
        let constraints = Constraints::tight(viewport);
        let size = self.measure_subtree(root, constraints, text);

        self.arrange_subtree(root, Rect::from_origin_size(Point::ZERO, size), text);
    }

    pub(crate) fn measure_subtree(
        &mut self,
        id: WidgetId,
        constraints: Constraints,
        text: &dyn TextMeasure,
    ) -> Size {
        if !self.tree.is_valid(id) {
            return Size::ZERO;
        }

        if let Some(cached) = self.tree.cache(id).get(constraints) {
            return cached;
        }

        let Some(mut widget) = self.tree.take_widget(id) else {
            return Size::ZERO;
        };

        let answered = {
            let mut context = MeasureContext {
                gui: self,
                id,
                text,
            };

            widget.measure(&mut context, constraints)
        };

        self.tree.put_widget(id, widget);

        let size = constraints.constrain(finite_size(answered, constraints, id));

        let mut cache = self.tree.cache(id);
        cache.put(constraints, size);
        self.tree.set_cache(id, cache);
        self.tree.measure_done(id, size);

        size
    }

    fn arrange_subtree(&mut self, id: WidgetId, bounds: Rect, text: &dyn TextMeasure) {
        if !self.tree.is_valid(id) {
            return;
        }

        self.tree.arrange_done(id, bounds);

        let Some(mut widget) = self.tree.take_widget(id) else {
            return;
        };

        // ウィジェットには自分の左上を原点とした矩形を見せる。
        let local = Rect::from_origin_size(Point::ZERO, bounds.size());

        let mut context = ArrangeContext {
            gui: self,
            id,
            origin: bounds.origin(),
            text,
            placements: Vec::new(),
        };

        widget.arrange(&mut context, local);

        let placements = std::mem::take(&mut context.placements);
        drop(context);

        // 子へ降りる前に戻す。降りた先から親を引けるようにしておく。
        self.tree.put_widget(id, widget);

        for (child, rect) in placements {
            self.arrange_subtree(child, rect, text);
        }
    }

    // --- 描画 ---

    /// 描くものを記録して返す。
    ///
    /// 毎フレーム全部を記録し直します。記録は CPU だけで済む安い処理なので、
    /// 「何が変わったか」を見るのは [`crate::gui::render`] の仕事です。
    pub fn paint(&mut self) -> &DisplayList {
        // 自分を読みながら記録先を書くので、記録先をいったん外へ出す。
        let mut list = std::mem::take(&mut self.display_list);
        list.clear();

        let mut overlays = std::mem::take(&mut self.overlays);
        overlays.clear();

        let root = self.tree.root();

        if root.is_some() {
            self.paint_node(&mut list, root, None, &mut overlays);

            // **浮かせるぶんは最後。** 切り抜きを渡さず、記録もあとに並ぶので、
            // どの図形よりも手前に出る。
            for id in &overlays {
                self.paint_overlay(&mut list, *id);
            }
        }

        self.overlays = overlays;
        self.display_list = list;

        &self.display_list
    }

    /// 直近に記録したもの。
    pub fn display_list(&self) -> &DisplayList {
        &self.display_list
    }

    fn paint_node(
        &self,
        list: &mut DisplayList,
        id: WidgetId,
        clip: Option<ClipRegion>,
        overlays: &mut Vec<WidgetId>,
    ) {
        let Some(node) = self.tree.get(id) else {
            return;
        };

        let Some(widget) = node.widget() else {
            // 周回中。描けないので飛ばす。
            return;
        };

        let bounds = node.bounds();

        // 浮かせるものは、見えている（切り抜きと重なっている）ときだけ覚える。
        if node.behavior().overlay
            && clip.is_none_or(|clip| clip.rect.overlaps(bounds))
        {
            overlays.push(id);
        }

        // 子に渡す切り抜き。自分の見た目は**親の**切り抜きのままにする。
        // 自分で自分を切っても何も変わらない。
        let child_clip = if node.behavior().clips_children {
            let region = ClipRegion::new(bounds, widget.clip_corners().clamp_to(bounds.size()));

            Some(match clip {
                Some(outer) => outer.intersect(region),
                None => region,
            })
        } else {
            clip
        };

        let context = PaintContext { gui: self, id };

        {
            let mut painter = list.painter(id, bounds.origin(), clip);
            widget.paint(&context, &mut painter);
        }

        for child in node.children() {
            self.paint_node(list, *child, child_clip, overlays);
        }

        {
            let mut painter = list.painter(id, bounds.origin(), clip);
            widget.paint_foreground(&context, &mut painter);
        }
    }

    /// 浮かせるぶんを描く。切り抜きは渡さない。
    fn paint_overlay(&self, list: &mut DisplayList, id: WidgetId) {
        let Some(node) = self.tree.get(id) else {
            return;
        };

        let Some(widget) = node.widget() else {
            return;
        };

        let context = PaintContext { gui: self, id };
        let mut painter = list.painter(id, node.bounds().origin(), None);

        widget.paint_overlay(&context, &mut painter);
    }

    // --- 中の始末 ---

    /// 消えたものを指した鍵を捨てる。
    fn prune(&mut self) {
        if !self.tree.is_valid(self.hovered) {
            self.hovered = WidgetId::NONE;
        }

        if !self.tree.is_valid(self.focused) {
            self.focused = WidgetId::NONE;
        }

        if !self.tree.is_valid(self.capture) {
            self.capture = WidgetId::NONE;
        }

        if self
            .pressed
            .is_some_and(|(_, id)| !self.tree.is_valid(id))
        {
            self.pressed = None;
        }
    }

    fn edit_state(&mut self, id: WidgetId, edit: impl FnOnce(&mut WidgetState)) {
        let mut state = self.tree.state(id);
        edit(&mut state);
        self.tree.set_state(id, state);
    }

    fn is_focusable(&self, id: WidgetId) -> bool {
        self.tree
            .get(id)
            .is_some_and(|node| node.behavior().focusable && !node.state().disabled)
    }

    /// 自分から根へ辿って、最初に焦点を取れるもの。
    ///
    /// 押したのが飾りでも、それを包む押せるものへ焦点が行きます。
    fn focus_candidate(&self, id: WidgetId) -> WidgetId {
        if self.is_focusable(id) {
            return id;
        }

        self.tree
            .ancestors(id)
            .find(|ancestor| self.is_focusable(*ancestor))
            .unwrap_or(WidgetId::NONE)
    }

    fn move_focus(&mut self, forward: bool) {
        let mut order = Vec::new();
        self.collect_focusable(self.tree.root(), &mut order);

        if order.is_empty() {
            self.set_focus(WidgetId::NONE);
            return;
        }

        let next = match order.iter().position(|id| *id == self.focused) {
            Some(index) if forward => (index + 1) % order.len(),
            Some(index) => (index + order.len() - 1) % order.len(),
            // まだどこにも焦点が無い。前なら先頭、後ろなら末尾から。
            None if forward => 0,
            None => order.len() - 1,
        };

        self.set_focus(order[next]);
    }

    /// 前から辿った順に、焦点を取れるものを集める。
    fn collect_focusable(&self, id: WidgetId, into: &mut Vec<WidgetId>) {
        let Some(node) = self.tree.get(id) else {
            return;
        };

        if node.behavior().focusable && !node.state().disabled {
            into.push(id);
        }

        for child in node.children() {
            self.collect_focusable(*child, into);
        }
    }
}

/// 画面座標の出来事を、そのウィジェットの局所座標に直す。
fn rebase(event: &Event, origin: Point) -> Event {
    match event {
        Event::PointerEnter { position } => Event::PointerEnter {
            position: *position - origin,
        },

        Event::PointerMove { position, delta } => Event::PointerMove {
            position: *position - origin,
            // 動いた量はずれを引いても変わらない。
            delta: *delta,
        },

        Event::PointerPressed { position, button } => Event::PointerPressed {
            position: *position - origin,
            button: *button,
        },

        Event::PointerReleased {
            position,
            button,
            inside,
        } => Event::PointerReleased {
            position: *position - origin,
            button: *button,
            inside: *inside,
        },

        Event::Scrolled { position, delta } => Event::Scrolled {
            position: *position - origin,
            delta: *delta,
        },

        // 座標を持たないものはそのまま。
        Event::PointerExit
        | Event::KeyPressed { .. }
        | Event::KeyReleased { .. }
        | Event::Text(_)
        | Event::FocusGained
        | Event::FocusLost => event.clone(),
    }
}

/// 無限を潰す。無限のまま矩形にすると何も描けなくなる。
fn finite_size(size: Size, constraints: Constraints, id: WidgetId) -> Size {
    if size.width.is_finite() && size.height.is_finite() {
        return size;
    }

    log::warn!(
        "widget {id} asked for an infinite size ({:?}); clamped. \
         measure must return what the content needs, not the constraint",
        size
    );

    let bounded = constraints.biggest();

    Size::new(
        if size.width.is_finite() {
            size.width
        } else {
            bounded.width
        },
        if size.height.is_finite() {
            size.height
        } else {
            bounded.height
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::color::Color;
    use crate::gui::context::NoTextMeasure;
    use crate::gui::event::{Key, NamedKey};
    use crate::gui::geometry::Corners;
    use crate::gui::painter::Painter;
    use crate::gui::widget::Behavior;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// 何が届いたかを覚えるだけのウィジェット。
    struct Probe {
        behavior: Behavior,
        /// 測ったときに返す大きさ。`None` なら制約いっぱい。
        wanted: Option<Size>,
        log: Rc<RefCell<Vec<String>>>,
        /// 受け取ったことにするか。
        consume: bool,
        /// 子を縦に積むか（既定は重ねる）。
        stack: bool,
        paints: bool,
    }

    impl Probe {
        fn new(log: &Rc<RefCell<Vec<String>>>) -> Self {
            Self {
                behavior: Behavior::DECORATION,
                wanted: None,
                log: Rc::clone(log),
                consume: false,
                stack: false,
                paints: false,
            }
        }

        fn behavior(mut self, behavior: Behavior) -> Self {
            self.behavior = behavior;
            self
        }

        fn wanted(mut self, size: Size) -> Self {
            self.wanted = Some(size);
            self
        }

        fn stacking(mut self) -> Self {
            self.stack = true;
            self
        }

        fn painting(mut self) -> Self {
            self.paints = true;
            self
        }

        fn boxed(self) -> Box<dyn Widget> {
            Box::new(self)
        }

        fn note(&self, what: &str) {
            self.log.borrow_mut().push(what.to_string());
        }
    }

    impl Widget for Probe {
        fn measure(&mut self, context: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
            self.note("measure");

            if self.stack {
                // 縦に積む。幅は制約いっぱい、高さは子の合計。
                let inner = constraints.unbound(crate::gui::geometry::Axis::Vertical);
                let mut height = 0.0;

                for child in context.children() {
                    height += context.measure_child(child, inner).height;
                }

                return Size::new(constraints.biggest().width, height);
            }

            match self.wanted {
                Some(wanted) => wanted,
                None => constraints.biggest(),
            }
        }

        fn arrange(&mut self, context: &mut ArrangeContext<'_>, bounds: Rect) {
            if !self.stack {
                for child in context.children() {
                    context.place(child, bounds);
                }
                return;
            }

            let mut y = 0.0;

            for child in context.children() {
                let height = context.desired(child).height;
                context.place(child, Rect::new(0.0, y, bounds.width, height));
                y += height;
            }
        }

        fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
            if self.paints {
                painter.rect(context.rect(), Color::WHITE);
            }
        }

        fn clip_corners(&self) -> Corners {
            Corners::all(4.0)
        }

        fn on_event(&mut self, context: &mut EventContext<'_>, event: &Event) -> EventResult {
            self.note(match event {
                Event::PointerEnter { .. } => "enter",
                Event::PointerMove { .. } => "move",
                Event::PointerExit => "exit",
                Event::PointerPressed { .. } => "press",
                Event::PointerReleased { inside: true, .. } => "release-inside",
                Event::PointerReleased { inside: false, .. } => "release-outside",
                Event::Scrolled { .. } => "scroll",
                Event::KeyPressed { .. } => "key",
                Event::KeyReleased { .. } => "key-up",
                Event::Text(_) => "text",
                Event::FocusGained => "focus",
                Event::FocusLost => "blur",
            });

            let _ = context;
            EventResult::consumed_if(self.consume)
        }

        fn behavior(&self) -> Behavior {
            self.behavior
        }
    }

    fn log() -> Rc<RefCell<Vec<String>>> {
        Rc::new(RefCell::new(Vec::new()))
    }

    fn laid_out(gui: &mut Gui, viewport: Size) {
        gui.layout(viewport, &NoTextMeasure);
    }

    #[test]
    fn root_fills_the_viewport() {
        let log = log();
        let mut gui = Gui::new();
        let root = gui.set_root(Probe::new(&log).boxed());

        laid_out(&mut gui, Size::new(800.0, 600.0));

        assert_eq!(gui.tree().bounds(root), Rect::new(0.0, 0.0, 800.0, 600.0));
    }

    #[test]
    fn children_are_placed_in_global_coordinates() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        let first = gui
            .tree_mut()
            .add_child(root, Probe::new(&log).wanted(Size::new(100.0, 30.0)).boxed());
        let second = gui
            .tree_mut()
            .add_child(root, Probe::new(&log).wanted(Size::new(100.0, 50.0)).boxed());

        laid_out(&mut gui, Size::new(200.0, 300.0));

        // 子は親の左上を基準に置いたが、木には画面座標で入る。
        assert_eq!(gui.tree().bounds(first), Rect::new(0.0, 0.0, 200.0, 30.0));
        assert_eq!(gui.tree().bounds(second), Rect::new(0.0, 30.0, 200.0, 50.0));
    }

    #[test]
    fn layout_is_skipped_when_nothing_changed() {
        let log = log();
        let mut gui = Gui::new();
        gui.set_root(Probe::new(&log).boxed());

        laid_out(&mut gui, Size::new(100.0, 100.0));
        let after_first = log.borrow().iter().filter(|e| *e == "measure").count();
        assert!(after_first >= 1);

        laid_out(&mut gui, Size::new(100.0, 100.0));
        let after_second = log.borrow().iter().filter(|e| *e == "measure").count();

        assert_eq!(after_first, after_second, "印が無ければ測らない");

        // 大きさが変われば測り直す。
        laid_out(&mut gui, Size::new(120.0, 100.0));
        assert!(log.borrow().iter().filter(|e| *e == "measure").count() > after_second);
    }

    #[test]
    fn hit_test_skips_non_interactive_widgets() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).behavior(Behavior::SURFACE).boxed());
        // 飾りは素通りして後ろの root に当たる。
        let decoration = gui.tree_mut().add_child(root, Probe::new(&log).boxed());

        laid_out(&mut gui, Size::new(100.0, 100.0));

        assert_eq!(gui.hit_test(Point::new(50.0, 50.0)), root);
        assert_ne!(gui.hit_test(Point::new(50.0, 50.0)), decoration);
    }

    #[test]
    fn hit_test_prefers_the_last_child() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).behavior(Behavior::SURFACE).boxed());
        let under = gui
            .tree_mut()
            .add_child(root, Probe::new(&log).behavior(Behavior::SURFACE).boxed());
        let over = gui
            .tree_mut()
            .add_child(root, Probe::new(&log).behavior(Behavior::SURFACE).boxed());

        laid_out(&mut gui, Size::new(100.0, 100.0));

        // 重なっているので、あとに描いたほうが手前。
        assert_eq!(gui.hit_test(Point::new(10.0, 10.0)), over);
        assert_ne!(gui.hit_test(Point::new(10.0, 10.0)), under);
    }

    #[test]
    fn clipping_parents_cut_the_hit_test_short() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        let viewport = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(100.0, 20.0))
                .behavior(Behavior::SURFACE.clips_children(true))
                .boxed(),
        );
        // 親より縦に長い子。切り抜かれる。
        let tall = gui.tree_mut().add_child(
            viewport,
            Probe::new(&log)
                .wanted(Size::new(100.0, 500.0))
                .behavior(Behavior::SURFACE)
                .boxed(),
        );

        laid_out(&mut gui, Size::new(100.0, 400.0));

        assert_eq!(gui.hit_test(Point::new(10.0, 10.0)), tall, "親の中なら当たる");
        assert_eq!(
            gui.hit_test(Point::new(10.0, 100.0)),
            WidgetId::NONE,
            "親の外は当たらない"
        );
    }

    #[test]
    fn hover_enters_and_exits_exactly_once() {
        let log = log();
        let mut gui = Gui::new();
        gui.set_root(Probe::new(&log).behavior(Behavior::SURFACE).boxed());
        laid_out(&mut gui, Size::new(100.0, 100.0));
        log.borrow_mut().clear();

        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(10.0, 10.0),
        });
        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(20.0, 20.0),
        });
        gui.handle_input(InputEvent::PointerLeft);

        let events = log.borrow().clone();

        assert_eq!(
            events,
            vec!["enter", "move", "move", "exit"],
            "乗ったのは 1 回だけ"
        );
    }

    #[test]
    fn a_click_is_press_then_release_inside() {
        let log = log();
        let mut gui = Gui::new();
        let root = gui.set_root(Probe::new(&log).behavior(Behavior::CONTROL).boxed());
        laid_out(&mut gui, Size::new(100.0, 100.0));

        let at = Point::new(10.0, 10.0);
        gui.handle_input(InputEvent::PointerMoved { position: at });
        gui.handle_input(InputEvent::PointerPressed {
            position: at,
            button: PointerButton::Primary,
        });

        assert!(gui.tree().state(root).pressed);
        assert!(gui.tree().state(root).armed());

        gui.handle_input(InputEvent::PointerReleased {
            position: at,
            button: PointerButton::Primary,
        });

        let actions = gui.drain_actions();

        assert_eq!(actions.len(), 1, "{actions:?}");
        assert_eq!(actions[0].widget, root);
        assert!(matches!(actions[0].kind, ActionKind::Clicked { .. }));
        assert!(!gui.tree().state(root).pressed);
    }

    #[test]
    fn dragging_out_cancels_the_click() {
        let log = log();
        let mut gui = Gui::new();
        let root = gui.set_root(Probe::new(&log).behavior(Behavior::CONTROL).boxed());
        laid_out(&mut gui, Size::new(100.0, 100.0));

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
        });

        // 外へずらす。掴んでいるので入力は届き続けるが、乗っていない扱い。
        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(500.0, 500.0),
        });

        let state = gui.tree().state(root);
        assert!(state.pressed, "押下は続く");
        assert!(!state.hovered, "乗ってはいない");
        assert!(!state.armed(), "放しても決まらない");

        gui.handle_input(InputEvent::PointerReleased {
            position: Point::new(500.0, 500.0),
            button: PointerButton::Primary,
        });

        assert!(gui.drain_actions().is_empty(), "外で放したら押されていない");
        assert!(log.borrow().contains(&"release-outside".to_string()));
    }

    #[test]
    fn capture_keeps_events_on_the_pressed_widget() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        let grabbed = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(100.0, 20.0))
                .behavior(Behavior::CONTROL)
                .boxed(),
        );
        let other = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(100.0, 20.0))
                .behavior(Behavior::CONTROL)
                .boxed(),
        );

        laid_out(&mut gui, Size::new(100.0, 200.0));

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(10.0, 5.0),
            button: PointerButton::Primary,
        });
        assert_eq!(gui.pointer_capture(), grabbed);

        // 隣の上まで引く。隣は光らない。
        gui.handle_input(InputEvent::PointerMoved {
            position: Point::new(10.0, 25.0),
        });

        assert!(!gui.tree().state(other).hovered, "引いている途中の相手は光らない");
        assert_eq!(gui.hovered(), WidgetId::NONE);

        gui.handle_input(InputEvent::PointerReleased {
            position: Point::new(10.0, 25.0),
            button: PointerButton::Primary,
        });

        assert_eq!(gui.pointer_capture(), WidgetId::NONE, "放したら掴みも切れる");
    }

    #[test]
    fn events_bubble_until_consumed() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).behavior(Behavior::SURFACE).boxed());
        let child = gui
            .tree_mut()
            .add_child(root, Probe::new(&log).behavior(Behavior::SURFACE).boxed());

        laid_out(&mut gui, Size::new(100.0, 100.0));
        log.borrow_mut().clear();

        gui.handle_input(InputEvent::Scrolled {
            position: Point::new(10.0, 10.0),
            delta: ScrollDelta::Lines { x: 0.0, y: -1.0 },
        });

        // 子が取らないので親にも届く。
        assert_eq!(
            log.borrow().iter().filter(|e| *e == "scroll").count(),
            2,
            "{:?}",
            log.borrow()
        );

        // 子が取ると止まる。
        log.borrow_mut().clear();
        gui.tree_mut().get_as_mut::<Probe>(child).unwrap().consume = true;

        gui.handle_input(InputEvent::Scrolled {
            position: Point::new(10.0, 10.0),
            delta: ScrollDelta::Lines { x: 0.0, y: -1.0 },
        });

        assert_eq!(log.borrow().iter().filter(|e| *e == "scroll").count(), 1);
    }

    #[test]
    fn focus_follows_the_press_and_keys_go_there() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        let control = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(100.0, 20.0))
                .behavior(Behavior::CONTROL)
                .boxed(),
        );

        laid_out(&mut gui, Size::new(100.0, 200.0));
        log.borrow_mut().clear();

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(10.0, 5.0),
            button: PointerButton::Primary,
        });

        assert_eq!(gui.focused(), control);
        assert!(gui.tree().state(control).focused);
        assert!(log.borrow().contains(&"focus".to_string()));

        log.borrow_mut().clear();
        gui.handle_input(InputEvent::KeyPressed {
            key: Key::Named(NamedKey::Enter),
            repeat: false,
        });

        assert!(log.borrow().contains(&"key".to_string()));
    }

    #[test]
    fn pressing_a_non_focusable_place_drops_focus() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        let control = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(100.0, 20.0))
                .behavior(Behavior::CONTROL)
                .boxed(),
        );
        // 焦点を取れない背景。
        let backdrop = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(100.0, 100.0))
                .behavior(Behavior::SURFACE)
                .boxed(),
        );

        laid_out(&mut gui, Size::new(100.0, 200.0));

        gui.set_focus(control);
        assert_eq!(gui.focused(), control);

        // 背景を押す。包む親も焦点を取れないので外れる。
        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(10.0, 50.0),
            button: PointerButton::Primary,
        });

        assert_eq!(gui.hit_test(Point::new(10.0, 50.0)), backdrop);
        assert_eq!(gui.focused(), WidgetId::NONE);
        assert!(log.borrow().contains(&"blur".to_string()));
    }

    #[test]
    fn focus_cannot_land_on_a_non_focusable_widget() {
        let log = log();
        let mut gui = Gui::new();
        let root = gui.set_root(Probe::new(&log).behavior(Behavior::SURFACE).boxed());

        laid_out(&mut gui, Size::new(100.0, 100.0));
        gui.set_focus(root);

        assert_eq!(gui.focused(), WidgetId::NONE);
    }

    #[test]
    fn tab_order_follows_the_tree() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        let controls: Vec<_> = (0..3)
            .map(|_| {
                gui.tree_mut().add_child(
                    root,
                    Probe::new(&log)
                        .wanted(Size::new(100.0, 20.0))
                        .behavior(Behavior::CONTROL)
                        .boxed(),
                )
            })
            .collect();

        laid_out(&mut gui, Size::new(100.0, 200.0));

        gui.focus_next();
        assert_eq!(gui.focused(), controls[0]);

        gui.focus_next();
        gui.focus_next();
        assert_eq!(gui.focused(), controls[2]);

        gui.focus_next();
        assert_eq!(gui.focused(), controls[0], "末尾から先頭へ戻る");

        gui.focus_previous();
        assert_eq!(gui.focused(), controls[2]);
    }

    #[test]
    fn disabled_widgets_take_nothing() {
        let log = log();
        let mut gui = Gui::new();
        let root = gui.set_root(Probe::new(&log).behavior(Behavior::CONTROL).boxed());

        laid_out(&mut gui, Size::new(100.0, 100.0));
        gui.tree_mut().set_disabled(root, true);
        log.borrow_mut().clear();

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(10.0, 10.0),
            button: PointerButton::Primary,
        });

        assert_eq!(gui.hit_test(Point::new(10.0, 10.0)), WidgetId::NONE);
        assert!(log.borrow().is_empty(), "{:?}", log.borrow());
        assert!(gui.drain_actions().is_empty());
    }

    #[test]
    fn removed_widgets_lose_focus_and_capture() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        let control = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(100.0, 20.0))
                .behavior(Behavior::CONTROL)
                .boxed(),
        );

        laid_out(&mut gui, Size::new(100.0, 200.0));

        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(10.0, 5.0),
            button: PointerButton::Primary,
        });

        assert_eq!(gui.focused(), control);
        assert_eq!(gui.pointer_capture(), control);

        gui.tree_mut().remove(control);
        laid_out(&mut gui, Size::new(100.0, 200.0));

        assert_eq!(gui.focused(), WidgetId::NONE);
        assert_eq!(gui.pointer_capture(), WidgetId::NONE);
        assert_eq!(gui.hovered(), WidgetId::NONE);
    }

    #[test]
    fn paint_records_parents_before_children() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().painting().boxed());
        let child = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(50.0, 20.0))
                .painting()
                .boxed(),
        );

        laid_out(&mut gui, Size::new(100.0, 100.0));

        let list = gui.paint();

        assert_eq!(list.len(), 2);
        assert_eq!(list.commands()[0].owner, root, "親が先（奥）");
        assert_eq!(list.commands()[1].owner, child, "子が後（手前）");
    }

    #[test]
    fn clipping_parents_bake_a_clip_into_their_children() {
        let log = log();
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        let viewport = gui.tree_mut().add_child(
            root,
            Probe::new(&log)
                .wanted(Size::new(100.0, 20.0))
                .behavior(Behavior::SURFACE.clips_children(true))
                .painting()
                .boxed(),
        );
        gui.tree_mut().add_child(
            viewport,
            Probe::new(&log)
                .wanted(Size::new(100.0, 500.0))
                .painting()
                .boxed(),
        );

        laid_out(&mut gui, Size::new(100.0, 400.0));

        let list = gui.paint();
        let commands = list.commands();

        // 切り抜く本人は親の切り抜きのまま。
        assert!(commands[0].clip.is_none());

        let clip = commands[1].clip.expect("子は切り抜かれる");
        assert_eq!(clip.rect, Rect::new(0.0, 0.0, 100.0, 20.0));
        assert_eq!(clip.corners.max(), 4.0, "clip_corners が効く");
    }

    #[test]
    fn an_infinite_measure_is_clamped() {
        struct Greedy;

        impl Widget for Greedy {
            fn measure(&mut self, _: &mut MeasureContext<'_>, _: Constraints) -> Size {
                Size::new(f32::INFINITY, f32::INFINITY)
            }
        }

        let mut gui = Gui::new();
        let root = gui.set_root(Box::new(Greedy));

        laid_out(&mut gui, Size::new(100.0, 50.0));

        // 無限のまま矩形にすると何も描けない。制約で止める。
        assert_eq!(gui.tree().bounds(root), Rect::new(0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn actions_carry_the_tag() {
        let log = log();
        let mut gui = Gui::new();
        let root = gui.set_root(Probe::new(&log).behavior(Behavior::CONTROL).boxed());
        let tag = crate::gui::id::Tag::new("ok-button");

        gui.tree_mut().set_tag(root, tag);
        laid_out(&mut gui, Size::new(100.0, 100.0));

        let at = Point::new(10.0, 10.0);
        gui.handle_input(InputEvent::PointerPressed {
            position: at,
            button: PointerButton::Primary,
        });
        gui.handle_input(InputEvent::PointerReleased {
            position: at,
            button: PointerButton::Primary,
        });

        let actions = gui.drain_actions();

        assert!(actions[0].is(tag));
        assert!(gui.drain_actions().is_empty(), "取り出したら空になる");
    }

    #[test]
    fn local_coordinates_reach_the_widget() {
        struct Spy(Rc<RefCell<Option<Point>>>);

        impl Widget for Spy {
            fn measure(&mut self, _: &mut MeasureContext<'_>, _: Constraints) -> Size {
                Size::new(100.0, 40.0)
            }

            fn behavior(&self) -> Behavior {
                Behavior::SURFACE
            }

            fn on_event(&mut self, _: &mut EventContext<'_>, event: &Event) -> EventResult {
                if let Event::PointerPressed { position, .. } = event {
                    *self.0.borrow_mut() = Some(*position);
                }

                EventResult::Consumed
            }
        }

        let log = log();
        let seen = Rc::new(RefCell::new(None));
        let mut gui = Gui::new();

        let root = gui.set_root(Probe::new(&log).stacking().boxed());
        gui.tree_mut().add_child(
            root,
            Probe::new(&log).wanted(Size::new(100.0, 40.0)).boxed(),
        );
        gui.tree_mut()
            .add_child(root, Box::new(Spy(Rc::clone(&seen))));

        laid_out(&mut gui, Size::new(100.0, 200.0));

        // 2 つめの子は y = 40 から始まる。
        gui.handle_input(InputEvent::PointerPressed {
            position: Point::new(30.0, 55.0),
            button: PointerButton::Primary,
        });

        assert_eq!(
            *seen.borrow(),
            Some(Point::new(30.0, 15.0)),
            "自分の左上からの座標で届く"
        );
    }
}
