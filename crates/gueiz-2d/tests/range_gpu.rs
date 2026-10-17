//! 窓に置いた [`Range`] に、押す・引く・キーを流して確かめる。
//!
//! 当たり判定（[`DrawManager::pick`]）を通すので GPU が要ります。
//! GPU か書体が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test range_gpu -- --nocapture
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use gueiz_2d::camera::ScaleMode;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::number::NumberValue;
use gueiz_2d::gui::range::{Range, RangeTheme};
use gueiz_2d::gui::window_frame::{Gap, Quad, QuadType, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::gui::window_item::{CursorShape, EditKey};
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::resource::{FontHandle, Resources};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// 部品は (105, 140) から 200 x 20。つまみの半径は 8 なので、まん中は x 113 から 297 を動く。
const START: f32 = 113.0;
const TRAVEL: f32 = 184.0;
const Y: f32 = 150.0;

/// 値の割合 `ratio` の、つまみのまん中の x。
fn x_at(ratio: f32) -> f32 {
    START + TRAVEL * ratio
}

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
            label: Some("range gpu test"),
            required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
            ..Default::default()
        }))
        .ok()?;

        Some(Gpu { device, queue })
    })
    .as_ref()
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

fn color(value: f32) -> ThemeColor {
    ThemeColor { r: value, g: value, b: value, a: 1.0 }
}

struct Scene<T: NumberValue> {
    draw_manager: DrawManager,
    window_frame: WindowFrame,
    inputs: Rc<RefCell<Vec<T>>>,
    changes: Rc<RefCell<Vec<T>>>,
}

fn scene<T: NumberValue>(customize: impl FnOnce(Range<T>) -> Range<T>) -> Option<Scene<T>> {
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
        Range::new(
            "Range".to_string(),
            200.0,
            20.0,
            Gap { x: 0.0, y: 5.0 },
            RangeTheme {
                track_color: color(0.3),
                fill_color: color(0.5),
                thumb_color: color(0.8),
                thumb_hover_color: color(0.9),
                thumb_pressed_color: color(1.0),
                focus_ring_color: color(0.6),
            },
        )
        .on_input(move |value| on_input.borrow_mut().push(value))
        .on_change(move |value| on_change.borrow_mut().push(value)),
    )));

    let mut scene = Scene { draw_manager, window_frame, inputs, changes };
    let surface_size = SurfaceSize::new(WIDTH, HEIGHT);
    scene.window_frame.create_object(&mut scene.draw_manager, &resources, surface_size, [WIDTH as f32, HEIGHT as f32]);

    Some(scene)
}

impl<T: NumberValue> Scene<T> {
    fn press(&mut self, x: f32, y: f32) -> bool {
        self.window_frame.mouse_left_pressed(&mut self.draw_manager, x, y)
    }

    fn moved(&mut self, x: f32, y: f32) -> bool {
        self.window_frame.mouse_moved(&mut self.draw_manager, x, y)
    }

    fn release(&mut self, x: f32, y: f32) {
        self.window_frame.mouse_left_released(&mut self.draw_manager, x, y);
    }

    fn click(&mut self, x: f32, y: f32) -> bool {
        let taken = self.press(x, y);
        self.release(x, y);
        taken
    }

    fn key(&mut self, key: EditKey) -> bool {
        let resources = Resources::new();
        self.window_frame.key_pressed(&mut self.draw_manager, &resources, key)
    }

    fn last_input(&self) -> Option<T> {
        self.inputs.borrow().last().copied()
    }

    fn last_change(&self) -> Option<T> {
        self.changes.borrow().last().copied()
    }

    /// いま描いているつまみのまん中の x。つまみは原点まわりに組んで、複製の位置で置いてある。
    fn thumb_x(&self) -> f32 {
        let thumb = self.draw_manager.object("Window-Range-0-thumb").expect("つまみの図形がある");
        thumb.instances()[0].translation()[0]
    }

    /// 部品の図形すべての頂点の写し。
    fn vertices(&self) -> Vec<Vec<[f32; 2]>> {
        ["", "-track", "-fill-cap", "-fill", "-focus-ring", "-thumb"]
            .into_iter()
            .map(|suffix| {
                let object = self.draw_manager.object(&format!("Window-Range-0{suffix}")).expect("図形がある");
                object.vertices().iter().map(|vertex| [vertex.x, vertex.y]).collect()
            })
            .collect()
    }
}

#[test]
fn starts_in_the_middle() {
    let Some(scene) = scene::<i32>(|range| range) else { return };

    assert!((scene.thumb_x() - x_at(0.5)).abs() < 0.5, "既定は min 0 と max 100 のまん中");
}

#[test]
fn pressing_the_track_jumps_and_commits_on_release() {
    let Some(mut scene) = scene::<i32>(|range| range) else { return };

    assert!(scene.press(x_at(0.25), Y), "溝を押したら窓が取る");
    assert_eq!(scene.last_input(), Some(25), "押した場所へ飛ぶ");
    assert_eq!(scene.last_change(), None, "放すまで change は来ない");
    assert!(scene.window_frame.has_focus());

    scene.release(x_at(0.25), Y);
    assert_eq!(scene.last_change(), Some(25));
    assert!((scene.thumb_x() - x_at(0.25)).abs() < 0.5);
}

#[test]
fn dragging_the_thumb_keeps_the_grab_point() {
    let Some(mut scene) = scene::<i32>(|range| range) else { return };

    // つまみのまん中から 3 右を掴む。飛ばずに 50 のまま。
    let grab = x_at(0.5) + 3.0;
    scene.press(grab, Y);
    assert_eq!(scene.last_input(), None, "つまみを掴んでも値は変わらない");

    assert!(scene.moved(grab + TRAVEL * 0.25, Y));
    assert_eq!(scene.last_input(), Some(75));

    // 部品の外へ出ても付いてきて、端で止まる。
    scene.moved(1000.0, 300.0);
    assert_eq!(scene.last_input(), Some(100));
    scene.moved(-1000.0, 0.0);
    assert_eq!(scene.last_input(), Some(0));

    scene.moved(grab, Y);
    scene.release(grab, Y);
    assert_eq!(scene.last_change(), None, "元の値に戻して放したら change は来ない");
}

#[test]
fn values_snap_to_the_step() {
    let Some(mut scene) = scene::<i32>(|range| range.min(0).max(100).step(10)) else { return };

    scene.click(x_at(0.33), Y);
    assert_eq!(scene.last_change(), Some(30));
    assert!((scene.thumb_x() - x_at(0.3)).abs() < 0.5, "つまみも刻みの位置に描く");
}

#[test]
fn keys_move_by_step_page_and_to_the_ends() {
    let Some(mut scene) = scene::<i32>(|range| range.value(50)) else { return };
    assert!(!scene.key(EditKey::Right), "フォーカスが無ければ届かない");

    scene.click(x_at(0.5), Y);

    assert!(scene.key(EditKey::Right));
    assert_eq!(scene.last_change(), Some(51), "キーでは change もすぐ来る");
    scene.key(EditKey::Down);
    assert_eq!(scene.last_change(), Some(50));
    scene.key(EditKey::PageUp);
    assert_eq!(scene.last_change(), Some(60), "幅の 1 割");
    scene.key(EditKey::Home);
    assert_eq!(scene.last_change(), Some(0));
    assert!(!scene.key(EditKey::Left), "min より下へは動かない");
    scene.key(EditKey::End);
    assert_eq!(scene.last_change(), Some(100));
}

#[test]
fn decimal_steps_report_clean_values() {
    let Some(mut scene) = scene::<f64>(|range| range.min(0.0).max(1.0).step(0.1).value(0.2)) else { return };
    scene.click(x_at(0.2), Y);

    scene.key(EditKey::Up);
    assert_eq!(scene.last_change(), Some(0.3));
}

#[test]
fn clicking_outside_releases_focus() {
    let Some(mut scene) = scene::<i32>(|range| range) else { return };

    scene.click(x_at(0.5), Y);
    assert!(scene.window_frame.has_focus());

    scene.click(20.0, 20.0);
    assert!(!scene.window_frame.has_focus());
}

#[test]
fn hovering_the_thumb_repaints_and_shows_the_default_cursor() {
    let Some(mut scene) = scene::<i32>(|range| range) else { return };

    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, x_at(0.1), Y), CursorShape::Default);
    assert!(scene.moved(x_at(0.5), Y), "つまみに乗ると塗り替える");
    assert!(!scene.moved(x_at(0.5) + 1.0, Y), "乗ったままなら何もしない");
    assert!(scene.moved(x_at(0.1), Y), "離れると戻す");
}

#[test]
fn dragging_and_hovering_do_not_touch_vertices() {
    // 頂点を書き換えると全図形の積み直しになり、引くたびにかくつく。動かすのは複製だけ。
    let Some(mut scene) = scene::<f32>(|range| range.step_any()) else { return };
    let before = scene.vertices();

    scene.moved(x_at(0.5), Y);
    scene.press(x_at(0.5), Y);
    for step in 0..=20 {
        scene.moved(x_at(step as f32 / 20.0), Y);
    }
    scene.release(x_at(1.0), Y);
    scene.key(EditKey::Home);
    scene.click(20.0, 20.0);

    assert_eq!(scene.last_change(), Some(0.0));
    assert_eq!(scene.vertices(), before, "頂点は組んだときのまま");
    assert!((scene.thumb_x() - x_at(0.0)).abs() < 0.5, "つまみは複製の位置で動いている");
}
