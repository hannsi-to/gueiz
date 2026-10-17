//! 重ねた 2 枚の窓（[`GuiManager`]）に、押す・打つを流して確かめる。
//!
//! 当たり判定（[`DrawManager::pick`]）を通すので GPU が要ります。
//! GPU か書体が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test gui_manager_gpu -- --nocapture
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use gueiz_2d::camera::ScaleMode;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::gui_manager::{GuiManager, WindowId, WINDOW_DEPTH_STEP};
use gueiz_2d::gui::input_text::{InputText, InputTextFont, InputTextTheme};
use gueiz_2d::gui::window_frame::{Gap, Quad, QuadType, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::gui::window_item::CursorShape;
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::resource::{FontHandle, Resources};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

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
            label: Some("gui manager gpu test"),
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

/// 窓 1 枚。帯 30・余白 5 で、欄は窓の左上から (5, 40) に 200 x 28。
fn window(name: &str, x: f32, y: f32, font: FontHandle, text: Rc<RefCell<String>>) -> WindowFrame {
    let mut window_frame = WindowFrame::new(
        name.to_string(),
        ScaleMode::Stretch,
        name.to_string(),
        Quad { quad_type: QuadType::Normal, x, y, width: 300.0, height: 200.0 },
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

    window_frame.add_window_item(Box::new(
        InputText::new(
            "Input".to_string(),
            String::new(),
            200.0,
            28.0,
            8.0,
            QuadType::Normal,
            Gap { x: 0.0, y: 5.0 },
            InputTextFont { base_font: font, base_font_size: 14.0 },
            InputTextTheme {
                fill_color: color(0.05),
                outline_color: color(0.3),
                focus_outline_color: color(0.6),
                text_color: color(1.0),
                placeholder_color: color(0.5),
                caret_color: color(1.0),
                selection_color: color(0.3),
            },
        )
        .on_change(move |changed| *text.borrow_mut() = changed.to_string()),
    ));

    window_frame
}

/// 窓 A を (100, 100)、窓 B を (250, 150) に重ねる。B が手前。
///
/// - A の欄: (105, 140)〜(305, 168)
/// - B の欄: (255, 190)〜(455, 218)。左の半分は A の上に重なる
struct Scene {
    draw_manager: DrawManager,
    resources: Resources,
    gui: GuiManager,
    a: WindowId,
    b: WindowId,
    a_text: Rc<RefCell<String>>,
    b_text: Rc<RefCell<String>>,
}

fn scene() -> Option<Scene> {
    let Some(gpu) = gpu() else {
        eprintln!("skipped: no gpu adapter");
        return None;
    };

    let mut resources = Resources::new();
    let Some(font) = load_font(&mut resources) else {
        eprintln!("skipped: no font");
        return None;
    };

    let mut draw_manager = DrawManager::new(&gpu.device, &gpu.queue, TextureFormat::Bgra8UnormSrgb, &DrawManagerDescriptor::default())
        .expect("DrawManager を作れなかった");

    let (a_text, b_text) = (Rc::new(RefCell::new(String::new())), Rc::new(RefCell::new(String::new())));
    let surface_size = SurfaceSize::new(WIDTH, HEIGHT);
    let design = [WIDTH as f32, HEIGHT as f32];

    let mut gui = GuiManager::new();
    let a = gui.add_window(&mut draw_manager, &resources, surface_size, design, window("A", 100.0, 100.0, font, a_text.clone()));
    let b = gui.add_window(&mut draw_manager, &resources, surface_size, design, window("B", 250.0, 150.0, font, b_text.clone()));

    let mut scene = Scene { draw_manager, resources, gui, a, b, a_text, b_text };
    scene.frame();
    Some(scene)
}

impl Scene {
    /// 1 フレーム進める。当たり判定の手前・奥は描く順で決まるので、並べ直しまでする。
    fn frame(&mut self) {
        self.gui.update(&mut self.draw_manager, &self.resources);
        self.draw_manager.prepare().expect("支度に失敗した");
    }

    fn click(&mut self, x: f32, y: f32) -> bool {
        let taken = self.gui.mouse_left_pressed(&mut self.draw_manager, x, y);
        self.gui.mouse_left_released(&mut self.draw_manager, x, y);
        self.frame();
        taken
    }

    fn type_text(&mut self, text: &str) -> bool {
        let typed = self.gui.text_input(&mut self.draw_manager, &self.resources, text);
        self.frame();
        typed
    }

    fn depth(&self, id: WindowId) -> f32 {
        self.gui.window(id).expect("窓が無い").depth()
    }

    fn has_focus(&self, id: WindowId) -> bool {
        self.gui.window(id).expect("窓が無い").has_focus()
    }
}

#[test]
fn later_windows_are_stacked_in_front() {
    let Some(scene) = scene() else { return };

    assert_eq!(scene.gui.ids().collect::<Vec<_>>(), vec![scene.a, scene.b]);
    assert_eq!(scene.gui.front(), Some(scene.b));
    assert_eq!(scene.depth(scene.a), 0.0);
    assert_eq!(scene.depth(scene.b), WINDOW_DEPTH_STEP);
}

#[test]
fn a_press_on_the_overlap_goes_to_the_front_window() {
    let Some(mut scene) = scene() else { return };

    // B の欄の左端。A の窓の中でもあるが、手前は B。
    assert!(scene.click(270.0, 204.0));

    assert!(scene.has_focus(scene.b), "手前の B の欄が押されていない");
    assert!(!scene.has_focus(scene.a), "奥の A まで押されている");
    assert_eq!(scene.gui.cursor_shape(&scene.draw_manager, 270.0, 204.0), CursorShape::Text);

    assert!(scene.type_text("b"));
    assert_eq!(scene.b_text.borrow().as_str(), "b");
    assert_eq!(scene.a_text.borrow().as_str(), "", "奥の窓に字が入った");
}

#[test]
fn pressing_a_back_window_brings_it_to_the_front() {
    let Some(mut scene) = scene() else { return };

    // まず B の欄にフォーカスを置く。
    scene.click(420.0, 204.0);
    assert!(scene.has_focus(scene.b));

    // A の帯（B と重なっていないところ）を押す。
    assert!(scene.click(150.0, 115.0));

    assert_eq!(scene.gui.front(), Some(scene.a));
    assert_eq!(scene.depth(scene.a), WINDOW_DEPTH_STEP);
    assert_eq!(scene.depth(scene.b), 0.0);
    assert!(!scene.has_focus(scene.b), "奥へ回った B がフォーカスを持ったまま");

    // さっきまで B が手前だった重なりは、いまは A の中身。B の欄には届かない。
    assert!(scene.click(270.0, 204.0));
    assert!(!scene.has_focus(scene.b), "奥の B の欄が押された");
    assert_eq!(scene.gui.front(), Some(scene.a));
    assert_eq!(scene.gui.cursor_shape(&scene.draw_manager, 270.0, 204.0), CursorShape::Default);

    // 打っても、どの欄にも入らない。
    assert!(!scene.type_text("x"));
    assert_eq!(scene.b_text.borrow().as_str(), "");
}

#[test]
fn only_one_window_keeps_the_focus() {
    let Some(mut scene) = scene() else { return };

    scene.click(150.0, 154.0);
    assert!(scene.has_focus(scene.a));
    assert_eq!(scene.gui.focused(), Some(scene.a));

    // B の欄（A と重なっていないところ）を押すと、A は手放す。
    scene.click(420.0, 204.0);
    assert!(scene.has_focus(scene.b));
    assert!(!scene.has_focus(scene.a));
    assert_eq!(scene.gui.focused(), Some(scene.b));

    // どの窓でもないところを押すと、どれも手放す。
    assert!(!scene.click(600.0, 450.0));
    assert_eq!(scene.gui.focused(), None);
}

#[test]
fn glyphs_typed_after_raising_are_drawn_with_their_window() {
    let Some(mut scene) = scene() else { return };

    // A を手前に出してから、A の欄に打つ。打って増えた字の形にも、A の底上げが掛かっていること。
    scene.click(150.0, 115.0);
    assert_eq!(scene.gui.front(), Some(scene.a));

    scene.click(150.0, 154.0);
    assert!(scene.type_text("Hello"));
    assert_eq!(scene.a_text.borrow().as_str(), "Hello");

    let glyphs: Vec<f32> = scene
        .draw_manager
        .group_names("A-Input-0-text")
        .filter_map(|name| scene.draw_manager.object(name))
        .map(|object| object.depth())
        .collect();

    assert!(!glyphs.is_empty(), "A の欄の字が見つからない");
    assert!(
        glyphs.iter().all(|&depth| depth >= WINDOW_DEPTH_STEP),
        "底上げが掛かっていない字がある: {glyphs:?}",
    );

    // 奥の B の図形より手前に描かれていれば、字の上を押しても A に届く。
    assert_eq!(scene.gui.cursor_shape(&scene.draw_manager, 150.0, 154.0), CursorShape::Text);
}
