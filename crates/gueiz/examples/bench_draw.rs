//! 描画の各段にどれだけ時間がかかっているかを測る。ウィンドウは開かない。
//!
//! インスタンス数を変えながら、
//!
//! - `animate` : CPU がインスタンスの姿勢を書き換える
//! - `prepare` : CPU が配列を組んで転送し、コンピュートパスを投入する
//! - `gpu wait`: GPU がそれを処理し終えるまで
//!
//! を分けて計る。60 fps の 1 フレームは 16.7 ms。
//!
//! 2 通りの使い方で計る。
//!
//! - `animated`: 毎フレーム全インスタンスを書き換える。いちばん重い使い方。
//! - `static`  : 置いたまま動かさない。dirty フラグが立たないので、
//!   `DrawManager` はインスタンスの組み直しも転送もしない。
//!
//! ```sh
//! cargo run --release -p gueiz --example bench_draw
//! ```
//!
//! **必ず `--release` で実行すること。** デバッグビルドでは CPU 側が
//! 10 倍以上遅くなり、比率が実態とかけ離れる。

use std::error::Error;
use std::f32::consts::TAU;
use std::time::Instant;

use gueiz_2d::camera::Camera;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::{JointType, PaintType};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const WIDTH: f32 = 1920.0;
const HEIGHT: f32 = 1080.0;

/// 計測するインスタンス総数。
const COUNTS: [u32; 5] = [600, 6_000, 60_000, 250_000, 1_000_000];

const WARMUP_FRAMES: u32 = 10;
const MEASURED_FRAMES: u32 = 60;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bench"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}", adapter.get_info().name);
    println!(
        "build  : {}\n",
        if cfg!(debug_assertions) {
            "debug (数値は参考にならない。--release で取り直すこと)"
        } else {
            "release"
        },
    );

    for animated in [true, false] {
        println!(
            "{}:",
            if animated {
                "animated（毎フレーム全インスタンスを書き換える）"
            } else {
                "static（置いたまま動かさない）"
            },
        );
        println!(
            "{:>10}  {:>10}  {:>10}  {:>10}  {:>10}  {:>8}",
            "instances", "animate", "prepare", "gpu wait", "total", "fps",
        );
        println!("{}", "-".repeat(68));

        for count in COUNTS {
            measure(&device, &queue, count, animated)?;
        }

        println!();
    }

    println!("\n60 fps = 16.7 ms / frame");

    Ok(())
}

fn measure(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    total: u32,
    animated: bool,
) -> Result<(), Box<dyn Error>> {
    let mut draw_manager = DrawManager::new(
        device,
        queue,
        TextureFormat::Bgra8UnormSrgb,
        &DrawManagerDescriptor {
            max_instances: total.next_power_of_two().max(1024),
            ..Default::default()
        },
    )?;

    let ids = build_scene(&mut draw_manager, total);

    let mut animate_total = 0.0_f64;
    let mut prepare_total = 0.0_f64;
    let mut wait_total = 0.0_f64;

    for frame in 0..WARMUP_FRAMES + MEASURED_FRAMES {
        let time = frame as f32 * 0.01;
        let measured = frame >= WARMUP_FRAMES;

        let start = Instant::now();
        if animated {
            animate(&mut draw_manager, &ids, time);
        }
        let after_animate = Instant::now();

        draw_manager.prepare(device, queue)?;
        let after_prepare = Instant::now();

        // ここまでの GPU 処理が終わるのを待つ。描画は含まないが、
        // コンピュートパス（行列組み立て + カリング）の時間が出る。
        device.poll(wgpu::PollType::wait_indefinitely())?;
        let after_wait = Instant::now();

        if measured {
            animate_total += (after_animate - start).as_secs_f64();
            prepare_total += (after_prepare - after_animate).as_secs_f64();
            wait_total += (after_wait - after_prepare).as_secs_f64();
        }
    }

    let frames = MEASURED_FRAMES as f64;
    let animate_ms = animate_total / frames * 1000.0;
    let prepare_ms = prepare_total / frames * 1000.0;
    let wait_ms = wait_total / frames * 1000.0;
    let total_ms = animate_ms + prepare_ms + wait_ms;

    println!(
        "{:>10}  {:>8.3}ms  {:>8.3}ms  {:>8.3}ms  {:>8.3}ms  {:>8.0}",
        total,
        animate_ms,
        prepare_ms,
        wait_ms,
        total_ms,
        1000.0 / total_ms,
    );

    Ok(())
}

/// 5 種類の図形に、合わせて `total` 個のインスタンスをばらまく。
fn build_scene(draw_manager: &mut DrawManager, total: u32) -> Vec<String> {
    let camera = Camera::orthographic_2d(WIDTH, HEIGHT);
    let mut ids = Vec::new();

    for kind in 0..5 {
        let mut shape = object::create_object(&format!("shape {kind}"));

        if kind % 2 == 0 {
            shape.begin(PaintType::Fill);
            for index in 0..(3 + kind) {
                let angle = index as f32 * TAU / (3 + kind) as f32;
                shape.put_vertex(Vertex::new_position_color(
                    angle.cos() * 16.0,
                    angle.sin() * 16.0,
                    0.0,
                    1.0,
                    0.6,
                    0.3,
                    1.0,
                ));
            }
        } else {
            shape.begin(PaintType::Stroke {
                line_width: 4.0,
                joint_type: JointType::Round,
                strip: false,
            });
            for index in 0..(3 + kind) {
                let angle = index as f32 * TAU / (3 + kind) as f32;
                shape.put_vertex(Vertex::new_position_color(
                    angle.cos() * 16.0,
                    angle.sin() * 16.0,
                    0.0,
                    0.3,
                    1.0,
                    0.7,
                    1.0,
                ));
            }
        }

        shape.end();
        shape.camera(camera);
        ids.push(draw_manager.register(shape));
    }

    // 画面いっぱいに格子状。全部見える位置に置く（カリングで消さない）。
    let per_object = total / ids.len() as u32;
    let columns = (per_object as f32).sqrt().ceil() as u32;

    for name in &ids {
        let Some(object) = draw_manager.object_mut(name) else {
            continue;
        };

        for index in 0..per_object {
            let x = (index % columns) as f32 / columns as f32 * WIDTH;
            let y = (index / columns) as f32 / columns as f32 * HEIGHT;
            object.instance(instance::create_instance().translate(x, y, 0.0));
        }
    }

    ids
}

/// 毎フレーム全インスタンスの姿勢を書き換える。いちばん重い使い方。
fn animate(draw_manager: &mut DrawManager, names: &[String], time: f32) {
    for name in names {
        let Some(object) = draw_manager.object_mut(name) else {
            continue;
        };

        for (index, instance) in object.instances_mut().iter_mut().enumerate() {
            instance.set_rotation(0.0, 0.0, time + index as f32 * 0.01);
        }
    }
}
