//! クリッピングを **GPU に描かせて、画素を読み戻して**確かめる。
//!
//! `clip.rs` の中のテストは焼く側だけを見ています。ここで見るのはその先、
//! シェーダが覆いをどう読んで不透明度を削るかです。取り決めが片方だけ
//! ずれていても単体テストは通ってしまうので、実際に描いて数えます。
//!
//! # アダプタが取れないときは飛ばす
//!
//! GPU の無い場所でも `cargo test` が赤くならないよう、デバイスを作れなければ
//! 何もせずに終わります。**飛ばしたことは必ず出力に残します**。
//!
//! ```sh
//! cargo test -p gueiz-2d --test clip_gpu -- --nocapture
//! ```

use std::sync::OnceLock;

use gueiz_2d::camera::Camera;
use gueiz_2d::clip::ClipMaskKind;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::effect::Block;
use gueiz_2d::instance;
use gueiz_2d::object::{Object, create_object};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

/// 描く先の 1 辺。読み戻しの行は 256 バイト境界なので、64 の倍数にする。
const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

// ---- 支度 ----

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// デバイスは 1 つを使い回す。テストごとに作ると支度のほうが長くなる。
fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

    GPU.get_or_init(|| {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("clip gpu test"),
                required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
                ..Default::default()
            }))
            .ok()?;

        Some(Gpu { device, queue })
    })
    .as_ref()
}

/// GPU が無ければ、飛ばしたと言い残して抜ける。
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

fn at(x: f32, y: f32) -> Vertex {
    Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0)
}

/// 画面いっぱいの白い四角。クリップはこれを削る。
fn canvas() -> Object {
    let full = SIZE as f32;
    let mut object = create_object("Canvas");

    object.begin(PaintType::Fill);
    for [x, y] in [[0.0, 0.0], [full, 0.0], [full, full], [0.0, full]] {
        object.put_vertex(at(x, y));
    }
    object.end();

    object.camera(Camera::orthographic_2d(full, full));
    object.instance(instance::create_instance());
    object
}

/// 山を積んだ白い四角を描いて、画素を返す。
fn clipped(blocks: &[Block]) -> Vec<u8> {
    let mut manager = manager(&DrawManagerDescriptor::default());

    let mut object = canvas();
    for block in blocks {
        object.effect(*block);
    }
    manager.register(object);

    render(&mut manager)
}

fn manager(descriptor: &DrawManagerDescriptor) -> DrawManager {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");

    DrawManager::new(&gpu.device, &gpu.queue, FORMAT, descriptor)
        .expect("DrawManager を作れなかった")
}

/// その画素が塗られているか。緑の成分で見る（白なので色は問わない）。
fn lit(pixels: &[u8], x: u32, y: u32) -> bool {
    value(pixels, x, y) > 128
}

fn value(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SIZE + x) * 4) as usize + 1]
}

/// 塗られた画素の数。
fn covered(pixels: &[u8]) -> usize {
    (0..SIZE * SIZE)
        .filter(|index| lit(pixels, index % SIZE, index / SIZE))
        .count()
}

/// 面積が理論値と合うか。縁の半端な画素のぶんだけ緩める。
fn area_near(pixels: &[u8], want: f64, perimeter: f64) {
    let got = covered(pixels) as f64;

    assert!(
        (got - want).abs() <= perimeter,
        "{got} 画素。{want} ± {perimeter} のはず",
    );
}

/// sRGB で書かれた値を光の量に戻す。表と裏を足すときは線形でないと合わない。
fn linear(value: u8) -> f32 {
    let value = value as f32 / 255.0;

    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// 表と裏が 1 画素ずつ過不足なく補い合うか。
///
/// 閾値で数えると、縁の半透明な画素が表でも裏でも数えられて二重になる。
/// 光の量で足せば、なめらかな縁まで含めて確かめられる。
fn complementary(front: &[u8], back: &[u8]) {
    let mut worst = 0.0f32;
    let mut off = 0;

    for index in 0..(SIZE * SIZE) as usize {
        let sum = linear(front[index * 4 + 1]) + linear(back[index * 4 + 1]);
        let gap = (sum - 1.0).abs();

        worst = worst.max(gap);

        if gap > 0.02 {
            off += 1;
        }
    }

    assert_eq!(off, 0, "{off} 画素が補い合っていない（ずれの最大 {worst}）");
}

/// 縁を 1 行横切って、半端な明るさの画素が何枚続くか。
///
/// にじんでいれば長く、縁が保てていれば短い。
///
/// **光の量で見ます。** 書かれているのは sRGB なので、そのまま閾値を当てると
/// 傾斜の半分しか数えられません（覆う割合 0.5 が sRGB では 188 まで持ち上がる）。
fn blur_width(pixels: &[u8], row: u32) -> u32 {
    let mut widest = 0;
    let mut run = 0;

    for x in 0..SIZE {
        let light = linear(value(pixels, x, row));

        if (0.02..0.98).contains(&light) {
            run += 1;
            widest = widest.max(run);
        } else {
            run = 0;
        }
    }

    widest
}

fn render(draw_manager: &mut DrawManager) -> Vec<u8> {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");
    let (device, queue) = (&gpu.device, &gpu.queue);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("clip gpu test target"),
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
        label: Some("clip gpu test readback"),
        size: (SIZE * SIZE * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    draw_manager.prepare().expect("支度に失敗した");

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("clip gpu test"),
    });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clip gpu test pass"),
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

// ---- 式で解くクリップ ----

#[test]
fn nothing_is_clipped_without_a_block() {
    gpu!();

    let pixels = clipped(&[]);

    assert_eq!(covered(&pixels), (SIZE * SIZE) as usize);
}

#[test]
fn a_rect_keeps_exactly_its_inside() {
    gpu!();

    let pixels = clipped(&[Block::ClipRect {
        min: [64.0, 64.0],
        max: [192.0, 192.0],
        radius: 0.0,
        softness: 0.0,
        invert: false,
    }]);

    // 128 四方。軸に沿っているので縁が升目に乗り、ぴたりと出る。
    assert_eq!(covered(&pixels), 128 * 128);

    assert!(lit(&pixels, 128, 128), "中");
    assert!(!lit(&pixels, 60, 128), "左の外");
    assert!(!lit(&pixels, 196, 128), "右の外");
}

#[test]
fn a_radius_rounds_the_corners_off() {
    gpu!();

    let radius = 40.0;
    let pixels = clipped(&[Block::ClipRect {
        min: [64.0, 64.0],
        max: [192.0, 192.0],
        radius,
        softness: 0.0,
        invert: false,
    }]);

    assert!(!lit(&pixels, 66, 66), "角が丸まっていない");
    assert!(lit(&pixels, 128, 66), "辺の真ん中まで削れている");

    // 四隅で 4 * (r^2 - πr^2/4) ぶん減る。
    let cut = 4.0 * (radius as f64).powi(2) * (1.0 - std::f64::consts::PI / 4.0);
    area_near(&pixels, 128.0 * 128.0 - cut, 600.0);
}

#[test]
fn an_ellipse_keeps_its_own_area() {
    gpu!();

    let (radius_x, radius_y) = (100.0, 40.0);
    let pixels = clipped(&[Block::ClipEllipse {
        center: [128.0, 128.0],
        radius_x,
        radius_y,
        softness: 0.0,
        invert: false,
    }]);

    area_near(
        &pixels,
        std::f64::consts::PI * radius_x as f64 * radius_y as f64,
        500.0,
    );

    assert!(lit(&pixels, 220, 128), "長い軸の内側");
    assert!(!lit(&pixels, 128, 175), "短い軸の外側");
}

#[test]
fn a_half_plane_cuts_the_screen_in_two() {
    gpu!();

    let pixels = clipped(&[Block::ClipHalfPlane {
        normal: [1.0, 0.0],
        distance: 128.0,
        softness: 0.0,
        invert: false,
    }]);

    assert_eq!(covered(&pixels), (SIZE * SIZE / 2) as usize);
    assert!(lit(&pixels, 64, 128), "法線の手前は残る");
    assert!(!lit(&pixels, 192, 128), "法線の先は削れる");
}

/// 半平面は積むと凸多角形になる。ここが `ClipHalfPlane` の要。
#[test]
fn stacked_half_planes_carve_a_triangle() {
    gpu!();

    // 頂点 (128,26) (26,218) (230,218)。
    let pixels = clipped(&[
        Block::ClipHalfPlane {
            normal: [0.0, 1.0],
            distance: 218.0,
            softness: 0.0,
            invert: false,
        },
        Block::ClipHalfPlane {
            normal: [-192.0, -102.0],
            distance: -27228.0,
            softness: 0.0,
            invert: false,
        },
        Block::ClipHalfPlane {
            normal: [192.0, -102.0],
            distance: 21924.0,
            softness: 0.0,
            invert: false,
        },
    ]);

    // 底辺 204、高さ 192。
    area_near(&pixels, 204.0 * 192.0 / 2.0, 400.0);

    assert!(lit(&pixels, 128, 150), "三角形の中");
    assert!(!lit(&pixels, 40, 60), "左上の外");
    assert!(!lit(&pixels, 220, 60), "右上の外");
    assert!(!lit(&pixels, 128, 230), "下の外");
}

/// 積むと交わりになる。片方の中でももう片方の外なら消える。
#[test]
fn stacking_clips_takes_the_overlap() {
    gpu!();

    let pixels = clipped(&[
        Block::ClipRect {
            min: [0.0, 0.0],
            max: [128.0, 256.0],
            radius: 0.0,
            softness: 0.0,
            invert: false,
        },
        Block::ClipEllipse {
            center: [128.0, 128.0],
            radius_x: 76.0,
            radius_y: 76.0,
            softness: 0.0,
            invert: false,
        },
    ]);

    assert!(lit(&pixels, 90, 128), "両方の中");
    assert!(!lit(&pixels, 170, 128), "楕円の中だが矩形の外");
    assert!(!lit(&pixels, 30, 128), "矩形の中だが楕円の外");

    // 半円ぶん。
    area_near(&pixels, std::f64::consts::PI * 76.0 * 76.0 / 2.0, 400.0);
}

/// 反転は「残る側を入れ替える」だけ。**縁の半端な画素まで含めて**
/// 表と裏で過不足なく画面を覆う。
#[test]
fn inverting_gives_the_exact_complement() {
    gpu!();

    let ring = |invert| {
        [Block::ClipEllipse {
            center: [128.0, 128.0],
            radius_x: 64.0,
            radius_y: 64.0,
            softness: 0.0,
            invert,
        }]
    };

    let front = clipped(&ring(false));
    let back = clipped(&ring(true));

    complementary(&front, &back);

    assert!(lit(&front, 128, 128) && !lit(&back, 128, 128), "真ん中");
    assert!(!lit(&front, 4, 4) && lit(&back, 4, 4), "隅");
}

#[test]
fn an_inverted_clip_punches_a_hole() {
    gpu!();

    let pixels = clipped(&[
        Block::ClipRect {
            min: [32.0, 32.0],
            max: [224.0, 224.0],
            radius: 0.0,
            softness: 0.0,
            invert: false,
        },
        Block::ClipEllipse {
            center: [128.0, 128.0],
            radius_x: 48.0,
            radius_y: 48.0,
            softness: 0.0,
            invert: true,
        },
    ]);

    assert!(!lit(&pixels, 128, 128), "穴の真ん中は抜ける");
    assert!(lit(&pixels, 40, 40), "四角の隅は残る");
    assert!(!lit(&pixels, 10, 128), "四角の外は消える");

    area_near(
        &pixels,
        192.0 * 192.0 - std::f64::consts::PI * 48.0 * 48.0,
        600.0,
    );
}

#[test]
fn softness_widens_the_edge() {
    gpu!();

    let ring = |softness| {
        [Block::ClipEllipse {
            center: [128.0, 128.0],
            radius_x: 76.0,
            radius_y: 76.0,
            softness,
            invert: false,
        }]
    };

    let crisp = blur_width(&clipped(&ring(0.0)), 128);
    let soft = blur_width(&clipped(&ring(24.0)), 128);

    assert!(crisp >= 1, "ぼかし 0 でも 1 画素はなめらかなはず");
    assert!(soft > crisp * 4, "ぼかしが効いていない（{crisp} → {soft}）");
}

/// クリップは山を積んだ図形にだけ効く。隣の図形は削らない。
#[test]
fn a_clip_leaves_other_shapes_alone() {
    gpu!();

    let mut manager = manager(&DrawManagerDescriptor::default());

    let mut narrow = canvas();
    narrow.effect(Block::ClipRect {
        min: [0.0, 0.0],
        max: [128.0, 256.0],
        radius: 0.0,
        softness: 0.0,
        invert: false,
    });
    manager.register(narrow);

    // 山を積まない四角を重ねる。こちらは削れない。
    let mut plain = canvas();
    plain.z(1.0);
    manager.register(plain);

    let pixels = render(&mut manager);

    assert_eq!(
        covered(&pixels),
        (SIZE * SIZE) as usize,
        "山を積んでいない図形まで削れている",
    );
}

// ---- 焼いた覆い ----

/// 十字。凹んでいるので式では書けない。
fn cross() -> Object {
    let mut object = create_object("Cross");

    object.begin(PaintType::Fill);
    for [x, y] in [
        [96.0, 32.0],
        [160.0, 32.0],
        [160.0, 96.0],
        [224.0, 96.0],
        [224.0, 160.0],
        [160.0, 160.0],
        [160.0, 224.0],
        [96.0, 224.0],
        [96.0, 160.0],
        [32.0, 160.0],
        [32.0, 96.0],
        [96.0, 96.0],
    ] {
        object.put_vertex(at(x, y));
    }
    object.end();

    object
}

/// 覆いを焼いて、それで削った白い四角を描く。
fn masked(kind: ClipMaskKind, shape: &Object, softness: f32, invert: bool) -> Vec<u8> {
    masked_with(&DrawManagerDescriptor::default(), kind, shape, softness, invert)
}

fn masked_with(
    descriptor: &DrawManagerDescriptor,
    kind: ClipMaskKind,
    shape: &Object,
    softness: f32,
    invert: bool,
) -> Vec<u8> {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");
    let mut manager = manager(descriptor);

    let mask = manager.add_clip_mask(shape, kind);

    let mut object = canvas();
    object.effect(mask.block_with(softness, invert));
    manager.register(object);

    render(&mut manager)
}

#[test]
fn a_coverage_mask_clips_a_concave_shape() {
    gpu!();

    let pixels = masked(ClipMaskKind::Coverage, &cross(), 0.0, false);

    // 縦棒 64x192 + 横棒 64x192 - 重なり 64x64。
    area_near(&pixels, 64.0 * 192.0 * 2.0 - 64.0 * 64.0, 700.0);

    assert!(lit(&pixels, 128, 128), "十字の真ん中");
    assert!(lit(&pixels, 128, 40), "上の腕");
    assert!(lit(&pixels, 40, 128), "左の腕");
    assert!(!lit(&pixels, 48, 48), "凹んだ隅。ここが式では書けない");
    assert!(!lit(&pixels, 208, 48), "反対の凹み");
    assert!(!lit(&pixels, 10, 10), "覆いの外");
}

#[test]
fn a_distance_mask_clips_the_same_shape() {
    gpu!();

    let pixels = masked(ClipMaskKind::Distance, &cross(), 0.0, false);

    area_near(&pixels, 64.0 * 192.0 * 2.0 - 64.0 * 64.0, 700.0);

    assert!(lit(&pixels, 128, 128), "十字の真ん中");
    assert!(!lit(&pixels, 48, 48), "凹んだ隅");
    assert!(!lit(&pixels, 10, 10), "覆いの外");
}

#[test]
fn inverting_a_mask_gives_the_exact_complement() {
    gpu!();

    for kind in [ClipMaskKind::Coverage, ClipMaskKind::Distance] {
        let front = masked(kind, &cross(), 0.0, false);
        let back = masked(kind, &cross(), 0.0, true);

        complementary(&front, &back);

        assert!(lit(&front, 128, 128) && !lit(&back, 128, 128), "{kind:?} 真ん中");
        assert!(!lit(&front, 10, 10) && lit(&back, 10, 10), "{kind:?} 覆いの外");
    }
}

/// **距離を焼く意味はここにある。** 粗く焼いた覆いを引き伸ばしても、
/// 覆う割合はにじむが距離は縁を保つ。
#[test]
fn a_distance_mask_keeps_its_edge_when_magnified() {
    gpu!();

    // わざと粗く焼く。形は 96 単位四方を画面の 96 画素に描くので、
    // 覆いの 1 升が 6 画素ぶんに伸びる。
    let coarse = DrawManagerDescriptor {
        clip_mask_resolution: 16,
        ..Default::default()
    };

    // 縁が斜めの形にする。軸に沿った四角は縁が升目に揃ってしまい、
    // 焼いた細かさの差がいちばん出にくい。
    let mut disc = create_object("Disc");
    disc.begin(PaintType::Fill);
    for step in 0..64 {
        let angle = std::f32::consts::TAU * step as f32 / 64.0;
        disc.put_vertex(at(64.0 + 48.0 * angle.cos(), 64.0 + 48.0 * angle.sin()));
    }
    disc.end();

    let coverage = blur_width(
        &masked_with(&coarse, ClipMaskKind::Coverage, &disc, 0.0, false),
        64,
    );
    let distance = blur_width(
        &masked_with(&coarse, ClipMaskKind::Distance, &disc, 0.0, false),
        64,
    );

    eprintln!("拡大したときの縁: 割合 {coverage} 画素、距離 {distance} 画素");

    assert!(
        coverage >= 4,
        "割合は拡大でにじむはず（{coverage} 画素。測り方が変わっていないか）",
    );
    assert!(
        distance * 2 < coverage,
        "距離なら拡大しても縁が保てるはず（割合 {coverage}、距離 {distance}）",
    );
}

/// ぼかしは距離にしか渡らない。覆う割合には境目からの距離が入っていない。
#[test]
fn softness_only_reaches_the_distance_mask() {
    gpu!();

    let coarse = DrawManagerDescriptor {
        clip_mask_resolution: 32,
        ..Default::default()
    };

    for (kind, widens) in [(ClipMaskKind::Coverage, false), (ClipMaskKind::Distance, true)] {
        let crisp = blur_width(&masked_with(&coarse, kind, &cross(), 0.0, false), 128);
        let soft = blur_width(&masked_with(&coarse, kind, &cross(), 16.0, false), 128);

        if widens {
            assert!(soft > crisp * 3, "{kind:?} でぼかしが効かない（{crisp} → {soft}）");
        } else {
            assert_eq!(soft, crisp, "{kind:?} にぼかしが効いてしまっている");
        }
    }
}

// ---- 層の出し入れ ----

#[test]
fn mask_layers_grow_when_they_run_out() {
    let gpu = gpu!();
    let mut manager = manager(&DrawManagerDescriptor::default());

    assert_eq!(manager.clip_mask_count(), 0);

    let start = manager.clip_mask_capacity();
    let shape = cross();

    for _ in 0..start + 5 {
        manager.add_clip_mask(&shape, ClipMaskKind::Coverage);
    }

    assert_eq!(manager.clip_mask_count() as u32, start + 5);
    assert!(
        manager.clip_mask_capacity() >= start + 5,
        "層が足りていない（{}）",
        manager.clip_mask_capacity(),
    );
}

#[test]
fn a_freed_layer_comes_back_round() {
    let gpu = gpu!();
    let mut manager = manager(&DrawManagerDescriptor::default());
    let shape = cross();

    let first = manager.add_clip_mask(&shape, ClipMaskKind::Coverage);
    let second = manager.add_clip_mask(&shape, ClipMaskKind::Coverage);

    assert_ne!(first.layer(), second.layer(), "同じ層を 2 枚に配っている");

    let capacity = manager.clip_mask_capacity();
    manager.remove_clip_mask(first);

    assert_eq!(manager.clip_mask_count(), 1);

    let reused = manager.add_clip_mask(&shape, ClipMaskKind::Coverage);

    assert_eq!(reused.layer(), first.layer(), "空いた層を使い回していない");
    assert_eq!(manager.clip_mask_capacity(), capacity, "使い回せるのに増えた");
}

/// 焼き直しても層は動かない。積んである山をそのまま使い続けられる。
#[test]
fn rebaking_keeps_the_same_layer() {
    let gpu = gpu!();
    let mut manager = manager(&DrawManagerDescriptor::default());

    let mask = manager.add_clip_mask(&cross(), ClipMaskKind::Coverage);

    let mut wider = create_object("Wide");
    wider.begin(PaintType::Fill);
    for [x, y] in [[0.0, 0.0], [256.0, 0.0], [256.0, 256.0], [0.0, 256.0]] {
        wider.put_vertex(at(x, y));
    }
    wider.end();

    let updated = manager.update_clip_mask(mask, &wider);

    assert_eq!(updated.layer(), mask.layer());
    assert_eq!(updated.kind(), mask.kind());
    assert_ne!(updated.bounds(), mask.bounds(), "覆う範囲が追いついていない");
    assert_eq!(manager.clip_mask_count(), 1, "枚数が増えている");
}

#[test]
fn a_distance_mask_reports_a_square_cover_and_a_spread() {
    let gpu = gpu!();
    let mut manager = manager(&DrawManagerDescriptor::default());

    let coverage = manager.add_clip_mask(&cross(), ClipMaskKind::Coverage);
    let distance = manager.add_clip_mask(&cross(), ClipMaskKind::Distance);

    assert_eq!(coverage.spread(), 0.0, "割合に広がりは無い");
    assert!(distance.spread() > 0.0, "距離の広がりが載っていない");

    let [_, _, width, height] = distance.bounds();
    assert!(
        (width - height).abs() < 0.01,
        "距離は正方形に揃うはず（{width}x{height}）",
    );
}
