//! 3D の描画を、**描いた画素と読み戻した引数で**確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 見るのは、2D と本当に違う 3 点。
//!
//! - **深度バッファ**が奥行きを解くこと（2D は描く順で解いている）
//! - **錐台カリング**が視野の外を捨てること（2D はクリップ矩形）
//! - 法線を使った明るさが出ること（2D には法線が無い）
//!
//! ```sh
//! cargo run -p gueiz --example mesh3d_test
//! cargo run -p gueiz --example mesh3d_test -- --window   # 結果を目で見る
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_3d::camera::Camera3d;
use gueiz_3d::draw_manager::{DrawManager3d, DrawManager3dDescriptor};
use gueiz_3d::mesh::Mesh;
use gueiz_3d::object::{create_instance, create_object};
use gueiz_3d::texture::TextureFormat;
use gueiz_3d::wgpu;

const SIZE: u32 = 256;

/// sRGB を通さない形式。読み戻した値がそのまま計算結果になる。
const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// 画面の中心。立方体はここに写る。
const CENTRE: (u32, u32) = (SIZE / 2, SIZE / 2);

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("mesh3d test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    let target = Target::new(&device);
    let mut preview = Preview::new("mesh3d_test");

    // 1. 立方体を 1 つ。正面から光を当てれば、手前の面は色そのまま。
    let pixels = render(&device, &queue, &target, &[Cube::new(RED, 0.0)], Light::Front)?;
    preview.capture("立方体 1 つ（正面から光）", SIZE, SIZE, &pixels);
    let centre = rgb(&pixels, CENTRE);
    println!("1. 立方体 1 つ（正面から光）    中={centre:?}");
    assert_eq!(centre, [255, 0, 0], "手前の面が真っ赤に出るはず");

    // 2. 法線が効いているか。背後から当てれば環境光だけになる。
    let pixels = render(&device, &queue, &target, &[Cube::new(WHITE, 0.0)], Light::Behind)?;
    preview.capture("背後から光（環境光 0.25）", SIZE, SIZE, &pixels);
    let dim = rgb(&pixels, CENTRE);
    println!("2. 背後から光（環境光 0.25）    中={dim:?}");
    for channel in dim {
        assert!(channel.abs_diff(64) <= 2, "{channel} は 64 前後のはず");
    }

    // 3 と 4 が本題。**深度バッファが描く順に勝つ**こと。
    //    2D では登録順（z ソート）で決まっていたが、3D は深度で決まる。
    //    赤を手前、青を奥に置いて、登録順だけ入れ替える。
    let near_red = Cube::new(RED, 2.0);
    let far_blue = Cube::new(BLUE, -6.0);

    let pixels = render(&device, &queue, &target, &[near_red, far_blue], Light::Front)?;
    preview.capture("手前を先に登録", SIZE, SIZE, &pixels);
    let registered_near_first = rgb(&pixels, CENTRE);
    println!("3. 手前を先に登録             中={registered_near_first:?}");
    assert_eq!(registered_near_first, [255, 0, 0], "手前の赤が見えるはず");

    let pixels = render(&device, &queue, &target, &[far_blue, near_red], Light::Front)?;
    preview.capture("奥を先に登録", SIZE, SIZE, &pixels);
    let registered_far_first = rgb(&pixels, CENTRE);
    println!("4. 奥を先に登録               中={registered_far_first:?}");
    assert_eq!(
        registered_far_first, [255, 0, 0],
        "登録順を変えても手前の赤が勝つはず（深度が解いている）",
    );

    assert_eq!(
        registered_near_first, registered_far_first,
        "登録順で結果が変わってはいけない",
    );

    // 5. 錐台カリング。視野の中と外に置いて、生き残りを読み戻す。
    let (inside, total) = culling_counts(&device, &queue, true)?;
    let (without, _) = culling_counts(&device, &queue, false)?;
    println!("5. カリング                  {total} 個中 {inside} 個が視野内（切らなければ {without} 個）");
    assert_eq!(without, total, "切らなければ全部残る");
    assert_eq!(inside, 2, "視野に入れた 2 個だけが残るはず");

    println!("\nOK: 深度・カリング・法線、3D 固有の 3 点が期待どおり");

    preview.show()?;

    Ok(())
}

/// どこから光を当てるか。
#[derive(Clone, Copy)]
enum Light {
    /// 手前の面に正対。いちばん明るい。
    Front,
    /// 裏から。手前の面は環境光だけ。
    Behind,
}

impl Light {
    fn apply(self, draw_manager: &mut DrawManager3d) {
        match self {
            // 光が進む向き。手前の面（法線 +z）に当てるには -z へ進める。
            Self::Front => draw_manager.set_light([0.0, 0.0, -1.0], 0.0),
            Self::Behind => draw_manager.set_light([0.0, 0.0, 1.0], 0.25),
        }
    }
}

/// 置く立方体 1 つぶん。
#[derive(Clone, Copy)]
struct Cube {
    color: [f32; 4],
    z: f32,
}

impl Cube {
    fn new(color: [f32; 4], z: f32) -> Self {
        Self { color, z }
    }
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

/// 視点を原点の手前に置いて、原点を見るカメラ。
fn camera() -> Camera3d {
    let mut camera = Camera3d::perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
    camera.look_at([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    camera
}

/// 立方体を並べて描き、画面を読み戻す。
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    cubes: &[Cube],
    light: Light,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut draw_manager =
        DrawManager3d::new(device, FORMAT, &DrawManager3dDescriptor::default())?;

    draw_manager.set_camera(camera());
    light.apply(&mut draw_manager);

    for cube in cubes {
        let mut object = create_object("Cube", Mesh::cube(4.0, cube.color));
        object.translate(0.0, 0.0, cube.z);
        object.instance(create_instance());
        draw_manager.register(object);
    }

    draw_manager.prepare(device, queue)?;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("mesh3d test"),
    });

    draw_manager.draw(
        device,
        &mut encoder,
        &target.view,
        SIZE,
        SIZE,
        wgpu::Color::BLACK,
    );

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

/// 視野の中と外に立方体を置いて、生き残った数を読み戻す。
///
/// 返すのは `(生き残り, 置いた総数)`。
fn culling_counts(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    culling: bool,
) -> Result<(u32, u32), Box<dyn Error>> {
    let mut draw_manager =
        DrawManager3d::new(device, FORMAT, &DrawManager3dDescriptor::default())?;

    draw_manager.set_camera(camera());
    draw_manager.set_culling(culling);

    let mut object = create_object("Cubes", Mesh::cube(2.0, WHITE));

    // 視野の中に 2 つ。
    let inside = [[0.0, 0.0, 0.0], [1.0, 1.0, -2.0]];
    // 視野の外に 4 つ。真横・真上・カメラの背後・遠すぎるところ。
    let outside = [
        [500.0, 0.0, 0.0],
        [0.0, 500.0, 0.0],
        [0.0, 0.0, 500.0],
        [0.0, 0.0, -500.0],
    ];

    for [x, y, z] in inside.iter().chain(outside.iter()) {
        object.instance(create_instance().translate(*x, *y, *z));
    }

    let total = (inside.len() + outside.len()) as u32;
    draw_manager.register(object);
    draw_manager.prepare(device, queue)?;

    // `DrawIndexedIndirectArgs` は 32 ビット 5 つ。instance_count はその 2 番目。
    let args_size = size_of::<u32>() as u64 * 5;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("indirect readback"),
        size: args_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("indirect readback"),
    });
    encoder.copy_buffer_to_buffer(
        draw_manager.indirect_buffer(),
        draw_manager.indirect_offset(),
        &readback,
        0,
        args_size,
    );
    queue.submit(Some(encoder.finish()));

    readback.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let view = readback.get_mapped_range(..)?;
    let survivors = u32::from_le_bytes(view[4..8].try_into()?);
    drop(view);
    readback.unmap();

    Ok((survivors, total))
}

/// その座標の色。読み戻したバイト列は BGRA の並び。
fn rgb(pixels: &[u8], (x, y): (u32, u32)) -> [u8; 3] {
    let index = ((y * SIZE + x) * 4) as usize;
    [pixels[index + 2], pixels[index + 1], pixels[index]]
}
