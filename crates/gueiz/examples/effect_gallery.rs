//! **すべてのエフェクトを窓に並べて、目で確かめる。**
//!
//! 読み戻して数える検証（`effect_test`、`post_test`、`clip_gpu`）とは狙いが別です。
//! あちらは「値が合っているか」、ここは**「見て変か」**を見ます。
//! 時間で動くもの（回る・揺れる・明滅する）は、動いている様子でないと分かりません。
//!
//! # 並ぶもの
//!
//! - Shape 段: 縁取り
//! - Transform 段: 回る・揺れる・周る・脈打つ
//! - Color 段: 色掛け・2 色グラデーション・溶け・明滅
//! - 多段グラデーション: 線形・放射・角度（＋繰り返し・折り返し）
//! - クリップ: 矩形・角丸・楕円・半平面・反転・焼いた覆い（割合・距離）
//! - 自前の WGSL
//! - 塗り方: 破線・点線
//! - **図形ごとのエフェクト**: ぼかし・グロー・白黒
//!
//! 最後のものは山を 1 つも積んでいません。**その図形にだけ掛けた**だけです。
//! ぼかしは隣の画素を読むので断片シェーダでは図形ごとに書けず、
//! 同じものが続く図形をまとめて 1 枚に描いてから掛けて重ねる、
//! というのがその答えになります。まとめる仕事は `DrawManager` がやります。
//!
//! 何も積んでいない**素の図形**を左上に置いてあります。**見比べる基準**です。
//! 効いていないエフェクトは、これと同じ見た目になります。
//!
//! # 操作
//!
//! | キー | すること |
//! |---|---|
//! | `Space` | 時間を止める・動かす |
//! | `P` | ふつうの升目に掛けるエフェクトを切り替える |
//! | `L` | 名札を消す・出す |
//! | `Esc` | 閉じる |
//!
//! # 窓を開かずに数える
//!
//! `--check` を付けると、窓を開かずに**全升目が基準と違う絵になっているか**を
//! 数えて表にします。目で見るだけでは「黋って何もしないエフェクト」を
//! 見落とすので、そちらはこれで捕まえます。
//!
//! ```sh
//! cargo run -p gueiz --example effect_gallery
//! cargo run -p gueiz --example effect_gallery -- --check
//! cargo run -p gueiz --example effect_gallery -- dx12
//! ```

use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use gueiz_2d::camera::{Camera, ScaleMode};
use gueiz_2d::clip::ClipMaskKind;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::effect::{
    Block, CustomBlock, EffectStage, Gradient, GradientSpread, CUSTOM_KIND_BASE,
};
use gueiz_2d::font::Font;
use gueiz_2d::instance::create_instance;
use gueiz_2d::msaa::DEFAULT_MULTISAMPLE;
use gueiz_2d::object::{Object, create_object};
use gueiz_2d::paint_type::{Dash, JointType, PaintType};
use gueiz_2d::post::{PostEffect, PostProcessor};
use gueiz_2d::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz_2d::text::{TextRenderer, TextStyle};
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

/// 名札の書体。日本語の字を持つものを探す。名札に使うだけなので、無くても絵は出ます。
#[path = "common/font.rs"]
mod example_font;

/// 並べる升目。
const COLUMNS: usize = 6;
/// 升目 1 つの大きさ。この中に図形と名札が収まる。
const CELL: f32 = 150.0;
/// 図形の一辺。升目より小さくして、動くエフェクトのはみ出す余地を残す。
const TILE: f32 = 86.0;
/// 名札のぶんだけ図形を上に寄せる。
const LABEL_ROOM: f32 = 26.0;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let checking = arguments.iter().any(|argument| argument == "--check");

    let renderer_backend = match arguments.iter().find(|argument| *argument != "--check") {
        Some(argument) => argument.parse::<RendererBackend>()?,
        None => RendererBackend::Auto,
    };

    // 窓を開かずに、全升目が基準と違う絵になっているかだけ数える。
    if checking {
        return check(renderer_backend);
    }

    println!("Space 止める / P ふつうの升目のエフェクト / L 名札 / Esc 閉じる\n");

    let event_loop = EventLoop::new()?;
    // 時間で動くものがあるので、待たずに回し続ける。
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(Application::new(renderer_backend))?;

    Ok(())
}

// ---- 並べるもの ----

/// 升目 1 つぶん。名札と、積む山。
struct Tile {
    label: &'static str,
    /// 図形をどう塗るか。破線や点線はここで変わる。
    paint_type: PaintType,
    /// 積む山。空なら素の図形。
    blocks: Vec<Block>,
    /// 焼いた覆いを使うなら、その種類。層は場面を組むときに取る。
    mask: Option<ClipMaskKind>,
    /// **この升目だけに掛ける**、画面全体のたぐいのエフェクト。
    post: Option<PostEffect>,
}

impl Tile {
    fn new(label: &'static str, blocks: Vec<Block>) -> Self {
        Self {
            label,
            paint_type: PaintType::Fill,
            blocks,
            mask: None,
            post: None,
        }
    }

    fn painted(label: &'static str, paint_type: PaintType) -> Self {
        Self {
            label,
            paint_type,
            blocks: Vec::new(),
            mask: None,
            post: None,
        }
    }

    fn masked(label: &'static str, kind: ClipMaskKind) -> Self {
        Self {
            label,
            paint_type: PaintType::Fill,
            blocks: Vec::new(),
            mask: Some(kind),
            post: None,
        }
    }

    /// **この升目だけ**にエフェクトを掛ける。
    ///
    /// ぼかしは隣の画素を読むので、断片シェーダでは図形ごとに書けません。
    /// 同じものが続く図形をまとめて 1 枚に描いてから掛けて重ねる、
    /// というのがその答えで、まとめる仕事は `DrawManager` がやります。
    fn with_post(label: &'static str, post: PostEffect) -> Self {
        Self {
            label,
            paint_type: PaintType::Fill,
            blocks: Vec::new(),
            mask: None,
            post: Some(post),
        }
    }
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const ORANGE: [f32; 4] = [1.0, 0.45, 0.1, 1.0];
const CYAN: [f32; 4] = [0.2, 0.8, 1.0, 1.0];

/// 画面に並べる順。**素の図形が先頭**で、これが見比べる基準。
fn tiles() -> Vec<Tile> {
    // クリップは図形ローカルのワールド座標で効く。図形は 0..TILE に置くので、
    // その真ん中と半分を基準にする。
    let middle = TILE / 2.0;
    let half = TILE / 4.0;

    vec![
        Tile::new("none (基準)", vec![]),
        // --- Shape 段 ---
        Tile::new(
            "Outline",
            vec![Block::Outline {
                width: 6.0,
                color: CYAN,
            }],
        ),
        // --- Transform 段 ---
        Tile::new("Spin", vec![Block::Spin { speed: 1.2 }]),
        Tile::new(
            "Wobble",
            vec![Block::Wobble {
                amplitude: 18.0,
                frequency: 1.5,
            }],
        ),
        Tile::new(
            "Orbit",
            vec![Block::Orbit {
                radius: 18.0,
                speed: 1.5,
            }],
        ),
        Tile::new(
            "Pulse",
            vec![Block::Pulse {
                amount: 0.3,
                speed: 2.0,
            }],
        ),
        // --- Color 段 ---
        Tile::new("Tint", vec![Block::Tint { color: ORANGE }]),
        Tile::new(
            "Gradient (2 色)",
            vec![Block::Gradient {
                from: ORANGE,
                to: CYAN,
                angle: 0.0,
            }],
        ),
        Tile::new(
            "Dissolve",
            vec![Block::Dissolve {
                threshold: 0.35,
                edge: 0.08,
                edge_color: [1.0, 0.9, 0.3, 1.0],
            }],
        ),
        Tile::new(
            "Flicker",
            vec![Block::Flicker {
                amount: 0.7,
                speed: 6.0,
            }],
        ),
        // --- 多段グラデーション ---
        Tile::new(
            "Stops 線形",
            vec![Block::GradientStops {
                gradient: Gradient::linear(0.6)
                    .stop(0.0, [1.0, 0.2, 0.2, 1.0])
                    .stop(0.5, [1.0, 1.0, 0.2, 1.0])
                    .stop(1.0, [0.2, 0.5, 1.0, 1.0]),
            }],
        ),
        Tile::new(
            "Stops 放射",
            vec![Block::GradientStops {
                gradient: Gradient::radial([0.5, 0.5], 0.5, 0.5)
                    .stop(0.0, [1.0, 1.0, 0.8, 1.0])
                    .stop(0.6, ORANGE)
                    .stop(1.0, [0.3, 0.0, 0.2, 1.0]),
            }],
        ),
        Tile::new(
            "Stops 角度",
            vec![Block::GradientStops {
                gradient: Gradient::conic([0.5, 0.5], 0.0)
                    .stop(0.0, [1.0, 0.2, 0.2, 1.0])
                    .stop(0.33, [0.2, 1.0, 0.3, 1.0])
                    .stop(0.66, [0.3, 0.4, 1.0, 1.0])
                    .stop(1.0, [1.0, 0.2, 0.2, 1.0]),
            }],
        ),
        Tile::new(
            "Stops 繰り返し 図形 頂点",
            vec![Block::GradientStops {
                gradient: Gradient::radial([0.5, 0.5], 0.18, 0.18)
                    .spread(GradientSpread::Repeat)
                    .stop(0.0, [0.1, 0.1, 0.2, 1.0])
                    .stop(1.0, CYAN),
            }],
        ),
        Tile::new(
            "Stops 折り返し",
            vec![Block::GradientStops {
                gradient: Gradient::radial([0.5, 0.5], 0.18, 0.18)
                    .spread(GradientSpread::Mirror)
                    .stop(0.0, [0.1, 0.1, 0.2, 1.0])
                    .stop(1.0, CYAN),
            }],
        ),
        // --- クリップ ---
        Tile::new(
            "ClipRect",
            vec![Block::ClipRect {
                min: [half, half],
                max: [TILE - half, TILE - half],
                radius: 0.0,
                softness: 0.0,
                invert: false,
            }],
        ),
        Tile::new(
            "ClipRect 角丸",
            vec![Block::ClipRect {
                min: [6.0, 6.0],
                max: [TILE - 6.0, TILE - 6.0],
                radius: 22.0,
                softness: 0.0,
                invert: false,
            }],
        ),
        Tile::new(
            "ClipRect ぼかし",
            vec![Block::ClipRect {
                min: [10.0, 10.0],
                max: [TILE - 10.0, TILE - 10.0],
                radius: 18.0,
                softness: 16.0,
                invert: false,
            }],
        ),
        Tile::new(
            "ClipEllipse",
            vec![Block::ClipEllipse {
                center: [middle, middle],
                radius_x: middle - 4.0,
                radius_y: middle * 0.6,
                softness: 0.0,
                invert: false,
            }],
        ),
        Tile::new(
            "ClipEllipse 反転",
            vec![Block::ClipEllipse {
                center: [middle, middle],
                radius_x: middle * 0.55,
                radius_y: middle * 0.55,
                softness: 0.0,
                invert: true,
            }],
        ),
        Tile::new(
            "ClipHalfPlane ×3",
            // 上向きの三角形にくり抜く。
            vec![
                Block::ClipHalfPlane {
                    normal: [0.0, 1.0],
                    distance: TILE - 8.0,
                    softness: 0.0,
                    invert: false,
                },
                Block::ClipHalfPlane {
                    normal: [-2.0, -1.0],
                    distance: -2.0 * middle - 8.0,
                    softness: 0.0,
                    invert: false,
                },
                Block::ClipHalfPlane {
                    normal: [2.0, -1.0],
                    distance: 2.0 * middle - 8.0,
                    softness: 0.0,
                    invert: false,
                },
            ],
        ),
        Tile::masked("ClipMask 割合", ClipMaskKind::Coverage),
        Tile::masked("ClipMask 距離", ClipMaskKind::Distance),
        // --- 自前の WGSL ---
        Tile::new(
            "Custom (縞)",
            vec![Block::Custom {
                stage: EffectStage::Color,
                kind: CUSTOM_KIND_BASE,
                params: [8.0, 0.0, 0.0, 0.0],
                color: CYAN,
            }],
        ),
        // --- 塗り方 ---
        Tile::painted(
            "Stroke",
            PaintType::Stroke {
                line_width: 6.0,
                joint_type: JointType::Round,
                strip: false,
                dash: None,
            },
        ),
        Tile::painted(
            "Dash",
            PaintType::Stroke {
                line_width: 6.0,
                joint_type: JointType::Round,
                strip: false,
                dash: Some(Dash::new(14.0, 8.0)),
            },
        ),
        // --- 図形ごとのエフェクト ---
        //
        // 山を 1 つも積んでいない。**その図形にだけ掛けた**だけで変わる。
        Tile::with_post("図形: ぼかし", PostEffect::Blur { radius: 4.0 }),
        Tile::with_post(
            "図形: グロー",
            PostEffect::Glow {
                threshold: 0.4,
                intensity: 1.6,
                // 升目のあいだは 32 画素。滲みがそこに収まる大きさにする。
                radius: 6.0,
            },
        ),
        Tile::with_post(
            "図形: 白黒",
            PostEffect::ColorGrade {
                exposure: 1.0,
                saturation: 0.0,
                tint: WHITE,
            },
        ),
        Tile::painted(
            "Dots",
            PaintType::Stroke {
                line_width: 7.0,
                joint_type: JointType::Round,
                strip: false,
                dash: Some(Dash::dots(18.0)),
            },
        ),
    ]
}

/// 名札に「何が起きるか」を添える。動いているか迷ったときの手がかり。
fn hint(label: &str) -> &'static str {
    match label {
        "none (基準)" => "見比べる元",
        "Spin" => "回る",
        "Wobble" => "上下に揺れる",
        "Orbit" => "円を描く",
        "Pulse" => "伸び縮み",
        "Dissolve" => "穴が空く",
        "Flicker" => "明滅する",
        "図形: ぼかし" => "この升目だけ",
        "図形: グロー" => "この升目だけ",
        "図形: 白黒" => "この升目だけ",
        _ => "",
    }
}

/// クリップの山を、図形を置いた先に合わせてずらす。
///
/// **クリップの坐標はワールドです。** 図形を `translate` で動かしても
/// クリップは置いた場所に留まるので、升目ごとに並べるなら
/// こちらも一緒に動かす必要があります。
///
/// これを忘れると、クリップが原点の近くに取り残されて
/// **図形が丸ごと消えます**（反転していれば丸ごと残る）。
fn shifted(block: Block, dx: f32, dy: f32) -> Block {
    match block {
        Block::ClipRect { min, max, radius, softness, invert } => Block::ClipRect {
            min: [min[0] + dx, min[1] + dy],
            max: [max[0] + dx, max[1] + dy],
            radius,
            softness,
            invert,
        },

        Block::ClipEllipse { center, radius_x, radius_y, softness, invert } => {
            Block::ClipEllipse {
                center: [center[0] + dx, center[1] + dy],
                radius_x,
                radius_y,
                softness,
                invert,
            }
        }

        // 直線は `dot(normal, 点) = distance`。ずらすと右辺がその分だけ動く。
        Block::ClipHalfPlane { normal, distance, softness, invert } => Block::ClipHalfPlane {
            normal,
            distance: distance + normal[0] * dx + normal[1] * dy,
            softness,
            invert,
        },

        Block::ClipMask { min, size, layer, invert } => Block::ClipMask {
            min: [min[0] + dx, min[1] + dy],
            size,
            layer,
            invert,
        },

        Block::ClipDistanceMask { min, size, layer, spread, softness, invert } => {
            Block::ClipDistanceMask {
                min: [min[0] + dx, min[1] + dy],
                size,
                layer,
                spread,
                softness,
                invert,
            }
        }

        // それ以外は場所を見ない。
        other => other,
    }
}

/// 書き先を黒で消すだけのパス。層はこの上に重なる。
fn clear(encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
    let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("gallery clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
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

/// 升目 `index` の図形を置く左上。
fn tile_origin(index: usize) -> (f32, f32) {
    let column = index % COLUMNS;
    let row = index / COLUMNS;

    (
        column as f32 * CELL + (CELL - TILE) / 2.0,
        row as f32 * CELL + (CELL - TILE - LABEL_ROOM) / 2.0,
    )
}

/// 「Custom (繞)」の中身。窓と点検の両方から使う。
fn custom_stripes() -> CustomBlock {
    CustomBlock {
        kind: CUSTOM_KIND_BASE,
        stage: EffectStage::Color,
        body: String::from(
            "let stripe = step(0.5, fract(uv.x * block.params.x));
             return color * mix(vec4<f32>(1.0), block.color_a, stripe);",
        ),
    }
}

/// 覆いに焼く形。凹んでいるので式では書けない。
fn notch() -> Object {
    let mut object = create_object("Notch");
    let (low, high) = (8.0, TILE - 8.0);
    let middle = TILE / 2.0;

    object.begin(PaintType::Fill);
    for [x, y] in [
        [low, low],
        [high, low],
        [high, high],
        [middle + 12.0, high],
        [middle, middle],
        [middle - 12.0, high],
        [low, high],
    ] {
        object.put_vertex(Vertex::new_position_color_uv_normal(
            x, y, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ));
    }
    object.end();

    object
}

/// 升目に置く図形。**0..TILE の正方形**で、角に色を散らしてある。
///
/// 頂点色を散らすのは、色に触るエフェクトが効いたか分かりやすくするため。
fn tile_shape(name: &str, paint_type: PaintType) -> Object {
    let mut object = create_object(name);

    object.begin(paint_type);

    for [x, y, r, g, b] in [
        [0.0, 0.0, 0.95, 0.95, 1.0],
        [TILE, 0.0, 0.75, 0.85, 1.0],
        [TILE, TILE, 0.85, 0.95, 0.9],
        [0.0, TILE, 1.0, 0.95, 0.8],
    ] {
        object.put_vertex(Vertex::new_position_color(x, y, 0.0, r, g, b, 1.0));
    }

    object.end();
    object
}

/// 画面全体のエフェクトの候補。`P` で順に切り替える。
fn post_effects() -> Vec<(&'static str, Option<PostEffect>)> {
    vec![
        ("なし", None),
        (
            "ColorGrade（白黒）",
            Some(PostEffect::ColorGrade {
                exposure: 1.0,
                saturation: 0.0,
                tint: WHITE,
            }),
        ),
        (
            "ColorGrade（明るく色味）",
            Some(PostEffect::ColorGrade {
                exposure: 1.6,
                saturation: 1.2,
                tint: [1.0, 0.85, 0.7, 1.0],
            }),
        ),
        (
            "Vignette",
            Some(PostEffect::Vignette {
                amount: 1.0,
                softness: 0.35,
            }),
        ),
        ("Blur", Some(PostEffect::Blur { radius: 4.0 })),
        (
            "Glow",
            Some(PostEffect::Glow {
                threshold: 0.6,
                intensity: 1.2,
                radius: 8.0,
            }),
        ),
    ]
}

// ---- 窓 ----

struct Application {
    renderer: Renderer,
    window: Option<Arc<dyn Window>>,
    scene: Option<Scene>,
    font: Option<Font<'static>>,

    started: Instant,
    /// 止めているあいだ進まない時間。
    clock: f32,
    paused: bool,
    labels: bool,
    post: usize,
}

impl Application {
    fn new(renderer_backend: RendererBackend) -> Self {
        // 名札用。読めなければ名札だけ諦める。絵は出る。
        let font = match example_font::read_japanese_font() {
            Ok((path, data)) => {
                println!("書体: {path}\n");
                Font::from_bytes(data.leak()).ok()
            }
            Err(error) => {
                println!("{error}\n名札なしで出します。\n");
                None
            }
        };

        // 縁を 4 点で均す。均さないと、小さい名札の細い画（1 px を割る）が
        // 画素の中心を外れて途切れる。
        let mut renderer = Renderer::new(renderer_backend);
        renderer.set_sample_count(DEFAULT_MULTISAMPLE);

        Self {
            renderer,
            window: None,
            scene: None,
            font,
            started: Instant::now(),
            clock: 0.0,
            paused: false,
            labels: true,
            post: 0,
        }
    }

    fn draw_frame(&mut self) {
        if !self.paused {
            self.clock = self.started.elapsed().as_secs_f32();
        } else {
            // 止めているあいだは、経った時間を巻き戻し続けて同じ絵を出す。
            self.started = Instant::now() - std::time::Duration::from_secs_f32(self.clock);
        }

        let Some(scene) = self.scene.as_mut() else {
            return;
        };

        let (Some(device), Some(queue)) = (self.renderer.device(), self.renderer.queue()) else {
            return;
        };

        scene.draw_manager.set_time(self.clock);

        if let Err(error) = scene.draw_manager.prepare(device, queue) {
            log::error!("failed to prepare the frame: {error}");
            return;
        }

        // まとまりごとに別の絵へ描いて、それぞれにエフェクトを通してから重ねる。
        // ぼかしは隣の画素を読むので、こうしないと図形ごとに掛けられない。
        self.renderer.render_passes(|frame| {
            clear(frame.encoder, frame.target);

            let groups: Vec<_> = scene
                .draw_manager
                .passes()
                .map(|pass| (pass.range.clone(), pass.chain.clone()))
                .collect();

            for (range, chain) in groups {
                let view = frame
                    .post_processor
                    .scene_view(frame.device, frame.width, frame.height, frame.format)
                    .clone();

                {
                    // 多点の描き先に描いて `view` へ均して書き出す。
                    // まとまりごとに透明で消す。前のを引きずらない。
                    let attachment = frame
                        .multisample
                        .color_attachment(&view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));

                    let mut pass =
                        frame
                            .encoder
                            .begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("gallery layer"),
                                color_attachments: &[Some(attachment)],
                                ..Default::default()
                            });

                    scene.draw_manager.draw_range(&mut pass, range);
                }

                frame
                    .post_processor
                    .run_over(frame.queue, frame.encoder, &chain, frame.target);
            }
        });
    }

    /// 名札を出し入れする。図形は触らない。
    fn rebuild_labels(&mut self) {
        let (Some(scene), Some(font)) = (self.scene.as_mut(), self.font.as_ref()) else {
            return;
        };

        scene.write_labels(font, self.labels);
    }

    fn cycle_post(&mut self) {
        let choices = post_effects();
        self.post = (self.post + 1) % choices.len();

        println!("ふつうの升目のエフェクト: {}", choices[self.post].0);

        let effect = choices[self.post].1;

        let Some(scene) = self.scene.as_mut() else {
            return;
        };

        // 自前のエフェクトを持たない升目にだけ掛ける。
        // **全部同じものになるので、パスは 1 回のまま**増えない。
        for name in scene.plain_tiles.clone() {
            let Some(object) = scene.draw_manager.object_mut(&name) else {
                continue;
            };

            object.clear_post_effects();

            if let Some(effect) = effect {
                object.post_effect(effect);
            }
        }
    }
}

impl ApplicationHandler for Application {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let rows = tiles().len().div_ceil(COLUMNS);
        let width = CELL * COLUMNS as f32;
        let height = CELL * rows as f32;

        let attributes = WindowAttributes::default()
            .with_title("gueiz effect gallery")
            .with_surface_size(LogicalSize::new(width, height));

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::<dyn Window>::from(window),
            Err(error) => {
                log::error!("failed to create the window: {error}");
                event_loop.exit();
                return;
            }
        };

        let size = window.surface_size();
        let surface_size = SurfaceSize::new(size.width, size.height);

        if let Err(error) = self.renderer.create_surface(window.clone(), surface_size) {
            log::error!("failed to create the surface: {error}");
            event_loop.exit();
            return;
        }

        match Scene::new(&self.renderer, width, height) {
            Ok(scene) => self.scene = Some(scene),
            Err(error) => {
                log::error!("failed to build the scene: {error}");
                event_loop.exit();
                return;
            }
        }

        self.rebuild_labels();
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => match logical_key.as_ref() {
                Key::Named(NamedKey::Escape) => event_loop.exit(),

                Key::Character(" ") => {
                    self.paused = !self.paused;
                    println!("時間: {}", if self.paused { "止めた" } else { "動かした" });
                }

                Key::Character("p") | Key::Character("P") => self.cycle_post(),

                Key::Character("l") | Key::Character("L") => {
                    self.labels = !self.labels;
                    self.rebuild_labels();
                }

                _ => {}
            },

            WindowEvent::SurfaceResized(size) => {
                self.renderer
                    .resize(SurfaceSize::new(size.width, size.height));
            }

            WindowEvent::RedrawRequested => self.draw_frame(),

            _ => {}
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.scene = None;
        self.renderer.destroy_surface();
        self.window = None;
    }
}

struct Scene {
    draw_manager: DrawManager,
    text: TextRenderer,
    /// 名札を置く位置。図形と同じ並び。
    label_spots: Vec<(&'static str, f32, f32)>,
    /// 自前のエフェクトを持たない升目の名前。`P` はここに掛ける。
    plain_tiles: Vec<String>,
    camera: Camera,
}

impl Scene {
    fn new(renderer: &Renderer, width: f32, height: f32) -> Result<Self, Box<dyn Error>> {
        let (Some(device), Some(queue), Some(format)) = (
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
        ) else {
            return Err("the renderer has no surface yet".into());
        };

        // 図形を描く側も、描き先と同じ数で均す。食い違うと wgpu が弾く。
        let descriptor = DrawManagerDescriptor {
            sample_count: renderer.sample_count(),
            ..DrawManagerDescriptor::default()
        };
        let mut draw_manager = DrawManager::new(device, queue, format, &descriptor)?;

        // 「Custom (縞)」の中身。自前の WGSL を 1 つだけ積んでおく。
        draw_manager.set_custom_blocks(device, &[custom_stripes()])?;

        // 窓を引っぱっても並びが崩れないよう、収まるように拡大する。
        let mut camera = Camera::orthographic_2d(width, height);
        camera.set_scale_mode(ScaleMode::Fit);

        let mut label_spots = Vec::new();
        let mut plain_tiles = Vec::new();

        for (index, tile) in tiles().into_iter().enumerate() {
            // 升目の中で、名札のぶんだけ上に寄せて中央に置く。
            let (x, y) = tile_origin(index);
            let mut object = tile_shape(tile.label, tile.paint_type);

            for block in &tile.blocks {
                object.effect(shifted(*block, x, y));
            }

            // 焼いた覆いは、図形と同じ座標で焼いてから積む。
            if let Some(kind) = tile.mask {
                let mask = draw_manager.add_clip_mask(device, queue, &notch(), kind);
                object.effect(shifted(mask.block(), x, y));
            }

            object.camera(camera);

            if let Some(post) = tile.post {
                object.post_effect(post);
            }
            object.translate(x, y, 0.0);
            object.instance(create_instance());

            let name = draw_manager.register(object);

            if tile.post.is_none() {
                plain_tiles.push(name);
            }

            let column = (index % COLUMNS) as f32;
            label_spots.push((tile.label, column * CELL + 6.0, y + TILE + 4.0));
        }

        let mut text = TextRenderer::new();
        text.camera(camera);
        // 図形より手前に置く。
        text.z(1.0);

        Ok(Self {
            draw_manager,
            text,
            label_spots,
            plain_tiles,
            camera,
        })
    }

    fn write_labels(&mut self, font: &Font, show: bool) {
        self.text.clear(&mut self.draw_manager);

        if !show {
            return;
        }

        self.text.camera(self.camera);
        self.text.z(1.0);

        let name_style = TextStyle::new(13.0);
        let hint_style = TextStyle::new(11.0);

        for (label, x, y) in self.label_spots.clone() {
            self.text.color(1.0, 1.0, 1.0, 1.0);

            if let Err(error) =
                self.text
                    .write(&mut self.draw_manager, font, label, &name_style, x, y)
            {
                log::warn!("名札 '{label}' を置けませんでした: {error}");
                continue;
            }

            let hint = hint(label);

            if hint.is_empty() {
                continue;
            }

            self.text.color(0.55, 0.75, 0.55, 1.0);

            let _ = self.text.write(
                &mut self.draw_manager,
                font,
                hint,
                &hint_style,
                x,
                y + 14.0,
            );
        }
    }
}

// ---- 自己点検 ----

/// 点検で見る時刻。
///
/// **2 つ見るのは、たまたま基準と重なる瞬間があるから。** 時刻 0 では
/// 回転も揺れも脈も 0 なので、動くエフェクトが「効いていない」ことになります。
const PROBE_TIMES: [f32; 2] = [0.37, 1.23];

/// 画素が違うと見なす差。読み戻しは sRGB のままで見る。
const DIFFERENT: u8 = 6;

/// 升目 1 つのうち、これだけ違えば「効いている」。
const ENOUGH: usize = 40;

/// ウィンドウを開かずに、**全升目が基準と違う絵になっているか**を数える。
///
/// 目で見るだけでは「黙って何もしないエフェクト」を見落とします。
/// ここは値の正しさは見ません。**効いているかどうか**だけを見ます。
fn check(renderer_backend: RendererBackend) -> Result<(), Box<dyn Error>> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        // 指定があればその裏側で。
        ..Default::default()
    }))?;
    let _ = renderer_backend;

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("effect gallery check"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    let list = tiles();
    let rows = list.len().div_ceil(COLUMNS);
    let width = (CELL * COLUMNS as f32) as u32;
    let height = (CELL * rows as f32) as u32;

    println!("adapter: {}", adapter.get_info().name);
    println!("升目 {} 個を {width}x{height} に描いて、基準と比べます\n", list.len());

    let mut frames = Vec::new();

    for time in PROBE_TIMES {
        frames.push(paint_gallery(&device, &queue, width, height, time)?);
    }

    let stride = padded_row(width);
    let mut dead = Vec::new();

    for (index, tile) in list.iter().enumerate() {
        // 基準の升目と比べる。0 番は基準そのものなので飛ばす。
        if index == 0 {
            println!("{:<22} 基準", tile.label);
            continue;
        }

        let most = frames
            .iter()
            .map(|pixels| cell_difference(pixels, stride, height, 0, index))
            .max()
            .unwrap_or(0);

        let alive = most >= ENOUGH;

        println!(
            "{:<22} {} 基準と違う画素 {most}",
            tile.label,
            if alive { "○" } else { "✗" },
        );

        if !alive {
            dead.push(tile.label);
        }
    }

    println!();

    if dead.is_empty() {
        println!("すべての升目が基準と違う絵になっています。");
        return Ok(());
    }

    Err(format!(
        "{} 個が基準と同じ絵のままです: {}",
        dead.len(),
        dead.join(", "),
    )
    .into())
}

/// 読み戻しの 1 行の長さ。256 バイト境界に揃える。
fn padded_row(width: u32) -> u32 {
    let bytes = width * 4;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    bytes.div_ceil(alignment) * alignment
}

/// 2 つの升目で、色が違う画素の数。
fn cell_difference(pixels: &[u8], stride: u32, height: u32, left: usize, right: usize) -> usize {
    let cell = CELL as u32;

    let origin = |index: usize| {
        (
            (index % COLUMNS) as u32 * cell,
            (index / COLUMNS) as u32 * cell,
        )
    };

    let (left_x, left_y) = origin(left);
    let (right_x, right_y) = origin(right);

    let at = |x: u32, y: u32| (y * stride + x * 4) as usize;
    let mut differing = 0;

    for row in 0..cell {
        if left_y + row >= height || right_y + row >= height {
            break;
        }

        for column in 0..cell {
            let a = at(left_x + column, left_y + row);
            let b = at(right_x + column, right_y + row);

            let off = (0..3).any(|channel| {
                pixels[a + channel].abs_diff(pixels[b + channel]) > DIFFERENT
            });

            if off {
                differing += 1;
            }
        }
    }

    differing
}

/// 並べた絵を 1 枚描いて読み戻す。
fn paint_gallery(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    time: f32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    const FORMAT: gueiz_2d::texture::TextureFormat =
        gueiz_2d::texture::TextureFormat::Bgra8UnormSrgb;

    let mut draw_manager =
        DrawManager::new(device, queue, FORMAT, &DrawManagerDescriptor::default())?;

    draw_manager.set_custom_blocks(device, &[custom_stripes()])?;

    let camera = Camera::orthographic_2d(width as f32, height as f32);

    for (index, tile) in tiles().into_iter().enumerate() {
        let (x, y) = tile_origin(index);
        let mut object = tile_shape(tile.label, tile.paint_type);

        for block in &tile.blocks {
            object.effect(shifted(*block, x, y));
        }

        if let Some(kind) = tile.mask {
            let mask = draw_manager.add_clip_mask(device, queue, &notch(), kind);
            object.effect(shifted(mask.block(), x, y));
        }

        object.camera(camera);

        if let Some(post) = tile.post {
            object.post_effect(post);
        }
        object.translate(x, y, 0.0);
        object.instance(create_instance());

        draw_manager.register(object);
    }

    draw_manager.set_time(time);
    draw_manager.prepare(device, queue)?;

    let stride = padded_row(width);
    let mut processor = PostProcessor::new(device, FORMAT.into());

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("effect gallery check target"),
        size: wgpu::Extent3d {
            width,
            height,
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
        label: Some("effect gallery check readback"),
        size: (stride * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("effect gallery check"),
    });

    // 窓と同じ道を通す。層のエフェクトもここで確かめられる。
    processor.begin_frame();
    clear(&mut encoder, &view);

    let groups: Vec<_> = draw_manager
        .passes()
        .map(|pass| (pass.range.clone(), pass.chain.clone()))
        .collect();

    for (range, chain) in groups {
        let scene = processor
            .scene_view(device, width, height, FORMAT.into())
            .clone();

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effect gallery check layer"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &scene,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            draw_manager.draw_range(&mut pass, range);
        }

        processor.run_over(queue, &mut encoder, &chain, &view);
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
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    readback.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let mapped = readback.get_mapped_range(..)?;
    let pixels = mapped.to_vec();
    drop(mapped);
    readback.unmap();

    Ok(pixels)
}
