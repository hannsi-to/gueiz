//! 窓に置いた [`Color`] に、押す・引く・キーを流し、描いた画素も読み戻して確かめる。
//!
//! 当たり判定（[`DrawManager::pick`]）と描画を通すので GPU が要ります。
//! GPU か書体が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test color_gpu -- --nocapture
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use gueiz_2d::camera::ScaleMode;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::color::{Color, ColorFont, ColorTheme, Rgba};
use gueiz_2d::gui::range::{Range, RangeTheme};
use gueiz_2d::gui::window_frame::{Gap, Quad, QuadType, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::gui::window_item::{CursorShape, EditKey, KeyModifiers};
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::resource::{FontHandle, Resources};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

/// 色見本は (105, 140) から 60 x 28。パレットはその 4 下、(105, 172) から幅 220。
const SWATCH: (f32, f32) = (135.0, 154.0);
/// 面は (115, 182) から 200 x 130、色相の帯は (115, 322) から、透明度の帯は (115, 342) から 200 x 12。
const SV: [f32; 4] = [115.0, 182.0, 315.0, 312.0];
const HUE: [f32; 4] = [115.0, 322.0, 315.0, 334.0];
const ALPHA: [f32; 4] = [115.0, 342.0, 315.0, 354.0];
/// R・G・B・A の欄は y 380 から 24 の高さ、幅 44 で 8 ずつ空けて並ぶ。
fn field(index: usize) -> (f32, f32) {
    (115.0 + 52.0 * index as f32 + 22.0, 392.0)
}
/// 窓の中で、パレットの外。
const OUTSIDE: (f32, f32) = (450.0, 250.0);

/// 面の上の、彩度 `s`・明度 `v` の点。
fn sv_at(s: f32, v: f32) -> (f32, f32) {
    (SV[0] + (SV[2] - SV[0]) * s, SV[1] + (SV[3] - SV[1]) * (1.0 - v))
}

/// 帯の上の、割合 `ratio` の点。
fn bar_at(bar: [f32; 4], ratio: f32) -> (f32, f32) {
    (bar[0] + (bar[2] - bar[0]) * ratio, (bar[1] + bar[3]) / 2.0)
}

fn hue_at(h: f32) -> (f32, f32) {
    bar_at(HUE, h)
}

fn alpha_at(a: f32) -> (f32, f32) {
    bar_at(ALPHA, a)
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
            label: Some("color gpu test"),
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

struct Scene {
    draw_manager: DrawManager,
    resources: Resources,
    window_frame: WindowFrame,
    inputs: Rc<RefCell<Vec<Rgba>>>,
    changes: Rc<RefCell<Vec<Rgba>>>,
    /// パレットの下に重なる Range の `input`。パレットが手前で受け取れているかを見る。
    range_inputs: Rc<RefCell<Vec<i32>>>,
}

fn scene(customize: impl FnOnce(Color) -> Color) -> Option<Scene> {
    let Some(gpu) = gpu() else {
        eprintln!("skipped: no gpu adapter");
        return None;
    };

    let mut resources = Resources::new();
    let Some(font) = load_font(&mut resources) else {
        eprintln!("skipped: no font");
        return None;
    };

    let draw_manager = DrawManager::new(&gpu.device, &gpu.queue, FORMAT, &DrawManagerDescriptor::default())
        .expect("DrawManager を作れなかった");

    // 窓は (100, 100) から 400 x 360（パレットの下の端は 414）。帯 30・余白 5 なので、部品は (105, 135) から積まれる。
    let mut window_frame = WindowFrame::new(
        "Window".to_string(),
        ScaleMode::Stretch,
        "Window".to_string(),
        Quad { quad_type: QuadType::Normal, x: 100.0, y: 100.0, width: 400.0, height: 360.0 },
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
    let range_inputs = Rc::new(RefCell::new(Vec::new()));
    let (on_input, on_change, on_range) = (inputs.clone(), changes.clone(), range_inputs.clone());

    window_frame.add_window_item(Box::new(customize(
        Color::new(
            "Color".to_string(),
            60.0,
            28.0,
            Gap { x: 0.0, y: 5.0 },
            ColorFont { base_font: font, base_font_size: 13.0 },
            ColorTheme {
                swatch_outline_color: color(0.4),
                focus_outline_color: color(0.7),
                panel_fill_color: color(0.15),
                panel_outline_color: color(0.4),
                marker_color: color(1.0),
                marker_shadow_color: ThemeColor { r: 0.0, g: 0.0, b: 0.0, a: 0.6 },
                field_fill_color: color(0.08),
                field_outline_color: color(0.4),
                label_color: color(0.7),
                text_color: color(1.0),
            },
        )
        .on_input(move |value| on_input.borrow_mut().push(value))
        .on_change(move |value| on_change.borrow_mut().push(value)),
    )));

    // 色見本のすぐ下（173 から 193）。開いたパレットの面の上の端と重なる。
    window_frame.add_window_item(Box::new(
        Range::<i32>::new(
            "Range".to_string(),
            300.0,
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
        .on_input(move |value| on_range.borrow_mut().push(value)),
    ));

    let mut scene = Scene { draw_manager, resources, window_frame, inputs, changes, range_inputs };
    let surface_size = SurfaceSize::new(WIDTH, HEIGHT);
    scene.window_frame.create_object(&mut scene.draw_manager, &scene.resources, surface_size, [WIDTH as f32, HEIGHT as f32]);
    scene.update();

    Some(scene)
}

impl Scene {
    fn press(&mut self, (x, y): (f32, f32)) -> bool {
        self.window_frame.mouse_left_pressed(&mut self.draw_manager, x, y)
    }

    fn moved(&mut self, (x, y): (f32, f32)) -> bool {
        self.window_frame.mouse_moved(&mut self.draw_manager, x, y)
    }

    fn release(&mut self, (x, y): (f32, f32)) {
        self.window_frame.mouse_left_released(&mut self.draw_manager, x, y);
    }

    fn click(&mut self, at: (f32, f32)) -> bool {
        let taken = self.press(at);
        self.release(at);
        self.update();
        taken
    }

    fn type_text(&mut self, text: &str) -> bool {
        let handled = self.window_frame.text_input(&mut self.draw_manager, &self.resources, text);
        self.update();
        handled
    }

    fn key(&mut self, key: EditKey) -> bool {
        let handled = self.window_frame.key_pressed(&mut self.draw_manager, &self.resources, key);
        self.update();
        handled
    }

    /// 描く前の 1 回ぶん。重なり順の並べ替えは prepare で決まるので、アプリと同じく毎回呼ぶ。
    fn update(&mut self) {
        self.window_frame.update(&mut self.draw_manager, &self.resources);
        self.draw_manager.prepare().expect("支度に失敗した");
    }

    fn last_input(&self) -> Option<Rgba> {
        self.inputs.borrow().last().copied()
    }

    fn last_change(&self) -> Option<Rgba> {
        self.changes.borrow().last().copied()
    }

    /// パレットが開いているか。閉じているあいだ、パレットは画面の外へ逃がしてある。
    fn is_open(&self) -> bool {
        let panel = self.draw_manager.object("Window-Color-0-panel").expect("パレットの図形がある");
        panel.instances()[0].translation()[0] == 0.0
    }

    /// 部品の図形すべての頂点の写し。
    fn vertices(&self) -> Vec<Vec<[f32; 2]>> {
        ["", "-swatch", "-swatch-checker", "-panel", "-sv", "-sv-white", "-sv-black", "-hue", "-alpha", "-alpha-checker", "-field-r", "-caret", "-sv-marker", "-hue-marker", "-alpha-marker"]
            .into_iter()
            .map(|suffix| {
                let object = self.draw_manager.object(&format!("Window-Color-0{suffix}")).expect("図形がある");
                object.vertices().iter().map(|vertex| [vertex.x, vertex.y]).collect()
            })
            .collect()
    }

    /// 描いて、画素を読み戻す（BGRA）。
    fn render(&mut self) -> Vec<u8> {
        let gpu = gpu().expect("GPU は scene で確かめてある");
        let (device, queue) = (&gpu.device, &gpu.queue);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color gpu test target"),
            size: wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT.into(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("color gpu test readback"),
            size: (WIDTH * HEIGHT * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        self.draw_manager.prepare().expect("支度に失敗した");

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("color gpu test") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("color gpu test pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            self.draw_manager.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(WIDTH * 4), rows_per_image: Some(HEIGHT) },
            },
            wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));

        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU を待てなかった");
        let mapped = readback.get_mapped_range(..).expect("読み戻せなかった");
        let pixels = mapped.to_vec();
        drop(mapped);
        readback.unmap();
        pixels
    }
}

/// その画素の色。
fn pixel(pixels: &[u8], (x, y): (f32, f32)) -> [u8; 3] {
    let at = ((y as u32 * WIDTH + x as u32) * 4) as usize;
    [pixels[at + 2], pixels[at + 1], pixels[at]]
}

/// sRGB の 1 ch を光の量へ。
fn linear(value: u8) -> f32 {
    let value = value as f32 / 255.0;
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

/// 思った色に近いか。光の量で比べる（暗い側の差を大げさに見ないように）。
fn assert_near(got: [u8; 3], want: Rgba, note: &str) {
    let want = [want.r, want.g, want.b];
    let off = (0..3).map(|index| (linear(got[index]) - linear(want[index])).abs()).fold(0.0, f32::max);
    assert!(off <= 0.04, "{note}: {got:?} は {want:?} のはず（光の量で {off:.3} ずれ）");
}

/// HSV（0 から 1）を RGB へ。部品の中の計算とは別に書いて、描いた色と比べる。
fn expected(h: f32, s: f32, v: f32) -> Rgba {
    let channel = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        let value = v - v * s * k.min(4.0 - k).clamp(0.0, 1.0);
        (value * 255.0).round() as u8
    };
    Rgba::new(channel(5.0), channel(3.0), channel(1.0), 255)
}

#[test]
fn the_swatch_opens_and_closes_the_palette() {
    let Some(mut scene) = scene(|color| color) else { return };
    assert!(!scene.is_open());

    assert!(scene.click(SWATCH), "色見本を押したら窓が取る");
    assert!(scene.is_open());
    assert!(scene.window_frame.has_focus());

    scene.click(SWATCH);
    assert!(!scene.is_open(), "もう一度押すと閉じる");
    assert_eq!(scene.last_change(), None, "色が変わっていなければ change は来ない");
}

#[test]
fn dragging_the_plane_and_the_hue_bar_picks_colors() {
    let Some(mut scene) = scene(|color| color.value(Rgba::new(255, 0, 0, 255))) else { return };
    scene.click(SWATCH);

    scene.press(sv_at(0.75, 0.5));
    assert_eq!(scene.last_input(), Some(expected(0.0, 0.75, 0.5)), "押した場所の彩度と明度");
    scene.moved(sv_at(0.25, 0.75));
    assert_eq!(scene.last_input(), Some(expected(0.0, 0.25, 0.75)));
    // パレットの外へ出ても、端で止めて付いてくる。
    scene.moved((1000.0, -1000.0));
    assert_eq!(scene.last_input(), Some(Rgba::new(255, 0, 0, 255)), "右上の端は彩度 1・明度 1");
    scene.release((1000.0, -1000.0));

    scene.press(hue_at(1.0 / 3.0));
    scene.release(hue_at(1.0 / 3.0));
    assert_eq!(scene.last_input(), Some(Rgba::new(0, 255, 0, 255)), "帯の 1/3 は緑");
    assert_eq!(scene.last_change(), None, "開いているあいだは change は来ない");

    // パレットの外を押すと決めて閉じる。
    scene.click(OUTSIDE);
    assert!(!scene.is_open());
    assert_eq!(scene.last_change(), Some(Rgba::new(0, 255, 0, 255)));
}

#[test]
fn escape_restores_the_color_from_before_opening() {
    let Some(mut scene) = scene(|color| color.value(Rgba::new(10, 20, 30, 255))) else { return };
    scene.click(SWATCH);
    scene.click(sv_at(0.9, 0.9));
    assert!(scene.last_input().is_some());

    assert!(scene.key(EditKey::Escape));
    assert!(!scene.is_open());
    assert_eq!(scene.last_change(), None, "戻したので change は来ない");

    // 開き直すと、見本は元の色。Enter で開き、Enter で閉じる。
    assert!(scene.key(EditKey::Enter));
    assert!(scene.is_open());
    assert!(scene.key(EditKey::Enter));
    assert_eq!(scene.last_change(), None);
}

#[test]
fn keys_move_saturation_value_and_hue() {
    let Some(mut scene) = scene(|color| color.value(Rgba::new(255, 0, 0, 255))) else { return };
    scene.click(SWATCH);

    scene.key(EditKey::Down);
    assert_eq!(scene.last_input(), Some(expected(0.0, 1.0, 0.99)), "↓ で明度を 1% 下げる");
    scene.key(EditKey::Left);
    assert_eq!(scene.last_input(), Some(expected(0.0, 0.99, 0.99)), "← で彩度を 1% 下げる");
    scene.key(EditKey::PageUp);
    assert_eq!(scene.last_input(), Some(expected(10.0 / 360.0, 0.99, 0.99)), "PageUp で色相を 10 度");

    scene.key(EditKey::Enter);
    assert_eq!(scene.last_change(), Some(expected(10.0 / 360.0, 0.99, 0.99)));
}

#[test]
fn the_palette_draws_in_front_of_later_items() {
    let Some(mut scene) = scene(|color| color) else { return };
    scene.click(SWATCH);

    // Range（173 から 193）とパレットの面が重なるところを押す。手前のパレットが受け取る。
    let overlap = (215.0, 186.0);
    assert!(scene.press(overlap));
    scene.release(overlap);
    assert!(scene.last_input().is_some(), "パレットが受け取る");
    assert!(scene.range_inputs.borrow().is_empty(), "下の Range には届かない");

    // 窓を動かしても手前のまま（窓のずらしで重なり順を消さない）。
    scene.window_frame.mouse_left_pressed(&mut scene.draw_manager, 400.0, 115.0);
    scene.window_frame.mouse_moved(&mut scene.draw_manager, 420.0, 125.0);
    scene.window_frame.mouse_left_released(&mut scene.draw_manager, 420.0, 125.0);
    scene.update();
    let panel = scene.draw_manager.object("Window-Color-0-panel").unwrap();
    assert!(panel.depth() > 0.0, "パレットの重なり順が残っている");
}

#[test]
fn the_drawn_plane_matches_the_picked_colors() {
    let Some(mut scene) = scene(|color| color.value(Rgba::new(255, 0, 0, 255))) else { return };
    scene.click(SWATCH);
    let pixels = scene.render();

    // 面の色は、選んだときの色と（光の量で）そろう。
    for (s, v) in [(0.5, 0.5), (0.25, 0.75), (0.8, 0.9), (0.1, 0.2), (0.6, 0.3)] {
        assert_near(pixel(&pixels, sv_at(s, v)), expected(0.0, s, v), &format!("面の s {s} v {v}"));
    }
    // 色相の帯。6 色の止めの上はちょうどその色。
    for (h, want) in [(1.0 / 6.0, Rgba::new(255, 255, 0, 255)), (0.5, Rgba::new(0, 255, 255, 255)), (2.0 / 3.0, Rgba::new(0, 0, 255, 255))] {
        assert_near(pixel(&pixels, hue_at(h)), want, &format!("帯の h {h}"));
    }
    // 色見本は値の色。
    assert_near(pixel(&pixels, (112.0, 147.0)), Rgba::new(255, 0, 0, 255), "色見本");
}

#[test]
fn dragging_does_not_touch_vertices() {
    // 頂点を書き換えると全図形の積み直しになるので、動かすのは複製だけ。
    let Some(mut scene) = scene(|color| color) else { return };
    let before = scene.vertices();

    scene.click(SWATCH);
    scene.press(sv_at(0.2, 0.2));
    for step in 0..=10 {
        scene.moved(sv_at(step as f32 / 10.0, 0.5));
    }
    scene.release(sv_at(1.0, 0.5));
    scene.click(hue_at(0.7));
    scene.key(EditKey::PageUp);
    scene.click(OUTSIDE);

    assert_eq!(scene.vertices(), before, "頂点は組んだときのまま");
}

#[test]
fn the_swatch_shows_the_pointer_cursor() {
    let Some(mut scene) = scene(|color| color) else { return };
    let (x, y) = SWATCH;
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, x, y), CursorShape::Pointer);

    scene.click(SWATCH);
    let (x, y) = sv_at(0.5, 0.5);
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, x, y), CursorShape::Default);
}

#[test]
fn the_alpha_bar_picks_transparency() {
    let Some(mut scene) = scene(|color| color.value(Rgba::new(255, 0, 0, 255))) else { return };
    scene.click(SWATCH);

    scene.press(alpha_at(0.5));
    assert_eq!(scene.last_input(), Some(Rgba::new(255, 0, 0, 128)), "帯のまん中は 128");
    scene.moved((-1000.0, 0.0));
    assert_eq!(scene.last_input(), Some(Rgba::new(255, 0, 0, 0)), "左の端は透明");
    scene.release((-1000.0, 0.0));

    scene.click(OUTSIDE);
    assert_eq!(scene.last_change(), Some(Rgba::new(255, 0, 0, 0)));
}

#[test]
fn fields_take_typed_numbers() {
    let Some(mut scene) = scene(|color| color.value(Rgba::new(10, 20, 30, 255))) else { return };
    scene.click(SWATCH);

    // 欄を押すと全部選ばれ、打つと置き換わる。打った値はすぐ色に出る。
    assert!(scene.click(field(0)));
    assert!(scene.type_text("1"));
    assert_eq!(scene.last_input(), Some(Rgba::new(1, 20, 30, 255)));
    assert!(!scene.type_text("x"), "数字のほかは打てない");
    scene.type_text("23");
    assert_eq!(scene.last_input(), Some(Rgba::new(123, 20, 30, 255)));
    scene.type_text("4");
    assert_eq!(scene.last_input(), Some(Rgba::new(123, 20, 30, 255)), "3 桁まで");

    // Tab で次の欄。255 を超える数は 255。
    scene.key(EditKey::Tab);
    scene.type_text("300");
    assert_eq!(scene.last_input(), Some(Rgba::new(123, 255, 30, 255)));

    // ↑↓ で 1 ずつ、Backspace で 1 字消す。
    scene.key(EditKey::Tab);
    scene.key(EditKey::Up);
    assert_eq!(scene.last_input(), Some(Rgba::new(123, 255, 31, 255)));
    scene.key(EditKey::Backspace);
    assert_eq!(scene.last_input(), Some(Rgba::new(123, 255, 3, 255)), "31 の 1 を消して 3");

    // A の欄。
    scene.key(EditKey::Tab);
    scene.type_text("64");
    assert_eq!(scene.last_input(), Some(Rgba::new(123, 255, 3, 64)));

    // Shift+Tab で前の欄（B）へ戻る。
    scene.window_frame.modifiers_changed(KeyModifiers { shift: true, word: false });
    scene.key(EditKey::Tab);
    scene.window_frame.modifiers_changed(KeyModifiers::default());
    scene.type_text("7");
    assert_eq!(scene.last_input(), Some(Rgba::new(123, 255, 7, 64)));

    // 欄には名前と数が描かれている（白い字の画素がある）。
    let pixels = scene.render();
    let bright = |(x, y): (f32, f32)| (-20..20).flat_map(|dx| (-10..10).map(move |dy| (x + dx as f32, y + dy as f32))).any(|at| pixel(&pixels, at)[0] > 150);
    assert!(bright(field(0)), "R の欄の数");
    assert!(bright((field(3).0, 372.0)), "A の欄の名前");

    // Enter で欄を出て、もう一度 Enter で閉じる。
    scene.key(EditKey::Enter);
    assert!(scene.is_open(), "1 回めは欄を出るだけ");
    scene.key(EditKey::Enter);
    assert!(!scene.is_open());
    assert_eq!(scene.last_change(), Some(Rgba::new(123, 255, 7, 64)));
}

#[test]
fn the_caret_follows_typing() {
    let Some(mut scene) = scene(|color| color) else { return };
    scene.click(SWATCH);
    let caret_x = |scene: &Scene| scene.draw_manager.object("Window-Color-0-caret").unwrap().instances()[0].translation()[0];

    scene.click(field(0));
    assert!(caret_x(&scene) < -1000.0, "全部選んでいるあいだは縦棒を出さない");

    scene.type_text("1");
    let one = caret_x(&scene);
    scene.type_text("11");
    let three = caret_x(&scene);
    assert!(one > 115.0 && three > one, "打つほど右へ: {one} -> {three}");
}

#[test]
fn transparency_shows_the_checkerboard() {
    let Some(mut scene) = scene(|color| color.value(Rgba::new(255, 0, 0, 0))) else { return };
    scene.click(SWATCH);
    let pixels = scene.render();

    // 透明な色見本の後ろには市松模様。隣り合うマスは明るさが違う。
    let light = pixel(&pixels, (111.0, 146.0));
    let dark = pixel(&pixels, (117.0, 146.0));
    assert_ne!(light, dark, "色見本の市松模様: {light:?} と {dark:?}");
    assert!(light.iter().all(|channel| (channel.abs_diff(light[0])) < 3), "色見本は透明なので赤が乗らない: {light:?}");

    // 透明度の帯は、左の端で市松模様、右の端で不透明な赤。
    let left_light = pixel(&pixels, (ALPHA[0] + 3.0, ALPHA[1] + 3.0));
    let left_dark = pixel(&pixels, (ALPHA[0] + 9.0, ALPHA[1] + 3.0));
    assert_ne!(left_light, left_dark, "帯の左の端の市松模様");
    assert_near(pixel(&pixels, alpha_at(0.97)), Rgba::new(255, 0, 0, 255), "帯の右の端");
}
