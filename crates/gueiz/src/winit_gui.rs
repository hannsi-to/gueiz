//! winit と gui（[`WindowFrame`]）をつなぐ。
//!
//! gui は winit を知りません。指の位置・押したボタン・打った文字を受け取り、
//! 「ここではこの形の指を出したい」「いま文字を打ち込める」を答えるだけです。
//! winit の出来事をそこへ振り分けるのと、答えを実際の窓に映すのは、ここの
//! [`WinitGui`] が引き受けます。
//!
//! - 指の入力（押す・放す・動かす）を窓に渡す
//! - キーを [`EditKey`] か打った文字に直して渡す。IME の確定と変換中の文字も渡す
//! - Shift・Ctrl の押し具合を [`KeyModifiers`] にして渡す
//! - Ctrl+A・Z・Y と、OS のクリップボードを使うコピー・切り取り・貼り付け（Ctrl+C・X・V）。
//!   macOS では Ctrl の代わりに Command
//! - 右クリックのメニューで押された項目の関数を呼ぶ（OS のクリップボードを渡して）
//! - キャレットの点滅など、gui が時間で動くあいだは、その時刻に描き直しを頼む（[`WinitGui::about_to_wait`]）
//! - 指の形を、窓が答えた [`CursorShape`] に合わせる
//! - 打ち込む欄がフォーカスを持っているあいだだけ IME を点け、候補を字を打つ位置に出す
//!
//! # 使い方
//!
//! ```ignore
//! fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
//!     let response = self.winit_gui.window_event(
//!         window.as_ref(),
//!         &event,
//!         &mut scene.window_frame,
//!         &mut scene.draw_manager,
//!         &scene.resources,
//!     );
//!
//!     if response.redraw {
//!         window.request_redraw();
//!     }
//! }
//! ```
//!
//! 描く前には、いつもどおり [`WindowFrame::update`] を 1 フレーム 1 回呼んでください。
//! キャレットの点滅などを動かすには、`ApplicationHandler::about_to_wait` から
//! [`WinitGui::about_to_wait`] も呼んでください。

use winit::cursor::CursorIcon;
use winit::dpi::{PhysicalPosition, PhysicalSize, Position, Size};
use std::time::Instant;
use winit::event::{ButtonSource, ElementState, Ime, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{ImeCapabilities, ImeEnableRequest, ImeHint, ImePurpose, ImeRequest, ImeRequestData, Window};

use gueiz_2d::draw_manager::DrawManager;
use gueiz_2d::gui::window_frame::WindowFrame;
use gueiz_2d::gui::clipboard::Clipboard;
use gueiz_2d::gui::window_item::{CursorShape, EditKey, KeyModifiers};
use gueiz_2d::resource::Resources;

/// 出来事を渡した結果。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct GuiResponse {
    /// gui が受け取った。`true` なら、後ろ（ゲームの操作など）へは渡さないでください。
    pub consumed: bool,
    /// 見た目が変わったかもしれない。描き直しを頼んでください。
    pub redraw: bool,
}

/// winit の出来事を gui へ渡し、指の形と IME を窓に映す。
///
/// 窓 1 枚につき 1 つ持ちます。いま出している指の形や IME の状態を覚えておき、
/// 変わったときだけ窓に伝えます。
#[derive(Default)]
pub struct WinitGui {
    /// いま押している Shift・Ctrl など。
    modifiers: ModifiersState,
    /// OS のクリップボード。
    clipboard: OsClipboard,
    /// 指の位置（画面の画素）。窓の外に出たら `None`。
    pointer: Option<[f32; 2]>,
    /// いま出している指の形。
    cursor: CursorShape,
    /// IME を点けているか。
    ime_enabled: bool,
    /// IME に最後に伝えた、字を打つ位置。
    ime_area: Option<[f32; 4]>,
}

impl WinitGui {
    pub fn new() -> Self {
        Self::default()
    }

    /// winit の出来事を gui へ渡す。どの出来事を渡しても構いません。関係ないものは素通りします。
    pub fn window_event(
        &mut self,
        window: &dyn Window,
        event: &WindowEvent,
        window_frame: &mut WindowFrame,
        draw_manager: &mut DrawManager,
        resources: &Resources,
    ) -> GuiResponse {
        let mut consumed = self.dispatch(event, window_frame, draw_manager, resources);

        // 右クリックのメニューで押された項目の関数。
        consumed |= window_frame.run_context_menu_event(draw_manager, resources, &mut self.clipboard);

        self.update_cursor(window, window_frame, draw_manager);
        self.update_ime(window, window_frame, draw_manager);

        GuiResponse {
            consumed,
            // 窓の外を押しても、フォーカスが外れて見た目が変わることがあるので描き直す。
            redraw: consumed || matches!(event, WindowEvent::PointerButton { .. }),
        }
    }

    /// 出来事を窓の関数に振り分ける。受け取られたら `true`。
    fn dispatch(
        &mut self,
        event: &WindowEvent,
        window_frame: &mut WindowFrame,
        draw_manager: &mut DrawManager,
        resources: &Resources,
    ) -> bool {
        match event {
            WindowEvent::PointerButton { state, position, button, .. } => {
                let [x, y] = [position.x as f32, position.y as f32];
                self.pointer = Some([x, y]);

                match (button, state) {
                    (ButtonSource::Mouse(MouseButton::Left), ElementState::Pressed) => {
                        window_frame.mouse_left_pressed(draw_manager, x, y)
                    }
                    (ButtonSource::Mouse(MouseButton::Left), ElementState::Released) => {
                        window_frame.mouse_left_released(draw_manager, x, y)
                    }
                    (ButtonSource::Mouse(MouseButton::Right), ElementState::Pressed) => {
                        window_frame.mouse_right_pressed(draw_manager, x, y)
                    }
                    (ButtonSource::Mouse(MouseButton::Right), ElementState::Released) => {
                        window_frame.mouse_right_released()
                    }
                    _ => false,
                }
            }

            WindowEvent::PointerMoved { position, .. } => {
                let [x, y] = [position.x as f32, position.y as f32];
                self.pointer = Some([x, y]);

                window_frame.mouse_moved(draw_manager, x, y)
            }

            WindowEvent::PointerLeft { .. } => {
                self.pointer = None;
                false
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                window_frame.modifiers_changed(self.key_modifiers());
                false
            }

            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key_input(event, window_frame, draw_manager, resources)
            }

            WindowEvent::Ime(Ime::Commit(text)) => window_frame.text_input(draw_manager, resources, text),

            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                window_frame.ime_preedit(draw_manager, resources, text, cursor.map(|(start, _)| start))
            }

            _ => false,
        }
    }

    /// キーを、文字を書き換えるキーか打った文字に直して渡す。
    fn key_input(&mut self, event: &KeyEvent, window_frame: &mut WindowFrame, draw_manager: &mut DrawManager, resources: &Resources) -> bool {
        let command = self.command();

        if command {
            if let Some(handled) = self.shortcut(&event.logical_key, window_frame, draw_manager, resources) {
                return handled;
            }
        }

        if let Some(key) = edit_key(&event.logical_key, command) {
            return window_frame.key_pressed(draw_manager, resources, key);
        }

        // Ctrl を押しながらの字は、打った字ではない。
        if command {
            return false;
        }

        match &event.text {
            Some(text) => window_frame.text_input(draw_manager, resources, text),
            None => false,
        }
    }

    /// Ctrl（macOS は Command）との組み合わせ。組み合わせでなければ `None`。
    fn shortcut(&mut self, key: &Key, window_frame: &mut WindowFrame, draw_manager: &mut DrawManager, resources: &Resources) -> Option<bool> {
        let Key::Character(character) = key else {
            return None;
        };
        let shift = self.modifiers.shift_key();

        let clipboard = &mut self.clipboard;
        let handled = match character.to_lowercase().as_str() {
            "a" => window_frame.key_pressed(draw_manager, resources, EditKey::SelectAll),
            "z" if shift => window_frame.key_pressed(draw_manager, resources, EditKey::Redo),
            "z" => window_frame.key_pressed(draw_manager, resources, EditKey::Undo),
            "y" => window_frame.key_pressed(draw_manager, resources, EditKey::Redo),
            // 見た目は変わらないが、選んでいたら受け取ったので後ろへは渡さない。
            "c" => window_frame.copy_to_clipboard(clipboard),
            "x" => window_frame.cut_to_clipboard(draw_manager, resources, clipboard),
            "v" => window_frame.paste_from_clipboard(draw_manager, resources, clipboard),
            _ => return None,
        };

        Some(handled)
    }

    /// イベントを待つ前に呼ぶ（`ApplicationHandler::about_to_wait` から）。
    ///
    /// gui が時間で動いているあいだ（キャレットの点滅、引いたままの自動スクロール）は、
    /// その時刻に起きて描き直しを頼みます。動くものが無ければ、次の入力まで眠ります。
    pub fn about_to_wait(&self, event_loop: &dyn ActiveEventLoop, window: &dyn Window, window_frame: &WindowFrame) {
        match window_frame.next_update() {
            Some(at) if at <= Instant::now() => {
                window.request_redraw();
                // 描いたあとの update で次の時刻が決まるので、それまでは待つ。
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    /// Ctrl（macOS は Command）を押しているか。AltGr（Ctrl+Alt）で打つ字は除く。
    fn command(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.modifiers.meta_key()
        } else {
            self.modifiers.control_key() && !self.modifiers.alt_key()
        }
    }

    /// gui に渡す修飾キー。単語ごとに動かすのは、macOS では Option、ほかでは Ctrl。
    fn key_modifiers(&self) -> KeyModifiers {
        KeyModifiers {
            shift: self.modifiers.shift_key(),
            word: if cfg!(target_os = "macos") { self.modifiers.alt_key() } else { self.modifiers.control_key() },
        }
    }

    /// 指の形を、窓が答えた形に合わせる。変わったときだけ窓に伝える。
    fn update_cursor(&mut self, window: &dyn Window, window_frame: &WindowFrame, draw_manager: &DrawManager) {
        let shape = match self.pointer {
            Some([x, y]) => window_frame.cursor_shape(draw_manager, x, y),
            None => CursorShape::Default,
        };

        if shape != self.cursor {
            window.set_cursor(cursor_icon(shape).into());
            self.cursor = shape;
        }
    }

    /// 打ち込む欄がフォーカスを持っていれば IME を点け、候補を字を打つ位置の近くに出す。
    fn update_ime(&mut self, window: &dyn Window, window_frame: &WindowFrame, draw_manager: &DrawManager) {
        let focused = window_frame.wants_ime();
        let area = window_frame.ime_cursor_area(draw_manager);

        if focused != self.ime_enabled {
            if focused {
                let (position, size) = cursor_area(area.unwrap_or_default());
                let capabilities = ImeCapabilities::new().with_hint_and_purpose().with_cursor_area();
                let request_data = ImeRequestData::default()
                    .with_hint_and_purpose(ImeHint::NONE, ImePurpose::Normal)
                    .with_cursor_area(position, size);

                if let Some(request) = ImeEnableRequest::new(capabilities, request_data) {
                    if let Err(error) = window.request_ime_update(ImeRequest::Enable(request)) {
                        log::warn!("failed to enable the IME: {error}");
                    }
                }
            } else {
                // 消すのは失敗しない。
                let _ = window.request_ime_update(ImeRequest::Disable);
            }

            self.ime_enabled = focused;
            self.ime_area = area;
            return;
        }

        if focused && area != self.ime_area {
            if let Some(area) = area {
                let (position, size) = cursor_area(area);
                let request_data = ImeRequestData::default().with_cursor_area(position, size);

                if let Err(error) = window.request_ime_update(ImeRequest::Update(request_data)) {
                    log::warn!("failed to move the IME candidates: {error}");
                }
            }

            self.ime_area = area;
        }
    }
}

/// OS のクリップボード。初めて使うときに開く。開けなければ、記録を残して何もしない。
#[derive(Default)]
struct OsClipboard {
    clipboard: Option<arboard::Clipboard>,
}

impl OsClipboard {
    fn with<T>(&mut self, f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>) -> Option<T> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(error) => {
                    log::warn!("failed to open the clipboard: {error}");
                    return None;
                }
            }
        }

        match f(self.clipboard.as_mut()?) {
            Ok(value) => Some(value),
            Err(error) => {
                log::warn!("failed to use the clipboard: {error}");
                None
            }
        }
    }
}

impl Clipboard for OsClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.with(|clipboard| clipboard.get_text())
    }

    fn set_text(&mut self, text: &str) -> bool {
        self.with(|clipboard| clipboard.set_text(text)).is_some()
    }
}

/// 文字を書き換えるキーなら、その [`EditKey`]。
///
/// `command` は Ctrl（macOS は Command）を押しているか。macOS では Command+←→ が行の先頭・末尾へ動きます。
pub fn edit_key(key: &Key, command: bool) -> Option<EditKey> {
    let line_jump = command && cfg!(target_os = "macos");

    match key {
        Key::Named(NamedKey::Backspace) => Some(EditKey::Backspace),
        Key::Named(NamedKey::Delete) => Some(EditKey::Delete),
        Key::Named(NamedKey::ArrowLeft) if line_jump => Some(EditKey::Home),
        Key::Named(NamedKey::ArrowRight) if line_jump => Some(EditKey::End),
        Key::Named(NamedKey::ArrowLeft) => Some(EditKey::Left),
        Key::Named(NamedKey::ArrowRight) => Some(EditKey::Right),
        Key::Named(NamedKey::ArrowUp) => Some(EditKey::Up),
        Key::Named(NamedKey::ArrowDown) => Some(EditKey::Down),
        Key::Named(NamedKey::Home) => Some(EditKey::Home),
        Key::Named(NamedKey::End) => Some(EditKey::End),
        Key::Named(NamedKey::Enter) => Some(EditKey::Enter),
        Key::Named(NamedKey::Escape) => Some(EditKey::Escape),
        _ => None,
    }
}

/// gui の指の形を、winit の指の形に。
pub fn cursor_icon(shape: CursorShape) -> CursorIcon {
    match shape {
        CursorShape::Default => CursorIcon::Default,
        CursorShape::Pointer => CursorIcon::Pointer,
        CursorShape::Text => CursorIcon::Text,
        CursorShape::Move => CursorIcon::Move,
        CursorShape::ResizeHorizontal => CursorIcon::EwResize,
        CursorShape::ResizeVertical => CursorIcon::NsResize,
        CursorShape::ResizeNwse => CursorIcon::NwseResize,
        CursorShape::ResizeNesw => CursorIcon::NeswResize,
    }
}

/// 画面の画素の `[x, y, 幅, 高さ]` を、IME に渡す形に。
fn cursor_area([x, y, width, height]: [f32; 4]) -> (Position, Size) {
    (
        PhysicalPosition::new(x as f64, y as f64).into(),
        PhysicalSize::new(width as f64, height as f64).into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cursor_shape_has_an_icon() {
        // 形が増えたら、ここと cursor_icon の両方に足すこと。
        let shapes = [
            CursorShape::Default,
            CursorShape::Pointer,
            CursorShape::Text,
            CursorShape::Move,
            CursorShape::ResizeHorizontal,
            CursorShape::ResizeVertical,
            CursorShape::ResizeNwse,
            CursorShape::ResizeNesw,
        ];

        let icons: Vec<CursorIcon> = shapes.into_iter().map(cursor_icon).collect();
        for (index, icon) in icons.iter().enumerate() {
            assert!(!icons[..index].contains(icon), "{icon:?} が 2 つの形に使われている");
        }
    }

    #[test]
    fn editing_keys_are_recognised() {
        assert_eq!(edit_key(&Key::Named(NamedKey::Backspace), false), Some(EditKey::Backspace));
        assert_eq!(edit_key(&Key::Named(NamedKey::ArrowLeft), false), Some(EditKey::Left));
        assert_eq!(edit_key(&Key::Named(NamedKey::ArrowUp), false), Some(EditKey::Up));
        assert_eq!(edit_key(&Key::Character("a".into()), false), None);
    }
}
