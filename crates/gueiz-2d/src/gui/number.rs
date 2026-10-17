use std::time::{Duration, Instant};
use num_traits::{Num, NumCast};
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::gui::context_menu::ContextMenuRequest;
use crate::gui::input_text::{InputText, InputTextFont, InputTextTheme};
use crate::gui::window_frame::{Gap, QuadType, ThemeColor};
use crate::gui::window_item::{replace_outline, Animation, CursorShape, EditKey, KeyModifiers, Pointer, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::rect::Rect;
use crate::objects::triangle::Triangle;
use crate::paint_type::PaintType;
use crate::resource::{FontHandle, Resources};

/// 縁取りの太さ（[`InputText`] と同じ）。スピンボタンはこの内側に置く。
const OUTLINE_WIDTH: f32 = 1.0;
/// スピンボタンを押したままにしたとき、続けて動き始めるまで。
const REPEAT_DELAY: Duration = Duration::from_millis(400);
/// 押したままのあいだ、続けて動かす間隔。
const REPEAT_INTERVAL: Duration = Duration::from_millis(50);
/// 欄の高さに対する、スピンボタンの幅の割合。
const SPIN_WIDTH_RATIO: f32 = 0.6;
/// 「1 つずつ」の既定の刻み（HTML の `step` の既定）。
const DEFAULT_STEP: f64 = 1.0;

/// [`Number`] で扱える数の型。`i32` や `u8`、`f32`、`f64` など。
///
/// 中では `f64` で数えるので、`i64` などで 2^53 を超える値は丸まります。
pub trait NumberValue: Num + NumCast + Copy + PartialOrd + 'static {}

impl<T: Num + NumCast + Copy + PartialOrd + 'static> NumberValue for T {}

/// `T` が整数の型か。0.5 を入れて 0.5 のまま戻らなければ整数とみなす。
pub(crate) fn is_integer<T: NumberValue>() -> bool {
    <T as NumCast>::from(0.5).and_then(|value: T| value.to_f64()) != Some(0.5)
}

pub(crate) fn to_f64<T: NumberValue>(value: T) -> f64 {
    value.to_f64().unwrap_or(0.0)
}

/// `f64` を `T` へ。整数の型に小数は入れず、`T` に収まらなければ `None`。
pub(crate) fn from_f64<T: NumberValue>(value: f64) -> Option<T> {
    if !value.is_finite() || (is_integer::<T>() && value.fract() != 0.0) {
        return None;
    }

    <T as NumCast>::from(value)
}

/// 数を打ち込む欄。Web の `<input type="number">` と同じ仕組みです。
///
/// 中身は [`InputText`] で、右端に上下のスピンボタン（▲▼）を重ねます。
/// 値の型は `T`（[`NumberValue`]）で、`Number::<i32>` なら整数、`Number::<f64>` なら小数を扱います。
///
/// - 打てるのは数に使う字（`0-9 + - . e E`）だけです。整数の型では `. e E` も打てません。
///   全角の数字は半角に直します。
/// - `T` で表せない字（`u8` で `300`、整数の型で `1.5` など）は、数として読めない字とみなします。
/// - ▲▼ を押すか ↑↓ を押すと、[`Number::step`] ずつ増減します（押したままなら続けて）。
///   フォーカスがあるあいだは、欄の上でホイールを回しても増減します。
/// - 増減の仕方は HTML の `stepUp()` / `stepDown()` と同じです。
///   - 値が刻み（`min`、無ければ [`Number::value`] で決めた初めの値、それも無ければ 0 が基準）に
///     そろっていなければ、まず進む向きの一番近い刻みへそろえます。
///   - `min` より小さく・`max` より大きくなるなら、範囲の中で一番近い刻みに留めます。
///   - 空や数として読めない字のときは、0 から数えます。
/// - 打ち込んだ値は、範囲の外でも刻みからずれていても直しません（HTML と同じ）。
///   正しいかは [`Number::validity`] で分かります。
/// - 字が変わるたびに [`Number::on_input`] を、フォーカスを手放したとき・Enter を押したとき・
///   ▲▼ で増減したときに、前と値が変わっていれば [`Number::on_change`] を呼びます
///   （HTML の `input` と `change` の出来事）。
/// - IME は使いません。
///
/// ▲▼ とホイールでの増減は、窓の [`update`](crate::gui::window_frame::WindowFrame::update) で
/// 字を並べ直すときに反映します（書体が要るため）。
pub struct Number<T: NumberValue> {
    input: InputText,
    /// 登録の名前のもと。
    name: String,
    width: f32,
    height: f32,
    gap: Gap,
    quad_type: QuadType,
    number_theme: NumberTheme,
    min: Option<f64>,
    max: Option<f64>,
    /// 刻み。`None` なら「any」（打ち込みは刻みを問わず、▲▼ は 1 ずつ）。
    step: Option<f64>,
    /// 初めの値（HTML の `value` 属性）。`min` が無いときの刻みの基準になる。
    default_value: Option<f64>,
    spin_width: f32,
    on_input: Option<Box<dyn FnMut(Option<T>)>>,
    on_change: Option<Box<dyn FnMut(Option<T>)>>,
    /// 最後に `change` を知らせたときの字。
    committed: String,
    number_register_name: NumberRegisterName,
    /// 組んだときの左上（余白を足す前）。
    built_at: [f32; 2],
    /// 指が上にあるスピンボタン。
    hovered: Option<Spin>,
    /// 押しているスピンボタン。
    pressed: Option<Spin>,
    /// 押したままのとき、次に動かす時刻。
    repeat_at: Option<Instant>,
    /// まだ反映していない増減（▲ とホイールの上で +1、▼ と下で -1）。
    pending_steps: i32,
    _value: std::marker::PhantomData<T>,
}

/// 上下どちらのスピンボタンか。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Spin {
    Up,
    Down,
}

impl Spin {
    fn direction(self) -> i32 {
        match self {
            Spin::Up => 1,
            Spin::Down => -1,
        }
    }
}

#[derive(Default)]
struct NumberRegisterName {
    up: Option<String>,
    down: Option<String>,
    up_arrow: Option<String>,
    down_arrow: Option<String>,
}

pub struct NumberFont {
    pub base_font: FontHandle,
    pub base_font_size: f32,
}

pub struct NumberTheme {
    /// 打ち込む欄の色。
    pub input_text_theme: InputTextTheme,
    pub spin_fill_color: ThemeColor,
    /// 指がスピンボタンの上にあるとき。
    pub spin_hover_fill_color: ThemeColor,
    /// スピンボタンを押しているあいだ。
    pub spin_pressed_fill_color: ThemeColor,
    /// ▲▼ の色。
    pub arrow_color: ThemeColor,
}

/// 字が数として正しいか（HTML の `ValidityState` のうち、数の欄に関わるもの）。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NumberValidity {
    /// 正しい（空も正しい）。
    Valid,
    /// 数として読めない（`1e` や `--1` など）。
    BadInput,
    /// `min` より小さい。
    RangeUnderflow,
    /// `max` より大きい。
    RangeOverflow,
    /// 刻みにそろっていない。
    StepMismatch,
}

impl<T: NumberValue> Number<T> {
    pub fn new(register_name_string: String, placeholder: String, width: f32, height: f32, padding: f32, quad_type: QuadType, gap: Gap, number_font: NumberFont, number_theme: NumberTheme) -> Self {
        let input_text_font = InputTextFont { base_font: number_font.base_font, base_font_size: number_font.base_font_size };
        let spin_width = height * SPIN_WIDTH_RATIO;
        let input = InputText::new(register_name_string.clone(), placeholder, width, height, padding, quad_type, Gap { x: gap.x, y: gap.y }, input_text_font, number_theme.input_text_theme)
            .reserve_right(spin_width + spin_inset(quad_type));

        Self {
            input,
            name: register_name_string,
            width,
            height,
            gap,
            quad_type,
            number_theme,
            min: None,
            max: None,
            step: Some(DEFAULT_STEP),
            default_value: None,
            spin_width,
            on_input: None,
            on_change: None,
            committed: String::new(),
            number_register_name: NumberRegisterName::default(),
            built_at: [0.0; 2],
            hovered: None,
            pressed: None,
            repeat_at: None,
            pending_steps: 0,
            _value: std::marker::PhantomData,
        }
    }

    /// いちばん小さい値（HTML の `min`）。
    pub fn min(mut self, min: T) -> Self {
        self.min = Some(to_f64(min));
        self
    }

    /// いちばん大きい値（HTML の `max`）。
    pub fn max(mut self, max: T) -> Self {
        self.max = Some(to_f64(max));
        self
    }

    /// 刻み（HTML の `step`）。0 以下なら既定の 1 にする（HTML と同じ）。
    pub fn step(mut self, step: T) -> Self {
        let step = to_f64(step);
        self.step = Some(if step.is_finite() && step > 0.0 { step } else { DEFAULT_STEP });
        self
    }

    /// 刻みを問わない（HTML の `step="any"`）。▲▼ は 1 ずつ動く。
    pub fn step_any(mut self) -> Self {
        self.step = None;
        self
    }

    /// 初めの値（HTML の `value` 属性）。`min` が無いときは、これが刻みの基準になる。
    pub fn value(mut self, value: T) -> Self {
        let value = to_f64(value);
        self.default_value = Some(value);
        let text = format_number(value, decimal_places(value));
        self.input.replace_text(&text);
        self.committed = text;
        self
    }

    /// スピンボタンの幅。既定は欄の高さの 0.6 倍。
    pub fn spin_width(mut self, spin_width: f32) -> Self {
        self.spin_width = spin_width.max(0.0);
        self.input = self.input.reserve_right(self.spin_width + spin_inset(self.quad_type));
        self
    }

    /// 字が変わるたびに呼ぶ関数（HTML の `input`）。数として読めなければ `None`。
    pub fn on_input(mut self, on_input: impl FnMut(Option<T>) + 'static) -> Self {
        self.on_input = Some(Box::new(on_input));
        self
    }

    /// 値が決まったときに呼ぶ関数（HTML の `change`）。フォーカスを手放したとき・Enter・▲▼ で、
    /// 前に知らせたときから字が変わっていれば呼ぶ。数として読めなければ `None`。
    pub fn on_change(mut self, on_change: impl FnMut(Option<T>) + 'static) -> Self {
        self.on_change = Some(Box::new(on_change));
        self
    }

    /// 右クリックで開くメニューの名前。[`InputText::context_menu`] と同じ。
    pub fn context_menu(mut self, name: impl Into<String>) -> Self {
        self.input = self.input.context_menu(name);
        self
    }

    /// いまの字。
    pub fn text(&self) -> &str {
        self.input.text()
    }

    /// いまの値（HTML の `valueAsNumber`）。空か、数として読めないか、`T` で表せなければ `None`。
    pub fn get_value(&self) -> Option<T> {
        parse_number(self.input.text()).and_then(from_f64)
    }

    /// 字が数として正しいか。
    pub fn validity(&self) -> NumberValidity {
        let text = self.input.text();
        if !text.is_empty() && self.get_value().is_none() {
            return NumberValidity::BadInput;
        }

        validity(text, self.min, self.max, self.step, self.step_base())
    }

    /// 値を差し替える。`None` で空にする。知らせ（`on_input` / `on_change`）は呼ばない（HTML と同じ）。
    pub fn set_value(&mut self, draw_manager: &mut DrawManager, resources: &Resources, value: Option<T>) -> bool {
        let text = value.map(to_f64).filter(|value| value.is_finite()).map_or_else(String::new, |value| format_number(value, decimal_places(value)));
        self.committed = text.clone();

        self.input.replace_text(&text) && self.input.refresh(draw_manager, resources)
    }

    /// `count` 刻み増やす（HTML の `stepUp()`）。変わったら `true`。知らせも呼ぶ。
    pub fn step_up(&mut self, draw_manager: &mut DrawManager, resources: &Resources, count: u32) -> bool {
        self.apply_steps(draw_manager, resources, count as i32)
    }

    /// `count` 刻み減らす（HTML の `stepDown()`）。変わったら `true`。知らせも呼ぶ。
    pub fn step_down(&mut self, draw_manager: &mut DrawManager, resources: &Resources, count: u32) -> bool {
        self.apply_steps(draw_manager, resources, -(count as i32))
    }

    // --- 値 ---

    /// 刻みの基準。`min`、無ければ初めの値、それも無ければ 0（HTML と同じ）。
    fn step_base(&self) -> f64 {
        self.min.or(self.default_value).unwrap_or(0.0)
    }

    /// 増減した値を字にするときの小数の桁。増減した値は「基準 + 刻みの整数倍」なので、
    /// 刻みと基準の桁に合わせれば、足し算の誤差（0.30000000000000004 など）だけが落ちる。
    fn step_decimals(&self, step: f64) -> usize {
        decimal_places(step).max(decimal_places(self.step_base()))
    }

    /// `steps` 刻み動かして、字を差し替える。変わったら `true`。
    fn apply_steps(&mut self, draw_manager: &mut DrawManager, resources: &Resources, steps: i32) -> bool {
        if steps == 0 || self.input.composing() {
            return false;
        }

        let step = self.step.unwrap_or(DEFAULT_STEP);
        let current = self.get_value().map(to_f64);
        // 結果が `T` に収まらない（`u8` で 256 になるなど）なら動かさない。
        let Some(value) = step_value(current, steps, self.min, self.max, step, self.step_base()).filter(|value| from_f64::<T>(*value).is_some()) else {
            return false;
        };

        let text = format_number(value, self.step_decimals(step));
        if !self.input.replace_text(&text) {
            return false;
        }

        self.input.refresh(draw_manager, resources);
        self.notify_input();
        self.commit();
        true
    }

    /// 字が変わったことを知らせる（HTML の `input`）。
    fn notify_input(&mut self) {
        let value = self.get_value();
        if let Some(on_input) = self.on_input.as_mut() {
            on_input(value);
        }
    }

    /// 前に知らせたときから字が変わっていれば、値が決まったことを知らせる（HTML の `change`）。
    fn commit(&mut self) {
        if self.committed == self.input.text() {
            return;
        }

        self.committed = self.input.text().to_string();
        let value = self.get_value();
        if let Some(on_change) = self.on_change.as_mut() {
            on_change(value);
        }
    }

    /// 中の欄に何かさせ、字が変わっていたら `input` を知らせる。
    fn edit_input(&mut self, edit: impl FnOnce(&mut InputText) -> bool) -> bool {
        let before = self.input.text().to_string();
        let changed = edit(&mut self.input);

        if self.input.text() != before {
            self.notify_input();
        }
        changed
    }

    // --- スピンボタン ---

    /// スピンボタン全体の左上と右下（組んだときの座標）。欄の縁取りの内側。
    fn spin_bounds(&self) -> [f32; 4] {
        let [x, y] = self.built_at;
        let (left, top) = (x + self.gap.x, y + self.gap.y);
        let (right, bottom) = (left + self.width, top + self.height);
        let inset = spin_inset(self.quad_type);

        [(right - inset - self.spin_width).max(left), top + inset, right - inset, bottom - inset]
    }

    /// 上か下のスピンボタンの左上と右下。
    fn half_bounds(&self, spin: Spin) -> [f32; 4] {
        let [left, top, right, bottom] = self.spin_bounds();
        let middle = (top + bottom) / 2.0;

        match spin {
            Spin::Up => [left, top, right, middle],
            Spin::Down => [left, middle, right, bottom],
        }
    }

    fn vertex(x: f32, y: f32, color: &ThemeColor) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a)
    }

    /// いまの見た目に合うスピンボタンの塗り。
    fn spin_color(&self, spin: Spin) -> &ThemeColor {
        let theme = &self.number_theme;

        if self.pressed == Some(spin) && self.hovered == Some(spin) {
            &theme.spin_pressed_fill_color
        } else if self.hovered == Some(spin) {
            &theme.spin_hover_fill_color
        } else {
            &theme.spin_fill_color
        }
    }

    /// スピンボタンの地。組み直しで頂点だけが欲しいときは、名前に `""` を渡します。
    fn build_spin(&self, name: &str, spin: Spin) -> Object {
        let [left, top, right, bottom] = self.half_bounds(spin);
        let color = self.spin_color(spin);

        Rect::new(name)
            .paint_type(PaintType::Fill)
            .from(Self::vertex(left, top, color))
            .to(Self::vertex(right, bottom, color))
            .end()
    }

    /// ▲ か ▼。
    fn build_arrow(&self, name: &str, spin: Spin) -> Object {
        let [left, top, right, bottom] = self.half_bounds(spin);
        let (center_x, center_y) = ((left + right) / 2.0, (top + bottom) / 2.0);
        // 横の半分の長さ。縦はその半分にして、平たい三角にする。
        let size = (right - left).min(bottom - top) * 0.3;
        let tip = match spin {
            Spin::Up => -size * 0.5,
            Spin::Down => size * 0.5,
        };
        let color = &self.number_theme.arrow_color;

        Triangle::new(name)
            .paint_type(PaintType::Fill)
            .vertex1(Self::vertex(center_x - size, center_y - tip, color))
            .vertex2(Self::vertex(center_x + size, center_y - tip, color))
            .vertex3(Self::vertex(center_x, center_y + tip, color))
            .end()
    }

    /// スピンボタンを、いまの場所と色で組み直す。
    fn refresh_spins(&self, draw_manager: &mut DrawManager) {
        let names = &self.number_register_name;
        let shapes = [
            (&names.up, self.build_spin("", Spin::Up)),
            (&names.down, self.build_spin("", Spin::Down)),
            (&names.up_arrow, self.build_arrow("", Spin::Up)),
            (&names.down_arrow, self.build_arrow("", Spin::Down)),
        ];

        for (name, built) in shapes {
            if let Some(object) = name.as_deref().and_then(|name| draw_manager.object_mut(name)) {
                replace_outline(object, &built);
            }
        }
    }

    /// 指の下で一番手前にあるスピンボタン。
    ///
    /// スピンボタンは欄の枠の内側に置くので、枠の外なら `pick` せずに断る（`pick` は全図形をなめるので重い）。
    fn spin_at(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<Spin> {
        if !self.input.inside(draw_manager, pointer) {
            return None;
        }

        let pick = pointer.pick(draw_manager)?;
        let names = &self.number_register_name;
        let is = |name: &Option<String>| name.as_deref() == Some(pick.name.as_str());

        if is(&names.up) || is(&names.up_arrow) {
            Some(Spin::Up)
        } else if is(&names.down) || is(&names.down_arrow) {
            Some(Spin::Down)
        } else {
            None
        }
    }

    /// 指が上にある・押しているスピンボタンを変えて、見た目が変わったら塗り直す。変わったら `true`。
    fn set_spin_state(&mut self, draw_manager: &mut DrawManager, hovered: Option<Spin>, pressed: Option<Spin>) -> bool {
        if (self.hovered, self.pressed) == (hovered, pressed) {
            return false;
        }

        self.hovered = hovered;
        self.pressed = pressed;
        self.refresh_spins(draw_manager);
        true
    }
}

impl<T: NumberValue> WindowItem for Number<T> {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let WindowItemArguments1 { draw_manager, resources, register_name, counter, item_x, item_y, width, surface_size, scale_mode, design } = window_item_arguments;

        // 欄を先に組み、スピンボタンはその上に重ねる。
        let size = self.input.create_object(WindowItemArguments1 {
            draw_manager: &mut *draw_manager,
            resources,
            register_name,
            counter,
            item_x,
            item_y,
            width,
            surface_size,
            scale_mode,
            design,
        });

        self.built_at = [item_x, item_y];

        let name = format!("{register_name}-{}-{counter}", self.name);
        let mut register = |mut object: Object| {
            object.camera_for(surface_size, design, scale_mode);
            object.instance(create_instance());
            // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
            Some(draw_manager.register(object))
        };

        let up = register(self.build_spin(&format!("{name}-spin-up"), Spin::Up));
        let down = register(self.build_spin(&format!("{name}-spin-down"), Spin::Down));
        let up_arrow = register(self.build_arrow(&format!("{name}-spin-up-arrow"), Spin::Up));
        let down_arrow = register(self.build_arrow(&format!("{name}-spin-down-arrow"), Spin::Down));

        self.number_register_name = NumberRegisterName { up, down, up_arrow, down_arrow };

        size
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        let WindowItemArguments2 { draw_manager, resources, register_name, counter, item_x, item_y, width, scale_mode } = window_item_arguments;

        let size = self.input.rebuild(WindowItemArguments2 {
            draw_manager: &mut *draw_manager,
            resources,
            register_name,
            counter,
            item_x,
            item_y,
            width,
            scale_mode,
        });

        self.built_at = [item_x, item_y];
        self.refresh_spins(draw_manager);

        size
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let names = &self.number_register_name;
        let mut owned = self.input.object_names(draw_manager);
        owned.extend([&names.up, &names.down, &names.up_arrow, &names.down_arrow].into_iter().flatten().cloned());

        owned
    }

    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        self.input.resize(draw_manager, surface_size, design);
    }

    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        let Some(spin) = self.spin_at(draw_manager, pointer) else {
            return self.input.mouse_left_pressed(draw_manager, pointer);
        };

        // ブラウザと同じく、スピンボタンを押しても欄にフォーカスが移る。
        self.input.focus(draw_manager);
        self.set_spin_state(draw_manager, Some(spin), Some(spin));

        // 字を並べ直すには書体が要るので、増減は animate でする。
        self.pending_steps += spin.direction();
        self.repeat_at = Some(Instant::now() + REPEAT_DELAY);
        true
    }

    fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        let released_spin = self.pressed.is_some();
        if released_spin {
            let hovered = self.spin_at(draw_manager, pointer);
            self.set_spin_state(draw_manager, hovered, None);
            self.repeat_at = None;
        }

        self.input.mouse_left_released(draw_manager, pointer) || released_spin
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        let hovered = self.spin_at(draw_manager, pointer);
        let pressed = self.pressed;
        let spin_changed = self.set_spin_state(draw_manager, hovered, pressed);

        self.input.mouse_moved(draw_manager, pointer) || spin_changed
    }

    fn mouse_wheel(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer, lines: [f32; 2]) -> bool {
        // ブラウザと同じく、フォーカスがあって欄の上にいるときだけ。
        let over = self.input.hit(draw_manager, pointer) || self.spin_at(draw_manager, pointer).is_some();
        if !self.input.has_focus() || !over || lines[1] == 0.0 {
            return false;
        }

        self.pending_steps += if lines[1] > 0.0 { 1 } else { -1 };
        true
    }

    fn has_focus(&self) -> bool {
        self.input.has_focus()
    }

    fn unfocus(&mut self, draw_manager: &mut DrawManager) -> bool {
        if !self.input.has_focus() {
            return false;
        }

        self.repeat_at = None;
        let pressed_changed = self.set_spin_state(draw_manager, self.hovered, None);
        let changed = self.input.unfocus(draw_manager);
        self.commit();

        changed || pressed_changed
    }

    fn text_input(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        let filtered = filter_number_chars(text, !is_integer::<T>());
        if filtered.is_empty() {
            return false;
        }

        self.edit_input(|input| input.text_input(draw_manager, resources, &filtered))
    }

    fn paste(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        let filtered = filter_number_chars(text, !is_integer::<T>());
        if filtered.is_empty() {
            return false;
        }

        self.edit_input(|input| input.paste(draw_manager, resources, &filtered))
    }

    fn key_pressed(&mut self, draw_manager: &mut DrawManager, resources: &Resources, key: EditKey) -> bool {
        match key {
            EditKey::Up => self.apply_steps(draw_manager, resources, 1),
            EditKey::Down => self.apply_steps(draw_manager, resources, -1),
            EditKey::Enter => {
                self.commit();
                false
            }
            _ => self.edit_input(|input| input.key_pressed(draw_manager, resources, key)),
        }
    }

    fn modifiers_changed(&mut self, modifiers: KeyModifiers) {
        self.input.modifiers_changed(modifiers);
    }

    fn selected_text(&self) -> Option<String> {
        self.input.selected_text()
    }

    fn offset_changed(&mut self, draw_manager: &mut DrawManager, offset: [f32; 2]) {
        self.input.offset_changed(draw_manager, offset);
    }

    fn caret_area(&self) -> Option<[f32; 4]> {
        self.input.caret_area()
    }

    fn wants_ime(&self) -> bool {
        // 数の欄では変換しない（ブラウザと同じ）。
        false
    }

    fn context_menu(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> Option<ContextMenuRequest> {
        WindowItem::context_menu(&mut self.input, draw_manager, pointer)
    }

    fn animate(&mut self, draw_manager: &mut DrawManager, resources: &Resources, now: Instant) -> Animation {
        let mut animation = self.input.animate(draw_manager, resources, now);

        // 押したまま、指がそのボタンの上にあるあいだは続けて動かす。
        if let (Some(spin), Some(repeat_at)) = (self.pressed, self.repeat_at) {
            if self.hovered == Some(spin) && now >= repeat_at {
                self.pending_steps += spin.direction();
                self.repeat_at = Some(now + REPEAT_INTERVAL);
            }
        }

        let steps = std::mem::take(&mut self.pending_steps);
        if self.apply_steps(draw_manager, resources, steps) {
            animation.relaid_out = true;
        }

        if let Some(repeat_at) = self.repeat_at.filter(|_| self.pressed.is_some()) {
            animation.next = Some(animation.next.map_or(repeat_at, |next| next.min(repeat_at)));
        }

        animation
    }

    fn cursor_shape(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<CursorShape> {
        if self.spin_at(draw_manager, pointer).is_some() {
            return Some(CursorShape::Default);
        }

        self.input.cursor_shape(draw_manager, pointer)
    }
}

/// スピンボタンを欄の縁取りの内側に置くための、端からの寄せ。角が丸ければ、角に掛からないぶん寄せる。
fn spin_inset(quad_type: QuadType) -> f32 {
    match quad_type {
        QuadType::Normal => OUTLINE_WIDTH,
        QuadType::Rounded { radius } => OUTLINE_WIDTH + radius * (1.0 - std::f32::consts::FRAC_1_SQRT_2),
    }
}

/// 打たれた字から、数に使う字だけを残す。全角の数字と記号は半角に直す。
/// `fraction` が `false`（整数の型）なら、小数点と指数（`. e E`）も落とす。
fn filter_number_chars(text: &str, fraction: bool) -> String {
    text.chars()
        .map(|character| match character {
            '０'..='９' => char::from_u32(character as u32 - '０' as u32 + '0' as u32).unwrap_or(character),
            '＋' => '+',
            '－' | '−' => '-',
            '．' => '.',
            'ｅ' => 'e',
            'Ｅ' => 'E',
            _ => character,
        })
        .filter(|character| match character {
            '0'..='9' | '+' | '-' => true,
            '.' | 'e' | 'E' => fraction,
            _ => false,
        })
        .collect()
}

/// HTML の「正しい浮動小数点数」として読む。`-?(\d+|\d*\.\d+)([eE][+-]?\d+)?` の形だけを受け付ける。
///
/// Rust の `parse` と違い、先頭の `+`・末尾の `.`・`inf`・`NaN` は読めない（HTML と同じ）。
pub(crate) fn parse_number(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let digits = |index: &mut usize| {
        let start = *index;
        while bytes.get(*index).is_some_and(u8::is_ascii_digit) {
            *index += 1;
        }
        *index - start
    };

    if bytes.first() == Some(&b'-') {
        index += 1;
    }

    let integer = digits(&mut index);
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        if digits(&mut index) == 0 {
            return None;
        }
    } else if integer == 0 {
        return None;
    }

    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        if digits(&mut index) == 0 {
            return None;
        }
    }

    if index != bytes.len() {
        return None;
    }

    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// `value - base` が `step` の整数倍か（誤差は見逃す）。
fn is_aligned(value: f64, base: f64, step: f64) -> bool {
    let multiple = (value - base) / step;
    (multiple - multiple.round()).abs() <= 1e-9 * multiple.abs().max(1.0)
}

/// HTML の `stepUp(n)`（`steps` が正）・`stepDown(n)`（負）の結果。動かないなら `None`。
///
/// 1. 読めない値は 0 とみなす。
/// 2. 刻みにそろっていなければ、進む向きの一番近い刻みへそろえる（そろえるだけで、さらには進まない）。
///    そろっていれば `steps` 刻み進む。
/// 3. `min` より小さければ `min` 以上で一番小さい刻みへ、`max` より大きければ `max` 以下で一番大きい刻みへ。
/// 4. 進む向きと逆に動いてしまう・同じ値のままなら、動かさない。
pub(crate) fn step_value(value: Option<f64>, steps: i32, min: Option<f64>, max: Option<f64>, step: f64, base: f64) -> Option<f64> {
    if steps == 0 || !step.is_finite() || step <= 0.0 {
        return None;
    }

    let aligned_ceil = |value: f64| base + ((value - base) / step - 1e-9).ceil() * step;
    let aligned_floor = |value: f64| base + ((value - base) / step + 1e-9).floor() * step;

    // 範囲の中に刻みが 1 つも無ければ、動かせない。
    if let (Some(min), Some(max)) = (min, max) {
        if min > max || aligned_ceil(min) > max + step * 1e-9 {
            return None;
        }
    }

    let before = value.unwrap_or(0.0);
    let mut next = if is_aligned(before, base, step) {
        let multiple = ((before - base) / step).round();
        base + (multiple + steps as f64) * step
    } else if steps > 0 {
        aligned_ceil(before)
    } else {
        aligned_floor(before)
    };

    if let Some(min) = min.filter(|min| next < *min) {
        next = aligned_ceil(min);
    }
    if let Some(max) = max.filter(|max| next > *max) {
        next = aligned_floor(max);
    }

    // 空からなら、0 に留まっても字が入るので動いたとみなす。
    let wrong_way = (steps > 0 && next < before) || (steps < 0 && next > before);
    let unchanged = value.is_some() && next == before;
    (!wrong_way && !unchanged && next.is_finite()).then_some(next)
}

/// 字が数として正しいか。
pub(crate) fn validity(text: &str, min: Option<f64>, max: Option<f64>, step: Option<f64>, base: f64) -> NumberValidity {
    if text.is_empty() {
        return NumberValidity::Valid;
    }
    let Some(value) = parse_number(text) else {
        return NumberValidity::BadInput;
    };

    if min.is_some_and(|min| value < min) {
        NumberValidity::RangeUnderflow
    } else if max.is_some_and(|max| value > max) {
        NumberValidity::RangeOverflow
    } else if step.is_some_and(|step| !is_aligned(value, base, step)) {
        NumberValidity::StepMismatch
    } else {
        NumberValidity::Valid
    }
}

/// 数の、小数点より下の桁の数（いちばん短く書いたとき）。
pub(crate) fn decimal_places(value: f64) -> usize {
    let text = value.abs().to_string();
    text.split_once('.').map_or(0, |(_, fraction)| fraction.len())
}

/// 数を `decimals` 桁で丸めて字にする。末尾の 0 と `-0` は落とす。
pub(crate) fn format_number(value: f64, decimals: usize) -> String {
    let mut text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.truncate(text.trim_end_matches('0').trim_end_matches('.').len());
    }
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_html_floating_point_numbers() {
        for (text, expected) in [("1", Some(1.0)), ("-1.5", Some(-1.5)), (".5", Some(0.5)), ("1e3", Some(1000.0)), ("2E-2", Some(0.02)), ("0010", Some(10.0))] {
            assert_eq!(parse_number(text), expected, "{text:?}");
        }
        for text in ["", "+1", "1.", "-", ".", "1e", "1e+", "--1", "1-", "inf", "NaN", "1 ", "1,5", "1e400"] {
            assert_eq!(parse_number(text), None, "{text:?}");
        }
    }

    #[test]
    fn steps_by_the_step_and_rounds_away_float_error() {
        let next = step_value(Some(0.2), 1, None, None, 0.1, 0.0).unwrap();
        assert_eq!(format_number(next, 1), "0.3");

        assert_eq!(step_value(Some(5.0), 2, None, None, 1.0, 0.0), Some(7.0));
        assert_eq!(step_value(Some(5.0), -1, None, None, 1.0, 0.0), Some(4.0));
    }

    #[test]
    fn empty_counts_from_zero() {
        assert_eq!(step_value(None, 1, None, None, 1.0, 0.0), Some(1.0));
        assert_eq!(step_value(None, -1, None, None, 1.0, 0.0), Some(-1.0));
        assert_eq!(step_value(None, 1, Some(5.0), None, 1.0, 5.0), Some(5.0), "min より下なら min へ");
    }

    #[test]
    fn misaligned_values_snap_toward_the_direction_without_stepping_further() {
        // 基準 0、刻み 5 で 7 は外れている。
        assert_eq!(step_value(Some(7.0), 1, None, None, 5.0, 0.0), Some(10.0));
        assert_eq!(step_value(Some(7.0), -1, None, None, 5.0, 0.0), Some(5.0));
        // 基準は min。min = 1、刻み 2 なら 1, 3, 5...
        assert_eq!(step_value(Some(4.0), 1, Some(1.0), None, 2.0, 1.0), Some(5.0));
    }

    #[test]
    fn clamps_into_the_range_on_a_step() {
        assert_eq!(step_value(Some(10.0), 1, None, Some(10.0), 1.0, 0.0), None, "max で止まる");
        assert_eq!(step_value(Some(9.0), 5, None, Some(10.0), 1.0, 0.0), Some(10.0));
        // max が刻みに乗っていなければ、max 以下で一番大きい刻みへ。
        assert_eq!(step_value(Some(0.0), 10, None, Some(10.0), 3.0, 0.0), Some(9.0));
        assert_eq!(step_value(Some(0.0), -1, Some(0.0), None, 1.0, 0.0), None, "min で止まる");
    }

    #[test]
    fn never_moves_the_wrong_way() {
        // 範囲の外（max より大）から ▲ を押しても、下がりはしない。
        assert_eq!(step_value(Some(50.0), 1, None, Some(10.0), 1.0, 0.0), None);
        // ▼ なら範囲の中へ戻る。
        assert_eq!(step_value(Some(50.0), -1, None, Some(10.0), 1.0, 0.0), Some(10.0));
    }

    #[test]
    fn no_step_fits_the_range() {
        assert_eq!(step_value(Some(0.0), 1, Some(5.0), Some(1.0), 1.0, 5.0), None, "min > max");
        assert_eq!(step_value(Some(1.0), 1, Some(1.0), Some(2.0), 5.0, 0.0), None, "範囲に刻みが無い");
    }

    #[test]
    fn reports_validity_like_html() {
        let check = |text| validity(text, Some(0.0), Some(10.0), Some(2.0), 0.0);

        assert_eq!(check(""), NumberValidity::Valid);
        assert_eq!(check("4"), NumberValidity::Valid);
        assert_eq!(check("1e"), NumberValidity::BadInput);
        assert_eq!(check("-2"), NumberValidity::RangeUnderflow);
        assert_eq!(check("12"), NumberValidity::RangeOverflow);
        assert_eq!(check("3"), NumberValidity::StepMismatch);
        assert_eq!(validity("3.3", None, None, None, 0.0), NumberValidity::Valid, "step any");
    }

    #[test]
    fn filters_typed_characters() {
        assert_eq!(filter_number_chars("a1b-2.c3e+4", true), "1-2.3e+4");
        assert_eq!(filter_number_chars("１２．５", true), "12.5");
        assert_eq!(filter_number_chars("abc", true), "");
        assert_eq!(filter_number_chars("-1.5e3", false), "-153", "整数の型では . e を落とす");
    }

    #[test]
    fn converts_into_the_value_type() {
        assert!(is_integer::<i32>() && is_integer::<u8>());
        assert!(!is_integer::<f32>() && !is_integer::<f64>());

        assert_eq!(from_f64::<i32>(3.0), Some(3));
        assert_eq!(from_f64::<i32>(1.5), None, "整数の型に小数は入らない");
        assert_eq!(from_f64::<u8>(256.0), None, "収まらない");
        assert_eq!(from_f64::<u8>(-1.0), None);
        assert_eq!(from_f64::<f32>(0.25), Some(0.25));
    }

    #[test]
    fn formats_without_trailing_zeros() {
        assert_eq!(format_number(1.5, 3), "1.5");
        assert_eq!(format_number(2.0, 2), "2");
        assert_eq!(format_number(-0.0001, 2), "0");
        assert_eq!(format_number(0.30000000000000004, 1), "0.3");
        assert_eq!(decimal_places(0.25), 2);
        assert_eq!(decimal_places(3.0), 0);
    }
}
