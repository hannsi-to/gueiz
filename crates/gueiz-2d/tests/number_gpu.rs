//! 窓に置いた [`Number`] に、スピンボタン・↑↓・ホイール・打ち込みを流して確かめる。
//!
//! 当たり判定（[`DrawManager::pick`]）を通すので GPU が要ります。
//! GPU か書体が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test number_gpu -- --nocapture
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::Duration;

use gueiz_2d::camera::ScaleMode;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::input_text::InputTextTheme;
use gueiz_2d::gui::number::{Number, NumberFont, NumberTheme, NumberValue};
use gueiz_2d::gui::window_frame::{Gap, Quad, QuadType, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::gui::window_item::{CursorShape, EditKey};
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::resource::{FontHandle, Resources};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// 欄は (105, 140) から 200 x 28、角の丸み 4。スピンボタンは右端の内側で、幅 16.8。
const UP: (f32, f32) = (294.0, 148.0);
const DOWN: (f32, f32) = (294.0, 160.0);
/// 欄の、字を打つところ。
const FIELD: (f32, f32) = (150.0, 154.0);

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

    GPU.get_or_init(|| {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("number gpu test"),
            required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
            ..Default::default()
        }))
        .ok()?;

        Some(Gpu { device, queue })
    })
    .as_ref()
}

fn color(value: f32) -> ThemeColor {
    ThemeColor { r: value, g: value, b: value, a: 1.0 }
}

fn load_font(resources: &mut Resources) -> Option<FontHandle> {
    [
        "C:/Windows/Fonts/YuGothM.ttc",
        "C:/Windows/Fonts/arial.ttf",
        "/System/Library/Fonts/Avenir Next.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .into_iter()
    .find_map(|path| resources.load_font_file("font", path).ok())
}

/// 窓 1 つに数の欄を 1 つ置いたもの。
struct Scene<T: NumberValue> {
    draw_manager: DrawManager,
    resources: Resources,
    window_frame: WindowFrame,
    /// `on_input` で届いた値。
    inputs: Rc<RefCell<Vec<Option<T>>>>,
    /// `on_change` で届いた値。
    changes: Rc<RefCell<Vec<Option<T>>>>,
}

/// 欄に手を加えて組む（min・max・step を決めるなど）。
fn scene<T: NumberValue>(customize: impl FnOnce(Number<T>) -> Number<T>) -> Option<Scene<T>> {
    let Some(gpu) = gpu() else {
        eprintln!("skipped: no gpu adapter");
        return None;
    };

    let mut resources = Resources::new();
    let Some(font) = load_font(&mut resources) else {
        eprintln!("skipped: no font");
        return None;
    };

    let draw_manager = DrawManager::new(&gpu.device, &gpu.queue, TextureFormat::Bgra8UnormSrgb, &DrawManagerDescriptor::default())
        .expect("DrawManager を作れなかった");

    // 窓は (100, 100) から。帯 30・余白 5 なので、部品は (105, 135) から積まれる。
    let mut window_frame = WindowFrame::new(
        "Window".to_string(),
        ScaleMode::Stretch,
        "Window".to_string(),
        Quad { quad_type: QuadType::Normal, x: 100.0, y: 100.0, width: 400.0, height: 200.0 },
        30.0,
        Gap { x: 5.0, y: 5.0 },
        WindowFont { base_font: font, base_font_size: 13.0 },
        WindowTheme {
            frame_fill_color: color(0.1),
            frame_outline_color: color(0.5),
            title_bar_fill_color: color(0.1),
            title_bar_slice_line_color: color(0.2),
            text_title_color: color(1.0),
        },
    );

    let inputs = Rc::new(RefCell::new(Vec::new()));
    let changes = Rc::new(RefCell::new(Vec::new()));
    let (on_input, on_change) = (inputs.clone(), changes.clone());

    window_frame.add_window_item(Box::new(customize(
        Number::new(
            "Number".to_string(),
            "number".to_string(),
            200.0,
            28.0,
            8.0,
            QuadType::Rounded { radius: 4.0 },
            Gap { x: 0.0, y: 5.0 },
            NumberFont { base_font: font, base_font_size: 14.0 },
            NumberTheme {
                input_text_theme: InputTextTheme {
                    fill_color: color(0.05),
                    outline_color: color(0.3),
                    focus_outline_color: color(0.6),
                    text_color: color(1.0),
                    placeholder_color: color(0.5),
                    caret_color: color(1.0),
                    selection_color: color(0.3),
                },
                spin_fill_color: color(0.15),
                spin_hover_fill_color: color(0.25),
                spin_pressed_fill_color: color(0.35),
                arrow_color: color(0.9),
            },
        )
        .on_input(move |value| on_input.borrow_mut().push(value))
        .on_change(move |value| on_change.borrow_mut().push(value)),
    )));

    let mut scene = Scene { draw_manager, resources, window_frame, inputs, changes };
    let surface_size = SurfaceSize::new(WIDTH, HEIGHT);
    scene.window_frame.create_object(&mut scene.draw_manager, &scene.resources, surface_size, [WIDTH as f32, HEIGHT as f32]);

    Some(scene)
}

impl<T: NumberValue> Scene<T> {
    fn press(&mut self, (x, y): (f32, f32)) -> bool {
        self.window_frame.mouse_left_pressed(&mut self.draw_manager, x, y)
    }

    fn release(&mut self, (x, y): (f32, f32)) {
        self.window_frame.mouse_left_released(&mut self.draw_manager, x, y);
    }

    /// 押して放し、描く前の 1 回ぶんを進める（スピンボタンの増減はここで入る）。
    fn click(&mut self, at: (f32, f32)) -> bool {
        let taken = self.press(at);
        self.release(at);
        self.update();
        taken
    }

    fn type_text(&mut self, text: &str) -> bool {
        self.window_frame.text_input(&mut self.draw_manager, &self.resources, text)
    }

    fn key(&mut self, key: EditKey) -> bool {
        self.window_frame.key_pressed(&mut self.draw_manager, &self.resources, key)
    }

    fn update(&mut self) {
        self.window_frame.update(&mut self.draw_manager, &self.resources);
    }

    /// いまの字。最後に届いた `on_input` の値を字にしたものではなく、選んでコピーできる字。
    fn text(&mut self) -> String {
        self.key(EditKey::SelectAll);
        self.window_frame.selected_text().unwrap_or_default()
    }

    fn last_input(&self) -> Option<Option<T>> {
        self.inputs.borrow().last().copied()
    }

    fn last_change(&self) -> Option<Option<T>> {
        self.changes.borrow().last().copied()
    }
}

#[test]
fn spin_buttons_step_up_and_down_and_focus_the_field() {
    let Some(mut scene) = scene::<i32>(|number| number) else { return };

    assert!(scene.click(UP), "スピンボタンを押したら窓が取る");
    assert!(scene.window_frame.has_focus(), "スピンボタンでも欄にフォーカスが移る");
    assert_eq!(scene.last_input(), Some(Some(1)), "空からは 0 + 1");
    assert_eq!(scene.last_change(), Some(Some(1)), "▲▼ では change も届く");

    scene.click(DOWN);
    scene.click(DOWN);
    assert_eq!(scene.last_change(), Some(Some(-1)));
    assert_eq!(scene.text(), "-1");
}

#[test]
fn arrow_keys_step_and_stop_at_the_range() {
    let Some(mut scene) = scene::<i32>(|number| number.min(0).max(10).step(3).value(9)) else { return };
    scene.click(FIELD);

    assert!(!scene.key(EditKey::Up), "max 10 以下の刻みは 9 なので動かない");
    assert_eq!(scene.text(), "9");

    scene.key(EditKey::End);
    assert!(scene.key(EditKey::Down));
    assert_eq!(scene.last_change(), Some(Some(6)));

    for _ in 0..5 {
        scene.key(EditKey::Down);
    }
    assert_eq!(scene.text(), "0", "min で止まる");
}

#[test]
fn decimal_steps_do_not_show_float_error() {
    let Some(mut scene) = scene::<f64>(|number| number.step(0.1).value(0.2)) else { return };
    scene.click(FIELD);

    scene.key(EditKey::Up);
    assert_eq!(scene.text(), "0.3");
    assert_eq!(scene.last_change(), Some(Some(0.3)));
}

#[test]
fn typing_keeps_only_number_characters_and_change_waits_for_commit() {
    let Some(mut scene) = scene::<i32>(|number| number.max(100)) else { return };
    scene.click(FIELD);

    assert!(scene.type_text("1.5a"));
    assert_eq!(scene.last_input(), Some(Some(15)), "整数の欄では . も落とす");
    assert!(!scene.type_text("abc"), "数に使わない字だけなら何も起きない");
    assert_eq!(scene.last_change(), None, "打っているあいだは change は来ない");

    scene.type_text("0");
    assert_eq!(scene.text(), "150");

    // 範囲の外でも直さない。よそを押すと change が届く。
    scene.click((20.0, 20.0));
    assert_eq!(scene.last_change(), Some(Some(150)));
}

#[test]
fn enter_commits_and_bad_input_reads_as_none() {
    let Some(mut scene) = scene::<f64>(|number| number) else { return };
    scene.click(FIELD);

    scene.type_text("1e");
    assert_eq!(scene.last_input(), Some(None), "読めない字は None");
    scene.key(EditKey::Enter);
    assert_eq!(scene.last_change(), Some(None));

    // 読めない字から ▲ を押すと 0 から数える。
    scene.click(UP);
    assert_eq!(scene.text(), "1");
}

#[test]
fn holding_a_spin_button_repeats() {
    let Some(mut scene) = scene::<i32>(|number| number) else { return };

    scene.press(UP);
    scene.update();
    assert_eq!(scene.last_input(), Some(Some(1)));

    sleep(Duration::from_millis(450));
    scene.update();
    sleep(Duration::from_millis(60));
    scene.update();
    scene.release(UP);
    scene.update();

    let count = scene.inputs.borrow().len();
    assert!(count >= 3, "押したままなら続けて増える: {count}");

    // 放した後は増えない。
    sleep(Duration::from_millis(60));
    scene.update();
    assert_eq!(scene.inputs.borrow().len(), count);
}

#[test]
fn wheel_steps_only_while_focused() {
    let Some(mut scene) = scene::<i32>(|number| number.value(5)) else { return };
    let (x, y) = FIELD;

    assert!(!scene.window_frame.mouse_wheel(&mut scene.draw_manager, x, y, [0.0, 1.0]), "フォーカスが無ければ取らない");

    scene.click(FIELD);
    assert!(scene.window_frame.mouse_wheel(&mut scene.draw_manager, x, y, [0.0, 1.0]));
    scene.update();
    assert_eq!(scene.last_change(), Some(Some(6)));

    scene.window_frame.mouse_wheel(&mut scene.draw_manager, x, y, [0.0, -2.0]);
    scene.update();
    assert_eq!(scene.last_change(), Some(Some(5)));
}

#[test]
fn values_outside_the_type_are_bad_input() {
    let Some(mut scene) = scene::<u8>(|number| number.value(255)) else { return };
    scene.click(FIELD);

    assert!(!scene.key(EditKey::Up), "u8 に 256 は入らないので動かない");

    scene.key(EditKey::SelectAll);
    scene.type_text("300");
    assert_eq!(scene.last_input(), Some(None));
}

#[test]
fn spin_buttons_show_the_default_cursor_and_hover_highlights() {
    let Some(mut scene) = scene::<i32>(|number| number) else { return };
    let (x, y) = UP;

    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, x, y), CursorShape::Default);
    let (x, y) = FIELD;
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, x, y), CursorShape::Text);

    let (x, y) = UP;
    assert!(scene.window_frame.mouse_moved(&mut scene.draw_manager, x, y), "指が乗ると塗り替える");
}
