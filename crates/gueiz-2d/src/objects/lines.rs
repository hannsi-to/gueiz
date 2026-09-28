use std::marker::PhantomData;
use gueiz_gpu::vertex::Vertex;
use crate::object::{create_object, Object};
use crate::paint_type::PaintType;

pub struct Lines<State> {
    object: Object,
    line_data: LineData,
    _state: PhantomData<State>,
}

#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub enum CurveRepresentationType {
    /// 曲げない。置かれた制御点をそのまま結ぶ折れ線。
    /// 刻まないので `segment_count` は効かない。
    #[default]
    Normal,
    /// 制御点全部で 1 本のベジェ曲線。両端だけを通る。
    BezierCurve,
    /// 一様な B スプライン。`degree` で滑らかさが決まる。両端だけを通る。
    BSpline,
    /// 重み付きの B スプライン。`point_weighted` で重みを指定する。
    NURBS,
    /// 制御点をすべて通る。`tension` で張りを決める。
    CatmullRomSpline,
    /// 区間ごとの 3 次エルミート。接線は `point_tangent` で指定できる。
    HermiteCurve,
    /// 制御点をすべて通る 1 本の多項式（ラグランジュ補間）。
    /// 点を増やすと端が激しく波打つ。
    PolynomialCurve,
    /// 曲率が長さに比例して変わるクロソイド曲線。
    /// 最初の点から、2 番目の点の向きに、折れ線の総長ぶん伸びる。
    /// 形は `curvature` と `curvature_rate` で決まる。
    ClothoidCurve,
}

struct PointData {
    vertex: Vertex,
    weight: f32,
    tangent: Option<(f32, f32)>,
}

struct LineData {
    curve_type: CurveRepresentationType,
    points: Vec<PointData>,
    segment_count: usize,
    degree: usize,
    tension: f32,
    closed: bool,
    curvature: f32,
    curvature_rate: f32,
}

pub struct Init;
pub struct PaintTypeSet;
pub struct CurveTypeSet;
pub struct End;

impl<State> Lines<State> {
    fn advance<Next>(self) -> Lines<Next> {
        Lines {
            object: self.object,
            line_data: self.line_data,
            _state: PhantomData,
        }
    }

    fn push(&mut self, vertex: Vertex, weight: f32, tangent: Option<(f32, f32)>) {
        self.line_data.points.push(PointData {
            vertex,
            weight,
            tangent,
        });
    }
}

impl Lines<Init> {
    pub fn new(name: &str) -> Self {
        Self {
            object: create_object(name),
            line_data: LineData {
                curve_type: CurveRepresentationType::default(),
                points: Vec::new(),
                segment_count: 64,
                degree: 3,
                tension: 0.5,
                closed: false,
                curvature: 0.0,
                curvature_rate: 0.0,
            },
            _state: PhantomData,
        }
    }

    pub fn paint_type(mut self, paint_type: PaintType) -> Lines<PaintTypeSet> {
        self.object.begin(paint_type);
        self.advance()
    }
}

impl Lines<PaintTypeSet> {
    pub fn curve_type(mut self, curve_type: CurveRepresentationType) -> Lines<CurveTypeSet> {
        self.line_data.curve_type = curve_type;
        self.advance()
    }
}

impl Lines<CurveTypeSet> {
    /// 曲線全体をいくつに刻むか。
    pub fn segment_count(mut self, segment_count: usize) -> Lines<CurveTypeSet> {
        self.line_data.segment_count = segment_count;
        self.advance()
    }

    /// B スプラインと NURBS の次数。制御点の数より小さくないと下げられる。
    pub fn degree(mut self, degree: usize) -> Lines<CurveTypeSet> {
        self.line_data.degree = degree;
        self.advance()
    }

    /// カトマル・ロムの張り。0.5 が標準、0 で折れ線になる。
    pub fn tension(mut self, tension: f32) -> Lines<CurveTypeSet> {
        self.line_data.tension = tension;
        self.advance()
    }

    /// 輪にする。カトマル・ロムと B スプラインと NURBS でだけ効く。
    pub fn closed(mut self, closed: bool) -> Lines<CurveTypeSet> {
        self.line_data.closed = closed;
        self.advance()
    }

    /// クロソイドの始まりの曲率。
    pub fn curvature(mut self, curvature: f32) -> Lines<CurveTypeSet> {
        self.line_data.curvature = curvature;
        self.advance()
    }

    /// クロソイドの曲率が長さ 1 あたりに変わる量。
    pub fn curvature_rate(mut self, curvature_rate: f32) -> Lines<CurveTypeSet> {
        self.line_data.curvature_rate = curvature_rate;
        self.advance()
    }

    pub fn point(mut self, vertex: Vertex) -> Lines<CurveTypeSet> {
        self.push(vertex, 1.0, None);
        self.advance()
    }

    /// NURBS の重み付きの制御点。重いほど曲線がその点に寄る。
    pub fn point_weighted(mut self, vertex: Vertex, weight: f32) -> Lines<CurveTypeSet> {
        self.push(vertex, weight, None);
        self.advance()
    }

    /// エルミートの接線付きの制御点。
    pub fn point_tangent(
        mut self,
        vertex: Vertex,
        tangent_x: f32,
        tangent_y: f32,
    ) -> Lines<CurveTypeSet> {
        self.push(vertex, 1.0, Some((tangent_x, tangent_y)));
        self.advance()
    }

    pub fn last_point(mut self, vertex: Vertex) -> Lines<End> {
        self.push(vertex, 1.0, None);
        self.advance()
    }

    pub fn last_point_weighted(mut self, vertex: Vertex, weight: f32) -> Lines<End> {
        self.push(vertex, weight, None);
        self.advance()
    }

    pub fn last_point_tangent(
        mut self,
        vertex: Vertex,
        tangent_x: f32,
        tangent_y: f32,
    ) -> Lines<End> {
        self.push(vertex, 1.0, Some((tangent_x, tangent_y)));
        self.advance()
    }
}

impl Lines<End> {
    pub fn end(mut self) -> Object {
        for vertex in outline(&self.line_data) {
            self.object.put_vertex(vertex);
        }

        self.object.end();
        self.object
    }
}

/// 頂点 1 つを、重みを掛けた 13 個の数として持つ。
///
/// 混ぜる計算をすべてこの形で書くので、位置も色も uv も法線も同じ式で動く。
/// 最後の 1 つが重みで、取り出すときにそれで割る。重み 1 なら割り算は効かない。
const PARTS: usize = 13;
type Parts = [f32; PARTS];

fn homogeneous(point: &PointData) -> Parts {
    let vertex = point.vertex;
    let weight = if point.weight.is_finite() {
        point.weight
    } else {
        1.0
    };

    [
        vertex.x * weight,
        vertex.y * weight,
        vertex.z * weight,
        vertex.r * weight,
        vertex.g * weight,
        vertex.b * weight,
        vertex.a * weight,
        vertex.u * weight,
        vertex.v * weight,
        vertex.n_x * weight,
        vertex.n_y * weight,
        vertex.n_z * weight,
        weight,
    ]
}

fn project(parts: Parts) -> Vertex {
    let weight = if parts[PARTS - 1] == 0.0 {
        1.0
    } else {
        parts[PARTS - 1]
    };

    Vertex::new_position_color_uv_normal(
        parts[0] / weight,
        parts[1] / weight,
        parts[2] / weight,
        parts[3] / weight,
        parts[4] / weight,
        parts[5] / weight,
        parts[6] / weight,
        parts[7] / weight,
        parts[8] / weight,
        parts[9] / weight,
        parts[10] / weight,
        parts[11] / weight,
    )
}

fn mix(from: Parts, to: Parts, ratio: f32) -> Parts {
    let mut out = from;

    for index in 0..PARTS {
        out[index] = from[index] + (to[index] - from[index]) * ratio;
    }

    out
}

fn axpy(accumulator: &mut Parts, term: Parts, scale: f32) {
    for index in 0..PARTS {
        accumulator[index] += term[index] * scale;
    }
}

fn difference(from: Parts, to: Parts) -> Parts {
    let mut out = from;

    for index in 0..PARTS {
        out[index] = to[index] - from[index];
    }

    out
}

fn outline(line_data: &LineData) -> Vec<Vertex> {
    let control: Vec<Parts> = line_data.points.iter().map(homogeneous).collect();

    // 2 点に満たなければ曲線にならない。置かれた点をそのまま返す。
    if control.len() < 2 {
        return line_data.points.iter().map(|point| point.vertex).collect();
    }

    let segments = line_data.segment_count.max(1);

    let sampled = match line_data.curve_type {
        // 曲げないので刻まない。制御点がそのまま輪郭になる。
        CurveRepresentationType::Normal => control.clone(),

        CurveRepresentationType::BezierCurve => bezier(&control, segments),

        CurveRepresentationType::BSpline | CurveRepresentationType::NURBS => {
            bspline(&control, line_data.degree, line_data.closed, segments)
        }

        CurveRepresentationType::CatmullRomSpline => {
            hermite(&control, &tangents(&control, line_data, false), line_data.closed, segments)
        }

        CurveRepresentationType::HermiteCurve => {
            hermite(&control, &tangents(&control, line_data, true), line_data.closed, segments)
        }

        CurveRepresentationType::PolynomialCurve => lagrange(&control, segments),

        CurveRepresentationType::ClothoidCurve => clothoid(&control, line_data, segments),
    };

    sampled.into_iter().map(project).collect()
}

/// 刻んだ位置の比率。輪のときは最後が最初に重なるので置かない。
fn ratios(segments: usize, closed: bool) -> impl Iterator<Item = f32> {
    let points = if closed { segments } else { segments + 1 };

    (0..points).map(move |step| step as f32 / segments as f32)
}

/// ド・カステリョ。制御点を順に混ぜていくだけ。
fn bezier(control: &[Parts], segments: usize) -> Vec<Parts> {
    ratios(segments, false)
        .map(|ratio| {
            let mut working = control.to_vec();

            for round in 1..working.len() {
                for index in 0..working.len() - round {
                    working[index] = mix(working[index], working[index + 1], ratio);
                }
            }

            working[0]
        })
        .collect()
}

/// ネヴィルの算法によるラグランジュ補間。節は 0, 1, 2, ... と等間隔に取る。
fn lagrange(control: &[Parts], segments: usize) -> Vec<Parts> {
    let last = (control.len() - 1) as f32;

    ratios(segments, false)
        .map(|ratio| {
            let position = ratio * last;
            let mut working = control.to_vec();

            for round in 1..working.len() {
                for index in 0..working.len() - round {
                    let blend = (position - index as f32) / round as f32;
                    working[index] = mix(working[index], working[index + 1], blend);
                }
            }

            working[0]
        })
        .collect()
}

/// 区間ごとの接線。
///
/// `explicit` が真なら、指定された接線を位置の成分だけ上書きに使う。
/// 指定が無い点は、前後の差から取る（`tension` 倍したもの）。
fn tangents(control: &[Parts], line_data: &LineData, explicit: bool) -> Vec<Parts> {
    let count = control.len();

    (0..count)
        .map(|index| {
            let (before, after) = if line_data.closed {
                ((index + count - 1) % count, (index + 1) % count)
            } else {
                // 端は隣が片側しかない。
                (index.saturating_sub(1), (index + 1).min(count - 1))
            };

            // またいだ区間の数で割り戻す。端の片側だけの差は 1 区間ぶんしかないので、
            // そのまま使うと傾きが半分になって端がたるむ。
            let spanned = if line_data.closed {
                2.0
            } else {
                (after - before) as f32
            };

            let mut tangent = difference(control[before], control[after]);
            let scale = if spanned > 0.0 {
                line_data.tension * 2.0 / spanned
            } else {
                0.0
            };

            for part in &mut tangent {
                *part *= scale;
            }

            if explicit && let Some((tangent_x, tangent_y)) = line_data.points[index].tangent {
                tangent[0] = tangent_x;
                tangent[1] = tangent_y;
            }

            tangent
        })
        .collect()
}

/// 区間ごとの 3 次エルミート。カトマル・ロムもここを通る。
fn hermite(control: &[Parts], tangents: &[Parts], closed: bool, segments: usize) -> Vec<Parts> {
    let count = control.len();
    let spans = if closed { count } else { count - 1 };

    ratios(segments, closed)
        .map(|ratio| {
            let position = (ratio * spans as f32).min(spans as f32);
            let span = (position.floor() as usize).min(spans.saturating_sub(1));
            let local = position - span as f32;

            let from = span;
            let to = (span + 1) % count;

            let (square, cube) = (local * local, local * local * local);

            let mut point = [0.0; PARTS];
            axpy(&mut point, control[from], 2.0 * cube - 3.0 * square + 1.0);
            axpy(&mut point, tangents[from], cube - 2.0 * square + local);
            axpy(&mut point, control[to], -2.0 * cube + 3.0 * square);
            axpy(&mut point, tangents[to], cube - square);

            point
        })
        .collect()
}

/// ド・ブーアによる B スプライン。重みが入ったままなので NURBS も同じ式で出る。
fn bspline(control: &[Parts], degree: usize, closed: bool, segments: usize) -> Vec<Parts> {
    // 輪にするときは頭の制御点を尻に足して巻きつける。
    let degree = degree.clamp(1, control.len() - 1);

    let control: Vec<Parts> = if closed {
        control
            .iter()
            .chain(control.iter().take(degree))
            .copied()
            .collect()
    } else {
        control.to_vec()
    };

    let count = control.len();
    let knots = knot_vector(count, degree, closed);

    // 動かせる範囲。
    let (first, last) = (knots[degree], knots[count]);

    ratios(segments, closed)
        .map(|ratio| {
            let position = first + (last - first) * ratio;
            de_boor(&control, &knots, degree, position)
        })
        .collect()
}

/// 開いた曲線は両端に節を重ねて端の制御点を通し、輪は等間隔にする。
fn knot_vector(count: usize, degree: usize, closed: bool) -> Vec<f32> {
    if closed {
        return (0..count + degree + 1).map(|index| index as f32).collect();
    }

    (0..count + degree + 1)
        .map(|index| {
            if index <= degree {
                0.0
            } else if index >= count {
                (count - degree) as f32
            } else {
                (index - degree) as f32
            }
        })
        .collect()
}

fn de_boor(control: &[Parts], knots: &[f32], degree: usize, position: f32) -> Parts {
    let count = control.len();

    // 位置が入っている節の区間を探す。終端はひとつ手前の区間に寄せる。
    let mut span = degree;

    while span + 1 < count && knots[span + 1] <= position {
        span += 1;
    }

    let mut working: Vec<Parts> = (0..=degree)
        .map(|index| control[span + index - degree])
        .collect();

    for round in 1..=degree {
        for index in (round..=degree).rev() {
            let low = knots[index + span - degree];
            let high = knots[index + 1 + span - round];
            let width = high - low;

            let blend = if width > 0.0 {
                (position - low) / width
            } else {
                0.0
            };

            working[index] = mix(working[index - 1], working[index], blend);
        }
    }

    working[degree]
}

/// クロソイド曲線。向きが `curvature` から `curvature_rate` の割で変わっていく。
///
/// 始まりは最初の制御点、向きは 2 番目の制御点の方、長さは折れ線の総長。
/// 色や uv は、その折れ線に沿った長さで混ぜる。
fn clothoid(control: &[Parts], line_data: &LineData, segments: usize) -> Vec<Parts> {
    let spans: Vec<f32> = control
        .windows(2)
        .map(|pair| {
            let (from, to) = (project(pair[0]), project(pair[1]));
            ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt()
        })
        .collect();

    let total: f32 = spans.iter().sum();

    // NaN も弾きたいので、正であることを直接確かめる。
    if !(total.is_finite() && total > 0.0) {
        return control.to_vec();
    }

    let start = project(control[0]);
    let next = project(control[1]);
    let heading = (next.y - start.y).atan2(next.x - start.x);

    let step = total / segments as f32;

    let mut at = (start.x, start.y);
    let mut travelled = 0.0;
    let mut points = Vec::with_capacity(segments + 1);

    for sample in 0..=segments {
        let mut point = along(control, &spans, total, travelled);
        point[0] = at.0 * point[PARTS - 1];
        point[1] = at.1 * point[PARTS - 1];
        points.push(point);

        if sample == segments {
            break;
        }

        // 区間の真ん中の向きで進める。
        let middle = travelled + step / 2.0;
        let angle = heading
            + line_data.curvature * middle
            + line_data.curvature_rate * middle * middle / 2.0;

        at = (at.0 + angle.cos() * step, at.1 + angle.sin() * step);
        travelled += step;
    }

    points
}

/// 制御点の折れ線を、先頭からの長さで辿る。
fn along(control: &[Parts], spans: &[f32], total: f32, travelled: f32) -> Parts {
    let mut remaining = travelled.clamp(0.0, total);

    for (index, span) in spans.iter().enumerate() {
        if remaining <= *span || index + 1 == spans.len() {
            let blend = if *span > 0.0 {
                (remaining / span).clamp(0.0, 1.0)
            } else {
                0.0
            };

            return mix(control[index], control[index + 1], blend);
        }

        remaining -= span;
    }

    control[0]
}
