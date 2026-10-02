//! 入力。**プラットフォームの型は一切出てきません。**
//!
//! # 2 種類ある
//!
//! | | 誰が作るか | 座標 | 誰が見るか |
//! |---|---|---|---|
//! | [`InputEvent`] | 呼ぶ側（ウィンドウ層から変換して流す） | 画面の左上から | [`Gui`](crate::gui::Gui) だけ |
//! | [`Event`] | [`Gui`] | **そのウィジェットの左上から** | ウィジェット |
//!
//! 分けている理由は 2 つあります。
//!
//! 1. **`gueiz-2d` はウィンドウを知りません。** `gueiz-window` に依存すると
//!    描画クレートが窓の都合に縛られます。呼ぶ側が 1 箇所で詰め替えます。
//! 2. **ウィジェットに届くのは生の入力ではありません。** 「乗った」「離れた」
//!    「焦点を得た」は入力そのものには無く、[`Gui`] が当たり判定の差分から
//!    組み立てます。座標も親からのずれを引いた**局所座標**に直します。
//!
//! # 押しっぱなしは掴みになる
//!
//! つまみを掴んで枠の外まで引いても、放すまでは**最初に掴んだウィジェット**に
//! 届き続けます（ポインタの掴み）。これが無いと、少し外へ出た瞬間に
//! ドラッグが切れます。掴みは [`Gui`] が勝手に張り、放した時点で外します。
//!
//! [`Gui`]: crate::gui::Gui

use bitflags::bitflags;

use crate::gui::geometry::Point;

/// 呼ぶ側が [`Gui`](crate::gui::Gui) に流す生の入力。
///
/// 座標は**画面の左上を原点とした論理ピクセル**です。物理ピクセルしか
/// 持っていないなら、倍率で割ってから渡してください
/// （[`Gui::set_scale_factor`](crate::gui::Gui::set_scale_factor)）。
#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum InputEvent {
    /// ポインタが動いた。
    PointerMoved { position: Point },
    /// ボタンが押された。
    PointerPressed {
        position: Point,
        button: PointerButton,
    },
    /// ボタンが放された。
    PointerReleased {
        position: Point,
        button: PointerButton,
    },
    /// ポインタが窓から出た。**押しっぱなしのときは掴みを切りません。**
    PointerLeft,
    /// 車輪やタッチパッド。
    Scrolled { position: Point, delta: ScrollDelta },
    /// 鍵が押された。`repeat` は押しっぱなしの連射。
    KeyPressed { key: Key, repeat: bool },
    /// 鍵が放された。
    KeyReleased { key: Key },
    /// 確定した文字。**鍵とは別に来ます。**
    ///
    /// `A` を打つと `KeyPressed` と `Text("A")` の両方が来ます。
    /// 入力欄は `Text` を、ショートカットは `KeyPressed` を見ます。
    /// かな漢字変換を通すと、鍵を押していないのに `Text` だけ来ます。
    Text(String),
    /// 修飾鍵の状態が変わった。
    ModifiersChanged(Modifiers),
}

/// ウィジェットに届く出来事。座標は**そのウィジェットの左上が原点**。
#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum Event {
    /// ポインタが乗った。[`Gui`](crate::gui::Gui) が当たり判定の差分から作る。
    PointerEnter { position: Point },
    /// 乗ったまま動いた。
    PointerMove { position: Point, delta: Point },
    /// ポインタが外れた。**位置は持ちません**（もう上に無いので）。
    PointerExit,
    PointerPressed {
        position: Point,
        button: PointerButton,
    },
    /// 放された。`inside` は放した場所がまだ自分の上かどうか。
    ///
    /// 押してから外へずらして放すのは「やめた」の合図です。
    /// `inside` が false なら押下を取り消してください。
    PointerReleased {
        position: Point,
        button: PointerButton,
        inside: bool,
    },
    Scrolled { position: Point, delta: ScrollDelta },
    /// 焦点を持っているあいだだけ届く。
    KeyPressed { key: Key, repeat: bool },
    KeyReleased { key: Key },
    /// 焦点を持っているあいだだけ届く。
    Text(String),
    /// 焦点を得た。
    FocusGained,
    /// 焦点を失った。入力欄なら確定する合図。
    FocusLost,
}

impl Event {
    /// 局所座標。持っていない出来事なら `None`。
    pub fn position(&self) -> Option<Point> {
        match self {
            Self::PointerEnter { position }
            | Self::PointerMove { position, .. }
            | Self::PointerPressed { position, .. }
            | Self::PointerReleased { position, .. }
            | Self::Scrolled { position, .. } => Some(*position),

            Self::PointerExit
            | Self::KeyPressed { .. }
            | Self::KeyReleased { .. }
            | Self::Text(_)
            | Self::FocusGained
            | Self::FocusLost => None,
        }
    }

    /// ポインタ由来か。
    pub fn is_pointer(&self) -> bool {
        matches!(
            self,
            Self::PointerEnter { .. }
                | Self::PointerMove { .. }
                | Self::PointerExit
                | Self::PointerPressed { .. }
                | Self::PointerReleased { .. }
                | Self::Scrolled { .. }
        )
    }

    /// 鍵盤由来か。**焦点を持つウィジェットにしか届きません。**
    pub fn is_keyboard(&self) -> bool {
        matches!(
            self,
            Self::KeyPressed { .. } | Self::KeyReleased { .. } | Self::Text(_)
        )
    }
}

/// 出来事を受け取ったかどうか。
///
/// [`EventResult::Consumed`] を返すと、そこで止まって**親へは上がりません**。
/// 返さなければ親、その親、と順に試されます。背景のパネルで
/// 「どこを押しても閉じる」を作るには、ボタンが取らなかったぶんだけが
/// 上がってくる、この形が要ります。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub enum EventResult {
    /// 自分には関係がない。親へ上げる。
    #[default]
    Ignored,
    /// 受け取った。ここで止める。
    Consumed,
}

impl EventResult {
    pub fn is_consumed(self) -> bool {
        matches!(self, Self::Consumed)
    }

    /// 受け取ったなら受け取ったまま。そうでなければ `other` を試す。
    pub fn or(self, other: Self) -> Self {
        match self {
            Self::Consumed => Self::Consumed,
            Self::Ignored => other,
        }
    }

    /// `true` なら [`EventResult::Consumed`]。
    pub fn consumed_if(condition: bool) -> Self {
        if condition { Self::Consumed } else { Self::Ignored }
    }
}

impl From<bool> for EventResult {
    fn from(value: bool) -> Self {
        Self::consumed_if(value)
    }
}

/// ポインタのボタン。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
    /// それ以外。番号はプラットフォームのまま。
    Other(u16),
}

/// 送り量。
///
/// 行とピクセルを分けているのは、**行は文脈で長さが変わる**からです。
/// 文字が 16 px の一覧で 3 行送るのと、48 px の一覧で 3 行送るのは別の距離です。
/// 受け取る側が自分の行の高さを掛けます。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum ScrollDelta {
    /// 行数。車輪の刻み。
    Lines { x: f32, y: f32 },
    /// ピクセル。タッチパッドの滑らかな送り。
    Pixels { x: f32, y: f32 },
}

impl ScrollDelta {
    /// ピクセルに直す。`line_height` は 1 行ぶんの高さ。
    pub fn to_pixels(self, line_height: f32) -> Point {
        match self {
            Self::Lines { x, y } => Point::new(x * line_height, y * line_height),
            Self::Pixels { x, y } => Point::new(x, y),
        }
    }
}

bitflags! {
    /// 修飾鍵。
    ///
    /// `COMMAND` は macOS の Command、ほかでは Control を指します。
    /// ショートカットを 1 本で書けるよう、[`Modifiers::command`] を使ってください。
    #[derive(Clone, Copy)]
    #[derive(Eq, PartialEq)]
    #[derive(Hash)]
    #[derive(Debug, Default)]
    pub struct Modifiers: u8 {
        const SHIFT = 1 << 0;
        const CONTROL = 1 << 1;
        const ALT = 1 << 2;
        /// macOS の Command / Windows の Windows 鍵。
        const SUPER = 1 << 3;
    }
}

impl Modifiers {
    /// この環境で「コピーの鍵」として使われる修飾が押されているか。
    ///
    /// macOS なら Command、ほかなら Control。
    pub fn command(self) -> bool {
        if cfg!(target_os = "macos") {
            self.contains(Self::SUPER)
        } else {
            self.contains(Self::CONTROL)
        }
    }

    pub fn shift(self) -> bool {
        self.contains(Self::SHIFT)
    }

    pub fn alt(self) -> bool {
        self.contains(Self::ALT)
    }

    /// 修飾なし。
    pub fn is_empty_set(self) -> bool {
        self.is_empty()
    }
}

/// 鍵。
///
/// 文字の出る鍵は [`Key::Character`]、出ない鍵は [`Key::Named`] です。
/// 配列の違いを吸収するため、**刻印ではなく意味**で持ちます。
#[derive(Clone)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug)]
pub enum Key {
    /// 文字の出る鍵。小文字に正規化して入れてください。
    Character(char),
    Named(NamedKey),
}

impl Key {
    /// その文字の鍵か。大小は見ない。
    pub fn is_character(&self, character: char) -> bool {
        match self {
            Self::Character(pressed) => {
                pressed.eq_ignore_ascii_case(&character)
            }
            Self::Named(_) => false,
        }
    }

    pub fn is_named(&self, named: NamedKey) -> bool {
        matches!(self, Self::Named(key) if *key == named)
    }
}

/// 文字の出ない鍵。
///
/// 足りなければ増やしてください。ここに無い鍵は呼ぶ側で落ちます。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug)]
pub enum NamedKey {
    // 移動
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,

    // 編集
    Backspace,
    Delete,
    Enter,
    Tab,
    Space,
    Escape,
    Insert,

    // 修飾（押し下げ自体を見たいとき用）
    Shift,
    Control,
    Alt,
    Super,
    CapsLock,

    // 機能
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumed_stops_the_chain() {
        assert_eq!(
            EventResult::Consumed.or(EventResult::Ignored),
            EventResult::Consumed
        );
        assert_eq!(
            EventResult::Ignored.or(EventResult::Consumed),
            EventResult::Consumed
        );
        assert_eq!(EventResult::from(false), EventResult::Ignored);
    }

    #[test]
    fn lines_scale_with_line_height() {
        let lines = ScrollDelta::Lines { x: 0.0, y: -3.0 };
        assert_eq!(lines.to_pixels(20.0), Point::new(0.0, -60.0));

        // ピクセルは行の高さを見ない。
        let pixels = ScrollDelta::Pixels { x: 0.0, y: -7.5 };
        assert_eq!(pixels.to_pixels(20.0), Point::new(0.0, -7.5));
    }

    #[test]
    fn character_match_ignores_case() {
        assert!(Key::Character('a').is_character('A'));
        assert!(!Key::Named(NamedKey::Enter).is_character('a'));
        assert!(Key::Named(NamedKey::Enter).is_named(NamedKey::Enter));
    }

    #[test]
    fn keyboard_events_carry_no_position() {
        assert!(Event::KeyReleased { key: Key::Named(NamedKey::Tab) }.position().is_none());
        assert!(Event::PointerExit.position().is_none());
        assert!(
            Event::PointerEnter { position: Point::new(1.0, 2.0) }
                .position()
                .is_some()
        );
    }
}
