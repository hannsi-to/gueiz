//! 輪郭を三角形リストに開く。
//!
//! 塗りも線もここを通って三角形になるので、描画側はトポロジを 1 つしか持たない。
//! ドローコールを増やさずに線を引けるのはこのため。

use std::f32::consts::PI;
use std::ops::Range;

use crate::paint_type::{JointType, PaintType};
use crate::vertex::Vertex;

/// 尖らせすぎを防ぐ上限。線幅に対する飛び出し量の比。
///
/// これを超える鋭角では [`JointType::Miter`] が [`JointType::Bevel`] に落ちる。
/// SVG の既定値と同じ 4.0。
const MITER_LIMIT: f32 = 4.0;

/// 丸い角・丸い端を刻む細かさ。この角度ごとに 1 枚三角形を置く。
const ROUND_STEP: f32 = PI / 8.0;

/// 輪郭を [`PaintType`] に従って三角形に開く。
///
/// `contour_starts` は輪郭の区切り。各輪郭が `outline` のどこから始まるかを
/// 昇順に並べる。先頭の輪郭が外周で、2 つめ以降は穴として扱う。
/// 空スライスを渡せば輪郭 1 つとみなす。
pub fn tessellate(
    outline: &[Vertex],
    contour_starts: &[usize],
    paint_type: PaintType,
    triangles: &mut Vec<Vertex>,
) {
    match paint_type {
        PaintType::Fill => fill(outline, contour_starts, triangles),

        PaintType::Stroke {
            line_width,
            joint_type,
            strip,
        } => {
            // 線は輪郭ごとに独立して引く。穴の縁にも同じ太さの線が付く。
            for range in contour_ranges(outline, contour_starts, 2) {
                stroke(&outline[range], line_width, joint_type, strip, triangles);
            }
        }
    }
}

/// 輪郭の内側を塗る。2 つめ以降の輪郭は穴として抜く。
///
/// 穴を外周につないで 1 本の輪郭にしてから耳刈り取りで潰すので、
/// 凹多角形も穴のある形も扱える。出るのは三角形リストだけなので、
/// 描画側のトポロジは変わらない。
///
/// [`crate::object::Object::end`] から 1 回呼ばれるだけで、毎フレームは走らない。
///
/// # 制限
///
/// **自己交差した輪郭は正しく塗れない。** 止まりはするし三角形も出るが、
/// 交差点は頂点として持たないので、房どうしを分けられずに重ね塗りになる。
/// 正しく塗るには辺どうしの交点を求めて輪郭を切り直す必要があり、
/// それは耳刈り取りとは別の仕組みになる。
pub fn fill(outline: &[Vertex], contour_starts: &[usize], triangles: &mut Vec<Vertex>) {
    let contours: Vec<Vec<Vertex>> = contour_ranges(outline, contour_starts, 3)
        .into_iter()
        .map(|range| outline[range].to_vec())
        .collect();

    if contours.is_empty() {
        return;
    }

    // どれが外周でどれが穴かは、**包含関係から決める**。
    // 順番で決め打つと、`i` や `=` のように離れた輪郭が 2 つある形で
    // 片方が穴にされて消える。
    for group in group_contours(&contours) {
        fill_group(&contours, &group, triangles);
    }
}

/// 外周 1 つと、その中の穴。
#[derive(Debug, Default)]
struct ContourGroup {
    outer: usize,
    holes: Vec<usize>,
}

/// 輪郭を「外周とその穴」の組に仕分ける。
///
/// 何重に囲まれているかを数え、**偶数なら外周、奇数なら穴**とする。
/// 穴は、自分を囲むもののうちいちばん内側のものに属する。
/// 穴の中にまた島がある形（`8` の中に点、など）もこれで扱える。
fn group_contours(contours: &[Vec<Vertex>]) -> Vec<ContourGroup> {
    let count = contours.len();

    // depth[i] = i を囲んでいる輪郭の数。
    let mut depth = vec![0_usize; count];
    // parent[i] = i を囲むもののうち、いちばん内側（depth が最大）のもの。
    let mut parent = vec![None; count];

    for inner in 0..count {
        // 輪郭上の点ではなく、内側の点で判定しないと境界で揺れる。
        let Some(probe) = representative_point(&contours[inner]) else {
            continue;
        };

        for outer in 0..count {
            if outer == inner || !contains(&contours[outer], probe) {
                continue;
            }

            depth[inner] += 1;
        }
    }

    for inner in 0..count {
        let Some(probe) = representative_point(&contours[inner]) else {
            continue;
        };

        let mut best: Option<usize> = None;

        for outer in 0..count {
            if outer == inner || !contains(&contours[outer], probe) {
                continue;
            }

            // 囲んでいるもののうち、いちばん深い = いちばん内側。
            if best.is_none_or(|current| depth[outer] > depth[current]) {
                best = Some(outer);
            }
        }

        parent[inner] = best;
    }

    let mut groups: Vec<ContourGroup> = Vec::new();
    let mut slot = vec![None; count];

    for index in 0..count {
        if depth[index] % 2 == 0 {
            slot[index] = Some(groups.len());
            groups.push(ContourGroup {
                outer: index,
                holes: Vec::new(),
            });
        }
    }

    for index in 0..count {
        if depth[index] % 2 == 0 {
            continue;
        }

        // 奇数なら穴。属する先はいちばん内側の囲み。
        if let Some(group) = parent[index].and_then(|outer| slot[outer]) {
            groups[group].holes.push(index);
        }
    }

    groups
}

/// 外周 1 つとその穴を 1 枚の多角形に畳んで、三角形に開く。
fn fill_group(contours: &[Vec<Vertex>], group: &ContourGroup, triangles: &mut Vec<Vertex>) {
    let mut polygon = contours[group.outer].clone();

    // 巻き方向はユーザーに要求しない。外周を反時計回りに、穴をその逆に揃える。
    // 任せると「穴が塗りつぶされる」類のバグを必ず踏む。
    if signed_area(&polygon) < 0.0 {
        polygon.reverse();
    }

    let mut holes: Vec<Vec<Vertex>> = group
        .holes
        .iter()
        .map(|&index| contours[index].clone())
        .collect();

    for hole in &mut holes {
        if signed_area(hole) > 0.0 {
            hole.reverse();
        }
    }

    // 右にある穴から順につなぐ。先につないだ通路が後の探索を邪魔しない。
    holes.sort_by(|a, b| rightmost_x(b).total_cmp(&rightmost_x(a)));

    for hole in &holes {
        bridge_hole(&mut polygon, hole);
    }

    ear_clip(&polygon, triangles);
}

/// 包含関係を調べるための代表点。**輪郭の上の頂点**を使う。
///
/// 内部の点を使ってはいけない。大きい四角の内部の点は、その中に空いた穴の
/// 内部にも入るので、「外周が穴に含まれる」と判定されてしまう。
/// 境界の上なら、A が B に含まれるときだけ B の内側に入る。
///
/// いちばん下の頂点を選ぶ。凸包の上にあるので、別の輪郭の辺と重なりにくい。
fn representative_point(contour: &[Vertex]) -> Option<Vertex> {
    contour
        .iter()
        .copied()
        .reduce(|lowest, vertex| if vertex.y < lowest.y { vertex } else { lowest })
}

/// 点が輪郭の内側にあるか。+X に線を飛ばして、横切った辺を数える。
fn contains(contour: &[Vertex], point: Vertex) -> bool {
    let count = contour.len();
    let mut inside = false;

    for index in 0..count {
        let start = contour[index];
        let end = contour[(index + 1) % count];

        if (start.y > point.y) != (end.y > point.y)
            && point.x
                < (end.x - start.x) * (point.y - start.y) / (end.y - start.y) + start.x
        {
            inside = !inside;
        }
    }

    inside
}

/// 輪郭の区切りを範囲に直す。`minimum` 頂点に満たない輪郭は落とす。
fn contour_ranges(
    outline: &[Vertex],
    contour_starts: &[usize],
    minimum: usize,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;

    // 先頭の 0 は書いても書かなくてもよい。範囲外や重複は黙って畳む。
    for &next in contour_starts.iter().chain(std::iter::once(&outline.len())) {
        let next = next.min(outline.len());

        if next > start {
            if next - start >= minimum {
                ranges.push(start..next);
            }

            start = next;
        }
    }

    ranges
}

/// 多角形の符号付き面積。正なら反時計回り。
fn signed_area(contour: &[Vertex]) -> f32 {
    let count = contour.len();
    let mut total = 0.0;

    for index in 0..count {
        let current = contour[index];
        let next = contour[(index + 1) % count];
        total += current.x * next.y - next.x * current.y;
    }

    total * 0.5
}

/// いちばん右にある頂点の x。穴をつなぐ順番を決めるのに使う。
fn rightmost_x(contour: &[Vertex]) -> f32 {
    contour
        .iter()
        .map(|vertex| vertex.x)
        .fold(f32::NEG_INFINITY, f32::max)
}

/// 3 点の外積。正なら `o -> a -> b` が左に曲がる。
fn cross(o: Vertex, a: Vertex, b: Vertex) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// 反時計回りの三角形の内側に点があるか。**辺の上も内側として数える**。
///
/// 辺を含めるのが要点。L 字の内角のように、凹んだ頂点が候補の三角形の辺に
/// ぴったり乗ることがあり、そこを見逃すと輪郭の外を塗ってしまう。
/// 穴が同じ高さに並んだときの通路探しでも同じことが起きる。
fn point_in_triangle_inclusive(a: Vertex, b: Vertex, c: Vertex, point: Vertex) -> bool {
    cross(a, b, point) >= 0.0 && cross(b, c, point) >= 0.0 && cross(c, a, point) >= 0.0
}

/// 同じ場所にある頂点か。穴をつないだ通路の複製を見分けるのに使う。
fn same_position(a: Vertex, b: Vertex) -> bool {
    a.x == b.x && a.y == b.y
}

/// 穴を外周につなぐ。幅ゼロの通路を作って 1 本の輪郭にする。
///
/// ```text
///   外周 .. P  Q ..                .. P [M .. 穴を一周 .. M] P  Q ..
///   穴   .. M ..      ->                    ^^^^^^^^^^^^^  通路
/// ```
///
/// `P` と `M` が 2 回ずつ現れ、行きと帰りで打ち消し合うので面積は増えない。
fn bridge_hole(polygon: &mut Vec<Vertex>, hole: &[Vertex]) {
    let Some(hole_index) = (0..hole.len()).reduce(|best, index| {
        if hole[index].x > hole[best].x {
            index
        } else {
            best
        }
    }) else {
        return;
    };

    let Some(bridge_index) = find_bridge_vertex(polygon, hole[hole_index]) else {
        // つなぎ先が見つからない。穴を諦めて外周だけ塗る。
        log::warn!("a hole could not be bridged to the outline; it will not be cut out");
        return;
    };

    let mut spliced = Vec::with_capacity(polygon.len() + hole.len() + 2);
    spliced.extend_from_slice(&polygon[..=bridge_index]);
    // 穴を `hole_index` から一周して同じ頂点に戻る。
    spliced.extend_from_slice(&hole[hole_index..]);
    spliced.extend_from_slice(&hole[..=hole_index]);
    spliced.extend_from_slice(&polygon[bridge_index..]);

    *polygon = spliced;
}

/// 穴の右端から +X に線を飛ばし、そこから見える外周の頂点を探す。
fn find_bridge_vertex(polygon: &[Vertex], from: Vertex) -> Option<usize> {
    let count = polygon.len();

    // 1. 線が最初に当たる辺を探す。
    let mut hit_x = f32::INFINITY;
    let mut hit_edge = None;

    for index in 0..count {
        let start = polygon[index];
        let end = polygon[(index + 1) % count];

        // 辺が from.y をまたがなければ当たらない。
        if (start.y > from.y) == (end.y > from.y) {
            continue;
        }

        let ratio = (from.y - start.y) / (end.y - start.y);
        let x = start.x + ratio * (end.x - start.x);

        if x > from.x && x < hit_x {
            hit_x = x;
            hit_edge = Some(index);
        }
    }

    let edge = hit_edge?;

    // 2. その辺の、より右にある端点をひとまずの候補にする。
    let next_edge = (edge + 1) % count;
    // 同じ x なら手前（辺の始点）を採る。遠い頂点へつなぐと通路が長くなり、
    // 後から来る穴の通路と交差しやすくなる。
    let mut best = if polygon[edge].x >= polygon[next_edge].x {
        edge
    } else {
        next_edge
    };

    // 3. 「穴の右端・線の当たった点・候補」が作る三角形の中に凹んだ頂点が
    //    入っていると、そこを通る通路は外周を横切ってしまう。入っているものの
    //    うち、線にいちばん近いものへ乗り換える。
    let hit = {
        let mut point = from;
        point.x = hit_x;
        point
    };

    let mut best_tangent = f32::INFINITY;

    for index in 0..count {
        if index == best {
            continue;
        }

        let vertex = polygon[index];
        if vertex.x < from.x {
            continue;
        }

        // 凸な頂点は通路を塞がない。凹んだ頂点だけを見る。
        let previous = polygon[(index + count - 1) % count];
        let next = polygon[(index + 1) % count];
        if cross(previous, vertex, next) > 0.0 {
            continue;
        }

        // 辺の上も数える。穴どうしが同じ高さに並ぶと、塞いでいる頂点が
        // ちょうど線の上に乗る。ここを見逃すと通路が別の穴を突き抜ける。
        // 候補が線の上下どちらにあるかで三角形の向きが変わるので両方試す。
        let inside = point_in_triangle_inclusive(from, hit, polygon[best], vertex)
            || point_in_triangle_inclusive(polygon[best], hit, from, vertex);
        if !inside {
            continue;
        }

        let run = vertex.x - from.x;
        let tangent = if run > 0.0 {
            (vertex.y - from.y).abs() / run
        } else {
            f32::INFINITY
        };

        // 線に近いものを採る。同じ角度なら近いほうへ。通路は短いほど、
        // 後から来る穴の通路と交差しにくい。
        if tangent < best_tangent || (tangent == best_tangent && vertex.x < polygon[best].x) {
            best_tangent = tangent;
            best = index;
        }
    }

    Some(best)
}

/// 割りすぎを止める。各段で輪郭は必ず 1 頂点以上短くなるので必ず終わるが、
/// 再帰の深さだけは頂点数に比例するため、念のため上限を置く。
const MAX_SPLIT_DEPTH: usize = 64;

/// 単一輪郭を耳刈り取りで三角形に潰す。
///
/// 「耳」＝凸で、他のどの凹んだ頂点も内側に含まない 3 連続頂点。
/// 見つけては切り落とし、頂点が 3 つになるまで繰り返す。
///
/// 単純な（自己交差しない）多角形なら耳は必ず存在する。見つからないのは
/// 入力が自己交差しているときで、その場合は対角線 1 本で 2 つに割って
/// やり直す（[`split_and_clip`]）。
///
/// 最悪 O(n^2) だが、図形あたりの頂点は数十で、しかも形が変わったときにしか
/// 走らないので毎フレームの負荷にはならない。
fn ear_clip(polygon: &[Vertex], triangles: &mut Vec<Vertex>) {
    if polygon.len() < 3 {
        return;
    }

    let mut indices: Vec<usize> = (0..polygon.len()).collect();

    // 耳の判定は反時計回りを前提にしている。
    if signed_area(polygon) < 0.0 {
        indices.reverse();
    }

    clip_loop(polygon, indices, triangles, 0);
}

/// 輪郭 1 本を潰す。詰まったら割って、それぞれを潰し直す。
fn clip_loop(
    polygon: &[Vertex],
    mut indices: Vec<usize>,
    triangles: &mut Vec<Vertex>,
    depth: usize,
) {
    while indices.len() > 3 {
        let count = indices.len();
        let mut clipped = None;

        for position in 0..count {
            if is_ear(polygon, &indices, position) {
                let before = (position + count - 1) % count;
                let after = (position + 1) % count;

                triangles.push(polygon[indices[before]]);
                triangles.push(polygon[indices[position]]);
                triangles.push(polygon[indices[after]]);
                clipped = Some(position);
                break;
            }
        }

        let Some(position) = clipped else {
            // 耳が 1 つも無い。自己交差している輪郭。
            if depth < MAX_SPLIT_DEPTH && split_and_clip(polygon, &indices, triangles, depth) {
                return;
            }

            // 割れる対角線も無い。正しくはないが、何も出ないよりましな形を出す。
            // 穴を外周につないだ通路は幅ゼロなので、最後に潰れた残りかすが
            // 出ることがある。面積が無いなら三角形にする意味も無いし、
            // 異常でもない。黙って捨てる。
            if is_degenerate(polygon, &indices) {
                return;
            }

            log::warn!(
                "ear clipping stalled with {} vertices left; the outline may be self-intersecting",
                indices.len(),
            );
            push_index_fan(polygon, &indices, triangles);
            return;
        };

        indices.remove(position);
    }

    push_index_fan(polygon, &indices, triangles);
}

/// 詰まった輪郭を対角線 1 本で 2 つに割り、それぞれを潰し直す。割れたら `true`。
///
/// 自己交差した輪郭には耳が無いことがあるが、交差している部分を跨がない
/// 対角線で切り分ければ、それぞれの側では耳が見つかる。
///
/// 対角線の候補は O(n^2) 本あり、1 本ごとに全辺との交差を見るので O(n^3)。
/// 詰まったときにしか走らないので、正しい入力では 1 度も通らない。
fn split_and_clip(
    polygon: &[Vertex],
    indices: &[usize],
    triangles: &mut Vec<Vertex>,
    depth: usize,
) -> bool {
    let count = indices.len();

    for pos_a in 0..count {
        // 隣は対角線にならないので 2 つ先から。
        for offset in 2..count - 1 {
            let pos_b = (pos_a + offset) % count;

            if !is_valid_diagonal(polygon, indices, pos_a, pos_b) {
                continue;
            }

            // 両側とも両端の頂点を持つ。合計は count + 2 で、
            // それぞれ 3 以上 count 未満なので、割るたびに必ず短くなる。
            let first = walk(indices, pos_a, pos_b);
            let second = walk(indices, pos_b, pos_a);

            clip_loop(polygon, first, triangles, depth + 1);
            clip_loop(polygon, second, triangles, depth + 1);

            return true;
        }
    }

    false
}

/// `from` から `to` まで輪郭を辿った並び。両端を含む。
fn walk(indices: &[usize], from: usize, to: usize) -> Vec<usize> {
    let count = indices.len();
    let mut walked = Vec::new();
    let mut position = from;

    loop {
        walked.push(indices[position]);

        if position == to {
            return walked;
        }

        position = (position + 1) % count;
    }
}

/// `pos_a` と `pos_b` を結ぶ線が、輪郭を 2 つに割る対角線として使えるか。
///
/// 条件は 3 つ。どの辺とも交差しないこと、両端で内側を向いていること、
/// 中点が輪郭の内側にあること。
fn is_valid_diagonal(
    polygon: &[Vertex],
    indices: &[usize],
    pos_a: usize,
    pos_b: usize,
) -> bool {
    let count = indices.len();

    // 隣り合う頂点を結んでも割れない。
    if (pos_a + 1) % count == pos_b || (pos_b + 1) % count == pos_a {
        return false;
    }

    let a = polygon[indices[pos_a]];
    let b = polygon[indices[pos_b]];

    // 同じ場所にある頂点どうしは長さ 0 で、向きが決まらない。
    if same_position(a, b) {
        return false;
    }

    for index in 0..count {
        let next = (index + 1) % count;

        // 端点を共有する辺は、触れていても交差ではない。
        if index == pos_a || index == pos_b || next == pos_a || next == pos_b {
            continue;
        }

        if segments_intersect(polygon[indices[index]], polygon[indices[next]], a, b) {
            return false;
        }
    }

    let previous_a = polygon[indices[(pos_a + count - 1) % count]];
    let next_a = polygon[indices[(pos_a + 1) % count]];
    let previous_b = polygon[indices[(pos_b + count - 1) % count]];
    let next_b = polygon[indices[(pos_b + 1) % count]];

    locally_inside(previous_a, a, next_a, b)
        && locally_inside(previous_b, b, next_b, a)
        && middle_inside(polygon, indices, a, b)
}

/// `a` から見て `a -> b` が輪郭の内側へ向かっているか。
fn locally_inside(previous: Vertex, a: Vertex, next: Vertex, b: Vertex) -> bool {
    if cross(previous, a, next) > 0.0 {
        // a は凸。2 辺が挟む扇の中なら内側。
        cross(a, b, next) <= 0.0 && cross(a, previous, b) <= 0.0
    } else {
        // a は凹。外側の扇から外れていれば内側。
        cross(a, b, previous) > 0.0 || cross(a, next, b) > 0.0
    }
}

/// 対角線の中点が輪郭の内側にあるか。+X に線を飛ばして辺を数える。
///
/// 両端が内側を向いていても、輪郭の外を跨ぐ対角線はあり得る。そこを弾く。
fn middle_inside(polygon: &[Vertex], indices: &[usize], a: Vertex, b: Vertex) -> bool {
    let x = (a.x + b.x) * 0.5;
    let y = (a.y + b.y) * 0.5;

    let count = indices.len();
    let mut inside = false;

    for index in 0..count {
        let start = polygon[indices[index]];
        let end = polygon[indices[(index + 1) % count]];

        if (start.y > y) != (end.y > y)
            && x < (end.x - start.x) * (y - start.y) / (end.y - start.y) + start.x
        {
            inside = !inside;
        }
    }

    inside
}

/// 2 本の線分が交わるか。端点で触れるだけでも交わりとみなす。
fn segments_intersect(p1: Vertex, q1: Vertex, p2: Vertex, q2: Vertex) -> bool {
    let o1 = orientation(p1, q1, p2);
    let o2 = orientation(p1, q1, q2);
    let o3 = orientation(p2, q2, p1);
    let o4 = orientation(p2, q2, q1);

    // それぞれの線分が、もう一方を挟んで反対側に端点を持つ。
    if o1 != o2 && o3 != o4 {
        return true;
    }

    // 同一直線上に乗って重なっている場合。
    (o1 == 0 && on_segment(p1, q1, p2))
        || (o2 == 0 && on_segment(p1, q1, q2))
        || (o3 == 0 && on_segment(p2, q2, p1))
        || (o4 == 0 && on_segment(p2, q2, q1))
}

/// `a -> b -> c` の曲がる向き。1 = 左、-1 = 右、0 = 同一直線上。
fn orientation(a: Vertex, b: Vertex, c: Vertex) -> i32 {
    let turn = cross(a, b, c);

    if turn > 0.0 {
        1
    } else if turn < 0.0 {
        -1
    } else {
        0
    }
}

/// 同一直線上にある前提で、`point` が線分 `a-b` の範囲に収まっているか。
fn on_segment(a: Vertex, b: Vertex, point: Vertex) -> bool {
    point.x >= a.x.min(b.x)
        && point.x <= a.x.max(b.x)
        && point.y >= a.y.min(b.y)
        && point.y <= a.y.max(b.y)
}

/// `position` の頂点を先端とする 3 連続頂点が耳か。
///
/// 条件は 2 つ。凸であること、そして**凹んだ頂点**が 1 つも内側に入っていないこと。
/// 凸な頂点は三角形の中にあっても輪郭を裂かないので見なくてよく、
/// 見ないほうが穴の通路（頂点が重なる）を正しく扱える。
fn is_ear(polygon: &[Vertex], indices: &[usize], position: usize) -> bool {
    let count = indices.len();
    let before = (position + count - 1) % count;
    let after = (position + 1) % count;

    let va = polygon[indices[before]];
    let vb = polygon[indices[position]];
    let vc = polygon[indices[after]];

    // 凹んでいる、または潰れている頂点は耳ではない。
    if cross(va, vb, vc) <= 0.0 {
        return false;
    }

    for other in 0..count {
        if other == before || other == position || other == after {
            continue;
        }

        let vertex = polygon[indices[other]];

        // 通路の複製は三角形の角そのものなので、邪魔はしていない。
        if same_position(vertex, va) || same_position(vertex, vb) || same_position(vertex, vc) {
            continue;
        }

        // 凸な頂点は内側にあっても構わない。凹んだ頂点だけが輪郭を裂く。
        let previous = polygon[indices[(other + count - 1) % count]];
        let next = polygon[indices[(other + 1) % count]];
        if cross(previous, vertex, next) > 0.0 {
            continue;
        }

        if point_in_triangle_inclusive(va, vb, vc, vertex) {
            return false;
        }
    }

    true
}

/// 残った部分に面積が無いか。
///
/// 輪郭全体と比べて判断する。絶対値で決めると、ピクセル単位の図形と
/// em 単位の図形で基準が変わってしまう。
fn is_degenerate(polygon: &[Vertex], indices: &[usize]) -> bool {
    let remainder: Vec<Vertex> = indices.iter().map(|&index| polygon[index]).collect();
    let left = signed_area(&remainder).abs();

    if left == 0.0 {
        return true;
    }

    let whole = signed_area(polygon).abs();

    whole > 0.0 && left <= whole * 1e-6
}

/// 残った輪郭を先頭から張るファンで潰す。
fn push_index_fan(polygon: &[Vertex], indices: &[usize], triangles: &mut Vec<Vertex>) {
    for position in 1..indices.len().saturating_sub(1) {
        triangles.push(polygon[indices[0]]);
        triangles.push(polygon[indices[position]]);
        triangles.push(polygon[indices[position + 1]]);
    }
}

/// 輪郭を太さのある帯に開く。
///
/// 辺ごとに四角形を 1 枚置き、角には [`JointType`] に応じた継ぎ目を足す。
pub fn stroke(
    outline: &[Vertex],
    line_width: f32,
    joint_type: JointType,
    strip: bool,
    triangles: &mut Vec<Vertex>,
) {
    if outline.len() < 2 || line_width <= 0.0 {
        return;
    }

    let half_width = line_width * 0.5;

    // 閉じた輪郭は最後の頂点から先頭へ戻る辺も持つ。
    let segment_count = if strip {
        outline.len() - 1
    } else {
        outline.len()
    };

    for index in 0..segment_count {
        let start = outline[index];
        let end = outline[(index + 1) % outline.len()];

        let Some(normal) = segment_normal(start, end) else {
            // 長さ 0 の辺には向きが無いので飛ばす。
            continue;
        };

        push_quad(start, end, normal, half_width, triangles);
    }

    if joint_type != JointType::None {
        push_joints(outline, half_width, joint_type, strip, triangles);
    }

    if strip {
        push_caps(outline, half_width, joint_type, triangles);
    }
}

/// 辺に垂直な単位ベクトル（進行方向の左）。長さ 0 の辺なら `None`。
fn segment_normal(start: Vertex, end: Vertex) -> Option<[f32; 2]> {
    let direction_x = end.x - start.x;
    let direction_y = end.y - start.y;
    let length = (direction_x * direction_x + direction_y * direction_y).sqrt();

    if length < f32::EPSILON {
        return None;
    }

    Some([-direction_y / length, direction_x / length])
}

/// 位置だけずらした頂点を作る。色などは元のまま。
fn offset(vertex: Vertex, offset_x: f32, offset_y: f32) -> Vertex {
    let mut moved = vertex;
    moved.x += offset_x;
    moved.y += offset_y;
    moved
}

/// 辺 1 本ぶんの帯（四角形 = 三角形 2 枚）。
fn push_quad(
    start: Vertex,
    end: Vertex,
    normal: [f32; 2],
    half_width: f32,
    triangles: &mut Vec<Vertex>,
) {
    let [nx, ny] = normal;
    let (dx, dy) = (nx * half_width, ny * half_width);

    let start_left = offset(start, dx, dy);
    let start_right = offset(start, -dx, -dy);
    let end_left = offset(end, dx, dy);
    let end_right = offset(end, -dx, -dy);

    triangles.push(start_left);
    triangles.push(start_right);
    triangles.push(end_right);

    triangles.push(start_left);
    triangles.push(end_right);
    triangles.push(end_left);
}

/// 角の外側にできる隙間を埋める。
fn push_joints(
    outline: &[Vertex],
    half_width: f32,
    joint_type: JointType,
    strip: bool,
    triangles: &mut Vec<Vertex>,
) {
    let count = outline.len();

    // 開いた折れ線なら両端は角ではないので、内側の頂点だけを見る。
    let (first, last) = if strip { (1, count - 1) } else { (0, count) };

    for index in first..last {
        let previous = outline[(index + count - 1) % count];
        let corner = outline[index];
        let next = outline[(index + 1) % count];

        let (Some(incoming), Some(outgoing)) = (
            segment_normal(previous, corner),
            segment_normal(corner, next),
        ) else {
            continue;
        };

        // 進行方向の外積で曲がる向きを見る。左に曲がるなら隙間は右側。
        let cross = incoming[0] * outgoing[1] - incoming[1] * outgoing[0];
        if cross.abs() < f32::EPSILON {
            // まっすぐ、または 180 度折り返し。継ぎ目は要らない。
            continue;
        }

        // 隙間が開く側の法線を選ぶ。
        let sign = if cross > 0.0 { -1.0 } else { 1.0 };
        let from = [incoming[0] * sign, incoming[1] * sign];
        let to = [outgoing[0] * sign, outgoing[1] * sign];

        if joint_type.is_round() {
            push_round_joint(corner, from, to, half_width, triangles);
        } else if joint_type == JointType::Miter {
            push_miter_joint(corner, from, to, half_width, triangles);
        } else {
            push_bevel_joint(corner, from, to, half_width, triangles);
        }
    }
}

/// 外側の角を三角形 1 枚で塞ぐ。
fn push_bevel_joint(
    corner: Vertex,
    from: [f32; 2],
    to: [f32; 2],
    half_width: f32,
    triangles: &mut Vec<Vertex>,
) {
    triangles.push(corner);
    triangles.push(offset(corner, from[0] * half_width, from[1] * half_width));
    triangles.push(offset(corner, to[0] * half_width, to[1] * half_width));
}

/// 外側の辺を延長して尖らせる。鋭すぎるときはベベルに落とす。
fn push_miter_joint(
    corner: Vertex,
    from: [f32; 2],
    to: [f32; 2],
    half_width: f32,
    triangles: &mut Vec<Vertex>,
) {
    let sum_x = from[0] + to[0];
    let sum_y = from[1] + to[1];
    let length = (sum_x * sum_x + sum_y * sum_y).sqrt();

    if length < f32::EPSILON {
        push_bevel_joint(corner, from, to, half_width, triangles);
        return;
    }

    let miter_x = sum_x / length;
    let miter_y = sum_y / length;

    // 2 つの法線の中間方向と法線のなす角から、飛び出し量が決まる。
    let cosine = miter_x * from[0] + miter_y * from[1];
    if cosine < f32::EPSILON || 1.0 / cosine > MITER_LIMIT {
        push_bevel_joint(corner, from, to, half_width, triangles);
        return;
    }

    let miter_length = half_width / cosine;
    let tip = offset(corner, miter_x * miter_length, miter_y * miter_length);

    let from_point = offset(corner, from[0] * half_width, from[1] * half_width);
    let to_point = offset(corner, to[0] * half_width, to[1] * half_width);

    triangles.push(corner);
    triangles.push(from_point);
    triangles.push(tip);

    triangles.push(corner);
    triangles.push(tip);
    triangles.push(to_point);
}

/// 外側の角を扇形で丸める。
fn push_round_joint(
    corner: Vertex,
    from: [f32; 2],
    to: [f32; 2],
    half_width: f32,
    triangles: &mut Vec<Vertex>,
) {
    let start_angle = from[1].atan2(from[0]);
    let end_angle = to[1].atan2(to[0]);

    // 短いほうの回り方を選ぶ。角の外側は必ず 180 度未満。
    let mut sweep = end_angle - start_angle;
    while sweep > PI {
        sweep -= 2.0 * PI;
    }
    while sweep < -PI {
        sweep += 2.0 * PI;
    }

    push_fan(corner, start_angle, sweep, half_width, triangles);
}

/// 開いた折れ線の端を半円で閉じる。
fn push_caps(
    outline: &[Vertex],
    half_width: f32,
    joint_type: JointType,
    triangles: &mut Vec<Vertex>,
) {
    if joint_type.caps_start() {
        if let Some(normal) = segment_normal(outline[0], outline[1]) {
            // 帯の左端から右端へ、進行方向と逆回りに半円を張る。
            let start_angle = normal[1].atan2(normal[0]);
            push_fan(outline[0], start_angle, PI, half_width, triangles);
        }
    }

    if joint_type.caps_end() {
        let last = outline.len() - 1;
        if let Some(normal) = segment_normal(outline[last - 1], outline[last]) {
            let start_angle = normal[1].atan2(normal[0]);
            push_fan(outline[last], start_angle, -PI, half_width, triangles);
        }
    }
}

/// `center` を要とした扇形。`sweep` はラジアン、符号が回る向き。
fn push_fan(
    center: Vertex,
    start_angle: f32,
    sweep: f32,
    radius: f32,
    triangles: &mut Vec<Vertex>,
) {
    // ちょうど割り切れる角度で f32 の丸めが 1 枚余計に刻むのを防ぐ。
    let steps = ((sweep.abs() / ROUND_STEP - 1e-3).ceil().max(1.0)) as u32;
    let step = sweep / steps as f32;

    for index in 0..steps {
        let angle = start_angle + step * index as f32;
        let next_angle = angle + step;

        triangles.push(center);
        triangles.push(offset(center, angle.cos() * radius, angle.sin() * radius));
        triangles.push(offset(
            center,
            next_angle.cos() * radius,
            next_angle.sin() * radius,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, y: f32) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0)
    }

    /// 三角形リストなので、頂点数は必ず 3 の倍数になる。
    fn assert_triangle_list(triangles: &[Vertex]) {
        assert_eq!(triangles.len() % 3, 0, "{} vertices", triangles.len());
    }

    #[test]
    fn a_triangle_fills_to_one_triangle() {
        let mut triangles = Vec::new();
        fill(
            &[point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)],
            &[0],
            &mut triangles,
        );

        assert_eq!(triangles.len(), 3);
    }

    #[test]
    fn a_quad_fills_to_two_triangles() {
        let mut triangles = Vec::new();
        fill(
            &[point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0), point(0.0, 10.0)],
            &[0],
            &mut triangles,
        );

        assert_eq!(triangles.len(), 6);
    }

    #[test]
    fn a_closed_stroke_has_one_quad_per_edge() {
        let mut triangles = Vec::new();
        stroke(
            &[point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)],
            2.0,
            JointType::None,
            false,
            &mut triangles,
        );

        // 辺 3 本 x 三角形 2 枚 x 頂点 3 つ
        assert_eq!(triangles.len(), 3 * 6);
        assert_triangle_list(&triangles);
    }

    #[test]
    fn an_open_strip_has_one_fewer_edge() {
        let mut triangles = Vec::new();
        stroke(
            &[point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)],
            2.0,
            JointType::None,
            true,
            &mut triangles,
        );

        // 折れ線なので辺は 2 本。閉じないぶん 1 本少ない。
        assert_eq!(triangles.len(), 2 * 6);
    }

    #[test]
    fn the_stroke_straddles_the_edge() {
        let mut triangles = Vec::new();
        stroke(
            &[point(0.0, 0.0), point(10.0, 0.0)],
            4.0,
            JointType::None,
            true,
            &mut triangles,
        );

        // 水平な辺なので、帯は y = -2 と y = 2 に開く。
        let ys: Vec<f32> = triangles.iter().map(|vertex| vertex.y).collect();
        assert!(ys.iter().any(|y| (*y - 2.0).abs() < 1e-6), "{ys:?}");
        assert!(ys.iter().any(|y| (*y + 2.0).abs() < 1e-6), "{ys:?}");
    }

    #[test]
    fn joints_add_geometry_at_the_corners() {
        let square = [
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 10.0),
            point(0.0, 10.0),
        ];

        let mut without = Vec::new();
        stroke(&square, 2.0, JointType::None, false, &mut without);

        let mut bevel = Vec::new();
        stroke(&square, 2.0, JointType::Bevel, false, &mut bevel);

        let mut miter = Vec::new();
        stroke(&square, 2.0, JointType::Miter, false, &mut miter);

        let mut round = Vec::new();
        stroke(&square, 2.0, JointType::Round, false, &mut round);

        // 角 4 つ x 三角形 1 枚
        assert_eq!(bevel.len(), without.len() + 4 * 3);
        // 角 4 つ x 三角形 2 枚
        assert_eq!(miter.len(), without.len() + 4 * 2 * 3);
        // 直角は ROUND_STEP (22.5 度) 刻みで 4 枚
        assert_eq!(round.len(), without.len() + 4 * 4 * 3);

        for triangles in [&bevel, &miter, &round] {
            assert_triangle_list(triangles);
        }
    }

    #[test]
    fn a_sharp_corner_falls_back_to_bevel() {
        // ほぼ折り返す鋭角。マイターだと遠くまで飛び出すので上限で落ちる。
        let spike = [point(0.0, 0.0), point(100.0, 0.0), point(0.0, 1.0)];

        let mut miter = Vec::new();
        stroke(&spike, 4.0, JointType::Miter, true, &mut miter);

        let mut bevel = Vec::new();
        stroke(&spike, 4.0, JointType::Bevel, true, &mut bevel);

        // 内側の角 1 つだけ。マイターが効いていれば三角形 2 枚になるはず。
        assert_eq!(miter.len(), bevel.len(), "the miter limit should kick in");

        // 角は x = 100。上限が効いていれば、そこから線幅 x MITER_LIMIT 以上は出ない。
        let max_x = miter.iter().map(|vertex| vertex.x).fold(f32::MIN, f32::max);
        assert!(
            max_x <= 100.0 + 2.0 * MITER_LIMIT + 1e-3,
            "the miter tip reached x = {max_x}",
        );
    }

    #[test]
    fn round_caps_close_an_open_strip() {
        let line = [point(0.0, 0.0), point(10.0, 0.0)];

        let mut plain = Vec::new();
        stroke(&line, 4.0, JointType::Round, true, &mut plain);

        let mut capped = Vec::new();
        stroke(&line, 4.0, JointType::RoundStartEnd, true, &mut capped);

        assert!(capped.len() > plain.len(), "caps should add geometry");
        assert_triangle_list(&capped);

        // 端の外側（x < 0 と x > 10）へ張り出す。
        assert!(capped.iter().any(|vertex| vertex.x < -1.0));
        assert!(capped.iter().any(|vertex| vertex.x > 11.0));
    }

    #[test]
    fn a_closed_outline_gets_no_caps() {
        let square = [
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 10.0),
            point(0.0, 10.0),
        ];

        let mut round = Vec::new();
        stroke(&square, 2.0, JointType::Round, false, &mut round);

        let mut round_capped = Vec::new();
        stroke(&square, 2.0, JointType::RoundStartEnd, false, &mut round_capped);

        // 閉じた輪郭に端は無いので、両者は同じになる。
        assert_eq!(round.len(), round_capped.len());
    }

    /// 三角形リストの総面積。塗りが正しいかを測る物差しになる。
    fn total_area(triangles: &[Vertex]) -> f32 {
        triangles
            .chunks_exact(3)
            .map(|triangle| {
                let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
                ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
            })
            .sum()
    }

    /// 反時計回りの正方形。
    fn square(x: f32, y: f32, size: f32) -> [Vertex; 4] {
        [
            point(x, y),
            point(x + size, y),
            point(x + size, y + size),
            point(x, y + size),
        ]
    }

    /// 穴を抜いたぶんだけ面積が減っていなければならない。
    /// ここが合っていれば、通路が余計な面積を生んでいないことも同時に言える。
    #[test]
    fn a_hole_removes_its_own_area() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 90.0));
        outline.extend_from_slice(&square(30.0, 30.0, 30.0));

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4], &mut triangles);

        assert_triangle_list(&triangles);
        assert!((total_area(&triangles) - (90.0 * 90.0 - 30.0 * 30.0)).abs() < 1e-2);
    }

    /// 穴の中を覆う三角形があってはいけない。面積だけでは、穴を塗ったぶんと
    /// どこかを塗り残したぶんが打ち消し合う可能性が残る。
    #[test]
    fn nothing_covers_the_hole() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 90.0));
        outline.extend_from_slice(&square(30.0, 30.0, 30.0));

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4], &mut triangles);

        let center = point(45.0, 45.0);
        for triangle in triangles.chunks_exact(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
            let covered = point_in_triangle(a, b, c, center) || point_in_triangle(a, c, b, center);
            assert!(!covered, "穴の中心が塗られている");
        }
    }

    /// 穴は何個でも開けられる。総面積でざっと見る。
    /// 重なりと塗り残しは [`several_holes_leave_no_overlap_and_no_gap`] が見る。
    #[test]
    fn several_holes_are_all_cut_out() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 100.0));
        outline.extend_from_slice(&square(10.0, 10.0, 20.0));
        outline.extend_from_slice(&square(60.0, 10.0, 20.0));
        outline.extend_from_slice(&square(35.0, 60.0, 20.0));

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4, 8, 12], &mut triangles);

        assert_triangle_list(&triangles);

        let expected = 100.0 * 100.0 - 3.0 * 20.0 * 20.0;
        assert!((total_area(&triangles) - expected).abs() < 1e-2);
    }

    /// 離れた輪郭は**どちらも塗る**。片方を穴にしてはいけない。
    ///
    /// フォントの `i` や `=` がこの形。順番で「先頭が外周、残りは穴」と
    /// 決め打つと、点や横棒が消える。
    #[test]
    fn disjoint_contours_are_both_filled() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 20.0));
        outline.extend_from_slice(&square(50.0, 0.0, 30.0));

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4], &mut triangles);

        assert_triangle_list(&triangles);

        let expected = 20.0 * 20.0 + 30.0 * 30.0;
        assert!(
            (total_area(&triangles) - expected).abs() < 1e-2,
            "{} vs {expected}",
            total_area(&triangles),
        );

        // どちらの中も 1 枚に覆われている。
        assert_eq!(coverage(&triangles, point(12.0, 6.0)), 1);
        assert_eq!(coverage(&triangles, point(70.0, 12.0)), 1);
        // 間は空いている。
        assert_eq!(coverage(&triangles, point(35.0, 12.0)), 0);
    }

    /// 穴の中の島は、また塗る。`0` の中に点があるような形。
    #[test]
    fn an_island_inside_a_hole_is_filled_again() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 100.0));
        outline.extend_from_slice(&square(20.0, 20.0, 60.0));
        outline.extend_from_slice(&square(40.0, 40.0, 20.0));

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4, 8], &mut triangles);

        assert_triangle_list(&triangles);

        let expected = 100.0 * 100.0 - 60.0 * 60.0 + 20.0 * 20.0;
        assert!(
            (total_area(&triangles) - expected).abs() < 1e-2,
            "{} vs {expected}",
            total_area(&triangles),
        );

        // 三角形の継ぎ目（対角線）に乗ると、厳密判定ではどちらにも数えない。
        // 継ぎ目を外した点で見る。
        assert_eq!(coverage(&triangles, point(10.0, 52.0)), 1, "外周の帯");
        assert_eq!(coverage(&triangles, point(30.0, 52.0)), 0, "穴");
        assert_eq!(coverage(&triangles, point(52.0, 46.0)), 1, "穴の中の島");
    }

    /// 順番に依存しない。穴を先に書いても結果は同じ。
    #[test]
    fn the_contour_order_does_not_decide_what_is_a_hole() {
        let outer = square(0.0, 0.0, 90.0);
        let hole = square(30.0, 30.0, 30.0);

        let mut hole_first = Vec::new();
        hole_first.extend_from_slice(&hole);
        hole_first.extend_from_slice(&outer);

        let mut triangles = Vec::new();
        fill(&hole_first, &[0, 4], &mut triangles);

        let expected = 90.0 * 90.0 - 30.0 * 30.0;
        assert!(
            (total_area(&triangles) - expected).abs() < 1e-2,
            "先に書いたほうが外周とは限らない",
        );
        assert_eq!(coverage(&triangles, point(45.0, 45.0)), 0, "穴は穴のまま");
    }

    /// 巻き方向はユーザーに要求しない。外周・穴のどちらを逆に書いても同じ形になる。
    #[test]
    fn the_winding_order_does_not_matter() {
        let outer = square(0.0, 0.0, 90.0);
        let hole = square(30.0, 30.0, 30.0);

        let mut areas = Vec::new();

        for flip_outer in [false, true] {
            for flip_hole in [false, true] {
                let mut outline: Vec<Vertex> = outer.to_vec();
                if flip_outer {
                    outline.reverse();
                }

                let mut hole = hole.to_vec();
                if flip_hole {
                    hole.reverse();
                }
                outline.extend_from_slice(&hole);

                let mut triangles = Vec::new();
                fill(&outline, &[0, 4], &mut triangles);
                areas.push(total_area(&triangles));
            }
        }

        for area in &areas {
            assert!((area - (90.0 * 90.0 - 30.0 * 30.0)).abs() < 1e-2, "{area}");
        }
    }

    /// 反時計回りの三角形の**内部**に点があるか。辺の上は含めない。
    ///
    /// 隣り合う三角形の継ぎ目に乗った点をどちらにも数えないので、
    /// 重ね塗りを数えるのに都合がよい。
    fn point_in_triangle(a: Vertex, b: Vertex, c: Vertex, point: Vertex) -> bool {
        cross(a, b, point) > 0.0 && cross(b, c, point) > 0.0 && cross(c, a, point) > 0.0
    }

    /// 点が何枚の三角形に覆われているか。0 = 塗り残し、2 以上 = 重ね塗り。
    ///
    /// 辺の真上に乗った点は、どちらの三角形にも数えない（`point_in_triangle` が
    /// 辺を含まないため）。判定は向きに依存しないよう両方の巻きで試す。
    fn coverage(triangles: &[Vertex], point: Vertex) -> usize {
        triangles
            .chunks_exact(3)
            .filter(|triangle| {
                let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
                point_in_triangle(a, b, c, point) || point_in_triangle(a, c, b, point)
            })
            .count()
    }

    /// 面積だけでは「穴を塗ったぶん」と「どこかの塗り残し」が打ち消し合う。
    /// 格子で数えれば、重ね塗りも塗り残しも直接見える。
    ///
    /// 穴が複数あると通路どうしが交差しやすく、ここがいちばん壊れやすい。
    #[test]
    fn several_holes_leave_no_overlap_and_no_gap() {
        let holes = [
            (10.0, 10.0, 20.0),
            (60.0, 10.0, 20.0),
            // わざと他の穴と同じ高さに置く。通路の判定が辺の上を
            // 見落とすと、ここで穴を突き抜ける。
            (35.0, 60.0, 20.0),
        ];

        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 100.0));
        for &(x, y, size) in &holes {
            outline.extend_from_slice(&square(x, y, size));
        }

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4, 8, 12], &mut triangles);

        assert_triangle_list(&triangles);

        let mut inside_hole = 0;
        let mut y = 0.5;

        while y < 100.0 {
            let mut x = 0.5;

            while x < 100.0 {
                let in_hole = holes
                    .iter()
                    .any(|&(hx, hy, size)| x > hx && x < hx + size && y > hy && y < hy + size);

                let covered = coverage(&triangles, point(x, y));

                if in_hole {
                    assert_eq!(covered, 0, "穴の中 ({x}, {y}) が塗られている");
                    inside_hole += 1;
                } else {
                    assert!(covered <= 1, "({x}, {y}) が {covered} 枚に重ね塗りされている");
                }

                x += 1.0;
            }

            y += 1.0;
        }

        assert_eq!(inside_hole, 3 * 20 * 20);
    }

    /// 耳刈り取りにしたので、凹多角形もそのまま塗れる。
    /// ファンのままなら、頂点 0 から見えない部分がはみ出していた。
    #[test]
    fn a_concave_outline_is_filled_correctly() {
        // L 字。面積は 100x100 から 50x50 を欠いた 7500。
        let outline = [
            point(0.0, 0.0),
            point(100.0, 0.0),
            point(100.0, 50.0),
            point(50.0, 50.0),
            point(50.0, 100.0),
            point(0.0, 100.0),
        ];

        let mut triangles = Vec::new();
        fill(&outline, &[0], &mut triangles);

        assert_triangle_list(&triangles);
        assert_eq!(triangles.len(), 3 * (outline.len() - 2));
        assert!((total_area(&triangles) - 7500.0).abs() < 1e-2);

        // 欠けた側に三角形が出ていないこと。
        let missing = point(75.0, 75.0);
        for triangle in triangles.chunks_exact(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
            let covered = point_in_triangle(a, b, c, missing) || point_in_triangle(a, c, b, missing);
            assert!(!covered, "凹んだ部分が塗られている");
        }
    }

    /// 決まった種から同じ列を出す線形合同法。テストを再現可能にするため。
    fn random_source() -> impl FnMut() -> f32 {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;

        move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32) / (u32::MAX >> 1) as f32
        }
    }

    /// 原点から見て全方向に 1 点ずつ置いた輪郭。
    ///
    /// 円を `sides` 等分し、各扇の中に 1 点だけ置くので角度が必ず単調に増え、
    /// 原点が必ず内側に入る。つまり原点について星型 = 自己交差しない。
    /// 半径は振るので、凹んだ形も凸な形も出る。
    ///
    /// 角度をただランダムに撒いて並べ替えるのでは駄目で、点が一方に偏ると
    /// 原点が外へ出て自己交差する。
    fn simple_polygon(next: &mut impl FnMut() -> f32, sides: usize) -> Vec<Vertex> {
        let sector = std::f32::consts::TAU / sides as f32;

        (0..sides)
            .map(|index| {
                // 扇の境界は避ける。隣と角度が並ぶと潰れた辺になる。
                let angle = (index as f32 + 0.1 + 0.8 * next()) * sector;
                let radius = 10.0 + next() * 40.0;
                point(angle.cos() * radius, angle.sin() * radius)
            })
            .collect()
    }

    /// 自己交差しない輪郭なら、塗った面積は輪郭の面積とぴったり一致する。
    ///
    /// 耳が見つからずに打ち切られると、輪郭の外まで塗るので面積がずれる。
    /// つまりこのテストは「耳刈り取りが最後まで走りきること」の見張りでもある。
    #[test]
    fn simple_outlines_are_filled_exactly() {
        let mut next = random_source();

        for _ in 0..2000 {
            let sides = 3 + (next() * 10.0) as usize;
            let outline = simple_polygon(&mut next, sides);

            let mut triangles = Vec::new();
            fill(&outline, &[0], &mut triangles);

            let expected = signed_area(&outline).abs();
            let actual = total_area(&triangles);

            assert!(
                (actual - expected).abs() < expected * 1e-3 + 1e-3,
                "{sides} 角形で {actual} と {expected} がずれた",
            );
        }
    }

    /// 自己交差していても、必ず止まり、必ず輪郭ぶんの三角形を出す。
    ///
    /// 対角線で 2 つに割ると両側が元より短くなるので、割りは必ず終わる。
    /// 頂点 n の輪郭を m1 + m2 = n + 2 に割ると三角形は
    /// `(m1 - 2) + (m2 - 2) = n - 2` 枚で、割る前と変わらない。
    #[test]
    fn broken_outlines_still_terminate() {
        let mut next = random_source();

        for _ in 0..5000 {
            // 角度順に並べないので、ほとんどが自己交差する。
            let sides = 4 + (next() * 9.0) as usize;
            let outline: Vec<Vertex> = (0..sides)
                .map(|_| point(next() * 100.0, next() * 100.0))
                .collect();

            let mut triangles = Vec::new();
            fill(&outline, &[0], &mut triangles);

            assert_triangle_list(&triangles);
            // 面積の無い残りは捨てるので、ぴったり n-2 枚とは限らない。
            // 超えることだけは無い。
            assert!(
                triangles.len() <= 3 * (sides - 2),
                "{sides} 角形で三角形が多すぎる（{}）",
                triangles.len(),
            );
        }
    }

    /// 穴をつないだ通路の残りかすで警告を出さない。
    ///
    /// 面積が無い部分を三角形にしても、画面には何も出ない。
    /// 出力が「面積の有る三角形だけ」になっていることを見る。
    #[test]
    fn a_degenerate_remainder_is_dropped_silently() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 90.0));
        outline.extend_from_slice(&square(30.0, 30.0, 30.0));

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4], &mut triangles);

        let expected = 90.0 * 90.0 - 30.0 * 30.0;
        assert!((total_area(&triangles) - expected).abs() < 1e-2);

        // 面積 0 の三角形が混ざっていないこと。
        for triangle in triangles.chunks_exact(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
            let area = ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5;

            assert!(area > 1e-6, "面積の無い三角形が残っている");
        }
    }

    /// 自己交差した輪郭は、止まりはするが**正しくは塗れない**。
    ///
    /// ねじれた四角は (37.5, 37.5) で交差し、房の面積は 1125 と 3125。
    /// 交差点を頂点として持たない以上、耳刈り取りでは房を分けられない。
    /// 正しく塗るには辺どうしの交点を求めて輪郭を切り直す必要があり、
    /// それは耳刈り取りとは別の仕組みになる。ここでは「落ちない」ことだけを固定する。
    #[test]
    fn a_self_intersecting_outline_is_not_corrected() {
        let outline = [
            point(0.0, 0.0),
            point(100.0, 100.0),
            point(100.0, 0.0),
            point(0.0, 60.0),
        ];

        let mut triangles = Vec::new();
        fill(&outline, &[0], &mut triangles);

        assert_triangle_list(&triangles);
        assert_eq!(triangles.len(), 3 * (outline.len() - 2));

        // 房の合計 4250 とは一致しない。これは既知の制限。
        assert!(total_area(&triangles) > 4250.0);
    }

    /// 円周上に頂点を並べた正 n 角形。
    fn polygon(radius: f32, sides: usize) -> Vec<Vertex> {
        (0..sides)
            .map(|index| {
                let angle = index as f32 * std::f32::consts::TAU / sides as f32;
                point(angle.cos() * radius, angle.sin() * radius)
            })
            .collect()
    }

    /// 正 n 角形の面積。
    fn polygon_area(radius: f32, sides: usize) -> f32 {
        0.5 * sides as f32 * radius * radius * (std::f32::consts::TAU / sides as f32).sin()
    }

    /// いちばん使うであろう形。同心の穴を持つリング。
    #[test]
    fn a_concentric_ring_keeps_its_area() {
        let mut outline = polygon(17.0, 24);
        let boundary = outline.len();
        outline.extend(polygon(8.0, 24));

        let mut triangles = Vec::new();
        fill(&outline, &[0, boundary], &mut triangles);

        assert_triangle_list(&triangles);

        let expected = polygon_area(17.0, 24) - polygon_area(8.0, 24);
        assert!(
            (total_area(&triangles) - expected).abs() < 1e-1,
            "{} vs {expected}",
            total_area(&triangles),
        );

        // 中心の穴が塗られていないこと。
        assert_eq!(coverage(&triangles, point(0.0, 0.0)), 0);

        // 帯の上はどこを取っても 1 枚だけ。
        // +X 軸の上は穴を外周につないだ通路が通るので避ける。
        for step in 0..24 {
            let angle = (step as f32 + 0.5) * std::f32::consts::TAU / 24.0;
            let on_band = point(angle.cos() * 12.5, angle.sin() * 12.5);
            assert_eq!(coverage(&triangles, on_band), 1, "step {step}");
        }
    }

    /// 5 芒星。中心に頂点を置かずに外周をなぞるだけで塗れる。
    /// ファンのままでは、隣り合う先端の間がはみ出していた。
    #[test]
    fn a_five_pointed_star_needs_no_centre_vertex() {
        let outline: Vec<Vertex> = (0..10)
            .map(|index| {
                let angle = index as f32 * std::f32::consts::TAU / 10.0;
                let radius = if index % 2 == 0 { 17.0 } else { 7.5 };
                point(angle.cos() * radius, angle.sin() * radius)
            })
            .collect();

        let mut triangles = Vec::new();
        fill(&outline, &[0], &mut triangles);

        assert_triangle_list(&triangles);
        assert_eq!(triangles.len(), 3 * (outline.len() - 2));

        // 中心から 10 枚の三角形に切り分けた合計。
        let expected = 10.0 * 0.5 * 17.0 * 7.5 * (std::f32::consts::TAU / 10.0).sin();
        assert!(
            (total_area(&triangles) - expected).abs() < 1e-2,
            "{} vs {expected}",
            total_area(&triangles),
        );

        // 先端と先端の間の窪みは塗られていないこと。
        // 角度 TAU/10 の方向、半径 12 は窪みの外側にあたる。
        let angle = std::f32::consts::TAU / 10.0;
        let outside = point(angle.cos() * 12.0, angle.sin() * 12.0);
        assert_eq!(coverage(&triangles, outside), 0, "窪みが塗られている");
    }

    /// 穴 1 つにつき頂点が 2 つ増える（通路の行き帰り）。三角形の数はそこから決まる。
    #[test]
    fn the_triangle_count_follows_from_the_contours() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 90.0));
        outline.extend_from_slice(&square(30.0, 30.0, 30.0));

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4], &mut triangles);

        // 頂点 4 + 4 + 通路 2 = 10 → 三角形 8 枚。
        assert_eq!(triangles.len(), 3 * (4 + 4 + 2 - 2));
    }

    /// 線は輪郭ごとに引かれる。穴の縁にも線が付く。
    #[test]
    fn a_stroke_traces_every_contour() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 90.0));
        outline.extend_from_slice(&square(30.0, 30.0, 30.0));

        let paint_type = PaintType::Stroke {
            line_width: 4.0,
            joint_type: JointType::Miter,
            strip: false,
        };

        let mut one = Vec::new();
        tessellate(&outline[..4], &[0], paint_type, &mut one);

        let mut both = Vec::new();
        tessellate(&outline, &[0, 4], paint_type, &mut both);

        // 同じ頂点数・同じ角数の輪郭が 2 本なので、ちょうど倍になる。
        assert_eq!(both.len(), one.len() * 2);
    }

    /// 3 頂点に満たない輪郭は無視する。区切りだけ置いても壊れない。
    #[test]
    fn an_empty_hole_is_ignored() {
        let outline = square(0.0, 0.0, 90.0);

        let mut plain = Vec::new();
        fill(&outline, &[0], &mut plain);

        let mut with_markers = Vec::new();
        fill(&outline, &[0, 4, 4, 4], &mut with_markers);

        assert_eq!(plain.len(), with_markers.len());
        assert!(!plain.is_empty());
    }

    #[test]
    fn degenerate_input_produces_nothing() {
        let mut triangles = Vec::new();

        fill(&[point(0.0, 0.0), point(1.0, 0.0)], &[0], &mut triangles);
        assert!(triangles.is_empty(), "a fill needs 3 vertices");

        stroke(&[point(0.0, 0.0)], 2.0, JointType::Miter, true, &mut triangles);
        assert!(triangles.is_empty(), "a stroke needs 2 vertices");

        stroke(
            &[point(0.0, 0.0), point(10.0, 0.0)],
            0.0,
            JointType::Miter,
            true,
            &mut triangles,
        );
        assert!(triangles.is_empty(), "a zero-width stroke draws nothing");
    }
}
