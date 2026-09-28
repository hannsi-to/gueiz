//! 図形ごとのエフェクトを **GPU に描かせて、画素を読み戻して**確かめる。
//!
//! ぼかしは隣の画素を読むので、断片シェーダでは図形ごとに書けません。
//! **同じエフェクトが続く図形をまとめて 1 枚に描き、それに掛けてから重ねる**、
//! というのが答えで、その筋道が通っているかをここで確かめます。
//!
//! GPU が無ければ何もせずに終わります。**飛ばしたことは出力に残します**。
//!
//! ```sh
//! cargo test -p gueiz-2d --test layer_gpu -- --nocapture
//! ```

use std::sync::OnceLock;

use gueiz_2d::camera::Camera;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::instance;
use gueiz_2d::object::{Object, create_object};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::post::{PostEffect, PostProcessor};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

const LEFT: (f32, f32) = (40.0, 100.0);
const RIGHT: (f32, f32) = (156.0, 100.0);
const MIDDLE: (f32, f32) = (128.0, 128.0);
const BOX_SIZE: f32 = 60.0;

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const RED: [f32; 3] = [1.0, 0.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];

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
            label: Some("layer gpu test"),
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
                eprintln!("GPU アダプタが取れないので飛ばします");
                return;
            }
        }
    };
}

/// 四角 1 つ。角が升目に乗るので、ぼけたかどうかが縁で分かる。
fn square(color: [f32; 3], (x, y): (f32, f32)) -> Object {
    let mut object = create_object("square");
    let half = BOX_SIZE / 2.0;

    object.begin(PaintType::Fill);
    for [dx, dy] in [[-half, -half], [half, -half], [half, half], [-half, half]] {
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

    object.camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32));
    object.instance(instance::create_instance());
    object
}

/// ぼかしを掛けた四角。
fn blurry(color: [f32; 3], at: (f32, f32), radius: f32) -> Object {
    let mut object = square(color, at);
    object.post_effect(PostEffect::Blur { radius });
    object
}

fn manager() -> DrawManager {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");

    DrawManager::new(
        &gpu.device,
        &gpu.queue,
        FORMAT,
        &DrawManagerDescriptor::default(),
    )
    .expect("DrawManager を作れなかった")
}

fn prepared(manager: &mut DrawManager) {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");

    manager
        .prepare(&gpu.device, &gpu.queue)
        .expect("支度に失敗した");
}

fn value(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIZE + x) * 4) as usize + 1]
}

fn red_of(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIZE + x) * 4) as usize + 2]
}

/// 縁を 1 行横切って、半端な明るさの画素が何枚続くか。ぼけているほど長い。
///
/// `channel` は読み戻しの並び（BGRA）での位置。1 が緑、2 が赤。
fn edge_width(pixels: &[u8], row: u32, from: u32, to: u32, channel: usize) -> u32 {
    let mut widest = 0;
    let mut run = 0;

    for x in from..to {
        let at = ((row * SIZE + x) * 4) as usize + channel;

        if (16..240).contains(&pixels[at]) {
            run += 1;
            widest = widest.max(run);
        } else {
            run = 0;
        }
    }

    widest
}

/// まとまりごとに描いて、その鎖を通してから重ねる。**これが答えの筋道そのもの。**
///
/// 掛けるものは図形が持っているので、ここで表を引く必要はありません。
fn render(draw_manager: &mut DrawManager) -> Vec<u8> {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");
    let (device, queue) = (&gpu.device, &gpu.queue);

    prepared(draw_manager);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("layer gpu test target"),
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
    let screen = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("layer gpu test readback"),
        size: (SIZE * SIZE * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut processor = PostProcessor::new(device, FORMAT.into());
    processor.begin_frame();

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("layer gpu test"),
    });

    // 積む先を黒で消しておく。
    {
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &screen,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }

    let groups: Vec<_> = draw_manager
        .passes()
        .map(|pass| (pass.range.clone(), pass.chain.clone()))
        .collect();

    for (range, chain) in groups {
        let scene = processor
            .scene_view(device, SIZE, SIZE, FORMAT.into())
            .clone();

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pass group"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scene,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // まとまりごとに透明で消す。前のを引きずらない。
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            draw_manager.draw_range(&mut pass, range);
        }

        processor.run_over(queue, &mut encoder, &chain, &screen);
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

    queue.submit(Some(encoder.finish()));

    readback.map_async(wgpu::MapMode::Read, .., |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("GPU を待てなかった");

    let mapped = readback.get_mapped_range(..).expect("読み戻せなかった");
    let pixels = mapped.to_vec();
    drop(mapped);
    readback.unmap();

    pixels
}

// ---- まとまりの切れ方 ----

/// 掛けるものが同じなら 1 回で描く。図形を増やしてもパスは増えない。
#[test]
fn shapes_with_the_same_effects_share_one_pass() {
    gpu!();

    let mut manager = manager();
    for at in [LEFT, RIGHT, MIDDLE] {
        manager.register(square(WHITE, at));
    }

    prepared(&mut manager);

    let ranges: Vec<_> = manager.passes().map(|pass| pass.range).collect();

    assert_eq!(ranges, vec![0..3], "同じなのに分かれている");
}

/// 掛けるものが変わったところで切れる。
#[test]
fn a_different_effect_splits_the_pass() {
    gpu!();

    let mut manager = manager();
    manager.register(square(WHITE, LEFT));
    manager.register(blurry(WHITE, RIGHT, 4.0));

    prepared(&mut manager);

    let passes: Vec<_> = manager
        .passes()
        .map(|pass| (pass.range, pass.chain.effects().len()))
        .collect();

    assert_eq!(passes, vec![(0..1, 0), (1..2, 1)]);
}

/// **同じ種類でも、設定が違えば別のパス。**
#[test]
fn a_different_radius_splits_the_pass_too() {
    gpu!();

    let mut manager = manager();
    manager.register(blurry(WHITE, LEFT, 4.0));
    manager.register(blurry(WHITE, RIGHT, 9.0));

    prepared(&mut manager);

    assert_eq!(manager.passes().count(), 2);
}

/// 掛ける・掛けないが交互だと、その数だけパスが出る。**費用はここに出る。**
#[test]
fn alternating_effects_cost_a_pass_each() {
    gpu!();

    let mut manager = manager();

    for index in 0..4 {
        let at = (40.0 + index as f32 * 50.0, 100.0);

        let mut object = if index % 2 == 0 {
            square(WHITE, at)
        } else {
            blurry(WHITE, at, 4.0)
        };

        object.z(index as f32);
        manager.register(object);
    }

    prepared(&mut manager);

    assert_eq!(manager.passes().count(), 4, "交互なら 4 回");
}

/// 交互で困るなら、層でまとめれば 2 回に落ちる。**`layer` が残っている理由。**
#[test]
fn the_layer_can_group_shapes_that_are_not_adjacent() {
    gpu!();

    let mut manager = manager();

    for index in 0..4 {
        let at = (40.0 + index as f32 * 50.0, 100.0);
        let blurred = index % 2 == 1;

        let mut object = if blurred {
            blurry(WHITE, at, 4.0)
        } else {
            square(WHITE, at)
        };

        object.z(index as f32);
        object.layer(u32::from(blurred));
        manager.register(object);
    }

    prepared(&mut manager);

    assert_eq!(manager.passes().count(), 2, "層でまとめれば 2 回");
}

/// 層が先、その中で z。
#[test]
fn the_layer_beats_the_depth_when_sorting() {
    gpu!();

    let mut manager = manager();

    let mut deep = square(WHITE, LEFT);
    deep.layer(1);
    deep.z(-10.0);
    manager.register(deep);

    let mut shallow = square(WHITE, RIGHT);
    shallow.layer(0);
    shallow.z(10.0);
    manager.register(shallow);

    prepared(&mut manager);

    let layers: Vec<_> = manager.passes().map(|pass| pass.layer).collect();

    assert_eq!(layers, vec![0, 1]);
}

// ---- 図形ごとのぼかし ----

/// **これが答えそのもの。** 片方にだけ掛ければ、片方だけぼける。
#[test]
fn a_blur_can_be_confined_to_one_shape() {
    gpu!();

    let mut manager = manager();
    manager.register(square(WHITE, LEFT));
    manager.register(blurry(WHITE, RIGHT, 6.0));

    let pixels = render(&mut manager);

    // 窓は**相手のぼけが届かないところ**で切る。ぼけは半径の 4 倍ほど広がる。
    let sharp = edge_width(&pixels, 100, 0, 90, 1);
    let soft = edge_width(&pixels, 100, 90, SIZE, 1);

    eprintln!("縁のぼけ: 掛けないほう {sharp} 画素、掛けたほう {soft} 画素");

    assert!(sharp <= 2, "掛けていないほうまでぼけている（{sharp} 画素）");
    assert!(soft > sharp * 3, "掛けたほうがぼけていない（{soft} 画素）");

    assert!(value(&pixels, LEFT.0 as u32, LEFT.1 as u32) > 200, "左が消えた");
    assert!(value(&pixels, RIGHT.0 as u32, RIGHT.1 as u32) > 150, "右が消えた");
}

/// 全部に掛けたときと違うこと。違わなければ図形ごとにした意味がない。
#[test]
fn a_shape_blur_differs_from_blurring_everything() {
    gpu!();

    let mut one = manager();
    one.register(square(WHITE, LEFT));
    one.register(blurry(WHITE, RIGHT, 6.0));
    let split = render(&mut one);

    let mut all = manager();
    all.register(blurry(WHITE, LEFT, 6.0));
    all.register(blurry(WHITE, RIGHT, 6.0));
    let whole = render(&mut all);

    let left_split = edge_width(&split, 100, 0, 90, 1);
    let left_whole = edge_width(&whole, 100, 0, 90, 1);

    eprintln!("左の縁: 片方だけなら {left_split} 画素、全部なら {left_whole} 画素");

    assert!(
        left_whole > left_split * 3,
        "全部に掛けたのに左がぼけていない（{left_whole} と {left_split}）",
    );
}

/// **重なっていても、掛けた図形だけがぼける。**
///
/// 隣り合うだけだと、ぼかしは足し算なので合計がほぼ同じになり差が出ません。
/// 重ねたときこそ違いがはっきり出ます。
#[test]
fn only_the_chosen_shape_blurs_where_they_overlap() {
    gpu!();

    // 下は青。**赤を含まない色にしないと**、ぼけた下の図形が赤の測りに混ざる。
    let mut split = manager();
    split.register(blurry(BLUE, MIDDLE, 8.0));
    let mut sharp = square(RED, MIDDLE);
    sharp.z(1.0);
    split.register(sharp);
    let split = render(&mut split);

    let mut together = manager();
    together.register(blurry(BLUE, MIDDLE, 8.0));
    let mut also = blurry(RED, MIDDLE, 8.0);
    also.z(1.0);
    together.register(also);
    let together = render(&mut together);

    let (apart, mixed) = (
        edge_width(&split, 100, 0, SIZE, 2),
        edge_width(&together, 100, 0, SIZE, 2),
    );

    eprintln!("赤の縁: 掛けなければ {apart} 画素、掛ければ {mixed} 画素");

    assert!(apart <= 2, "掛けていない赤までぼけている（{apart} 画素）");
    assert!(mixed > 10, "掛けた赤がぼけていない（{mixed} 画素）");
}

// ---- 重ね方 ----

/// 何も掛けていない図形も、ちゃんと積まれること。
#[test]
fn a_shape_without_effects_still_composites() {
    gpu!();

    let mut manager = manager();
    manager.register(square(WHITE, LEFT));
    manager.register(square(WHITE, RIGHT));

    let pixels = render(&mut manager);

    assert!(value(&pixels, LEFT.0 as u32, LEFT.1 as u32) > 200, "左が出ていない");
    assert!(value(&pixels, RIGHT.0 as u32, RIGHT.1 as u32) > 200, "右が出ていない");
    assert!(value(&pixels, 10, 10) < 20, "何も無いところが塗られている");
}

/// 後のものが上に乗ること。まとまりが分かれても順番は崩れない。
#[test]
fn a_later_shape_lands_on_top() {
    gpu!();

    let mut manager = manager();

    manager.register(square(WHITE, MIDDLE));

    // 掛けるものが違うのでパスが分かれる。それでも上に乗る。
    let mut over = blurry(RED, MIDDLE, 2.0);
    over.z(1.0);
    manager.register(over);

    let pixels = render(&mut manager);

    assert!(manager.passes().count() >= 2, "分かれていないと試験にならない");
    assert!(red_of(&pixels, 128, 128) > 200, "赤が出ていない");
    assert!(value(&pixels, 128, 128) < 40, "白が残っている。上に乗っていない");
}
