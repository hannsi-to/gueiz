//! 塗りにどれだけ時間がかかるかを測る。ウィンドウは開かない。
//!
//! [`bench_draw`](../bench_draw.rs) はコンピュートパスまでで、1 枚も塗っていない。
//! エフェクトの速度を決めるのはフラグメント側なので、こちらで測る。
//!
//! 測るのは 2 つ。
//!
//! - **重ね塗り 1 層あたりの値段**: 画面いっぱいの四角を N 枚重ねて、N に対する傾き。
//!   ポストプロセス 1 パスの値段もほぼこれと同じになる（1 パス = 画面 1 枚の読み書き）。
//! - **普通のシーン**: 小さい図形をばらまいた場合。実際のアプリに近いほう。
//!
//! ```sh
//! cargo run --release -p gueiz --example bench_fill
//! ```
//!
//! **必ず `--release` で。**

use std::error::Error;
use std::f32::consts::TAU;
use std::time::Instant;

use gueiz_2d::camera::Camera;
use gueiz_2d::effect::Block;
use gueiz_2d::post::{PostChain, PostEffect, PostProcessor};
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

const SURFACE_FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

const WARMUP_FRAMES: u32 = 20;
const MEASURED_FRAMES: u32 = 120;

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
        label: Some("bench fill"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}", adapter.get_info().name);
    println!("target : {WIDTH}x{HEIGHT} ({} 画素)", WIDTH * HEIGHT);
    println!(
        "build  : {}\n",
        if cfg!(debug_assertions) {
            "debug (数値は参考にならない。--release で取り直すこと)"
        } else {
            "release"
        },
    );

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SURFACE_FORMAT.into(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // --- 1. 画面いっぱいを何枚重ねるか ---
    println!("画面いっぱいの四角を重ねる（= 重ね塗り / ポスト 1 パスの値段）");
    println!("{:>8}  {:>12}  {:>14}", "層", "1 フレーム", "1 層あたり");
    println!("{}", "-".repeat(40));

    let mut baseline = 0.0_f64;

    for layers in [1_u32, 2, 4, 8, 16, 32] {
        let elapsed = measure(&device, &queue, &view, &mut full_screen_scene(&device, &queue, layers)?)?;

        if layers == 1 {
            baseline = elapsed;
        }

        // 1 層目にはパスの立ち上げが乗るので、2 層目以降の傾きで見る。
        let per_layer = if layers > 1 {
            (elapsed - baseline) / (layers - 1) as f64
        } else {
            elapsed
        };

        println!("{layers:>8}  {:>10.3}ms  {:>12.3}ms", elapsed, per_layer);
    }

    // --- 2. 普通のシーン ---
    println!("\n小さい図形をばらまく（実際のアプリに近いほう）");
    println!(
        "{:>10}  {:>14}  {:>14}",
        "インスタンス", "エフェクト無し", "Color 段 4 山",
    );
    println!("{}", "-".repeat(46));

    for count in [600_u32, 6_000, 60_000] {
        let plain = measure(&device, &queue, &view, &mut scattered_scene(&device, &queue, count, 0)?)?;
        let four = measure(&device, &queue, &view, &mut scattered_scene(&device, &queue, count, 4)?)?;
        println!("{count:>10}  {plain:>12.3}ms  {four:>12.3}ms");
    }

    // --- 3. 画面全体のエフェクト ---
    println!("\n画面全体のエフェクト（1 パス = 画面 1 枚の読み書き）");
    println!("{:>10}  {:>14}  {:>14}", "パス", "1 フレーム", "1 パスあたり");
    println!("{}", "-".repeat(44));

    let mut processor = PostProcessor::new(&device, SURFACE_FORMAT.into());
    let mut scene = scattered_scene(&device, &queue, 600, 0)?;
    scene.prepare(&device, &queue)?;

    let mut baseline = 0.0_f64;

    for passes in [0_usize, 1, 2, 4, 8] {
        let mut chain = PostChain::new();
        for _ in 0..passes {
            // 1 パスで済むものを並べて、パス数と値段の関係だけを見る。
            chain.push(PostEffect::Vignette {
                amount: 0.5,
                softness: 0.3,
            });
        }

        let elapsed = measure_post(&device, &queue, &mut processor, &scene, &chain)?;

        if passes == 0 {
            baseline = elapsed;
        }

        let per_pass = if passes > 0 {
            (elapsed - baseline) / passes as f64
        } else {
            0.0
        };

        println!("{passes:>10}  {elapsed:>12.3}ms  {per_pass:>12.3}ms");
    }

    println!("\n60 fps = 16.7 ms / frame");

    Ok(())
}

/// 場面を描いてから鎖を通すまで。ポストの値段を測る。
fn measure_post(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    processor: &mut PostProcessor,
    scene: &DrawManager,
    chain: &PostChain,
) -> Result<f64, Box<dyn Error>> {
    let scene_view = processor
        .scene_view(device, WIDTH, HEIGHT, SURFACE_FORMAT.into())
        .clone();

    // 最後のパスの書き先。実際のアプリではサーフェスにあたる。
    let destination = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("destination"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SURFACE_FORMAT.into(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let destination_view = destination.create_view(&wgpu::TextureViewDescriptor::default());

    let mut total = 0.0_f64;

    for frame in 0..WARMUP_FRAMES + MEASURED_FRAMES {
        let start = Instant::now();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bench post"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scene_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            scene.draw(&mut render_pass);
        }

        processor.run(queue, &mut encoder, chain, &destination_view);

        queue.submit(Some(encoder.finish()));
        device.poll(wgpu::PollType::wait_indefinitely())?;

        if frame >= WARMUP_FRAMES {
            total += start.elapsed().as_secs_f64();
        }
    }

    Ok(total / MEASURED_FRAMES as f64 * 1000.0)
}

/// 1 フレーム（コンピュート + 描画）が終わるまでの時間。
fn measure(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &wgpu::TextureView,
    scene: &mut DrawManager,
) -> Result<f64, Box<dyn Error>> {
    // 形もインスタンスも動かさないので、コンピュートパスは 1 度でよい。
    // ここで測りたいのは塗りの値段だけ。
    scene.prepare(device, queue)?;

    let mut total = 0.0_f64;

    for frame in 0..WARMUP_FRAMES + MEASURED_FRAMES {
        let start = Instant::now();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bench"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bench pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            scene.draw(&mut render_pass);
        }

        queue.submit(Some(encoder.finish()));
        device.poll(wgpu::PollType::wait_indefinitely())?;

        if frame >= WARMUP_FRAMES {
            total += start.elapsed().as_secs_f64();
        }
    }

    Ok(total / MEASURED_FRAMES as f64 * 1000.0)
}

/// 画面いっぱいの四角を `layers` 枚重ねる。
fn full_screen_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: u32,
) -> Result<DrawManager, Box<dyn Error>> {
    let mut draw_manager = new_manager(device, queue)?;

    let mut quad = object::create_object("Full screen");
    quad.begin(PaintType::Fill);
    for [x, y] in [
        [0.0, 0.0],
        [WIDTH as f32, 0.0],
        [WIDTH as f32, HEIGHT as f32],
        [0.0, HEIGHT as f32],
    ] {
        // 半透明。2D は深度で弾けないので、重ねたぶんだけ全部塗ることになる。
        quad.put_vertex(Vertex::new_position_color(x, y, 0.0, 0.3, 0.5, 0.9, 0.5));
    }
    quad.end();
    quad.camera(Camera::orthographic_2d(WIDTH as f32, HEIGHT as f32));

    for _ in 0..layers {
        quad.instance(instance::create_instance());
    }

    draw_manager.register(quad);

    Ok(draw_manager)
}

/// 小さい図形を格子状にばらまく。
fn scattered_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    total: u32,
    effects: u32,
) -> Result<DrawManager, Box<dyn Error>> {
    let mut draw_manager = new_manager(device, queue)?;
    let camera = Camera::orthographic_2d(WIDTH as f32, HEIGHT as f32);

    let mut shape = object::create_object("Hexagon");
    shape.begin(PaintType::Fill);
    for index in 0..6 {
        let angle = index as f32 * TAU / 6.0;
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
    shape.end();
    shape.camera(camera);

    // Color 段に山を積む。1 画素あたりの仕事がそのぶん増える。
    for index in 0..effects {
        shape.effect(Block::Gradient {
            from: [1.0, 0.5, 0.2, 1.0],
            to: [0.2, 0.5, 1.0, 1.0],
            angle: index as f32 * 0.7,
        });
    }

    let columns = (total as f32).sqrt().ceil() as u32;
    for index in 0..total {
        let x = (index % columns) as f32 / columns as f32 * WIDTH as f32;
        let y = (index / columns) as f32 / columns as f32 * HEIGHT as f32;
        shape.instance(instance::create_instance().translate(x, y, 0.0));
    }

    draw_manager.register(shape);

    Ok(draw_manager)
}

fn new_manager(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<DrawManager, Box<dyn Error>> {
    Ok(DrawManager::new(
        device,
        queue,
        SURFACE_FORMAT,
        &DrawManagerDescriptor {
            max_instances: 128 * 1024,
            ..Default::default()
        },
    )?)
}
