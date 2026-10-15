//! 1 行の文字を書き換える中身。描画は持たない。
//!
//! [`InputText`](crate::gui::input_text::InputText) が、打ち込まれた文字やキーを
//! ここに渡し、結果の文字と字を打つ位置・選んでいる範囲を描きます。
//! 描画と切り離してあるので、GPU なしで確かめられます。
//!
//! 位置はすべて `text` の中の**バイト位置**で、いつも字の境目にあります。

use std::ops::Range;

/// 元に戻せる回数。
const UNDO_LIMIT: usize = 100;

/// 1 行の文字と、字を打つ位置・選んでいる範囲・元に戻す記録。
#[derive(Debug, Clone, Default)]
pub struct TextEdit {
    text: String,
    /// 字を打つ位置。選んでいるときは、選び始めと反対の端。
    cursor: usize,
    /// 選び始めた位置。`cursor` と同じなら何も選んでいない。
    anchor: usize,
    /// 入れられる字の数の上限。
    max_length: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// 最後の書き換えの種類。同じ種類が続くあいだは、元に戻す記録を 1 つにまとめる。
    last_edit: Option<EditKind>,
}

/// 元に戻すために覚えておく、書き換える前の状態。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

/// 書き換えの種類。続けて打った字・続けて消した字は、まとめて 1 回で戻す。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
    /// 選んだ範囲の置き換えや貼り付けなど。いつも 1 回ずつ戻す。
    Other,
}

impl TextEdit {
    pub fn new() -> Self {
        Self::default()
    }

    // --- 見る ---

    pub fn text(&self) -> &str {
        &self.text
    }

    /// 字を打つ位置。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 選んでいる範囲。何も選んでいなければ `None`。
    pub fn selection(&self) -> Option<Range<usize>> {
        (self.anchor != self.cursor).then(|| self.anchor.min(self.cursor)..self.anchor.max(self.cursor))
    }

    /// 選んでいる文字。
    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|range| &self.text[range])
    }

    /// 入れられる字の数の上限。`None` で上限なし。
    pub fn set_max_length(&mut self, max_length: Option<usize>) {
        self.max_length = max_length;
    }

    // --- 動かす ---

    /// 字を打つ位置を `index` へ。`extend` なら選んでいる範囲を広げる（Shift を押しながら）。
    ///
    /// `index` は字の境目に寄せます。
    pub fn move_to(&mut self, index: usize, extend: bool) {
        self.cursor = self.floor_boundary(index.min(self.text.len()));
        if !extend {
            self.anchor = self.cursor;
        }
        self.last_edit = None;
    }

    /// 1 字（`word` なら 1 単語）左へ。選んでいて広げないなら、範囲の左端へ縮める。
    pub fn move_left(&mut self, extend: bool, word: bool) {
        if let (Some(selection), false, false) = (self.selection(), extend, word) {
            return self.move_to(selection.start, false);
        }

        let index = if word { word_left(&self.text, self.cursor) } else { previous_boundary(&self.text, self.cursor) };
        self.move_to(index, extend);
    }

    /// 1 字（`word` なら 1 単語）右へ。選んでいて広げないなら、範囲の右端へ縮める。
    pub fn move_right(&mut self, extend: bool, word: bool) {
        if let (Some(selection), false, false) = (self.selection(), extend, word) {
            return self.move_to(selection.end, false);
        }

        let index = if word { word_right(&self.text, self.cursor) } else { next_boundary(&self.text, self.cursor) };
        self.move_to(index, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
        self.last_edit = None;
    }

    /// `index` にある単語を選ぶ（ダブルクリック）。空白の上なら、続く空白を選ぶ。
    pub fn select_word_at(&mut self, index: usize) {
        let index = self.floor_boundary(index.min(self.text.len()));

        // 末尾なら、その手前の字の単語。
        let Some(class) = self.text[index..]
            .chars()
            .next()
            .or_else(|| self.text[..index].chars().next_back())
            .map(char_class)
        else {
            return;
        };

        let start = self.text[..index]
            .char_indices()
            .rev()
            .take_while(|(_, character)| char_class(*character) == class)
            .last()
            .map_or(index, |(position, _)| position);
        let end = self.text[index..]
            .char_indices()
            .take_while(|(_, character)| char_class(*character) == class)
            .last()
            .map_or(index, |(position, character)| index + position + character.len_utf8());

        self.anchor = start;
        self.cursor = end;
        self.last_edit = None;
    }

    // --- 書き換える ---

    /// 字を打つ。選んでいれば置き換える。改行やタブなどの制御文字は入れない。
    /// 上限を超える分は捨てる。何か変わったら `true`。
    pub fn insert(&mut self, text: &str) -> bool {
        let selection = self.selection();
        let removed = selection.as_ref().map_or(0, |range| self.text[range.clone()].chars().count());
        let room = self
            .max_length
            .map_or(usize::MAX, |max| max.saturating_sub(self.text.chars().count() - removed));

        let text: String = text.chars().filter(|character| !character.is_control()).take(room).collect();
        if text.is_empty() && selection.is_none() {
            return false;
        }

        // 範囲を置き換えるのは、打ち続けとは別の 1 回にする。
        self.record(if selection.is_some() { EditKind::Other } else { EditKind::Insert });

        let range = selection.unwrap_or(self.cursor..self.cursor);
        self.text.replace_range(range.clone(), &text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;

        true
    }

    /// 貼り付け。打ち続けとまとめず、1 回で戻せるようにする。
    pub fn paste(&mut self, text: &str) -> bool {
        self.last_edit = None;
        let changed = self.insert(text);
        self.last_edit = None;

        changed
    }

    /// 選んでいる範囲を消す。選んでいなければ何もしない。
    pub fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection() else {
            return false;
        };

        self.record(EditKind::Other);
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.anchor = range.start;

        true
    }

    /// Backspace。選んでいれば範囲を、でなければ手前の 1 字（`word` なら 1 単語）を消す。
    pub fn delete_backward(&mut self, word: bool) -> bool {
        if self.selection().is_some() {
            return self.delete_selection();
        }

        let start = if word { word_left(&self.text, self.cursor) } else { previous_boundary(&self.text, self.cursor) };
        self.delete_range(start..self.cursor)
    }

    /// Delete。選んでいれば範囲を、でなければ後ろの 1 字（`word` なら 1 単語）を消す。
    pub fn delete_forward(&mut self, word: bool) -> bool {
        if self.selection().is_some() {
            return self.delete_selection();
        }

        let end = if word { word_right(&self.text, self.cursor) } else { next_boundary(&self.text, self.cursor) };
        self.delete_range(self.cursor..end)
    }

    fn delete_range(&mut self, range: Range<usize>) -> bool {
        if range.is_empty() {
            return false;
        }

        self.record(EditKind::Delete);
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.anchor = range.start;

        true
    }

    // --- 元に戻す ---

    /// 元に戻す。戻すものが無ければ `false`。
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };

        let current = self.snapshot();
        self.redo.push(current);
        self.restore(snapshot);

        true
    }

    /// やり直す。やり直すものが無ければ `false`。
    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };

        let current = self.snapshot();
        self.undo.push(current);
        self.restore(snapshot);

        true
    }

    /// 書き換える前の状態を覚える。同じ種類が続くあいだは、最初の 1 回だけ覚える。
    fn record(&mut self, kind: EditKind) {
        if kind == EditKind::Other || self.last_edit != Some(kind) {
            let snapshot = self.snapshot();
            self.undo.push(snapshot);

            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }

        self.redo.clear();
        self.last_edit = Some(kind);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), cursor: self.cursor, anchor: self.anchor }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
        self.last_edit = None;
    }

    /// `index` 以下で一番近い字の境目。
    fn floor_boundary(&self, mut index: usize) -> usize {
        while !self.text.is_char_boundary(index) {
            index -= 1;
        }
        index
    }
}

fn previous_boundary(text: &str, index: usize) -> usize {
    text[..index].char_indices().next_back().map_or(0, |(position, _)| position)
}

fn next_boundary(text: &str, index: usize) -> usize {
    text[index..].chars().next().map_or(index, |character| index + character.len_utf8())
}

/// 単語の区切りに使う、字の種類。
///
/// 空白・単語の字（文字と数字と `_`）・それ以外（記号）の 3 つに分けます。
/// 日本語の文字も「文字」なので、続く日本語はひとまとまりになります
/// （形態素で区切るブラウザとは違います）。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum CharClass {
    Space,
    Word,
    Symbol,
}

fn char_class(character: char) -> CharClass {
    if character.is_whitespace() {
        CharClass::Space
    } else if character.is_alphanumeric() || character == '_' {
        CharClass::Word
    } else {
        CharClass::Symbol
    }
}

/// 1 単語左（Ctrl+←）。手前の空白を飛ばし、その単語の頭へ。
fn word_left(text: &str, index: usize) -> usize {
    let mut characters = text[..index].char_indices().rev().peekable();

    while characters.next_if(|(_, character)| char_class(*character) == CharClass::Space).is_some() {}

    let Some(&(_, first)) = characters.peek() else {
        return 0;
    };
    let class = char_class(first);

    let mut start = index;
    while let Some((position, _)) = characters.next_if(|(_, character)| char_class(*character) == class) {
        start = position;
    }

    start.min(index)
}

/// 1 単語右（Ctrl+→）。いまの単語を抜け、続く空白も飛ばして次の単語の頭へ（Windows と同じ）。
fn word_right(text: &str, index: usize) -> usize {
    let mut characters = text[index..].char_indices().peekable();

    if let Some(&(_, first)) = characters.peek() {
        let class = char_class(first);
        if class != CharClass::Space {
            while characters.next_if(|(_, character)| char_class(*character) == class).is_some() {}
        }
    }
    while characters.next_if(|(_, character)| char_class(*character) == CharClass::Space).is_some() {}

    characters.peek().map_or(text.len(), |(position, _)| index + position)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(text: &str) -> TextEdit {
        let mut edit = TextEdit::new();
        edit.insert(text);
        edit
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut edit = edit("hello world");
        edit.select_word_at(8);
        assert_eq!(edit.selected_text(), Some("world"));

        edit.insert("rust");
        assert_eq!(edit.text(), "hello rust");
        assert_eq!(edit.selection(), None);
        assert_eq!(edit.cursor(), 10);
    }

    #[test]
    fn shift_moves_extend_the_selection_and_plain_moves_collapse_it() {
        let mut edit = edit("abcdef");
        edit.move_left(true, false);
        edit.move_left(true, false);
        assert_eq!(edit.selected_text(), Some("ef"));

        edit.move_left(false, false);
        assert_eq!((edit.cursor(), edit.selection()), (4, None), "左へ縮める");

        edit.move_home(true);
        assert_eq!(edit.selected_text(), Some("abcd"));
        edit.move_right(false, false);
        assert_eq!(edit.cursor(), 4, "右へ縮める");
    }

    #[test]
    fn words_are_skipped_like_windows() {
        let mut edit = edit("foo bar.baz  qux");

        edit.move_left(false, true);
        assert_eq!(edit.cursor(), 13, "qux の頭");
        edit.move_left(false, true);
        assert_eq!(edit.cursor(), 8, "baz の頭");
        edit.move_left(false, true);
        assert_eq!(edit.cursor(), 7, "記号は 1 まとまり");

        edit.move_home(false);
        edit.move_right(false, true);
        assert_eq!(edit.cursor(), 4, "bar の頭");
        edit.move_right(false, true);
        assert_eq!(edit.cursor(), 7);
    }

    #[test]
    fn deleting_by_word_and_by_character() {
        let mut edit = edit("日本語 テキスト");
        assert!(edit.delete_backward(true));
        assert_eq!(edit.text(), "日本語 ");
        assert!(edit.delete_backward(false));
        assert!(edit.delete_backward(false));
        assert_eq!(edit.text(), "日本", "1 字ずつ");

        edit.move_home(false);
        assert!(edit.delete_forward(false));
        assert_eq!(edit.text(), "本");
        assert!(!edit.delete_backward(false), "先頭では何も消えない");
    }

    #[test]
    fn select_all_then_delete_clears_everything() {
        let mut edit = edit("abc");
        edit.select_all();
        assert!(edit.delete_forward(false));
        assert_eq!(edit.text(), "");
    }

    #[test]
    fn undo_groups_continuous_typing() {
        let mut edit = TextEdit::new();
        edit.insert("a");
        edit.insert("b");
        edit.insert("c");
        edit.delete_backward(false);
        assert_eq!(edit.text(), "ab");

        assert!(edit.undo());
        assert_eq!(edit.text(), "abc", "消したのを戻す");
        assert!(edit.undo());
        assert_eq!(edit.text(), "", "続けて打ったのはまとめて戻る");
        assert!(!edit.undo());

        assert!(edit.redo());
        assert!(edit.redo());
        assert_eq!(edit.text(), "ab");
        assert!(!edit.redo());
    }

    #[test]
    fn moving_breaks_the_typing_group_and_new_edits_clear_redo() {
        let mut edit = TextEdit::new();
        edit.insert("ab");
        edit.move_left(false, false);
        edit.insert("X");
        assert_eq!(edit.text(), "aXb");

        edit.undo();
        assert_eq!(edit.text(), "ab", "動かした後の打ち込みは別の 1 回");

        edit.insert("Y");
        assert!(!edit.redo(), "書き換えたら、やり直しは消える");
    }

    #[test]
    fn paste_is_its_own_undo_step_and_drops_control_characters() {
        let mut edit = edit("a");
        edit.paste("b\nc\td");
        assert_eq!(edit.text(), "abcd");

        edit.undo();
        assert_eq!(edit.text(), "a");
    }

    #[test]
    fn max_length_counts_characters() {
        let mut edit = TextEdit::new();
        edit.set_max_length(Some(3));
        edit.insert("日本語です");
        assert_eq!(edit.text(), "日本語");
        assert!(!edit.insert("x"));

        // 選んで置き換えるなら、その分は空く。
        edit.select_word_at(0);
        edit.insert("ab");
        assert_eq!(edit.text(), "ab");
    }

    #[test]
    fn moves_snap_to_character_boundaries() {
        let mut edit = edit("日本");
        edit.move_to(1, false);
        assert_eq!(edit.cursor(), 0);
        edit.move_to(100, false);
        assert_eq!(edit.cursor(), "日本".len());
    }

    #[test]
    fn selecting_a_word_at_the_end_takes_the_last_word() {
        let mut edit = edit("one two");
        edit.select_word_at(7);
        assert_eq!(edit.selected_text(), Some("two"));

        edit.select_word_at(3);
        assert_eq!(edit.selected_text(), Some(" "), "空白の上なら空白");
    }
}
