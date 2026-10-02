//! ウィジェットの識別子。
//!
//! # なぜ添字ではなく世代付きの鍵なのか
//!
//! ウィジェットは [`WidgetTree`](crate::gui::tree::WidgetTree) の連続した枠に
//! 入っていて、消すと枠が空きます。空いた枠は次の追加で**再利用します**。
//!
//! ここで鍵が単なる添字だと、消えたウィジェットを指していた古い鍵が、
//! 同じ枠に入った**別の**ウィジェットを指してしまいます。焦点や掴みは
//! フレームをまたいで鍵を持ち回るので、これは必ず踏みます。
//!
//! そこで枠ごとに**世代**を数え、鍵に埋めます。枠が再利用されると世代が
//! 進むので、古い鍵は世代が合わず弾かれます。
//!
//! ```
//! # use gueiz_2d::gui::id::WidgetId;
//! // 既定値は「誰も指していない」。
//! assert_eq!(WidgetId::default(), WidgetId::NONE);
//! assert!(WidgetId::NONE.is_none());
//! ```

use std::fmt::{Display, Formatter};
use std::num::NonZeroU32;

/// ウィジェット 1 つを指す鍵。
///
/// `Copy` なので気軽に持ち回せます。指す先が消えていることはあるので、
/// 中身を触るときは [`WidgetTree::get`](crate::gui::tree::WidgetTree::get) を
/// 通して `Option` で受けてください。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug, Default)]
pub struct WidgetId {
    index: u32,
    /// `None` が「誰も指していない」を表す。世代に 0 を使わないので、
    /// 空き表現が中に入り、`WidgetId` 自身が 8 バイトのまま
    /// [`WidgetId::NONE`] を持てる。外側を `Option` で包む必要がない。
    generation: Option<NonZeroU32>,
}

impl WidgetId {
    /// 誰も指していない鍵。
    pub const NONE: Self = Self {
        index: 0,
        generation: None,
    };

    pub(crate) const fn new(index: u32, generation: NonZeroU32) -> Self {
        Self {
            index,
            generation: Some(generation),
        }
    }

    pub fn is_none(self) -> bool {
        self.generation.is_none()
    }

    pub fn is_some(self) -> bool {
        self.generation.is_some()
    }

    pub(crate) fn index(self) -> usize {
        self.index as usize
    }

    pub(crate) fn generation(self) -> Option<NonZeroU32> {
        self.generation
    }
}

impl Display for WidgetId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.generation {
            Some(generation) => write!(f, "#{}v{}", self.index, generation),
            None => write!(f, "#none"),
        }
    }
}

/// 外から名前で引くための札。
///
/// [`WidgetId`] は木に入れたときに決まるので、作る前には分かりません。
/// 「設定画面の音量つまみ」のように**コードの側が名前で掴みたい**ものには
/// これを付けておき、[`WidgetTree::find`](crate::gui::tree::WidgetTree::find)
/// で引きます。
///
/// 文字列は比較のたびに舐めると高くつくので、作るときに 1 度だけ畳みます。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug)]
pub struct Tag(u64);

impl Tag {
    /// 名前から札を作る。同じ名前なら必ず同じ札になる。
    ///
    /// `const` なので、定数として置けます。**毎フレーム作り直さずに
    /// 1 箇所で名前を決められる**ので、こちらを勧めます。
    ///
    /// ```
    /// # use gueiz_2d::gui::id::Tag;
    /// const OK: Tag = Tag::new("ok");
    /// const CANCEL: Tag = Tag::new("cancel");
    ///
    /// assert_ne!(OK, CANCEL);
    /// assert_eq!(OK, Tag::new("ok"));
    /// ```
    pub const fn new(name: &str) -> Self {
        // FNV-1a。短い名前しか来ないので、これで十分に散る。
        // `const` で回すので `for` ではなく `while`。
        let bytes = name.as_bytes();
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut index = 0;

        while index < bytes.len() {
            hash ^= bytes[index] as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            index += 1;
        }

        Self(hash)
    }
}

impl From<&str> for Tag {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl Display for Tag {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "tag:{:016x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_the_default() {
        assert!(WidgetId::default().is_none());
        assert!(!WidgetId::default().is_some());
    }

    #[test]
    fn id_stays_two_words() {
        // 世代に 0 を使わないので `None` が空き表現に入り、
        // 「誰も指していない」を別の枠無しで表せる。
        assert_eq!(size_of::<WidgetId>(), 8);
    }

    #[test]
    fn same_name_makes_the_same_tag() {
        assert_eq!(Tag::new("volume"), Tag::from("volume"));
        assert_ne!(Tag::new("volume"), Tag::new("brightness"));
    }

    #[test]
    fn tags_can_be_constants() {
        const VOLUME: Tag = Tag::new("volume");

        assert_eq!(VOLUME, Tag::new("volume"));
        assert_ne!(VOLUME, Tag::new("volume "));
    }
}
