//! 任意の形でのクリッピング。
//!
//! 矩形・楕円・半平面は式で書けるので [`crate::effect::Block`] がその場で解きますが、
//! 凹んだ形や穴の空いた形は式になりません。そこで**形を覆いの絵に焼いて**おき、
//! 描くときに引いて不透明度に掛けます。
//!
//! # 1 マスク = 1 層
//!
//! 覆いはテクスチャ配列の 1 層をまるごと使います。詰め込みが要らないので
//! 場所の管理が無く、書き換えも層 1 枚の差し替えで済みます。
//! 層はすべて同じ辺の長さで、[`crate::draw_manager::DrawManagerDescriptor`] の
//! `clip_mask_resolution` で決まります。
//!
//! **どれだけ増やしてもドローの数は増えません。** 1 枚のテクスチャ配列に収まって
//! いるので、バインドグループの差し替えが起きないからです。
//!
//! # 焼くのは CPU
//!
//! 形が変わったときだけ走るので、毎フレームの費用はテクスチャ 1 回の読み取りだけです。
//!
//! # 何を焼くか
//!
//! [`ClipMaskKind`] で 2 通りあります。
//!
//! | | 焼くもの | 拡大したとき | `softness` |
//! |---|---|---|---|
//! | [`ClipMaskKind::Coverage`] | 覆う割合 | ぼける | 効かない |
//! | [`ClipMaskKind::Distance`] | 境目までの距離 | **縁が保てる** | 効く |
//!
//! 距離は混ぜてもほぼ距離のままなので、画素のあいだを引いても形が崩れません。
//! 覆う割合は混ぜると縁が鈍るので、焼いた倍率で見るとき向けです。

use fxhash::FxHashMap;

use gueiz_gpu::vertex::Vertex;

use crate::effect::Block;

/// 覆う割合を焼くときの細かさ。1 画素をこの 2 乗の点で数える。
pub const SUPERSAMPLE: u32 = 4;

/// 距離を焼くとき、境目から何画素ぶんまで正しい距離を持つか。
///
/// ここを超えると頭打ちになります。縁をなめらかにするだけなら 1 画素で足りますが、
/// `softness` でぼかす余地を持たせてあります。
pub const SPREAD: f32 = 8.0;

/// 覆いに何を焼くか。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub enum ClipMaskKind {
    /// 覆う割合。焼いた倍率で見るならこれがいちばん素直。
    #[default]
    Coverage,
    /// 境目までの符号付き距離。**拡大しても縁が保てる。**
    ///
    /// 距離なので `softness` でぼかせます。そのぶん焼くのは少し重く、
    /// 覆う範囲は正方形に揃えられます（縦と横で距離の意味を揃えるため）。
    Distance,
}

/// 焼いた覆い 1 枚への参照。
///
/// [`crate::draw_manager::DrawManager::add_clip_mask`] が返します。
/// [`ClipMask::block`] で山にしてから図形に積んでください。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct ClipMask {
    layer: u32,
    /// 覆いが覆うワールドの範囲。`[min_x, min_y, 幅, 高さ]`。
    bounds: [f32; 4],
    kind: ClipMaskKind,
    /// 距離を焼いたとき、頭打ちになるまでのワールドの長さ。
    spread: f32,
}

impl ClipMask {
    pub(crate) fn new(layer: u32, bounds: [f32; 4], kind: ClipMaskKind, spread: f32) -> Self {
        Self {
            layer,
            bounds,
            kind,
            spread,
        }
    }

    /// この覆いを使う山。図形に積むと、覆いの外が削れます。
    pub fn block(self) -> Block {
        self.block_with(0.0, false)
    }

    /// 内と外を入れ替えた山。覆いの**中**が削れます。
    pub fn block_inverted(self, invert: bool) -> Block {
        self.block_with(0.0, invert)
    }

    /// ぼかしと反転を指定した山。
    ///
    /// **`softness` が効くのは [`ClipMaskKind::Distance`] だけです。**
    /// 覆う割合には境目からの距離が入っていないので、広げようがありません。
    pub fn block_with(self, softness: f32, invert: bool) -> Block {
        let min = [self.bounds[0], self.bounds[1]];
        let size = [self.bounds[2], self.bounds[3]];

        match self.kind {
            ClipMaskKind::Coverage => Block::ClipMask {
                min,
                size,
                layer: self.layer,
                invert,
            },

            ClipMaskKind::Distance => Block::ClipDistanceMask {
                min,
                size,
                layer: self.layer,
                spread: self.spread,
                softness,
                invert,
            },
        }
    }

    /// 何を焼いたか。
    pub fn kind(self) -> ClipMaskKind {
        self.kind
    }

    /// 距離が頭打ちになるまでのワールドの長さ。[`ClipMaskKind::Coverage`] では 0。
    pub fn spread(self) -> f32 {
        self.spread
    }

    /// テクスチャ配列の何層目か。
    pub fn layer(self) -> u32 {
        self.layer
    }

    /// 覆っているワールドの範囲。`[min_x, min_y, 幅, 高さ]`。
    ///
    /// この外は必ず削られます。図形がここからはみ出すなら、
    /// 焼き直すときに広い範囲を渡してください。
    pub fn bounds(self) -> [f32; 4] {
        self.bounds
    }
}

/// 三角形の並びを覆う矩形。`[min_x, min_y, 幅, 高さ]`。
///
/// 縁がちょうど境目に乗ると半分だけ数えられて薄くなるので、
/// `margin` 画素ぶん外に広げます。
pub(crate) fn cover(
    triangles: &[Vertex],
    resolution: u32,
    margin: f32,
    square: bool,
) -> [f32; 4] {
    if triangles.is_empty() {
        return [0.0, 0.0, 0.0, 0.0];
    }

    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];

    for vertex in triangles {
        low[0] = low[0].min(vertex.x);
        low[1] = low[1].min(vertex.y);
        high[0] = high[0].max(vertex.x);
        high[1] = high[1].max(vertex.y);
    }

    if !low[0].is_finite() || !high[0].is_finite() {
        return [0.0, 0.0, 0.0, 0.0];
    }

    let (mut width, mut height) = (high[0] - low[0], high[1] - low[1]);

    // 画素 1 つぶんを求めてから広げる。潰れた形でも 0 割りしない。
    let step = width.max(height).max(1.0) / resolution.max(1) as f32;
    let pad = step * margin;

    let (mut min_x, mut min_y) = (low[0] - pad, low[1] - pad);
    width = (width + pad * 2.0).max(step);
    height = (height + pad * 2.0).max(step);

    // 距離を焼くときは、縦と横で 1 画素の意味が変わると距離が歪む。
    // 長い方に合わせて正方形にし、中心を保ったまま広げる。
    if square {
        let side = width.max(height);

        min_x -= (side - width) / 2.0;
        min_y -= (side - height) / 2.0;
        width = side;
        height = side;
    }

    [min_x, min_y, width, height]
}

/// 三角形の並びを、覆う割合の絵に焼く。
///
/// 返るのは `resolution * resolution` バイト。0 が外、255 が中。
/// 三角形は**重ね合わせ**で見るので、穴を橋渡しで表した輪郭もそのまま通ります。
pub(crate) fn rasterise(triangles: &[Vertex], bounds: [f32; 4], resolution: u32) -> Vec<u8> {
    let resolution = resolution.max(1) as usize;
    let step = SUPERSAMPLE as usize;

    // 細かい升目で内外を取ってから平均する。
    let fine = resolution * step;
    let hit = scan(triangles, bounds, fine);

    let mut coverage = vec![0u8; resolution * resolution];
    let samples = (step * step) as u32;

    for row in 0..resolution {
        for column in 0..resolution {
            let mut count = 0u32;

            for inner_row in 0..step {
                let line = (row * step + inner_row) * fine + column * step;

                for inner_column in 0..step {
                    if hit[line + inner_column] {
                        count += 1;
                    }
                }
            }

            coverage[row * resolution + column] = (count * 255 / samples) as u8;
        }
    }

    coverage
}

/// 三角形の並びを、境目までの符号付き距離の絵に焼く。
///
/// 返るのは絵と、頭打ちになるまでのワールドの長さ。
/// 0.5 が境目で、**大きいほうが中**です。[`SPREAD`] 画素ぶん離れると頭打ちになります。
///
/// 距離は画素のあいだを混ぜてもほぼ距離のままなので、拡大しても縁が保てます。
pub(crate) fn distance_field(
    triangles: &[Vertex],
    bounds: [f32; 4],
    resolution: u32,
) -> (Vec<u8>, f32) {
    let resolution = resolution.max(1) as usize;
    let [min_x, min_y, width, height] = bounds;

    let usable = |span: f32| span.is_finite() && span > 0.0;

    // 画素 1 つがワールドで何単位か。正方形に揃えてあるので縦横は同じ。
    let scale = if usable(width) {
        width / resolution as f32
    } else {
        1.0
    };
    let spread_world = SPREAD * scale;

    // 何も無ければ「どこまでも外」。
    if triangles.len() < 3 || !usable(width) || !usable(height) {
        return (vec![0u8; resolution * resolution], spread_world);
    }

    // 内か外かは升目を塗って決める。距離の大きさが精度を持つので、
    // 符号は画素の真ん中で取れば足りる。
    let inside = scan(triangles, bounds, resolution);

    // 境目の辺だけを集める。中で 2 枚の三角形が共有している辺は境目ではない。
    let edges = boundary(triangles);

    // 画素の座標に直してから測る。
    let to_x = |x: f32| (x - min_x) / width * resolution as f32;
    let to_y = |y: f32| (y - min_y) / height * resolution as f32;

    let mut distance = vec![SPREAD; resolution * resolution];

    for [from, to] in edges {
        let start = [to_x(from[0]), to_y(from[1])];
        let finish = [to_x(to[0]), to_y(to[1])];

        // その辺が届く範囲だけ触る。全体を舐めないので、辺が増えても伸びにくい。
        let low_x = start[0].min(finish[0]) - SPREAD;
        let high_x = start[0].max(finish[0]) + SPREAD;
        let low_y = start[1].min(finish[1]) - SPREAD;
        let high_y = start[1].max(finish[1]) + SPREAD;

        if !low_x.is_finite() || !low_y.is_finite() {
            continue;
        }

        let first_x = (low_x.floor().max(0.0) as usize).min(resolution);
        let last_x = ((high_x.ceil() + 1.0).max(0.0) as usize).min(resolution);
        let first_y = (low_y.floor().max(0.0) as usize).min(resolution);
        let last_y = ((high_y.ceil() + 1.0).max(0.0) as usize).min(resolution);

        for row in first_y..last_y {
            let y = row as f32 + 0.5;

            for column in first_x..last_x {
                let span = to_segment(start, finish, [column as f32 + 0.5, y]);
                let cell = &mut distance[row * resolution + column];

                if span < *cell {
                    *cell = span;
                }
            }
        }
    }

    let mut pixels = vec![0u8; resolution * resolution];

    for index in 0..resolution * resolution {
        // 中を負にして、0.5 を境目に写す。中が大きい側になる。
        let signed = if inside[index] {
            -distance[index]
        } else {
            distance[index]
        };
        let stored = (0.5 - signed / (2.0 * SPREAD)).clamp(0.0, 1.0);

        pixels[index] = (stored * 255.0).round() as u8;
    }

    (pixels, spread_world)
}

/// 升目の真ん中が形の中にあるかを塗る。`grid` は 1 辺の升の数。
///
/// 三角形は**重ね合わせ**で見ます。巻き方は問いません。
fn scan(triangles: &[Vertex], bounds: [f32; 4], grid: usize) -> Vec<bool> {
    let mut hit = vec![false; grid * grid];
    let [min_x, min_y, width, height] = bounds;

    // NaN も弾きたいので、正であることを直接確かめる。
    let usable = |span: f32| span.is_finite() && span > 0.0;

    if triangles.len() < 3 || !usable(width) || !usable(height) || grid == 0 {
        return hit;
    }

    let to_x = |x: f32| (x - min_x) / width * grid as f32;
    let to_y = |y: f32| (y - min_y) / height * grid as f32;

    for corners in triangles.chunks_exact(3) {
        let points = [
            [to_x(corners[0].x), to_y(corners[0].y)],
            [to_x(corners[1].x), to_y(corners[1].y)],
            [to_x(corners[2].x), to_y(corners[2].y)],
        ];

        // 三角形を囲む範囲だけ走る。全体を舐めないので、形が小さければ安い。
        let low_x = points.iter().fold(f32::INFINITY, |a, p| a.min(p[0]));
        let high_x = points.iter().fold(f32::NEG_INFINITY, |a, p| a.max(p[0]));
        let low_y = points.iter().fold(f32::INFINITY, |a, p| a.min(p[1]));
        let high_y = points.iter().fold(f32::NEG_INFINITY, |a, p| a.max(p[1]));

        if !low_x.is_finite() || !low_y.is_finite() {
            continue;
        }

        let first_x = (low_x.floor().max(0.0) as usize).min(grid);
        let last_x = ((high_x.ceil() + 1.0).max(0.0) as usize).min(grid);
        let first_y = (low_y.floor().max(0.0) as usize).min(grid);
        let last_y = ((high_y.ceil() + 1.0).max(0.0) as usize).min(grid);

        // 面積の符号。巻き方を問わないので、どちら回りの三角形も通る。
        let area = edge(points[0], points[1], points[2]);

        if area == 0.0 {
            continue;
        }

        for row in first_y..last_y {
            let y = row as f32 + 0.5;

            for column in first_x..last_x {
                let point = [column as f32 + 0.5, y];

                let a = edge(points[1], points[2], point);
                let b = edge(points[2], points[0], point);
                let c = edge(points[0], points[1], point);

                // 3 つとも面積と同じ符号なら中。
                let covered = if area > 0.0 {
                    a >= 0.0 && b >= 0.0 && c >= 0.0
                } else {
                    a <= 0.0 && b <= 0.0 && c <= 0.0
                };

                if covered {
                    hit[row * grid + column] = true;
                }
            }
        }
    }

    hit
}

/// 辺 1 本。両端の位置。
type Edge = [[f32; 2]; 2];

/// 辺を照らし合わせる鍵。両端の座標のビットを、並びを揃えて並べたもの。
type EdgeKey = (u64, u64);

/// 形の**外周**の辺だけを取り出す。
///
/// 中で 2 枚の三角形が背中合わせに持っている辺は 2 回出てくるので消えます。
/// 穴へ渡した橋も往復で 2 回出てくるので、同じように消えます。
/// 残るのは本当の境目だけで、距離を測りたいのはそこまでの長さです。
///
/// 共有された辺を消さずに測ると、**形の奥のほうが「境目のすぐそば」に見えて**
/// しまい、中身が削れます。
fn boundary(triangles: &[Vertex]) -> Vec<Edge> {
    // 同じ辺は同じ頂点から作られるので、ビットがそのまま一致する。
    let pack = |point: [f32; 2]| ((point[0].to_bits() as u64) << 32) | point[1].to_bits() as u64;

    let mut counts: FxHashMap<EdgeKey, (u32, Edge)> = FxHashMap::default();

    for corners in triangles.chunks_exact(3) {
        let points = [
            [corners[0].x, corners[0].y],
            [corners[1].x, corners[1].y],
            [corners[2].x, corners[2].y],
        ];

        for step in 0..3 {
            let from = points[step];
            let to = points[(step + 1) % 3];

            let (a, b) = (pack(from), pack(to));
            let key = if a <= b { (a, b) } else { (b, a) };

            counts
                .entry(key)
                .and_modify(|(count, _)| *count += 1)
                .or_insert((1, [from, to]));
        }
    }

    counts
        .into_values()
        .filter(|(count, _)| count % 2 == 1)
        .map(|(_, edge)| edge)
        .collect()
}

/// 点から線分までの距離。
fn to_segment(from: [f32; 2], to: [f32; 2], point: [f32; 2]) -> f32 {
    let line = [to[0] - from[0], to[1] - from[1]];
    let offset = [point[0] - from[0], point[1] - from[1]];

    let length = line[0] * line[0] + line[1] * line[1];

    // 潰れた辺は点として測る。
    let ratio = if length > 0.0 {
        ((offset[0] * line[0] + offset[1] * line[1]) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let closest = [offset[0] - line[0] * ratio, offset[1] - line[1] * ratio];

    (closest[0] * closest[0] + closest[1] * closest[1]).sqrt()
}

/// 2 点が張る辺から見て、3 点目がどちら側にあるか。符号付きの面積の 2 倍。
fn edge(from: [f32; 2], to: [f32; 2], point: [f32; 2]) -> f32 {
    (to[0] - from[0]) * (point[1] - from[1]) - (to[1] - from[1]) * (point[0] - from[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESOLUTION: u32 = 64;

    fn at(x: f32, y: f32) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0)
    }

    /// 四角 1 枚を 2 つの三角形に開いたもの。**真ん中に対角線が通る。**
    fn square(low: f32, high: f32) -> Vec<Vertex> {
        vec![
            at(low, low),
            at(high, low),
            at(high, high),
            at(low, low),
            at(high, high),
            at(low, high),
        ]
    }

    /// 焼いた絵から、その升目の値を引く。
    fn value(pixels: &[u8], resolution: u32, column: u32, row: u32) -> u8 {
        pixels[(row * resolution + column) as usize]
    }

    /// ワールドの点が、焼いた絵のどの升目に落ちるか。
    fn cell(bounds: [f32; 4], resolution: u32, x: f32, y: f32) -> (u32, u32) {
        let [min_x, min_y, width, height] = bounds;
        let column = ((x - min_x) / width * resolution as f32) as u32;
        let row = ((y - min_y) / height * resolution as f32) as u32;

        (column.min(resolution - 1), row.min(resolution - 1))
    }

    // ---- 覆う範囲 ----

    #[test]
    fn the_cover_wraps_every_vertex() {
        let bounds = cover(&square(10.0, 90.0), RESOLUTION, 0.0, false);

        assert_eq!(bounds, [10.0, 10.0, 80.0, 80.0]);
    }

    #[test]
    fn the_margin_pushes_the_edge_off_the_grid() {
        // 縁がちょうど升目の境目に乗ると半分だけ数えられて薄くなる。
        let bare = cover(&square(10.0, 90.0), RESOLUTION, 0.0, false);
        let padded = cover(&square(10.0, 90.0), RESOLUTION, 1.0, false);

        let step = 80.0 / RESOLUTION as f32;

        assert!((padded[0] - (bare[0] - step)).abs() < 0.01, "{padded:?}");
        assert!((padded[2] - (bare[2] + step * 2.0)).abs() < 0.01, "{padded:?}");
    }

    #[test]
    fn squaring_keeps_the_middle_where_it_was() {
        // 横長の形。正方形に揃えても中心は動かない。
        let wide = vec![at(0.0, 0.0), at(100.0, 0.0), at(100.0, 20.0)];

        let plain = cover(&wide, RESOLUTION, 0.0, false);
        let squared = cover(&wide, RESOLUTION, 0.0, true);

        assert!((squared[2] - squared[3]).abs() < 0.01, "正方形でない {squared:?}");
        assert!(squared[2] >= plain[2], "縮んでいる");

        let middle = |bounds: [f32; 4]| [bounds[0] + bounds[2] / 2.0, bounds[1] + bounds[3] / 2.0];

        let (before, after) = (middle(plain), middle(squared));
        assert!((before[0] - after[0]).abs() < 0.01 && (before[1] - after[1]).abs() < 0.01);
    }

    #[test]
    fn an_empty_shape_covers_nothing() {
        assert_eq!(cover(&[], RESOLUTION, 1.0, false), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_flattened_shape_still_gets_a_width() {
        // 高さ 0 の形。0 で割らずに、1 升ぶんの厚みを持たせる。
        let flat = vec![at(0.0, 50.0), at(100.0, 50.0), at(50.0, 50.0)];
        let bounds = cover(&flat, RESOLUTION, 0.0, false);

        assert!(bounds[2] > 0.0 && bounds[3] > 0.0, "{bounds:?}");
    }

    // ---- 覆う割合を焼く ----

    #[test]
    fn coverage_is_solid_inside_and_empty_outside() {
        let triangles = square(20.0, 80.0);
        let bounds = [0.0, 0.0, 100.0, 100.0];
        let pixels = rasterise(&triangles, bounds, RESOLUTION);

        let inside = cell(bounds, RESOLUTION, 50.0, 50.0);
        let outside = cell(bounds, RESOLUTION, 5.0, 5.0);

        assert_eq!(value(&pixels, RESOLUTION, inside.0, inside.1), 255);
        assert_eq!(value(&pixels, RESOLUTION, outside.0, outside.1), 0);
    }

    /// 対角線で 2 枚に割れていても、真ん中は隙間なく塗られる。
    #[test]
    fn the_seam_between_triangles_does_not_show() {
        let bounds = [0.0, 0.0, 100.0, 100.0];
        let pixels = rasterise(&square(0.0, 100.0), bounds, RESOLUTION);

        assert!(
            pixels.iter().all(|value| *value == 255),
            "塗り残しがある（いちばん薄いところ {}）",
            pixels.iter().min().unwrap(),
        );
    }

    #[test]
    fn the_edge_is_partly_covered() {
        // 升目の真ん中で縁を切ると、その升は半分だけ数えられる。
        let bounds = [0.0, 0.0, 64.0, 64.0];
        let pixels = rasterise(&square(0.0, 32.5), bounds, 64);

        let edge = value(&pixels, 64, 32, 10);

        assert!(
            (64..=192).contains(&edge),
            "縁が 0 か 255 に振り切れている（{edge}）",
        );
    }

    #[test]
    fn the_winding_does_not_matter() {
        let forward = rasterise(&square(20.0, 80.0), [0.0, 0.0, 100.0, 100.0], RESOLUTION);

        let mut backward = square(20.0, 80.0);
        backward.chunks_exact_mut(3).for_each(|triangle| triangle.swap(0, 2));

        assert_eq!(
            forward,
            rasterise(&backward, [0.0, 0.0, 100.0, 100.0], RESOLUTION),
        );
    }

    #[test]
    fn degenerate_input_bakes_nothing() {
        let bounds = [0.0, 0.0, 100.0, 100.0];

        for (triangles, note) in [
            (Vec::new(), "頂点なし"),
            (vec![at(0.0, 0.0), at(1.0, 1.0)], "三角形にならない"),
        ] {
            let pixels = rasterise(&triangles, bounds, RESOLUTION);
            assert!(pixels.iter().all(|value| *value == 0), "{note}");
        }

        for (bounds, note) in [
            ([0.0, 0.0, 0.0, 100.0], "幅がない"),
            ([0.0, 0.0, 100.0, f32::NAN], "高さが NaN"),
        ] {
            let pixels = rasterise(&square(20.0, 80.0), bounds, RESOLUTION);
            assert!(pixels.iter().all(|value| *value == 0), "{note}");
        }
    }

    // ---- 外周の辺を取り出す ----

    /// **これが距離を測る土台。** 中の対角線を境目と数えてしまうと、
    /// 形の奥が「縁のすぐそば」に見えて中身が削れる。
    #[test]
    fn the_inner_seam_is_not_a_boundary() {
        let edges = boundary(&square(0.0, 100.0));

        assert_eq!(edges.len(), 4, "四角の境目は 4 本のはず");

        for [from, to] in edges {
            let diagonal = (from[0] - to[0]).abs() > 0.01 && (from[1] - to[1]).abs() > 0.01;
            assert!(!diagonal, "対角線が残っている {from:?} -> {to:?}");
        }
    }

    #[test]
    fn a_hole_keeps_its_own_boundary() {
        // 外周 4 本 + 穴 4 本。橋は往復で消える。
        let mut triangles = square(0.0, 100.0);
        triangles.extend(square(40.0, 60.0));

        // 穴を重ねただけなので、境目は外と中で 8 本。
        assert_eq!(boundary(&triangles).len(), 8);
    }

    // ---- 距離を焼く ----

    #[test]
    fn the_boundary_sits_at_the_middle_value() {
        let bounds = cover(&square(20.0, 80.0), RESOLUTION, SPREAD, true);
        let (pixels, _) = distance_field(&square(20.0, 80.0), bounds, RESOLUTION);

        // 縁のすぐ内と外。0.5 = 128 を挟む。
        let inner = cell(bounds, RESOLUTION, 50.0, 22.0);
        let outer = cell(bounds, RESOLUTION, 50.0, 18.0);

        assert!(value(&pixels, RESOLUTION, inner.0, inner.1) > 128, "内が薄い");
        assert!(value(&pixels, RESOLUTION, outer.0, outer.1) < 128, "外が濃い");
    }

    /// 奥に行くほど値が上がる。対角線を境目と数えていたら、ここで崩れる。
    #[test]
    fn the_value_climbs_towards_the_middle() {
        let triangles = square(0.0, 100.0);
        let bounds = cover(&triangles, RESOLUTION, SPREAD, true);
        let (pixels, _) = distance_field(&triangles, bounds, RESOLUTION);

        let along = |y: f32| {
            let (column, row) = cell(bounds, RESOLUTION, 50.0, y);
            value(&pixels, RESOLUTION, column, row)
        };

        let steps = [along(2.0), along(10.0), along(25.0), along(50.0)];

        for pair in steps.windows(2) {
            assert!(pair[1] >= pair[0], "奥ほど濃くならない {steps:?}");
        }
        assert!(steps[3] > steps[0], "真ん中と縁が同じ {steps:?}");
    }

    #[test]
    fn the_distance_stops_climbing_past_the_spread() {
        let triangles = square(0.0, 100.0);
        let bounds = cover(&triangles, RESOLUTION, SPREAD, true);
        let (pixels, spread) = distance_field(&triangles, bounds, RESOLUTION);

        // 真ん中は境目からうんと遠いので、頭打ちの 255 に張り付く。
        let (column, row) = cell(bounds, RESOLUTION, 50.0, 50.0);
        assert_eq!(value(&pixels, RESOLUTION, column, row), 255);

        // 外も同じように 0 で止まる。
        assert_eq!(value(&pixels, RESOLUTION, 0, 0), 0);

        // 広がりは「何画素ぶんか」をワールドの長さに直したもの。
        let step = bounds[2] / RESOLUTION as f32;
        assert!((spread - SPREAD * step).abs() < 0.01, "{spread}");
    }

    /// 焼いた値を距離に戻すと、本当の距離と合う。シェーダがやるのと同じ式。
    #[test]
    fn the_stored_value_decodes_back_to_the_distance() {
        let triangles = square(0.0, 100.0);
        let bounds = cover(&triangles, RESOLUTION, SPREAD, true);
        let (pixels, spread) = distance_field(&triangles, bounds, RESOLUTION);

        let step = bounds[2] / RESOLUTION as f32;

        // 下の縁から内側へ、距離が分かっているところを見る。
        for depth in [1.0, 2.0, 4.0] {
            let y = depth * step;
            let (column, row) = cell(bounds, RESOLUTION, 50.0, y);
            let stored = value(&pixels, RESOLUTION, column, row) as f32 / 255.0;

            // シェーダと同じ戻し方。中を負とする符号付き距離。
            let decoded = (0.5 - stored) * 2.0 * spread;

            // 升目の真ん中で測るので、半升ぶんずれる。
            let want = -(y + step / 2.0 - 0.0);

            assert!(
                (decoded - want).abs() < step,
                "深さ {depth} 升で {decoded}、{want} のはず",
            );
        }
    }

    #[test]
    fn a_shape_with_no_area_bakes_as_all_outside() {
        let bounds = [0.0, 0.0, 100.0, 100.0];
        let (pixels, spread) = distance_field(&[], bounds, RESOLUTION);

        assert!(pixels.iter().all(|value| *value == 0));
        assert!(spread > 0.0, "広がりは 0 割りせずに返るはず");
    }

    // ---- 山の組み立て ----

    #[test]
    fn a_coverage_mask_builds_a_coverage_block() {
        let mask = ClipMask::new(3, [10.0, 20.0, 40.0, 80.0], ClipMaskKind::Coverage, 0.0);

        assert_eq!(
            mask.block(),
            Block::ClipMask {
                min: [10.0, 20.0],
                size: [40.0, 80.0],
                layer: 3,
                invert: false,
            },
        );
    }

    #[test]
    fn a_distance_mask_builds_a_distance_block() {
        let mask = ClipMask::new(1, [0.0, 0.0, 64.0, 64.0], ClipMaskKind::Distance, 8.0);

        assert_eq!(
            mask.block_with(2.5, true),
            Block::ClipDistanceMask {
                min: [0.0, 0.0],
                size: [64.0, 64.0],
                layer: 1,
                spread: 8.0,
                softness: 2.5,
                invert: true,
            },
        );
    }

    /// ぼかしは距離にしか渡らない。覆う割合には入れる先が無い。
    #[test]
    fn softness_only_reaches_the_distance_block() {
        let coverage = ClipMask::new(0, [0.0, 0.0, 10.0, 10.0], ClipMaskKind::Coverage, 0.0);

        assert_eq!(coverage.block_with(9.0, false), coverage.block());
    }

    #[test]
    fn inverting_only_moves_the_flag() {
        let mask = ClipMask::new(2, [0.0, 0.0, 10.0, 10.0], ClipMaskKind::Coverage, 0.0);

        let Block::ClipMask { invert, layer, .. } = mask.block_inverted(true) else {
            panic!("覆いの山にならなかった");
        };

        assert!(invert);
        assert_eq!(layer, 2);
    }

    // ---- GPU に渡す形 ----

    /// **寸法に sRGB を掛けてはいけない。** 色と同じ入れ物を使っているので、
    /// うっかり通すと丸みもぼかし幅も反転の印も壊れる。
    #[test]
    fn dimensions_are_not_treated_as_colour() {
        let raw = Block::ClipRect {
            min: [1.0, 2.0],
            max: [3.0, 4.0],
            radius: 0.5,
            softness: 0.25,
            invert: true,
        }
        .to_raw();

        assert_eq!(raw.params, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(raw.color_a[0], 0.5, "丸みが変換された");
        assert_eq!(raw.color_a[1], 0.25, "ぼかし幅が変換された");
        assert_eq!(raw.color_a[2], 1.0, "反転の印が変換された");
    }

    #[test]
    fn the_mask_size_is_sent_as_its_reciprocal() {
        // 画素ごとの割り算を省くため、逆数にして渡す。
        let raw = Block::ClipMask {
            min: [5.0, 7.0],
            size: [4.0, 8.0],
            layer: 6,
            invert: false,
        }
        .to_raw();

        assert_eq!(raw.params, [5.0, 7.0, 0.25, 0.125]);
        assert_eq!(raw.color_a[0], 6.0, "層が載っていない");
        assert_eq!(raw.color_a[1], 0.0);
    }

    #[test]
    fn a_zero_sized_mask_does_not_divide_by_zero() {
        let raw = Block::ClipMask {
            min: [0.0, 0.0],
            size: [0.0, 0.0],
            layer: 0,
            invert: false,
        }
        .to_raw();

        assert_eq!(raw.params[2], 0.0);
        assert_eq!(raw.params[3], 0.0);
    }

    #[test]
    fn the_distance_block_carries_its_spread_and_softness() {
        let raw = Block::ClipDistanceMask {
            min: [0.0, 0.0],
            size: [2.0, 2.0],
            layer: 9,
            spread: 12.0,
            softness: 3.0,
            invert: true,
        }
        .to_raw();

        assert_eq!(raw.color_a, [9.0, 1.0, 12.0, 3.0]);
    }

    #[test]
    fn every_clip_block_runs_in_the_colour_stage() {
        let blocks = [
            Block::ClipRect {
                min: [0.0; 2],
                max: [1.0; 2],
                radius: 0.0,
                softness: 0.0,
                invert: false,
            },
            Block::ClipEllipse {
                center: [0.0; 2],
                radius_x: 1.0,
                radius_y: 1.0,
                softness: 0.0,
                invert: false,
            },
            Block::ClipHalfPlane {
                normal: [1.0, 0.0],
                distance: 0.0,
                softness: 0.0,
                invert: false,
            },
            Block::ClipMask {
                min: [0.0; 2],
                size: [1.0; 2],
                layer: 0,
                invert: false,
            },
            Block::ClipDistanceMask {
                min: [0.0; 2],
                size: [1.0; 2],
                layer: 0,
                spread: 1.0,
                softness: 0.0,
                invert: false,
            },
        ];

        for block in blocks {
            assert_eq!(block.stage(), crate::effect::EffectStage::Color, "{block:?}");
        }
    }

    /// 番号がぶつかるとシェーダが別の山として解く。
    #[test]
    fn the_clip_blocks_have_their_own_numbers() {
        let numbers = [
            Block::ClipRect {
                min: [0.0; 2],
                max: [1.0; 2],
                radius: 0.0,
                softness: 0.0,
                invert: false,
            }
            .to_raw()
            .kind,
            Block::ClipEllipse {
                center: [0.0; 2],
                radius_x: 1.0,
                radius_y: 1.0,
                softness: 0.0,
                invert: false,
            }
            .to_raw()
            .kind,
            Block::ClipHalfPlane {
                normal: [1.0, 0.0],
                distance: 0.0,
                softness: 0.0,
                invert: false,
            }
            .to_raw()
            .kind,
            Block::ClipMask {
                min: [0.0; 2],
                size: [1.0; 2],
                layer: 0,
                invert: false,
            }
            .to_raw()
            .kind,
            Block::ClipDistanceMask {
                min: [0.0; 2],
                size: [1.0; 2],
                layer: 0,
                spread: 1.0,
                softness: 0.0,
                invert: false,
            }
            .to_raw()
            .kind,
        ];

        assert_eq!(numbers, [9, 10, 11, 12, 13]);

        // 自前の山の領分は侵さない。
        for number in numbers {
            assert!(number < crate::effect::CUSTOM_KIND_BASE);
        }
    }
}
