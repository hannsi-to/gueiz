//! 輪郭を三角形リストに開く。
//!
//! 塗りも線もここを通って三角形になるので、描画側はトポロジを 1 つしか持たない。
//! ドローコールを増やさずに線を引けるのはこのため。

use std::f32::consts::PI;
use std::ops::Range;

use fxhash::FxHashMap;

use crate::paint_type::{Dash, JointType, PaintType};
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

        PaintType::FillNonZero => fill_nonzero(outline, contour_starts, triangles),

        PaintType::Stroke {
            line_width,
            joint_type,
            strip,
            dash,
        } => {
            // 線は輪郭ごとに独立して引く。穴の縁にも同じ太さの線が付く。
            //
            // 塗りと同じく長さ 0 の辺を落とす。残すと法線が出せず、
            // その継ぎ目だけ帯が飛ぶ。閉じた輪のときだけ末尾の重複も落とす
            // （開いた折れ線では、始点に戻る線分は消してはいけない）。
            for contour in contours(outline, contour_starts, 2, !strip) {
                let Some(dash) = dash else {
                    stroke(&contour, line_width, joint_type, strip, triangles);
                    continue;
                };

                // 刻んだ破片はどれも開いた折れ線。輪であっても切れ目ができる。
                // 模様は輪郭ごとに頭から数え直す。
                //
                // 破片の端は元の線の途中なので、折れ線全体の端を見る
                // `JointType` では届かない。模様の指定で寄せる。
                let ends = if dash.has_round_ends() {
                    joint_type.with_round_ends()
                } else {
                    joint_type
                };

                for run in dash_runs(&contour, strip, dash) {
                    stroke(&run, line_width, ends, true, triangles);
                }
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
    // 長さ 0 の辺を落としてから数える。落とすと 3 点を切る輪郭があるので、
    // 先に数えて捨ててしまうと「畳めば形になる輪郭」を取り落とす。
    let contours = contours(outline, contour_starts, 3, true);

    if contours.is_empty() {
        return;
    }

    // どれが外周でどれが穴かは、**包含関係から決める**。
    // 順番で決め打つと、`i` や `=` のように離れた輪郭が 2 つある形で
    // 片方が穴にされて消える。
    // つなげなかった穴。**捨てずに、それぞれ単独で塗る。**
    let mut stray = Vec::new();

    for group in group_contours(&contours) {
        fill_group(&contours, &group, triangles, &mut stray);
    }

    // つなげないということは、外周の中に無かったということです。
    // 包含関係の判定は輪郭の 1 点だけで見るので、**重なっているだけの輪郭**を
    // 穴と取り違えることがあります。`Ç` の下の飾りや `Å` の上の輪は、
    // 本体と少しだけ重なった別の形で、穴ではありません。
    //
    // 取り違えたまま捨てると飾りが消えます。単独で塗れば、本体と重なるぶんは
    // 二重に塗られますが、1 色の形なら見た目は正しく出ます。
    for index in stray {
        let mut polygon = contours[index].clone();

        if signed_area(&polygon) < 0.0 {
            polygon.reverse();
        }

        ear_clip(&polygon, triangles);
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

        for (outer, contour) in contours.iter().enumerate() {
            if outer == inner || !contour_contains(contour, probe) {
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

        for (outer, contour) in contours.iter().enumerate() {
            if outer == inner || !contour_contains(contour, probe) {
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
        if depth[index].is_multiple_of(2) {
            slot[index] = Some(groups.len());
            groups.push(ContourGroup {
                outer: index,
                holes: Vec::new(),
            });
        }
    }

    for index in 0..count {
        if depth[index].is_multiple_of(2) {
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
///
/// つなげなかった穴の番号を `stray` に積む。呼ぶ側が単独で塗り直す。
fn fill_group(
    contours: &[Vec<Vertex>],
    group: &ContourGroup,
    triangles: &mut Vec<Vertex>,
    stray: &mut Vec<usize>,
) {
    let mut polygon = contours[group.outer].clone();

    // 巻き方向はユーザーに要求しない。外周を反時計回りに、穴をその逆に揃える。
    // 任せると「穴が塗りつぶされる」類のバグを必ず踏む。
    if signed_area(&polygon) < 0.0 {
        polygon.reverse();
    }

    let mut holes: Vec<(usize, Vec<Vertex>)> = group
        .holes
        .iter()
        .map(|&index| (index, contours[index].clone()))
        .collect();

    for (_, hole) in &mut holes {
        if signed_area(hole) > 0.0 {
            hole.reverse();
        }
    }

    // 右にある穴から順につなぐ。先につないだ通路が後の探索を邪魔しない。
    holes.sort_by(|(_, a), (_, b)| rightmost_x(b).total_cmp(&rightmost_x(a)));

    for (index, hole) in &holes {
        if !bridge_hole(&mut polygon, hole) {
            stray.push(*index);
        }
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
fn contour_contains(contour: &[Vertex], point: Vertex) -> bool {
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
///
/// 点を落とさないので、[`contours`] と違って**写しを作りません**。
/// 長さ 0 の辺が混じっていても気にしない処理だけがこちらを使います。
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

/// 輪郭を 1 本ずつ取り出して、**長さ 0 の辺を落とす**。
///
/// # なぜ落とすのか
///
/// ここから下は輪郭を暗黙に閉じて扱う（`(index + 1) % count`）ので、
/// 同じ場所に 2 つ点があると長さ 0 の辺ができます。耳刈り取りはその辺を
/// 潰せず、面積 0 の三角形を耳と認めないので、**耳が 1 つも見つからない
/// 状態に落ちることがあります。**
///
/// # 1 点目と同じ末尾の点
///
/// 字形の輪郭は**最後の点が 1 点目と同じ**です
/// （`line_to` で始点に戻って閉じる）。ここが今回の落とし穴でした。
///
/// - 連続する重複は前から順に見れば落ちる
/// - **末尾が 1 点目と同じ**ものは、直前の点とは違うので前から見ても落ちない
///
/// 外周と穴の両方にこれが残り、かつ穴が 2 つ以上あると、穴をつないだあとの
/// 輪郭で耳が見つからなくなって `面` や `B` が崩れていました。
///
/// # 閉じている輪郭だけ
///
/// `closed` を倒すと末尾の重複を残します。開いた折れ線
/// （[`PaintType::Stroke`] の `strip`）では、始点に戻る最後の点は
/// **消してはいけない線分**だからです。落とすと一辺足りない線になります。
fn contours(
    outline: &[Vertex],
    contour_starts: &[usize],
    minimum: usize,
    closed: bool,
) -> Vec<Vec<Vertex>> {
    // 区切りだけ先に出す。点を落とすと数が変わるので、`minimum` は後で見る。
    contour_ranges(outline, contour_starts, 1)
        .into_iter()
        .map(|range| dedup_contour(&outline[range], closed))
        .filter(|contour| contour.len() >= minimum)
        .collect()
}

/// 同じ場所に続く点を 1 つに畳む。`closed` なら 1 点目と同じ末尾も落とす。
fn dedup_contour(contour: &[Vertex], closed: bool) -> Vec<Vertex> {
    let mut kept: Vec<Vertex> = Vec::with_capacity(contour.len());

    for &vertex in contour {
        if kept.last().is_some_and(|last| same_place(*last, vertex)) {
            continue;
        }

        kept.push(vertex);
    }

    // 閉じた輪郭なら、1 点目へ戻る点は辺を 1 本も増やさない。
    // 畳んだ結果が 2 点以下になるものは残しても形にならないので、そのまま返す。
    while closed
        && kept.len() > 1
        && same_place(kept[0], *kept.last().expect("空ではない"))
    {
        kept.pop();
    }

    kept
}

/// 2 点が同じ場所か。
///
/// 距離の 2 乗で見る。`1e-12` は em 単位（1.0 = 1 em）で 1/1000000 em
/// 相当なので、字形の折れ線が意図して置く点より細かい。
/// 画素座標で使っても、1 画素の百万分の 1 より近い点しか畳まない。
fn same_place(left: Vertex, right: Vertex) -> bool {
    let dx = left.x - right.x;
    let dy = left.y - right.y;

    dx * dx + dy * dy < 1e-12
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

/// 点が図形の中にあるか。**開いた三角形で判定する。**
///
/// # なぜ輪郭ではなく三角形なのか
///
/// 輪郭で判定すると、**描かれているものと食い違います。**
///
/// - **線**（[`crate::paint_type::PaintType::Stroke`]）は、輪郭の内側ではなく
///   **帯の上**が図形です。輪郭で見ると、何も塗っていない真ん中が「中」になる
/// - **穴**は三角形が張られないので、三角形で見れば自動的に外になる。
///   輪郭で見るなら、偶奇か巻き数の規則を別に決めることになる
///
/// 三角形は[描いたものそのもの](crate::object::Object::triangles)なので、
/// **見えているとおりに当たります。**
///
/// 座標は図形のローカル。変換を通した判定は
/// [`crate::object::Object::hit`] を使います。
///
/// ```
/// # use gueiz_2d::tessellate::contains;
/// # use gueiz_2d::vertex::Vertex;
/// let at = |x: f32, y: f32| Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0);
/// // 10x10 の四角を 2 枚の三角形で。
/// let triangles = [
///     at(0.0, 0.0), at(10.0, 0.0), at(10.0, 10.0),
///     at(0.0, 0.0), at(10.0, 10.0), at(0.0, 10.0),
/// ];
///
/// assert!(contains(&triangles, 5.0, 5.0));
/// assert!(!contains(&triangles, 15.0, 5.0));
/// ```
pub fn contains(triangles: &[Vertex], x: f32, y: f32) -> bool {
    let point = Vertex::new_position_color(x, y, 0.0, 0.0, 0.0, 0.0, 0.0);

    triangles
        .chunks_exact(3)
        .any(|corner| inside_triangle(corner[0], corner[1], corner[2], point))
}

/// 三角形の内側に点があるか。**巻き方向を問わず、辺の上も内側**。
///
/// 巻き方向を決め打ちにしないのは、[`fill`] と [`stroke`] で
/// 出てくる向きが揃っているとは限らないため。3 つの外積の符号が
/// 揃っていれば内側、割れていれば外側。
fn inside_triangle(a: Vertex, b: Vertex, c: Vertex, point: Vertex) -> bool {
    let sides = [cross(a, b, point), cross(b, c, point), cross(c, a, point)];

    // 0 はどちらにも数えない。辺の上はこれで内側になる。
    !(sides.iter().any(|side| *side < 0.0) && sides.iter().any(|side| *side > 0.0))
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
///
/// つなげたら `true`。つなげないのは、その輪郭が外周の中に無いときです
/// （[`fill`] が単独で塗り直します）。
fn bridge_hole(polygon: &mut Vec<Vertex>, hole: &[Vertex]) -> bool {
    let Some(hole_index) = (0..hole.len()).reduce(|best, index| {
        if hole[index].x > hole[best].x {
            index
        } else {
            best
        }
    }) else {
        return false;
    };

    let Some(bridge_index) = open_bridge(polygon, hole[hole_index]) else {
        // 右に外周が無い。この輪郭は穴ではなく、重なっているだけの別の形。
        return false;
    };

    let mut spliced = Vec::with_capacity(polygon.len() + hole.len() + 2);
    spliced.extend_from_slice(&polygon[..=bridge_index]);
    // 穴を `hole_index` から一周して同じ頂点に戻る。
    spliced.extend_from_slice(&hole[hole_index..]);
    spliced.extend_from_slice(&hole[..=hole_index]);
    spliced.extend_from_slice(&polygon[bridge_index..]);

    *polygon = spliced;

    true
}

/// 穴の右端から +X に線を飛ばし、**当たった点そのものを外周に差し込んで**
/// 通路の行き先にする。差し込んだ位置を返す。
///
/// # なぜ頂点に寄せないのか
///
/// 以前は「当たった辺の、より右にある端点」へ寄せていました。これだと
/// **当たった高さが無視される**ので、縦の辺に複数の穴が当たったとき、
/// どの穴も同じ頂点へつながります。
///
/// ```text
///   穴A ─→│        3 本の通路が 1 つの頂点に集まり、
///   穴B ─→│  ←同じ縦の辺   行きと帰りが互いを跨いでしまう
///   穴C ─→│
/// ```
///
/// 通路が交差した輪郭には耳が 1 つも無く、`ear clipping stalled` で崩れます。
/// 游ゴシックの `面`（縦に並んだ穴 3 つの右に別の穴）や Arial の `B` が
/// これでした。
///
/// 当たった点を辺の上に差し込めば、穴ごとに**別の点**へつながります。
/// 差し込む点は辺の上なので形は変わらず、通路は高さ順に並ぶので交差しません。
/// 線が最初に当たる辺を選んでいるので、穴の右端からその点までのあいだに
/// 外周はありません（**見えていることが保証されている**）。
/// 寄せる先を探す必要が無くなったので、凹んだ頂点を避ける手当ても消えました。
fn open_bridge(polygon: &mut Vec<Vertex>, from: Vertex) -> Option<usize> {
    let count = polygon.len();

    // 線が最初に当たる辺を探す。
    let mut hit = None;

    for index in 0..count {
        let start = polygon[index];
        let end = polygon[(index + 1) % count];

        // 辺が from.y をまたがなければ当たらない。
        // 長さ 0 の辺（先につないだ通路の継ぎ目）もここで落ちる。
        if (start.y > from.y) == (end.y > from.y) {
            continue;
        }

        let ratio = (from.y - start.y) / (end.y - start.y);
        let x = start.x + ratio * (end.x - start.x);

        if x > from.x && hit.is_none_or(|(_, hit_x, _)| x < hit_x) {
            hit = Some((index, x, ratio));
        }
    }

    let (edge, hit_x, ratio) = hit?;
    let next_edge = (edge + 1) % count;

    // 当たった点。色や uv は辺の上で混ぜる。端点の値をそのまま使うと、
    // 差し込んだところで色が飛ぶ。
    let mut point = lerp_vertex(polygon[edge], polygon[next_edge], ratio);
    // x は混ぜた値ではなく、交点の計算結果をそのまま入れる。
    // 混ぜ算の丸めで辺から僅かに外れると、通路が外へ出る。
    point.x = hit_x;
    point.y = from.y;

    // ちょうど端点に当たったなら差し込まない。同じ場所に 2 つ置くと
    // 長さ 0 の辺になり、いま直したばかりの詰まりに戻る。
    if same_place(polygon[edge], point) {
        return Some(edge);
    }

    if same_place(polygon[next_edge], point) {
        return Some(next_edge);
    }

    // 辺の途中に差し込む。`edge` が末尾なら、末尾に足すのが「末尾と先頭の
    // あいだ」になる。
    polygon.insert(edge + 1, point);

    Some(edge + 1)
}

/// 2 点のあいだを混ぜる。位置だけでなく色や uv も混ぜる。
fn lerp_vertex(start: Vertex, end: Vertex, ratio: f32) -> Vertex {
    let mix = |a: f32, b: f32| a + (b - a) * ratio;

    Vertex::new_position_color_uv_normal(
        mix(start.x, end.x),
        mix(start.y, end.y),
        mix(start.z, end.z),
        mix(start.r, end.r),
        mix(start.g, end.g),
        mix(start.b, end.b),
        mix(start.a, end.a),
        mix(start.u, end.u),
        mix(start.v, end.v),
        mix(start.n_x, end.n_x),
        mix(start.n_y, end.n_y),
        mix(start.n_z, end.n_z),
    )
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

/// 輪郭の内側を**非ゼロ巻き数**で塗る。[`PaintType::FillNonZero`] の中身。
///
/// # 帯に割って塗る
///
/// 耳刈り取りは「1 本の単純な輪郭」しか潰せないので、重なりや交差は
/// 扱えません。こちらは形を**横の帯**に割ります。
///
/// 1. 頂点の高さと、辺どうしの交点の高さで帯を区切る。
///    **帯の中では辺が交わらない**ので、左から順に並べられる
/// 2. 帯ごとに左から辺を数え、巻き数が 0 でない区間を拾う
/// 3. 上の帯から同じ 2 辺で続く区間は 1 枚の台形にまとめる。
///    まとめないと、別の場所の頂点で割られた細切れの台形が山ほど出る
///
/// 台形の数はおおむね辺の数に比例します。
///
/// # 継ぎ目
///
/// 上下に並んだ台形の幅が違うと、片方の角がもう片方の上辺の途中に乗ります
/// （T 字の継ぎ目）。そのままだと上辺と下辺が辺として一致せず、
/// [`crate::clip`] の距離場が中に境目を見てしまいます。
/// そこで**同じ高さに乗っている角を上辺・下辺に差し込んでから**三角形にします。
pub fn fill_nonzero(outline: &[Vertex], contour_starts: &[usize], triangles: &mut Vec<Vertex>) {
    let edges = winding_edges(outline, contour_starts);

    if edges.is_empty() {
        return;
    }

    let levels = sweep_levels(&edges);
    let trapezoids = sweep_trapezoids(&edges, &levels);

    // 高さごとに、その高さに乗っている角の x を集める。
    let mut corners: FxHashMap<u32, Vec<f32>> = FxHashMap::default();

    for trapezoid in &trapezoids {
        for y in [trapezoid.top, trapezoid.bottom] {
            corners.entry(y.to_bits()).or_default().extend([
                edges[trapezoid.left].at(y).x,
                edges[trapezoid.right].at(y).x,
            ]);
        }
    }

    for xs in corners.values_mut() {
        xs.sort_by(f32::total_cmp);
        xs.dedup();
    }

    for trapezoid in &trapezoids {
        let top = chain(&edges, trapezoid, trapezoid.top, &corners);
        let bottom = chain(&edges, trapezoid, trapezoid.bottom, &corners);

        zip_chains(&top, &bottom, triangles);
    }
}

/// 巻き数を数えるための辺。**上の端を `top` に揃えてある。**
#[derive(Clone, Copy, Debug)]
struct WindingEdge {
    top: Vertex,
    bottom: Vertex,
    /// 下向きに辿る辺なら +1、上向きなら -1。
    winding: i32,
}

impl WindingEdge {
    /// 高さ `y` での辺の上の点。色や uv も混ぜる。
    ///
    /// 端ちょうどなら端点そのものを返す。混ぜ算の丸めで端がずれると、
    /// 隣の台形と角が合わなくなる。
    fn at(&self, y: f32) -> Vertex {
        if y <= self.top.y {
            return self.top;
        }

        if y >= self.bottom.y {
            return self.bottom;
        }

        let mut point = lerp_vertex(self.top, self.bottom, (y - self.top.y) / (self.bottom.y - self.top.y));
        point.y = y;
        point
    }

    fn min_x(&self) -> f32 {
        self.top.x.min(self.bottom.x)
    }

    fn max_x(&self) -> f32 {
        self.top.x.max(self.bottom.x)
    }
}

/// 輪郭を辺に分ける。水平な辺は帯を横切らないので巻き数に効かず、要らない。
fn winding_edges(outline: &[Vertex], contour_starts: &[usize]) -> Vec<WindingEdge> {
    let mut edges = Vec::new();

    for contour in contours(outline, contour_starts, 3, true) {
        let count = contour.len();

        for index in 0..count {
            let from = contour[index];
            let to = contour[(index + 1) % count];

            if from.y < to.y {
                edges.push(WindingEdge { top: from, bottom: to, winding: 1 });
            } else if from.y > to.y {
                edges.push(WindingEdge { top: to, bottom: from, winding: -1 });
            }
        }
    }

    edges
}

/// 帯の区切りの高さ。頂点の高さと、辺どうしが交わる高さ。昇順で重複なし。
///
/// 交点は x の範囲が重なる組だけ調べる。字を横に並べた文字列でも、
/// 遠い字どうしは比べずに済む。
fn sweep_levels(edges: &[WindingEdge]) -> Vec<f32> {
    let mut levels: Vec<f32> = edges
        .iter()
        .flat_map(|edge| [edge.top.y, edge.bottom.y])
        .collect();

    let mut by_x: Vec<usize> = (0..edges.len()).collect();
    by_x.sort_by(|a, b| edges[*a].min_x().total_cmp(&edges[*b].min_x()));

    for (position, &first) in by_x.iter().enumerate() {
        let a = edges[first];

        for &second in &by_x[position + 1..] {
            let b = edges[second];

            if b.min_x() > a.max_x() {
                break;
            }

            if b.top.y >= a.bottom.y || a.top.y >= b.bottom.y {
                continue;
            }

            if let Some(y) = crossing_height(a, b) {
                levels.push(y);
            }
        }
    }

    levels.sort_by(f32::total_cmp);
    levels.dedup();
    levels
}

/// 2 辺が**両方の内側で**交わるなら、その高さ。端で触れるだけなら `None`。
fn crossing_height(a: WindingEdge, b: WindingEdge) -> Option<f32> {
    let r = [a.bottom.x - a.top.x, a.bottom.y - a.top.y];
    let s = [b.bottom.x - b.top.x, b.bottom.y - b.top.y];
    let denominator = r[0] * s[1] - r[1] * s[0];

    // 平行。重なっていても、同じ x に並ぶだけなので帯を割る必要は無い。
    if denominator == 0.0 {
        return None;
    }

    let offset = [b.top.x - a.top.x, b.top.y - a.top.y];
    let t = (offset[0] * s[1] - offset[1] * s[0]) / denominator;
    let u = (offset[0] * r[1] - offset[1] * r[0]) / denominator;

    (t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0).then(|| a.top.y + t * r[1])
}

/// 塗る台形 1 枚。左右の辺と、上下の高さ。
#[derive(Clone, Copy, Debug)]
struct Trapezoid {
    left: usize,
    right: usize,
    top: f32,
    bottom: f32,
}

/// 帯を上から順に見て、塗る台形を拾う。
fn sweep_trapezoids(edges: &[WindingEdge], levels: &[f32]) -> Vec<Trapezoid> {
    let mut by_top: Vec<usize> = (0..edges.len()).collect();
    by_top.sort_by(|a, b| edges[*a].top.y.total_cmp(&edges[*b].top.y));

    let mut next = 0;
    let mut active: Vec<usize> = Vec::new();
    // 上の帯から続いている台形。下の端はまだ決まっていない。
    let mut open: Vec<Trapezoid> = Vec::new();
    let mut trapezoids = Vec::new();

    for band in levels.windows(2) {
        let (top, bottom) = (band[0], band[1]);

        while next < by_top.len() && edges[by_top[next]].top.y <= top {
            active.push(by_top[next]);
            next += 1;
        }

        // 区切りには全頂点の高さが入っているので、ここで終わらない辺は帯を貫く。
        active.retain(|&edge| edges[edge].bottom.y > top);

        // 帯の中では辺が交わらないので、真ん中の高さで並べれば帯じゅうその順。
        let middle = (top + bottom) * 0.5;
        active.sort_by(|a, b| edges[*a].at(middle).x.total_cmp(&edges[*b].at(middle).x));

        let mut still_open = Vec::with_capacity(open.len());
        let mut winding = 0;
        let mut left = None;

        for &edge in &active {
            let before = winding;
            winding += edges[edge].winding;

            if before == 0 && winding != 0 {
                left = Some(edge);
            } else if before != 0 && winding == 0 {
                let left = left.take().expect("塗り始めの辺がある");

                // 同じ 2 辺で上から続いているなら、伸ばすだけ。
                let top = open
                    .iter()
                    .position(|span| span.left == left && span.right == edge)
                    .map_or(top, |position| open.swap_remove(position).top);

                still_open.push(Trapezoid { left, right: edge, top, bottom });
            }
        }

        // 続かなかったものは、この帯の上で閉じる。
        trapezoids.extend(open.drain(..).map(|span| Trapezoid { bottom: top, ..span }));
        open = still_open;
    }

    trapezoids.extend(open);
    trapezoids
}

/// 台形の上辺（または下辺）。左の角から右の角まで、途中に乗る角を差し込む。
fn chain(
    edges: &[WindingEdge],
    trapezoid: &Trapezoid,
    y: f32,
    corners: &FxHashMap<u32, Vec<f32>>,
) -> Vec<Vertex> {
    let left = edges[trapezoid.left].at(y);
    let right = edges[trapezoid.right].at(y);

    let mut chain = vec![left];

    if let Some(xs) = corners.get(&y.to_bits()) {
        let width = right.x - left.x;

        for &x in xs.iter().filter(|x| **x > left.x && **x < right.x) {
            let mut point = lerp_vertex(left, right, (x - left.x) / width);
            point.x = x;
            point.y = y;
            chain.push(point);
        }
    }

    // 尖った先では左右の角が重なる。2 つ置くと面積の無い三角形ができ、
    // 距離場が、そこで背中合わせになった辺を打ち消し合って境目を見落とす。
    if !same_position(left, right) {
        chain.push(right);
    }

    chain
}

/// 上辺と下辺を左から綴じて三角形にする。どちらも左から右へ並んでいる前提。
///
/// x の小さいほうを先に進めるので、細長い三角形になりにくい。
fn zip_chains(top: &[Vertex], bottom: &[Vertex], triangles: &mut Vec<Vertex>) {
    let (mut upper, mut lower) = (0, 0);

    while upper + 1 < top.len() || lower + 1 < bottom.len() {
        let advance_top = lower + 1 == bottom.len()
            || (upper + 1 < top.len() && top[upper + 1].x <= bottom[lower + 1].x);

        // 他の三角形と巻き方向を揃える（`cross` が正）。
        if advance_top {
            upper += 1;
            triangles.extend([top[upper - 1], top[upper], bottom[lower]]);
        } else {
            lower += 1;
            triangles.extend([top[upper], bottom[lower], bottom[lower - 1]]);
        }
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
    let count = outline.len();

    // 閉じた輪郭は最後の頂点から先頭へ戻る辺も持つ。
    let segment_count = if strip { count - 1 } else { count };

    // 角では前後の辺を見るので、向きと長さを先に出しておく。
    let segments: Vec<Option<Segment>> = (0..segment_count)
        .map(|index| {
            let start = outline[index];
            let end = outline[(index + 1) % count];

            segment_normal(start, end).map(|normal| Segment {
                normal,
                length: ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt(),
            })
        })
        .collect();

    for index in 0..segment_count {
        let Some(segment) = segments[index] else {
            // 長さ 0 の辺には向きが無いので飛ばす。
            continue;
        };

        let start = outline[index];
        let end = outline[(index + 1) % count];

        push_quad(
            start,
            end,
            segment.normal,
            half_width,
            joint_at(&segments, index, strip, half_width),
            joint_at(&segments, index + 1, strip, half_width),
            triangles,
        );
    }

    if joint_type != JointType::None {
        push_joints(outline, &segments, half_width, joint_type, strip, triangles);
    }

    if strip {
        push_caps(outline, half_width, joint_type, triangles);
    }
}

/// 1 本の辺について、角の処理に要るぶんだけ。
#[derive(Clone, Copy)]
struct Segment {
    normal: [f32; 2],
    length: f32,
}

/// 頂点 `vertex` にある角の、内側の交点。角でなければ `None`。
///
/// 閉じた輪郭なら端でも前後があります。開いた折れ線の両端には角がありません。
fn joint_at(
    segments: &[Option<Segment>],
    vertex: usize,
    strip: bool,
    half_width: f32,
) -> Option<InnerJoint> {
    let count = segments.len();

    let (before, after) = if strip {
        // 先頭と末尾は端。詰める角が無い。
        if vertex == 0 || vertex >= count {
            return None;
        }

        (vertex - 1, vertex)
    } else {
        ((vertex + count - 1) % count, vertex % count)
    };

    inner_miter(segments[before]?, segments[after]?, half_width)
}

/// 角の内側で、隣り合う帯の縁が交わるところ。
#[derive(Clone, Copy)]
struct InnerJoint {
    /// どちらの側が内側か。`+1` なら法線の向き、`-1` なら逆。
    side: f32,
    /// 角からその交点までのずらし。
    shift: [f32; 2],
}

/// 角の内側で、隣り合う帯の縁が交わる点を出す。
///
/// # なぜ要るか
///
/// 辺ごとに幅いっぱいの帯を置くと、曲がった**内側で半幅ぶん重なります**。
/// 単色で塗りつぶすだけなら見えませんが、
///
/// - 頂点ごとに色を変えると、重なったところだけ色が飛ぶ
/// - 半透明にすると、重なったところだけ濃くなる
///
/// 内側をこの交点まで詰めれば重なりません。詰めたぶん**継ぎ目の付け根も
/// ここへ動かす**ので、隙間も開きません（[`push_joints`]）。
///
/// # 詰めない場合
///
/// - まっすぐ、または 180 度の折り返し（角が無い、交点が飛ぶ）
/// - 辺が短くて、詰めると帯が裏返る
///
/// どちらも `None` を返し、**元どおり重ねたまま**にします。壊れるよりましです。
fn inner_miter(before: Segment, after: Segment, half_width: f32) -> Option<InnerJoint> {
    let (before_normal, after_normal) = (before.normal, after.normal);

    // 曲がる向きで内側が決まる。法線は向きを 90 度回したものなので、
    // 法線どうしの外積は向きどうしの外積と同じ符号になる。
    let turn = before_normal[0] * after_normal[1] - before_normal[1] * after_normal[0];

    if turn.abs() < f32::EPSILON {
        return None;
    }

    let spread = 1.0 + before_normal[0] * after_normal[0] + before_normal[1] * after_normal[1];

    // 折り返しに近いと交点が遠くへ飛ぶ。
    if spread < f32::EPSILON {
        return None;
    }

    let side = if turn > 0.0 { 1.0 } else { -1.0 };
    let shift = [
        side * half_width * (before_normal[0] + after_normal[0]) / spread,
        side * half_width * (before_normal[1] + after_normal[1]) / spread,
    ];

    // 辺に沿ってどれだけ引っ込むか。辺の半分を超えると、
    // 両端から詰めたときに行き過ぎて帯が裏返る。
    let pull = |normal: [f32; 2]| (shift[0] * normal[1] - shift[1] * normal[0]).abs();

    if pull(before_normal) > before.length * 0.5 || pull(after_normal) > after.length * 0.5 {
        return None;
    }

    Some(InnerJoint { side, shift })
}

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
    start_joint: Option<InnerJoint>,
    end_joint: Option<InnerJoint>,
    triangles: &mut Vec<Vertex>,
) {
    let (start_left, start_right) = band_ends(start, normal, half_width, start_joint);
    let (end_left, end_right) = band_ends(end, normal, half_width, end_joint);

    triangles.push(start_left);
    triangles.push(start_right);
    triangles.push(end_right);

    triangles.push(start_left);
    triangles.push(end_right);
    triangles.push(end_left);
}

/// 帯の片端にある 2 点。**内側だけ**、角の交点まで引っ込める。
///
/// 外側は隙間が空く側なので、そのまま真横に出します。そこは
/// [`push_joints`] が埋めます。
fn band_ends(
    point: Vertex,
    normal: [f32; 2],
    half_width: f32,
    joint: Option<InnerJoint>,
) -> (Vertex, Vertex) {
    let [nx, ny] = normal;
    let (dx, dy) = (nx * half_width, ny * half_width);

    let straight_left = offset(point, dx, dy);
    let straight_right = offset(point, -dx, -dy);

    match joint {
        Some(InnerJoint { side, shift }) if side > 0.0 => {
            (offset(point, shift[0], shift[1]), straight_right)
        }

        Some(InnerJoint { shift, .. }) => {
            (straight_left, offset(point, shift[0], shift[1]))
        }

        None => (straight_left, straight_right),
    }
}

fn push_joints(
    outline: &[Vertex],
    segments: &[Option<Segment>],
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

        // 帯を内側で詰めたぶん、継ぎ目の付け根もそこへ動かす。
        // 動かさないと、詰めたところに三角形の穴が残る。
        let apex = match joint_at(segments, index, strip, half_width) {
            Some(InnerJoint { shift, .. }) => offset(corner, shift[0], shift[1]),
            None => corner,
        };

        if joint_type.is_round() {
            push_round_joint(apex, corner, from, to, half_width, triangles);
        } else if joint_type == JointType::Miter {
            push_miter_joint(apex, corner, from, to, half_width, triangles);
        } else {
            push_bevel_joint(apex, corner, from, to, half_width, triangles);
        }
    }
}

/// 角の外側を三角形 1 枚で塞ぐ。
///
/// `apex` は継ぎ目の付け根。帯を内側で詰めていれば、その交点が来ます。
/// **`corner` とは別**で、円弧や尖りは `corner` を中心に出します。
fn push_bevel_joint(
    apex: Vertex,
    corner: Vertex,
    from: [f32; 2],
    to: [f32; 2],
    half_width: f32,
    triangles: &mut Vec<Vertex>,
) {
    triangles.push(apex);
    triangles.push(offset(corner, from[0] * half_width, from[1] * half_width));
    triangles.push(offset(corner, to[0] * half_width, to[1] * half_width));
}

fn push_miter_joint(
    apex: Vertex,
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
        push_bevel_joint(apex, corner, from, to, half_width, triangles);
        return;
    }

    let miter_x = sum_x / length;
    let miter_y = sum_y / length;

    // 2 つの法線の中間方向と法線のなす角から、飛び出し量が決まる。
    let cosine = miter_x * from[0] + miter_y * from[1];
    if cosine < f32::EPSILON || 1.0 / cosine > MITER_LIMIT {
        push_bevel_joint(apex, corner, from, to, half_width, triangles);
        return;
    }

    let miter_length = half_width / cosine;
    let tip = offset(corner, miter_x * miter_length, miter_y * miter_length);

    let from_point = offset(corner, from[0] * half_width, from[1] * half_width);
    let to_point = offset(corner, to[0] * half_width, to[1] * half_width);

    triangles.push(apex);
    triangles.push(from_point);
    triangles.push(tip);

    triangles.push(apex);
    triangles.push(tip);
    triangles.push(to_point);
}

fn push_round_joint(
    apex: Vertex,
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

    push_fan(apex, corner, start_angle, sweep, half_width, triangles);
}

/// 扇。`apex` から出て、`center` のまわりの弧を張る。
///
/// 端を丸めるときは `apex` と `center` が同じですが、角の継ぎ目では
/// 帯を詰めた交点が `apex` に来るので、別々に受け取ります。
fn push_fan(
    apex: Vertex,
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

        triangles.push(apex);
        triangles.push(offset(center, angle.cos() * radius, angle.sin() * radius));
        triangles.push(offset(
            center,
            next_angle.cos() * radius,
            next_angle.sin() * radius,
        ));
    }
}

fn push_caps(
    outline: &[Vertex],
    half_width: f32,
    joint_type: JointType,
    triangles: &mut Vec<Vertex>,
) {
    if joint_type.caps_start()
        && let Some(normal) = segment_normal(outline[0], outline[1])
    {
        // 帯の左端から右端へ、進行方向と逆回りに半円を張る。
        let start_angle = normal[1].atan2(normal[0]);
        push_fan(outline[0], outline[0], start_angle, PI, half_width, triangles);
    }

    let last = outline.len() - 1;

    if joint_type.caps_end()
        && let Some(normal) = segment_normal(outline[last - 1], outline[last])
    {
        let start_angle = normal[1].atan2(normal[0]);
        push_fan(outline[last], outline[last], start_angle, -PI, half_width, triangles);
    }
}

/// 折れ線を模様どおりに刻んで、描くところだけを取り出す。
///
/// 返るのはどれも**開いた折れ線**です。輪であっても刻めば切れ目ができるので、
/// 破片に閉じた輪はありません。
///
/// 刻む位置の頂点は前後から混ぜて作るので、色も uv も法線も線に沿って続きます。
pub fn dash_runs(outline: &[Vertex], strip: bool, dash: Dash) -> Vec<Vec<Vertex>> {
    // 模様として成り立たないものは刻まない。1 本の線として返す。
    if outline.len() < 2 || !dash.is_usable() {
        return vec![outline.to_vec()];
    }

    let mut points = outline.to_vec();

    // 閉じた輪は最後に先頭へ戻る辺も持つ。
    if !strip {
        points.push(outline[0]);
    }

    let mut runs = Vec::new();
    let mut current: Vec<Vertex> = Vec::new();
    let mut travelled = 0.0;

    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let length = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();

        if !(length.is_finite() && length > 0.0) {
            continue;
        }

        let mut walked = 0.0;

        while walked < length {
            let (drawn, remaining) = dash.at(travelled + walked);

            let step = remaining.min(length - walked);

            // `Dash::at` は必ず正の長さを返すので、ここには来ない。
            // 来たら模様が壊れているので、切り上げて先へ進む。
            // NaN も弾きたいので、正であることを直接確かめる。
            if !step.is_finite() || step <= 0.0 {
                break;
            }

            if drawn {
                // 続きなら入口は置かない。前の辺の出口と同じ点になる。
                if current.is_empty() {
                    current.push(mix(from, to, walked / length));
                }

                current.push(mix(from, to, (walked + step) / length));
            } else if !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }

            walked += step;
        }

        travelled += length;
    }

    if !current.is_empty() {
        runs.push(current);
    }

    // 刻んだ結果、描くところが 1 つも無いこともある。
    runs.retain(|run| run.len() >= 2);
    runs
}

/// 頂点を丸ごと混ぜる。位置だけでなく色も uv も法線も。
fn mix(from: Vertex, to: Vertex, ratio: f32) -> Vertex {
    let ratio = ratio.clamp(0.0, 1.0);
    let blend = |a: f32, b: f32| a + (b - a) * ratio;

    Vertex::new_position_color_uv_normal(
        blend(from.x, to.x),
        blend(from.y, to.y),
        blend(from.z, to.z),
        blend(from.r, to.r),
        blend(from.g, to.g),
        blend(from.b, to.b),
        blend(from.a, to.a),
        blend(from.u, to.u),
        blend(from.v, to.v),
        blend(from.n_x, to.n_x),
        blend(from.n_y, to.n_y),
        blend(from.n_z, to.n_z),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_vertices(points: &[(f32, f32)]) -> Vec<Vertex> {
        points
            .iter()
            .map(|(x, y)| Vertex::new_position_color(*x, *y, 0.0, 1.0, 1.0, 1.0, 1.0))
            .collect()
    }

    /// 1 つの輪郭を塗って三角形にする。
    fn filled(points: &[(f32, f32)]) -> Vec<Vertex> {
        let mut triangles = Vec::new();
        fill(&to_vertices(points), &[0], &mut triangles);

        triangles
    }

    // --- 角で帯が重ならないこと ---

    /// 四角に線を引いた三角形。
    fn ring(width: f32, height: f32, line_width: f32, joint_type: JointType) -> Vec<Vertex> {
        let outline = to_vertices(&[
            (0.0, 0.0),
            (width, 0.0),
            (width, height),
            (0.0, height),
        ]);

        let mut triangles = Vec::new();
        stroke(&outline, line_width, joint_type, false, &mut triangles);

        triangles
    }

    /// **角で帯が重なっていないこと。** ここが今回の要点。
    ///
    /// 辺ごとに幅いっぱいの帯を置くと、曲がった内側で半幅ぶん重なります。
    /// 単色なら見えませんが、頂点ごとに色を変えると重なったところだけ色が飛び、
    /// 半透明なら濃くなります。
    ///
    /// 尖らせる継ぎ目なら、外周は角まできっちり四角。面積が読めるので、
    /// 三角形の合計と突き合わせれば重なりの有無が分かります。
    #[test]
    fn the_bands_do_not_overlap_at_the_corners() {
        for line_width in [10.0_f32, 50.0, 90.0] {
            let half = line_width / 2.0;

            // 外側 (200+w) x (100+w) から、内側の穴を抜いたもの。
            let painted = (200.0 + line_width) * (100.0 + line_width)
                - (200.0 - line_width) * (100.0 - line_width);
            let total = total_area(&ring(200.0, 100.0, line_width, JointType::Miter));

            assert!(
                (total - painted).abs() < 0.5,
                "幅 {line_width}: 三角形 {total} に対して実面積 {painted}。                 差 {} は角 4 つぶんの重なり（{}）に近いか？",
                total - painted,
                4.0 * half * half,
            );
        }
    }

    /// 角を落とす継ぎ目でも重ならない。
    #[test]
    fn bevelled_corners_do_not_overlap_either() {
        let line_width = 50.0_f32;
        let half = line_width / 2.0;

        // 尖りを落としたぶん、四隅から直角三角形が消える。
        let painted = (200.0 + line_width) * (100.0 + line_width)
            - 4.0 * half * half * 0.5
            - (200.0 - line_width) * (100.0 - line_width);

        let total = total_area(&ring(200.0, 100.0, line_width, JointType::Bevel));

        assert!((total - painted).abs() < 0.5, "{total} != {painted}");
    }

    /// 開いた折れ線の角でも重ならない。
    #[test]
    fn an_open_corner_does_not_overlap() {
        let outline = to_vertices(&[(0.0, 0.0), (200.0, 0.0), (200.0, 150.0)]);
        let mut triangles = Vec::new();
        stroke(&outline, 40.0, JointType::Miter, true, &mut triangles);

        // 帯 2 本ぶんから角の重なりを引き、尖らせた外側を足す。
        let painted = 200.0 * 40.0 + 150.0 * 40.0 - 20.0 * 20.0 + 20.0 * 20.0;

        assert!((total_area(&triangles) - painted).abs() < 0.5);
    }

    /// 詰めても**内側に穴が開かない**こと。
    ///
    /// 帯だけ詰めて継ぎ目の付け根を角に残すと、角の内側に三角形の穴が残ります。
    /// 付け根も交点へ動かしているので、そこが埋まっていること。
    #[test]
    fn trimming_leaves_no_gap_at_the_corner() {
        let triangles = ring(200.0, 100.0, 50.0, JointType::Round);

        // 左上の角のまわり。穴 (25..175 x 25..75) の外で、外周の内側。
        for (x, y) in [
            (5.0, 5.0),
            (10.0, 10.0),
            (15.0, 15.0),
            (20.0, 20.0),
            (24.0, 12.0),
            (12.0, 24.0),
        ] {
            assert!(contains(&triangles, x, y), "({x}, {y}) に穴が開いている");
        }
    }

    /// 詰めても**塗る範囲は変わらない**こと。穴の大きさも外周も同じ。
    #[test]
    fn trimming_does_not_change_what_is_painted() {
        let triangles = ring(200.0, 100.0, 50.0, JointType::Miter);

        // 穴の中。
        assert!(!contains(&triangles, 100.0, 50.0));
        assert!(!contains(&triangles, 26.0, 26.0), "穴のすぐ内");
        // 帯の上。
        assert!(contains(&triangles, 24.0, 24.0), "穴のすぐ外");
        assert!(contains(&triangles, 100.0, 0.0), "上の辺");
        assert!(contains(&triangles, 0.0, 50.0), "左の辺");
        // 外周の外。
        assert!(!contains(&triangles, 100.0, -26.0));
        assert!(!contains(&triangles, -26.0, 50.0));
    }

    /// 辺が短すぎるときは詰めない。詰めると帯が裏返って形が壊れる。
    /// 重なったままだが、壊れるよりはまし。
    #[test]
    fn a_segment_too_short_to_trim_is_left_alone() {
        // 一辺 10 に対して幅 50。詰めようとすると行き過ぎる。
        let triangles = ring(10.0, 10.0, 50.0, JointType::Miter);

        // 裏返っていなければ、真ん中は塗られている。
        assert!(contains(&triangles, 5.0, 5.0), "形が壊れている");
        assert!(contains(&triangles, 0.0, 0.0));
    }

    // --- 点が中にあるか ---

    /// 四角の中と外。いちばん基本。
    #[test]
    fn a_point_inside_the_shape_is_found() {
        let square = filled(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);

        assert!(contains(&square, 5.0, 5.0));
        assert!(contains(&square, 0.5, 9.5));

        assert!(!contains(&square, -0.5, 5.0));
        assert!(!contains(&square, 10.5, 5.0));
        assert!(!contains(&square, 5.0, -0.5));
        assert!(!contains(&square, 5.0, 10.5));
    }

    /// 縁は中。外にすると、縁を狙ったときに 1 画素ぶん反応しない。
    #[test]
    fn the_edge_counts_as_inside() {
        let square = filled(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);

        assert!(contains(&square, 0.0, 0.0), "角");
        assert!(contains(&square, 10.0, 10.0), "反対の角");
        assert!(contains(&square, 0.0, 5.0), "辺の上");
        assert!(contains(&square, 5.0, 0.0), "辺の上");
    }

    /// 三角形を割った対角線の上でも落ちない。
    /// 片方の三角形から見て外でも、もう片方から見れば中。
    #[test]
    fn the_seam_between_triangles_is_still_inside() {
        let square = filled(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);

        for step in 0..=10 {
            let along = step as f32;
            assert!(contains(&square, along, along), "対角線上の {along}");
        }
    }

    /// **穴は外。** 三角形が張られていないので自動的にそうなる。
    /// 輪郭で判定していたら、ここを別に処理することになる。
    #[test]
    fn a_hole_is_outside() {
        // 外周 20x20、中に 10x10 の穴。
        let outline = to_vertices(&[
            (0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0),
            (5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0),
        ]);
        let mut triangles = Vec::new();
        fill(&outline, &[0, 4], &mut triangles);

        assert!(contains(&triangles, 2.0, 10.0), "外周の帯は中");
        assert!(contains(&triangles, 18.0, 10.0), "反対側の帯も中");
        assert!(!contains(&triangles, 10.0, 10.0), "穴の真ん中は外");
        assert!(!contains(&triangles, 25.0, 10.0), "外は外");
    }

    /// **線は帯の上だけが中。** 囲まれた内側は塗っていないので外。
    /// ここが輪郭で判定した場合といちばん食い違う。
    #[test]
    fn a_stroke_is_only_inside_on_the_band() {
        let outline = to_vertices(&[(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)]);
        let mut triangles = Vec::new();
        stroke(&outline, 4.0, JointType::Miter, false, &mut triangles);

        assert!(contains(&triangles, 0.0, 20.0), "左の線の上");
        assert!(contains(&triangles, 40.0, 20.0), "右の線の上");
        assert!(!contains(&triangles, 20.0, 20.0), "囲まれた真ん中は塗っていない");
        assert!(!contains(&triangles, 60.0, 20.0), "外は外");
    }

    /// 離れた輪郭は、どちらの側でも中。
    #[test]
    fn separate_contours_are_both_inside() {
        let outline = to_vertices(&[
            (0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0),
            (20.0, 0.0), (30.0, 0.0), (30.0, 10.0), (20.0, 10.0),
        ]);
        let mut triangles = Vec::new();
        fill(&outline, &[0, 4], &mut triangles);

        assert!(contains(&triangles, 5.0, 5.0), "左の島");
        assert!(contains(&triangles, 25.0, 5.0), "右の島");
        assert!(!contains(&triangles, 15.0, 5.0), "あいだは外");
    }

    /// 三角形が 1 枚も無ければ、どこも中ではない。
    #[test]
    fn nothing_is_inside_an_empty_shape() {
        assert!(!contains(&[], 0.0, 0.0));
        assert!(!contains(&[], 5.0, 5.0));
    }

    /// 巻き方向を変えても同じに当たる。向きを決め打ちにしていないこと。
    #[test]
    fn the_winding_direction_does_not_matter() {
        let clockwise = filled(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let widdershins = filled(&[(0.0, 10.0), (10.0, 10.0), (10.0, 0.0), (0.0, 0.0)]);

        for (x, y) in [(5.0, 5.0), (1.0, 9.0), (9.0, 1.0)] {
            assert_eq!(
                contains(&clockwise, x, y),
                contains(&widdershins, x, y),
                "({x}, {y}) で食い違う",
            );
        }
    }

    /// 凹んだ形。出っ張りの外は外、くぼみの中も外。
    #[test]
    fn a_concave_shape_excludes_its_notch() {
        // L 字。右上が欠けている。
        let l_shape = filled(&[
            (0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (5.0, 5.0), (5.0, 10.0), (0.0, 10.0),
        ]);

        assert!(contains(&l_shape, 2.0, 2.0), "根本");
        assert!(contains(&l_shape, 8.0, 2.0), "横の腕");
        assert!(contains(&l_shape, 2.0, 8.0), "縦の腕");
        assert!(!contains(&l_shape, 8.0, 8.0), "欠けたところは外");
    }

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

    /// 穴 1 つにつき頂点が 3 つ増える。三角形の数はそこから決まる。
    ///
    /// 増えるのは**通路の行き帰りで 2 つ**と、**外周に差し込む接続点で 1 つ**です。
    /// 接続点は [`open_bridge`] が辺の上に置くもので、形は変わりませんが
    /// 頂点は 1 つ増えます（穴ごとに別の点へつなぐため。詳しくは
    /// [`open_bridge`] の注釈）。
    #[test]
    fn the_triangle_count_follows_from_the_contours() {
        let mut outline = Vec::new();
        outline.extend_from_slice(&square(0.0, 0.0, 90.0));
        outline.extend_from_slice(&square(30.0, 30.0, 30.0));

        let mut triangles = Vec::new();
        fill(&outline, &[0, 4], &mut triangles);

        // 頂点 4 + 4 + 通路 2 + 接続点 1 = 11 → 三角形 9 枚。
        assert_eq!(triangles.len(), 3 * (4 + 4 + 2 + 1 - 2));
    }

    // --- 字形の輪郭で踏んだ 2 つの詰まり ---

    /// 1 点目を末尾でもう一度打つ。字形の輪郭と同じ形にする。
    fn closed_ring(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
        let mut points = points.to_vec();
        points.push(points[0]);

        points
    }

    /// 輪郭を 1 本につないで塗る。
    fn filled_contours(contours: &[Vec<(f32, f32)>]) -> Vec<Vertex> {
        let mut points = Vec::new();
        let mut starts = Vec::new();

        for contour in contours {
            starts.push(points.len());
            points.extend_from_slice(&to_vertices(contour));
        }

        let mut triangles = Vec::new();
        fill(&points, &starts, &mut triangles);

        triangles
    }

    /// 格子で数えて、穴が抜けていて重ね塗りが無いことを見る。
    ///
    /// 1 点だけ名指しで見ると、そこがたまたま三角形の辺の上に乗っていて
    /// 「塗られていない」と出ることがあります（[`coverage`] は開いた三角形で
    /// 判定するため）。格子で舐めて、**穴の中は必ず 0、外は 1 枚まで**を見ます。
    /// 塗り残しが無いことは面積で別に見ます。
    fn assert_holes_are_clean(triangles: &[Vertex], side: f32, holes: &[(f32, f32, f32, f32)]) {
        let mut inside_hole = 0;
        let mut y = 0.5;

        while y < side {
            let mut x = 0.5;

            while x < side {
                let in_hole = holes.iter().any(|&(left, top, right, bottom)| {
                    x > left && x < right && y > top && y < bottom
                });

                let covered = coverage(triangles, point(x, y));

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

        assert!(inside_hole > 0, "穴の中を 1 点も見ていない");
    }

    /// **1 点目と同じ末尾の点があっても塗れること。**
    ///
    /// 字形の輪郭は `line_to` で始点に戻って閉じるので、最後の点が 1 点目と
    /// 同じになります。輪郭は閉じたものとして扱うので、これは長さ 0 の辺です。
    /// 残すと耳刈り取りが詰まり、外周と穴の両方に残っていて穴が 2 つ以上あると
    /// 形が崩れていました（[`dedup_contour`]）。
    #[test]
    fn a_duplicate_closing_point_does_not_break_two_holes() {
        let outer = closed_ring(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let lower = closed_ring(&[(20.0, 20.0), (80.0, 20.0), (80.0, 40.0), (20.0, 40.0)]);
        let upper = closed_ring(&[(20.0, 60.0), (80.0, 60.0), (80.0, 80.0), (20.0, 80.0)]);

        let triangles = filled_contours(&[outer, lower, upper]);

        assert_triangle_list(&triangles);

        // 100 x 100 から 60 x 20 の穴 2 つを抜いた面積。
        let expected = 100.0 * 100.0 - 2.0 * 60.0 * 20.0;
        assert!(
            (total_area(&triangles) - expected).abs() < 1e-2,
            "{} vs {expected}",
            total_area(&triangles),
        );

        assert_holes_are_clean(
            &triangles,
            100.0,
            &[(20.0, 20.0, 80.0, 40.0), (20.0, 60.0, 80.0, 80.0)],
        );
    }

    /// **同じ辺に当たる穴が 3 つあっても塗れること。**
    ///
    /// 游ゴシックの `面` と同じ並びです。縦に積んだ穴 3 つの右に、
    /// その 3 つを縦に跨ぐ穴が 1 つあります。
    ///
    /// ```text
    ///   ┌──────────────┐
    ///   │ ┌──┐  ┌────┐ │   左の 3 つから +X に線を飛ばすと、
    ///   │ └──┘  │    │ │   どれも右の穴の**同じ縦の辺**に当たる
    ///   │ ┌──┐  │    │ │
    ///   │ └──┘  │    │ │   以前は 3 本の通路が 1 つの頂点に集まり、
    ///   │ ┌──┐  │    │ │   互いを跨いで耳が無くなっていた
    ///   │ └──┘  └────┘ │
    ///   └──────────────┘
    /// ```
    ///
    /// 直したのは [`open_bridge`]。当たった点を辺の上に差し込むので、
    /// 穴ごとに別の点へつながります。
    #[test]
    fn three_holes_hitting_one_edge_each_get_their_own_bridge() {
        let outer = closed_ring(&[(0.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)]);
        // 右の、縦に長い穴。
        let tall = closed_ring(&[(120.0, 20.0), (170.0, 20.0), (170.0, 180.0), (120.0, 180.0)]);
        // 左の、縦に積んだ 3 つ。どれも `tall` の左の辺に当たる高さ。
        let stack: Vec<Vec<(f32, f32)>> = [30.0, 90.0, 150.0]
            .into_iter()
            .map(|y| closed_ring(&[(30.0, y), (90.0, y), (90.0, y + 30.0), (30.0, y + 30.0)]))
            .collect();

        let mut contours = vec![outer, tall];
        contours.extend(stack);

        let triangles = filled_contours(&contours);

        assert_triangle_list(&triangles);

        let expected = 200.0 * 200.0 - 50.0 * 160.0 - 3.0 * 60.0 * 30.0;
        assert!(
            (total_area(&triangles) - expected).abs() < 1e-2,
            "{} vs {expected}",
            total_area(&triangles),
        );

        assert_holes_are_clean(
            &triangles,
            200.0,
            &[
                (120.0, 20.0, 170.0, 180.0),
                (30.0, 30.0, 90.0, 60.0),
                (30.0, 90.0, 90.0, 120.0),
                (30.0, 150.0, 90.0, 180.0),
            ],
        );
    }

    /// **重なっているだけの輪郭を、穴と取り違えて捨てないこと。**
    ///
    /// `Ç` の下の飾りや `Å` の上の輪は、本体と少しだけ重なった別の形です。
    /// 包含関係の判定は輪郭の 1 点（いちばん上の頂点）だけで見るので、
    /// 重なったぶんに入っていると穴に見えます。
    ///
    /// 穴として扱うと外周につなげず、以前は
    /// `a hole could not be bridged` と言って**捨てていました**。
    /// 捨てると飾りが消えます。つなげなかったものは
    /// 単独で塗り直します（[`fill`]）。
    #[test]
    fn an_overlapping_contour_is_filled_instead_of_dropped() {
        // 本体。
        let body = closed_ring(&[(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)]);
        // 飾り。下にぶら下がり、本体と 10 だけ重なる。巻き方向は本体と同じ。
        let tail = closed_ring(&[(30.0, 50.0), (70.0, 50.0), (70.0, 90.0), (30.0, 90.0)]);

        let triangles = filled_contours(&[body, tail]);

        assert_triangle_list(&triangles);

        // 飾りが出ている。消えていたらここが 0 になる。
        assert!(
            coverage(&triangles, point(50.5, 80.5)) >= 1,
            "飾りが塗られていない",
        );

        // 本体も出ている。
        assert!(coverage(&triangles, point(10.5, 30.5)) >= 1, "本体が塗られていない");

        // 重なっていないところには何も無い。
        assert_eq!(coverage(&triangles, point(10.5, 80.5)), 0, "飾りの外");

        // 面積は本体 + 飾り。重なったぶんは二重に塗られるので、そのぶん多い。
        // 1 色の形なら見た目は変わらない。
        let body_area = 100.0 * 60.0;
        let tail_area = 40.0 * 40.0;
        let expected = body_area + tail_area;

        assert!(
            (total_area(&triangles) - expected).abs() < 1e-2,
            "{} vs {expected}",
            total_area(&triangles),
        );
    }

    /// 本物の穴は、重なっていなければこれまでどおり抜けること。
    ///
    /// 上の手当てで穴が塗られるようになっては困る。
    #[test]
    fn a_real_hole_is_still_cut_out() {
        let outer = closed_ring(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let hole = closed_ring(&[(30.0, 30.0), (70.0, 30.0), (70.0, 70.0), (30.0, 70.0)]);

        let triangles = filled_contours(&[outer, hole]);

        assert_triangle_list(&triangles);
        assert_holes_are_clean(&triangles, 100.0, &[(30.0, 30.0, 70.0, 70.0)]);

        let expected = 100.0 * 100.0 - 40.0 * 40.0;
        assert!(
            (total_area(&triangles) - expected).abs() < 1e-2,
            "{} vs {expected}",
            total_area(&triangles),
        );
    }

    /// 同じ場所に続く点も落ちること。曲線を細かく刻むと出ることがある。
    #[test]
    fn repeated_points_are_folded_away() {
        let triangles = filled_contours(&[vec![
            (0.0, 0.0),
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 10.0),
        ]]);

        assert_triangle_list(&triangles);
        // 4 点の四角として扱われる。
        assert_eq!(triangles.len(), 3 * 2);
        assert!((total_area(&triangles) - 100.0).abs() < 1e-3);
    }

    /// **開いた折れ線では、始点に戻る最後の点を落とさないこと。**
    ///
    /// 閉じた輪郭では長さ 0 の辺ですが、開いた折れ線では**消してはいけない
    /// 線分**です。落とすと一辺足りない線になります。
    #[test]
    fn an_open_polyline_keeps_the_point_that_returns_to_the_start() {
        let paint_type = |strip| PaintType::Stroke {
            line_width: 4.0,
            joint_type: JointType::Miter,
            strip,
            dash: None,
        };

        let square = to_vertices(&[(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 50.0)]);
        let returning = to_vertices(&closed_ring(&[
            (0.0, 0.0),
            (50.0, 0.0),
            (50.0, 50.0),
            (0.0, 50.0),
        ]));

        // 開いた折れ線。始点に戻る点があるぶん、1 辺ぶん長い。
        let mut open_without = Vec::new();
        tessellate(&square, &[0], paint_type(true), &mut open_without);

        let mut open_with = Vec::new();
        tessellate(&returning, &[0], paint_type(true), &mut open_with);

        assert!(
            open_with.len() > open_without.len(),
            "{} vs {}",
            open_with.len(),
            open_without.len(),
        );

        // 閉じた輪では、戻る点があっても無くても同じ。
        let mut closed_without = Vec::new();
        tessellate(&square, &[0], paint_type(false), &mut closed_without);

        let mut closed_with = Vec::new();
        tessellate(&returning, &[0], paint_type(false), &mut closed_with);

        assert_eq!(closed_with.len(), closed_without.len());
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
            dash: None,
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

    // --- 破線 ---

    use crate::paint_type::MAX_DASH;

    /// 三角形が塗る面積の合計。重なりは数え直さない。
    fn area(triangles: &[Vertex]) -> f32 {
        triangles
            .chunks_exact(3)
            .map(|t| {
                ((t[1].x - t[0].x) * (t[2].y - t[0].y) - (t[2].x - t[0].x) * (t[1].y - t[0].y))
                    .abs()
                    / 2.0
            })
            .sum()
    }

    /// 横一直線。長さ `length`。
    fn straight(length: f32) -> Vec<Vertex> {
        to_vertices(&[(0.0, 0.0), (length, 0.0)])
    }

    /// 刻んだ破片ごとの、始まりと終わりの x。
    fn pieces(outline: &[Vertex], strip: bool, dash: Dash) -> Vec<(f32, f32)> {
        dash_runs(outline, strip, dash)
            .iter()
            .map(|run| (run[0].x, run[run.len() - 1].x))
            .collect()
    }

    /// 刻んだあとに残る長さの合計。
    fn drawn_length(outline: &[Vertex], strip: bool, dash: Dash) -> f32 {
        dash_runs(outline, strip, dash)
            .iter()
            .flat_map(|run| run.windows(2))
            .map(|pair| ((pair[1].x - pair[0].x).powi(2) + (pair[1].y - pair[0].y).powi(2)).sqrt())
            .sum()
    }

    /// 刻んだ場所が、思った所に来ているか。
    ///
    /// 混ぜて作る点なので `100.0 * 0.3` が `30.000002` になる。ぴったりでは比べない。
    fn assert_pieces(got: &[(f32, f32)], want: &[(f32, f32)]) {
        assert_eq!(got.len(), want.len(), "本数が違う。{got:?}");

        for (got, want) in got.iter().zip(want) {
            assert!(
                (got.0 - want.0).abs() < 0.01 && (got.1 - want.1).abs() < 0.01,
                "{got:?} は {want:?} のはず",
            );
        }
    }

    #[test]
    fn a_dash_cuts_the_line_into_pieces() {
        // 100 の線を「10 描いて 10 空ける」。5 本になる。
        let got = pieces(&straight(100.0), true, Dash::new(10.0, 10.0));

        assert_pieces(
            &got,
            &[(0.0, 10.0), (20.0, 30.0), (40.0, 50.0), (60.0, 70.0), (80.0, 90.0)],
        );
    }

    #[test]
    fn the_drawn_length_matches_the_pattern() {
        // 描くと空けるが同じなら、残るのは半分。
        let got = drawn_length(&straight(100.0), true, Dash::new(10.0, 10.0));
        assert!((got - 50.0).abs() < 0.01, "{got}");

        // 3 対 1 なら 4 分の 3。
        let got = drawn_length(&straight(100.0), true, Dash::new(15.0, 5.0));
        assert!((got - 75.0).abs() < 0.01, "{got}");
    }

    /// 刻む位置が辺をまたいでも、破片は 1 本につながる。
    #[test]
    fn a_piece_carries_across_a_corner() {
        // 角が (50, 0) にある折れ線。50 描いて 10 空けるので、
        // 1 本目は角をまたいで (50, 40) まで伸びる。
        let outline = to_vertices(&[(0.0, 0.0), (50.0, 0.0), (50.0, 100.0)]);
        let runs = dash_runs(&outline, true, Dash::new(60.0, 10.0));

        let first = &runs[0];

        assert_eq!(first.len(), 3, "角の頂点が落ちている");
        assert_eq!((first[0].x, first[0].y), (0.0, 0.0));
        assert_eq!((first[1].x, first[1].y), (50.0, 0.0), "角");
        assert!((first[2].y - 10.0).abs() < 0.01, "{:?}", first[2].y);
    }

    /// 刻んだ切り口の頂点は前後から混ぜて作る。色も uv も線に沿って続く。
    #[test]
    fn a_cut_blends_the_attributes() {
        let mut outline = straight(100.0);
        outline[0].r = 0.0;
        outline[1].r = 1.0;
        outline[0].u = 0.0;
        outline[1].u = 1.0;

        let runs = dash_runs(&outline, true, Dash::new(50.0, 50.0));

        // 50 のところで切れる。半分なので赤も uv も半分。
        let cut = runs[0][1];

        assert!((cut.r - 0.5).abs() < 0.01, "赤 {}", cut.r);
        assert!((cut.u - 0.5).abs() < 0.01, "uv {}", cut.u);
    }

    #[test]
    fn the_offset_slides_the_pattern() {
        // 模様を 10 ずらすと、空けるところから始まる。
        let got = pieces(&straight(40.0), true, Dash::new(10.0, 10.0).offset(10.0));

        assert_pieces(&got, &[(10.0, 20.0), (30.0, 40.0)]);
    }

    #[test]
    fn the_offset_wraps_round() {
        // ひと回りぶんずらしても元と同じ。
        let plain = pieces(&straight(100.0), true, Dash::new(10.0, 10.0));
        let wrapped = pieces(&straight(100.0), true, Dash::new(10.0, 10.0).offset(20.0));

        assert_eq!(plain, wrapped);
    }

    /// 閉じた輪は先頭へ戻る辺も刻む。
    #[test]
    fn a_closed_contour_dashes_all_the_way_round() {
        // 1 辺 40 の四角。周は 160。
        let square = to_vertices(&[(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)]);

        let closed = drawn_length(&square, false, Dash::new(10.0, 10.0));
        let open = drawn_length(&square, true, Dash::new(10.0, 10.0));

        // 閉じれば 1 辺ぶん長い。
        assert!((closed - 80.0).abs() < 0.01, "閉じた輪 {closed}");
        assert!((open - 60.0).abs() < 0.01, "開いた折れ線 {open}");
    }

    /// 刻んだ破片に閉じた輪は無い。輪でも切れ目ができる。
    #[test]
    fn dashing_a_ring_never_leaves_a_closed_piece() {
        let square = to_vertices(&[(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)]);

        for run in dash_runs(&square, false, Dash::new(10.0, 10.0)) {
            let (first, last) = (run[0], run[run.len() - 1]);
            let gap = ((last.x - first.x).powi(2) + (last.y - first.y).powi(2)).sqrt();

            assert!(gap > 0.01, "端どうしが重なっている");
        }
    }

    // --- 模様の組み立て ---

    #[test]
    fn an_odd_pattern_is_repeated_to_even_it_out() {
        // 奇数だと描くと空けるが一周ごとに入れ替わる。繰り返して均す。
        let dash = Dash::pattern(&[4.0, 2.0, 1.0]);

        assert_eq!(dash.lengths(), &[4.0, 2.0, 1.0, 4.0, 2.0, 1.0]);
        assert_eq!(dash.period(), 14.0);
    }

    /// 1 つだけ渡したら「同じ長さで描いて空ける」。SVG と同じ読み方。
    #[test]
    fn a_single_length_means_equal_on_and_off() {
        let dash = Dash::pattern(&[10.0]);

        assert_eq!(dash.lengths(), &[10.0, 10.0]);
        assert_eq!(dash, Dash::new(10.0, 10.0));
    }

    #[test]
    fn an_even_pattern_is_left_alone() {
        let dash = Dash::pattern(&[4.0, 2.0]);

        assert_eq!(dash.lengths(), &[4.0, 2.0]);
    }

    #[test]
    fn a_pattern_that_is_too_long_is_cut_to_an_even_count() {
        let dash = Dash::pattern(&[1.0; 20]);

        assert_eq!(dash.lengths().len(), MAX_DASH);
        assert!(dash.lengths().len().is_multiple_of(2));
    }

    #[test]
    fn an_unusable_pattern_draws_one_solid_line() {
        let outline = straight(100.0);

        for (lengths, note) in [
            (vec![], "空"),
            (vec![10.0, 0.0], "0 が混ざる"),
            (vec![10.0, -5.0], "負が混ざる"),
            (vec![10.0, f32::NAN], "数でない"),
        ] {
            let dash = Dash::pattern(&lengths);

            assert!(!dash.is_usable(), "{note} が使えることになっている");

            let runs = dash_runs(&outline, true, dash);

            assert_eq!(runs.len(), 1, "{note} で刻まれた");
            assert_eq!(runs[0].len(), 2, "{note} で頂点が変わった");
        }
    }

    #[test]
    fn a_pattern_longer_than_the_line_leaves_one_piece() {
        // 描くところが線より長い。切れ目は出ない。
        let got = pieces(&straight(30.0), true, Dash::new(100.0, 10.0));

        assert_pieces(&got, &[(0.0, 30.0)]);
    }

    #[test]
    fn a_line_that_lands_entirely_in_a_gap_draws_nothing() {
        // 空けるところから始めて、線が終わるまで空いたまま。
        let dash = Dash::new(10.0, 100.0).offset(10.0);

        assert!(dash_runs(&straight(30.0), true, dash).is_empty());
    }

    #[test]
    fn dots_are_a_very_short_dash() {
        let dash = Dash::dots(10.0);

        assert!(dash.is_usable());
        assert!(dash.lengths()[0] < 0.1, "点が長すぎる");
        assert_eq!(dash.lengths()[1], 10.0);
        assert!(dash.has_round_ends(), "端が丸まらないと点が現れない");

        let runs = dash_runs(&straight(100.0), true, dash);
        assert_eq!(runs.len(), 10);
    }

    /// **`JointType` を選ばずに点が出ること。**
    ///
    /// 刻んでできた端は元の線の途中なので、折れ線全体の端しか見ない
    /// `JointType` では届かない。ここが届いていないと、点は
    /// 面積のない極細の棒になって消える。
    #[test]
    fn dots_show_up_whatever_the_joint_type_is() {
        for joint_type in [
            JointType::None,
            JointType::Miter,
            JointType::Bevel,
            JointType::Round,
            JointType::RoundStartEnd,
        ] {
            let mut triangles = Vec::new();
            tessellate(
                &straight(100.0),
                &[0],
                PaintType::Stroke {
                    line_width: 10.0,
                    joint_type,
                    strip: true,
                    // 間隔は線の太さより広く取る。同じだと点どうしが接する。
                    dash: Some(Dash::dots(30.0)),
                },
                &mut triangles,
            );

            // 半径 5 の丸が 4 個で 314。折れ線で近似するぶん少し減る。
            let painted = area(&triangles);

            assert!(
                painted > 280.0,
                "{joint_type:?} で点が潰れている（面積 {painted:.2}）",
            );

            // 点の真ん中は塗られ、間は空いている。
            assert!(contains(&triangles, 0.0, 0.0), "{joint_type:?} 1 つめの点");
            assert!(contains(&triangles, 30.0, 0.0), "{joint_type:?} 2 つめの点");
            assert!(!contains(&triangles, 15.0, 0.0), "{joint_type:?} 点の間が埋まっている");
        }
    }

    /// 間隔を広げても点は消えない。
    ///
    /// 点の長さを決め打ちにすると、模様の切り替わり際を見る丸め誤差の幅に
    /// 飲み込まれて消える。間隔に対する割合にしてあるのはそのため。
    #[test]
    fn dots_survive_a_wide_gap() {
        for gap in [1.0, 10.0, 100.0, 10_000.0] {
            let dash = Dash::dots(gap);
            let runs = dash_runs(&straight(gap * 10.0), true, dash);

            assert_eq!(runs.len(), 10, "間隔 {gap} で点の数が合わない");
        }
    }

    /// 破線も端を丸められる。点線だけの仕組みではない。
    #[test]
    fn a_dash_can_have_round_ends_too() {
        let plain = Dash::new(10.0, 10.0);
        let round = Dash::new(10.0, 10.0).round_ends(true);

        assert!(!plain.has_round_ends());
        assert!(round.has_round_ends());

        let paint = |dash| PaintType::Stroke {
            line_width: 10.0,
            joint_type: JointType::Miter,
            strip: true,
            dash: Some(dash),
        };

        let mut square_ends = Vec::new();
        tessellate(&straight(100.0), &[0], paint(plain), &mut square_ends);

        let mut rounded = Vec::new();
        tessellate(&straight(100.0), &[0], paint(round), &mut rounded);

        // 丸めたぶんだけ広がる。半円 2 つ × 5 本ぶん。
        assert!(
            area(&rounded) > area(&square_ends) + 300.0,
            "丸めても広がっていない（{:.1} と {:.1}）",
            area(&rounded),
            area(&square_ends),
        );
    }

    // --- 実際に三角形にする ---

    #[test]
    fn a_dashed_stroke_makes_less_ink_than_a_solid_one() {
        let outline = straight(100.0);

        let mut solid = Vec::new();
        tessellate(&outline, &[0], PaintType::stroke(4.0), &mut solid);

        let mut dashed = Vec::new();
        tessellate(
            &outline,
            &[0],
            PaintType::dashed(4.0, Dash::new(10.0, 10.0)),
            &mut dashed,
        );

        assert!(!dashed.is_empty(), "破線が消えている");
        assert!(
            area(&dashed) < area(&solid) * 0.6,
            "塗られた面積が減っていない（{} / {}）",
            area(&dashed),
            area(&solid),
        );
    }

    #[test]
    fn a_dashed_stroke_covers_the_dashes_and_skips_the_gaps() {
        let mut triangles = Vec::new();
        tessellate(
            &straight(100.0),
            &[0],
            // 2 点の線は `strip` を立てないと往復する。往復すると
            // 帰りの破線が行きの隙間を埋めてしまう。
            PaintType::Stroke {
                line_width: 4.0,
                joint_type: JointType::Miter,
                strip: true,
                dash: Some(Dash::new(10.0, 10.0)),
            },
            &mut triangles,
        );

        // 描くところの真ん中と、空けるところの真ん中。
        assert!(contains(&triangles, 5.0, 0.0), "1 本目の上");
        assert!(!contains(&triangles, 15.0, 0.0), "空けたところに線がある");
        assert!(contains(&triangles, 25.0, 0.0), "2 本目の上");
        assert!(!contains(&triangles, 35.0, 0.0), "空けたところに線がある");
    }

    // --- 非ゼロ巻き数の塗り ---

    /// 輪郭をつないで、区切りと一緒に返す。
    fn joined(contours: &[&[Vertex]]) -> (Vec<Vertex>, Vec<usize>) {
        let mut outline = Vec::new();
        let mut starts = Vec::new();

        for contour in contours {
            starts.push(outline.len());
            outline.extend_from_slice(contour);
        }

        (outline, starts)
    }

    fn filled_nonzero(contours: &[&[Vertex]]) -> Vec<Vertex> {
        let (outline, starts) = joined(contours);
        let mut triangles = Vec::new();
        fill_nonzero(&outline, &starts, &mut triangles);

        assert_triangle_list(&triangles);
        triangles
    }

    /// 時計回りの正方形。`square` の逆向き。
    fn square_clockwise(x: f32, y: f32, size: f32) -> [Vertex; 4] {
        let mut corners = square(x, y, size);
        corners.reverse();
        corners
    }

    /// 答え合わせ用の巻き数。点から +X に線を飛ばし、向きつきで辺を数える。
    fn winding_number(contours: &[&[Vertex]], point: Vertex) -> i32 {
        let mut total = 0;

        for contour in contours {
            for index in 0..contour.len() {
                let a = contour[index];
                let b = contour[(index + 1) % contour.len()];

                if (a.y <= point.y) != (b.y <= point.y) {
                    let x = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x);

                    if x > point.x {
                        total += if b.y > a.y { 1 } else { -1 };
                    }
                }
            }
        }

        total
    }

    /// 格子で、**巻き数が 0 でないところをちょうど 1 枚で覆っている**こと。
    ///
    /// 0 枚なら塗り残し、2 枚なら重ね塗り。格子は辺に乗らないよう半端にずらす。
    fn assert_matches_winding(contours: &[&[Vertex]], triangles: &[Vertex], extent: f32) {
        let steps = 60;

        for row in 0..steps {
            for column in 0..steps {
                let at = point(
                    (column as f32 + 0.5) / steps as f32 * extent + 0.0137,
                    (row as f32 + 0.5) / steps as f32 * extent + 0.0291,
                );

                let expected = usize::from(winding_number(contours, at) != 0);

                assert_eq!(
                    coverage(triangles, at),
                    expected,
                    "({}, {}) の巻き数は {}",
                    at.x,
                    at.y,
                    winding_number(contours, at),
                );
            }
        }
    }

    /// **重なった 2 本の画は、重なりも塗る。** ここが今回の要点。
    ///
    /// 日本語のフォントは画ごとの輪郭を重ねて字を作ります。`十` なら横棒と
    /// 縦棒の 2 本で、真ん中が重なる。包含の偶奇で塗るとここが抜けます。
    #[test]
    fn nonzero_fills_where_two_strokes_overlap() {
        let horizontal = [point(0.0, 40.0), point(100.0, 40.0), point(100.0, 60.0), point(0.0, 60.0)];
        let vertical = [point(40.0, 0.0), point(60.0, 0.0), point(60.0, 100.0), point(40.0, 100.0)];
        let contours: [&[Vertex]; 2] = [&horizontal, &vertical];

        let triangles = filled_nonzero(&contours);

        assert_eq!(coverage(&triangles, point(50.3, 50.7)), 1, "重なりが抜けている");
        // 和集合の面積。重なりを 2 回数えていないこと。
        let area = total_area(&triangles);
        assert!((area - (2000.0 + 2000.0 - 400.0)).abs() < 1e-2, "{area}");

        assert_matches_winding(&contours, &triangles, 100.0);
    }

    /// 逆向きに巻いた内側の輪郭は穴になる。
    #[test]
    fn nonzero_cuts_a_hole_that_winds_the_other_way() {
        let outer = square(0.0, 0.0, 90.0);
        let hole = square_clockwise(30.0, 30.0, 30.0);
        let contours: [&[Vertex]; 2] = [&outer, &hole];

        let triangles = filled_nonzero(&contours);

        assert_eq!(coverage(&triangles, point(45.3, 46.1)), 0);
        let area = total_area(&triangles);
        assert!((area - (90.0 * 90.0 - 30.0 * 30.0)).abs() < 1e-2, "{area}");

        assert_matches_winding(&contours, &triangles, 90.0);
    }

    /// 同じ向きに巻いた内側の輪郭は穴ではない。**ここが `fill` と違う。**
    #[test]
    fn nonzero_fills_an_inner_contour_that_winds_the_same_way() {
        let outer = square(0.0, 0.0, 90.0);
        let inner = square(30.0, 30.0, 30.0);
        let contours: [&[Vertex]; 2] = [&outer, &inner];

        let triangles = filled_nonzero(&contours);

        assert_eq!(coverage(&triangles, point(45.3, 46.1)), 1);
        let area = total_area(&triangles);
        assert!((area - 90.0 * 90.0).abs() < 1e-2, "{area}");

        // 偶奇で塗る `fill` は、同じ形を穴にする。違いがここで出ていること。
        let (outline, starts) = joined(&contours);
        let mut even_odd = Vec::new();
        fill(&outline, &starts, &mut even_odd);
        assert_eq!(coverage(&even_odd, point(45.3, 46.1)), 0);
    }

    /// 自分と交わる輪郭も塗れる。五芒星の真ん中は 2 周囲まれているので塗る。
    #[test]
    fn nonzero_fills_the_middle_of_a_pentagram() {
        let star: Vec<Vertex> = (0..5)
            .map(|index| {
                // 1 つ飛ばしに結ぶ。
                let angle = (index * 2) as f32 * std::f32::consts::TAU / 5.0;
                point(50.0 + 45.0 * angle.sin(), 50.0 - 45.0 * angle.cos())
            })
            .collect();
        let contours: [&[Vertex]; 1] = [&star];

        let triangles = filled_nonzero(&contours);

        assert_eq!(winding_number(&contours, point(50.0, 50.0)).abs(), 2);
        assert_eq!(coverage(&triangles, point(50.3, 50.7)), 1);

        assert_matches_winding(&contours, &triangles, 100.0);
    }

    /// 向きの揃った三角形を出す。`fill` と同じ巻き（`cross` が正）。
    #[test]
    fn nonzero_triangles_wind_like_fill() {
        let horizontal = square(0.0, 40.0, 60.0);
        let vertical = square_clockwise(20.0, 0.0, 60.0);
        let triangles = filled_nonzero(&[&horizontal, &vertical]);

        assert!(!triangles.is_empty());

        for corner in triangles.chunks_exact(3) {
            assert!(cross(corner[0], corner[1], corner[2]) > 0.0, "{corner:?}");
        }
    }

    /// 関係の無い頂点の高さで、形が細切れにならないこと。
    ///
    /// 帯は全頂点の高さで区切るので、そのまま台形にすると、隣の円の
    /// 頂点の数だけ四角が割れます。同じ 2 辺で続くあいだはまとめる。
    #[test]
    fn unrelated_vertices_do_not_slice_a_shape() {
        let rectangle = [point(0.0, 0.0), point(10.0, 0.0), point(10.0, 100.0), point(0.0, 100.0)];
        let circle: Vec<Vertex> = (0..32)
            .map(|index| {
                let angle = index as f32 * std::f32::consts::TAU / 32.0;
                point(60.0 + 30.0 * angle.cos(), 50.0 + 45.0 * angle.sin())
            })
            .collect();

        let triangles = filled_nonzero(&[&rectangle, &circle]);

        let in_rectangle = triangles
            .chunks_exact(3)
            .filter(|corner| corner.iter().all(|vertex| vertex.x <= 10.0))
            .count();

        assert_eq!(in_rectangle, 2, "四角が {in_rectangle} 枚に割れている");
    }

    /// **継ぎ目に T 字を残さない。** 幅の違う台形が上下に並ぶところ。
    ///
    /// 残すと、中で背中合わせになっている辺が一致せず、外周に見えてしまう。
    /// 外周だけが 1 回ずつ出てくるなら、奇数回の辺の長さは周の長さと等しい。
    #[test]
    fn stacked_trapezoids_share_their_seams() {
        // 凸の字。上が細く、下が広い。
        let outline = [
            point(30.0, 0.0),
            point(70.0, 0.0),
            point(70.0, 40.0),
            point(100.0, 40.0),
            point(100.0, 80.0),
            point(0.0, 80.0),
            point(0.0, 40.0),
            point(30.0, 40.0),
        ];

        let triangles = filled_nonzero(&[&outline]);

        let mut counts: Vec<((u64, u64), f32, u32)> = Vec::new();
        let pack = |vertex: Vertex| ((vertex.x.to_bits() as u64) << 32) | vertex.y.to_bits() as u64;

        for corner in triangles.chunks_exact(3) {
            for step in 0..3 {
                let (from, to) = (corner[step], corner[(step + 1) % 3]);
                let (a, b) = (pack(from), pack(to));
                let key = if a <= b { (a, b) } else { (b, a) };
                let length = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();

                match counts.iter_mut().find(|(seen, _, _)| *seen == key) {
                    Some((_, _, count)) => *count += 1,
                    None => counts.push((key, length, 1)),
                }
            }
        }

        let boundary: f32 = counts
            .iter()
            .filter(|(_, _, count)| count % 2 == 1)
            .map(|(_, length, _)| length)
            .sum();

        // 40 + 40 + 30 + 40 + 100 + 40 + 30 + 40
        assert!((boundary - 360.0).abs() < 1e-3, "外周の長さが {boundary}");
    }

    /// 尖った先で、面積の無い三角形を出さないこと。
    #[test]
    fn a_pointed_tip_makes_no_flat_triangles() {
        let diamond = [point(50.0, 0.0), point(100.0, 50.0), point(50.0, 100.0), point(0.0, 50.0)];
        let triangles = filled_nonzero(&[&diamond]);

        assert!(!triangles.is_empty());

        for corner in triangles.chunks_exact(3) {
            assert!(
                !same_position(corner[0], corner[1])
                    && !same_position(corner[1], corner[2])
                    && !same_position(corner[2], corner[0]),
                "{corner:?}",
            );
        }
    }

    /// **角はちょうど入力の頂点に乗る。** 1 ulp もずらさない。
    ///
    /// 輪郭の 1 点は、上の辺の下端でもあり、下の辺の上端でもあります。
    /// 下端を混ぜ算で出すと丸めで僅かにずれ、上端から出した点と食い違う。
    /// すると継ぎ目が一致せず、すぐ隣に別の点が 2 つ並びます。
    #[test]
    fn corners_land_exactly_on_the_input_vertices() {
        // 半端な座標の多角形。丸めが出やすい。
        let polygon: Vec<Vertex> = (0..23)
            .map(|index| {
                let angle = index as f32 * std::f32::consts::TAU / 23.0 + 0.37;
                let radius = if index % 2 == 0 { 41.3 } else { 29.7 };
                point(50.13 + radius * angle.cos(), 50.71 + radius * angle.sin())
            })
            .collect();

        // 0 のすぐそばの x も混ぜる。`1.0 + (3e-8 - 1.0)` は丸めで 0 になり、
        // 3e-8 には戻らない。字形は em 単位なので、0 付近の座標はふつうに出る。
        let notch = [
            point(1.0, 0.0),
            point(5.0, 0.0),
            point(5.0, 2.0),
            point(1.0, 2.0),
            point(3e-8, 1.0),
        ];

        let mut triangles = filled_nonzero(&[&polygon]);
        triangles.extend(filled_nonzero(&[&notch]));

        for (index, a) in triangles.iter().enumerate() {
            for b in &triangles[index + 1..] {
                let apart = (a.x - b.x).abs().max((a.y - b.y).abs());

                assert!(
                    apart == 0.0 || apart > 1e-6,
                    "({}, {}) と ({}, {}) がほとんど重なっている",
                    a.x,
                    a.y,
                    b.x,
                    b.y,
                );
            }
        }
    }

    /// 帯で割って差し込んだ点の色は、辺の上で混ぜたものになる。
    #[test]
    fn nonzero_blends_colours_along_the_edges() {
        // 上が白、下が黒。縦に色が変わる。
        let shade = |x: f32, y: f32| {
            let level = 1.0 - y / 100.0;
            Vertex::new_position_color(x, y, 0.0, level, level, level, 1.0)
        };
        let first = [shade(0.0, 0.0), shade(60.0, 0.0), shade(60.0, 100.0), shade(0.0, 100.0)];
        // 交わる斜めの形で、途中の高さに点を作らせる。
        let second = [shade(30.0, 20.0), shade(90.0, 50.0), shade(30.0, 80.0)];

        let triangles = filled_nonzero(&[&first, &second]);

        for vertex in &triangles {
            let expected = 1.0 - vertex.y / 100.0;
            assert!(
                (vertex.r - expected).abs() < 1e-4,
                "({}, {}) の色 {}",
                vertex.x,
                vertex.y,
                vertex.r,
            );
        }
    }

    /// 形にならない入力では何も出さない。
    #[test]
    fn nonzero_ignores_degenerate_input() {
        let line = [point(0.0, 0.0), point(10.0, 10.0)];
        let flat = [point(0.0, 0.0), point(10.0, 0.0), point(20.0, 0.0)];

        assert!(filled_nonzero(&[]).is_empty());
        assert!(filled_nonzero(&[&line]).is_empty());
        assert!(filled_nonzero(&[&flat]).is_empty());
    }

    /// `tessellate` から呼べること。
    #[test]
    fn the_paint_type_selects_the_nonzero_fill() {
        let outer = square(0.0, 0.0, 90.0);
        let inner = square(30.0, 30.0, 30.0);
        let (outline, starts) = joined(&[&outer, &inner]);

        let mut triangles = Vec::new();
        tessellate(&outline, &starts, PaintType::FillNonZero, &mut triangles);

        assert_eq!(coverage(&triangles, point(45.3, 46.1)), 1);
    }
}
