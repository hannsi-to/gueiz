//! クリップボード。
//!
//! gui は OS に触れないので、文字の出し入れだけをこのトレイトで受け取ります。
//! OS のクリップボードは、つなぐ側（`gueiz::winit_gui` など）が実装して渡します。
//! テストや、OS のクリップボードを使わないときは [`MemoryClipboard`] を使えます。

/// 文字を出し入れできるクリップボード。
pub trait Clipboard {
    /// いま入っている文字。入っていないか、読めなければ `None`。
    fn get_text(&mut self) -> Option<String>;

    /// 文字を入れる。入れられたら `true`。
    fn set_text(&mut self, text: &str) -> bool;
}

/// 手元に覚えておくだけのクリップボード。OS とはやりとりしない。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// いま入っている文字。
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

impl Clipboard for MemoryClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.text.clone()
    }

    fn set_text(&mut self, text: &str) -> bool {
        self.text = Some(text.to_string());
        true
    }
}
