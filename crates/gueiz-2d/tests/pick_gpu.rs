//! 当たり判定が**描いた画素と一致する**かを、GPU に描かせて確かめる。
//!
//! [`DrawManager::pick`] は「画面のここを押した」に対して
//! 「この図形のこの複製」を返します。**返すものと見えているものが食い違うと、
//! 押しているのに反応しない / 隣が反応する**という形で出ます。
//!
//! 画素を読み戻して、色から「そこに見えている図形」を割り出し、
//! `pick` の答えと突き合わせます。
//!
//! GPU が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test pick_gpu -- --nocapture
//! ```

use std::sync::OnceLock;

use gueiz_2d::camera::{Camera, ScaleMode};
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::instance::create_instance;
use gueiz_2d::object::{Object, create_object};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

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
            label: Some("pick gpu test"),
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

fn manager(gpu: &Gpu) -> DrawManager {
    DrawManager::new(
        &gpu.device,
        &gpu.queue,
        FORMAT,
        &DrawManagerDescriptor::default(),
    )
    .expect("DrawManager を作れなかった")
}

/// 画素ぴったりの座標系。
fn camera() -> Camera {
    Camera::orthographic_2d(SIZE as f32, SIZE as f32)
}

/// 左上を `(x, y)` とした `size` 四方の塗った四角。色で見分ける。
fn square(name: &str, x: f32, y: f32, size: f32, color: [f32; 3]) -> Object {
    let mut object = create_object(name);
    object.begin(PaintType::Fill);

    for (dx, dy) in [(0.0, 0.0), (size, 0.0), (size, size), (0.0, size)] {
        object.put_vertex(Vertex::new_position_color(
            x + dx,
            y + dy,
            0.0,
            color[0],
            color[1],
            color[2],
            1.0,
        ));
    }

    object.end();
    object.camera(camera());

    object
}

/// 描いて画素を読み戻す。
fn render(gpu: &Gpu, manager: &mut DrawManager) -> Vec<u8> {
    manager
        .prepare(&gpu.device, &gpu.queue)
        .expect("支度に失敗した");

    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pick target"),
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
        label: Some("pick readback"),
        size: (SIZE * SIZE * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pick"),
        });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shapes"),
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

/// その画素の色。`[B, G, R, A]` を `[R, G, B]` に直して返す。
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let at = ((y * SIZE + x) * 4) as usize;

    [pixels[at + 2], pixels[at + 1], pixels[at]]
}

/// 色が付いているか（黒い背景でないか）。
fn painted(pixels: &[u8], x: u32, y: u32) -> bool {
    pixel(pixels, x, y) != [0, 0, 0]
}

// --- 試験 ---

#[test]
fn what_is_picked_is_what_is_painted() {
    let gpu = gpu!();
    let mut manager = manager(gpu);

    // 離れた 2 つ。重なっていないので、見えているものがそのまま当たる。
    let mut left = square("left", 20.0, 20.0, 60.0, [1.0, 0.0, 0.0]);
    left.instance(create_instance());
    manager.register(left);

    let mut right = square("right", 150.0, 150.0, 60.0, [0.0, 0.0, 1.0]);
    right.instance(create_instance());
    manager.register(right);

    let pixels = render(gpu, &mut manager);

    // 画素を全部なめて、「塗られている ⇔ 当たる」を突き合わせる。
    let mut painted_count = 0;
    let mut disagreed = 0;

    for y in 0..SIZE {
        for x in 0..SIZE {
            // 画素の真ん中を聞く。縁は丸めの違いでずれるので数えない。
            let point = (x as f32 + 0.5, y as f32 + 0.5);
            let hit = manager.pick(point.0, point.1).is_some();
            let lit = painted(&pixels, x, y);

            if lit {
                painted_count += 1;
            }

            if lit != hit {
                disagreed += 1;
            }
        }
    }

    println!("塗り {painted_count} 画素、食い違い {disagreed}");

    assert!(painted_count > 2 * 50 * 50, "ほとんど描けていない");
    // 縁の 1 画素は、ラスタライザの丸めと判定の丸めが違うのでずれる。
    assert!(
        disagreed < painted_count / 20,
        "中身が食い違っている: {disagreed} / {painted_count}",
    );
}

#[test]
fn the_front_one_wins_where_they_overlap() {
    let gpu = gpu!();
    let mut manager = manager(gpu);

    // 同じ場所に 2 つ。z で前後を決める。
    let mut back = square("back", 60.0, 60.0, 120.0, [1.0, 0.0, 0.0]);
    back.instance(create_instance());
    back.z(0.0);
    manager.register(back);

    let mut front = square("front", 100.0, 100.0, 120.0, [0.0, 0.0, 1.0]);
    front.instance(create_instance());
    front.z(1.0);
    manager.register(front);

    let pixels = render(gpu, &mut manager);

    // 重なっているところ。青が見えている。
    assert_eq!(pixel(&pixels, 140, 140), [0, 0, 255]);
    assert_eq!(
        manager.pick(140.5, 140.5).map(|pick| pick.name).as_deref(),
        Some("front"),
        "見えているほうが取れない",
    );

    // 重なっていない奥のところ。赤が見えている。
    assert_eq!(pixel(&pixels, 70, 70), [255, 0, 0]);
    assert_eq!(
        manager.pick(70.5, 70.5).map(|pick| pick.name).as_deref(),
        Some("back"),
    );

    // 下にあるものまで拾う。
    let all = manager.pick_all(140.5, 140.5);
    let names: Vec<&str> = all.iter().map(|pick| pick.name.as_str()).collect();

    assert_eq!(names, vec!["front", "back"], "手前から順に並ぶ");
}

#[test]
fn each_instance_is_told_apart() {
    let gpu = gpu!();
    let mut manager = manager(gpu);

    // 1 つの図形を 3 箇所に置く。
    let mut row = square("row", 0.0, 0.0, 40.0, [0.0, 1.0, 0.0]);
    for step in 0..3 {
        row.instance(create_instance().translate(step as f32 * 80.0 + 10.0, 100.0, 0.0));
    }
    manager.register(row);

    let pixels = render(gpu, &mut manager);

    for (step, x) in [(0usize, 30u32), (1, 110), (2, 190)] {
        assert!(painted(&pixels, x, 120), "{step} 番目が描かれていない");

        let pick = manager.pick(x as f32 + 0.5, 120.5).expect("当たる");

        assert_eq!(pick.name, "row");
        assert_eq!(pick.instance, step, "複製の番号が違う");
    }

    // 複製と複製のあいだは空いている。
    assert!(!painted(&pixels, 70, 120));
    assert!(manager.pick(70.5, 120.5).is_none());
}

#[test]
fn the_local_point_comes_back_in_the_shapes_own_coordinates() {
    let gpu = gpu!();
    let mut manager = manager(gpu);

    // 図形は原点に組んで、複製で動かして 2 倍にする。
    let mut square = square("scaled", 0.0, 0.0, 50.0, [1.0, 1.0, 0.0]);
    square.instance(create_instance().translate(100.0, 100.0, 0.0).scale(2.0, 2.0, 1.0));
    manager.register(square);

    let pixels = render(gpu, &mut manager);

    // 100..200 に広がっている。
    assert!(painted(&pixels, 150, 150));
    assert!(!painted(&pixels, 90, 150));

    let pick = manager.pick(150.5, 150.5).expect("当たる");

    // 画面の (150.5, 150.5) は、2 倍して 100 動かす前では (25.25, 25.25)。
    assert!((pick.local[0] - 25.25).abs() < 0.01, "{:?}", pick.local);
    assert!((pick.local[1] - 25.25).abs() < 0.01, "{:?}", pick.local);
}

#[test]
fn a_relative_camera_still_lands_on_the_right_spot() {
    let gpu = gpu!();
    let mut manager = manager(gpu);

    // 128x128 で組んだ絵を、256x256 の画面へ 2 倍に伸ばす。
    // **画素位置と図形の座標が一致しなくなる**ので、戻し忘れるとここで出る。
    let mut object = create_object("relative");
    object.begin(PaintType::Fill);

    for (x, y) in [(10.0, 10.0), (50.0, 10.0), (50.0, 50.0), (10.0, 50.0)] {
        object.put_vertex(Vertex::new_position_color(x, y, 0.0, 1.0, 0.0, 1.0, 1.0));
    }

    object.end();
    object.camera_for(
        SurfaceSize::new(SIZE, SIZE),
        [128.0, 128.0],
        ScaleMode::Stretch,
    );
    object.instance(create_instance());
    manager.register(object);

    let pixels = render(gpu, &mut manager);

    // 2 倍なので、画面では 20..100。
    assert!(painted(&pixels, 60, 60), "描かれていない");
    assert!(!painted(&pixels, 10, 10));

    let pick = manager.pick(60.5, 60.5).expect("当たる");

    assert_eq!(pick.name, "relative");
    // 画面の 60.5 は、図形の座標では 30.25。
    assert!((pick.local[0] - 30.25).abs() < 0.01, "{:?}", pick.local);

    assert!(manager.pick(10.5, 10.5).is_none(), "外に当たっている");
}

#[test]
fn a_hole_is_not_picked() {
    let gpu = gpu!();
    let mut manager = manager(gpu);

    // 穴の空いた四角。見た目どおり、穴の中は当たらないこと。
    let mut object = create_object("ring");
    object.begin(PaintType::Fill);

    for (x, y) in [(40.0, 40.0), (216.0, 40.0), (216.0, 216.0), (40.0, 216.0)] {
        object.put_vertex(Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0));
    }

    object.begin_hole();

    for (x, y) in [(100.0, 100.0), (156.0, 100.0), (156.0, 156.0), (100.0, 156.0)] {
        object.put_vertex(Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0));
    }

    object.end();
    object.camera(camera());
    object.instance(create_instance());
    manager.register(object);

    let pixels = render(gpu, &mut manager);

    assert!(!painted(&pixels, 128, 128), "穴が塗られている");
    assert!(manager.pick(128.5, 128.5).is_none(), "穴に当たっている");

    assert!(painted(&pixels, 60, 128));
    assert!(manager.pick(60.5, 128.5).is_some(), "帯に当たらない");
}

#[test]
fn nothing_is_picked_where_nothing_is_drawn() {
    let gpu = gpu!();
    let mut manager = manager(gpu);

    let mut object = square("alone", 100.0, 100.0, 20.0, [1.0, 1.0, 1.0]);
    object.instance(create_instance());
    manager.register(object);

    render(gpu, &mut manager);

    assert!(manager.pick(10.5, 10.5).is_none());
    assert!(manager.pick_all(10.5, 10.5).is_empty());
    // 画面の外。
    assert!(manager.pick(-5.0, 10.0).is_none());
    assert!(manager.pick(1000.0, 1000.0).is_none());
}
