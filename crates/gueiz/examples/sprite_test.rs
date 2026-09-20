//! スプライトの絵と Z の重なり順を、**描いた画素を読み戻して**確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 見るのは 4 つ。
//!
//! - シートの層ごとに違う絵が出ること
//! - 切り出し範囲でコマを選べること
//! - **Z の大きいほうが手前に描かれること**
//! - 絵を貼っていない図形が巻き込まれないこと
//!
//! ```sh
//! cargo run -p gueiz --example sprite_test
//! cargo run -p gueiz --example sprite_test -- --window   # 結果を目で見る
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::camera::Camera;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::sprite::{SpriteFilter, SpriteSheet};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const SIZE: u32 = 256;

/// sRGB を通さない形式。読み戻した値がそのまま絵の値になる。
const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

/// シート 1 層の大きさ。左半分と右半分で色を変えてある。
const SHEET: u32 = 16;

/// 四角を置く場所と大きさ。
const CENTRE: f32 = 128.0;
const HALF: f32 = 40.0;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("sprite test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    // 層 0 = 左半分が赤・右半分が緑、層 1 = 全面が青。
    let sheet = SpriteSheet::new(
        &device,
        &queue,
        SHEET,
        SHEET,
        &[&split_layer(), &solid_layer([0, 0, 255, 255])],
        SpriteFilter::Nearest,
    )?;

    assert_eq!(sheet.layer_count(), 2);

    let target = Target::new(&device);
    let mut preview = Preview::new("sprite_test");

    // 1. 層 0 をまるごと。四角の左が赤、右が緑。
    let pixels = draw(&device, &queue, &target, &[Sprite::layer(0, 0.0)])?;
    preview.capture("層 0 をまるごと", SIZE, SIZE, &pixels);
    let (left, right) = (rgb(&pixels, 100, 128), rgb(&pixels, 156, 128));
    println!("1. 層 0 をまるごと            左={left:?} 右={right:?}");
    assert_eq!(left, [255, 0, 0], "左半分は赤");
    assert_eq!(right, [0, 255, 0], "右半分は緑");

    // 2. 層 1。全面が青。
    let pixels = draw(&device, &queue, &target, &[Sprite::layer(1, 0.0)])?;
    preview.capture("層 1（全面が青）", SIZE, SIZE, &pixels);
    let centre = rgb(&pixels, 128, 128);
    println!("2. 層 1                      中={centre:?}");
    assert_eq!(centre, [0, 0, 255], "層 1 は青");

    // 3. 切り出し。層 0 の左半分だけを引き伸ばすと、四角全体が赤になる。
    //    形は変えずに UV だけ動かしているので、コマ送りはこれで済む。
    let left_half = sheet.region(0, 0, SHEET / 2, SHEET);
    let pixels = draw(&device, &queue, &target, &[Sprite::region(0, left_half, 0.0)])?;
    preview.capture("層 0 の左半分を切り出し", SIZE, SIZE, &pixels);
    let (left, right) = (rgb(&pixels, 100, 128), rgb(&pixels, 156, 128));
    println!("3. 層 0 の左半分を切り出し     左={left:?} 右={right:?}");
    assert_eq!(left, [255, 0, 0], "切り出した赤が全体に伸びる");
    assert_eq!(right, [255, 0, 0], "右も赤になる");

    // 4 と 5 が本題。同じ 2 枚を、z だけ入れ替える。
    //    青（層 1）と赤緑（層 0）を重ねて、手前に来たほうが見える。
    let pixels = draw(
        &device,
        &queue,
        &target,
        &[Sprite::layer(0, 0.0), Sprite::layer(1, 1.0)],
    )?;
    preview.capture("層 1 を手前（z=1）", SIZE, SIZE, &pixels);
    let blue_on_top = rgb(&pixels, 100, 128);
    println!("4. 層 1 を手前（z=1）         中={blue_on_top:?}");
    assert_eq!(blue_on_top, [0, 0, 255], "青が上に乗るはず");

    let pixels = draw(
        &device,
        &queue,
        &target,
        &[Sprite::layer(0, 1.0), Sprite::layer(1, 0.0)],
    )?;
    preview.capture("層 0 を手前（z=1）", SIZE, SIZE, &pixels);
    let red_on_top = rgb(&pixels, 100, 128);
    println!("5. 層 0 を手前（z=1）         中={red_on_top:?}");
    assert_eq!(red_on_top, [255, 0, 0], "赤が上に乗るはず");

    assert_ne!(blue_on_top, red_on_top, "z を入れ替えたら結果も変わるはず");

    // 6. 登録順は z に勝たない。先に登録したほうを奥にしても、z が手前なら上。
    let pixels = draw(
        &device,
        &queue,
        &target,
        &[Sprite::layer(1, 5.0), Sprite::layer(0, -5.0)],
    )?;
    let by_z = rgb(&pixels, 100, 128);
    println!("6. 登録順と z が逆            中={by_z:?}");
    assert_eq!(by_z, [0, 0, 255], "登録順ではなく z で決まるはず");

    // 7. 絵を貼っていない図形は、シートを差してあっても頂点色のまま。
    let pixels = draw(&device, &queue, &target, &[Sprite::none(0.0)])?;
    let plain = rgb(&pixels, 128, 128);
    println!("7. 絵を貼らない               中={plain:?}");
    assert_eq!(plain, [255, 255, 0], "頂点色（黄）がそのまま出るはず");

    // 8. 頂点色は絵に掛かる。白い絵に色を付けたいときはこれを使う。
    //    層 0 の左半分（赤）に緑を掛けると、どちらも残らず黒になる。
    let pixels = draw(
        &device,
        &queue,
        &target,
        &[Sprite::tinted(0, [0.0, 1.0, 0.0, 1.0], 0.0)],
    )?;
    let (left, right) = (rgb(&pixels, 100, 128), rgb(&pixels, 156, 128));
    println!("8. 頂点色を掛ける（緑）        左={left:?} 右={right:?}");
    assert_eq!(left, [0, 0, 0], "赤 x 緑 = 黒");
    assert_eq!(right, [0, 255, 0], "緑 x 緑 = 緑");

    println!("\nOK: 層・切り出し・Z の重なり順、すべて期待どおり");

    preview.show()?;

    Ok(())
}

/// 描く四角 1 枚ぶんの指定。
struct Sprite {
    layer: Option<u32>,
    region: [f32; 4],
    z: f32,
    /// 頂点色。絵にはこれが掛かるので、絵を見たいときは白にしておく。
    color: [f32; 4],
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// 絵を貼っていないことが分かる色。絵に掛けると濁るので、そちらには使わない。
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

impl Sprite {
    fn layer(layer: u32, z: f32) -> Self {
        Self {
            layer: Some(layer),
            region: gueiz_2d::sprite::WHOLE_LAYER,
            z,
            color: WHITE,
        }
    }

    fn region(layer: u32, region: [f32; 4], z: f32) -> Self {
        Self {
            layer: Some(layer),
            region,
            z,
            color: WHITE,
        }
    }

    fn none(z: f32) -> Self {
        Self {
            layer: None,
            region: gueiz_2d::sprite::WHOLE_LAYER,
            z,
            color: YELLOW,
        }
    }

    /// 頂点色が絵に掛かることを見るための指定。
    fn tinted(layer: u32, color: [f32; 4], z: f32) -> Self {
        Self {
            layer: Some(layer),
            region: gueiz_2d::sprite::WHOLE_LAYER,
            z,
            color,
        }
    }
}

/// 左半分が赤、右半分が緑の層。
fn split_layer() -> Vec<u8> {
    let mut pixels = Vec::with_capacity((SHEET * SHEET * 4) as usize);

    for _ in 0..SHEET {
        for x in 0..SHEET {
            if x < SHEET / 2 {
                pixels.extend_from_slice(&[255, 0, 0, 255]);
            } else {
                pixels.extend_from_slice(&[0, 255, 0, 255]);
            }
        }
    }

    pixels
}

fn solid_layer(color: [u8; 4]) -> Vec<u8> {
    color.repeat((SHEET * SHEET) as usize)
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
            // 幅 256 x 4 = 1024 で、要求される 256 の倍数。
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: (SIZE * SIZE * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        }
    }
}

/// 指定ぶんの四角を描いて、画面を読み戻す。
fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    sprites: &[Sprite],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut draw_manager =
        DrawManager::new(device, queue, FORMAT, &DrawManagerDescriptor::default())?;

    // 毎回シートを作り直す。差し替えが効いていることの確認も兼ねる。
    draw_manager.set_sprite_sheet(
        device,
        SpriteSheet::new(
            device,
            queue,
            SHEET,
            SHEET,
            &[&split_layer(), &solid_layer([0, 0, 255, 255])],
            SpriteFilter::Nearest,
        )?,
    );

    for sprite in sprites {
        let mut square = object::create_object("Sprite");
        square.begin(PaintType::Fill);
        for [x, y] in [[-HALF, -HALF], [HALF, -HALF], [HALF, HALF], [-HALF, HALF]] {
            let [r, g, b, a] = sprite.color;
            square.put_vertex(Vertex::new_position_color(x, y, 0.0, r, g, b, a));
        }
        square.end();
        square.camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32));
        square.translate(CENTRE, CENTRE, sprite.z);

        match sprite.layer {
            Some(layer) => {
                square.sprite_region(layer, sprite.region);
            }
            None => {
                square.no_sprite();
            }
        }

        square.instance(instance::create_instance());
        draw_manager.register(square);
    }

    draw_manager.prepare(device, queue)?;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sprite test"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sprites"),
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
