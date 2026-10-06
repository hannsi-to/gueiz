//! 題名の帯を掴んで窓が動くかを、**実際に図形を動かして**確かめる。
//!
//! 指の位置から図形の座標へ戻す道（カメラ）と、掴んだ場所を覚える道が
//! 両方そろって初めて動きます。片方でも欠けると、掴んだ瞬間に窓が飛びます。
//!
//! GPU が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test drag_gpu -- --nocapture
//! ```

use std::sync::OnceLock;

use gueiz_2d::camera::ScaleMode;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::window_frame::{Gap, Quad, ThemeColor, WindowFrame, WindowTheme};
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const SIZE: u32 = 512;
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

/// 窓の置き場所と大きさ。
const FRAME: (f32, f32, f32, f32) = (100.0, 100.0, 300.0, 160.0);
const TITLE_BAR: f32 = 30.0;

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
            label: Some("drag gpu test"),
            required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
            ..Default::default()
        }))
        .ok()?;

        Some(Gpu { device, queue })
    })
    .as_ref()
}

macro_rules! gpu {
    () => {
        match gpu() {
            Some(gpu) => gpu,
            None => {
                eprintln!("skipped: no gpu adapter");
                return;
            }
        }
    };
}

fn theme() -> WindowTheme {
    let grey = |level: f32| ThemeColor {
        r: level,
        g: level,
        b: level,
        a: 1.0,
    };

    WindowTheme {
        frame_fill_color: grey(0.1),
        frame_outline_color: grey(0.5),
        title_bar_fill_color: grey(0.2),
        title_bar_slice_line_color: grey(0.3),
        text_title_color: grey(1.0),
    }
}

/// 窓を 1 つ置いた `DrawManager` と、その窓。
///
/// 字は書体がいる環境としない環境があるので、題名は空にして形だけ試す。
fn scene(gpu: &Gpu) -> (DrawManager, WindowFrame) {
    let mut draw_manager = DrawManager::new(
        &gpu.device,
        &gpu.queue,
        FORMAT,
        &DrawManagerDescriptor::default(),
    )
    .expect("DrawManager を作れなかった");

    let mut window_frame = WindowFrame::new(
        ScaleMode::Fixed,
        String::new(),
        Quad {
            x: FRAME.0,
            y: FRAME.1,
            width: FRAME.2,
            height: FRAME.3,
        },
        TITLE_BAR,
        Gap { x: 5.0, y: 0.0 },
        theme(),
    );

    let surface = SurfaceSize::new(SIZE, SIZE);

    window_frame.create_object("Window", surface, [SIZE as f32, SIZE as f32]);
    window_frame.register_draw_manager(&mut draw_manager);

    // 並びを決めるのに 1 度通す。`pick` は描く順を見る。
    draw_manager
        .prepare(&gpu.device, &gpu.queue)
        .expect("支度に失敗した");

    (draw_manager, window_frame)
}

/// 帯の真ん中あたり。
fn on_the_bar() -> (f32, f32) {
    (FRAME.0 + FRAME.2 / 2.0, FRAME.1 + TITLE_BAR / 2.0)
}

/// 中身の真ん中あたり。
fn on_the_body() -> (f32, f32) {
    (FRAME.0 + FRAME.2 / 2.0, FRAME.1 + TITLE_BAR + 40.0)
}

/// 描いて画素を読み戻す。
fn render(gpu: &Gpu, manager: &mut DrawManager) -> Vec<u8> {
    manager
        .prepare(&gpu.device, &gpu.queue)
        .expect("支度に失敗した");

    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("drag target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT.into(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("drag readback"),
        size: (SIZE * SIZE * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("drag") });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("window"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        manager.draw(&mut pass);
    }

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );

    gpu.queue.submit(Some(encoder.finish()));

    readback.map_async(wgpu::MapMode::Read, .., |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("待てなかった");

    let mapped = readback.get_mapped_range(..);
    let pixels = mapped.expect("読み戻せなかった").to_vec();
    readback.unmap();

    pixels
}

/// そこに何か描かれているか（黒い背景でないか）。
fn painted(pixels: &[u8], x: u32, y: u32) -> bool {
    let at = ((y * SIZE + x) * 4) as usize;

    pixels[at] != 0 || pixels[at + 1] != 0 || pixels[at + 2] != 0
}

/// **動かしたぶんだけ画素も動くこと。**
///
/// `Object::translate` は親の変換で、図形の記述として毎フレーム GPU へ送られます。
/// ここが届いていないと、当たり判定だけ動いて絵が残ります。
#[test]
fn the_painted_pixels_move_with_the_window() {
    let gpu = gpu!();
    let (mut draw_manager, mut frame) = scene(gpu);

    let before = render(gpu, &mut draw_manager);

    // 元の場所に描かれていて、移す先には何も無い。
    assert!(painted(&before, 150, 150), "元の場所に描かれていない");
    assert!(!painted(&before, 350, 300), "移す先に何かある");

    let (x, y) = on_the_bar();
    frame.mouse_left_pressed(&draw_manager, x, y);
    frame.mouse_moved(&mut draw_manager, x + 200.0, y + 150.0);
    frame.mouse_left_released();

    let after = render(gpu, &mut draw_manager);

    assert!(painted(&after, 350, 300), "移した先に描かれていない");
    assert!(!painted(&after, 150, 150), "元の場所に残っている");
}

#[test]
fn the_title_bar_drags_the_window() {
    let gpu = gpu!();
    let (mut draw_manager, mut frame) = scene(gpu);

    assert_eq!(frame.position(), [FRAME.0, FRAME.1]);

    let (x, y) = on_the_bar();
    assert!(frame.mouse_left_pressed(&draw_manager, x, y), "帯で掴めない");
    assert!(frame.is_dragging());

    // 右下へ 60, 40 動かす。
    assert!(frame.mouse_moved(&mut draw_manager, x + 60.0, y + 40.0));

    assert_eq!(frame.position(), [FRAME.0 + 60.0, FRAME.1 + 40.0]);

    assert!(frame.mouse_left_released());
    assert!(!frame.is_dragging());

    // 放したあとは付いてこない。
    assert!(!frame.mouse_moved(&mut draw_manager, x + 200.0, y + 200.0));
    assert_eq!(frame.position(), [FRAME.0 + 60.0, FRAME.1 + 40.0]);
}

/// **掴んだ場所が指の下に留まること。**
///
/// 動いた量を足していく作りだと、ここが端にずれます。
#[test]
fn the_grab_point_stays_under_the_finger() {
    let gpu = gpu!();
    let (mut draw_manager, mut frame) = scene(gpu);

    // 帯の左の端のほうを掴む。
    let grab_x = FRAME.0 + 10.0;
    let grab_y = FRAME.1 + 5.0;

    assert!(frame.mouse_left_pressed(&draw_manager, grab_x, grab_y));

    for step in 1..=20 {
        let x = grab_x + step as f32 * 7.0;
        let y = grab_y + step as f32 * 3.0;

        frame.mouse_moved(&mut draw_manager, x, y);

        // 掴んだ場所の、窓の左上からのずれは変わらない。
        let [left, top] = frame.position();

        assert!((x - left - 10.0).abs() < 1e-3, "{step}: {}", x - left);
        assert!((y - top - 5.0).abs() < 1e-3, "{step}: {}", y - top);
    }
}

#[test]
fn the_body_does_not_drag() {
    let gpu = gpu!();
    let (mut draw_manager, mut frame) = scene(gpu);

    let (x, y) = on_the_body();

    assert!(!frame.mouse_left_pressed(&draw_manager, x, y), "中身で掴めている");
    assert!(!frame.is_dragging());

    frame.mouse_moved(&mut draw_manager, x + 100.0, y + 100.0);
    assert_eq!(frame.position(), [FRAME.0, FRAME.1], "動いてしまった");
}

#[test]
fn nothing_outside_the_window_drags() {
    let gpu = gpu!();
    let (draw_manager, mut frame) = scene(gpu);

    // 窓の外。
    assert!(!frame.mouse_left_pressed(&draw_manager, 10.0, 10.0));
    // 帯の高さより下（中身の上端のすぐ下）。
    assert!(!frame.mouse_left_pressed(&draw_manager, FRAME.0 + 5.0, FRAME.1 + TITLE_BAR + 1.0));
    // 帯の右の外。
    assert!(!frame.mouse_left_pressed(&draw_manager, FRAME.0 + FRAME.2 + 5.0, FRAME.1 + 5.0));

    assert!(!frame.is_dragging());
}

/// 掴んだまま枠の外へ出ても離さないこと。
///
/// 離すと、少し外れた瞬間に窓が置き去りになります。
#[test]
fn dragging_keeps_going_outside_the_window() {
    let gpu = gpu!();
    let (mut draw_manager, mut frame) = scene(gpu);

    let (x, y) = on_the_bar();
    assert!(frame.mouse_left_pressed(&draw_manager, x, y));

    // 画面の外まで引く。
    assert!(frame.mouse_moved(&mut draw_manager, -200.0, -150.0));

    assert!(frame.is_dragging());
    assert!(frame.position()[0] < 0.0, "{:?}", frame.position());
}

/// 動かした先で掴み直せること。
///
/// 図形を動かしたのに当たり判定が元の場所のままだと、2 回目で掴めません。
#[test]
fn the_window_can_be_grabbed_again_where_it_landed() {
    let gpu = gpu!();
    let (mut draw_manager, mut frame) = scene(gpu);

    let (x, y) = on_the_bar();
    frame.mouse_left_pressed(&draw_manager, x, y);
    frame.mouse_moved(&mut draw_manager, x + 80.0, y + 50.0);
    frame.mouse_left_released();

    // `pick` は描く順を見るので、動かしたら通し直す。
    draw_manager
        .prepare(&gpu.device, &gpu.queue)
        .expect("支度に失敗した");

    // 元の場所では掴めない。
    assert!(!frame.mouse_left_pressed(&draw_manager, x, y), "元の場所で掴めている");

    // 動かした先では掴める。
    assert!(
        frame.mouse_left_pressed(&draw_manager, x + 80.0, y + 50.0),
        "動かした先で掴めない",
    );
}

/// `set_position` でも同じように動くこと。
#[test]
fn it_can_be_moved_without_a_finger() {
    let gpu = gpu!();
    let (mut draw_manager, mut frame) = scene(gpu);

    frame.set_position(&mut draw_manager, 20.0, 30.0);

    assert_eq!(frame.position(), [20.0, 30.0]);

    draw_manager
        .prepare(&gpu.device, &gpu.queue)
        .expect("支度に失敗した");

    // 置いた先で掴める。
    assert!(frame.mouse_left_pressed(&draw_manager, 20.0 + 150.0, 30.0 + 15.0));
}
