//! 全図形の要素を 1 本につないで持つ置き場。2D と 3D で共通。
//!
//! # なぜここにあるか
//!
//! 図形ごとに別のバッファを持つと、図形が変わるたびにバインドや
//! 頂点バッファの差し替えが起きて、**ドローが図形の数だけ割れます**。
//! どちらのクレートも `multi_draw_indirect` 1 回に畳むのが前提なので、
//! 全部を 1 本につないで「どこから何個か」で引きます。
//!
//! この「つないで、範囲を覚える」という部分だけは、2D と 3D で
//! 完全に同じでした。使い道は違います。
//!
//! | | 何を積むか | 範囲の使い道 |
//! |---|---|---|
//! | 2D | 三角形の頂点 | シェーダが `shape_pool[base + 番号]` で引く |
//! | 3D | メッシュの頂点 | インダイレクト引数の `base_vertex` |
//! | 3D | メッシュの索引 | インダイレクト引数の `first_index` / `index_count` |
//!
//! 3D は 2 本使います。**1 本の [`Pool`] は 1 種類の要素だけを持ちます。**
//!
//! # 積むだけで、上げ方は決めない
//!
//! [`Pool::rebuild`] は CPU 側で並べるところまでで、GPU へ上げるのは
//! [`Pool::upload`] です。分けてあるので、**GPU 無しで並びを検証できます**。

use crate::buffer::{Allocation, BufferHeap};
use crate::error::Gueiz2DError;

/// 置き場の中での、図形 1 つぶんの持ち分。要素数で数える。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub struct PoolRange {
    pub base: u32,
    pub count: u32,
}

impl PoolRange {
    /// 最後の要素の次。
    pub fn end(&self) -> u32 {
        self.base + self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// 積む元。図形ごとの要素の並びを覗かせるだけ。
pub trait PoolSource {
    /// 積む要素。
    type Item: bytemuck::Pod;

    fn object_count(&self) -> usize;

    /// その図形が持つ要素。空でもよい（持ち分が 0 になる）。
    fn elements(&self, object: usize) -> &[Self::Item];
}

/// 全図形の要素をつないだ置き場。
pub struct Pool<T> {
    items: Vec<T>,
    ranges: Vec<PoolRange>,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            ranges: Vec::new(),
        }
    }
}

impl<T: bytemuck::Pod> Pool<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// その図形の持ち分。[`Pool::rebuild`] の後に見ること。
    pub fn range(&self, object: usize) -> PoolRange {
        self.ranges.get(object).copied().unwrap_or_default()
    }

    /// つないだ結果。
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// 積んである要素の数。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 積んである要素のバイト数。
    pub fn byte_len(&self) -> u64 {
        std::mem::size_of_val(self.items.as_slice()) as u64
    }

    /// 全図形の要素を先頭からつなぎ直し、それぞれの持ち分を覚える。
    ///
    /// 容量は使い回されるので、形が変わらない限り確保は起きません。
    pub fn rebuild(&mut self, source: &impl PoolSource<Item = T>) {
        let object_count = source.object_count();

        self.items.clear();
        self.ranges.clear();
        self.ranges.reserve(object_count);

        for object in 0..object_count {
            let elements = source.elements(object);
            let base = self.items.len() as u32;

            self.items.extend_from_slice(elements);

            self.ranges.push(PoolRange {
                base,
                count: elements.len() as u32,
            });
        }
    }

    /// 積んだ結果を、確保済みの区画の**先頭から**書く。
    ///
    /// 区画に入り切らなければ [`Gueiz2DError::HeapExhaustedError`] を返します。
    /// 黙って切り詰めると、図形が途中で欠けた状態で描かれてしまいます。
    pub fn upload(
        &self,
        queue: &wgpu::Queue,
        heap: &BufferHeap,
        allocation: &Allocation,
    ) -> Result<(), Gueiz2DError> {
        if self.items.is_empty() {
            return Ok(());
        }

        let needed = self.byte_len();

        if needed > allocation.size() {
            return Err(Gueiz2DError::HeapExhaustedError {
                requested: needed,
                largest_free_block: allocation.size(),
            });
        }

        heap.write(queue, allocation, bytemuck::cast_slice(&self.items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 図形ごとの要素を持つだけの、試験用の元。
    struct Fake(Vec<Vec<u32>>);

    impl PoolSource for Fake {
        type Item = u32;

        fn object_count(&self) -> usize {
            self.0.len()
        }

        fn elements(&self, object: usize) -> &[u32] {
            &self.0[object]
        }
    }

    #[test]
    fn elements_are_concatenated_in_order() {
        let mut pool = Pool::<u32>::new();
        pool.rebuild(&Fake(vec![vec![1, 2, 3], vec![4, 5], vec![6]]));

        assert_eq!(pool.items(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(pool.len(), 6);
    }

    #[test]
    fn ranges_point_at_each_objects_share() {
        let mut pool = Pool::<u32>::new();
        pool.rebuild(&Fake(vec![vec![1, 2, 3], vec![4, 5], vec![6]]));

        assert_eq!(pool.range(0), PoolRange { base: 0, count: 3 });
        assert_eq!(pool.range(1), PoolRange { base: 3, count: 2 });
        assert_eq!(pool.range(2), PoolRange { base: 5, count: 1 });

        // 範囲は要素と食い違ってはいけない。
        for object in 0..3 {
            let range = pool.range(object);
            let slice = &pool.items()[range.base as usize..range.end() as usize];
            assert_eq!(slice.len(), range.count as usize);
        }
    }

    /// 空の図形も場所は取らないが、番号はずれない。
    /// ずれると、後ろの図形が別の要素を読む。
    #[test]
    fn an_empty_object_keeps_the_numbering() {
        let mut pool = Pool::<u32>::new();
        pool.rebuild(&Fake(vec![vec![1, 2], vec![], vec![3]]));

        assert_eq!(pool.range(0), PoolRange { base: 0, count: 2 });
        assert_eq!(pool.range(1), PoolRange { base: 2, count: 0 });
        assert_eq!(pool.range(2), PoolRange { base: 2, count: 1 });
        assert!(pool.range(1).is_empty());
    }

    /// 積み直しても前回のぶんが残らない。残ると要素が二重になる。
    #[test]
    fn rebuilding_replaces_the_previous_contents() {
        let mut pool = Pool::<u32>::new();
        pool.rebuild(&Fake(vec![vec![1, 2, 3], vec![4, 5]]));
        pool.rebuild(&Fake(vec![vec![9]]));

        assert_eq!(pool.items(), &[9]);
        assert_eq!(pool.range(0), PoolRange { base: 0, count: 1 });
        // 消えた図形の範囲は残らない。
        assert_eq!(pool.range(1), PoolRange::default());
    }

    #[test]
    fn an_empty_source_leaves_an_empty_pool() {
        let mut pool = Pool::<u32>::new();
        pool.rebuild(&Fake(vec![]));

        assert!(pool.is_empty());
        assert_eq!(pool.byte_len(), 0);
        assert_eq!(pool.range(0), PoolRange::default());
    }

    #[test]
    fn the_byte_length_follows_the_element_size() {
        let mut pool = Pool::<u32>::new();
        pool.rebuild(&Fake(vec![vec![1, 2, 3]]));

        assert_eq!(pool.byte_len(), 12);
    }
}
