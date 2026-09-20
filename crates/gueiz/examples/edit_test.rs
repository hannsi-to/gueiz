//! 実行中に頂点を足し引きした結果が GPU まで届いているかを確かめる。
//! ウィンドウは開かない。
//!
//! 図形を書き換えると三角形の数が変わり、それは
//! `DrawIndirectArgs.vertex_count` に出る。そこを読み戻せば、
//!
//! - 編集 → 積み直しの印 → プールの積み直し → インダイレクト引数
//!
//! という鎖がつながっているかが分かる。CPU 側の形だけ合っていて
//! GPU に届いていない、という壊れ方をここで捕まえる。
//!
//! ```sh
//! cargo run -p gueiz --example edit_test
//! ```

use std::error::Error;

use gueiz_2d::camera::Camera;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object;
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("edit test"),
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

    // 30x30 の四角。塗りなので三角形 2 枚 = 頂点 6 個。
    let mut square = object::create_object("Square");
    square.begin(PaintType::Fill);
    for [x, y] in [[0.0, 0.0], [30.0, 0.0], [30.0, 30.0], [0.0, 30.0]] {
        square.put_vertex(point(x, y));
    }
    square.end();
    square.camera(Camera::orthographic_2d(WIDTH, HEIGHT));

    let id = draw_manager.register(square);

    let step = |draw_manager: &mut DrawManager, label: &str| -> Result<u32, Box<dyn Error>> {
        let count = vertex_count(&device, &queue, draw_manager)?;
        println!("{label:<30} 頂点 {count:>3} 個 = 三角形 {:>2} 枚", count / 3);
        Ok(count)
    };

    // 1. 四角のまま。n 角形は n - 2 枚。
    assert_eq!(step(&mut draw_manager, "1. 四角")?, 3 * 2);

    // 2. 頂点を足して五角形に。
    edit(&mut draw_manager, &id, |object| {
        object.push_vertex(0, point(-15.0, 15.0));
    });
    assert_eq!(step(&mut draw_manager, "2. 頂点を 1 つ足す")?, 3 * 3);

    // 3. 足した頂点を消して四角に戻す。
    edit(&mut draw_manager, &id, |object| {
        object.remove_vertex(0, 4);
    });
    assert_eq!(step(&mut draw_manager, "3. 足した頂点を消す")?, 3 * 2);

    // 4. 穴を開ける。外周 4 + 穴 4 + 通路 2 = 10 → 8 枚。
    edit(&mut draw_manager, &id, |object| {
        let hole = object.add_hole();
        for [x, y] in [[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]] {
            object.push_vertex(hole, point(x, y));
        }
    });
    assert_eq!(step(&mut draw_manager, "4. 穴を開ける")?, 3 * 8);

    // 5. 穴の頂点を 1 つ消す。穴が三角形になり、外周 4 + 穴 3 + 通路 2 = 9 → 7 枚。
    edit(&mut draw_manager, &id, |object| {
        object.remove_vertex(1, 3);
    });
    assert_eq!(step(&mut draw_manager, "5. 穴の頂点を 1 つ消す")?, 3 * 7);

    // 6. 穴を塞ぐ。
    edit(&mut draw_manager, &id, |object| {
        object.remove_contour(1);
    });
    assert_eq!(step(&mut draw_manager, "6. 穴を塞ぐ")?, 3 * 2);

    // 7. まとめて編集しても結果は同じ。
    edit(&mut draw_manager, &id, |object| {
        let mut batch = object.edit();

        for vertex in batch.vertices_mut() {
            vertex.x *= 2.0;
        }

        batch.push_vertex(0, point(-15.0, 15.0));
        batch.push_vertex(0, point(-15.0, 45.0));
    });
    assert_eq!(step(&mut draw_manager, "7. まとめて編集")?, 3 * 4);

    println!("\nOK: 実行中の編集がインダイレクト引数まで届いている");

    Ok(())
}

fn point(x: f32, y: f32) -> Vertex {
    Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0)
}

fn edit(draw_manager: &mut DrawManager, name: &str, change: impl FnOnce(&mut object::Object)) {
    if let Some(object) = draw_manager.object_mut(name) {
        change(object);
    }
}

/// コンピュートパスを 1 回走らせて、GPU が使う頂点数を読み戻す。
fn vertex_count(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    draw_manager: &mut DrawManager,
) -> Result<u32, Box<dyn Error>> {
    draw_manager.prepare(device, queue)?;

    // `DrawIndirectArgs` は u32 4 つ。vertex_count はその 1 番目。
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
    let count = u32::from_le_bytes(view[0..4].try_into()?);
    drop(view);
    readback.unmap();

    Ok(count)
}
