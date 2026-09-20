//! GPU カリングが効いていることを、インダイレクト引数を読み戻して確かめる。
//! ウィンドウは開かない。
//!
//! コンピュートパスが `DrawIndirectArgs.instance_count` を書くので、
//! そこを読めば「何個生き残ったか」が分かる。
//!
//! ```sh
//! cargo run -p gueiz --example cull_test
//! ```

use std::error::Error;

use gueiz_2d::camera::Camera;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // ウィンドウを作らずに GPU だけ掴む。
    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("cull test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    let mut draw_manager = DrawManager::new(
        &device,
        &queue,
        TextureFormat::Bgra8UnormSrgb,
        &DrawManagerDescriptor::default(),
    )?;

    // 30x30 の四角を 1 つ登録し、画面内に 3 個・画面外に 5 個ばらまく。
    let mut square = object::create_object("Square");
    square.begin(PaintType::Fill);
    for [x, y] in [[0.0, 0.0], [30.0, 0.0], [30.0, 30.0], [0.0, 30.0]] {
        square.put_vertex(Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0));
    }
    square.end();
    square.camera(Camera::orthographic_2d(WIDTH, HEIGHT));

    let inside = [[100.0, 100.0], [400.0, 300.0], [700.0, 500.0]];
    let outside = [
        [-500.0, 300.0],
        [1300.0, 300.0],
        [400.0, -500.0],
        [400.0, 1100.0],
        [-900.0, -900.0],
    ];

    for [x, y] in inside.iter().chain(outside.iter()) {
        square.instance(instance::create_instance().translate(*x, *y, 0.0));
    }

    draw_manager.register(square);
    let total = (inside.len() + outside.len()) as u32;

    println!("{total} instances: {} inside, {} outside", inside.len(), outside.len());

    let with_culling = run(&device, &queue, &mut draw_manager, true)?;
    let without_culling = run(&device, &queue, &mut draw_manager, false)?;

    println!("culling on : {with_culling} drawn");
    println!("culling off: {without_culling} drawn");

    assert_eq!(
        without_culling, total,
        "culling off should keep every instance",
    );
    assert_eq!(
        with_culling,
        inside.len() as u32,
        "culling on should keep only the visible ones",
    );

    println!("\nOK: the compute pass rejected {} off-screen instances", total - with_culling);

    Ok(())
}

/// コンピュートパスを 1 回走らせて、生き残ったインスタンス数を読み戻す。
fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    draw_manager: &mut DrawManager,
    culling: bool,
) -> Result<u32, Box<dyn Error>> {
    draw_manager.set_culling(culling);
    draw_manager.prepare(device, queue)?;

    // `DrawIndirectArgs` は u32 4 つ。instance_count はその 2 番目。
    let args_size = size_of::<u32>() as u64 * 4;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: args_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback"),
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
    let instance_count = u32::from_le_bytes(view[4..8].try_into()?);
    drop(view);
    readback.unmap();

    Ok(instance_count)
}
