//! 「変わっていないインスタンスを送り直さない」経路が正しいかを確かめる。
//! ウィンドウは開かない。
//!
//! インスタンスの部分転送はオフセット計算を間違えやすい。ずれれば
//! 別の図形の持ち分を踏み、送り忘れれば書き換えが画面に出ない。
//! ここではカリングを使って「いま何個が画面内にあるか」を図形ごとに読み戻し、
//!
//! - 書き換えた図形だけが変わること
//! - 書き換えていない図形が前フレームの値を保つこと
//! - 先頭の図形を書き換えても後ろの図形を踏まないこと
//!
//! を見る。
//!
//! ```sh
//! cargo run -p gueiz --example dirty_test
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

/// 画面内に置くときの位置。
const ON_SCREEN: [f32; 2] = [400.0, 300.0];
/// 画面外に置くときの位置。境界円ごと外に出る距離。
const OFF_SCREEN: [f32; 2] = [-5000.0, -5000.0];

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("dirty test"),
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

    // 先に登録したほうが入力バッファの先頭を取る。後ろの図形の base は 3 になり、
    // 部分転送のオフセット計算がそこで効いてくる。
    let front = register_squares(&mut draw_manager, "Front", 3, ON_SCREEN);
    let back = register_squares(&mut draw_manager, "Back", 4, OFF_SCREEN);

    let step = |draw_manager: &mut DrawManager, label: &str| -> Result<[u32; 2], Box<dyn Error>> {
        let counts = run(&device, &queue, draw_manager)?;
        println!("{label:<34} front={} back={}", counts[0], counts[1]);
        Ok(counts)
    };

    // 1. 最初のフレーム。並びを組んだばかりなので全部が送られる。
    let counts = step(&mut draw_manager, "1. 初回")?;
    assert_eq!(counts, [3, 0], "front は画面内に 3 個、back は全部画面外");

    // 2. 何も触らずにもう一度。送り直しは起きないが、値は保たれていなければならない。
    let counts = step(&mut draw_manager, "2. 何も書き換えず")?;
    assert_eq!(counts, [3, 0], "送らなかったぶんが消えてはいけない");

    // 3. 後ろの図形だけを画面内へ。base = 3 の位置に正しく書けているか。
    move_instances(&mut draw_manager, &back, ON_SCREEN);
    let counts = step(&mut draw_manager, "3. back だけ画面内へ")?;
    assert_eq!(counts, [3, 4], "back が全部出てきて、front は変わらない");

    // 4. 先頭の図形だけを画面外へ。base = 0 への書き込みが後ろを踏まないか。
    move_instances(&mut draw_manager, &front, OFF_SCREEN);
    let counts = step(&mut draw_manager, "4. front だけ画面外へ")?;
    assert_eq!(counts, [0, 4], "front だけが消え、back は残る");

    // 5. インスタンスを足すと並びが崩れる。全部組み直す経路に入る。
    if let Some(object) = draw_manager.object_mut(&front) {
        let [x, y] = ON_SCREEN;
        object.instance(instance::create_instance().translate(x, y, 0.0));
    }
    let counts = step(&mut draw_manager, "5. front に 1 個足す")?;
    assert_eq!(counts, [1, 4], "足した 1 個だけが画面内。back は組み直しても同じ");

    println!("\nOK: 書き換えた図形だけが変わり、触っていない図形は保たれた");

    Ok(())
}

/// 30x30 の四角を 1 つ登録し、同じ場所に `count` 個ばらまく。
fn register_squares(
    draw_manager: &mut DrawManager,
    name: &str,
    count: u32,
    [x, y]: [f32; 2],
) -> String {
    let mut square = object::create_object(name);
    square.begin(PaintType::Fill);
    for [vx, vy] in [[0.0, 0.0], [30.0, 0.0], [30.0, 30.0], [0.0, 30.0]] {
        square.put_vertex(Vertex::new_position_color(vx, vy, 0.0, 1.0, 1.0, 1.0, 1.0));
    }
    square.end();
    square.camera(Camera::orthographic_2d(WIDTH, HEIGHT));

    for _ in 0..count {
        square.instance(instance::create_instance().translate(x, y, 0.0));
    }

    draw_manager.register(square)
}

/// その図形のインスタンスを全部同じ場所へ動かす。
fn move_instances(draw_manager: &mut DrawManager, name: &str, [x, y]: [f32; 2]) {
    let Some(object) = draw_manager.object_mut(name) else {
        return;
    };

    for instance in object.instances_mut() {
        instance.set_translation(x, y, 0.0);
    }
}

/// コンピュートパスを 1 回走らせて、図形ごとの生き残り数を読み戻す。
fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    draw_manager: &mut DrawManager,
) -> Result<[u32; 2], Box<dyn Error>> {
    draw_manager.prepare(device, queue)?;

    // `DrawIndirectArgs` は u32 4 つ。instance_count はその 2 番目。
    let args_size = size_of::<u32>() as u64 * 4;
    let total = args_size * draw_manager.draw_count() as u64;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: total,
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
        total,
    );
    queue.submit(Some(encoder.finish()));

    readback.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let view = readback.get_mapped_range(..)?;
    let counts = [
        u32::from_le_bytes(view[4..8].try_into()?),
        u32::from_le_bytes(view[20..24].try_into()?),
    ];
    drop(view);
    readback.unmap();

    Ok(counts)
}
