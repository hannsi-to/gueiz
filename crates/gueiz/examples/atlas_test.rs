//! 大きさの違う絵をアトラスに詰め込んで、**描いた画素で**確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 見るのは 6 つ。
//!
//! - 大きさの違う絵が 1 枚のシートに入ること
//! - 上げたあとも**取っ手が生きている**こと（前は差した瞬間に消えていた）
//! - **透明なところに色がにじんで**、縁が黒ずまないこと
//! - 縮小用の段を作ると、余白が自動で広がること
//! - あとから足しても、前のぶんが残ること
//! - **実行中に消すと場所が戻り、次の絵がそこに入る**こと
//!
//! ```sh
//! cargo run -p gueiz --example atlas_test
//! cargo run -p gueiz --example atlas_test -- --window
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::atlas::{AtlasDescriptor, MipLevels};
use gueiz_2d::camera::Camera;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::resource::Resources;
use gueiz_2d::sprite::SpriteFilter;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("atlas test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("adapter: {}\n", adapter.get_info().name);

    let target = Target::new(&device);
    let mut preview = Preview::new("atlas_test");

    // 1. 大きさの違う絵を 4 枚。ページは 1 枚で足りるはず。
    let mut resources = Resources::new();
    resources.set_atlas_descriptor(AtlasDescriptor {
        page_size: 256,
        padding: 1,
        filter: SpriteFilter::Nearest,
        ..Default::default()
    });

    let red = resources.load_texture("red", 64, 64, &solid(64, 64, [255, 0, 0, 255]))?;
    let green = resources.load_texture("green", 32, 96, &solid(32, 96, [0, 255, 0, 255]))?;
    let blue = resources.load_texture("blue", 96, 16, &solid(96, 16, [0, 0, 255, 255]))?;
    let white = resources.load_texture("white", 8, 8, &solid(8, 8, [255, 255, 255, 255]))?;

    println!("1. 預ける                   {} 枚、まだ GPU に上げていない", resources.pending_texture_count());
    assert_eq!(resources.pending_texture_count(), 4);
    assert!(resources.texture(red).is_none(), "確定前は場所が無い");

    let mut draw_manager =
        DrawManager::new(&device, &queue, FORMAT, &DrawManagerDescriptor::default())?;
    resources.commit_textures(&device, &queue, &mut draw_manager)?;

    let atlas = resources.atlas().expect("できている");
    println!(
        "2. 詰め込む                 {} ページ、{:.0}% 埋まる、残り {} 枚",
        atlas.page_count(),
        atlas.occupancy() * 100.0,
        resources.pending_texture_count(),
    );
    assert_eq!(atlas.page_count(), 1, "1 ページに収まるはず");
    assert_eq!(resources.pending_texture_count(), 0);

    // 3. **取っ手が生きている。** ここが前は死んでいた。
    let region = resources.texture(red).expect("上げたのに引けない");
    println!(
        "3. 取っ手が生きる           red は {} ページ目の {:?}",
        region.page, region.size,
    );
    assert_eq!(region.size, [64, 64]);
    assert!(resources.texture(green).is_some());
    assert!(resources.texture_named("blue").is_some());

    // 4. 4 枚とも正しい色で出て、**ドローは 1 回**。
    let pixels = render(&device, &queue, &target, &mut draw_manager, &resources, &[
        (red, 16.0, 16.0),
        (green, 96.0, 16.0),
        (blue, 16.0, 136.0),
        (white, 160.0, 136.0),
    ])?;
    preview.capture("4. 大きさ違いを 4 枚", SIZE, SIZE, &pixels);

    let found = [
        ("red", pixel(&pixels, 48, 48), [255, 0, 0]),
        ("green", pixel(&pixels, 112, 64), [0, 255, 0]),
        ("blue", pixel(&pixels, 64, 144), [0, 0, 255]),
        ("white", pixel(&pixels, 164, 140), [255, 255, 255]),
    ];

    println!("4. 4 枚とも貼れる           {}", found
        .iter()
        .map(|(name, got, _)| format!("{name}={got:?}"))
        .collect::<Vec<_>>()
        .join(" "));

    for (name, got, want) in found {
        assert_eq!(got, want, "{name} の色が違う");
    }
    assert_eq!(draw_manager.draw_count(), 4, "図形ごとに 1 ドロー");

    // 5. **にじませ。** 透明な画素の RGB を隣から埋めておくと、
    //    線形補間で縁が黒ずまない。
    //
    //    16 画素の絵を 128 画素に引き伸ばして、**不透明と透明の境目**を見る。
    //    境目はちょうど半々に混ざるので、
    //
    //    画面 x=80 の画素は、透明側に 0.44 寄ったところを読む。
    //
    //    - にじませてある: 赤 1.0、覆い 0.44 → 出力 0.44 → **112**
    //    - にじませない  : 赤 0.44、覆い 0.44 → 出力 0.19 → **49**
    //
    //    倍以上の差が付くので、効いているかどうかがはっきり出る。
    let mut resources = Resources::new();
    resources.set_atlas_descriptor(AtlasDescriptor {
        page_size: 64,
        padding: 1,
        // 混ぜないと、にじませの有無が見えない。
        filter: SpriteFilter::Linear,
        ..Default::default()
    });

    // 左半分が不透明な赤、右半分が「透明な黒」。
    let mut half = Vec::new();
    for _ in 0..16 {
        for x in 0..16 {
            half.extend_from_slice(if x < 8 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }

    let faded = resources.load_texture("faded", 16, 16, &half)?;
    let mut draw_manager =
        DrawManager::new(&device, &queue, FORMAT, &DrawManagerDescriptor::default())?;
    resources.commit_textures(&device, &queue, &mut draw_manager)?;

    // (16,16) に 128x128 で引き伸ばす。境目は画面の x = 80。
    let pixels = render_scaled(
        &device,
        &queue,
        &target,
        &mut draw_manager,
        &resources,
        faded,
        16.0,
        16.0,
        128.0,
    )?;
    preview.capture("5. 透明側に色をにじませる", SIZE, SIZE, &pixels);

    let solid_side = pixel(&pixels, 40, 80);
    let boundary = pixel(&pixels, 80, 80);
    println!(
        "5. にじませ                 不透明側 {solid_side:?}、境目 {boundary:?}（にじませないと赤は 49）",
    );

    assert_eq!(solid_side, [255, 0, 0], "不透明側が赤でない");
    assert!(
        boundary[0] > 100,
        "境目が黒ずんでいる（にじませが効いていない）: {boundary:?}",
    );
    assert!(boundary[1] < 20 && boundary[2] < 20, "赤以外が出ている: {boundary:?}");

    // 6. 縮小用の段。段を作ると余白が自動で広がること。
    let mut resources = Resources::new();
    resources.set_atlas_descriptor(AtlasDescriptor {
        page_size: 256,
        padding: 1,
        mip_levels: MipLevels::Count(4),
        ..Default::default()
    });

    let checker = resources.load_texture("checker", 64, 64, &checkerboard(64))?;
    let mut draw_manager =
        DrawManager::new(&device, &queue, FORMAT, &DrawManagerDescriptor::default())?;
    resources.commit_textures(&device, &queue, &mut draw_manager)?;

    let atlas = resources.atlas().expect("できている");
    println!(
        "6. 縮小用の段               {} 段、余白 {} 画素（1 を指定）",
        atlas.mip_level_count(),
        atlas.padding(),
    );
    assert_eq!(atlas.mip_level_count(), 4);
    assert_eq!(atlas.padding(), 8, "段のぶん余白が広がるはず");

    let pixels = render(
        &device,
        &queue,
        &target,
        &mut draw_manager,
        &resources,
        &[(checker, 96.0, 96.0)],
    )?;
    preview.capture("6. 縮小用の段つき", SIZE, SIZE, &pixels);
    assert!(lit_count(&pixels) > 0, "段を作ったら描けなくなった");

    // 7. あとから足しても、前のぶんは残ること。
    let stripe = resources.load_texture("stripe", 32, 32, &solid(32, 32, [255, 0, 255, 255]))?;
    resources.commit_textures(&device, &queue, &mut draw_manager)?;

    println!(
        "7. あとから足す             checker {}、stripe {}",
        resources.texture(checker).is_some(),
        resources.texture(stripe).is_some(),
    );
    assert!(resources.texture(checker).is_some(), "前のぶんが消えた");
    assert!(resources.texture(stripe).is_some());

    let pixels = render(
        &device,
        &queue,
        &target,
        &mut draw_manager,
        &resources,
        &[(checker, 32.0, 96.0), (stripe, 160.0, 112.0)],
    )?;
    preview.capture("7. あとから足す", SIZE, SIZE, &pixels);
    assert_eq!(pixel(&pixels, 176, 128), [255, 0, 255], "足したほうが出ていない");
    assert!(lit_count(&pixels) > 0, "前のほうが消えた");

    // 8. 実行中の出し入れ。消した場所が戻って、次の絵がそこに入ること。
    //    戻らないと、出し入れを繰り返すたびにページが伸びます。
    let mut resources = Resources::new();
    resources.set_atlas_descriptor(AtlasDescriptor {
        page_size: 128,
        padding: 1,
        filter: SpriteFilter::Nearest,
        ..Default::default()
    });

    let mut draw_manager =
        DrawManager::new(&device, &queue, FORMAT, &DrawManagerDescriptor::default())?;

    let doomed = resources.load_texture("doomed", 64, 64, &solid(64, 64, [255, 0, 0, 255]))?;
    let keeper = resources.load_texture("keeper", 64, 64, &solid(64, 64, [0, 255, 0, 255]))?;
    resources.commit_textures(&device, &queue, &mut draw_manager)?;

    let vacated = resources.texture(doomed).expect("上げてある");
    let pages_before = resources.atlas().expect("ある").page_count();

    // 消す。取っ手はその場で死ぬ。
    assert!(resources.remove_texture(doomed));
    assert_eq!(resources.texture(doomed), None, "消した取っ手で引けている");
    assert!(resources.texture(keeper).is_some(), "隣まで消えた");

    // 同じ大きさを足すと、空いたところに入るはず。
    let replacement =
        resources.load_texture("replacement", 64, 64, &solid(64, 64, [0, 0, 255, 255]))?;
    resources.commit_textures(&device, &queue, &mut draw_manager)?;

    let reused = resources.texture(replacement).expect("上げてある");
    let atlas = resources.atlas().expect("ある");

    println!(
        "8. 実行中に消して足す       空けた {:?} → 入った {:?}、ページ {} のまま",
        (vacated.page, vacated.uv_rect[0]),
        (reused.page, reused.uv_rect[0]),
        atlas.page_count(),
    );

    assert_eq!(reused.page, vacated.page, "別のページに行った");
    assert_eq!(reused.uv_rect, vacated.uv_rect, "空いた場所を使っていない");
    assert_eq!(atlas.page_count(), pages_before, "ページが増えた");
    assert_eq!(atlas.len(), 2, "枚数が合わない");

    // 画素まで届いていること。消したほうの赤はもう出ず、青が出る。
    let pixels = render(&device, &queue, &target, &mut draw_manager, &resources, &[
        (replacement, 16.0, 16.0),
        (keeper, 96.0, 96.0),
    ])?;
    preview.capture("8. 消して足す", SIZE, SIZE, &pixels);

    let at_replacement = pixel(&pixels, 48, 48);
    let at_keeper = pixel(&pixels, 128, 128);
    println!("                            新しい絵 {at_replacement:?}、残したほう {at_keeper:?}");

    assert_eq!(at_replacement, [0, 0, 255], "空いた場所に古い赤が残っている");
    assert_eq!(at_keeper, [0, 255, 0], "残したほうが壊れた");

    // 9. 出し入れを 50 回繰り返しても、ページが伸びないこと。
    //    場所が戻っていなければ、繰り返すたびに新しい場所を取って伸びる。
    let pages_before_churn = resources.atlas().expect("ある").page_count();

    for round in 0..50 {
        if let Some(handle) = resources.texture_handle("churn") {
            resources.remove_texture(handle);
        }

        let shade = (round * 5) as u8;
        resources.load_texture("churn", 32, 32, &solid(32, 32, [shade, shade, shade, 255]))?;
        resources.commit_textures(&device, &queue, &mut draw_manager)?;
    }

    let atlas = resources.atlas().expect("ある");
    println!(
        "9. 50 回出し入れ            ページ {} のまま（前 {}）、{} 枚、{:.0}% 埋まる",
        atlas.page_count(),
        pages_before_churn,
        atlas.len(),
        atlas.occupancy() * 100.0,
    );

    assert_eq!(
        atlas.page_count(),
        pages_before_churn,
        "出し入れでページが増えている",
    );
    assert_eq!(atlas.len(), 3, "消したぶんが残っている");

    println!("\nすべて通りました。");

    if common::preview::window_requested() {
        preview.show()?;
    } else {
        println!("`-- --window` を付けると描いた結果を窓に出します。");
    }

    Ok(())
}

// --- 絵を作る ---

fn solid(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    color
        .iter()
        .copied()
        .cycle()
        .take((width * height * 4) as usize)
        .collect()
}

fn checkerboard(side: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((side * side * 4) as usize);

    for y in 0..side {
        for x in 0..side {
            let dark = (x / 4 + y / 4) % 2 == 0;
            pixels.extend_from_slice(if dark {
                &[32, 32, 32, 255]
            } else {
                &[224, 224, 224, 255]
            });
        }
    }

    pixels
}

// --- 描く ---

/// 貼った絵を 1 枚ずつ四角に貼って描く。
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    draw_manager: &mut DrawManager,
    resources: &Resources,
    placements: &[(gueiz_2d::resource::TextureHandle, f32, f32)],
) -> Result<Vec<u8>, Box<dyn Error>> {
    // 図形は毎回組み直す。絵の差し替えだけを見たいので。
    let camera = Camera::orthographic_2d(SIZE as f32, SIZE as f32);

    // 名前は登録先でひとつに保たれるので、重ねると付け替えられて警告が出る。
    // この関数は何度も呼ばれるので、通し番号は登録済みの数から取る。
    for (handle, x, y) in placements {
        let region = resources.texture(*handle).expect("上げてある");
        let [width, height] = region.size;

        let mut quad =
            object::create_object(&format!("Sprite {}", draw_manager.object_count()));
        quad.begin(PaintType::Fill);

        for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            quad.put_vertex(Vertex::new_position_color_uv_normal(
                x + dx * width as f32,
                y + dy * height as f32,
                0.0,
                1.0,
                1.0,
                1.0,
                1.0,
                dx,
                dy,
                0.0,
                0.0,
                1.0,
            ));
        }

        quad.end();
        quad.camera(camera);
        quad.sprite_texture(region);
        quad.instance(instance::create_instance());

        draw_manager.register(quad);
    }

    present(device, queue, target, draw_manager)
}

/// 積んだものを描いて、画素を読み戻す。
fn present(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    draw_manager: &mut DrawManager,
) -> Result<Vec<u8>, Box<dyn Error>> {
    draw_manager.prepare(device, queue)?;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("atlas test"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sprites"),
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

/// 1 枚だけを、決めた大きさに引き伸ばして描く。
#[allow(clippy::too_many_arguments)]
fn render_scaled(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    draw_manager: &mut DrawManager,
    resources: &Resources,
    handle: gueiz_2d::resource::TextureHandle,
    x: f32,
    y: f32,
    side: f32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let region = resources.texture(handle).expect("上げてある");
    let camera = Camera::orthographic_2d(SIZE as f32, SIZE as f32);

    let mut quad =
        object::create_object(&format!("Scaled {}", draw_manager.object_count()));
    quad.begin(PaintType::Fill);

    for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        quad.put_vertex(Vertex::new_position_color_uv_normal(
            x + dx * side,
            y + dy * side,
            0.0,
            1.0,
            1.0,
            1.0,
            1.0,
            dx,
            dy,
            0.0,
            0.0,
            1.0,
        ));
    }

    quad.end();
    quad.camera(camera);
    quad.sprite_texture(region);
    quad.instance(instance::create_instance());

    draw_manager.register(quad);

    present(device, queue, target, draw_manager)
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

/// BGRA で並んでいるので、赤緑青の順に並べ替えて返す。
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let at = ((y * SIZE + x) * 4) as usize;

    [pixels[at + 2], pixels[at + 1], pixels[at]]
}

fn lit_count(pixels: &[u8]) -> usize {
    (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .filter(|(x, y)| pixel(pixels, *x, *y).iter().any(|channel| *channel > 16))
        .count()
}
