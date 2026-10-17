use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::gui::number::{decimal_places, from_f64, is_integer, step_value, to_f64, NumberValue};
use crate::gui::window_frame::{Gap, ThemeColor};
use crate::gui::window_item::{built_point, replace_outline, CursorShape, EditKey, Pointer, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::{create_instance, Instance};
use crate::object::Object;
use crate::objects::circle::Circle;
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::PaintType;

/// 既定のいちばん小さい値（HTML の `min` の既定）。
const DEFAULT_MIN: f64 = 0.0;
/// 既定のいちばん大きい値（HTML の `max` の既定）。
const DEFAULT_MAX: f64 = 100.0;
/// 既定の刻み（HTML の `step` の既定）。
const DEFAULT_STEP: f64 = 1.0;
/// 欄の高さに対する、つまみの半径の割合。
const THUMB_RADIUS_RATIO: f32 = 0.4;
/// つまみの半径に対する、溝の太さの割合。
const TRACK_THICKNESS_RATIO: f32 = 0.5;
/// フォーカスの輪の太さと、つまみからの離れ。
const FOCUS_RING_WIDTH: f32 = 2.0;
const FOCUS_RING_GAP: f32 = 2.0;
/// 円を組む細かさ。
const CIRCLE_SEGMENTS: usize = 32;
/// PageUp・PageDown で動かす、幅に対する割合（ブラウザと同じく 1 割）。
const PAGE_RATIO: f64 = 0.1;
/// 刻みが「any」のとき、矢印キーで動かす、幅に対する割合。
const ANY_STEP_RATIO: f64 = 0.01;
/// 形は白で組み、色は複製の色で掛ける。
const WHITE: ThemeColor = ThemeColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
const CLEAR: ThemeColor = ThemeColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// 範囲の中から値を選ぶつまみ。Web の `<input type="range">` と同じ仕組みです。
///
/// 値の型は `T`（[`NumberValue`]）で、`Range::<i32>` なら整数、`Range::<f32>` なら小数を扱います。
///
/// - 値はいつも `min` から `max` のあいだで、刻みにそろっています。既定は `min` 0・`max` 100・刻み 1、
///   値は `min` と `max` のまん中です。`max` が `min` より小さければ、`max` は `min` とみなします。
///   刻みの基準は `min` で、刻みにそろわないときは近いほう（ちょうど半分なら大きいほう）へ寄せます。
/// - 溝を押すとつまみがそこへ飛び、そのまま引けます。つまみを掴んで引くと、掴んだ場所がずれずに付いてきます。
/// - フォーカスがあるあいだ、←↓ で 1 刻み減らし、→↑ で増やします。PageUp・PageDown は幅の 1 割、
///   Home・End は `min`・`max` へ。
/// - 値が変わるたびに [`Range::on_input`] を、引き終えたとき・キーで動かしたときに、前と値が
///   変わっていれば [`Range::on_change`] を呼びます（HTML の `input` と `change` の出来事）。
///
/// 溝の `min` からつまみまでは [`RangeTheme::fill_color`] で塗ります（ブラウザの見た目と同じ）。
///
/// # 動かし方
///
/// 頂点を書き換えると、次の [`DrawManager::prepare`] で**すべての図形の頂点**を積み直すことになり、
/// 引くたびにかくつきます。そこで頂点は組むとき（と窓の大きさが変わったとき）だけ作り、
/// 値や見た目が変わったときは複製（[`Instance`]）の位置・横の倍率・色だけを書き換えます。
/// 複製はその図形のぶんだけ送り直されるので軽く済みます。
///
/// - つまみとフォーカスの輪は、原点のまわりに組んで、複製の位置で動かす。
/// - 塗った溝は、左端の丸い頭（動かない）と、幅 1 の四角を複製の横の倍率で伸ばしたものに分ける。
///   右端はつまみの下に隠れる。
/// - 色（指が乗る・押す・フォーカスの輪の出し入れ）は、白で組んだ形に複製の色を掛ける。
pub struct Range<T: NumberValue> {
    /// 登録の名前のもと。
    name: String,
    width: f32,
    height: f32,
    gap: Gap,
    range_theme: RangeTheme,
    thumb_radius: f32,
    track_thickness: f32,
    min: f64,
    max: f64,
    /// 刻み。`None` なら「any」。
    step: Option<f64>,
    /// 決められた値。そろえる前のもの。`None` ならまん中。
    raw: Option<f64>,
    on_input: Option<Box<dyn FnMut(T)>>,
    on_change: Option<Box<dyn FnMut(T)>>,
    range_register_name: RangeRegisterName,
    /// 組んだときの左上（余白を足す前）。
    built_at: [f32; 2],
    focused: bool,
    /// 指がつまみの上にあるか。
    hovered: bool,
    /// 引いているあいだ、指とつまみのまん中との横のずれ。溝を押したときは 0。
    drag_offset: Option<f32>,
    /// 引き始めたときの値。放したとき、変わっていれば `change` を知らせる。
    drag_start: f64,
    _value: std::marker::PhantomData<T>,
}

#[derive(Default)]
struct RangeRegisterName {
    background: Option<String>,
    track: Option<String>,
    fill_cap: Option<String>,
    fill: Option<String>,
    focus_ring: Option<String>,
    thumb: Option<String>,
}

pub struct RangeTheme {
    /// 溝の色。
    pub track_color: ThemeColor,
    /// 溝の、`min` からつまみまでの色。
    pub fill_color: ThemeColor,
    pub thumb_color: ThemeColor,
    /// 指がつまみの上にあるとき。
    pub thumb_hover_color: ThemeColor,
    /// 引いているあいだ。
    pub thumb_pressed_color: ThemeColor,
    /// フォーカスがあるときに、つまみの周りに出す輪。
    pub focus_ring_color: ThemeColor,
}

impl<T: NumberValue> Range<T> {
    pub fn new(register_name_string: String, width: f32, height: f32, gap: Gap, range_theme: RangeTheme) -> Self {
        let thumb_radius = (height * THUMB_RADIUS_RATIO).max(0.0);

        Self {
            name: register_name_string,
            width,
            height,
            gap,
            range_theme,
            thumb_radius,
            track_thickness: thumb_radius * TRACK_THICKNESS_RATIO,
            min: DEFAULT_MIN,
            max: DEFAULT_MAX,
            step: Some(DEFAULT_STEP),
            raw: None,
            on_input: None,
            on_change: None,
            range_register_name: RangeRegisterName::default(),
            built_at: [0.0; 2],
            focused: false,
            hovered: false,
            drag_offset: None,
            drag_start: 0.0,
            _value: std::marker::PhantomData,
        }
    }

    /// いちばん小さい値（HTML の `min`）。既定は 0。
    pub fn min(mut self, min: T) -> Self {
        self.min = to_f64(min);
        self
    }

    /// いちばん大きい値（HTML の `max`）。既定は 100。
    pub fn max(mut self, max: T) -> Self {
        self.max = to_f64(max);
        self
    }

    /// 刻み（HTML の `step`）。0 以下なら既定の 1 にする（HTML と同じ）。
    pub fn step(mut self, step: T) -> Self {
        let step = to_f64(step);
        self.step = Some(if step.is_finite() && step > 0.0 { step } else { DEFAULT_STEP });
        self
    }

    /// 刻みを問わない（HTML の `step="any"`）。整数の型では 1 ずつのまま。
    pub fn step_any(mut self) -> Self {
        self.step = None;
        self
    }

    /// 初めの値（HTML の `value`）。範囲と刻みにそろえて使う。
    pub fn value(mut self, value: T) -> Self {
        self.raw = Some(to_f64(value));
        self
    }

    /// つまみの半径。既定は高さの 0.4 倍。
    pub fn thumb_radius(mut self, thumb_radius: f32) -> Self {
        self.thumb_radius = thumb_radius.max(0.0);
        self
    }

    /// 溝の太さ。既定はつまみの半径の半分。
    pub fn track_thickness(mut self, track_thickness: f32) -> Self {
        self.track_thickness = track_thickness.max(0.0);
        self
    }

    /// 値が変わるたびに呼ぶ関数（HTML の `input`）。
    pub fn on_input(mut self, on_input: impl FnMut(T) + 'static) -> Self {
        self.on_input = Some(Box::new(on_input));
        self
    }

    /// 値が決まったときに呼ぶ関数（HTML の `change`）。引き終えたとき・キーで動かしたときに、
    /// 前から値が変わっていれば呼ぶ。
    pub fn on_change(mut self, on_change: impl FnMut(T) + 'static) -> Self {
        self.on_change = Some(Box::new(on_change));
        self
    }

    /// いまの値。
    pub fn get_value(&self) -> T {
        let value = self.current();
        from_f64(value)
            .or_else(|| from_f64(value.round()))
            .unwrap_or_else(T::zero)
    }

    /// 値を差し替える（範囲と刻みにそろえる）。知らせは呼ばない（HTML と同じ）。変わったら `true`。
    pub fn set_value(&mut self, draw_manager: &mut DrawManager, value: T) -> bool {
        let before = self.current();
        self.raw = Some(to_f64(value));
        self.place(draw_manager);

        self.current() != before
    }

    /// `count` 刻み増やす（HTML の `stepUp()`）。変わったら `true`。知らせも呼ぶ。
    pub fn step_up(&mut self, draw_manager: &mut DrawManager, count: u32) -> bool {
        self.apply_steps(draw_manager, count as i32)
    }

    /// `count` 刻み減らす（HTML の `stepDown()`）。変わったら `true`。知らせも呼ぶ。
    pub fn step_down(&mut self, draw_manager: &mut DrawManager, count: u32) -> bool {
        self.apply_steps(draw_manager, -(count as i32))
    }

    // --- 値 ---

    /// 実際に使う `max`。`min` より小さければ `min`。
    fn effective_max(&self) -> f64 {
        self.max.max(self.min)
    }

    /// そろえるときの刻み。「any」でも、整数の型なら 1。
    fn effective_step(&self) -> Option<f64> {
        self.step.or_else(|| is_integer::<T>().then_some(DEFAULT_STEP))
    }

    /// いまの値（範囲と刻みにそろえたもの）。
    fn current(&self) -> f64 {
        let (min, max) = (self.min, self.effective_max());
        let raw = self.raw.unwrap_or(min + (max - min) / 2.0);

        sanitize(raw, min, max, self.effective_step())
    }

    /// `value` を値にして、つまみを動かす。変わったら `input` を知らせて `true`。
    fn move_to(&mut self, draw_manager: &mut DrawManager, value: f64) -> bool {
        let before = self.current();
        self.raw = Some(value);
        if self.current() == before {
            return false;
        }

        self.place(draw_manager);
        self.notify_input();
        true
    }

    /// キーで動かす。`input` と `change` を続けて知らせる。
    fn move_by_key(&mut self, draw_manager: &mut DrawManager, value: f64) -> bool {
        let moved = self.move_to(draw_manager, value);
        if moved {
            self.notify_change();
        }
        moved
    }

    fn apply_steps(&mut self, draw_manager: &mut DrawManager, steps: i32) -> bool {
        let Some(step) = self.step else {
            // HTML では「any」で stepUp() を呼ぶと誤りになる。ここでは何もしない。
            return false;
        };
        let Some(value) = step_value(Some(self.current()), steps, Some(self.min), Some(self.effective_max()), step, self.min) else {
            return false;
        };

        self.move_by_key(draw_manager, value)
    }

    /// 矢印キー 1 回で動かす量。
    fn arrow_step(&self) -> f64 {
        self.effective_step().unwrap_or((self.effective_max() - self.min) * ANY_STEP_RATIO)
    }

    /// PageUp・PageDown 1 回で動かす量。幅の 1 割で、刻みより小さくはしない。
    fn page_step(&self) -> f64 {
        ((self.effective_max() - self.min) * PAGE_RATIO).max(self.arrow_step())
    }

    fn notify_input(&mut self) {
        let value = self.get_value();
        if let Some(on_input) = self.on_input.as_mut() {
            on_input(value);
        }
    }

    fn notify_change(&mut self) {
        let value = self.get_value();
        if let Some(on_change) = self.on_change.as_mut() {
            on_change(value);
        }
    }

    // --- 形 ---

    /// 部品の左上と右下（組んだときの座標）。
    fn bounds(&self) -> [f32; 4] {
        let [x, y] = self.built_at;
        let (left, top) = (x + self.gap.x, y + self.gap.y);

        [left, top, left + self.width, top + self.height]
    }

    /// つまみのまん中が動く左端と右端。つまみが部品からはみ出さないよう、半径ぶん内側。
    fn travel(&self) -> [f32; 2] {
        let [left, _, right, _] = self.bounds();
        let radius = self.thumb_radius.min(self.width / 2.0);

        [left + radius, right - radius]
    }

    /// 値の、`min` から `max` までのうちの割合（0 から 1）。
    fn ratio(&self) -> f32 {
        let (min, max) = (self.min, self.effective_max());
        if max <= min {
            return 0.0;
        }

        ((self.current() - min) / (max - min)).clamp(0.0, 1.0) as f32
    }

    /// つまみのまん中（組んだときの座標）。
    fn thumb_center(&self) -> [f32; 2] {
        let [start, end] = self.travel();
        let [_, top, _, bottom] = self.bounds();

        [start + (end - start) * self.ratio(), (top + bottom) / 2.0]
    }

    /// 組んだときの座標の x を、値へ（そろえる前）。
    fn value_at(&self, local_x: f32) -> f64 {
        let [start, end] = self.travel();
        let ratio = if end > start { ((local_x - start) / (end - start)).clamp(0.0, 1.0) } else { 0.0 };

        self.min + (self.effective_max() - self.min) * ratio as f64
    }

    fn vertex(x: f32, y: f32, color: &ThemeColor) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a)
    }

    /// 部品をまるごと覆う、見えない四角。溝の上下を押しても当たるように。
    fn build_background(&self, name: &str) -> Object {
        let [left, top, right, bottom] = self.bounds();

        Rect::new(name)
            .paint_type(PaintType::Fill)
            .from(Self::vertex(left, top, &CLEAR))
            .to(Self::vertex(right, bottom, &CLEAR))
            .end()
    }

    /// 溝の縦のまん中。
    fn center_y(&self) -> f32 {
        let [_, top, _, bottom] = self.bounds();
        (top + bottom) / 2.0
    }

    /// 横に `left` から `right` までの、両端の丸い帯（白）。
    fn build_pill(&self, name: &str, left: f32, right: f32) -> Object {
        let center = self.center_y();
        let half = self.track_thickness / 2.0;
        let right = right.max(left);
        let radius = half.min((right - left) / 2.0);
        let (from, to) = (Self::vertex(left, center - half, &WHITE), Self::vertex(right, center + half, &WHITE));

        if radius > 0.0 {
            RectRounded::new(name)
                .paint_type(PaintType::Fill)
                .from(CornerType::Circle { radius }, from)
                .to(to)
                .end()
        } else {
            Rect::new(name).paint_type(PaintType::Fill).from(from).to(to).end()
        }
    }

    fn build_track(&self, name: &str) -> Object {
        let [left, _, right, _] = self.bounds();
        self.build_pill(name, left, right)
    }

    /// 塗った溝の左端の丸い頭。溝の太さぶんの丸。
    fn build_fill_cap(&self, name: &str) -> Object {
        let [left, _, right, _] = self.bounds();
        self.build_pill(name, left, (left + self.track_thickness).min(right))
    }

    /// 塗った溝の胴。x が 0 から 1 の四角で、複製の位置と横の倍率で頭からつまみまで伸ばす。
    fn build_fill(&self, name: &str) -> Object {
        let center = self.center_y();
        let half = self.track_thickness / 2.0;

        Rect::new(name)
            .paint_type(PaintType::Fill)
            .from(Self::vertex(0.0, center - half, &WHITE))
            .to(Self::vertex(1.0, center + half, &WHITE))
            .end()
    }

    /// 原点を中心にした丸（白）。複製の位置で置く。
    fn build_circle(name: &str, radius: f32, paint_type: PaintType) -> Object {
        Circle::new(name)
            .paint_type(paint_type)
            .vertex_center(Self::vertex(0.0, 0.0, &WHITE))
            .radius(radius)
            .segment_count(CIRCLE_SEGMENTS)
            .end_circle_option()
            .end()
    }

    fn build_focus_ring(&self, name: &str) -> Object {
        let radius = self.thumb_radius + FOCUS_RING_GAP + FOCUS_RING_WIDTH / 2.0;
        Self::build_circle(name, radius, PaintType::stroke(FOCUS_RING_WIDTH))
    }

    fn build_thumb(&self, name: &str) -> Object {
        Self::build_circle(name, self.thumb_radius, PaintType::Fill)
    }

    /// 頂点を、いまの場所と大きさで組み直す。**全図形の積み直しになるので、窓の大きさが変わったときだけ。**
    fn rebuild_shapes(&self, draw_manager: &mut DrawManager) {
        let names = &self.range_register_name;
        let shapes = [
            (&names.background, self.build_background("")),
            (&names.track, self.build_track("")),
            (&names.fill_cap, self.build_fill_cap("")),
            (&names.fill, self.build_fill("")),
            (&names.focus_ring, self.build_focus_ring("")),
            (&names.thumb, self.build_thumb("")),
        ];

        for (name, built) in shapes {
            if let Some(object) = name.as_deref().and_then(|name| draw_manager.object_mut(name)) {
                replace_outline(object, &built);
            }
        }
    }

    /// いまのつまみの色。
    fn thumb_color(&self) -> &ThemeColor {
        let theme = &self.range_theme;

        if self.drag_offset.is_some() {
            &theme.thumb_pressed_color
        } else if self.hovered {
            &theme.thumb_hover_color
        } else {
            &theme.thumb_color
        }
    }

    /// 値と見た目を、複製の位置・横の倍率・色で映す。頂点には触らない。
    fn place(&self, draw_manager: &mut DrawManager) {
        let theme = &self.range_theme;
        let names = &self.range_register_name;
        let [thumb_x, thumb_y] = self.thumb_center();
        // 胴は頭のまん中から、つまみのまん中まで。
        let fill_start = self.bounds()[0] + self.track_thickness / 2.0;
        let fill_length = (thumb_x - fill_start).max(0.0);
        let ring = &theme.focus_ring_color;
        let ring_color = ThemeColor { a: if self.focused { ring.a } else { 0.0 }, ..*ring };

        let placements = [
            (&names.background, [0.0, 0.0], 1.0, &CLEAR),
            (&names.track, [0.0, 0.0], 1.0, &theme.track_color),
            (&names.fill_cap, [0.0, 0.0], 1.0, &theme.fill_color),
            (&names.fill, [fill_start, 0.0], fill_length, &theme.fill_color),
            (&names.focus_ring, [thumb_x, thumb_y], 1.0, &ring_color),
            (&names.thumb, [thumb_x, thumb_y], 1.0, self.thumb_color()),
        ];

        for (name, [x, y], scale_x, color) in placements {
            let Some(object) = name.as_deref().and_then(|name| draw_manager.object_mut(name)) else {
                continue;
            };
            let instance = create_instance()
                .translate(x, y, 0.0)
                .scale(scale_x, 1.0, 1.0)
                .color(color.r, color.g, color.b, color.a);

            // 同じなら触らない。引くと送り直しの印が付く。
            if object.instances().first() != Some(&instance) {
                set_first_instance(object, instance);
            }
        }
    }

    // --- 指 ---

    /// 部品の枠（つまみと輪がはみ出すぶんも含む）の中か。組んだときの座標で見る。
    fn inside(&self, [x, y]: [f32; 2]) -> bool {
        let [left, top, right, bottom] = self.bounds();
        let reach = (self.thumb_radius + FOCUS_RING_GAP + FOCUS_RING_WIDTH - self.height / 2.0).max(0.0);

        x >= left && x <= right && y >= top - reach && y <= bottom + reach
    }

    /// 指の下で一番手前にあるのが、この部品の図形か。
    ///
    /// [`DrawManager::pick`] は全図形を手前から当てるので重い。指が動くたびに呼ばれるので、
    /// 枠の外なら座標だけで断る。枠の中のときだけ、上に別の窓が重なっていないかを `pick` で見る。
    fn hit(&self, draw_manager: &DrawManager, pointer: &Pointer) -> bool {
        if !self.local(draw_manager, pointer.x, pointer.y).is_some_and(|local| self.inside(local)) {
            return false;
        }

        let Some(pick) = pointer.pick(draw_manager) else {
            return false;
        };

        let names = &self.range_register_name;
        [&names.background, &names.track, &names.fill_cap, &names.fill, &names.focus_ring, &names.thumb]
            .into_iter()
            .any(|name| name.as_deref() == Some(pick.name.as_str()))
    }

    /// 指がつまみの上か（形の当たり判定ではなく、円の内側かで見る）。
    fn over_thumb(&self, local: [f32; 2]) -> bool {
        let [center_x, center_y] = self.thumb_center();
        let (dx, dy) = (local[0] - center_x, local[1] - center_y);

        dx * dx + dy * dy <= self.thumb_radius * self.thumb_radius
    }

    /// 画素の位置を、組んだときの座標へ。窓のずれもここで外れる。
    fn local(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<[f32; 2]> {
        built_point(draw_manager, self.range_register_name.background.as_deref(), x, y)
    }
}

impl<T: NumberValue> WindowItem for Range<T> {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let counter = window_item_arguments.counter;
        let name = format!("{register_name}-{}-{counter}", self.name);

        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];

        let surface_size = window_item_arguments.surface_size;
        let design = window_item_arguments.design;
        let scale_mode = window_item_arguments.scale_mode;
        let draw_manager = window_item_arguments.draw_manager;
        let mut register = |mut object: Object| {
            object.camera_for(surface_size, design, scale_mode);
            object.instance(create_instance());
            // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
            Some(draw_manager.register(object))
        };

        // 見えない地、溝、塗った溝、フォーカスの輪、つまみの順に重ねる。
        let background = register(self.build_background(&name));
        let track = register(self.build_track(&format!("{name}-track")));
        let fill_cap = register(self.build_fill_cap(&format!("{name}-fill-cap")));
        let fill = register(self.build_fill(&format!("{name}-fill")));
        let focus_ring = register(self.build_focus_ring(&format!("{name}-focus-ring")));
        let thumb = register(self.build_thumb(&format!("{name}-thumb")));

        self.range_register_name = RangeRegisterName { background, track, fill_cap, fill, focus_ring, thumb };
        self.place(draw_manager);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];
        self.rebuild_shapes(window_item_arguments.draw_manager);
        self.place(window_item_arguments.draw_manager);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn object_names(&self, _draw_manager: &mut DrawManager) -> Vec<String> {
        let names = &self.range_register_name;
        [&names.background, &names.track, &names.fill_cap, &names.fill, &names.focus_ring, &names.thumb]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }

    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        let (x, y) = (pointer.x, pointer.y);
        if !self.hit(draw_manager, pointer) {
            return false;
        }
        let Some(local) = self.local(draw_manager, x, y) else {
            return false;
        };

        self.focused = true;
        self.drag_start = self.current();

        // つまみを掴んだら掴んだ場所のまま、溝を押したらつまみをそこへ飛ばす（ブラウザと同じ）。
        let offset = if self.over_thumb(local) { local[0] - self.thumb_center()[0] } else { 0.0 };
        self.drag_offset = Some(offset);
        self.hovered = true;

        if !self.move_to(draw_manager, self.value_at(local[0] - offset)) {
            self.place(draw_manager);
        }
        true
    }

    fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        let (x, y) = (pointer.x, pointer.y);
        if self.drag_offset.take().is_none() {
            return false;
        }

        self.hovered = self.local(draw_manager, x, y).is_some_and(|local| self.over_thumb(local)) && self.hit(draw_manager, pointer);
        self.place(draw_manager);

        if self.current() != self.drag_start {
            self.notify_change();
        }
        true
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        let Some(local) = self.local(draw_manager, pointer.x, pointer.y) else {
            return false;
        };

        // 引いているあいだは、部品の外へ出ても付いてくる。
        if let Some(offset) = self.drag_offset {
            return self.move_to(draw_manager, self.value_at(local[0] - offset));
        }

        let hovered = self.over_thumb(local) && self.hit(draw_manager, pointer);
        if hovered == self.hovered {
            return false;
        }

        self.hovered = hovered;
        self.place(draw_manager);
        true
    }

    fn has_focus(&self) -> bool {
        self.focused
    }

    fn unfocus(&mut self, draw_manager: &mut DrawManager) -> bool {
        if !self.focused {
            return false;
        }

        self.focused = false;
        self.place(draw_manager);
        true
    }

    fn wants_ime(&self) -> bool {
        false
    }

    fn key_pressed(&mut self, draw_manager: &mut DrawManager, _resources: &crate::resource::Resources, key: EditKey) -> bool {
        let value = self.current();

        match key {
            EditKey::Left | EditKey::Down => self.move_by_key(draw_manager, value - self.arrow_step()),
            EditKey::Right | EditKey::Up => self.move_by_key(draw_manager, value + self.arrow_step()),
            EditKey::PageDown => self.move_by_key(draw_manager, value - self.page_step()),
            EditKey::PageUp => self.move_by_key(draw_manager, value + self.page_step()),
            EditKey::Home => self.move_by_key(draw_manager, self.min),
            EditKey::End => self.move_by_key(draw_manager, self.effective_max()),
            EditKey::Escape => self.unfocus(draw_manager),
            _ => false,
        }
    }

    fn cursor_shape(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<CursorShape> {
        // 引いているあいだは当てるまでもない。指が動くたびに聞かれるので、pick を省く。
        if self.drag_offset.is_some() {
            return Some(CursorShape::Default);
        }

        self.hit(draw_manager, pointer).then_some(CursorShape::Default)
    }
}

/// 図形の 1 つめの複製を差し替える（組んだときに 1 つ足してある）。
fn set_first_instance(object: &mut Object, instance: Instance) {
    match object.instance_mut(0) {
        Some(first) => *first = instance,
        None => {
            object.instance(instance);
        }
    }
}

/// HTML の range の値のそろえ方。
///
/// 1. `min` から `max` のあいだに収める（`max` < `min` なら `min`）。
/// 2. 刻みがあれば、`min` を基準に一番近い刻みへ（ちょうど半分なら大きいほう）。
///    `max` を超えたら、`max` 以下で一番大きい刻みへ。
/// 3. 足し算の誤差（0.30000000000000004 など）を、刻みと `min` の桁で落とす。
pub(crate) fn sanitize(value: f64, min: f64, max: f64, step: Option<f64>) -> f64 {
    let max = max.max(min);
    let value = if value.is_finite() { value.clamp(min, max) } else { min };

    let Some(step) = step.filter(|step| step.is_finite() && *step > 0.0) else {
        return value;
    };

    let mut snapped = min + ((value - min) / step).round() * step;
    if snapped > max + step * 1e-9 {
        snapped = min + ((max - min) / step + 1e-9).floor() * step;
    }

    let decimals = decimal_places(step).max(decimal_places(min));
    format!("{snapped:.decimals$}").parse().unwrap_or(snapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_clamped_and_snapped_like_html() {
        assert_eq!(sanitize(50.0, 0.0, 100.0, Some(1.0)), 50.0);
        assert_eq!(sanitize(-5.0, 0.0, 100.0, Some(1.0)), 0.0);
        assert_eq!(sanitize(150.0, 0.0, 100.0, Some(1.0)), 100.0);
        assert_eq!(sanitize(7.0, 0.0, 100.0, Some(5.0)), 5.0, "近いほうへ");
        assert_eq!(sanitize(7.5, 0.0, 100.0, Some(5.0)), 10.0, "ちょうど半分なら大きいほうへ");
        assert_eq!(sanitize(4.0, 1.0, 100.0, Some(2.0)), 5.0, "基準は min");
    }

    #[test]
    fn the_largest_step_below_max_wins_over_max() {
        // 0, 3, 6, 9 の刻みで max 10。10 に近い 9 へ。
        assert_eq!(sanitize(10.0, 0.0, 10.0, Some(3.0)), 9.0);
        assert_eq!(sanitize(9.8, 0.0, 10.0, Some(3.0)), 9.0, "12 へは寄せない");
    }

    #[test]
    fn max_below_min_collapses_to_min() {
        assert_eq!(sanitize(50.0, 10.0, 5.0, Some(1.0)), 10.0);
    }

    #[test]
    fn decimal_steps_drop_float_error() {
        assert_eq!(sanitize(0.30000000000000004, 0.0, 1.0, Some(0.1)), 0.3);
        assert_eq!(sanitize(0.26, 0.0, 1.0, Some(0.1)), 0.3);
    }

    #[test]
    fn step_any_keeps_the_value() {
        assert_eq!(sanitize(12.345, 0.0, 100.0, None), 12.345);
        assert_eq!(sanitize(f64::NAN, 3.0, 100.0, None), 3.0);
    }
}
