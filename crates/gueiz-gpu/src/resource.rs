//! 取っ手つきの置き場。2D と 3D で共通。
//!
//! # なぜ取っ手を配るのか
//!
//! リソース（フォント、絵、メッシュ）は大きいので複製したくありません。
//! かといって参照を配ると、**持ち主より長生きできない**ので、
//! 置き場と使う側の寿命が絡み合います。
//!
//! そこで [`Store`] に預けて、小さな [`Handle`] だけを配ります。
//! 取っ手は `Copy` で 8 バイト、寿命も持ちません。好きなだけ複製できます。
//!
//! # 消した後の取っ手
//!
//! 取っ手には**世代**が入っています。消した枠は作り直しに使い回されますが、
//! そのとき世代が 1 つ進むので、**古い取っ手は新しい中身を掴みません**。
//!
//! これが無いと、消したフォントの取っ手が、たまたま同じ枠に入った
//! 別のフォントを指してしまいます。気づきにくい壊れ方をするので、
//! 世代を持つ価値があります。
//!
//! ```
//! # use gueiz_gpu::resource::Store;
//! let mut store = Store::<String>::new();
//!
//! let old = store.insert(String::from("古い"));
//! store.remove(old);
//! let new = store.insert(String::from("新しい"));
//!
//! // 枠は使い回されたが、古い取っ手では引けない。
//! assert_eq!(store.get(old), None);
//! assert_eq!(store.get(new).map(String::as_str), Some("新しい"));
//! ```

use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// 預けたものを引くための取っ手。
///
/// `Copy` で寿命を持たないので、どこにでも置けます。
/// 型が違う取っ手は混ざりません（`Handle<Font>` を `Store<Mesh>` に渡せない）。
pub struct Handle<T> {
    index: u32,
    generation: u32,
    /// 型を混ぜないための印。`fn() -> T` にしておくと、`T` が何であれ
    /// 取っ手そのものは `Send` にも `Sync` にもなる。
    marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    fn new(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            marker: PhantomData,
        }
    }

    /// 置き場の中の何番目か。並べ替えや索引に使う。
    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }
}

// `PhantomData<T>` があると自動導出が `T` に条件を付けてしまうので、手で書く。
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> Debug for Handle<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Handle({}, gen {})", self.index, self.generation))
    }
}

/// 1 枠ぶん。
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// 同じ種類のものをまとめて預かる置き場。
///
/// 名前を付けて預ければ、名前で引き直せます。**同じ名前で預け直すと
/// 前のものは消え**、古い取っ手は使えなくなります（読み込み直しに使えます）。
pub struct Store<T> {
    slots: Vec<Slot<T>>,
    /// 空いた枠。使い回して、番号が無闇に増えないようにする。
    free: Vec<u32>,
    names: HashMap<String, Handle<T>>,
}

impl<T> Default for Store<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            names: HashMap::new(),
        }
    }
}

impl<T> Store<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// 名前なしで預ける。
    pub fn insert(&mut self, value: T) -> Handle<T> {
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.value = Some(value);

                Handle::new(index, slot.generation)
            }

            None => {
                let index = self.slots.len() as u32;
                self.slots.push(Slot {
                    generation: 0,
                    value: Some(value),
                });

                Handle::new(index, 0)
            }
        }
    }

    /// 名前を付けて預ける。**同じ名前が居たら入れ替える。**
    ///
    /// 入れ替えられた古い取っ手は使えなくなります。
    pub fn insert_named(&mut self, name: &str, value: T) -> Handle<T> {
        if let Some(previous) = self.names.remove(name) {
            self.remove(previous);
        }

        let handle = self.insert(value);
        self.names.insert(String::from(name), handle);

        handle
    }

    /// 名前から取っ手を引く。
    pub fn handle(&self, name: &str) -> Option<Handle<T>> {
        self.names.get(name).copied()
    }

    /// 取っ手が指すもの。消されていたり世代が古ければ `None`。
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        let slot = self.slots.get(handle.index as usize)?;

        if slot.generation != handle.generation {
            return None;
        }

        slot.value.as_ref()
    }

    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let slot = self.slots.get_mut(handle.index as usize)?;

        if slot.generation != handle.generation {
            return None;
        }

        slot.value.as_mut()
    }

    /// 名前で引く。
    pub fn get_named(&self, name: &str) -> Option<&T> {
        self.get(self.handle(name)?)
    }

    /// 取り出して消す。枠は使い回され、世代が 1 つ進む。
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let slot = self.slots.get_mut(handle.index as usize)?;

        if slot.generation != handle.generation {
            return None;
        }

        let value = slot.value.take()?;

        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(handle.index);
        self.names.retain(|_, named| *named != handle);

        Some(value)
    }

    /// 預かっている数。
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.value.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 全部を取っ手つきで。順番は預けた順とは限らない。
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let value = slot.value.as_ref()?;
                Some((Handle::new(index as u32, slot.generation), value))
            })
    }

    /// 付いている名前。
    pub fn names(&self) -> impl Iterator<Item = (&str, Handle<T>)> {
        self.names.iter().map(|(name, handle)| (name.as_str(), *handle))
    }

    /// 全部消す。
    pub fn clear(&mut self) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.value.take().is_some() {
                slot.generation = slot.generation.wrapping_add(1);
                self.free.push(index as u32);
            }
        }

        self.names.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_goes_in_comes_out() {
        let mut store = Store::<u32>::new();

        let a = store.insert(1);
        let b = store.insert(2);

        assert_eq!(store.get(a), Some(&1));
        assert_eq!(store.get(b), Some(&2));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn a_removed_handle_stops_working() {
        let mut store = Store::<u32>::new();
        let handle = store.insert(1);

        assert_eq!(store.remove(handle), Some(1));
        assert_eq!(store.get(handle), None);
        assert!(store.is_empty());
    }

    /// 消した枠は使い回されるが、古い取っ手が新しい中身を掴んではいけない。
    /// ここが世代の要点。
    #[test]
    fn a_reused_slot_does_not_answer_to_the_old_handle() {
        let mut store = Store::<u32>::new();

        let old = store.insert(1);
        store.remove(old);
        let new = store.insert(2);

        assert_eq!(old.index(), new.index(), "枠は使い回されている");
        assert_ne!(old.generation(), new.generation(), "世代は進んでいる");

        assert_eq!(store.get(old), None);
        assert_eq!(store.get(new), Some(&2));
    }

    #[test]
    fn names_find_their_handles() {
        let mut store = Store::<u32>::new();
        let handle = store.insert_named("arial", 7);

        assert_eq!(store.handle("arial"), Some(handle));
        assert_eq!(store.get_named("arial"), Some(&7));
        assert_eq!(store.handle("meiryo"), None);
    }

    /// 同じ名前で預け直したら入れ替わる。読み込み直しがこれで済む。
    #[test]
    fn the_same_name_replaces_what_was_there() {
        let mut store = Store::<u32>::new();

        let old = store.insert_named("font", 1);
        let new = store.insert_named("font", 2);

        assert_eq!(store.len(), 1, "古いほうは消えている");
        assert_eq!(store.get(old), None, "古い取っ手はもう使えない");
        assert_eq!(store.get_named("font"), Some(&2));
        assert_eq!(store.handle("font"), Some(new));
    }

    /// 消したら名前も消える。名前だけ残ると、無いものを指し続ける。
    #[test]
    fn removing_also_drops_the_name() {
        let mut store = Store::<u32>::new();
        let handle = store.insert_named("font", 1);

        store.remove(handle);

        assert_eq!(store.handle("font"), None);
        assert_eq!(store.get_named("font"), None);
    }

    #[test]
    fn values_can_be_changed_in_place() {
        let mut store = Store::<u32>::new();
        let handle = store.insert(1);

        *store.get_mut(handle).expect("居る") = 5;

        assert_eq!(store.get(handle), Some(&5));
    }

    #[test]
    fn iterating_visits_everything_that_is_still_there() {
        let mut store = Store::<u32>::new();
        let a = store.insert(1);
        let b = store.insert(2);
        let c = store.insert(3);

        store.remove(b);

        let mut found: Vec<u32> = store.iter().map(|(_, value)| *value).collect();
        found.sort_unstable();

        assert_eq!(found, vec![1, 3]);
        assert!(store.get(a).is_some() && store.get(c).is_some());
    }

    #[test]
    fn clearing_empties_everything() {
        let mut store = Store::<u32>::new();
        let handle = store.insert_named("a", 1);
        store.insert_named("b", 2);

        store.clear();

        assert!(store.is_empty());
        assert_eq!(store.get(handle), None);
        assert_eq!(store.handle("a"), None);
        assert_eq!(store.iter().count(), 0);
    }

    /// 取っ手は小さくて、複製が自由で、寿命を持たない。
    #[test]
    fn a_handle_is_small_and_copyable() {
        assert_eq!(size_of::<Handle<String>>(), 8);

        let mut store = Store::<u32>::new();
        let handle = store.insert(1);
        let copy = handle;

        assert_eq!(handle, copy);
        assert_eq!(store.get(copy), Some(&1));
    }

    /// 別の置き場の取っ手を混ぜても、型で弾かれる。
    /// （これは書けないことの確認なので、同じ型どうしの比較だけ見る。）
    #[test]
    fn handles_from_different_slots_differ() {
        let mut store = Store::<u32>::new();

        assert_ne!(store.insert(1), store.insert(2));
    }
}
