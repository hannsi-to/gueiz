//! 窓に置いた [`Dropdown`] に、押す・指を乗せる・ホイール・キー・打つを流し、描いた画素も読み戻して確かめる。
//!
//! 当たり判定（[`DrawManager::pick`]）と描画を通すので GPU が要ります。
//! GPU か書体が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test dropdown_gpu -- --nocapture
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use gueiz_2d::camera::ScaleMode;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::dropdown::{Dropdown, DropdownFont, DropdownTheme};
use gueiz_2d::gui::range::{Range, RangeTheme};
use gueiz_2d::gui::window_frame::{Gap, Quad, QuadType, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::gui::window_item::{CursorShape, EditKey};
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::resource::{FontHandle, Resources};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

const FRUITS: [&str; 12] = [
    "Apple", "Banana", "Blueberry", "Cherry", "Date", "Elderberry", "Fig", "Grape", "Kiwi", "Lemon", "Mango", "Nectarine",
];

/// ボタンは (105, 140) から 200 x 28。一覧はその 2 下（170）から、行は 174 から 28 ずつ 5 行。
const BUTTON: (f32, f32) = (180.0, 154.0);
const LIST_TOP: f32 = 170.0;
const ROW_TOP: f32 = 174.0;
const ROW: f32 = 28.0;
/// 窓の中で、一覧の外。
const OUTSIDE: (f32, f32) = (450.0, 250.0);

/// 一覧の `slot` 行めのまん中。
fn row(slot: usize) -> (f32, f32) {
    (200.0, ROW_TOP + ROW * slot as f32 + ROW / 2.0)
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
            label: Some("dropdown gpu test"),
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
    changes: Rc<RefCell<Vec<(usize, String)>>>,
    /// 一覧の下に重なる Range の `input`。一覧が手前で受け取れているかを見る。
    range_inputs: Rc<RefCell<Vec<i32>>>,
}

fn scene(options: &[&str], customize: impl FnOnce(Dropdown) -> Dropdown) -> Option<Scene> {
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

    // 窓は (100, 100) から 400 x 300。帯 30・余白 5 なので、部品は (105, 135) から積まれる。
    let mut window_frame = WindowFrame::new(
        "Window".to_string(),
        ScaleMode::Stretch,
        "Window".to_string(),
        Quad { quad_type: QuadType::Normal, x: 100.0, y: 100.0, width: 400.0, height: 300.0 },
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

    let changes = Rc::new(RefCell::new(Vec::new()));
    let range_inputs = Rc::new(RefCell::new(Vec::new()));
    let (on_change, on_range) = (changes.clone(), range_inputs.clone());

    window_frame.add_window_item(Box::new(customize(
        Dropdown::new(
            "Dropdown".to_string(),
            200.0,
            28.0,
            QuadType::Rounded { radius: 4.0 },
            Gap { x: 0.0, y: 5.0 },
            DropdownFont { base_font: font, base_font_size: 14.0 },
            DropdownTheme {
                fill_color: color(0.08),
                outline_color: color(0.4),
                focus_outline_color: color(0.7),
                text_color: color(1.0),
                arrow_color: color(0.8),
                list_fill_color: color(0.12),
                list_outline_color: color(0.4),
                highlight_fill_color: ThemeColor { r: 0.2, g: 0.35, b: 0.7, a: 1.0 },
                highlight_text_color: color(1.0),
                scrollbar_color: color(0.5),
            },
        )
        .options(options.iter().copied())
        .max_visible(5)
        .on_change(move |index, text| on_change.borrow_mut().push((index, text.to_string()))),
    )));

    // ボタンのすぐ下（173 から 193）。開いた一覧の上の行と重なる。
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

    let mut scene = Scene { draw_manager, resources, window_frame, changes, range_inputs };
    let surface_size = SurfaceSize::new(WIDTH, HEIGHT);
    scene.window_frame.create_object(&mut scene.draw_manager, &scene.resources, surface_size, [WIDTH as f32, HEIGHT as f32]);
    scene.update();

    Some(scene)
}

impl Scene {
    fn click(&mut self, (x, y): (f32, f32)) -> bool {
        let taken = self.window_frame.mouse_left_pressed(&mut self.draw_manager, x, y);
        self.window_frame.mouse_left_released(&mut self.draw_manager, x, y);
        self.update();
        taken
    }

    fn moved(&mut self, (x, y): (f32, f32)) -> bool {
        let changed = self.window_frame.mouse_moved(&mut self.draw_manager, x, y);
        self.update();
        changed
    }

    fn wheel(&mut self, (x, y): (f32, f32), lines: f32) -> bool {
        let taken = self.window_frame.mouse_wheel(&mut self.draw_manager, x, y, [0.0, lines]);
        self.update();
        taken
    }

    fn key(&mut self, key: EditKey) -> bool {
        let handled = self.window_frame.key_pressed(&mut self.draw_manager, &self.resources, key);
        self.update();
        handled
    }

    fn type_text(&mut self, text: &str) -> bool {
        let handled = self.window_frame.text_input(&mut self.draw_manager, &self.resources, text);
        self.update();
        handled
    }

    /// 描く前の 1 回ぶん。重なり順の並べ替えは prepare で決まるので、アプリと同じく毎回呼ぶ。
    fn update(&mut self) {
        self.window_frame.update(&mut self.draw_manager, &self.resources);
        self.draw_manager.prepare().expect("支度に失敗した");
    }

    fn last_change(&self) -> Option<(usize, String)> {
        self.changes.borrow().last().cloned()
    }

    fn translation(&self, suffix: &str) -> [f32; 3] {
        let object = self.draw_manager.object(&format!("Window-Dropdown-0{suffix}")).expect("図形がある");
        object.instances()[0].translation()
    }

    /// 一覧が開いているか。閉じているあいだ、一覧は画面の外へ逃がしてある。
    fn is_open(&self) -> bool {
        self.translation("-list")[0] == 0.0
    }

    /// 光らせている行（見えている行の何番め）。光らせていなければ `None`。
    fn highlighted_slot(&self) -> Option<usize> {
        let [x, y, _] = self.translation("-highlight");
        (x == 0.0).then(|| ((y - ROW_TOP) / ROW).round() as usize)
    }

    fn vertices(&self) -> Vec<Vec<[f32; 2]>> {
        ["", "-outline", "-arrow", "-list", "-list-outline", "-highlight", "-scrollbar"]
            .into_iter()
            .map(|suffix| {
                let object = self.draw_manager.object(&format!("Window-Dropdown-0{suffix}")).expect("図形がある");
                object.vertices().iter().map(|vertex| [vertex.x, vertex.y]).collect()
            })
            .collect()
    }

    /// 描いて、画素を読み戻す（BGRA）。
    fn render(&mut self) -> Vec<u8> {
        let gpu = gpu().expect("GPU は scene で確かめてある");
        let (device, queue) = (&gpu.device, &gpu.queue);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dropdown gpu test target"),
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
            label: Some("dropdown gpu test readback"),
            size: (WIDTH * HEIGHT * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        self.draw_manager.prepare().expect("支度に失敗した");

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("dropdown gpu test") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("dropdown gpu test pass"),
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

/// その画素の明るさ（赤のチャンネル）。
fn brightness(pixels: &[u8], x: f32, y: f32) -> u8 {
    pixels[((y as u32 * WIDTH + x as u32) * 4 + 2) as usize]
}

/// 枠の中に、明るい（白い字の）画素があるか。
fn has_text(pixels: &[u8], [left, top, right, bottom]: [f32; 4]) -> bool {
    (left as u32..right as u32).any(|x| (top as u32..bottom as u32).any(|y| brightness(pixels, x as f32, y as f32) > 200))
}

fn change(index: usize) -> Option<(usize, String)> {
    Some((index, FRUITS[index].to_string()))
}

#[test]
fn the_first_option_is_selected_and_drawn() {
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown) else { return };
    assert!(!scene.is_open());

    let pixels = scene.render();
    assert!(has_text(&pixels, [113.0, 140.0, 200.0, 168.0]), "ボタンに最初の項目の字");
}

#[test]
fn clicking_opens_and_picks_an_option() {
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown) else { return };

    assert!(scene.click(BUTTON), "ボタンを押したら窓が取る");
    assert!(scene.is_open());
    assert_eq!(scene.highlighted_slot(), Some(0), "開くと選んでいる項目を光らせる");
    let pixels = scene.render();
    assert!(has_text(&pixels, [113.0, ROW_TOP + ROW * 4.0, 250.0, ROW_TOP + ROW * 5.0]), "5 行めの字");

    // 指が乗った行を光らせ、押すと選んで閉じる。
    assert!(scene.moved(row(2)));
    assert_eq!(scene.highlighted_slot(), Some(2));
    scene.click(row(2));
    assert!(!scene.is_open());
    assert_eq!(scene.last_change(), change(2));
}

#[test]
fn clicking_outside_closes_without_picking() {
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown) else { return };
    scene.click(BUTTON);
    scene.moved(row(3));

    scene.click(OUTSIDE);
    assert!(!scene.is_open());
    assert_eq!(scene.last_change(), None);
}

#[test]
fn arrow_keys_change_the_selection_while_closed() {
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown.selected(1)) else { return };
    scene.click(BUTTON);
    scene.click(BUTTON);
    assert!(!scene.is_open(), "2 回押すと閉じる");

    assert!(scene.key(EditKey::Down));
    assert_eq!(scene.last_change(), change(2), "閉じたまま次の項目");
    scene.key(EditKey::End);
    assert_eq!(scene.last_change(), change(11));
    assert!(!scene.key(EditKey::Down), "最後より先へは行かない");
    scene.key(EditKey::Home);
    assert_eq!(scene.last_change(), change(0));
    assert!(!scene.key(EditKey::Up), "最初より前へは行かない");
    assert!(!scene.is_open(), "↑↓ では開かない");
}

#[test]
fn keys_move_the_highlight_while_open() {
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown) else { return };
    scene.click(BUTTON);
    scene.key(EditKey::Escape);
    assert!(!scene.is_open());

    assert!(scene.key(EditKey::Enter), "Enter で開く");
    scene.key(EditKey::Down);
    scene.key(EditKey::Down);
    assert_eq!(scene.highlighted_slot(), Some(2));
    assert_eq!(scene.last_change(), None, "光らせるだけでは選ばない");

    // 見えている 5 行の外へ動かすと、送って見せる。
    scene.key(EditKey::End);
    assert_eq!(scene.highlighted_slot(), Some(4), "最後の項目は一番下の行");
    scene.key(EditKey::PageUp);
    assert_eq!(scene.highlighted_slot(), Some(0), "4 つ上（7 番）は、送らずに見えている一番上の行");

    scene.key(EditKey::Enter);
    assert!(!scene.is_open());
    assert_eq!(scene.last_change(), change(7));

    scene.key(EditKey::Enter);
    scene.key(EditKey::Down);
    scene.key(EditKey::Escape);
    assert_eq!(scene.last_change(), change(7), "Esc は選ばずに閉じる");
}

#[test]
fn the_wheel_scrolls_the_list() {
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown) else { return };
    scene.click(BUTTON);
    let thumb_top = scene.translation("-scrollbar")[1];
    assert!(thumb_top > LIST_TOP, "項目が多いのでスクロールバーが出る");

    assert!(scene.wheel(row(2), -3.0), "一覧の上のホイールは取る");
    assert!(scene.translation("-scrollbar")[1] > thumb_top, "つまみが下がる");
    scene.click(row(0));
    assert_eq!(scene.last_change(), change(3), "3 行送ったので 1 行めは 4 番め");

    scene.click(BUTTON);
    scene.wheel(row(0), -100.0);
    scene.click(row(4));
    assert_eq!(scene.last_change(), change(11), "送りすぎても最後で止まる");
}

#[test]
fn typing_jumps_to_matching_options() {
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown) else { return };
    scene.click(BUTTON);
    scene.click(BUTTON);

    assert!(scene.type_text("b"));
    assert_eq!(scene.last_change(), change(1), "b で始まる最初の項目");
    scene.type_text("B");
    assert_eq!(scene.last_change(), change(2), "同じ字を続けると次の項目");
    scene.type_text("b");
    assert_eq!(scene.last_change(), change(1), "一回りして戻る");
    assert!(!scene.type_text("z"), "当てはまらなければ何もしない");

    // Space で開き、開いているあいだは光らせるだけ。
    assert!(scene.type_text(" "));
    assert!(scene.is_open());
    scene.type_text("n");
    assert_eq!(scene.highlighted_slot(), Some(4), "最後の Nectarine まで送って光らせる");
    assert_eq!(scene.last_change(), change(1));
}

#[test]
fn the_list_draws_in_front_of_later_items() {
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown) else { return };
    scene.click(BUTTON);

    // Range（173 から 193）と一覧の 1 行めが重なるところを押す。手前の一覧が受け取る。
    scene.click(row(0));
    assert_eq!(scene.last_change(), None, "1 行めは選んでいる項目なので変わらない");
    assert!(!scene.is_open(), "一覧が受け取って閉じた");
    assert!(scene.range_inputs.borrow().is_empty(), "下の Range には届かない");
}

#[test]
fn long_text_is_clipped_before_the_arrow() {
    let long = "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW";
    let Some(mut scene) = scene(&[long, "short"], |dropdown| dropdown) else { return };
    let pixels = scene.render();

    assert!(has_text(&pixels, [113.0, 140.0, 270.0, 168.0]), "字はある");
    assert!(!has_text(&pixels, [279.0, 141.0, 304.0, 149.0]), "▼ の場所には字がはみ出さない");
}

#[test]
fn moving_and_opening_do_not_touch_vertices() {
    // 頂点を書き換えると全図形の積み直しになるので、動かすのは複製だけ。
    let Some(mut scene) = scene(&FRUITS, |dropdown| dropdown) else { return };
    let before = scene.vertices();

    scene.click(BUTTON);
    for slot in 0..5 {
        scene.moved(row(slot));
    }
    scene.wheel(row(0), -2.0);
    scene.key(EditKey::End);
    scene.click(row(1));
    scene.key(EditKey::Down);

    assert_eq!(scene.vertices(), before, "頂点は組んだときのまま");
}

#[test]
fn the_dropdown_shows_the_default_cursor() {
    let Some(scene) = scene(&FRUITS, |dropdown| dropdown) else { return };
    let (x, y) = BUTTON;
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, x, y), CursorShape::Default);
}
