//! エフェクトが効いているか、**描いた画素を読み戻して**確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 見るのは 3 つ。
//!
//! - Color 段の山が色に効くこと
//! - Transform 段の山が位置に効くこと
//! - **同じ段の中では、積んだ順が結果を変えること**
//!
//! ```sh
//! cargo run -p gueiz --example effect_test
//! cargo run -p gueiz --example effect_test -- --window   # 結果を目で見る
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::camera::Camera;
use gueiz_2d::effect::{Block, CustomBlock, EffectStack, EffectStage, CUSTOM_KIND_BASE};
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const SIZE: u32 = 512;
const SURFACE_FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

/// 図形を置く場所。ここの画素を見る。
const HOME: (f32, f32) = (128.0, 128.0);
/// Orbit で動かす距離。
const ORBIT: f32 = 200.0;

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];
const BLACK: [u8; 3] = [0, 0, 0];

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("effect test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    let target = Target::new(&device);
    let mut preview = Preview::new("effect_test");

    // 1. エフェクト無し。白い四角がそのまま出る。
    let pixels = render(&device, &queue, &target, EffectStack::new())?;
    preview.capture("エフェクト無し", SIZE, SIZE, &pixels);
    let home = pixel(&pixels, HOME);
    println!("1. エフェクト無し              {home:?}");
    assert_eq!(home, WHITE, "白い四角がそのまま出るはず");

    // 2. Color 段。白に赤を掛けたら赤。
    let mut effects = EffectStack::new();
    effects.push(Block::Tint { color: [1.0, 0.0, 0.0, 1.0] });
    let pixels = render(&device, &queue, &target, effects)?;
    preview.capture("Tint(赤)", SIZE, SIZE, &pixels);
    let home = pixel(&pixels, HOME);
    println!("2. Tint(赤)                  {home:?}");
    assert_eq!(home, RED, "赤くなるはず");

    // 3. Transform 段。時刻 0・0 番目の複製は、Orbit が +X にまるごとずらす。
    let mut effects = EffectStack::new();
    effects.push(Block::Orbit { radius: ORBIT, speed: 0.0 });
    let pixels = render(&device, &queue, &target, effects)?;
    preview.capture("Orbit(+200)", SIZE, SIZE, &pixels);
    let home = pixel(&pixels, HOME);
    let moved = pixel(&pixels, (HOME.0 + ORBIT, HOME.1));
    println!("3. Orbit(+{ORBIT})              元={home:?} 先={moved:?}");
    assert_eq!(home, BLACK, "元の場所からは消えるはず");
    assert_eq!(moved, WHITE, "{ORBIT} だけずれた場所に出るはず");

    // 4 と 5 が本題。同じ 2 つの山を、順番だけ変える。
    //
    // Dissolve は溶け際の色で**塗り替える**ので、掛け算と違って順番が効く。
    // threshold 0 / edge 1 にすると全画素が溶け際に入り、ノイズに依らず決まる。
    let dissolve = Block::Dissolve {
        threshold: 0.0,
        edge: 1.0,
        edge_color: [1.0, 1.0, 1.0, 1.0],
    };
    let tint = Block::Tint { color: [1.0, 0.0, 0.0, 1.0] };

    // 4. 先に白で塗り替えてから、赤を掛ける → 赤。
    let mut effects = EffectStack::new();
    effects.push(dissolve);
    effects.push(tint);
    let pixels = render(&device, &queue, &target, effects)?;
    preview.capture("[Dissolve, Tint]", SIZE, SIZE, &pixels);
    let first = pixel(&pixels, HOME);
    println!("4. [Dissolve, Tint]          {first:?}");
    assert_eq!(first, RED, "白に塗り替えてから赤を掛けるので赤");

    // 5. 先に赤を掛けてから、白で塗り替える → 白。
    let mut effects = EffectStack::new();
    effects.push(tint);
    effects.push(dissolve);
    let pixels = render(&device, &queue, &target, effects)?;
    preview.capture("[Tint, Dissolve]", SIZE, SIZE, &pixels);
    let second = pixel(&pixels, HOME);
    println!("5. [Tint, Dissolve]          {second:?}");
    assert_eq!(second, WHITE, "赤を掛けた後に白で塗り替えるので白");

    assert_ne!(first, second, "順番を変えたら結果も変わるはず");

    // 6. 乗算済みアルファ。シェーダの出力とブレンド設定が噛み合っているか。
    //
    // 半分だけ透けた白を黒に重ねる。線形で 0.5 になり、sRGB に直すと 182 前後。
    // どちらか片方だけ乗算済みだと 255（掛け忘れ）や 128 付近（二重掛け）に化ける。
    let mut effects = EffectStack::new();
    effects.push(Block::Tint { color: [1.0, 1.0, 1.0, 0.5] });
    let pixels = render(&device, &queue, &target, effects)?;
    preview.capture("半透明（乗算済み）", SIZE, SIZE, &pixels);
    let blended = pixel(&pixels, HOME);
    println!("6. 半透明（乗算済み）          {blended:?}");

    let expected = srgb(0.5);
    for channel in blended {
        assert!(
            channel.abs_diff(expected) <= 2,
            "{channel} が期待値 {expected} から離れている（乗算済みが噛み合っていない）",
        );
    }

    // 7. 自前の山。差し込んだ WGSL がそのまま走る。
    //    ここでは params.x に入れた値で青成分を潰す。
    let custom = CustomBlock {
        kind: CUSTOM_KIND_BASE,
        stage: EffectStage::Color,
        body: String::from("return color * vec4<f32>(1.0, 1.0, block.params.x, 1.0);"),
    };

    let mut effects = EffectStack::new();
    effects.push(Block::Custom {
        stage: EffectStage::Color,
        kind: CUSTOM_KIND_BASE,
        params: [0.0, 0.0, 0.0, 0.0],
        color: [0.0; 4],
    });

    let pixels = render_with(&device, &queue, &target, effects, &[custom])?;
    preview.capture("自前の山（青を潰す）", SIZE, SIZE, &pixels);
    let home = pixel(&pixels, HOME);
    println!("7. 自前の山（青を潰す）        {home:?}");
    assert_eq!(home, [255, 255, 0], "青だけ落ちて黄色になるはず");

    // 8. 通らない WGSL は、パニックではなく Err で返る。
    let broken = CustomBlock {
        kind: CUSTOM_KIND_BASE,
        stage: EffectStage::Color,
        body: String::from("this is not wgsl"),
    };
    let outcome = render_with(&device, &queue, &target, EffectStack::new(), &[broken]);
    println!(
        "8. 壊れた WGSL               {}",
        if outcome.is_err() { "Err で返った" } else { "通ってしまった" },
    );
    assert!(outcome.is_err(), "壊れた WGSL は Err になるはず");

    // 9. 用意された山の番号は使えない。
    let reserved = CustomBlock {
        kind: 1,
        stage: EffectStage::Color,
        body: String::from("return color;"),
    };
    let outcome = render_with(&device, &queue, &target, EffectStack::new(), &[reserved]);
    println!(
        "9. 予約済みの番号             {}",
        if outcome.is_err() { "Err で返った" } else { "通ってしまった" },
    );
    assert!(outcome.is_err(), "予約済みの番号は Err になるはず");

    println!("\nOK: 段・順番・アルファ・自前の山、すべて期待どおり");

    preview.show()?;

    Ok(())
}

/// 描き先と読み戻し用のバッファ。
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
            format: SURFACE_FORMAT.into(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 幅 512 x 4 バイト = 2048 で、要求される 256 の倍数になっている。
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (SIZE * SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            view,
            texture,
            readback,
        }
    }
}

/// 40x40 の白い四角を 1 つ描いて、画面を読み戻す。
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    effects: EffectStack,
) -> Result<Vec<u8>, Box<dyn Error>> {
    render_with(device, queue, target, effects, &[])
}

/// 自前の山を差し込んでから描く。
fn render_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    effects: EffectStack,
    custom: &[CustomBlock],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut draw_manager = DrawManager::new(
        device,
        queue,
        SURFACE_FORMAT,
        &DrawManagerDescriptor::default(),
    )?;

    if !custom.is_empty() {
        draw_manager.set_custom_blocks(device, custom)?;
    }

    let mut square = object::create_object("Square");
    square.begin(PaintType::Fill);
    for [x, y] in [[-20.0, -20.0], [20.0, -20.0], [20.0, 20.0], [-20.0, 20.0]] {
        square.put_vertex(Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0));
    }
    square.end();
    square.camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32));
    square.set_effects(effects);
    square.instance(instance::create_instance().translate(HOME.0, HOME.1, 0.0));

    draw_manager.register(square);
    draw_manager.set_time(0.0);
    draw_manager.prepare(device, queue)?;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("effect test"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("effect pass"),
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

/// 線形の値を sRGB の 8 ビットに直す。読み戻した画素と比べるため。
fn srgb(linear: f32) -> u8 {
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };

    (encoded * 255.0).round() as u8
}

/// その座標の色を RGB で。読み戻したバイト列は BGRA の並び。
fn pixel(pixels: &[u8], (x, y): (f32, f32)) -> [u8; 3] {
    let index = ((y as u32 * SIZE + x as u32) * 4) as usize;
    [pixels[index + 2], pixels[index + 1], pixels[index]]
}
