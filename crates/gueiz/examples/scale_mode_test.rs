//! 窓の大きさが変わったとき、図形をどうするか。**描いた画素で**確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 基準 128x128 に赤い正方形（32x32）を置いて、**描き先を 256x128 に広げる**。
//! 合わせ方ごとに、正方形が何画素になるかを見る。
//!
//! ```sh
//! cargo run -p gueiz --example scale_mode_test
//! cargo run -p gueiz --example scale_mode_test -- --window
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::camera::{Camera, ScaleMode};
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

/// 絵を組むときの基準。
const DESIGN: f32 = 128.0;
/// 基準の中に置く正方形の一辺。
const SQUARE: f32 = 32.0;
/// 正方形の左上。**基準の真ん中**に置く。隅に置くと、
/// はみ出す合わせ方（`Fill`）で切り落とされて比べられない。
const ORIGIN: f32 = (DESIGN - SQUARE) / 2.0;

/// 描き先。基準より**横に 2 倍**広い。縦横比が合わないので違いが出る。
const WIDTH: u32 = 256;
const HEIGHT: u32 = 128;

const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("scale mode test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}", adapter.get_info().name);
    println!("基準 {DESIGN}x{DESIGN} の {SQUARE}x{SQUARE} を {WIDTH}x{HEIGHT} に描く\n");

    let target = Target::new(&device);
    let mut preview = Preview::new("scale_mode_test");

    // 1. 絶対。窓が広がっても図形は 1 単位 = 1 画素のまま。
    let fixed = render(&device, &queue, &target, ScaleMode::Fixed)?;
    let fixed_box = bounds(&fixed).expect("塗られている");
    preview.capture("1. Fixed（絶対）", WIDTH, HEIGHT, &fixed);
    println!(
        "1. Fixed    絶対    {}x{} 画素（基準のまま）",
        fixed_box.width(),
        fixed_box.height(),
    );
    assert_eq!(fixed_box.width(), SQUARE as u32, "画素の大きさが変わっている");
    assert_eq!(fixed_box.height(), SQUARE as u32);

    // 2. 引き伸ばし。基準を窓いっぱいに。横だけ 2 倍なので**歪む**。
    let stretch = render(&device, &queue, &target, ScaleMode::Stretch)?;
    let stretch_box = bounds(&stretch).expect("塗られている");
    preview.capture("2. Stretch（相対・歪む）", WIDTH, HEIGHT, &stretch);
    println!(
        "2. Stretch  相対    {}x{} 画素（横だけ 2 倍 = 歪む）",
        stretch_box.width(),
        stretch_box.height(),
    );
    assert_eq!(stretch_box.width(), SQUARE as u32 * 2);
    assert_eq!(stretch_box.height(), SQUARE as u32);

    // 3. 収める。縦横比を保つので、高さに合わせて等倍のまま。
    let fit = render(&device, &queue, &target, ScaleMode::Fit)?;
    let fit_box = bounds(&fit).expect("塗られている");
    preview.capture("3. Fit（相対・帯が出る）", WIDTH, HEIGHT, &fit);
    println!(
        "3. Fit      相対    {}x{} 画素（正方形のまま、左右に帯）",
        fit_box.width(),
        fit_box.height(),
    );
    assert_eq!(fit_box.width(), fit_box.height(), "縦横比が崩れている");
    assert_eq!(fit_box.width(), SQUARE as u32);
    // 基準が真ん中に来るので、左右に 64 画素ずつ帯が空く。
    assert!(column_is_empty(&fit, 10), "左の帯に何か描かれている");
    assert!(column_is_empty(&fit, WIDTH - 10), "右の帯に何か描かれている");
    assert!(fit_box.left >= 64, "帯のぶん右に寄るはず: {}", fit_box.left);

    // 4. 埋める。縦横比を保ったまま幅を使い切るので、2 倍になって縦は溢れる。
    //    基準の上下の端は画面の外へ出る。
    let fill = render(&device, &queue, &target, ScaleMode::Fill)?;
    let fill_box = bounds(&fill).expect("塗られている");
    preview.capture("4. Fill（相対・はみ出す）", WIDTH, HEIGHT, &fill);
    println!(
        "4. Fill     相対    {}x{} 画素（正方形のまま 2 倍、縦ははみ出す）",
        fill_box.width(),
        fill_box.height(),
    );
    assert_eq!(fill_box.width(), SQUARE as u32 * 2);
    assert_eq!(fill_box.height(), SQUARE as u32 * 2);
    // 帯は出ない。基準の左上は画面の上に追い出されている。
    assert!(camera_for(ScaleMode::Fill).world_to_screen(0.0, 0.0)[1] < 0.0);

    // 5. 合わせ先を選ぶ。中間なら、収めると埋めるのあいだに入る。
    let matched = render(&device, &queue, &target, ScaleMode::Match(0.5))?;
    let match_box = bounds(&matched).expect("塗られている");
    preview.capture("5. Match(0.5)（相対・中間）", WIDTH, HEIGHT, &matched);
    println!(
        "5. Match    相対    {}x{} 画素（{}..{} のあいだ）",
        match_box.width(),
        match_box.height(),
        fit_box.width(),
        fill_box.width(),
    );
    assert_eq!(match_box.width(), match_box.height(), "縦横比が崩れている");
    assert!(match_box.width() > fit_box.width());
    assert!(match_box.width() < fill_box.width());

    // 6. 絶対と相対で、押した場所の戻し方が違うこと。
    //    ここを通さないと、相対にしたとたん当たり判定がずれる。
    let center = [WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0];

    for mode in [ScaleMode::Fixed, ScaleMode::Stretch, ScaleMode::Fit, ScaleMode::Fill] {
        let camera = camera_for(mode);
        let [x, y] = camera.screen_to_world(center[0], center[1]).expect("戻せる");
        let [back_x, back_y] = camera.world_to_screen(x, y);

        println!(
            "6. {:<11} 窓の中心 ({}, {}) → 座標 ({x:.1}, {y:.1})",
            format!("{mode:?}"),
            center[0],
            center[1],
        );

        assert!(
            (back_x - center[0]).abs() < 0.1 && (back_y - center[1]).abs() < 0.1,
            "{mode:?}: 戻して往復しない",
        );
    }

    // 絶対なら画素位置がそのまま座標。相対ならずれる。
    let absolute = camera_for(ScaleMode::Fixed)
        .screen_to_world(center[0], center[1])
        .expect("戻せる");
    let relative = camera_for(ScaleMode::Stretch)
        .screen_to_world(center[0], center[1])
        .expect("戻せる");

    assert_eq!(absolute, center, "絶対なら素通りのはず");
    assert_ne!(relative, center, "相対でずれないなら、切り替わっていない");

    println!("\nすべて通りました。");

    if common::preview::window_requested() {
        preview.show()?;
    } else {
        println!("`-- --window` を付けると描いた結果を窓に出します。");
    }

    Ok(())
}

/// 基準で組んで、描き先の大きさに合わせたカメラ。
fn camera_for(scale_mode: ScaleMode) -> Camera {
    let mut camera = Camera::orthographic_2d(DESIGN, DESIGN).with_scale_mode(scale_mode);
    camera.resize(WIDTH as f32, HEIGHT as f32);

    camera
}

/// 基準の真ん中に正方形を 1 つ置いて描く。
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    scale_mode: ScaleMode,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut draw_manager =
        DrawManager::new(device, queue, FORMAT, &DrawManagerDescriptor::default())?;

    let mut square = object::create_object("Square");
    square.begin(PaintType::Fill);

    for (x, y) in [(0.0, 0.0), (SQUARE, 0.0), (SQUARE, SQUARE), (0.0, SQUARE)] {
        square.put_vertex(Vertex::new_position_color(
            ORIGIN + x,
            ORIGIN + y,
            0.0,
            1.0,
            0.0,
            0.0,
            1.0,
        ));
    }

    square.end();
    square.camera(camera_for(scale_mode));
    square.instance(instance::create_instance());

    draw_manager.register(square);
    draw_manager.prepare(device, queue)?;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("scale mode"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("square"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        draw_manager.draw(&mut render_pass);
    }

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &target.readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    target.readback.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let view = target.readback.get_mapped_range(..)?;
    let pixels = view.to_vec();
    drop(view);
    target.readback.unmap();

    Ok(pixels)
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

impl Target {
    fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT.into(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            texture,
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: (WIDTH * HEIGHT * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        }
    }
}

struct Bounds {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

impl Bounds {
    fn width(&self) -> u32 {
        self.right - self.left + 1
    }

    fn height(&self) -> u32 {
        self.bottom - self.top + 1
    }
}

/// その列に赤が 1 つも無いか。帯が空いているかを見る。
fn column_is_empty(pixels: &[u8], x: u32) -> bool {
    (0..HEIGHT).all(|y| pixels[((y * WIDTH + x) * 4) as usize + 2] <= 16)
}

/// 赤く塗られた画素の外枠。はみ出したぶんは切れているので入らない。
fn bounds(pixels: &[u8]) -> Option<Bounds> {
    let mut found: Option<Bounds> = None;

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            // BGRA で並んでいる。赤だけ見る。
            if pixels[((y * WIDTH + x) * 4) as usize + 2] <= 16 {
                continue;
            }

            found = Some(match found {
                None => Bounds {
                    left: x,
                    right: x,
                    top: y,
                    bottom: y,
                },
                Some(box_) => Bounds {
                    left: box_.left.min(x),
                    right: box_.right.max(x),
                    top: box_.top.min(y),
                    bottom: box_.bottom.max(y),
                },
            });
        }
    }

    found
}
