//! フォントを読んで、グリフの輪郭を取り出す。
//!
//! # 方針: 絵にせず、形にする
//!
//! 文字の描き方は大きく 2 つあります。
//!
//! - **絵にする**: グリフをテクスチャに焼いて、四角に貼る（ビットマップ / SDF）
//! - **形にする**: 輪郭を三角形に開いて、他の図形と同じように描く
//!
//! ここは後者です。理由は、このクレートが**すでに持っているものだけで済む**ためです。
//!
//! - 輪郭を三角形に開くのは [`crate::tessellate`]。穴（`o` `a` `e`）も、
//!   離れた輪郭（`i` `=` `%`）も、そのまま扱えます
//! - 同じ文字は形を 1 つだけ登録して、出てくるたびにインスタンスを足す。
//!   **`e` が 50 回出ても、形は 1 つ・インスタンスが 50 個**で、
//!   `multi_draw_indirect` 1 回のまま描けます
//! - 拡大しても崩れません。テクスチャに焼くと寸法が固定されます
//!
//! 代わりに、**小さい文字はギザギザになります**（アンチエイリアスが無いため）。
//! UI の 12〜16 px を綺麗に出すには、MSAA を入れるか SDF に切り替える必要が
//! あります。大きく出す文字はこちらのほうが綺麗です。
//!
//! # 座標系
//!
//! 取り出す輪郭は **em を 1.0 とした座標**です。フォントの単位に依存しないので、
//! 大きさは掛け算だけで決まります。y は**下向き**（このクレートのカメラに合わせて
//! 反転済み）なので、そのまま置けば上下が正しく出ます。

use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::error::Gueiz2DError;

/// 曲線を折れ線に開くときの既定の許容誤差。em に対する割合。
///
/// 1/1000 em なら、100 px で描いても 0.1 px のずれ。目には見えない。
pub const DEFAULT_TOLERANCE: f32 = 0.001;

/// 1 本の曲線を割る上限。
///
/// 既定の許容なら、em いっぱいの大きな曲線でも 20 本前後で足ります。
/// ここに当たるのは、許容を極端に厳しくしたときだけ。
/// 上限があるのは、変な入力で頂点が爆発しないようにするため。
const MAX_STEPS: usize = 256;

/// 読み込んだフォント。
///
/// バイト列は**呼ぶ側が持ちます**。フォントは大きいので、勝手に複製しません。
///
/// ```no_run
/// # use gueiz_2d::font::Font;
/// # fn run() -> Result<(), gueiz_2d::error::Gueiz2DError> {
/// let data = std::fs::read("C:/Windows/Fonts/arial.ttf").unwrap();
/// let font = Font::from_bytes(&data)?;
///
/// println!("1 行の高さ: {} em", font.line_height());
/// # Ok(())
/// # }
/// ```
pub struct Font<'a> {
    face: Face<'a>,
    /// フォント単位から em への換算。
    to_em: f32,
}

impl<'a> Font<'a> {
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, Gueiz2DError> {
        let face = Face::parse(data, 0).map_err(|error| {
            Gueiz2DError::FontParseError(error.to_string())
        })?;

        let units_per_em = face.units_per_em();

        if units_per_em == 0 {
            return Err(Gueiz2DError::FontParseError(String::from(
                "units_per_em is zero",
            )));
        }

        Ok(Self {
            face,
            to_em: 1.0 / units_per_em as f32,
        })
    }

    /// ベースラインから上の高さ（em）。
    pub fn ascender(&self) -> f32 {
        self.face.ascender() as f32 * self.to_em
    }

    /// ベースラインから下の深さ（em）。下向きが正。
    pub fn descender(&self) -> f32 {
        -self.face.descender() as f32 * self.to_em
    }

    /// 行を送る量（em）。
    pub fn line_height(&self) -> f32 {
        (self.face.ascender() as f32 - self.face.descender() as f32
            + self.face.line_gap() as f32)
            * self.to_em
    }

    /// 下線を引く高さ（em）。ベースラインからの**下向き**の距離。
    ///
    /// フォントが持っていなければ、見た目が破綻しない値に落とす。
    pub fn underline_position(&self) -> f32 {
        self.face
            .underline_metrics()
            // フォントは上向きに持っているので符号を返す。
            .map_or(0.1, |metrics| -metrics.position as f32 * self.to_em)
    }

    /// 打消し線を引く高さ（em）。ベースラインからの**下向き**の距離。
    /// x ハイトの真ん中あたりなので、ふつうは負。
    pub fn strikeout_position(&self) -> f32 {
        self.face
            .strikeout_metrics()
            .map_or(-0.26, |metrics| -metrics.position as f32 * self.to_em)
    }

    /// 上線を引く高さ（em）。ベースラインからの**下向き**の距離。
    ///
    /// フォントは持っていないので、いちばん高いところ（アセンダ）に引く。
    pub fn overline_position(&self) -> f32 {
        -self.ascender()
    }

    /// フォントが勧める線の太さ（em）。無ければ [`DEFAULT_LINE_WIDTH`]。
    ///
    /// [`DEFAULT_LINE_WIDTH`]: crate::format::DEFAULT_LINE_WIDTH
    pub fn line_thickness(&self) -> f32 {
        self.face
            .underline_metrics()
            .map(|metrics| metrics.thickness as f32 * self.to_em)
            .filter(|thickness| *thickness > 0.0)
            .unwrap_or(crate::format::DEFAULT_LINE_WIDTH)
    }

    /// その文字のグリフ。無ければ `None`。
    pub fn glyph(&self, character: char) -> Option<GlyphId> {
        self.face.glyph_index(character)
    }

    /// 次の文字までどれだけ進むか（em）。
    pub fn advance(&self, glyph: GlyphId) -> f32 {
        self.face
            .glyph_hor_advance(glyph)
            .map_or(0.0, |advance| advance as f32 * self.to_em)
    }

    /// 2 文字の詰め（em）。`AV` のような組で効く。無ければ 0。
    pub fn kerning(&self, left: GlyphId, right: GlyphId) -> f32 {
        let Some(table) = self.face.tables().kern else {
            return 0.0;
        };

        table
            .subtables
            .into_iter()
            .filter(|subtable| subtable.horizontal && !subtable.variable)
            .find_map(|subtable| subtable.glyphs_kerning(left, right))
            .map_or(0.0, |value| value as f32 * self.to_em)
    }

    /// グリフの輪郭を折れ線にして返す。**em 単位・y は下向き。**
    ///
    /// 空白のように形を持たないグリフは `None`。
    pub fn outline(&self, glyph: GlyphId, tolerance: f32) -> Option<GlyphOutline> {
        let mut builder = OutlineFlattener::new(self.to_em, tolerance.max(1e-6));
        self.face.outline_glyph(glyph, &mut builder)?;
        builder.finish()
    }
}

/// 折れ線にしたグリフの輪郭。
///
/// [`crate::tessellate::tessellate`] にそのまま渡せる形。
#[derive(Clone)]
#[derive(Debug, Default)]
pub struct GlyphOutline {
    /// 全輪郭をつないだ点。
    pub points: Vec<[f32; 2]>,
    /// 各輪郭の開始位置。
    pub contour_starts: Vec<usize>,
}

impl GlyphOutline {
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn contour_count(&self) -> usize {
        self.contour_starts.len()
    }
}

/// `ttf-parser` の輪郭を受け取って、曲線を折れ線に開く。
struct OutlineFlattener {
    to_em: f32,
    tolerance: f32,
    points: Vec<[f32; 2]>,
    contour_starts: Vec<usize>,
    current: [f32; 2],
}

impl OutlineFlattener {
    fn new(to_em: f32, tolerance: f32) -> Self {
        Self {
            to_em,
            tolerance,
            points: Vec::new(),
            contour_starts: Vec::new(),
            current: [0.0, 0.0],
        }
    }

    /// フォント単位から em へ。**y はここで反転する。**
    ///
    /// フォントは y が上向き、このクレートのカメラは下向き。ここで合わせておけば
    /// 呼ぶ側が意識しなくて済む。巻き方向も裏返るが、テッセレータが
    /// 包含関係で外周と穴を決めるので影響しない。
    fn to_local(&self, x: f32, y: f32) -> [f32; 2] {
        [x * self.to_em, -y * self.to_em]
    }

    fn push(&mut self, point: [f32; 2]) {
        // 同じ場所が続くと、長さ 0 の辺になってテッセレータを困らせる。
        if let Some(last) = self.points.last() {
            let dx = point[0] - last[0];
            let dy = point[1] - last[1];

            if dx * dx + dy * dy < 1e-14 {
                return;
            }
        }

        self.points.push(point);
        self.current = point;
    }

    fn finish(mut self) -> Option<GlyphOutline> {
        // 3 点に満たない輪郭は形にならないので落とす。
        self.drop_degenerate_contours();

        if self.points.is_empty() {
            return None;
        }

        Some(GlyphOutline {
            points: self.points,
            contour_starts: self.contour_starts,
        })
    }

    fn drop_degenerate_contours(&mut self) {
        let mut kept_points = Vec::with_capacity(self.points.len());
        let mut kept_starts = Vec::with_capacity(self.contour_starts.len());

        for (index, &start) in self.contour_starts.iter().enumerate() {
            let end = self
                .contour_starts
                .get(index + 1)
                .copied()
                .unwrap_or(self.points.len());

            if end - start < 3 {
                continue;
            }

            kept_starts.push(kept_points.len());
            kept_points.extend_from_slice(&self.points[start..end]);
        }

        self.points = kept_points;
        self.contour_starts = kept_starts;
    }
}

impl OutlineBuilder for OutlineFlattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contour_starts.push(self.points.len());
        let point = self.to_local(x, y);
        // 新しい輪郭の 1 点目は、前の輪郭の終点と重なっていても必要。
        self.points.push(point);
        self.current = point;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let point = self.to_local(x, y);
        self.push(point);
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        let start = self.current;
        let control = self.to_local(control_x, control_y);
        let end = self.to_local(x, y);

        for step in 1..=quadratic_steps(start, control, end, self.tolerance) {
            let t = step as f32 / quadratic_steps(start, control, end, self.tolerance) as f32;
            self.push(quadratic_at(start, control, end, t));
        }
    }

    fn curve_to(
        &mut self,
        first_x: f32,
        first_y: f32,
        second_x: f32,
        second_y: f32,
        x: f32,
        y: f32,
    ) {
        let start = self.current;
        let first = self.to_local(first_x, first_y);
        let second = self.to_local(second_x, second_y);
        let end = self.to_local(x, y);

        let steps = cubic_steps(start, first, second, end, self.tolerance);

        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            self.push(cubic_at(start, first, second, end, t));
        }
    }

    fn close(&mut self) {
        // 終点は始点に戻るが、閉じた輪郭として扱うので点は足さない。
        // 足すと長さ 0 の辺ができる。
    }
}

/// 2 次ベジェを何本の直線に割るか。
///
/// 2 階微分は一定で `2|p0 - 2p1 + p2|`。`n` 等分したときの弦からのずれは
/// `|B''| / (8 n^2)` を超えないので、`|D| / (4 n^2) <= 許容` を満たす `n` を取る。
fn quadratic_steps(start: [f32; 2], control: [f32; 2], end: [f32; 2], tolerance: f32) -> usize {
    let deviation_x = start[0] - 2.0 * control[0] + end[0];
    let deviation_y = start[1] - 2.0 * control[1] + end[1];
    let deviation = (deviation_x * deviation_x + deviation_y * deviation_y).sqrt();

    let steps = (deviation / (4.0 * tolerance)).sqrt().ceil();

    (steps as usize).clamp(1, MAX_STEPS)
}

fn quadratic_at(start: [f32; 2], control: [f32; 2], end: [f32; 2], t: f32) -> [f32; 2] {
    let inverse = 1.0 - t;
    let a = inverse * inverse;
    let b = 2.0 * inverse * t;
    let c = t * t;

    [
        a * start[0] + b * control[0] + c * end[0],
        a * start[1] + b * control[1] + c * end[1],
    ]
}

/// 3 次ベジェを何本の直線に割るか。2 つの 2 階差分の大きいほうで見積もる。
fn cubic_steps(
    start: [f32; 2],
    first: [f32; 2],
    second: [f32; 2],
    end: [f32; 2],
    tolerance: f32,
) -> usize {
    let deviation = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        let x = a[0] - 2.0 * b[0] + c[0];
        let y = a[1] - 2.0 * b[1] + c[1];
        (x * x + y * y).sqrt()
    };

    let worst = deviation(start, first, second).max(deviation(first, second, end));
    let steps = (worst * 3.0 / (4.0 * tolerance)).sqrt().ceil();

    (steps as usize).clamp(1, MAX_STEPS)
}

fn cubic_at(
    start: [f32; 2],
    first: [f32; 2],
    second: [f32; 2],
    end: [f32; 2],
    t: f32,
) -> [f32; 2] {
    let inverse = 1.0 - t;
    let a = inverse * inverse * inverse;
    let b = 3.0 * inverse * inverse * t;
    let c = 3.0 * inverse * t * t;
    let d = t * t * t;

    [
        a * start[0] + b * first[0] + c * second[0] + d * end[0],
        a * start[1] + b * first[1] + c * second[1] + d * end[1],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 折れ線が本物の曲線からどれだけ離れているか。
    ///
    /// 曲線を細かく刻んで、いちばん近い線分までの距離を測る。
    fn max_error(
        polyline: &[[f32; 2]],
        curve: impl Fn(f32) -> [f32; 2],
        samples: usize,
    ) -> f32 {
        let mut worst = 0.0_f32;

        for sample in 0..=samples {
            let t = sample as f32 / samples as f32;
            let point = curve(t);

            let mut nearest = f32::INFINITY;

            for pair in polyline.windows(2) {
                nearest = nearest.min(distance_to_segment(point, pair[0], pair[1]));
            }

            worst = worst.max(nearest);
        }

        worst
    }

    fn distance_to_segment(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
        let dx = end[0] - start[0];
        let dy = end[1] - start[1];
        let length_squared = dx * dx + dy * dy;

        let t = if length_squared < f32::EPSILON {
            0.0
        } else {
            (((point[0] - start[0]) * dx + (point[1] - start[1]) * dy) / length_squared)
                .clamp(0.0, 1.0)
        };

        let nearest_x = start[0] + dx * t;
        let nearest_y = start[1] + dy * t;

        ((point[0] - nearest_x).powi(2) + (point[1] - nearest_y).powi(2)).sqrt()
    }

    /// 刻みの数は、許容誤差を実際に満たしていないと意味がない。
    #[test]
    fn a_quadratic_stays_within_the_tolerance() {
        let start = [0.0, 0.0];
        let control = [0.5, 1.0];
        let end = [1.0, 0.0];

        for tolerance in [0.1, 0.01, 0.001, 0.0001] {
            let steps = quadratic_steps(start, control, end, tolerance);
            let polyline: Vec<[f32; 2]> = (0..=steps)
                .map(|step| quadratic_at(start, control, end, step as f32 / steps as f32))
                .collect();

            let error = max_error(&polyline, |t| quadratic_at(start, control, end, t), 512);

            assert!(
                error <= tolerance * 1.05,
                "許容 {tolerance} に対して誤差 {error}（{steps} 分割）",
            );
        }
    }

    #[test]
    fn a_cubic_stays_within_the_tolerance() {
        let start = [0.0, 0.0];
        let first = [0.0, 1.0];
        let second = [1.0, 1.0];
        let end = [1.0, 0.0];

        for tolerance in [0.1, 0.01, 0.001, 0.0001] {
            let steps = cubic_steps(start, first, second, end, tolerance);
            let polyline: Vec<[f32; 2]> = (0..=steps)
                .map(|step| cubic_at(start, first, second, end, step as f32 / steps as f32))
                .collect();

            let error = max_error(&polyline, |t| cubic_at(start, first, second, end, t), 512);

            assert!(
                error <= tolerance * 1.05,
                "許容 {tolerance} に対して誤差 {error}（{steps} 分割）",
            );
        }
    }

    /// まっすぐな「曲線」を細かく刻むのは無駄。
    ///
    /// 制御点は等間隔にする。同じ直線でも間隔が偏っていると 2 階差分が
    /// 残るので、見積もりは刻みを増やす（余分なだけで害は無い）。
    #[test]
    fn a_straight_curve_needs_one_segment() {
        assert_eq!(quadratic_steps([0.0, 0.0], [0.5, 0.0], [1.0, 0.0], 0.001), 1);
        assert_eq!(
            cubic_steps([0.0, 0.0], [1.0 / 3.0, 0.0], [2.0 / 3.0, 0.0], [1.0, 0.0], 0.001),
            1,
        );
    }

    /// 許容を厳しくすれば刻みは増える。緩めれば減る。
    #[test]
    fn a_tighter_tolerance_uses_more_segments() {
        let coarse = quadratic_steps([0.0, 0.0], [0.5, 1.0], [1.0, 0.0], 0.01);
        let fine = quadratic_steps([0.0, 0.0], [0.5, 1.0], [1.0, 0.0], 0.0001);

        assert!(fine > coarse, "{fine} <= {coarse}");
    }

    /// 端点はそのまま通る。ずれると文字がつながらない。
    #[test]
    fn the_endpoints_are_exact() {
        assert_eq!(quadratic_at([1.0, 2.0], [3.0, 4.0], [5.0, 6.0], 0.0), [1.0, 2.0]);
        assert_eq!(quadratic_at([1.0, 2.0], [3.0, 4.0], [5.0, 6.0], 1.0), [5.0, 6.0]);
        assert_eq!(
            cubic_at([1.0, 2.0], [3.0, 4.0], [5.0, 6.0], [7.0, 8.0], 0.0),
            [1.0, 2.0],
        );
        assert_eq!(
            cubic_at([1.0, 2.0], [3.0, 4.0], [5.0, 6.0], [7.0, 8.0], 1.0),
            [7.0, 8.0],
        );
    }

    /// 輪郭を組み立てる側。フォントが無くても試せる。
    #[test]
    fn the_flattener_splits_contours_and_flips_y() {
        // units_per_em = 1000 とみなす換算。
        let mut builder = OutlineFlattener::new(0.001, DEFAULT_TOLERANCE);

        builder.move_to(0.0, 0.0);
        builder.line_to(500.0, 0.0);
        builder.line_to(500.0, 500.0);
        builder.close();

        builder.move_to(100.0, 100.0);
        builder.line_to(200.0, 100.0);
        builder.line_to(200.0, 200.0);
        builder.close();

        let outline = builder.finish().expect("2 輪郭ぶんある");

        assert_eq!(outline.contour_count(), 2);
        assert_eq!(outline.contour_starts, vec![0, 3]);
        assert_eq!(outline.points.len(), 6);

        // em 単位になっていること。
        assert_eq!(outline.points[1], [0.5, 0.0]);
        // y が反転していること。フォントは上向き、こちらは下向き。
        assert_eq!(outline.points[2], [0.5, -0.5]);
    }

    /// 3 点に満たない輪郭は形にならないので落とす。
    /// 残すと、後段が長さ 0 の辺を掴む。
    #[test]
    fn degenerate_contours_are_dropped() {
        let mut builder = OutlineFlattener::new(0.001, DEFAULT_TOLERANCE);

        builder.move_to(0.0, 0.0);
        builder.line_to(100.0, 0.0);
        builder.close();

        builder.move_to(0.0, 0.0);
        builder.line_to(500.0, 0.0);
        builder.line_to(500.0, 500.0);
        builder.close();

        let outline = builder.finish().expect("1 輪郭は残る");

        assert_eq!(outline.contour_count(), 1);
        assert_eq!(outline.points.len(), 3);
    }

    #[test]
    fn an_empty_outline_is_none() {
        let builder = OutlineFlattener::new(0.001, DEFAULT_TOLERANCE);

        assert!(builder.finish().is_none());
    }

    /// 同じ場所が続いても点を増やさない。長さ 0 の辺は後段を困らせる。
    #[test]
    fn repeated_points_are_collapsed() {
        let mut builder = OutlineFlattener::new(0.001, DEFAULT_TOLERANCE);

        builder.move_to(0.0, 0.0);
        builder.line_to(0.0, 0.0);
        builder.line_to(500.0, 0.0);
        builder.line_to(500.0, 0.0);
        builder.line_to(500.0, 500.0);
        builder.close();

        let outline = builder.finish().expect("形になる");

        assert_eq!(outline.points.len(), 3);
    }
}
