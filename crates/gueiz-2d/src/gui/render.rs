//! 記録した命令を [`DrawManager`] の図形に移す。
//!
//! # 命令 1 つ = 図形 1 つ
//!
//! [`DisplayList`] の命令ごとに [`Object`] を 1 つ持ち、**鍵で結び付けて
//! 使い回します。** 鍵は「誰が出したか」と「その人の何番目か」です。
//!
//! ```text
//! (WidgetId, 何番目) ──▶ 図形の名前
//! ```
//!
//! 並びの順ではなく出した人で結ぶので、兄弟が増えたり減ったりしても
//! 他の図形は結び付きを失いません。
//!
//! # 変わっていなければ触らない
//!
//! 前のフレームと [`Primitive`] も切り抜きも同じなら、**何もしません。**
//! [`DrawManager`] は書き換えられた図形だけを送り直すので、動いていない UI は
//! 最初のフレーム以降 GPU への転送が起きません。
//!
//! 変わったのが重なり順だけなら [`Object::z`] を書くだけで済みます。
//! 頂点を積み直すのは、形か色か切り抜きが変わったときだけです。
//!
//! # 消えた図形は捨てずに取っておく
//!
//! [`DrawManager`] には**図形を消す手立てがありません**（登録するだけ）。
//! そこで、使われなくなった図形は
//!
//! 1. 頂点を空にして複製も外し（描かれなくなる）
//! 2. 名前を空き一覧に積み
//! 3. 次に要るときに積んだものから取る
//!
//! という形で回しています。つまり **GUI が使う図形の数は、同時に出した
//! 命令の数のいちばん多かったときの値まで増えて、そこで止まります。**
//! 開いたり閉じたりを繰り返しても増え続けません。
//!
//! # 文字は命令ごとに輪郭を焼く
//!
//! [`TextRenderer`](crate::text::TextRenderer) は字の形を 1 つだけ登録して
//! 複製を足す作りで、同じ文字がたくさん出る本文に向いています。
//! ただし**形ごとに z が 1 つ**なので、窓が重なったときに前後が正しく出ません。
//!
//! GUI では文字は短く、重なり順のほうが大事なので、ここでは
//! **その命令の図形にそのまま輪郭を積みます。** 頂点は増えますが、
//! 背景・文字・前の窓の順がそのまま出ます。

use fxhash::{FxHashMap, FxHashSet};

use crate::camera::Camera;
use crate::draw_manager::DrawManager;
use crate::effect::{Block, EffectStack};
use crate::font::Font;
use crate::gui::color::Color;
use crate::gui::geometry::{Corners, Point, Rect};
use crate::gui::id::WidgetId;
use crate::gui::painter::{
    ClipRegion, DisplayList, ImageSource, Primitive, TextLayoutOptions,
};
use crate::instance::create_instance;
use crate::object::{Object, create_object};
use crate::objects::points::circle_steps;
use crate::paint_type::{JointType, PaintType};
use crate::text::{TextStyle, layout, layout_wrapped};
use crate::vertex::Vertex;

/// 丸い角を何辺で作るか決める刻み。半径に応じて増える。
const MIN_CORNER_STEPS: u32 = 2;

/// 図形 1 つを指す鍵。
type SlotKey = (WidgetId, u32);

/// 命令 1 つに結び付いた図形。
struct Slot {
    /// [`DrawManager`] に登録した名前。
    name: String,
    /// 前に積んだもの。これと同じなら触らない。
    primitive: Primitive,
    clip: Option<ClipRegion>,
    z: f32,
}

/// [`DisplayList`] と [`DrawManager`] のあいだ。
///
/// ```no_run
/// # use gueiz_2d::camera::Camera;
/// # use gueiz_2d::draw_manager::DrawManager;
/// # use gueiz_2d::font::Font;
/// # use gueiz_2d::gui::render::GuiRenderer;
/// # use gueiz_2d::gui::Gui;
/// # fn frame(gui: &mut Gui, renderer: &mut GuiRenderer, draw: &mut DrawManager, font: &Font) {
/// let camera = Camera::orthographic_2d(gui.viewport().width, gui.viewport().height);
/// let list = gui.paint();
///
/// renderer.sync(draw, list, camera, Some(font));
/// # }
/// ```
pub struct GuiRenderer {
    slots: FxHashMap<SlotKey, Slot>,
    /// 使われなくなった図形の名前。
    recycled: Vec<String>,
    /// 今フレームで、その人が何番目まで出したか。
    counters: FxHashMap<WidgetId, u32>,
    /// 今フレームで使う鍵。**割り当てより先に数える。**
    live: FxHashSet<SlotKey>,
    /// 捨てる鍵の置き場。毎フレーム使い回す。
    expired: Vec<SlotKey>,

    layer: u32,
    z_base: f32,
    z_step: f32,
    /// 名前を一意にするための番号。
    minted: u64,
    name_prefix: String,
}

impl Default for GuiRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl GuiRenderer {
    pub fn new() -> Self {
        Self {
            slots: FxHashMap::default(),
            recycled: Vec::new(),
            counters: FxHashMap::default(),
            live: FxHashSet::default(),
            expired: Vec::new(),
            layer: 0,
            z_base: 0.0,
            z_step: 1.0,
            minted: 0,
            name_prefix: String::from("gui"),
        }
    }

    /// どの層に描くか。層ごとにエフェクトを分けられます。
    pub fn layer(mut self, layer: u32) -> Self {
        self.layer = layer;
        self
    }

    /// 奥行きの始まりと刻み。
    ///
    /// 命令 `index` 番目は `z_base + index * z_step` に置かれます。
    /// 3D や他の 2D の図形と重ねるときにずらしてください。
    pub fn depth(mut self, z_base: f32, z_step: f32) -> Self {
        self.z_base = z_base;
        self.z_step = z_step;
        self
    }

    /// 登録する図形の名前の頭。他と衝突するときに変えてください。
    pub fn name_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.name_prefix = prefix.into();
        self
    }

    /// いま持っている図形の数。
    pub fn object_count(&self) -> usize {
        self.slots.len() + self.recycled.len()
    }

    /// 使われずに取ってある図形の数。
    pub fn recycled_count(&self) -> usize {
        self.recycled.len()
    }

    /// 命令を図形に移す。
    ///
    /// `font` が `None` だと文字の命令は飛ばします。**字が出ないだけで、
    /// ほかの図形は出ます。**
    pub fn sync(
        &mut self,
        draw_manager: &mut DrawManager,
        list: &DisplayList,
        camera: Camera,
        font: Option<&Font>,
    ) {
        // **先に、今フレームで使う鍵を全部数える。**
        //
        // 割り当てながら空きを回収すると、中身が丸ごと入れ替わったフレームで
        // 図形が倍に増えます。新しい鍵のぶんを先に取ってから、古い鍵を
        // 空きに積むことになるので、取るときに空きが無いからです。
        // スクロールを端まで飛ばすと、まさにこれが起きます。
        self.live.clear();
        self.counters.clear();

        for command in list.commands() {
            let slot = self.next_slot(command.owner);
            self.live.insert((command.owner, slot));
        }

        self.retire_unused(draw_manager);

        self.counters.clear();

        for (index, command) in list.commands().iter().enumerate() {
            let key = (command.owner, self.next_slot(command.owner));
            let z = self.z_base + index as f32 * self.z_step;

            // 前と同じなら、重なり順だけ見る。
            if let Some(slot) = self.slots.get(&key)
                && slot.primitive == command.primitive
                && slot.clip == command.clip
            {
                if slot.z != z {
                    if let Some(object) = draw_manager.object_mut(&slot.name) {
                        object.z(z);
                    }

                    if let Some(slot) = self.slots.get_mut(&key) {
                        slot.z = z;
                    }
                }

                continue;
            }

            let name = match self.slots.get(&key) {
                Some(slot) => slot.name.clone(),
                None => self.acquire(draw_manager, camera),
            };

            write_object(
                draw_manager,
                &name,
                command.primitive.clone(),
                command.clip,
                camera,
                self.layer,
                z,
                font,
            );

            self.slots.insert(
                key,
                Slot {
                    name,
                    primitive: command.primitive.clone(),
                    clip: command.clip,
                    z,
                },
            );
        }
    }

    /// すべての図形を空にする。UI を丸ごと作り直すとき。
    ///
    /// 図形は捨てずに取っておくので、作り直したあとも増えません。
    pub fn clear(&mut self, draw_manager: &mut DrawManager) {
        for (_, slot) in self.slots.drain() {
            hide(draw_manager, &slot.name);
            self.recycled.push(slot.name);
        }

        self.counters.clear();
    }

    /// 空き一覧から取る。無ければ登録する。
    fn acquire(&mut self, draw_manager: &mut DrawManager, camera: Camera) -> String {
        if let Some(name) = self.recycled.pop() {
            return name;
        }

        self.minted += 1;

        let mut object = create_object(&format!("{} {}", self.name_prefix, self.minted));
        object.camera(camera);
        object.layer(self.layer);

        // 空で登録しておく。中身は呼んだ先で積む。
        object.begin(PaintType::Fill);
        object.end();

        draw_manager.register(object)
    }

    /// その人が次に使う枠の番号。
    fn next_slot(&mut self, owner: WidgetId) -> u32 {
        let counter = self.counters.entry(owner).or_insert(0);
        let current = *counter;
        *counter += 1;

        current
    }

    /// [`GuiRenderer::live`] に無い枠を空ける。
    fn retire_unused(&mut self, draw_manager: &mut DrawManager) {
        self.expired.clear();
        self.expired.extend(
            self.slots
                .keys()
                .filter(|key| !self.live.contains(*key))
                .copied(),
        );

        for key in self.expired.drain(..) {
            if let Some(slot) = self.slots.remove(&key) {
                hide(draw_manager, &slot.name);
                self.recycled.push(slot.name);
            }
        }
    }
}

/// 描かれないようにする。頂点を空にし、複製も外す。
fn hide(draw_manager: &mut DrawManager, name: &str) {
    let Some(object) = draw_manager.object_mut(name) else {
        return;
    };

    object.clear_instances();
    object.begin(PaintType::Fill);
    object.end();
}

/// 図形に積み直す。
#[allow(clippy::too_many_arguments)]
fn write_object(
    draw_manager: &mut DrawManager,
    name: &str,
    primitive: Primitive,
    clip: Option<ClipRegion>,
    camera: Camera,
    layer: u32,
    z: f32,
    font: Option<&Font>,
) {
    let Some(object) = draw_manager.object_mut(name) else {
        log::warn!("gui object '{name}' is gone; the command is dropped");
        return;
    };

    object.camera(camera);
    object.layer(layer);
    object.z(z);

    match &primitive {
        Primitive::Image { source, .. } => {
            object.sprite_region(source.page, uv_to_region(*source));
        }
        _ => {
            object.no_sprite();
        }
    }

    write_geometry(object, &primitive, font);

    // 切り抜きは色の段で不透明度に掛かる。積み直すので、先に空にする。
    let mut effects = EffectStack::new();

    if let Some(clip) = clip {
        effects.push(clip_block(clip));
    }

    object.set_effects(effects);

    // 色は頂点に入っているので、複製は変換なしの 1 つでよい。
    object.clear_instances();
    object.instance(create_instance());
}

fn write_geometry(object: &mut Object, primitive: &Primitive, font: Option<&Font>) {
    match primitive {
        Primitive::Rect {
            rect,
            corners,
            color,
        } => {
            object.begin(PaintType::Fill);
            put_ring(object, &rounded_outline(*rect, *corners), *color);
            object.end();
        }

        Primitive::Border {
            rect,
            corners,
            width,
            color,
        } => {
            // 線は道の真ん中に乗るので、内側に引くには半分だけ縮める。
            let half = width / 2.0;
            let inset = Rect::new(
                rect.x + half,
                rect.y + half,
                (rect.width - width).max(0.0),
                (rect.height - width).max(0.0),
            );
            let corners = shrink_corners(*corners, half);

            object.begin(PaintType::Stroke {
                line_width: *width,
                joint_type: JointType::Miter,
                strip: false,
                dash: None,
            });
            put_ring(object, &rounded_outline(inset, corners), *color);
            object.end();
        }

        Primitive::Ellipse { rect, color } => {
            object.begin(PaintType::Fill);
            put_ring(object, &ellipse_outline(*rect), *color);
            object.end();
        }

        Primitive::Polyline {
            points,
            width,
            color,
            closed,
        } => {
            object.begin(PaintType::Stroke {
                line_width: *width,
                joint_type: JointType::Miter,
                // `strip` は「開いた折れ線」。閉じた輪のときは倒す。
                strip: !*closed,
                dash: None,
            });
            put_ring(object, points, *color);
            object.end();
        }

        Primitive::Image { rect, tint, .. } => {
            // 絵は四隅に uv を貼る。切り出しはスプライト側で決まる。
            object.begin(PaintType::Fill);

            for (point, (u, v)) in corners_of(*rect)
                .into_iter()
                .zip([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])
            {
                object.put_vertex(Vertex::new_position_color_uv(
                    point.x, point.y, 0.0, tint.red, tint.green, tint.blue, tint.alpha, u, v,
                ));
            }

            object.end();
        }

        Primitive::Text {
            rect,
            text,
            options,
            color,
        } => match font {
            Some(font) => write_text(object, *rect, text, *options, *color, font),
            None => {
                // 書体が無い。空にしておく。次のフレームで来れば出る。
                object.begin(PaintType::Fill);
                object.end();
            }
        },
    }
}

/// 文字の輪郭を積む。
fn write_text(
    object: &mut Object,
    rect: Rect,
    text: &str,
    options: TextLayoutOptions,
    color: Color,
    font: &Font,
) {
    let style = TextStyle::new(options.size);

    let placed = if options.wrap {
        layout_wrapped(font, text, &style, rect.width)
    } else {
        layout(font, text, &style)
    };

    // 並べた結果は左上を原点としているので、箱の中へ寄せる。
    let left = rect.x + options.horizontal.offset(rect.width, placed.width);
    let top = rect.y + options.vertical.offset(rect.height, placed.height);

    object.begin(PaintType::Fill);

    let mut written = 0_usize;

    for glyph in &placed.glyphs {
        let Some(outline) = font.outline(glyph.glyph, style.tolerance) else {
            // 空白のように形を持たない字。送り幅だけで済んでいる。
            continue;
        };

        let size = glyph.style.size;
        let origin_x = left + glyph.x;
        let baseline = top + glyph.y;
        let skew = glyph.style.italic;

        let mut color = glyph.style.color.map(Color::from).unwrap_or(color);
        color.alpha *= glyph.style.alpha_scale;

        for (index, point) in outline.points.iter().enumerate() {
            if index == 0 || outline.contour_starts.contains(&index) {
                if written > 0 {
                    object.begin_hole();
                }

                written += 1;
            }

            // 形は em 単位なので大きさを掛ける。傾けるときは
            // y が下向きなので、字の上（y が負）ほど右へ寄る。
            object.put_vertex(vertex(
                Point::new(
                    origin_x + (point[0] - skew * point[1]) * size,
                    baseline + point[1] * size,
                ),
                color,
            ));
        }
    }

    object.end();
}

fn put_ring(object: &mut Object, points: &[Point], color: Color) {
    for point in points {
        object.put_vertex(vertex(*point, color));
    }
}

fn vertex(point: Point, color: Color) -> Vertex {
    Vertex::new_position_color(
        point.x,
        point.y,
        0.0,
        color.red,
        color.green,
        color.blue,
        color.alpha,
    )
}

fn corners_of(rect: Rect) -> [Point; 4] {
    [
        Point::new(rect.x, rect.y),
        Point::new(rect.right(), rect.y),
        Point::new(rect.right(), rect.bottom()),
        Point::new(rect.x, rect.bottom()),
    ]
}

/// 角の丸い矩形の輪郭。丸みが 0 なら 4 点で返る。
fn rounded_outline(rect: Rect, corners: Corners) -> Vec<Point> {
    let corners = corners.clamp_to(rect.size());

    if corners.is_zero() {
        return corners_of(rect).to_vec();
    }

    let mut points = Vec::new();

    // 左上から時計回り。中心・始まりの角・半径の組で 4 回。
    let arcs = [
        (
            Point::new(rect.x + corners.top_left, rect.y + corners.top_left),
            corners.top_left,
            std::f32::consts::PI,
        ),
        (
            Point::new(rect.right() - corners.top_right, rect.y + corners.top_right),
            corners.top_right,
            std::f32::consts::FRAC_PI_2 * 3.0,
        ),
        (
            Point::new(
                rect.right() - corners.bottom_right,
                rect.bottom() - corners.bottom_right,
            ),
            corners.bottom_right,
            0.0,
        ),
        (
            Point::new(
                rect.x + corners.bottom_left,
                rect.bottom() - corners.bottom_left,
            ),
            corners.bottom_left,
            std::f32::consts::FRAC_PI_2,
        ),
    ];

    for (center, radius, start) in arcs {
        if radius <= 0.0 {
            points.push(center);
            continue;
        }

        // 四分円ぶん。半径が大きいほど細かく刻む。
        let steps = (circle_steps(radius) / 4).max(MIN_CORNER_STEPS);

        for step in 0..=steps {
            let angle = start + std::f32::consts::FRAC_PI_2 * step as f32 / steps as f32;

            points.push(Point::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            ));
        }
    }

    points
}

/// 内側へ引いた丸み。枠線を内側に寄せるときに使う。
fn shrink_corners(corners: Corners, amount: f32) -> Corners {
    Corners::new(
        (corners.top_left - amount).max(0.0),
        (corners.top_right - amount).max(0.0),
        (corners.bottom_right - amount).max(0.0),
        (corners.bottom_left - amount).max(0.0),
    )
}

/// 矩形に内接する楕円の輪郭。
fn ellipse_outline(rect: Rect) -> Vec<Point> {
    let radius_x = rect.width / 2.0;
    let radius_y = rect.height / 2.0;
    let center = rect.center();

    // 細かさは大きいほうの半径で決める。
    let steps = circle_steps(radius_x.max(radius_y));

    (0..steps)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / steps as f32;

            Point::new(
                center.x + radius_x * angle.cos(),
                center.y + radius_y * angle.sin(),
            )
        })
        .collect()
}

/// 切り抜きを色の段の山に直す。
///
/// [`Block::ClipRect`] の丸みは 1 つなので、**四隅がばらばらなら
/// いちばん大きいものに揃えます。** UI でここが問題になることはまずありません。
fn clip_block(clip: ClipRegion) -> Block {
    let (min, max) = clip.rect.to_min_max();

    Block::ClipRect {
        min,
        max,
        radius: clip.corners.max(),
        softness: 0.0,
        invert: false,
    }
}

fn uv_to_region(source: ImageSource) -> [f32; 4] {
    source.uv_rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::geometry::Size;

    #[test]
    fn a_square_corner_gives_four_points() {
        let outline = rounded_outline(Rect::new(0.0, 0.0, 10.0, 10.0), Corners::ZERO);

        assert_eq!(outline.len(), 4);
        assert_eq!(outline[0], Point::new(0.0, 0.0));
        assert_eq!(outline[2], Point::new(10.0, 10.0));
    }

    #[test]
    fn rounded_outline_stays_inside_the_rect() {
        let rect = Rect::new(0.0, 0.0, 100.0, 40.0);
        let outline = rounded_outline(rect, Corners::all(8.0));

        assert!(outline.len() > 4);

        for point in &outline {
            assert!(
                point.x >= rect.x - 0.01 && point.x <= rect.right() + 0.01,
                "{point:?}"
            );
            assert!(
                point.y >= rect.y - 0.01 && point.y <= rect.bottom() + 0.01,
                "{point:?}"
            );
        }
    }

    #[test]
    fn huge_corners_are_clamped_to_the_shorter_side() {
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let outline = rounded_outline(rect, Corners::all(500.0));

        // 高さの半分（10）で頭打ち。はみ出さない。
        for point in &outline {
            assert!(point.y >= -0.01 && point.y <= 20.01, "{point:?}");
        }
    }

    #[test]
    fn an_ellipse_spans_the_rect() {
        let outline = ellipse_outline(Rect::new(0.0, 0.0, 100.0, 50.0));

        let left = outline.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let right = outline.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);

        assert!((left - 0.0).abs() < 0.5, "{left}");
        assert!((right - 100.0).abs() < 0.5, "{right}");
    }

    #[test]
    fn borders_shrink_their_corners_with_the_inset() {
        let shrunk = shrink_corners(Corners::all(10.0), 2.0);

        assert_eq!(shrunk.max(), 8.0);
        // 引きすぎても負にはならない。
        assert_eq!(shrink_corners(Corners::all(1.0), 5.0).max(), 0.0);
    }

    #[test]
    fn a_clip_becomes_a_rect_block() {
        let clip = ClipRegion::new(Rect::new(10.0, 20.0, 30.0, 40.0), Corners::all(4.0));

        match clip_block(clip) {
            Block::ClipRect {
                min,
                max,
                radius,
                invert,
                ..
            } => {
                assert_eq!(min, [10.0, 20.0]);
                assert_eq!(max, [40.0, 60.0]);
                assert_eq!(radius, 4.0);
                assert!(!invert);
            }
            other => panic!("expected a rect clip, got {other:?}"),
        }
    }

    #[test]
    fn uneven_corners_collapse_to_the_largest() {
        let clip = ClipRegion::new(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Corners::new(2.0, 9.0, 1.0, 4.0),
        );

        let Block::ClipRect { radius, .. } = clip_block(clip) else {
            panic!("expected a rect clip");
        };

        assert_eq!(radius, 9.0);
    }

    #[test]
    fn corner_outline_scales_with_the_radius() {
        let small = rounded_outline(Rect::new(0.0, 0.0, 200.0, 200.0), Corners::all(2.0));
        let large = rounded_outline(Rect::new(0.0, 0.0, 200.0, 200.0), Corners::all(60.0));

        assert!(
            large.len() > small.len(),
            "大きい丸みは細かく刻む: {} vs {}",
            large.len(),
            small.len()
        );
    }

    #[test]
    fn a_zero_sized_rect_still_produces_a_ring() {
        // 空の矩形は `DisplayList` が先に落とすので、ここへは来ない。
        // それでも落ちないことだけ確かめる。
        let outline = rounded_outline(Rect::ZERO, Corners::all(4.0));
        assert_eq!(outline.len(), 4, "丸みは 0 に潰れる");

        let _ = Size::ZERO;
    }
}
