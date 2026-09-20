//! 複製をまとめて GPU に送る仕組み。2D と 3D で共通。
//!
//! # なぜここにあるか
//!
//! 2D と 3D の `DrawManager` を**別々に素直に書いた結果**、複製の送り方だけは
//! アルゴリズムが 1 行も変わりませんでした。カリング（円 vs 錐台）も
//! 奥行き（並べ替え vs 深度）も頂点の渡し方も違いましたが、ここは同じでした。
//! 2 度書いて同じだと確かめてから引き上げています。
//!
//! # 何をするか
//!
//! 複製は数が多いので、毎フレーム全部を組み直して上げるとそこがいちばん
//! 重くなります（100 万個で 13 ms 前後）。そこで
//!
//! - 図形は書き換えられたときだけ印を立て、
//! - こちらは各図形の持ち分（[`InstanceRange`]）を覚えておき、
//! - **並びが変わらないフレームでは印の立った図形の範囲だけ**送り直します。
//!
//! 図形の増減や複製の数の変化で並びが崩れたときだけ、全部を組み直します。
//! 動かさない図形は、登録した最初のフレーム以降いっさい CPU 時間を食いません。
//!
//! # 使い方
//!
//! 送り元は [`InstanceSource`] を実装した小さな覗き窓を渡します。
//! `DrawManager` 自身に実装すると `&mut self` が二重になるので、
//! 図形の列だけを借りる薄い型を作るのが素直です。

/// 複製入力バッファ内での、図形 1 つぶんの持ち分。要素数で数える。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub struct InstanceRange {
    pub base: u32,
    pub count: u32,
}

/// 複製の送り元。
///
/// [`InstanceUploader`] はここに 5 つだけ問い合わせます。
/// どの図形が何個持っているか、書き換えられたか、素データはどう作るか。
pub trait InstanceSource {
    /// GPU に載せる 1 つぶんの形。
    type Raw: bytemuck::Pod;

    /// 図形の数。
    fn object_count(&self) -> usize;

    /// その図形が持つ複製の数。
    ///
    /// **0 を返しても 1 つぶんの場所が取られます。** 複製を 1 つも足していない
    /// 図形は「変換なしの 1 つ」として描かれるという決まりで、これは 2D と 3D で
    /// 同じです。そのとき [`InstanceSource::write`] には `count = 1` が渡るので、
    /// 既定の複製を 1 つ積んでください。
    fn instance_count(&self, object: usize) -> usize;

    /// その図形の複製が書き換えられたか。
    fn is_dirty(&self, object: usize) -> bool;

    /// その図形の複製を先頭から `count` 個、素データに直して `out` に積む。
    ///
    /// `out` は呼ぶ側が空にしてあります。ちょうど `count` 個積んでください。
    fn write(&self, object: usize, count: usize, out: &mut Vec<Self::Raw>);

    /// 送り終えたので、全図形の印を落とす。
    fn clear_dirty(&mut self);
}

/// 複製の持ち分を覚えて、変わったぶんだけ送る。
pub struct InstanceUploader<R> {
    ranges: Vec<InstanceRange>,
    scratch: Vec<R>,
    /// 並びを組み直す必要がある。図形が増減したら立てる。
    layout_dirty: bool,
    max_instances: u32,
    max_instances_per_object: u32,
}

impl<R: bytemuck::Pod> InstanceUploader<R> {
    /// 全図形を合わせた複製の上限を決めて作る。
    pub fn new(max_instances: u32) -> Self {
        Self {
            ranges: Vec::new(),
            scratch: Vec::new(),
            layout_dirty: true,
            max_instances,
            max_instances_per_object: 0,
        }
    }

    /// 図形が増減したことを伝える。次のフレームで並びを組み直す。
    pub fn invalidate_layout(&mut self) {
        self.layout_dirty = true;
    }

    pub fn max_instances(&self) -> u32 {
        self.max_instances
    }

    /// その図形の持ち分。[`InstanceUploader::upload`] の後に見ること。
    ///
    /// インダイレクト引数の `first_instance` と、図形の記述に入れる
    /// `instance_base` / `instance_count` はここから取る。
    pub fn range(&self, object: usize) -> InstanceRange {
        self.ranges.get(object).copied().unwrap_or_default()
    }

    /// 1 図形あたりの複製数の最大値。コンピュートのディスパッチ幅に使う。
    pub fn max_instances_per_object(&self) -> u32 {
        self.max_instances_per_object
    }

    /// 各図形の持ち分を数え直す。前フレームから変わったら `true`。
    ///
    /// 数えるだけで複製そのものには触れないので、図形の数にしか比例しません。
    /// [`InstanceUploader::upload`] が先頭で呼ぶので、普通は直接呼びません。
    pub fn plan(&mut self, source: &impl InstanceSource<Raw = R>) -> bool {
        let object_count = source.object_count();
        let mut changed = self.layout_dirty || self.ranges.len() != object_count;

        self.ranges.resize(object_count, InstanceRange::default());
        self.max_instances_per_object = 0;

        let mut base = 0_u32;

        for object in 0..object_count {
            // 複製が 1 つも無ければ、変換なしの 1 つがあるものとして場所を取る。
            let wanted = source.instance_count(object).max(1) as u32;
            let count = wanted.min(self.max_instances.saturating_sub(base));
            let range = InstanceRange { base, count };

            if self.ranges[object] != range {
                self.ranges[object] = range;
                changed = true;
            }

            base += count;
            self.max_instances_per_object = self.max_instances_per_object.max(count);
        }

        self.layout_dirty = false;
        changed
    }

    /// 複製の素データを上げる。
    ///
    /// 並びが変わっていなければ、印の立った図形の範囲だけを書き換えます。
    /// 動かない図形は組み直しも転送も起きません。
    pub fn upload(
        &mut self,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        offset: u64,
        source: &mut impl InstanceSource<Raw = R>,
    ) {
        let layout_changed = self.plan(source);
        let object_count = source.object_count();
        let stride = size_of::<R>() as u64;

        if layout_changed {
            // 並びが崩れたので全部組み直す。1 回の転送で上げる。
            self.scratch.clear();

            for object in 0..object_count {
                let count = self.ranges[object].count as usize;
                source.write(object, count, &mut self.scratch);
            }

            if !self.scratch.is_empty() {
                queue.write_buffer(buffer, offset, bytemuck::cast_slice(&self.scratch));
            }
        } else {
            for object in 0..object_count {
                if !source.is_dirty(object) {
                    continue;
                }

                let range = self.ranges[object];
                if range.count == 0 {
                    continue;
                }

                // 1 図形ぶんの置き場として使い回す。容量は縮まないので確保は起きない。
                self.scratch.clear();
                source.write(object, range.count as usize, &mut self.scratch);

                queue.write_buffer(
                    buffer,
                    offset + range.base as u64 * stride,
                    bytemuck::cast_slice(&self.scratch),
                );
            }
        }

        source.clear_dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 数と印だけを持つ、試験用の送り元。
    struct Fake {
        counts: Vec<usize>,
        dirty: Vec<bool>,
        /// `write` が呼ばれた図形と個数。
        written: std::cell::RefCell<Vec<(usize, usize)>>,
    }

    impl Fake {
        fn new(counts: &[usize]) -> Self {
            Self {
                counts: counts.to_vec(),
                dirty: vec![true; counts.len()],
                written: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl InstanceSource for Fake {
        type Raw = u32;

        fn object_count(&self) -> usize {
            self.counts.len()
        }

        fn instance_count(&self, object: usize) -> usize {
            self.counts[object]
        }

        fn is_dirty(&self, object: usize) -> bool {
            self.dirty[object]
        }

        fn write(&self, object: usize, count: usize, out: &mut Vec<u32>) {
            self.written.borrow_mut().push((object, count));
            out.extend(std::iter::repeat_n(object as u32, count));
        }

        fn clear_dirty(&mut self) {
            self.dirty.fill(false);
        }
    }

    #[test]
    fn ranges_are_packed_in_order() {
        let mut uploader = InstanceUploader::<u32>::new(1024);
        let source = Fake::new(&[3, 5, 2]);

        assert!(uploader.plan(&source));

        assert_eq!(uploader.range(0), InstanceRange { base: 0, count: 3 });
        assert_eq!(uploader.range(1), InstanceRange { base: 3, count: 5 });
        assert_eq!(uploader.range(2), InstanceRange { base: 8, count: 2 });
        assert_eq!(uploader.max_instances_per_object(), 5);
    }

    /// 複製が 0 の図形も 1 つぶんの場所を取る。
    /// 取らないと、変換なしの 1 つが描けない。
    #[test]
    fn an_empty_object_still_gets_one_slot() {
        let mut uploader = InstanceUploader::<u32>::new(1024);
        let source = Fake::new(&[0, 4]);

        uploader.plan(&source);

        assert_eq!(uploader.range(0), InstanceRange { base: 0, count: 1 });
        assert_eq!(uploader.range(1), InstanceRange { base: 1, count: 4 });
    }

    /// 2 度目は変わっていないと答える。ここが毎フレームの転送を省く鍵。
    #[test]
    fn an_unchanged_layout_is_reported_as_unchanged() {
        let mut uploader = InstanceUploader::<u32>::new(1024);
        let source = Fake::new(&[3, 5]);

        assert!(uploader.plan(&source), "初回は組み直しが要る");
        assert!(!uploader.plan(&source), "2 度目は変わっていない");
    }

    #[test]
    fn changing_a_count_changes_the_layout() {
        let mut uploader = InstanceUploader::<u32>::new(1024);
        let mut source = Fake::new(&[3, 5]);
        uploader.plan(&source);

        source.counts[0] = 4;

        assert!(uploader.plan(&source), "数が変われば並びも変わる");
        assert_eq!(uploader.range(1), InstanceRange { base: 4, count: 5 });
    }

    #[test]
    fn adding_an_object_changes_the_layout() {
        let mut uploader = InstanceUploader::<u32>::new(1024);
        let mut source = Fake::new(&[3]);
        uploader.plan(&source);

        source.counts.push(2);
        source.dirty.push(true);

        assert!(uploader.plan(&source));
        assert_eq!(uploader.range(1), InstanceRange { base: 3, count: 2 });
    }

    /// 明示的に崩したと伝えたら、数が同じでも組み直す。
    #[test]
    fn invalidating_forces_a_rebuild() {
        let mut uploader = InstanceUploader::<u32>::new(1024);
        let source = Fake::new(&[3]);

        uploader.plan(&source);
        assert!(!uploader.plan(&source));

        uploader.invalidate_layout();
        assert!(uploader.plan(&source));
    }

    /// 上限を超えたぶんは切り詰める。溢れて別の図形の領域を踏むより、
    /// 出ないほうがまし。
    #[test]
    fn the_limit_truncates_instead_of_overflowing() {
        let mut uploader = InstanceUploader::<u32>::new(6);
        let source = Fake::new(&[4, 4, 4]);

        uploader.plan(&source);

        assert_eq!(uploader.range(0), InstanceRange { base: 0, count: 4 });
        assert_eq!(uploader.range(1), InstanceRange { base: 4, count: 2 });
        assert_eq!(uploader.range(2), InstanceRange { base: 6, count: 0 });
    }

    #[test]
    fn a_missing_object_reports_an_empty_range() {
        let uploader = InstanceUploader::<u32>::new(16);

        assert_eq!(uploader.range(99), InstanceRange::default());
    }
}
