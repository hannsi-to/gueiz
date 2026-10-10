//! 多段グラデーションを **GPU に描かせて、画素を読み戻して**確かめる。
//!
//! WGSL はパイプラインを組むときにしか検かめられないので、
//! 組み立てが噛み合っているかはここでしか分かりません。
//!
//! GPU が無ければ何もせずに終わります。**飛ばしたことは出力に残します**。
//!
//! ```sh
//! cargo test -p gueiz-2d --test gradient_gpu -- --nocapture
//! ```

use std::sync::OnceLock;

use gueiz_2d::camera::Camera;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::effect::{Block, Gradient, GradientSpread};
use gueiz_2d::instance;
use gueiz_2d::object::{Object, create_object};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

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
            label: Some("gradient gpu test"),
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

/// 画面いっぱいの白い四角。掛け算なので、白ならグラデーションの色がそのまま出る。
fn canvas() -> Object {
    let full = SIZE as f32;
    let mut object = create_object("Canvas");

    object.begin(PaintType::Fill);
    for [x, y] in [[0.0, 0.0], [full, 0.0], [full, full], [0.0, full]] {
        object.put_vertex(Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0));
    }
    object.end();

    object.camera(Camera::orthographic_2d(full, full));
    object.instance(instance::create_instance());
    object
}

fn painted(blocks: &[Block]) -> Vec<u8> {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");
    let mut manager = DrawManager::new(
        &gpu.device,
        &gpu.queue,
        FORMAT,
        &DrawManagerDescriptor::default(),
    )
    .expect("DrawManager を作れなかった");

    let mut object = canvas();
    for block in blocks {
        object.effect(*block);
    }
    manager.register(object);

    render(&mut manager)
}

/// その画素の色。読み戻しは BGRA の並び。
fn rgb(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let at = ((y * SIZE + x) * 4) as usize;

    [pixels[at + 2], pixels[at + 1], pixels[at]]
}

/// sRGB で書かれた値を光の量に戻す。
fn linear(value: u8) -> f32 {
    let value = value as f32 / 255.0;

    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// 思った色に近いか。
///
/// **光の量で比べます。** sRGB は暗い側を大きく引き伸ばすので、
/// そのまま比べると「ほんのわずかな混ざり」が 40 くらいの差に見えてしまいます。
fn assert_rgb(got: [u8; 3], want: [u8; 3], note: &str) {
    let off = (0..3).any(|index| (linear(got[index]) - linear(want[index])).abs() > 0.03);

    assert!(!off, "{note}: {got:?} は {want:?} のはず");
}

fn render(draw_manager: &mut DrawManager) -> Vec<u8> {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");
    let (device, queue) = (&gpu.device, &gpu.queue);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gradient gpu test target"),
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

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gradient gpu test readback"),
        size: (SIZE * SIZE * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    draw_manager.prepare().expect("支度に失敗した");

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gradient gpu test"),
    });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gradient gpu test pass"),
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

        draw_manager.draw(&mut pass);
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

// ---- 線形 ----

#[test]
fn a_linear_gradient_runs_left_to_right() {
    gpu!();

    let pixels = painted(&[Block::GradientStops {
        gradient: Gradient::linear(0.0)
            .stop(0.0, RED)
            .stop(0.5, GREEN)
            .stop(1.0, BLUE),
    }]);

    assert_rgb(rgb(&pixels, 2, 128), [255, 0, 0], "左端は赤");
    assert_rgb(rgb(&pixels, 128, 128), [0, 255, 0], "真ん中は緑");
    assert_rgb(rgb(&pixels, 253, 128), [0, 0, 255], "右端は青");
}

/// **3 色目が効いていること。** 2 色しか読めていないと真ん中が赤青の混ざりになる。
#[test]
fn the_middle_stop_actually_shows() {
    gpu!();

    let three = painted(&[Block::GradientStops {
        gradient: Gradient::linear(0.0)
            .stop(0.0, RED)
            .stop(0.5, GREEN)
            .stop(1.0, BLUE),
    }]);

    let two = painted(&[Block::GradientStops {
        gradient: Gradient::linear(0.0).stop(0.0, RED).stop(1.0, BLUE),
    }]);

    assert!(rgb(&three, 128, 128)[1] > 200, "真ん中に緑が出ていない");
    assert!(rgb(&two, 128, 128)[1] < 40, "2 色なのに緑が出ている");
}

#[test]
fn the_angle_turns_the_gradient() {
    gpu!();

    // 90 度。上から下へ流れる。
    let pixels = painted(&[Block::GradientStops {
        gradient: Gradient::linear(std::f32::consts::FRAC_PI_2)
            .stop(0.0, RED)
            .stop(1.0, BLUE),
    }]);

    assert_rgb(rgb(&pixels, 128, 2), [255, 0, 0], "上は赤");
    assert_rgb(rgb(&pixels, 128, 253), [0, 0, 255], "下は青");
    // 横には変わらない。
    assert_eq!(rgb(&pixels, 10, 128), rgb(&pixels, 246, 128));
}

/// 色は線形で混ぜる。真ん中の灰色が sRGB の 128 ではなく 188 になる。
#[test]
fn the_blend_happens_in_linear_light() {
    gpu!();

    let pixels = painted(&[Block::GradientStops {
        gradient: Gradient::linear(0.0)
            .stop(0.0, [0.0, 0.0, 0.0, 1.0])
            .stop(1.0, [1.0, 1.0, 1.0, 1.0]),
    }]);

    let middle = rgb(&pixels, 128, 128)[0];

    assert!(
        middle.abs_diff(188) <= 6,
        "{middle} は 188 前後のはず（線形で混ぜて sRGB で書き出す）",
    );
}

// ---- 放射 ----

#[test]
fn a_radial_gradient_spreads_from_its_centre() {
    gpu!();

    let pixels = painted(&[Block::GradientStops {
        gradient: Gradient::radial([0.5, 0.5], 0.5, 0.5)
            .stop(0.0, RED)
            .stop(1.0, BLUE),
    }]);

    assert_rgb(rgb(&pixels, 128, 128), [255, 0, 0], "中心は赤");

    // 中心から等しく離れた 4 点は同じ色。
    let right = rgb(&pixels, 228, 128);
    for (x, y, note) in [(28, 128, "左"), (128, 28, "上"), (128, 228, "下")] {
        assert_rgb(rgb(&pixels, x, y), right, note);
    }
}

#[test]
fn the_radii_can_differ() {
    gpu!();

    // 横に潰した楕円。横は早く、縦はゆっくり変わる。
    let pixels = painted(&[Block::GradientStops {
        gradient: Gradient::radial([0.5, 0.5], 0.25, 0.5)
            .stop(0.0, RED)
            .stop(1.0, BLUE),
    }]);

    let sideways = rgb(&pixels, 192, 128);
    let downwards = rgb(&pixels, 128, 192);

    assert!(
        sideways[2] > downwards[2],
        "横のほうが早く青くなるはず（{sideways:?} と {downwards:?}）",
    );
}

// ---- 角度 ----

#[test]
fn a_conic_gradient_goes_round_the_centre() {
    gpu!();

    let pixels = painted(&[Block::GradientStops {
        gradient: Gradient::conic([0.5, 0.5], 0.0)
            .stop(0.0, RED)
            .stop(0.5, GREEN)
            .stop(1.0, RED),
    }]);

    // 0 度（右）が始まりで赤。半周（左）で緑。
    assert_rgb(rgb(&pixels, 240, 128), [255, 0, 0], "右は赤");
    assert_rgb(rgb(&pixels, 16, 128), [0, 255, 0], "左は緑");

    // 中心から同じ角度なら、遠さが違っても同じ色。
    assert_rgb(rgb(&pixels, 200, 128), rgb(&pixels, 240, 128), "同じ角度");
}

// ---- 0..1 の外 ----

#[test]
fn the_spread_decides_what_happens_outside() {
    gpu!();

    // 埋め方が効くのは 0..1 を外れたところだけ。線形で図形いっぱいに流すと
    // 割合は 0..1 に収まってしまうので、半径を小さくした放射で見る。
    let ramp = |spread| Block::GradientStops {
        gradient: Gradient::radial([0.5, 0.5], 0.25, 0.25)
            .spread(spread)
            .stop(0.0, RED)
            .stop(1.0, BLUE),
    };

    let clamp = painted(&[ramp(GradientSpread::Clamp)]);
    let repeat = painted(&[ramp(GradientSpread::Repeat)]);
    let mirror = painted(&[ramp(GradientSpread::Mirror)]);

    // どれも真ん中は赤。ここは 0..1 の中。
    for (pixels, note) in [(&clamp, "伸ばす"), (&repeat, "繰り返す"), (&mirror, "折り返す")] {
        assert_rgb(rgb(pixels, 128, 128), [255, 0, 0], note);
    }

    // 半径のすぐ外（割合が 1 をわずかに超えたところ）で分かれる。
    // 伸ばす → 青のまま。繰り返す → 頭に飛んで赤。折り返す → 折り返して青のまま。
    //
    // 標本を取るのは境目の近くでないと意味がない。遠くまで行くと、
    // 繰り返しも折り返しも周期の途中に来てしまう。
    assert_rgb(rgb(&clamp, 195, 128), [0, 0, 255], "伸ばすと端の色が続く");
    assert!(rgb(&repeat, 195, 128)[0] > 200, "繰り返せていない");
    assert!(rgb(&mirror, 195, 128)[2] > 200, "折り返せていない");

    // 遠くまで行っても伸ばすのは変わらない。
    assert_rgb(rgb(&clamp, 250, 128), [0, 0, 255], "伸ばすと遠くでも端の色");

    // 繰り返しは境目で色が飛ぶ。折り返しはつながる。
    let across = |pixels: &[u8]| {
        let inside = rgb(pixels, 190, 128);
        let outside = rgb(pixels, 194, 128);
        (linear(inside[0]) - linear(outside[0])).abs()
    };

    assert!(
        across(&repeat) > across(&mirror),
        "繰り返しのほうが境目で飛ぶはず（{} と {}）",
        across(&repeat),
        across(&mirror),
    );
}

// ---- ほかの山と積む ----

/// **色を収めた続きを飛ばし損ねていないか。**
///
/// グラデーションは 1 山で 2 つ以上を占める。進む数を間違えると、
/// あとに積んだ山が飛ばされる。
#[test]
fn a_gradient_does_not_swallow_the_block_after_it() {
    gpu!();

    let gradient = Block::GradientStops {
        gradient: Gradient::linear(0.0)
            .stop(0.0, [1.0, 1.0, 1.0, 1.0])
            .stop(0.3, [1.0, 1.0, 1.0, 1.0])
            .stop(0.6, [1.0, 1.0, 1.0, 1.0])
            .stop(1.0, [1.0, 1.0, 1.0, 1.0]),
    };

    // 白いグラデーションのあとに、緑を掛ける。
    let tinted = painted(&[gradient, Block::Tint { color: GREEN }]);

    assert_rgb(rgb(&tinted, 128, 128), [0, 255, 0], "あとの山が効いていない");
}

/// 山を積む順がそのまま効く順。
#[test]
fn a_gradient_multiplies_what_came_before() {
    gpu!();

    let half = [0.5, 0.5, 0.5, 1.0];

    let pixels = painted(&[
        Block::Tint { color: half },
        Block::GradientStops {
            gradient: Gradient::linear(0.0).stop(0.0, RED).stop(1.0, RED),
        },
    ]);

    // 掛けた 0.5 は **sRGB の 0.5**。線形では 0.214 なので、
    // 書き出しで sRGB に戻ると 128 前後になる。
    let got = rgb(&pixels, 128, 128);

    assert!(got[0].abs_diff(128) <= 8, "赤 {}", got[0]);
    assert!(got[1] < 10 && got[2] < 10, "赤以外が残っている {got:?}");
}

// ---- 使いにくい入力 ----

#[test]
fn a_single_stop_paints_flat() {
    gpu!();

    let pixels = painted(&[Block::GradientStops {
        gradient: Gradient::linear(0.0).stop(0.5, GREEN),
    }]);

    assert_rgb(rgb(&pixels, 2, 128), [0, 255, 0], "左端");
    assert_rgb(rgb(&pixels, 253, 128), [0, 255, 0], "右端");
}

#[test]
fn the_most_stops_still_work() {
    gpu!();

    let mut gradient = Gradient::linear(0.0);

    for index in 0..8 {
        let ratio = index as f32 / 7.0;
        gradient = gradient.stop(ratio, [ratio, 0.0, 1.0 - ratio, 1.0]);
    }

    let pixels = painted(&[Block::GradientStops { gradient }]);

    assert_rgb(rgb(&pixels, 2, 128), [0, 0, 255], "左端は青");
    assert_rgb(rgb(&pixels, 253, 128), [255, 0, 0], "右端は赤");
}
