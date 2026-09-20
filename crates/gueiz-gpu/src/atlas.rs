//! 大きさの違う絵を 1 本の配列テクスチャに詰め込む。
//!
//! # なぜ詰め込むのか
//!
//! [`crate::sprite::SpriteSheet`] は配列テクスチャなので、**層は全部同じ大きさ**
//! でなければなりません。16x16 のアイコンと 512x512 の背景を素直に並べると、
//! アイコン側が 512x512 ぶん場所を食います。
//!
//! そこで層を「**ページ**」として使い、その中に絵を敷き詰めます。
//! どの絵かは「ページ番号 + 切り出し範囲」で指すので、
//! 図形側の仕組み（[`crate::sprite::SpriteSheet::region`] と
//! `Object::sprite_region`）がそのまま使えます。
//!
//! ```text
//! ページ 0 (2048x2048)          ページ 1
//! ┌──────┬────┬──┐              ┌────────────┐
//! │ 背景 │ 木 │草│              │  人物      │
//! │      ├────┴──┤              │            │
//! ├──────┴───────┤              └────────────┘
//! │ 地面         │
//! └──────────────┘
//! ```
//!
//! # 一度に入れる
//!
//! 詰め込みは**全部見てから**のほうが密になります。[`Atlas::insert_batch`] は
//! 高い順に並べ替えてから詰めるので、1 枚ずつ [`Atlas::insert`] するより
//! ページ数が減ります。
//!
//! # 増えたとき
//!
//! ページが足りなくなったら、層の多いテクスチャを作り直して
//! **GPU 上で古いページを写します**（`copy_texture_to_texture`）。
//! 画素を CPU 側に持ち続ける必要はありません。
//!
//! # 実行中に出し入れする
//!
//! [`Atlas::remove`] は棚の中に**空き区間**を返します。区間は隣とつながり、
//! 棚の右端まで空けば端が戻り、いちばん上の棚が空になれば棚ごと降ります。
//! 次に入れる絵は**空いた穴から先に**埋めるので、出し入れを繰り返しても
//! ページは伸びません。
//!
//! 消しても**画素は消しません。** 誰も読まないので、消すためだけに
//! 書き込むのは無駄です。
//!
//! 代わりに**詰め直しはしません。** 穴に入らない大きさばかりが来ると
//! 隙間が残るので、そのときは [`Atlas::clear`] からやり直してください。
//! 詰まり具合は [`Atlas::occupancy`] で見られます。

use crate::error::Gueiz2DError;
use crate::sprite::{SpriteFilter, SpriteSheet};

/// 1 画素あたりのバイト数。RGBA8 固定。
const BYTES_PER_PIXEL: usize = 4;

/// ページ 1 辺の既定。`wgpu` の既定上限は 8192。
pub const DEFAULT_PAGE_SIZE: u32 = 2048;

/// 絵と絵のあいだに空ける既定の余白（画素）。
///
/// 0 にすると、線形補間が隣の絵を舐めて縁に別の色が出ます。
pub const DEFAULT_PADDING: u32 = 1;

/// 縮小用の段の数。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub enum MipLevels {
    /// 作らない。拡大しか使わないならこれで足ります。
    #[default]
    None,
    /// ページが 1x1 になるまで作る。
    Full,
    /// 段数を決める。1 は「作らない」と同じ。
    Count(u32),
}

impl MipLevels {
    /// ページの 1 辺から、実際に作る段数を出す。
    fn resolve(self, page_size: u32) -> u32 {
        let full = page_size.ilog2() + 1;

        match self {
            Self::None => 1,
            Self::Full => full,
            Self::Count(count) => count.clamp(1, full),
        }
    }
}

/// アトラスの作り方。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct AtlasDescriptor {
    /// ページ 1 辺の画素数。2 の冪にすること。
    pub page_size: u32,
    /// 絵と絵のあいだに空ける画素数。
    ///
    /// **縮小用の段を作るなら、ここは自動で `2^(段数-1)` まで引き上げられます。**
    /// 段を下るほど隣の絵が近づくので、余白が足りないと混ざります。
    pub padding: u32,
    pub filter: SpriteFilter,
    pub mip_levels: MipLevels,
    /// ページの上限。`wgpu` の既定上限は 256。
    pub max_pages: u32,
}

impl Default for AtlasDescriptor {
    fn default() -> Self {
        Self {
            page_size: DEFAULT_PAGE_SIZE,
            padding: DEFAULT_PADDING,
            filter: SpriteFilter::default(),
            mip_levels: MipLevels::default(),
            max_pages: 256,
        }
    }
}

/// 詰め込んだ絵 1 枚の場所。画素単位。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub struct Placement {
    pub page: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// 図形に貼るときの指定。
///
/// ```no_run
/// # use gueiz_gpu::atlas::TextureRegion;
/// # fn run(region: TextureRegion) {
/// // object.sprite_region(region.page, region.uv_rect);
/// # let _ = region;
/// # }
/// ```
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct TextureRegion {
    /// 何ページ目か。配列テクスチャの層。
    pub page: u32,
    /// 切り出し範囲（`[u, v, 幅, 高さ]`、いずれも 0..1）。
    pub uv_rect: [f32; 4],
    /// 元の絵の大きさ（画素）。
    pub size: [u32; 2],
}

// --- 詰め込み ---

/// 棚に順に載せていく詰め込み。
///
/// # なぜ棚なのか
///
/// 高い順に入れれば、棚の高さが揃って隙間が少なくなります。
/// スカイラインのほうが密になりますが、実装も状態も重く、
/// **絵を後から足せる**という性質が要るのでこちらにしています。
///
/// 棚は「上端・高さ・どこまで使ったか」の 3 つだけで表せるので、
/// 足すのは棚を端から見るだけで済みます。
#[derive(Clone)]
#[derive(Debug)]
pub struct ShelfPacker {
    page_size: u32,
    padding: u32,
    /// 置き場所を `alignment` の倍数に揃える。縮小用の段のために要る。
    alignment: u32,
    max_pages: u32,
    pages: Vec<Page>,
}

#[derive(Clone)]
#[derive(Debug, Default)]
struct Page {
    shelves: Vec<Shelf>,
    /// 次の棚を載せられる高さ。
    used_y: u32,
}

#[derive(Clone)]
#[derive(Debug)]
struct Shelf {
    top: u32,
    height: u32,
    /// ここから右はまだ誰も使っていない。
    used_x: u32,
    /// 返ってきた区間。x の小さい順。隣り合ったらつなぐ。
    free: Vec<Span>,
}

/// 棚の中の、空いている横幅。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
struct Span {
    x: u32,
    width: u32,
}

impl Span {
    fn end(self) -> u32 {
        self.x + self.width
    }
}

impl ShelfPacker {
    pub fn new(page_size: u32, padding: u32, alignment: u32, max_pages: u32) -> Self {
        Self {
            page_size,
            padding,
            alignment: alignment.max(1),
            max_pages: max_pages.max(1),
            pages: Vec::new(),
        }
    }

    /// 1 枚ぶんの場所を取る。入らなければ `None`。
    pub fn insert(&mut self, width: u32, height: u32) -> Option<Placement> {
        // 余白は絵の右と下に付ける。左と上は隣が付けた余白で足りる。
        let needed_width = self.round_up(width + self.padding * 2);
        let needed_height = self.round_up(height + self.padding * 2);

        if needed_width > self.page_size || needed_height > self.page_size {
            return None;
        }

        for (index, page) in self.pages.iter_mut().enumerate() {
            if let Some((x, y)) = page.place(
                needed_width,
                needed_height,
                self.page_size,
            ) {
                return Some(Placement {
                    page: index as u32,
                    x: x + self.padding,
                    y: y + self.padding,
                    width,
                    height,
                });
            }
        }

        if self.pages.len() as u32 >= self.max_pages {
            return None;
        }

        self.pages.push(Page::default());
        let index = self.pages.len() - 1;
        let (x, y) = self.pages[index]
            .place(needed_width, needed_height, self.page_size)
            .expect("新しいページには必ず入る");

        Some(Placement {
            page: index as u32,
            x: x + self.padding,
            y: y + self.padding,
            width,
            height,
        })
    }

    /// 取った場所を返す。**同じ [`Placement`] を 2 度返さないこと。**
    ///
    /// 返った区間は隣の空きとつながり、棚の右端まで空けば端が戻り、
    /// いちばん上の棚が空になれば棚ごと降ります。次に入れる絵は
    /// **空いた穴から先に**埋めるので、出し入れを繰り返してもページは伸びません。
    ///
    /// 知らない場所を渡されたら `false`。
    pub fn remove(&mut self, placement: Placement) -> bool {
        // 取ったときと同じ計算で、余白を含んだ元の大きさに戻す。
        // ここがずれると、返した区間が隣に食い込む。
        let reserved_width = self.round_up(placement.width + self.padding * 2);
        let padding = self.padding;

        let Some(page) = self.pages.get_mut(placement.page as usize) else {
            return false;
        };

        let freed = page.free(
            placement.x - padding,
            placement.y - padding,
            reserved_width,
        );

        // 末尾のページが空になったら落とす。途中のページは番号が
        // ずれてしまうので、空でも残す。
        while self.pages.last().is_some_and(Page::is_empty) {
            self.pages.pop();
        }

        freed
    }

    /// 置き場所の刻みに合わせて切り上げる。
    fn round_up(&self, value: u32) -> u32 {
        value.div_ceil(self.alignment) * self.alignment
    }

    pub fn page_count(&self) -> u32 {
        self.pages.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    pub fn clear(&mut self) {
        self.pages.clear();
    }

    /// 使っている面積の割合。0..1。詰まり具合を見るのに。
    pub fn occupancy(&self, placed: &[Placement]) -> f32 {
        if self.pages.is_empty() {
            return 0.0;
        }

        let used: u64 = placed
            .iter()
            .map(|placement| u64::from(placement.width) * u64::from(placement.height))
            .sum();
        let total = u64::from(self.page_size) * u64::from(self.page_size)
            * u64::from(self.page_count());

        used as f32 / total as f32
    }
}

impl Page {
    fn place(&mut self, width: u32, height: u32, page_size: u32) -> Option<(u32, u32)> {
        // まず返ってきた区間を当たる。棚の右端を伸ばす前に、
        // 空いた穴を埋めないと、出し入れのたびにページが伸びる。
        if let Some(position) = self.place_in_free_space(width, height) {
            return Some(position);
        }

        // 高さが合う棚の右端に足す。高すぎる棚に低い絵を置くと上が無駄になるので、
        // いちばん無駄の少ない棚を選ぶ。
        let mut best: Option<(usize, u32)> = None;

        for (index, shelf) in self.shelves.iter().enumerate() {
            if shelf.height < height || shelf.used_x + width > page_size {
                continue;
            }

            let waste = shelf.height - height;

            if best.is_none_or(|(_, best_waste)| waste < best_waste) {
                best = Some((index, waste));
            }
        }

        if let Some((index, _)) = best {
            let shelf = &mut self.shelves[index];
            let x = shelf.used_x;
            shelf.used_x += width;

            return Some((x, shelf.top));
        }

        // 新しい棚を載せる。
        if self.used_y + height > page_size {
            return None;
        }

        let top = self.used_y;
        self.used_y += height;
        self.shelves.push(Shelf {
            top,
            height,
            used_x: width,
            free: Vec::new(),
        });

        Some((0, top))
    }

    /// 返ってきた区間に入れる。**いちばん惜しくないところ**を選ぶ。
    ///
    /// 大きい穴に小さい絵を入れると、残りが半端になって次が入らなくなります。
    /// 棚の高さと区間の幅、どちらも余りの少ないほうから埋めます。
    fn place_in_free_space(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let mut best: Option<(usize, usize, u32)> = None;

        for (shelf_index, shelf) in self.shelves.iter().enumerate() {
            if shelf.height < height {
                continue;
            }

            let height_waste = shelf.height - height;

            for (span_index, span) in shelf.free.iter().enumerate() {
                if span.width < width {
                    continue;
                }

                let waste = height_waste + (span.width - width);

                if best.is_none_or(|(_, _, best_waste)| waste < best_waste) {
                    best = Some((shelf_index, span_index, waste));
                }
            }
        }

        let (shelf_index, span_index, _) = best?;
        let shelf = &mut self.shelves[shelf_index];
        let span = shelf.free[span_index];
        let x = span.x;

        if span.width == width {
            shelf.free.remove(span_index);
        } else {
            shelf.free[span_index] = Span {
                x: span.x + width,
                width: span.width - width,
            };
        }

        Some((x, shelf.top))
    }

    /// 場所を返す。隣の空きとつなぎ、棚の端まで空いたら端を縮める。
    fn free(&mut self, x: u32, y: u32, width: u32) -> bool {
        let Some(shelf) = self.shelves.iter_mut().find(|shelf| shelf.top == y) else {
            return false;
        };

        if x + width > shelf.used_x {
            return false;
        }

        // x 順のまま入れる。つなげるかどうかは隣としか比べない。
        let at = shelf
            .free
            .iter()
            .position(|span| span.x > x)
            .unwrap_or(shelf.free.len());
        shelf.free.insert(at, Span { x, width });

        // 右とつなぐ。先に右を見るのは、左とつないだあと番号がずれるため。
        if at + 1 < shelf.free.len() && shelf.free[at].end() == shelf.free[at + 1].x {
            shelf.free[at].width += shelf.free[at + 1].width;
            shelf.free.remove(at + 1);
        }

        if at > 0 && shelf.free[at - 1].end() == shelf.free[at].x {
            shelf.free[at - 1].width += shelf.free[at].width;
            shelf.free.remove(at);
        }

        // 右端まで空いたら、棚の使用済みを戻す。区間として持ち続けると、
        // 棚が空でも「使っている」ように見える。
        if let Some(last) = shelf.free.last().copied()
            && last.end() == shelf.used_x
        {
            shelf.used_x = last.x;
            shelf.free.pop();
        }

        // いちばん上の棚が空になったら、棚ごと降ろす。
        while self
            .shelves
            .last()
            .is_some_and(|shelf| shelf.used_x == 0 && shelf.free.is_empty())
        {
            let shelf = self.shelves.pop().expect("いま見た");
            self.used_y = shelf.top;
        }

        true
    }

    /// この棚に何か載っているか。
    fn is_empty(&self) -> bool {
        self.shelves.iter().all(|shelf| shelf.used_x == 0)
    }
}

// --- 画素の下ごしらえ ---

/// 透明なところに、隣の色をにじませる。
///
/// # なぜ要るか
///
/// このクレートのシェーダは**普通のアルファのまま**絵を読み、
/// 出す直前にだけ乗算済みへ直します。ところが**ハードウェアの線形補間は
/// 乗算済みでないと正しくありません。**
///
/// 赤の不透明画素 `(255,0,0,255)` と透明黒 `(0,0,0,0)` が隣り合うと、
/// 中間は `(127,0,0,127)` になり、出す段で `rgb * a` して `(64,0,0)`。
/// **本来の半分の明るさ**で、縁が黒ずみます。
///
/// 透明側の RGB を赤にしておけば、中間は `(255,0,0,127)` → `(127,0,0)` で
/// 正しくなります。**アルファは触りません。**見た目は変わらず、
/// 補間に使われる色だけが直ります。
///
/// 近いほうから順に塗るので、幅は元の絵ぜんぶに届きます。
///
/// ```
/// # use gueiz_gpu::atlas::bleed_transparent;
/// // 左が赤、右が透明。
/// let mut pixels = vec![255, 0, 0, 255, 0, 0, 0, 0];
/// bleed_transparent(&mut pixels, 2, 1);
///
/// // 右の RGB が赤に。アルファは 0 のまま。
/// assert_eq!(&pixels[4..], &[255, 0, 0, 0]);
/// ```
pub fn bleed_transparent(pixels: &mut [u8], width: u32, height: u32) {
    let count = (width as usize) * (height as usize);

    if pixels.len() < count * BYTES_PER_PIXEL || count == 0 {
        return;
    }

    // 不透明な画素を種にして、外へ広げる（幅優先）。
    let mut filled: Vec<bool> = (0..count)
        .map(|index| pixels[index * BYTES_PER_PIXEL + 3] != 0)
        .collect();

    let mut frontier: Vec<usize> = (0..count).filter(|index| filled[*index]).collect();

    if frontier.is_empty() || frontier.len() == count {
        return;
    }

    let mut next = Vec::new();

    while !frontier.is_empty() {
        for &index in &frontier {
            let x = (index % width as usize) as i64;
            let y = (index / width as usize) as i64;

            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x + dx, y + dy);

                if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                    continue;
                }

                let neighbour = (ny as usize) * (width as usize) + (nx as usize);

                if filled[neighbour] {
                    continue;
                }

                filled[neighbour] = true;

                // 色だけ写す。アルファは元のまま。
                let (from, to) = (index * BYTES_PER_PIXEL, neighbour * BYTES_PER_PIXEL);
                pixels[to] = pixels[from];
                pixels[to + 1] = pixels[from + 1];
                pixels[to + 2] = pixels[from + 2];

                next.push(neighbour);
            }
        }

        frontier.clear();
        std::mem::swap(&mut frontier, &mut next);
    }
}

/// 絵のまわりを、いちばん外の画素で埋めて広げる。
///
/// アトラスでは絵と絵が隣り合うので、線形補間が**隣の絵**を舐めます。
/// 自分の縁を伸ばした余白で挟んでおけば、舐めても自分の色しか出ません。
///
/// ```
/// # use gueiz_gpu::atlas::extend_edges;
/// // 1x1 の赤を 1 画素ぶん広げると 3x3 の赤になる。
/// let expanded = extend_edges(&[255, 0, 0, 255], 1, 1, 1);
///
/// assert_eq!(expanded.len(), 3 * 3 * 4);
/// assert_eq!(&expanded[..4], &[255, 0, 0, 255]);
/// ```
pub fn extend_edges(pixels: &[u8], width: u32, height: u32, padding: u32) -> Vec<u8> {
    let (padded_width, padded_height) = (width + padding * 2, height + padding * 2);
    let mut out = vec![0_u8; (padded_width * padded_height) as usize * BYTES_PER_PIXEL];

    for y in 0..padded_height {
        // 端を越えたら、いちばん近い行・列に貼り付く。
        let source_y = (y as i64 - padding as i64).clamp(0, height as i64 - 1) as u32;

        for x in 0..padded_width {
            let source_x = (x as i64 - padding as i64).clamp(0, width as i64 - 1) as u32;

            let from = ((source_y * width + source_x) as usize) * BYTES_PER_PIXEL;
            let to = ((y * padded_width + x) as usize) * BYTES_PER_PIXEL;

            out[to..to + BYTES_PER_PIXEL]
                .copy_from_slice(&pixels[from..from + BYTES_PER_PIXEL]);
        }
    }

    out
}

/// 縦横を半分にする。**乗算済みに直してから混ぜる。**
///
/// 普通のアルファのまま平均すると、透明な画素の色が同じ重みで混ざって
/// 縁が濁ります。乗算済みなら透明な画素は色にも寄与しません。
///
/// ```
/// # use gueiz_gpu::atlas::downsample;
/// // 2x2。左上だけ不透明な赤、残りは透明。
/// let half = downsample(&[
///     255, 0, 0, 255,  0, 0, 0, 0,
///     0, 0, 0, 0,      0, 0, 0, 0,
/// ], 2, 2);
///
/// // 色は赤のまま、覆っているのは 4 分の 1（255 / 4 = 63）。
/// assert_eq!(half, vec![255, 0, 0, 63]);
/// ```
pub fn downsample(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let half_width = (width / 2).max(1);
    let half_height = (height / 2).max(1);
    let mut out = vec![0_u8; (half_width * half_height) as usize * BYTES_PER_PIXEL];

    for y in 0..half_height {
        for x in 0..half_width {
            let mut sum = [0_u32; 4];
            let mut taken = 0_u32;

            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = (x * 2 + dx, y * 2 + dy);

                if sx >= width || sy >= height {
                    continue;
                }

                let at = ((sy * width + sx) as usize) * BYTES_PER_PIXEL;
                let alpha = u32::from(pixels[at + 3]);

                // 乗算済みにしてから足す。
                sum[0] += u32::from(pixels[at]) * alpha;
                sum[1] += u32::from(pixels[at + 1]) * alpha;
                sum[2] += u32::from(pixels[at + 2]) * alpha;
                sum[3] += alpha;

                taken += 1;
            }

            let to = ((y * half_width + x) as usize) * BYTES_PER_PIXEL;
            let alpha_sum = sum[3];

            out[to + 3] = (alpha_sum / taken.max(1)) as u8;

            if alpha_sum == 0 {
                continue;
            }

            // 普通のアルファへ戻す。
            for channel in 0..3 {
                out[to + channel] = (sum[channel] / alpha_sum).min(255) as u8;
            }
        }
    }

    out
}

// --- GPU 側 ---

/// 詰め込んだ絵を載せた配列テクスチャ。
pub struct Atlas {
    descriptor: AtlasDescriptor,
    /// 実際に使う余白。段を作るぶんだけ [`AtlasDescriptor::padding`] より広い。
    padding: u32,
    mip_level_count: u32,
    packer: ShelfPacker,
    texture: Option<wgpu::Texture>,
    sheet: Option<std::sync::Arc<SpriteSheet>>,
    placed: Vec<Placement>,
}

impl Atlas {
    pub fn new(descriptor: AtlasDescriptor) -> Self {
        let mip_level_count = descriptor.mip_levels.resolve(descriptor.page_size);

        // 段を下るほど隣が近づく。段の数だけ余白を広げないと混ざる。
        let alignment = 1 << (mip_level_count - 1);
        let padding = descriptor.padding.max(alignment);

        if padding != descriptor.padding {
            log::info!(
                "atlas: padding {} -> {} ({} mip levels)",
                descriptor.padding,
                padding,
                mip_level_count,
            );
        }

        Self {
            packer: ShelfPacker::new(
                descriptor.page_size,
                padding,
                alignment,
                descriptor.max_pages,
            ),
            descriptor,
            padding,
            mip_level_count,
            texture: None,
            sheet: None,
            placed: Vec::new(),
        }
    }

    /// 絵を 1 枚入れる。`pixels` は `width * height * 4` バイトの RGBA8。
    pub fn insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<TextureRegion, Gueiz2DError> {
        let expected = (width as usize) * (height as usize) * BYTES_PER_PIXEL;

        if width == 0 || height == 0 {
            return Err(Gueiz2DError::EmptySpriteSheetError);
        }

        if pixels.len() != expected {
            return Err(Gueiz2DError::SpriteSizeMismatchError {
                layer: 0,
                expected,
                found: pixels.len(),
            });
        }

        let Some(placement) = self.packer.insert(width, height) else {
            return Err(Gueiz2DError::AtlasFullError {
                width,
                height,
                pages: self.packer.page_count(),
            });
        };

        self.ensure_pages(device, queue, self.packer.page_count())?;
        self.write(queue, placement, pixels);
        self.placed.push(placement);

        Ok(self.region(placement))
    }

    /// 何枚かまとめて入れる。**高い順に詰めるのでページが減ります。**
    ///
    /// 返る順番は渡した順番と同じです。
    pub fn insert_batch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &[(u32, u32, &[u8])],
    ) -> Result<Vec<TextureRegion>, Gueiz2DError> {
        // 高い順に入れると棚の高さが揃う。返す順番は元のままにしたいので、
        // 並べ替えるのは番号のほうだけ。
        let mut order: Vec<usize> = (0..images.len()).collect();
        order.sort_by_key(|index| std::cmp::Reverse(images[*index].1));

        let mut regions = vec![None; images.len()];
        let mut pending = Vec::with_capacity(images.len());

        for index in order {
            let (width, height, pixels) = images[index];
            let expected = (width as usize) * (height as usize) * BYTES_PER_PIXEL;

            if width == 0 || height == 0 {
                return Err(Gueiz2DError::EmptySpriteSheetError);
            }

            if pixels.len() != expected {
                return Err(Gueiz2DError::SpriteSizeMismatchError {
                    layer: index as u32,
                    expected,
                    found: pixels.len(),
                });
            }

            let Some(placement) = self.packer.insert(width, height) else {
                return Err(Gueiz2DError::AtlasFullError {
                    width,
                    height,
                    pages: self.packer.page_count(),
                });
            };

            regions[index] = Some(self.region(placement));
            pending.push((placement, pixels));
        }

        // 場所が全部決まってから、1 度だけ器を広げる。
        self.ensure_pages(device, queue, self.packer.page_count())?;

        for (placement, pixels) in pending {
            self.write(queue, placement, pixels);
            self.placed.push(placement);
        }

        Ok(regions.into_iter().map(|region| region.expect("全部決めた")).collect())
    }

    /// バインドグループに差すシート。まだ 1 枚も入れていなければ `None`。
    pub fn sheet(&self) -> Option<&std::sync::Arc<SpriteSheet>> {
        self.sheet.as_ref()
    }

    pub fn page_count(&self) -> u32 {
        self.packer.page_count()
    }

    pub fn page_size(&self) -> u32 {
        self.descriptor.page_size
    }

    pub fn mip_level_count(&self) -> u32 {
        self.mip_level_count
    }

    /// 実際に使っている余白。段を作るなら広げてある。
    pub fn padding(&self) -> u32 {
        self.padding
    }

    /// 詰まり具合。0..1。
    pub fn occupancy(&self) -> f32 {
        self.packer.occupancy(&self.placed)
    }

    /// 絵 1 枚ぶんの場所を返す。
    ///
    /// **画素はそのまま残ります。** 消しても誰も読まないので、
    /// 消すためだけに書き込むのは無駄です。次の絵がそこに入ると
    /// 上書きされ、余白まで塗り直されるので、古い色が漏れることもありません。
    ///
    /// 知らない場所を渡されたら `false`。
    pub fn remove(&mut self, region: TextureRegion) -> bool {
        let Some(at) = self
            .placed
            .iter()
            .position(|placement| self.region(*placement) == region)
        else {
            return false;
        };

        let placement = self.placed.swap_remove(at);

        self.packer.remove(placement)
    }

    /// 入っている絵の数。
    pub fn len(&self) -> usize {
        self.placed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// 全部捨てて空にする。**貼ってある絵は全部使えなくなります。**
    pub fn clear(&mut self) {
        self.packer.clear();
        self.placed.clear();
        self.texture = None;
        self.sheet = None;
    }

    fn region(&self, placement: Placement) -> TextureRegion {
        let size = self.descriptor.page_size as f32;

        TextureRegion {
            page: placement.page,
            uv_rect: [
                placement.x as f32 / size,
                placement.y as f32 / size,
                placement.width as f32 / size,
                placement.height as f32 / size,
            ],
            size: [placement.width, placement.height],
        }
    }

    /// 層が足りなければ、広いテクスチャを作って**GPU 上で写す**。
    fn ensure_pages(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pages: u32,
    ) -> Result<(), Gueiz2DError> {
        let existing = self
            .texture
            .as_ref()
            .map_or(0, |texture| texture.depth_or_array_layers());

        if existing >= pages {
            return Ok(());
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gueiz atlas"),
            size: wgpu::Extent3d {
                width: self.descriptor.page_size,
                height: self.descriptor.page_size,
                depth_or_array_layers: pages,
            },
            mip_level_count: self.mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        if let Some(old) = self.texture.take() {
            // 画素を CPU に持ち帰らずに済む。段も一緒に写す。
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gueiz atlas grow"),
            });

            for level in 0..self.mip_level_count {
                let side = (self.descriptor.page_size >> level).max(1);

                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &old,
                        mip_level: level,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: existing,
                    },
                );
            }

            queue.submit(Some(encoder.finish()));
        }

        log::info!(
            "atlas: {} pages of {}px ({} mip levels, {} px padding)",
            pages,
            self.descriptor.page_size,
            self.mip_level_count,
            self.padding,
        );

        self.sheet = Some(std::sync::Arc::new(SpriteSheet::from_texture(
            device,
            &texture,
            self.descriptor.filter,
            self.mip_level_count > 1,
        )));
        self.texture = Some(texture);

        Ok(())
    }

    /// 1 枚ぶんを、余白と縮小段つきで書き込む。
    fn write(&self, queue: &wgpu::Queue, placement: Placement, pixels: &[u8]) {
        let texture = self.texture.as_ref().expect("器は先に広げてある");

        // 透明なところに色をにじませてから、まわりを伸ばす。
        // この順でないと、伸ばした余白にも透明の黒が残る。
        let mut prepared = pixels.to_vec();
        bleed_transparent(&mut prepared, placement.width, placement.height);

        let mut level_pixels =
            extend_edges(&prepared, placement.width, placement.height, self.padding);
        let mut level_width = placement.width + self.padding * 2;
        let mut level_height = placement.height + self.padding * 2;

        for level in 0..self.mip_level_count {
            // 置き場所は刻みに揃えてあるので、段ごとにそのまま割れる。
            let origin_x = (placement.x - self.padding) >> level;
            let origin_y = (placement.y - self.padding) >> level;

            if level_width == 0 || level_height == 0 {
                break;
            }

            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: level,
                    origin: wgpu::Origin3d {
                        x: origin_x,
                        y: origin_y,
                        z: placement.page,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &level_pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(level_width * BYTES_PER_PIXEL as u32),
                    rows_per_image: Some(level_height),
                },
                wgpu::Extent3d {
                    width: level_width,
                    height: level_height,
                    depth_or_array_layers: 1,
                },
            );

            if level + 1 >= self.mip_level_count {
                break;
            }

            level_pixels = downsample(&level_pixels, level_width, level_height);
            level_width = (level_width / 2).max(1);
            level_height = (level_height / 2).max(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- 詰め込み ---

    #[test]
    fn a_single_image_lands_on_the_first_page() {
        let mut packer = ShelfPacker::new(256, 0, 1, 4);
        let placement = packer.insert(32, 32).expect("入る");

        assert_eq!(placement.page, 0);
        assert_eq!((placement.x, placement.y), (0, 0));
        assert_eq!((placement.width, placement.height), (32, 32));
    }

    /// 同じ高さの絵は同じ棚に横並びになる。
    #[test]
    fn images_of_the_same_height_share_a_shelf() {
        let mut packer = ShelfPacker::new(256, 0, 1, 4);

        let first = packer.insert(32, 32).expect("入る");
        let second = packer.insert(32, 32).expect("入る");

        assert_eq!(first.y, second.y, "同じ棚のはず");
        assert_eq!(second.x, 32);
        assert_eq!(packer.page_count(), 1);
    }

    /// 棚の幅が尽きたら次の棚へ。
    #[test]
    fn a_full_shelf_starts_a_new_one() {
        let mut packer = ShelfPacker::new(64, 0, 1, 4);

        packer.insert(64, 16).expect("入る");
        let next = packer.insert(64, 16).expect("入る");

        assert_eq!(next.y, 16, "下の段に移るはず");
        assert_eq!(next.x, 0);
    }

    /// ページが尽きたら次のページへ。
    #[test]
    fn a_full_page_starts_a_new_one() {
        let mut packer = ShelfPacker::new(64, 0, 1, 4);

        packer.insert(64, 64).expect("入る");
        let next = packer.insert(64, 64).expect("入る");

        assert_eq!(next.page, 1);
        assert_eq!(packer.page_count(), 2);
    }

    /// ページの上限を超えたら入らない。黙って溢れさせない。
    #[test]
    fn running_out_of_pages_is_reported() {
        let mut packer = ShelfPacker::new(64, 0, 1, 2);

        packer.insert(64, 64).expect("入る");
        packer.insert(64, 64).expect("入る");

        assert_eq!(packer.insert(64, 64), None);
    }

    /// ページより大きい絵は入らない。
    #[test]
    fn an_oversized_image_does_not_fit() {
        let mut packer = ShelfPacker::new(64, 0, 1, 8);

        assert_eq!(packer.insert(65, 8), None);
        assert_eq!(packer.insert(8, 65), None);
    }

    /// 余白ぶんだけ内側にずれ、隣とのあいだが空く。
    #[test]
    fn padding_separates_neighbours() {
        let mut packer = ShelfPacker::new(256, 2, 1, 4);

        let first = packer.insert(32, 32).expect("入る");
        let second = packer.insert(32, 32).expect("入る");

        assert_eq!((first.x, first.y), (2, 2), "左上にも余白が付く");
        // 32 + 余白 2 枚ぶん。
        assert_eq!(second.x - first.x, 36);
        assert!(second.x - (first.x + first.width) >= 2, "あいだが空いていない");
    }

    /// 刻みに揃う。縮小段のときに段ごとの位置が割り切れなくなるのを防ぐ。
    #[test]
    fn placements_snap_to_the_alignment() {
        let mut packer = ShelfPacker::new(256, 0, 8, 4);

        packer.insert(5, 5).expect("入る");
        let second = packer.insert(5, 5).expect("入る");

        assert_eq!(second.x % 8, 0, "x が刻みに乗っていない: {}", second.x);
    }

    /// 高い順に入れると棚の高さが揃って、ページが減る。
    ///
    /// 交互に来ると、高い絵が作った棚に低い絵が載って**上が余ります**。
    /// 高い順なら余りが最後の 1 段にまとまります。
    #[test]
    fn sorting_by_height_packs_tighter() {
        // 100 幅のページに、幅 10 の絵を高い／低い交互で 30 枚。
        let sizes: Vec<(u32, u32)> = (0..30)
            .map(|index| if index % 2 == 0 { (10, 45) } else { (10, 10) })
            .collect();

        let mut as_given = ShelfPacker::new(100, 0, 1, 8);
        for (width, height) in &sizes {
            as_given.insert(*width, *height).expect("入る");
        }

        let mut sorted = sizes.clone();
        sorted.sort_by_key(|(_, height)| std::cmp::Reverse(*height));

        let mut by_height = ShelfPacker::new(100, 0, 1, 8);
        for (width, height) in &sorted {
            by_height.insert(*width, *height).expect("入る");
        }

        assert_eq!(as_given.page_count(), 2, "交互だと 2 ページに溢れる");
        assert_eq!(by_height.page_count(), 1, "高い順なら 1 ページに収まる");
    }

    // --- 返す ---

    /// 返した場所は次の絵が使う。使い回さないとページが伸び続ける。
    #[test]
    fn a_removed_slot_is_used_again() {
        let mut packer = ShelfPacker::new(256, 0, 1, 4);

        let first = packer.insert(32, 32).expect("入る");
        packer.insert(32, 32).expect("入る");

        assert!(packer.remove(first));

        let reused = packer.insert(32, 32).expect("入る");

        assert_eq!((reused.x, reused.y), (first.x, first.y), "穴を埋めていない");
    }

    /// 出し入れを繰り返してもページは伸びない。ここが要点。
    #[test]
    fn churning_does_not_grow_the_atlas() {
        let mut packer = ShelfPacker::new(64, 0, 1, 8);
        let mut current = packer.insert(64, 64).expect("入る");

        for _ in 0..100 {
            assert!(packer.remove(current));
            current = packer.insert(64, 64).expect("入る");
        }

        assert_eq!(packer.page_count(), 1, "出し入れでページが増えている");
    }

    /// 隣り合った空きはつながる。つながらないと、返した数だけ
    /// 半端な区間が残って大きい絵が入らなくなる。
    #[test]
    fn neighbouring_gaps_are_joined() {
        let mut packer = ShelfPacker::new(256, 0, 1, 4);

        let a = packer.insert(32, 32).expect("入る");
        let b = packer.insert(32, 32).expect("入る");
        packer.insert(32, 32).expect("入る");

        packer.remove(a);
        packer.remove(b);

        // 32 が 2 つぶん空いたので、64 幅が入るはず。
        let wide = packer.insert(64, 32).expect("つながっていれば入る");

        assert_eq!(wide.x, a.x);
    }

    /// 順番を変えてもつながる。左から返しても右から返しても同じ。
    #[test]
    fn gaps_join_no_matter_the_order() {
        for reverse in [false, true] {
            let mut packer = ShelfPacker::new(256, 0, 1, 4);

            let a = packer.insert(32, 32).expect("入る");
            let b = packer.insert(32, 32).expect("入る");
            let c = packer.insert(32, 32).expect("入る");
            packer.insert(32, 32).expect("入る");

            let mut order = [a, b, c];

            if reverse {
                order.reverse();
            }

            for placement in order {
                assert!(packer.remove(placement));
            }

            assert!(
                packer.insert(96, 32).is_some(),
                "reverse={reverse} でつながっていない",
            );
        }
    }

    /// 棚の右端まで空いたら、端が戻る。戻らないと右側が死んだままになる。
    #[test]
    fn freeing_the_tail_gives_the_width_back() {
        let mut packer = ShelfPacker::new(64, 0, 1, 4);

        packer.insert(32, 32).expect("入る");
        let tail = packer.insert(32, 32).expect("入る");

        packer.remove(tail);

        // 棚が 32 まで戻っているので、32 幅がまた入る。
        assert_eq!(packer.insert(32, 32).expect("入る").x, 32);
    }

    /// 全部返すと、ページごと降りる。
    #[test]
    fn emptying_a_page_drops_it() {
        let mut packer = ShelfPacker::new(64, 0, 1, 4);

        let first = packer.insert(64, 32).expect("入る");
        let second = packer.insert(64, 32).expect("入る");
        assert_eq!(packer.page_count(), 1);

        packer.remove(second);
        packer.remove(first);

        assert_eq!(packer.page_count(), 0);
        assert!(packer.is_empty());
    }

    /// 返したあとは 2 ページめが要らなくなる。
    #[test]
    fn freeing_room_avoids_a_second_page() {
        let mut packer = ShelfPacker::new(64, 0, 1, 4);

        let first = packer.insert(64, 64).expect("入る");
        packer.remove(first);

        let next = packer.insert(64, 64).expect("入る");

        assert_eq!(next.page, 0, "空いているのに次のページへ行った");
        assert_eq!(packer.page_count(), 1);
    }

    /// 知らない場所を返しても落ちない。
    #[test]
    fn removing_something_unknown_is_reported() {
        let mut packer = ShelfPacker::new(64, 0, 1, 4);

        assert!(!packer.remove(Placement {
            page: 7,
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        }));
    }

    /// 余白ぶんも一緒に返る。返し忘れると、出し入れのたびに
    /// 余白の幅だけ削れていく。
    #[test]
    fn the_padding_comes_back_too() {
        let mut packer = ShelfPacker::new(64, 4, 1, 4);

        let first = packer.insert(40, 40).expect("入る");
        packer.remove(first);
        let again = packer.insert(40, 40).expect("入る");

        assert_eq!((again.x, again.y), (first.x, first.y));
    }

    /// 惜しくないほうから埋める。大きい穴を小さい絵で潰さない。
    #[test]
    fn the_tightest_gap_is_filled_first() {
        let mut packer = ShelfPacker::new(256, 0, 1, 4);

        let small = packer.insert(16, 32).expect("入る");
        packer.insert(8, 32).expect("入る");
        let large = packer.insert(64, 32).expect("入る");
        packer.insert(8, 32).expect("入る");

        packer.remove(small);
        packer.remove(large);

        // 16 の穴と 64 の穴がある。16 の絵は 16 の穴へ。
        assert_eq!(packer.insert(16, 32).expect("入る").x, small.x);
    }

    // --- にじませ ---

    #[test]
    fn bleeding_fills_transparent_pixels_with_the_nearest_colour() {
        // 左が赤、右 3 つが透明。
        let mut pixels = vec![255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bleed_transparent(&mut pixels, 4, 1);

        for index in 0..4 {
            let at = index * 4;
            assert_eq!(&pixels[at..at + 3], &[255, 0, 0], "{index} 番目");
        }
    }

    /// アルファは触らない。触ると見た目が変わる。
    #[test]
    fn bleeding_leaves_the_alpha_alone() {
        let mut pixels = vec![255, 0, 0, 255, 0, 0, 0, 0];
        bleed_transparent(&mut pixels, 2, 1);

        assert_eq!(pixels[3], 255);
        assert_eq!(pixels[7], 0, "透明のままのはず");
    }

    /// いちばん近い色が来る。遠いほうに引っぱられない。
    #[test]
    fn bleeding_takes_the_nearest_of_two_colours() {
        // 赤 _ _ 青。真ん中 2 つはそれぞれ近いほうを取る。
        let mut pixels = vec![
            255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255,
        ];
        bleed_transparent(&mut pixels, 4, 1);

        assert_eq!(&pixels[4..7], &[255, 0, 0], "左は赤寄り");
        assert_eq!(&pixels[8..11], &[0, 0, 255], "右は青寄り");
    }

    /// 全部透明なら何もしない。落ちないこと。
    #[test]
    fn bleeding_a_fully_transparent_image_is_harmless() {
        let mut pixels = vec![0_u8; 4 * 4];
        bleed_transparent(&mut pixels, 4, 1);

        assert_eq!(pixels, vec![0_u8; 4 * 4]);
    }

    #[test]
    fn bleeding_a_fully_opaque_image_changes_nothing() {
        let mut pixels = vec![1, 2, 3, 255, 4, 5, 6, 255];
        let before = pixels.clone();
        bleed_transparent(&mut pixels, 2, 1);

        assert_eq!(pixels, before);
    }

    // --- 縁を伸ばす ---

    #[test]
    fn extending_grows_the_image_on_every_side() {
        let expanded = extend_edges(&[9, 8, 7, 6], 1, 1, 2);

        assert_eq!(expanded.len(), 5 * 5 * 4);
        // どこを見ても元の 1 画素。
        for chunk in expanded.chunks_exact(4) {
            assert_eq!(chunk, &[9, 8, 7, 6]);
        }
    }

    /// 伸ばすのは**いちばん近い縁**。折り返しでも繰り返しでもない。
    #[test]
    fn extending_clamps_to_the_nearest_edge() {
        // 2x1。左が赤、右が青。
        let expanded = extend_edges(&[255, 0, 0, 255, 0, 0, 255, 255], 2, 1, 1);

        // 幅 4 になる。左端は赤、右端は青。
        assert_eq!(&expanded[..4], &[255, 0, 0, 255]);
        assert_eq!(&expanded[12..16], &[0, 0, 255, 255]);
    }

    #[test]
    fn extending_by_zero_changes_nothing() {
        let original = vec![1, 2, 3, 4, 5, 6, 7, 8];

        assert_eq!(extend_edges(&original, 2, 1, 0), original);
    }

    // --- 縮小 ---

    #[test]
    fn downsampling_halves_both_sides() {
        let half = downsample(&[0_u8; 4 * 4 * 4], 4, 4);

        assert_eq!(half.len(), 2 * 2 * 4);
    }

    /// 透明な画素に色を引っぱられないこと。ここが乗算済みで混ぜる理由。
    #[test]
    fn downsampling_ignores_the_colour_of_transparent_pixels() {
        // 左上が不透明な赤。残りは「緑だが透明」。
        let half = downsample(
            &[
                255, 0, 0, 255, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0,
            ],
            2,
            2,
        );

        // 普通のアルファのまま平均すると緑が混ざって (64, 191, 0) になる。
        assert_eq!(&half[..3], &[255, 0, 0], "緑が混ざっている");
        assert_eq!(half[3], 63, "覆っているのは 4 分の 1");
    }

    #[test]
    fn downsampling_averages_opaque_colours() {
        let half = downsample(
            &[
                0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
            ],
            2,
            2,
        );

        assert_eq!(half[3], 255);
        assert!((127..=128).contains(&half[0]), "{}", half[0]);
    }

    /// 1 画素まで縮んでも 0 にならない。0 だとテクスチャが作れない。
    #[test]
    fn downsampling_never_reaches_zero() {
        let half = downsample(&[1, 2, 3, 255], 1, 1);

        assert_eq!(half.len(), BYTES_PER_PIXEL);
    }

    // --- 段の数 ---

    #[test]
    fn mip_levels_resolve_against_the_page_size() {
        // 256 なら 256,128,64,32,16,8,4,2,1 で 9 段。
        assert_eq!(MipLevels::Full.resolve(256), 9);
        assert_eq!(MipLevels::None.resolve(256), 1);
        assert_eq!(MipLevels::Count(4).resolve(256), 4);
        // 行きすぎた指定は切り詰める。
        assert_eq!(MipLevels::Count(99).resolve(256), 9);
        assert_eq!(MipLevels::Count(0).resolve(256), 1);
    }

    /// 段を作るなら余白は自動で広がる。足りないと段の下で隣と混ざる。
    #[test]
    fn mip_levels_widen_the_padding() {
        let atlas = Atlas::new(AtlasDescriptor {
            page_size: 256,
            padding: 1,
            mip_levels: MipLevels::Count(4),
            ..Default::default()
        });

        // 4 段なら 2^3 = 8 画素。
        assert_eq!(atlas.padding(), 8);
        assert_eq!(atlas.mip_level_count(), 4);
    }

    #[test]
    fn without_mips_the_padding_is_left_alone() {
        let atlas = Atlas::new(AtlasDescriptor {
            padding: 1,
            mip_levels: MipLevels::None,
            ..Default::default()
        });

        assert_eq!(atlas.padding(), 1);
        assert_eq!(atlas.mip_level_count(), 1);
    }

    /// 何も入れていないアトラスは、差すシートを持たない。
    #[test]
    fn a_fresh_atlas_has_nothing_to_bind() {
        let atlas = Atlas::new(AtlasDescriptor::default());

        assert!(atlas.sheet().is_none());
        assert_eq!(atlas.page_count(), 0);
        assert_eq!(atlas.occupancy(), 0.0);
    }

    /// 詰まり具合は、置いた面積 ÷ ページの面積。
    #[test]
    fn occupancy_counts_the_area_that_was_placed() {
        let mut packer = ShelfPacker::new(64, 0, 1, 4);
        let placed: Vec<Placement> = (0..2)
            .map(|_| packer.insert(32, 64).expect("入る"))
            .collect();

        // 32x64 が 2 枚で 64x64 ちょうど。
        assert!((packer.occupancy(&placed) - 1.0).abs() < 1e-6);
    }
}
