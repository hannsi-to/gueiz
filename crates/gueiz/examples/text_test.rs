//! 文字が描けているかを、**描いた画素を読み戻して**確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 見るのは 4 つ。
//!
//! - グリフが実際に塗られること
//! - **穴が抜けること**（`o` の中は背景のまま）
//! - **離れた輪郭が両方出ること**（`i` の点と棒）
//! - 同じ文字は形を 1 つしか登録しないこと
//! - **マルチサンプルで縁が均されること**
//!
//! ```sh
//! cargo run -p gueiz --example text_test
//! cargo run -p gueiz --example text_test -- --window   # 結果を目で見る
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::camera::Camera;
use gueiz_2d::font::Font;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::text::{TextRenderer, TextStyle, layout};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::msaa::{MultisampleTarget, DEFAULT_MULTISAMPLE, NO_MULTISAMPLE};
use gueiz_2d::wgpu;

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

/// 手元にあるフォント。無ければ分かるように落とす。
const FONT_PATH: &str = "C:/Windows/Fonts/arial.ttf";

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let data = std::fs::read(FONT_PATH)
        .map_err(|error| format!("{FONT_PATH} を読めませんでした: {error}"))?;
    let font = Font::from_bytes(&data)?;

    println!("font   : {FONT_PATH}");
    println!(
        "metrics: ascender {:.3} em, descender {:.3} em, line {:.3} em\n",
        font.ascender(),
        font.descender(),
        font.line_height(),
    );

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("text test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    let target = Target::new(&device);
    let mut preview = Preview::new("text_test");

    // 1. 並べ方。GPU を触らずに計算だけ確かめる。
    let style = TextStyle::new(64.0);
    let placed = layout(&font, "Hi", &style);
    println!(
        "1. 並べ方                   {} 文字、幅 {:.1} px、高さ {:.1} px",
        placed.glyphs.len(),
        placed.width,
        placed.height,
    );
    assert_eq!(placed.glyphs.len(), 2);
    assert!(placed.glyphs[0].x == 0.0, "1 文字目は左端");
    assert!(placed.glyphs[1].x > 0.0, "2 文字目は右にずれる");
    assert!(placed.width > 0.0 && placed.height > 0.0);

    // 2. 改行で行が下がる。
    let two_lines = layout(&font, "Hi\nHi", &style);
    println!(
        "2. 改行                     高さ {:.1} px（1 行は {:.1} px）",
        two_lines.height, placed.height,
    );
    assert_eq!(two_lines.glyphs.len(), 4);
    assert_eq!(two_lines.glyphs[2].x, 0.0, "2 行目は左端に戻る");
    assert!(two_lines.glyphs[2].y > two_lines.glyphs[0].y, "2 行目は下");
    assert!(two_lines.height > placed.height);

    // 3. 大きな `o` を描いて、**穴が抜けている**ことを見る。
    //    ここが抜けないと、テッセレータが輪郭の包含関係を取り違えている。
    let (pixels, renderer) = render(&device, &queue, &target, &font, "o", 180.0, 20.0, 10.0)?;
    preview.capture("`o` の穴", SIZE, SIZE, &pixels);
    let box_o = lit_bounds(&pixels).expect("`o` が 1 画素も塗られていない");
    // 上下の真ん中なら、輪の左右の縁を横切る。
    let middle = (box_o.top + box_o.bottom) / 2;
    let ring = scan_row(&pixels, middle);
    println!(
        "3. `o` の穴                 横 {middle} 行: 塗り {} 画素、区間 {}",
        ring.filled, ring.runs,
    );
    assert_eq!(ring.runs, 2, "左右の縁で 2 区間になるはず（穴が抜けている）");
    assert_eq!(renderer.shape_count(), 1);

    // 4. `i` は**離れた輪郭が 2 つ**（点と棒）。片方が穴にされると消える。
    let (pixels, _) = render(&device, &queue, &target, &font, "i", 180.0, 90.0, 10.0)?;
    preview.capture("`i` の点と棒", SIZE, SIZE, &pixels);
    // いちばん塗られている列 = 縦棒。そこを縦に見れば、点と棒で 2 区間。
    let stem = densest_column(&pixels).expect("`i` が 1 画素も塗られていない");
    let column = scan_column(&pixels, stem);
    println!(
        "4. `i` の点と棒             縦 {stem} 列: 塗り {} 画素、区間 {}",
        column.filled, column.runs,
    );
    assert!(column.filled > 0, "`i` が塗られていない");
    assert_eq!(column.runs, 2, "点と棒で 2 区間になるはず");

    // 5. 同じ文字は形を 1 つだけ。`lll` でも形は 1 つ、インスタンスが 3 つ。
    let (pixels, renderer) = render(&device, &queue, &target, &font, "lll", 100.0, 20.0, 40.0)?;
    preview.capture("`lll`（形は 1 つ）", SIZE, SIZE, &pixels);
    println!("5. `lll` の形の数           {}", renderer.shape_count());
    assert_eq!(renderer.shape_count(), 1, "同じ文字は形を共有するはず");

    // 6. 違う文字は違う形。
    let (pixels, renderer) = render(&device, &queue, &target, &font, "abc", 100.0, 20.0, 40.0)?;
    preview.capture("`abc`（形は 3 つ）", SIZE, SIZE, &pixels);
    println!("6. `abc` の形の数           {}", renderer.shape_count());
    assert_eq!(renderer.shape_count(), 3);

    // 7 と 8 が本題。**マルチサンプルで縁が均される**こと。
    //
    // 均していないと、縁の画素は塗るか塗らないかの二択で中間の明るさが出ない。
    // 均すと 4 点の平均になり、0 と 255 の間の値が現れる。
    let (plain, _) =
        render_with_samples(&device, &queue, &target, &font, "O", 200.0, 30.0, 10.0, NO_MULTISAMPLE)?;
    preview.capture("`O` 均さない", SIZE, SIZE, &plain);
    let plain_edges = intermediate_pixels(&plain);
    println!("7. 均さない                  中間の明るさ {plain_edges} 画素");
    assert_eq!(plain_edges, 0, "均していなければ中間は出ないはず");

    let (smooth, _) = render_with_samples(
        &device, &queue, &target, &font, "O", 200.0, 30.0, 10.0, DEFAULT_MULTISAMPLE,
    )?;
    preview.capture("`O` 4 点で均す", SIZE, SIZE, &smooth);
    let smooth_edges = intermediate_pixels(&smooth);
    println!("8. 4 点で均す                中間の明るさ {smooth_edges} 画素");
    assert!(
        smooth_edges > 200,
        "縁が均されていない（中間 {smooth_edges} 画素）",
    );

    // 塗り面積そのものは変わらない。均すのは縁だけ。
    let plain_lit = lit_pixels(&plain);
    let smooth_lit = lit_pixels(&smooth);
    println!("   塗り面積                 {plain_lit} → {smooth_lit} 画素");
    assert!(
        (plain_lit as i32 - smooth_lit as i32).unsigned_abs() < plain_lit as u32 / 20,
        "均しただけで形が変わってはいけない",
    );

    // 目で見るためだけの 1 枚。改行とカーニングの様子が分かる。
    let (pixels, _) = render(&device, &queue, &target, &font, "Hello\nWorld", 56.0, 12.0, 20.0)?;
    preview.capture("Hello / World", SIZE, SIZE, &pixels);

    println!("\nOK: グリフ・穴・離れた輪郭・形の共有・縁の均し、すべて期待どおり");

    preview.show()?;

    Ok(())
}

/// 1 本の走査線の塗り具合。
struct Scan {
    /// 塗られた画素の数。
    filled: usize,
    /// 塗られた区間の数。穴が抜けていれば 2 以上になる。
    runs: usize,
}

fn scan_row(pixels: &[u8], y: u32) -> Scan {
    scan((0..SIZE).map(|x| lit(pixels, x, y)))
}

fn scan_column(pixels: &[u8], x: u32) -> Scan {
    scan((0..SIZE).map(|y| lit(pixels, x, y)))
}

fn scan(samples: impl Iterator<Item = bool>) -> Scan {
    let mut filled = 0;
    let mut runs = 0;
    let mut previous = false;

    for sample in samples {
        if sample {
            filled += 1;

            if !previous {
                runs += 1;
            }
        }

        previous = sample;
    }

    Scan { filled, runs }
}

/// 中間の明るさを持つ画素の数。均されていれば増える。
fn intermediate_pixels(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 8 && pixel[0] < 247)
        .count()
}

/// 半分より明るい画素の数。だいたいの塗り面積。
fn lit_pixels(pixels: &[u8]) -> usize {
    pixels.chunks_exact(4).filter(|pixel| pixel[0] > 127).count()
}

/// 塗られている範囲。
struct Bounds {
    top: u32,
    bottom: u32,
}

/// 塗られた画素を囲む上下の範囲。1 画素も無ければ `None`。
fn lit_bounds(pixels: &[u8]) -> Option<Bounds> {
    let mut top = None;
    let mut bottom = 0;

    for y in 0..SIZE {
        if (0..SIZE).any(|x| lit(pixels, x, y)) {
            top.get_or_insert(y);
            bottom = y;
        }
    }

    top.map(|top| Bounds { top, bottom })
}

/// いちばん塗られている列。縦棒を探すのに使う。
fn densest_column(pixels: &[u8]) -> Option<u32> {
    (0..SIZE)
        .map(|x| (x, scan_column(pixels, x).filled))
        .filter(|(_, filled)| *filled > 0)
        .max_by_key(|(_, filled)| *filled)
        .map(|(x, _)| x)
}

/// その画素が塗られているか。背景は黒、文字は白。
fn lit(pixels: &[u8], x: u32, y: u32) -> bool {
    let index = ((y * SIZE + x) * 4) as usize;
    pixels[index] > 127
}

struct Target {
    view: wgpu::TextureView,
    texture: wgpu::Texture,
    readback: wgpu::Buffer,
}

impl Target {
    fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
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

        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            texture,
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: (SIZE * SIZE * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        }
    }
}

/// 文字列を描いて、画面と使った [`TextRenderer`] を返す。
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    font: &Font,
    text: &str,
    size: f32,
    x: f32,
    y: f32,
) -> Result<(Vec<u8>, TextRenderer), Box<dyn Error>> {
    render_with_samples(device, queue, target, font, text, size, x, y, NO_MULTISAMPLE)
}

/// 均す点の数を指定して描く。
#[allow(clippy::too_many_arguments)]
fn render_with_samples(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    font: &Font,
    text: &str,
    size: f32,
    x: f32,
    y: f32,
    sample_count: u32,
) -> Result<(Vec<u8>, TextRenderer), Box<dyn Error>> {
    let mut draw_manager = DrawManager::new(
        device,
        queue,
        FORMAT,
        &DrawManagerDescriptor {
            sample_count,
            ..Default::default()
        },
    )?;

    let mut renderer = TextRenderer::new();
    renderer
        .camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32))
        .color(1.0, 1.0, 1.0, 1.0);

    let style = TextStyle::new(size);
    renderer.write(&mut draw_manager, font, text, &style, x, y)?;

    draw_manager.prepare(device, queue)?;

    // 均すときは多点の描き先に描いて、読み戻す側へ書き出す。
    let mut multisample = MultisampleTarget::new(sample_count);
    multisample.ensure(device, SIZE, SIZE, FORMAT.into());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("text test"),
    });

    {
        let attachment =
            multisample.color_attachment(&target.view, wgpu::LoadOp::Clear(wgpu::Color::BLACK));

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("text"),
            color_attachments: &[Some(attachment)],
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

    queue.submit(Some(encoder.finish()));

    target.readback.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let view = target.readback.get_mapped_range(..)?;
    let pixels = view.to_vec();
    drop(view);
    target.readback.unmap();

    Ok((pixels, renderer))
}
