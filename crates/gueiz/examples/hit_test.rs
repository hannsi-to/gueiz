//! 当たり判定が**描いた画素と一致する**かを確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 全画素について「塗られているか」と「[`Object::hit`] が当たるか」を
//! 突き合わせる。**食い違うのは縁だけ**であるべきで、中身がずれていたら
//! 判定と見た目が別のことを言っている。
//!
//! 見るのは 4 つ。
//!
//! - 塗った四角
//! - **穴**（抜けたところは当たらない）
//! - **線**（囲まれた真ん中は当たらない）
//! - 動かして回して拡大した複製
//!
//! ```sh
//! cargo run -p gueiz --example hit_test
//! cargo run -p gueiz --example hit_test -- --window
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::camera::Camera;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{create_object, Object};
use gueiz_2d::paint_type::{JointType, PaintType};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::{instance, wgpu};
use gueiz_2d::instance::Instance;

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("hit test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    let target = Target::new(&device);
    let mut preview = Preview::new("hit_test");

    let cases: [(&str, fn() -> Object); 4] = [
        ("塗った四角", filled_square),
        ("穴あき", square_with_hole),
        ("線", stroked_ring),
        ("動かして回して拡大", transformed),
    ];

    for (label, build) in cases {
        let object = build();
        let pixels = render(&device, &queue, &target, build())?;
        preview.capture(label, SIZE, SIZE, &pixels);

        let report = compare(&object, &pixels);

        println!(
            "{label:<22} 塗り {:>6} 画素、当たり {:>6} 画素、食い違い {:>4}（縁の外 {}）",
            report.painted, report.hit, report.disagreed, report.disagreed_away_from_edge,
        );

        assert!(report.painted > 500, "{label}: ほとんど描けていない");
        assert!(report.hit > 500, "{label}: ほとんど当たらない");

        // 縁の画素は、ラスタライザの丸めと判定の丸めが違うのでずれる。
        // **縁から離れたところで食い違ってはいけない。**
        assert_eq!(
            report.disagreed_away_from_edge, 0,
            "{label}: 中身が食い違っている（判定と見た目が別)",
        );
    }

    // 穴と線は「中が空く」ことが要点なので、真ん中を名指しで見る。
    let hole = square_with_hole();
    println!(
        "\n穴の真ん中 (128,128)   当たり {:?}",
        hole.hit(128.0, 128.0),
    );
    assert_eq!(hole.hit(128.0, 128.0), None, "穴に当たっている");
    assert_eq!(hole.hit(50.0, 128.0), Some(0), "外周の帯に当たらない");

    let ring = stroked_ring();
    println!("線の真ん中 (128,128)   当たり {:?}", ring.hit(128.0, 128.0));
    assert_eq!(ring.hit(128.0, 128.0), None, "塗っていない真ん中に当たっている");
    assert_eq!(ring.hit(40.0, 128.0), Some(0), "線の上に当たらない");

    println!("\nすべて通りました。");

    if common::preview::window_requested() {
        preview.show()?;
    } else {
        println!("`-- --window` を付けると描いた結果を窓に出します。");
    }

    Ok(())
}

// --- 図形 ---

fn at(x: f32, y: f32) -> Vertex {
    Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0)
}

fn placed(mut object: Object) -> Object {
    object.camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32));
    object.instance(instance::create_instance());

    object
}

fn filled_square() -> Object {
    let mut object = create_object("Filled");
    object.begin(PaintType::Fill);

    for (x, y) in [(40.0, 40.0), (216.0, 40.0), (216.0, 216.0), (40.0, 216.0)] {
        object.put_vertex(at(x, y));
    }

    object.end();
    placed(object)
}

fn square_with_hole() -> Object {
    let mut object = create_object("Holed");
    object.begin(PaintType::Fill);

    for (x, y) in [(20.0, 20.0), (236.0, 20.0), (236.0, 236.0), (20.0, 236.0)] {
        object.put_vertex(at(x, y));
    }

    object.begin_hole();

    for (x, y) in [(90.0, 90.0), (166.0, 90.0), (166.0, 166.0), (90.0, 166.0)] {
        object.put_vertex(at(x, y));
    }

    object.end();
    placed(object)
}

fn stroked_ring() -> Object {
    let mut object = create_object("Ring");
    object.begin(PaintType::Stroke {
        line_width: 12.0,
        joint_type: JointType::Miter,
        strip: false,
            dash: None,
    });

    for (x, y) in [(40.0, 40.0), (216.0, 40.0), (216.0, 216.0), (40.0, 216.0)] {
        object.put_vertex(at(x, y));
    }

    object.end();
    placed(object)
}

/// 図形の変換と複製の変換を両方掛けたもの。
fn transformed() -> Object {
    let mut object = create_object("Moved");
    object.begin(PaintType::Fill);

    // 原点まわりの小さな四角。置き場所と大きさは変換で決める。
    for (x, y) in [(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)] {
        object.put_vertex(at(x, y));
    }

    object.end();
    object.camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32));
    object.translate(128.0, 128.0, 0.0);

    object.instance(
        Instance::new()
            .rotate(0.0, 0.0, std::f32::consts::FRAC_PI_4)
            .scale(4.0, 2.0, 1.0),
    );

    object
}

// --- 突き合わせ ---

struct Report {
    painted: usize,
    hit: usize,
    disagreed: usize,
    /// 食い違っていて、**しかも縁ではない**画素。ここは 0 でなければならない。
    disagreed_away_from_edge: usize,
}

/// 全画素で「塗られているか」と「当たるか」を比べる。
fn compare(object: &Object, pixels: &[u8]) -> Report {
    let painted: Vec<bool> = (0..SIZE * SIZE)
        .map(|index| pixels[(index * 4) as usize + 2] > 16)
        .collect();

    let mut report = Report {
        painted: painted.iter().filter(|lit| **lit).count(),
        hit: 0,
        disagreed: 0,
        disagreed_away_from_edge: 0,
    };

    for y in 0..SIZE {
        for x in 0..SIZE {
            // 画素の真ん中で聞く。角で聞くと、隣とどちらつかずになる。
            let hit = object.hit(x as f32 + 0.5, y as f32 + 0.5).is_some();
            let lit = painted[(y * SIZE + x) as usize];

            if hit {
                report.hit += 1;
            }

            if hit == lit {
                continue;
            }

            report.disagreed += 1;

            // 周り 1 画素に塗りと非塗りが混ざっていれば、そこは縁。
            if !on_edge(&painted, x, y) {
                report.disagreed_away_from_edge += 1;
            }
        }
    }

    report
}

/// 周り 1 画素に塗りと非塗りが混ざっているか。混ざっていればそこは縁。
fn on_edge(painted: &[bool], x: u32, y: u32) -> bool {
    let mut lit = false;
    let mut dark = false;

    for dy in -1_i64..=1 {
        for dx in -1_i64..=1 {
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);

            if nx < 0 || ny < 0 || nx >= SIZE as i64 || ny >= SIZE as i64 {
                // 画面の外は塗られていないものとして扱う。
                dark = true;
                continue;
            }

            if painted[(ny as usize) * (SIZE as usize) + nx as usize] {
                lit = true;
            } else {
                dark = true;
            }
        }
    }

    lit && dark
}

// --- 描く ---

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    object: Object,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut draw_manager =
        DrawManager::new(device, queue, FORMAT, &DrawManagerDescriptor::default())?;

    draw_manager.register(object);
    draw_manager.prepare(device, queue)?;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hit test"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shape"),
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
