//! 画面全体のエフェクトが効いているか、**描いた画素を読み戻して**確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 見るのは 4 つ。
//!
//! - 色を触る系が効くこと（ColorGrade）
//! - 位置で効き方が変わる系が効くこと（Vignette）
//! - 隣の画素を読む系が効くこと（Blur）— これが図形ごとの段では書けないもの
//! - **積んだ順が結果を変えること**
//!
//! ```sh
//! cargo run -p gueiz --example post_test
//! cargo run -p gueiz --example post_test -- --window   # 結果を目で見る
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::camera::Camera;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::post::{PostChain, PostEffect, PostProcessor};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const SIZE: u32 = 512;

/// sRGB ではなく素の形式を使う。読み戻した値がそのまま線形なので、
/// 「2 倍したら 2 倍」と計算で確かめられる。
const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

/// 白い四角の範囲。
const SQUARE: (f32, f32, f32, f32) = (100.0, 100.0, 300.0, 300.0);
/// 四角の中。ぼかしても白のまま。
const INSIDE: (u32, u32) = (200, 200);
/// 四角の外、右に 10 画素。ぼかすとここまでにじむ。
const OUTSIDE: (u32, u32) = (310, 200);
/// 角。ビネットがいちばん効く。
const CORNER: (u32, u32) = (8, 8);

const BLUR_RADIUS: f32 = 8.0;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("post test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    let mut harness = Harness::new(&device, &queue)?;
    let mut preview = Preview::new("post_test");

    // 1. エフェクト無しは通さない（走らせるパスが無い）ので、まず 1 つだけで基準を取る。
    let mut chain = PostChain::new();
    chain.push(PostEffect::ColorGrade {
        exposure: 1.0,
        saturation: 1.0,
        tint: [1.0; 4],
    });
    let pixels = harness.run(&device, &queue, &chain)?;
    preview.capture("素通し", SIZE, SIZE, &pixels);
    println!("1. 素通し                    中={:?} 外={:?}", red(&pixels, INSIDE), red(&pixels, OUTSIDE));
    assert_eq!(red(&pixels, INSIDE), 255, "四角の中は白のまま");
    assert_eq!(red(&pixels, OUTSIDE), 0, "四角の外は黒のまま");

    // 2. 明るさを半分に。線形な形式なので、ちょうど半分になる。
    let mut chain = PostChain::new();
    chain.push(PostEffect::ColorGrade {
        exposure: 0.5,
        saturation: 1.0,
        tint: [1.0; 4],
    });
    let pixels = harness.run(&device, &queue, &chain)?;
    preview.capture("ColorGrade(明るさ 0.5)", SIZE, SIZE, &pixels);
    let half = red(&pixels, INSIDE);
    println!("2. ColorGrade(明るさ 0.5)    中={half}");
    assert!(half.abs_diff(128) <= 2, "{half} は 128 前後のはず");

    // 3. ビネット。中心は残り、角が落ちる。
    let mut chain = PostChain::new();
    chain.push(PostEffect::Vignette {
        amount: 1.0,
        softness: 0.2,
    });
    let pixels = harness.run(&device, &queue, &chain)?;
    preview.capture("Vignette", SIZE, SIZE, &pixels);
    let centre = red(&pixels, INSIDE);
    println!("3. Vignette                  中={centre} 角={}", red(&pixels, CORNER));
    assert!(centre > 200, "中心はあまり落ちないはず（{centre}）");

    // 4. ぼかし。**四角の外ににじむ** = 隣の画素を読んでいる。
    //    これは図形ごとの段では書けない。
    let mut chain = PostChain::new();
    chain.push(PostEffect::Blur { radius: BLUR_RADIUS });
    let pixels = harness.run(&device, &queue, &chain)?;
    preview.capture("Blur(8)", SIZE, SIZE, &pixels);
    let bled = red(&pixels, OUTSIDE);
    println!("4. Blur({BLUR_RADIUS})                  外={bled}");
    assert!(bled > 0, "四角の外ににじむはず");
    assert!(bled < 255, "外が真っ白になるのはおかしい（{bled}）");

    // 5 と 6 が本題。同じ 2 つを、順番だけ変える。
    //
    // 値は 1.0 で頭打ちになるので、
    //   ぼかしてから明るくする → にじんだ中間値が持ち上がって飽和する
    //   明るくしてからぼかす   → 先に飽和しても白黒は変わらず、にじみは同じ
    // で差が出る。
    let blur = PostEffect::Blur { radius: BLUR_RADIUS };
    let brighten = PostEffect::ColorGrade {
        exposure: 4.0,
        saturation: 1.0,
        tint: [1.0; 4],
    };

    let mut chain = PostChain::new();
    chain.push(blur);
    chain.push(brighten);
    let pixels = harness.run(&device, &queue, &chain)?;
    preview.capture("[Blur, ColorGrade]", SIZE, SIZE, &pixels);
    let blur_first = red(&pixels, OUTSIDE);
    println!("5. [Blur, ColorGrade]        外={blur_first}");

    let mut chain = PostChain::new();
    chain.push(brighten);
    chain.push(blur);
    let pixels = harness.run(&device, &queue, &chain)?;
    preview.capture("[ColorGrade, Blur]", SIZE, SIZE, &pixels);
    let grade_first = red(&pixels, OUTSIDE);
    println!("6. [ColorGrade, Blur]        外={grade_first}");

    assert!(
        blur_first > grade_first,
        "ぼかしてから明るくするほうが強く出るはず（{blur_first} vs {grade_first}）",
    );

    // 7. パス数の数え方。ぼかしは 2 パス使う。
    let mut chain = PostChain::new();
    chain.push(blur);
    chain.push(brighten);
    println!("7. パス数                    {}", chain.pass_count());
    assert_eq!(chain.pass_count(), 3, "ぼかし 2 + 色 1");

    println!("\nOK: 画面全体のエフェクトが効き、積んだ順が結果を変えている");

    preview.show()?;

    Ok(())
}

/// 場面を 1 度だけ組んで、鎖を差し替えながら描き直す。
struct Harness {
    processor: PostProcessor,
    draw_manager: DrawManager,
    destination: wgpu::Texture,
    destination_view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

impl Harness {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, Box<dyn Error>> {
        let mut draw_manager =
            DrawManager::new(device, queue, FORMAT, &DrawManagerDescriptor::default())?;

        let (left, top, right, bottom) = SQUARE;
        let mut square = object::create_object("Square");
        square.begin(PaintType::Fill);
        for [x, y] in [[left, top], [right, top], [right, bottom], [left, bottom]] {
            square.put_vertex(Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0));
        }
        square.end();
        square.camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32));
        square.instance(instance::create_instance());

        draw_manager.register(square);
        draw_manager.prepare(device, queue)?;

        let destination = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("destination"),
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

        Ok(Self {
            processor: PostProcessor::new(device, FORMAT.into()),
            draw_manager,
            destination_view: destination
                .create_view(&wgpu::TextureViewDescriptor::default()),
            destination,
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: (SIZE * SIZE * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        })
    }

    /// 場面を描いて鎖を通し、結果を読み戻す。
    fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        chain: &PostChain,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        let scene_view = self
            .processor
            .scene_view(device, SIZE, SIZE, FORMAT.into())
            .clone();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("post test"),
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

            self.draw_manager.draw(&mut render_pass);
        }

        self.processor
            .run(queue, &mut encoder, chain, &self.destination_view);

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.destination,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
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

        self.readback.map_async(wgpu::MapMode::Read, .., |_| {});
        device.poll(wgpu::PollType::wait_indefinitely())?;

        let view = self.readback.get_mapped_range(..)?;
        let pixels = view.to_vec();
        drop(view);
        self.readback.unmap();

        Ok(pixels)
    }
}

/// その座標の赤成分。読み戻したバイト列は BGRA の並び。
fn red(pixels: &[u8], (x, y): (u32, u32)) -> u8 {
    pixels[((y * SIZE + x) * 4 + 2) as usize]
}
