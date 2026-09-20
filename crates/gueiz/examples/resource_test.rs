//! リソース置き場が、フォント・絵・自前のエフェクトを実際に GPU まで届けるか。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 見るのは 4 つ。
//!
//! - 預けたフォントで文字が描けること
//! - 預けた絵が貼られること
//! - **番号を手で書かずに**自前のエフェクトが走ること
//! - 取っ手が死んだら安全に無効になること
//!
//! ```sh
//! cargo run -p gueiz --example resource_test
//! cargo run -p gueiz --example resource_test -- --window   # 結果を目で見る
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::camera::Camera;
use gueiz_2d::effect::EffectStage;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::resource::{FontHandle, Resources};
use gueiz_2d::sprite::{SpriteFilter, SpriteSheet};
use gueiz_2d::text::{TextRenderer, TextStyle};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;
const FONT_PATH: &str = "C:/Windows/Fonts/arial.ttf";

/// シート 1 層の大きさ。全面を緑にする。
const SHEET: u32 = 8;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("resource test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    let target = Target::new(&device);
    let mut preview = Preview::new("resource_test");
    let mut resources = Resources::new();

    // 1. フォントを預ける。バイト列は置き場が持つので、呼ぶ側は取っ手だけ。
    let arial = resources.load_font_file("arial", FONT_PATH)?;
    println!("1. フォントを預ける           取っ手 {arial:?}");
    assert!(resources.font(arial).is_some());
    assert_eq!(resources.font_handle("arial"), Some(arial));

    // 2. 預けたフォントで描ける。
    let pixels = draw_text(&device, &queue, &target, &resources, arial)?;
    preview.capture("預けたフォントで描く", SIZE, SIZE, &pixels);
    let lit = lit_pixels(&pixels);
    println!("2. 預けたフォントで描く       塗り {lit} 画素");
    assert!(lit > 100, "文字が出ていない");

    // 3. 絵を預けて貼る。
    let sheet = SpriteSheet::new(
        &device,
        &queue,
        SHEET,
        SHEET,
        &[&[0, 255, 0, 255].repeat((SHEET * SHEET) as usize)],
        SpriteFilter::Nearest,
    )?;
    let green = resources.add_sprite_sheet("green", sheet);
    assert_eq!(resources.sprite_sheet_handle("green"), Some(green));
    assert!(resources.sprite_sheet(green).is_some());

    let pixels = draw_sprite(&device, &queue, &target, &mut resources, green)?;
    preview.capture("預けた絵を貼る", SIZE, SIZE, &pixels);
    let middle = rgb(&pixels, SIZE / 2, SIZE / 2);
    println!("3. 預けた絵を貼る             中={middle:?}");
    assert_eq!(middle, [0, 255, 0], "緑の絵が出るはず");

    // 4 と 5 が本題。**番号を手で書かずに**自前のエフェクトを積む。
    //
    //    これまでは CUSTOM_KIND_BASE + n を自分で振って、Block::Custom と
    //    揃え続ける必要があった。ずれると別のエフェクトが走る。
    let redden = resources.add_effect(
        "redden",
        EffectStage::Color,
        "return color * vec4<f32>(1.0, block.params.x, block.params.x, 1.0);",
    );
    let darken = resources.add_effect(
        "darken",
        EffectStage::Color,
        "return vec4<f32>(color.rgb * block.params.x, color.a);",
    );

    println!(
        "4. 番号は置き場が振る         redden={} darken={}",
        resources.effect_kind(redden).expect("登録した"),
        resources.effect_kind(darken).expect("登録した"),
    );
    assert_ne!(
        resources.effect_kind(redden),
        resources.effect_kind(darken),
        "番号がかぶってはいけない",
    );

    // 白い四角に「緑と青を潰す」を掛ける → 赤。
    let pixels = draw_with_effect(&device, &queue, &target, &resources, redden, 0.0)?;
    preview.capture("自前のエフェクト（赤に）", SIZE, SIZE, &pixels);
    let middle = rgb(&pixels, SIZE / 2, SIZE / 2);
    println!("5. 自前のエフェクト           中={middle:?}");
    assert_eq!(middle, [255, 0, 0], "赤になるはず");

    // 6. 2 つめも、同じやり方で正しく走る。番号がずれていればここで出る。
    let pixels = draw_with_effect(&device, &queue, &target, &resources, darken, 0.5)?;
    preview.capture("2 つめのエフェクト（半分に）", SIZE, SIZE, &pixels);
    let middle = rgb(&pixels, SIZE / 2, SIZE / 2);
    println!("6. 2 つめのエフェクト         中={middle:?}");
    for channel in middle {
        assert!(channel.abs_diff(128) <= 2, "{channel} は 128 前後のはず");
    }

    // 7. 同じ名前で入れ替えると、古い取っ手は死ぬ。
    let replaced = resources.add_effect("redden", EffectStage::Color, "return color;");
    println!(
        "7. 名前で入れ替え             古い取っ手 {:?} / 新しい番号 {}",
        resources.effect_kind(redden),
        resources.effect_kind(replaced).expect("登録した"),
    );
    assert_eq!(resources.effect_kind(redden), None, "古い取っ手は死ぬ");
    assert_ne!(
        resources.effect_kind(replaced),
        Some(0),
        "新しいほうは生きている",
    );

    println!("\nOK: フォント・絵・自前のエフェクト、すべて取っ手で扱えている");

    preview.show()?;

    Ok(())
}

/// 預けたフォントで文字を描く。
fn draw_text(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    resources: &Resources,
    font: FontHandle,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut draw_manager =
        DrawManager::new(device, queue, FORMAT, &DrawManagerDescriptor::default())?;

    let font = resources.font(font).ok_or("フォントが居ない")?;

    let mut renderer = TextRenderer::new();
    renderer
        .camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32))
        .color(1.0, 1.0, 1.0, 1.0);
    renderer.write(&mut draw_manager, &font, "Res", &TextStyle::new(72.0), 16.0, 60.0)?;

    render(device, queue, target, &mut draw_manager)
}

/// 預けた絵を四角に貼る。
fn draw_sprite(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    resources: &mut Resources,
    sheet: gueiz_2d::resource::SpriteSheetHandle,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut draw_manager =
        DrawManager::new(device, queue, FORMAT, &DrawManagerDescriptor::default())?;

    assert!(
        resources.use_sprite_sheet(device, &mut draw_manager, sheet),
        "シートを差せなかった",
    );

    let mut square = white_square();
    square.sprite(0);
    square.instance(instance::create_instance());
    draw_manager.register(square);

    render(device, queue, target, &mut draw_manager)
}

/// 自前のエフェクトを 1 つ積んで描く。**番号は書かない。**
fn draw_with_effect(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    resources: &Resources,
    effect: gueiz_2d::resource::EffectHandle,
    param: f32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut draw_manager =
        DrawManager::new(device, queue, FORMAT, &DrawManagerDescriptor::default())?;

    // 預かっているエフェクトを全部差し込む。
    resources.apply_effects(device, &mut draw_manager)?;

    let mut square = white_square();
    square.effect(resources.effect_block(effect, [param, 0.0, 0.0, 0.0], [1.0; 4]));
    square.instance(instance::create_instance());
    draw_manager.register(square);

    render(device, queue, target, &mut draw_manager)
}

/// 画面の中ほどを覆う白い四角。
fn white_square() -> object::Object {
    let mut square = object::create_object("Square");
    square.begin(PaintType::Fill);
    for [x, y] in [[60.0, 60.0], [196.0, 60.0], [196.0, 196.0], [60.0, 196.0]] {
        square.put_vertex(Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0));
    }
    square.end();
    square.camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32));
    square
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

/// 描いて読み戻す。
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    draw_manager: &mut DrawManager,
) -> Result<Vec<u8>, Box<dyn Error>> {
    draw_manager.prepare(device, queue)?;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("resource test"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
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

    Ok(pixels)
}

/// その座標の色。読み戻したバイト列は BGRA の並び。
fn rgb(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let index = ((y * SIZE + x) * 4) as usize;
    [pixels[index + 2], pixels[index + 1], pixels[index]]
}

fn lit_pixels(pixels: &[u8]) -> usize {
    pixels.chunks_exact(4).filter(|pixel| pixel[0] > 127).count()
}
