//! 窓（[`WindowFrame`]）を何枚も重ねて持つ。
//!
//! 窓 1 枚なら [`WindowFrame`] をそのまま使えます。何枚も置くと、次のことを誰かが
//! 決めなければなりません。[`GuiManager`] がそれを引き受けます。
//!
//! - **重なり順**: 押した窓を手前に出す。手前の窓ほど図形の z を底上げする
//!   （[`WindowFrame::set_depth`]）
//! - **指**: 押す・右クリック・ホイールは手前の窓から順に渡し、取った窓で止める。
//!   放すはすべての窓に渡す（押したまま外へ出た部品を戻せるように）
//! - **フォーカス**: キーと文字を受け取るのは 1 枚だけ。別の窓が押されたら、ほかの窓は手放す
//! - **指の形**: 指の下の、一番手前の窓に聞く
//!
//! 窓 1 枚と同じ形で呼べるように、[`Gui`] を [`WindowFrame`] と [`GuiManager`] の両方に
//! 実装してあります。winit とつなぐ側（`gueiz::winit_gui` など）は、どちらでも受け取れます。
//!
//! # 使い方
//!
//! ```ignore
//! let mut gui = GuiManager::new();
//! let settings = gui.add_window(&mut draw_manager, &resources, surface_size, design, settings_frame);
//! let palette = gui.add_window(&mut draw_manager, &resources, surface_size, design, palette_frame);
//!
//! // 出来事は窓 1 枚のときと同じように渡す。
//! gui.mouse_left_pressed(&mut draw_manager, x, y);
//!
//! // 描く前に 1 フレーム 1 回。
//! gui.update(&mut draw_manager, &resources);
//! ```

use std::time::Instant;

use gueiz_gpu::renderer::SurfaceSize;

use crate::draw_manager::DrawManager;
use crate::gui::clipboard::Clipboard;
use crate::gui::window_frame::WindowFrame;
use crate::gui::window_item::{CursorShape, EditKey, KeyModifiers, Pointer};
use crate::resource::Resources;

/// 窓 1 枚ぶんの重なり順の幅。
///
/// 窓の中では、部品が z を 0、ポップアップ（ドロップダウンの一覧など）が 500、
/// 右クリックのメニューが 1000 から使います。それより十分広く取り、隣の窓と混ざらないようにします。
/// 2D の投影は z を捨てるので（描く順にしか使わない）、大きくしても切り落とされません。
pub const WINDOW_DEPTH_STEP: f32 = 10_000.0;

/// [`GuiManager`] に入れた窓を指す番号。入れた順に振られ、手前に出しても変わりません。
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(u64);

/// 窓を何枚も重ねて持ち、出来事を振り分ける。
#[derive(Default)]
pub struct GuiManager {
    /// **奥から手前の順。** 最後が一番手前。
    windows: Vec<(WindowId, WindowFrame)>,
    next_id: u64,
}

impl GuiManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 窓を組んで（[`WindowFrame::create_object`]）、一番手前に入れる。
    ///
    /// 部品とメニューは、窓を渡す前に足しておいてください。
    pub fn add_window(
        &mut self,
        draw_manager: &mut DrawManager,
        resources: &Resources,
        surface_size: SurfaceSize,
        design: [f32; 2],
        mut window_frame: WindowFrame,
    ) -> WindowId {
        // 組む前に底上げを決めておけば、組んだ図形に 1 回で掛かる（後からなめ直さずに済む）。
        window_frame.set_depth(draw_manager, Self::depth_of(self.windows.len()));
        window_frame.create_object(draw_manager, resources, surface_size, design);

        let id = WindowId(self.next_id);
        self.next_id += 1;
        self.windows.push((id, window_frame));

        id
    }

    /// その番号の窓。
    pub fn window(&self, id: WindowId) -> Option<&WindowFrame> {
        self.windows.iter().find(|(window_id, _)| *window_id == id).map(|(_, window)| window)
    }

    /// その番号の窓を書き換える。
    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut WindowFrame> {
        self.windows.iter_mut().find(|(window_id, _)| *window_id == id).map(|(_, window)| window)
    }

    /// 窓の番号を、奥から手前の順に。
    pub fn ids(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.windows.iter().map(|(id, _)| *id)
    }

    /// 持っている窓の数。
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// 一番手前の窓。
    pub fn front(&self) -> Option<WindowId> {
        self.windows.last().map(|(id, _)| *id)
    }

    /// キーと文字を受け取っている窓（部品がフォーカスを持っている窓）。
    pub fn focused(&self) -> Option<WindowId> {
        self.windows.iter().rev().find(|(_, window)| window.has_focus()).map(|(id, _)| *id)
    }

    /// その窓を一番手前に出す。出したら `true`（もう一番手前なら `false`）。
    ///
    /// 当たり判定（[`DrawManager::pick`]）の手前・奥は描く順で決まるので、
    /// 新しい重なりが当たりに効くのは、次に [`DrawManager::prepare`] で並べ直したあとです。
    pub fn bring_to_front(&mut self, draw_manager: &mut DrawManager, id: WindowId) -> bool {
        match self.windows.iter().position(|(window_id, _)| *window_id == id) {
            Some(index) => self.raise(draw_manager, index),
            None => false,
        }
    }

    /// `index` 番目の窓を一番手前へ。並びが変わった窓だけ底上げを掛け直す。
    fn raise(&mut self, draw_manager: &mut DrawManager, index: usize) -> bool {
        if index + 1 >= self.windows.len() {
            return false;
        }

        let window = self.windows.remove(index);
        self.windows.push(window);

        for (position, (_, window)) in self.windows.iter_mut().enumerate().skip(index) {
            window.set_depth(draw_manager, Self::depth_of(position));
        }

        true
    }

    fn depth_of(position: usize) -> f32 {
        position as f32 * WINDOW_DEPTH_STEP
    }

    /// 指を独り占めしている窓（掴んで動かしている・縁を引いている・メニューを開いている）。
    fn captured(&mut self) -> Option<&mut WindowFrame> {
        self.windows
            .iter_mut()
            .rev()
            .map(|(_, window)| window)
            .find(|window| window.is_dragging() || window.is_resizing() || window.is_context_menu_open())
    }

    /// キーを渡す窓。メニューが開いている窓（Esc で閉じる）か、フォーカスを持っている窓。
    fn key_target(&mut self) -> Option<&mut WindowFrame> {
        self.windows
            .iter_mut()
            .rev()
            .map(|(_, window)| window)
            .find(|window| window.is_context_menu_open() || window.has_focus())
    }

    fn focused_window(&self) -> Option<&WindowFrame> {
        self.windows.iter().rev().map(|(_, window)| window).find(|window| window.has_focus())
    }

    fn focused_window_mut(&mut self) -> Option<&mut WindowFrame> {
        self.windows.iter_mut().rev().map(|(_, window)| window).find(|window| window.has_focus())
    }

    // --- フレームごと ---

    /// すべての窓の [`WindowFrame::update`]。**描く前に 1 フレーム 1 回呼んでください。** 何か移したら `true`。
    pub fn update(&mut self, draw_manager: &mut DrawManager, resources: &Resources) -> bool {
        let mut changed = false;
        for (_, window) in &mut self.windows {
            changed |= window.update(draw_manager, resources);
        }
        changed
    }

    /// どれかの窓が次に動かしてほしい時刻のうち、一番早いもの。
    pub fn next_update(&self) -> Option<Instant> {
        self.windows.iter().filter_map(|(_, window)| window.next_update()).min()
    }

    /// 描き先の大きさが変わった。すべての窓のカメラを貼り直す。
    pub fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        for (_, window) in &mut self.windows {
            window.resize(draw_manager, surface_size, design);
        }
    }

    // --- 指 ---

    /// 左を押した。手前の窓から順に渡し、取った窓を一番手前に出す。**どれかの窓が取ったら `true`。**
    ///
    /// 取った窓より奥の窓には届かないので、ここでフォーカスを手放させます
    /// （手前の窓は、外を押されたときに自分で手放します）。
    pub fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let taken = (0..self.windows.len())
            .rev()
            .find(|&index| self.windows[index].1.mouse_left_pressed(draw_manager, x, y));

        let Some(index) = taken else {
            return false;
        };

        for (_, window) in &mut self.windows[..index] {
            window.unfocus(draw_manager);
        }
        self.raise(draw_manager, index);

        true
    }

    /// 左を放した。**どこで放しても、すべての窓に渡します。** 何か変わったら `true`。
    pub fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let mut changed = false;
        for (_, window) in &mut self.windows {
            changed |= window.mouse_left_released(draw_manager, x, y);
        }
        changed
    }

    /// 右を押した。手前の窓から順に渡し、取った窓を一番手前に出す。**どれかの窓が取ったら `true`。**
    pub fn mouse_right_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let taken = (0..self.windows.len())
            .rev()
            .find(|&index| self.windows[index].1.mouse_right_pressed(draw_manager, x, y));

        let Some(index) = taken else {
            return false;
        };

        // メニューを開いた窓より奥の窓には届いていない。開いているメニューがあれば閉じる。
        for (_, window) in &mut self.windows[..index] {
            if window.is_context_menu_open() {
                window.unfocus(draw_manager);
            }
        }
        self.raise(draw_manager, index);

        true
    }

    pub fn mouse_right_released(&mut self) -> bool {
        false
    }

    /// 指が動いた。見た目が変わったら `true`。
    ///
    /// 掴んで動かしている・縁を引いている・メニューを開いている窓があれば、その窓だけに渡します。
    /// なければすべての窓に渡します（指が離れた部品の光を消すため）。当たり判定は全部の窓で 1 回だけです。
    pub fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let pointer = Pointer::new(x, y);

        if let Some(window) = self.captured() {
            return window.mouse_moved_at(draw_manager, &pointer);
        }

        let mut changed = false;
        for (_, window) in &mut self.windows {
            changed |= window.mouse_moved_at(draw_manager, &pointer);
        }
        changed
    }

    /// ホイールが回った。手前の窓から順に渡し、取った窓で止める。**どれかの窓が取ったら `true`。**
    pub fn mouse_wheel(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32, lines: [f32; 2]) -> bool {
        self.windows
            .iter_mut()
            .rev()
            .any(|(_, window)| window.mouse_wheel(draw_manager, x, y, lines))
    }

    /// その画素の位置で出したい指の形。指の下の、一番手前の窓に聞く。
    pub fn cursor_shape(&self, draw_manager: &DrawManager, x: f32, y: f32) -> CursorShape {
        let pointer = Pointer::new(x, y);

        // 掴んでいる窓は、指が窓の外へ出ても形を保つ。
        let captured = self.windows.iter().rev().map(|(_, window)| window).find(|window| window.is_dragging() || window.is_resizing());
        if let Some(window) = captured {
            return window.cursor_shape_at(draw_manager, &pointer);
        }

        self.windows
            .iter()
            .rev()
            .map(|(_, window)| window)
            .find(|window| window.contains_at(draw_manager, &pointer))
            .map_or(CursorShape::Default, |window| window.cursor_shape_at(draw_manager, &pointer))
    }

    /// 指がどれかの窓の上か。`false` なら、指の入力は後ろ（ゲームの操作など）のもの。
    pub fn contains(&self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        let pointer = Pointer::new(x, y);
        self.windows.iter().any(|(_, window)| window.contains_at(draw_manager, &pointer))
    }

    // --- キーボード ---

    /// キーと文字を受け取りたい部品が、どれかの窓にあるか。
    pub fn has_focus(&self) -> bool {
        self.focused_window().is_some()
    }

    /// IME を使いたい部品が、どれかの窓にあるか。
    pub fn wants_ime(&self) -> bool {
        self.focused_window().is_some_and(WindowFrame::wants_ime)
    }

    /// 文字が打たれた。フォーカスを持っている窓に渡す。
    pub fn text_input(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        self.focused_window_mut().is_some_and(|window| window.text_input(draw_manager, resources, text))
    }

    /// IME で変換中の文字が変わった。フォーカスを持っている窓に渡す。
    pub fn ime_preedit(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str, cursor: Option<usize>) -> bool {
        self.focused_window_mut().is_some_and(|window| window.ime_preedit(draw_manager, resources, text, cursor))
    }

    /// 文字を書き換えるキーが押された。メニューが開いている窓か、フォーカスを持っている窓に渡す。
    pub fn key_pressed(&mut self, draw_manager: &mut DrawManager, resources: &Resources, key: EditKey) -> bool {
        self.key_target().is_some_and(|window| window.key_pressed(draw_manager, resources, key))
    }

    /// Shift や Ctrl の押し具合が変わった。すべての窓に知らせる。
    pub fn modifiers_changed(&mut self, modifiers: KeyModifiers) {
        for (_, window) in &mut self.windows {
            window.modifiers_changed(modifiers);
        }
    }

    /// フォーカスを持っている部品で選んでいる文字。
    pub fn selected_text(&self) -> Option<String> {
        self.focused_window()?.selected_text()
    }

    /// 選んでいる文字をクリップボードへ入れる。
    pub fn copy_to_clipboard(&self, clipboard: &mut dyn Clipboard) -> bool {
        self.focused_window().is_some_and(|window| window.copy_to_clipboard(clipboard))
    }

    /// 選んでいる文字をクリップボードへ入れて消す。
    pub fn cut_to_clipboard(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
        self.focused_window_mut().is_some_and(|window| window.cut_to_clipboard(draw_manager, resources, clipboard))
    }

    /// クリップボードの文字を貼り付ける。
    pub fn paste_from_clipboard(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
        self.focused_window_mut().is_some_and(|window| window.paste_from_clipboard(draw_manager, resources, clipboard))
    }

    /// 文字を、フォーカスを持っている部品に貼り付ける。
    pub fn paste(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        self.focused_window_mut().is_some_and(|window| window.paste(draw_manager, resources, text))
    }

    /// 字を打つ位置を、画面の画素で。IME の候補を出す場所に使う。
    pub fn ime_cursor_area(&self, draw_manager: &DrawManager) -> Option<[f32; 4]> {
        self.focused_window()?.ime_cursor_area(draw_manager)
    }

    /// メニューで押された項目の関数を呼ぶ。押された項目が無ければ `false`。
    pub fn run_context_menu_event(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
        let mut ran = false;
        for (_, window) in &mut self.windows {
            ran |= window.run_context_menu_event(draw_manager, resources, clipboard);
        }
        ran
    }
}

/// 出来事を受け取る gui。窓 1 枚（[`WindowFrame`]）でも、何枚も重ねたもの（[`GuiManager`]）でも同じ形で呼べる。
///
/// winit とつなぐ側（`gueiz::winit_gui` など）は、これだけを知っていれば足ります。
/// 中身はそれぞれの同じ名前の関数と同じです。
pub trait Gui {
    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool;
    fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool;
    fn mouse_right_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool;
    fn mouse_right_released(&mut self) -> bool;
    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool;
    fn mouse_wheel(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32, lines: [f32; 2]) -> bool;
    fn cursor_shape(&self, draw_manager: &DrawManager, x: f32, y: f32) -> CursorShape;

    fn wants_ime(&self) -> bool;
    fn ime_cursor_area(&self, draw_manager: &DrawManager) -> Option<[f32; 4]>;
    fn text_input(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool;
    fn ime_preedit(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str, cursor: Option<usize>) -> bool;
    fn key_pressed(&mut self, draw_manager: &mut DrawManager, resources: &Resources, key: EditKey) -> bool;
    fn modifiers_changed(&mut self, modifiers: KeyModifiers);

    fn copy_to_clipboard(&self, clipboard: &mut dyn Clipboard) -> bool;
    fn cut_to_clipboard(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool;
    fn paste_from_clipboard(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool;
    fn run_context_menu_event(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool;

    fn next_update(&self) -> Option<Instant>;
}

/// [`Gui`] を、同じ名前の関数へそのまま渡して実装する。
macro_rules! forward_gui {
    ($target:ty) => {
        impl Gui for $target {
            fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
                <$target>::mouse_left_pressed(self, draw_manager, x, y)
            }

            fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
                <$target>::mouse_left_released(self, draw_manager, x, y)
            }

            fn mouse_right_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
                <$target>::mouse_right_pressed(self, draw_manager, x, y)
            }

            fn mouse_right_released(&mut self) -> bool {
                <$target>::mouse_right_released(self)
            }

            fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
                <$target>::mouse_moved(self, draw_manager, x, y)
            }

            fn mouse_wheel(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32, lines: [f32; 2]) -> bool {
                <$target>::mouse_wheel(self, draw_manager, x, y, lines)
            }

            fn cursor_shape(&self, draw_manager: &DrawManager, x: f32, y: f32) -> CursorShape {
                <$target>::cursor_shape(self, draw_manager, x, y)
            }

            fn wants_ime(&self) -> bool {
                <$target>::wants_ime(self)
            }

            fn ime_cursor_area(&self, draw_manager: &DrawManager) -> Option<[f32; 4]> {
                <$target>::ime_cursor_area(self, draw_manager)
            }

            fn text_input(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
                <$target>::text_input(self, draw_manager, resources, text)
            }

            fn ime_preedit(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str, cursor: Option<usize>) -> bool {
                <$target>::ime_preedit(self, draw_manager, resources, text, cursor)
            }

            fn key_pressed(&mut self, draw_manager: &mut DrawManager, resources: &Resources, key: EditKey) -> bool {
                <$target>::key_pressed(self, draw_manager, resources, key)
            }

            fn modifiers_changed(&mut self, modifiers: KeyModifiers) {
                <$target>::modifiers_changed(self, modifiers)
            }

            fn copy_to_clipboard(&self, clipboard: &mut dyn Clipboard) -> bool {
                <$target>::copy_to_clipboard(self, clipboard)
            }

            fn cut_to_clipboard(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
                <$target>::cut_to_clipboard(self, draw_manager, resources, clipboard)
            }

            fn paste_from_clipboard(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
                <$target>::paste_from_clipboard(self, draw_manager, resources, clipboard)
            }

            fn run_context_menu_event(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
                <$target>::run_context_menu_event(self, draw_manager, resources, clipboard)
            }

            fn next_update(&self) -> Option<Instant> {
                <$target>::next_update(self)
            }
        }
    };
}

forward_gui!(WindowFrame);
forward_gui!(GuiManager);
