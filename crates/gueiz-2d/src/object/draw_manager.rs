//! 登録された図形を **GPU-driven + インダイレクト描画**でまとめて描く。
//!
//! # 流れ
//!
//! 1. CPU は「図形の記述」と「インスタンスの素データ（TRS と色）」を上げるだけ。
//!    変換行列は計算しない。
//! 2. コンピュートパスがインスタンスごとに 1 スレッド走り、
//!    - 変換行列を組み立て（`camera * object * instance`）
//!    - 画面外なら捨て（視錐台カリング）
//!    - 生き残りを詰めて出力バッファに書き
//!    - そのぶん `DrawIndirectArgs.instance_count` を atomicAdd で増やす
//! 3. `multi_draw_indirect` が、GPU が書いた引数どおりに図形ごとのドローを発行する。
//!
//! # なぜドローを増やすのか
//!
//! 1 回の `draw` で全インスタンスを描くと、頂点数が全図形の最大値に揃う。
//! 頂点 3 個の三角形も、頂点 96 個の折れ線に合わせて 96 回頂点シェーダを起動し、
//! 93 回ぶんを捨てることになる。図形ごとにドローを分けるとこの無駄が消える。
//! CPU から見たコマンドは `multi_draw_indirect` 1 回のままで、
//! ドローの中身は GPU が決めるので、CPU の負荷は増えない。
//!
//! # バッファ
//!
//! | | usage | 誰が書くか |
//! |---|---|---|
//! | 形状プール | `STORAGE \| COPY_DST` | CPU（形が変わったときだけ） |
//! | 図形の記述 | `STORAGE \| COPY_DST` | CPU（毎フレーム、図形の数ぶん） |
//! | インスタンス入力 | `STORAGE \| COPY_DST` | CPU（書き換わった図形のぶんだけ） |
//! | インスタンス出力 | `STORAGE \| VERTEX` | **GPU**（コンピュートパス） |
//! | インダイレクト引数 | `STORAGE \| INDIRECT \| COPY_DST` | CPU が初期化、**GPU** が個数を書く |
//!
//! # アルファ
//!
//! 出力は**乗算済みアルファ**（`rgb` に `a` が掛かった状態）で、ブレンドは
//! [`wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING`]。
//! こうしないと `over` 合成が結合則を満たさず、半透明を重ねたときに
//! 順番で色がずれ、縮小・拡大で縁が暗くなる。
//!
//! エフェクトの山どうしは「普通の」アルファで計算し、フラグメントが出す
//! 直前にだけ乗算済みへ直している。山を書く側は直感的なままでよい。
//!
//! # 送り直しを省く
//!
//! インスタンスは数が多いので、毎フレーム全部を組み直して上げると
//! そこがいちばん重くなる（100 万個で 13 ms 前後）。そこで
//!
//! - [`Object`] は書き換えられたときだけ印を立て（[`Object::instances_mut`] など）、
//! - [`DrawManager`] は各図形のインスタンス範囲を覚えておき、
//!   **並びが変わらないフレームでは印の立った図形の範囲だけ**送り直す。
//!
//! 図形の増減やインスタンス数の変化で並びが崩れたときだけ、全部を組み直す。
//! 動かさない図形は、登録した最初のフレーム以降いっさい CPU 時間を食わない。

use crate::buffer::{Allocation, BufferHeap, BufferHeapDescriptor};
use std::sync::Arc;

use fxhash::FxHashMap;

use gueiz_gpu::instance::{InstanceSource, InstanceUploader};
use gueiz_gpu::msaa::NO_MULTISAMPLE;
use gueiz_gpu::pool::{Pool, PoolSource};

use crate::effect::{BlockRaw, CustomBlock, EffectStage, CUSTOM_KIND_BASE};
use crate::error::Gueiz2DError;
use crate::object::instance::Instance;
use crate::object::object::Object;
use crate::sprite::SpriteSheet;
use crate::texture::TextureFormat;
use crate::vertex::Vertex;

/// コンピュートシェーダのワークグループサイズ。シェーダ側と揃える。
const CULL_WORKGROUP_SIZE: u32 = 64;

const DEFAULT_POOL_SIZE: u64 = 4 * 1024 * 1024;
const DEFAULT_MAX_OBJECTS: u32 = 1024;
const DEFAULT_MAX_INSTANCES: u32 = 64 * 1024;
const DEFAULT_MAX_EFFECT_BLOCKS: u32 = 4 * 1024;

/// [`DrawManager`] の作成パラメータ。
pub struct DrawManagerDescriptor {
    /// 全図形の頂点を収める容量（バイト）。
    pub pool_size: u64,
    /// 登録できる図形の上限。
    pub max_objects: u32,
    /// 全図形を合わせたインスタンスの上限。
    pub max_instances: u32,
    /// 全図形を合わせたエフェクトの山の上限。
    pub max_effect_blocks: u32,
    /// 画面外のインスタンスを GPU で捨てる。
    pub culling: bool,
    /// 縁のギザギザを均す点の数。1 で均さない、4 が無難。
    ///
    /// **描き先のアタッチメントと同じ数**にすること。
    /// [`gueiz_gpu::msaa::MultisampleTarget`] が面倒を見る。
    pub sample_count: u32,
}

impl Default for DrawManagerDescriptor {
    fn default() -> Self {
        Self {
            pool_size: DEFAULT_POOL_SIZE,
            max_objects: DEFAULT_MAX_OBJECTS,
            max_instances: DEFAULT_MAX_INSTANCES,
            max_effect_blocks: DEFAULT_MAX_EFFECT_BLOCKS,
            culling: true,
            sample_count: NO_MULTISAMPLE,
        }
    }
}

/// 図形 1 つぶんの記述。コンピュートシェーダが読む。
///
/// WGSL 側は `mat4x4<f32>` を含むので構造体のアラインメントが 16 になる。
/// サイズを 16 の倍数にするため、末尾に詰め物を明示している。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
struct ObjectRaw {
    /// `camera * object`。インスタンスの変換はこの後に GPU 側で掛かる。
    transform: [[f32; 4]; 4],
    vertex_base: u32,
    vertex_count: u32,
    instance_base: u32,
    instance_count: u32,
    /// ローカル原点からいちばん遠い頂点までの距離。カリングと UV に使う。
    bounding_radius: f32,
    /// この図形のエフェクトが、山のプールのどこから始まるか。
    effect_base: u32,
    /// そのうち Transform 段が何山か。先頭から数えて。
    transform_count: u32,
    /// 続く Color 段が何山か。
    color_count: u32,
    /// スプライトシートから切り出す範囲。`[u, v, 幅, 高さ]`。
    uv_rect: [f32; 4],
    /// 外接矩形。`[min_x, min_y, 1/幅, 1/高さ]`。頂点座標を 0..1 の UV に直す。
    bounds: [f32; 4],
    /// 読むスプライトシートの層。
    texture_layer: u32,
    /// 絵を貼るか。0 なら頂点色だけ。
    has_sprite: u32,
    _padding: [u32; 2],
}

/// インスタンスの素データ。行列にする前の TRS と色。
///
/// WGSL の `vec3<f32>` はアラインメント 16 なので、各成分の後に詰め物が要る。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceIn {
    translation: [f32; 3],
    _padding0: f32,
    scale: [f32; 3],
    _padding1: f32,
    rotation: [f32; 3],
    _padding2: f32,
    tint: [f32; 4],
}

/// コンピュートパスが書き出す、生き残ったインスタンス。頂点バッファとして読む。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceOut {
    transform: [[f32; 4]; 4],
    tint: [f32; 4],
    vertex_base: u32,
    /// Color 段の山がプールのどこから始まるか。コンピュートパスが畳んで書く。
    color_base: u32,
    color_count: u32,
    /// 図形の記述を引くための番号。外接矩形・切り出し範囲・層はそこから読む。
    ///
    /// **並べ替えた後の位置**なので、登録順とは限らない。
    object_index: u32,
}

impl InstanceOut {
    fn vertex_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<InstanceOut>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // mat4 は vec4 4 本に割って渡す。
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Uint32,
                },
                wgpu::VertexAttribute {
                    offset: 84,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Uint32,
                },
                wgpu::VertexAttribute {
                    offset: 88,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Uint32,
                },
                wgpu::VertexAttribute {
                    offset: 92,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Uint32,
                },
            ],
        }
    }
}

/// 形から測っておく値。置き場の持ち分（[`Pool`]）とは別に持つ。
///
/// 積む順や範囲は共通の [`Pool`] が見るので、ここには**2D 固有のぶんだけ**残る。
#[derive(Clone, Copy)]
#[derive(Debug, Default)]
struct ShapeMetrics {
    /// ローカル原点からいちばん遠い頂点までの距離。カリングの境界円。
    bounding_radius: f32,
    /// 外接矩形。`[min_x, min_y, 1/幅, 1/高さ]`。UV を作るのに使う。
    bounds: [f32; 4],
}

/// 山のプール内での 1 図形の持ち分。Transform 段が先、Color 段が後ろに並ぶ。
#[derive(Clone, Copy)]
#[derive(Debug, Default)]
struct EffectRange {
    base: u32,
    transform_count: u32,
    color_count: u32,
}

impl From<&Instance> for InstanceIn {
    fn from(instance: &Instance) -> Self {
        let [tx, ty, tz] = instance.translation();
        let [sx, sy, sz] = instance.scale_factors();
        let [rx, ry, rz] = instance.rotation_angles();

        Self {
            translation: [tx, ty, tz],
            _padding0: 0.0,
            scale: [sx, sy, sz],
            _padding1: 0.0,
            rotation: [rx, ry, rz],
            _padding2: 0.0,
            tint: instance.tint(),
        }
    }
}

const CULL_SHADER: &str = r#"
struct ObjectDesc {
    transform: mat4x4<f32>,
    vertex_base: u32,
    vertex_count: u32,
    instance_base: u32,
    instance_count: u32,
    bounding_radius: f32,
    effect_base: u32,
    transform_count: u32,
    color_count: u32,
    uv_rect: vec4<f32>,
    bounds: vec4<f32>,
    texture_layer: u32,
    has_sprite: u32,
    padding0: u32,
    padding1: u32,
}
/// エフェクトの 1 山。Rust 側の `BlockRaw` と並びを揃える。
struct EffectBlock {
    params: vec4<f32>,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    kind: u32,
    padding0: u32,
    padding1: u32,
    padding2: u32,
}


struct InstanceIn {
    translation: vec3<f32>,
    scale: vec3<f32>,
    rotation: vec3<f32>,
    tint: vec4<f32>,
}

struct InstanceOut {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    vertex_base: u32,
    color_base: u32,
    color_count: u32,
    object_index: u32,
}

// `wgpu::util::DrawIndirectArgs` と同じ並び。
// instance_count だけ atomic にして、生き残りの数を GPU 側で数える。
struct DrawArgs {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
}

struct CullConfig {
    object_count: u32,
    culling: u32,
    time: f32,
    padding0: u32,
}

@group(0) @binding(0) var<storage, read>       objects: array<ObjectDesc>;
@group(0) @binding(1) var<storage, read>       instances_in: array<InstanceIn>;
@group(0) @binding(2) var<storage, read_write> instances_out: array<InstanceOut>;
@group(0) @binding(3) var<storage, read_write> draws: array<DrawArgs>;
@group(0) @binding(4) var<uniform>             config: CullConfig;
@group(0) @binding(5) var<storage, read>       blocks: array<EffectBlock>;

/// Transform 段の 1 山を掛ける。VFX Graph の Update Context の Block にあたる。
///
/// `seed` はインスタンスごとの個体差。同じ図形の複製が揃って動かないようにする。
fn apply_transform_block(instance: ptr<function, InstanceIn>, block: EffectBlock, time: f32, seed: f32) {
    switch block.kind {
        // Spin: 回し続ける。
        case 1u: {
            (*instance).rotation.z = (*instance).rotation.z + block.params.x * time;
        }
        // Wobble: 上下に揺らす。
        case 2u: {
            let offset = sin(time * block.params.y + seed) * block.params.x;
            (*instance).translation.y = (*instance).translation.y + offset;
        }
        // Orbit: 元の位置のまわりを回る。
        case 3u: {
            let angle = time * block.params.y + seed;
            (*instance).translation.x = (*instance).translation.x + cos(angle) * block.params.x;
            (*instance).translation.y = (*instance).translation.y + sin(angle) * block.params.x;
        }
        // Pulse: 大きさを脈打たせる。
        case 4u: {
            let factor = 1.0 + sin(time * block.params.y + seed) * block.params.x;
            (*instance).scale = (*instance).scale * vec3<f32>(factor, factor, 1.0);
        }
//GUEIZ_CUSTOM_TRANSFORM
        default: {}
    }
}

fn rotation_matrix(rotation: vec3<f32>) -> mat3x3<f32> {
    let cx = cos(rotation.x);
    let sx = sin(rotation.x);
    let cy = cos(rotation.y);
    let sy = sin(rotation.y);
    let cz = cos(rotation.z);
    let sz = sin(rotation.z);

    // CPU 側の Mat4::from_rotation と同じ Rx * Ry * Rz。
    let rx = mat3x3<f32>(
        vec3<f32>(1.0, 0.0, 0.0),
        vec3<f32>(0.0, cx, sx),
        vec3<f32>(0.0, -sx, cx),
    );
    let ry = mat3x3<f32>(
        vec3<f32>(cy, 0.0, -sy),
        vec3<f32>(0.0, 1.0, 0.0),
        vec3<f32>(sy, 0.0, cy),
    );
    let rz = mat3x3<f32>(
        vec3<f32>(cz, sz, 0.0),
        vec3<f32>(-sz, cz, 0.0),
        vec3<f32>(0.0, 0.0, 1.0),
    );

    return rx * ry * rz;
}

/// 平行移動 -> 回転 -> 拡大。CPU 側の Instance::transform と同じ順。
fn model_matrix(instance: InstanceIn) -> mat4x4<f32> {
    let rotation = rotation_matrix(instance.rotation);

    return mat4x4<f32>(
        vec4<f32>(rotation[0] * instance.scale.x, 0.0),
        vec4<f32>(rotation[1] * instance.scale.y, 0.0),
        vec4<f32>(rotation[2] * instance.scale.z, 0.0),
        vec4<f32>(instance.translation, 1.0),
    );
}

// x = 図形内のインスタンス番号、y = 図形番号。
@compute @workgroup_size(64, 1, 1)
fn cull_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let object_index = id.y;
    if (object_index >= config.object_count) {
        return;
    }

    let object = objects[object_index];
    let local_index = id.x;
    if (local_index >= object.instance_count || object.vertex_count == 0u) {
        return;
    }

    var instance = instances_in[object.instance_base + local_index];

    // Transform 段。積まれた順に掛ける。順番が結果を変える（回してから動かす、動かしてから回す）。
    let seed = f32(local_index) * 0.618;
    for (var step = 0u; step < object.transform_count; step = step + 1u) {
        apply_transform_block(&instance, blocks[object.effect_base + step], config.time, seed);
    }

    let transform = object.transform * model_matrix(instance);

    if (config.culling != 0u) {
        // ローカル原点を写し、境界円がクリップ空間と重なるかを見る。
        let center = transform * vec4<f32>(0.0, 0.0, 0.0, 1.0);

        // 変換後の半径は、上 2x2 の列の長さの大きいほうで見積もる。
        let scale_x = length(transform[0].xy);
        let scale_y = length(transform[1].xy);
        let radius = object.bounding_radius * max(scale_x, scale_y);

        if (center.x + radius < -1.0 || center.x - radius > 1.0 ||
            center.y + radius < -1.0 || center.y - radius > 1.0) {
            return;
        }
    }

    // 生き残りを前から詰める。戻り値が自分の置き場所。
    // 出力の枠は入力と同じ範囲なので、はみ出すことはない。
    let slot = atomicAdd(&draws[object_index].instance_count, 1u);

    var output: InstanceOut;
    output.transform = transform;
    output.tint = instance.tint;
    output.vertex_base = object.vertex_base;
    // Color 段は Transform 段の後ろに並んでいる。頭出しをここで畳んでおく。
    output.color_base = object.effect_base + object.transform_count;
    output.color_count = object.color_count;
    // 外接矩形・切り出し範囲・層は、描くときに同じ記述から読む。
    output.object_index = object_index;
    instances_out[object.instance_base + slot] = output;
}
"#;

const RENDER_SHADER: &str = r#"
// Rust 側の `Vertex` と同じ並び。
//
// vec3<f32> を使うと WGSL のアラインメント規則で 16 バイト境界に揃えられ、
// 構造体のサイズが 48 から 64 に変わって Rust 側とずれる。
// f32 を並べれば align 4 / size 48 のまま一致する。
struct PoolVertex {
    x: f32,
    y: f32,
    z: f32,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    u: f32,
    v: f32,
    nx: f32,
    ny: f32,
    nz: f32,
}

@group(0) @binding(0)
var<storage, read> shape_pool: array<PoolVertex>;

// コンピュート側と同じ並び。頂点シェーダが外接矩形と切り出し範囲を読む。
struct ObjectDesc {
    transform: mat4x4<f32>,
    vertex_base: u32,
    vertex_count: u32,
    instance_base: u32,
    instance_count: u32,
    bounding_radius: f32,
    effect_base: u32,
    transform_count: u32,
    color_count: u32,
    uv_rect: vec4<f32>,
    bounds: vec4<f32>,
    texture_layer: u32,
    has_sprite: u32,
    padding0: u32,
    padding1: u32,
}

@group(0) @binding(3)
var<storage, read> objects: array<ObjectDesc>;

@group(0) @binding(4)
var sprite_sheet: texture_2d_array<f32>;

@group(0) @binding(5)
var sprite_sampler: sampler;

/// エフェクトの 1 山。Rust 側の `BlockRaw` と並びを揃える。
struct EffectBlock {
    params: vec4<f32>,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    kind: u32,
    padding0: u32,
    padding1: u32,
    padding2: u32,
}

@group(0) @binding(1)
var<storage, read> blocks: array<EffectBlock>;

struct RenderConfig {
    object_count: u32,
    culling: u32,
    time: f32,
    padding0: u32,
}

@group(0) @binding(2)
var<uniform> config: RenderConfig;

fn hash21(point: vec2<f32>) -> f32 {
    var q = fract(point * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

/// 値ノイズ。Dissolve の溶け方に使う。
fn noise21(point: vec2<f32>) -> f32 {
    let cell = floor(point);
    let inner = fract(point);
    let weight = inner * inner * (3.0 - 2.0 * inner);

    let a = hash21(cell);
    let b = hash21(cell + vec2<f32>(1.0, 0.0));
    let c = hash21(cell + vec2<f32>(0.0, 1.0));
    let d = hash21(cell + vec2<f32>(1.0, 1.0));

    return mix(mix(a, b, weight.x), mix(c, d, weight.x), weight.y);
}

/// Color 段の 1 山を掛ける。VFX Graph の Output Context の Block にあたる。
fn apply_color_block(color: vec4<f32>, block: EffectBlock, uv: vec2<f32>, time: f32) -> vec4<f32> {
    switch block.kind {
        // Tint: 色を掛ける。
        case 5u: {
            return color * block.color_a;
        }
        // Gradient: 図形の中で色を混ぜて掛ける。
        case 6u: {
            let direction = vec2<f32>(cos(block.params.x), sin(block.params.x));
            let ratio = clamp(dot(uv - 0.5, direction) + 0.5, 0.0, 1.0);
            return color * mix(block.color_a, block.color_b, ratio);
        }
        // Dissolve: ノイズで溶かす。溶け際だけ別の色で光らせる。
        case 7u: {
            let value = noise21(uv * 8.0);
            let cut = block.params.x;
            let edge = block.params.y;

            if (value < cut) {
                return vec4<f32>(color.rgb, 0.0);
            }

            if (value < cut + edge) {
                return vec4<f32>(block.color_a.rgb, color.a * block.color_a.a);
            }

            return color;
        }
        // Flicker: 時間で明滅させる。
        case 8u: {
            let factor = 1.0 - block.params.x + sin(time * block.params.y) * block.params.x;
            return vec4<f32>(color.rgb, color.a * factor);
        }
//GUEIZ_CUSTOM_COLOR
        default: {
            return color;
        }
    }
}

struct InstanceInput {
    @location(0) transform_0: vec4<f32>,
    @location(1) transform_1: vec4<f32>,
    @location(2) transform_2: vec4<f32>,
    @location(3) transform_3: vec4<f32>,
    @location(4) tint: vec4<f32>,
    @location(5) vertex_base: u32,
    @location(6) color_base: u32,
    @location(7) color_count: u32,
    @location(8) object_index: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    /// 図形ローカルの UV。外接矩形を 0..1 に正規化したもの。エフェクトが使う。
    @location(1) uv: vec2<f32>,
    /// スプライトシートを読む UV。図形ローカルの UV を切り出し範囲に写したもの。
    @location(2) sprite_uv: vec2<f32>,
    // 索引と層はインスタンスごとに同じなので、補間しない。
    @location(3) @interpolate(flat) color_base: u32,
    @location(4) @interpolate(flat) color_count: u32,
    @location(5) @interpolate(flat) texture_layer: u32,
    @location(6) @interpolate(flat) has_sprite: u32,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: InstanceInput,
) -> VertexOutput {
    // 頂点数はインダイレクト引数で図形ごとに決まるので、範囲外チェックは要らない。
    let vertex = shape_pool[instance.vertex_base + vertex_index];

    // mat4x4 は列を取るので、Rust 側の列優先の並びがそのまま入る。
    let transform = mat4x4<f32>(
        instance.transform_0,
        instance.transform_1,
        instance.transform_2,
        instance.transform_3,
    );

    var output: VertexOutput;
    output.clip_position = transform * vec4<f32>(vertex.x, vertex.y, vertex.z, 1.0);
    output.color = instance.tint * vec4<f32>(vertex.r, vertex.g, vertex.b, vertex.a);

    // テッセレータは UV を書かないので、外接矩形から作る。
    // 四角なら 0..1 がちょうど四隅に来るので、絵が素直に収まる。
    let object = objects[instance.object_index];
    let uv = (vec2<f32>(vertex.x, vertex.y) - object.bounds.xy) * object.bounds.zw;

    output.uv = uv;
    // 切り出し範囲に写す。コマ送りはここを動かすだけで済む。
    output.sprite_uv = uv * object.uv_rect.zw + object.uv_rect.xy;

    output.color_base = instance.color_base;
    output.color_count = instance.color_count;
    output.texture_layer = object.texture_layer;
    output.has_sprite = object.has_sprite;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var color = input.color;

    // 絵を貼っていれば、その色を掛ける。頂点色は色味として残る。
    // Color 段より先に掛けるので、エフェクトは「絵に対して」効く。
    if (input.has_sprite != 0u) {
        color = color * textureSample(
            sprite_sheet,
            sprite_sampler,
            input.sprite_uv,
            input.texture_layer,
        );
    }

    // Color 段。積まれた順に掛ける。
    for (var step = 0u; step < input.color_count; step = step + 1u) {
        color = apply_color_block(color, blocks[input.color_base + step], input.uv, config.time);
    }

    // 山どうしは「普通の」アルファ（rgb と a が独立）で計算し、出すときだけ
    // 乗算済みに直す。書く側は直感的なまま、合成は結合則を満たす。
    return vec4<f32>(color.rgb * color.a, color.a);
}
"#;

/// コンピュートシェーダに渡す設定。ユニフォームなので 16 バイト境界に揃える。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
struct CullConfig {
    object_count: u32,
    culling: u32,
    /// 秒。Transform / Color 段の山が時間を使う。
    time: f32,
    _padding: u32,
}

/// 登録された図形をまとめて描く。
pub struct DrawManager {
    render_pipeline: wgpu::RenderPipeline,
    cull_pipeline: wgpu::ComputePipeline,
    render_bind_group: wgpu::BindGroup,
    cull_bind_group: wgpu::BindGroup,

    // 自前の山を差し替えるとパイプラインを組み直すので、材料を持っておく。
    cull_pipeline_layout: wgpu::PipelineLayout,
    render_pipeline_layout: wgpu::PipelineLayout,
    render_bind_group_layout: wgpu::BindGroupLayout,
    surface_format: TextureFormat,
    sample_count: u32,
    /// いま貼っている絵。差し替えるとバインドグループを組み直す。
    sprite_sheet: Arc<SpriteSheet>,

    pool_heap: BufferHeap,
    block_heap: BufferHeap,
    object_heap: BufferHeap,
    instance_input_heap: BufferHeap,
    instance_output_heap: BufferHeap,
    indirect_heap: BufferHeap,
    config_buffer: wgpu::Buffer,

    pool_allocation: Allocation,
    blocks_allocation: Allocation,
    objects_allocation: Allocation,
    instance_input: Allocation,
    instance_output: Allocation,
    indirect: Allocation,

    objects: Vec<Object>,
    /// 名前 → 番号。名前はここでひとつに保たれる。
    names: FxHashMap<String, usize>,
    /// 全図形の三角形をつないだ置き場。**3D と共通の仕組み**。
    shape_pool: Pool<Vertex>,
    shape_metrics: Vec<ShapeMetrics>,
    /// 各図形のエフェクトの持ち分。
    effect_ranges: Vec<EffectRange>,
    /// 描く順。z の小さい順に並べた図形の番号。
    draw_order: Vec<usize>,
    /// エフェクトのプールを積み直す必要がある。
    effects_dirty: bool,
    /// 秒。Transform / Color 段の山に渡る。
    time: f32,
    /// 複製の持ち分と、変わったぶんだけの送り直し。**3D と共通の仕組み**。
    instance_uploader: InstanceUploader<InstanceIn>,
    pool_dirty: bool,
    culling: bool,
    max_objects: u32,
    max_effect_blocks: u32,

    block_scratch: Vec<BlockRaw>,
    object_scratch: Vec<ObjectRaw>,
    indirect_scratch: Vec<wgpu::util::DrawIndirectArgs>,
    /// 今フレームの図形の数。ドローの発行数になる。
    draw_count: u32,
}

impl DrawManager {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: TextureFormat,
        descriptor: &DrawManagerDescriptor,
    ) -> Result<Self, Gueiz2DError> {
        let mut pool_heap = single_purpose_heap(
            device,
            "gueiz shape pool",
            descriptor.pool_size,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        // シェーダが `shape_pool[base + 番号]` で引くので、先頭から使う。
        // 積み直すたびに確保し直さず、最初にまるごと押さえておく（3D と同じ）。
        let pool_allocation = pool_heap.allocate(pool_heap.size())?;

        let mut block_heap = single_purpose_heap(
            device,
            "gueiz effect blocks",
            (descriptor.max_effect_blocks as u64 * size_of::<BlockRaw>() as u64).max(256),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        // 山が 1 つも無くてもバインドできるよう、丸ごと押さえておく。
        let blocks_allocation = block_heap.allocate(block_heap.size())?;

        let mut object_heap = single_purpose_heap(
            device,
            "gueiz objects",
            descriptor.max_objects as u64 * size_of::<ObjectRaw>() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let objects_allocation = object_heap.allocate(object_heap.size())?;

        let mut instance_input_heap = single_purpose_heap(
            device,
            "gueiz instance input",
            descriptor.max_instances as u64 * size_of::<InstanceIn>() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let instance_input = instance_input_heap.allocate(instance_input_heap.size())?;

        // 出力は GPU しか書かないので COPY_DST は要らない。
        let mut instance_output_heap = single_purpose_heap(
            device,
            "gueiz instance output",
            descriptor.max_instances as u64 * size_of::<InstanceOut>() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
        );
        let instance_output = instance_output_heap.allocate(instance_output_heap.size())?;

        let mut indirect_heap = single_purpose_heap(
            device,
            "gueiz indirect args",
            descriptor.max_objects as u64 * size_of::<wgpu::util::DrawIndirectArgs>() as u64,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST
                // 生き残った数を読み戻して確認できるように。
                | wgpu::BufferUsages::COPY_SRC,
        );
        let indirect = indirect_heap.allocate(indirect_heap.size())?;

        let config_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gueiz cull config"),
            size: size_of::<CullConfig>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- コンピュート側 ---
        let cull_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gueiz cull layout"),
                entries: &[
                    storage_entry(0, true),
                    storage_entry(1, true),
                    storage_entry(2, false),
                    storage_entry(3, false),
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    storage_entry(5, true),
                ],
            });

        let cull_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gueiz cull bind group"),
            layout: &cull_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: object_heap.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: instance_input_heap.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: instance_output_heap.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: indirect_heap.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: block_heap.buffer().as_entire_binding(),
                },
            ],
        });

        let cull_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gueiz cull pipeline layout"),
            bind_group_layouts: &[Some(&cull_bind_group_layout)],
            immediate_size: 0,
        });

        let cull_pipeline = build_cull_pipeline(device, &cull_pipeline_layout, &[])?;

        // --- 描画側 ---
        let render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gueiz shape pool layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // Color 段の山はフラグメントが読む。
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 外接矩形と切り出し範囲は頂点シェーダが読む。
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        // 絵を指定していない図形もここを読む。真っ白なので掛けても変わらない。
        let sprite_sheet = Arc::new(SpriteSheet::white(device, queue));

        let render_bind_group = build_render_bind_group(
            device,
            &render_bind_group_layout,
            pool_heap.buffer(),
            block_heap.buffer(),
            &config_buffer,
            object_heap.buffer(),
            &sprite_sheet,
        );

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("gueiz shape pipeline layout"),
                bind_group_layouts: &[Some(&render_bind_group_layout)],
                immediate_size: 0,
            });

        let render_pipeline = build_render_pipeline(
            device,
            &render_pipeline_layout,
            surface_format,
            descriptor.sample_count.max(1),
            &[],
        )?;

        log::info!(
            "draw manager: GPU-driven, culling {}, up to {} objects / {} instances",
            if descriptor.culling { "on" } else { "off" },
            descriptor.max_objects,
            descriptor.max_instances,
        );

        Ok(Self {
            render_pipeline,
            cull_pipeline,
            render_bind_group,
            cull_bind_group,
            cull_pipeline_layout,
            render_pipeline_layout,
            render_bind_group_layout,
            surface_format,
            sample_count: descriptor.sample_count.max(1),
            sprite_sheet,
            pool_heap,
            block_heap,
            object_heap,
            instance_input_heap,
            instance_output_heap,
            indirect_heap,
            config_buffer,
            pool_allocation,
            blocks_allocation,
            objects_allocation,
            instance_input,
            instance_output,
            indirect,
            objects: Vec::new(),
            names: FxHashMap::default(),
            shape_pool: Pool::new(),
            shape_metrics: Vec::new(),
            effect_ranges: Vec::new(),
            draw_order: Vec::new(),
            effects_dirty: true,
            time: 0.0,
            instance_uploader: InstanceUploader::new(descriptor.max_instances),
            pool_dirty: false,
            culling: descriptor.culling,
            max_objects: descriptor.max_objects,
            max_effect_blocks: descriptor.max_effect_blocks,
            block_scratch: Vec::new(),
            object_scratch: Vec::new(),
            indirect_scratch: Vec::new(),
            draw_count: 0,
        })
    }

    /// 図形を預ける。以降の書き換えは [`DrawManager::object_mut`] から。
    /// 図形を預ける。**返るのは、実際に付いた名前。**
    ///
    /// 渡した名前がすでに使われていたら後ろに数が足される
    /// （`Square` → `Square 2`）ので、返ってきたほうを使ってください。
    /// 以降の書き換えは [`DrawManager::object_mut`] から。
    ///
    /// ```no_run
    /// # use gueiz_2d::object::{self, DrawManager};
    /// # fn run(draw_manager: &mut DrawManager, square: object::Object) {
    /// let name = draw_manager.register(square);
    ///
    /// if let Some(object) = draw_manager.object_mut(&name) {
    ///     object.z(1.0);
    /// }
    /// # }
    /// ```
    pub fn register(&mut self, mut object: Object) -> String {
        if self.objects.len() as u32 >= self.max_objects {
            log::warn!(
                "the object limit ({}) is reached; '{}' will not be drawn",
                self.max_objects,
                object.name(),
            );
        }

        let index = self.objects.len();

        // 名前はここでひとつに保つ。重なっていたら後ろに数を足す。
        if self.names.contains_key(object.name()) {
            let unique = self.unique_name(object.name());

            log::warn!(
                "two objects are called '{}'; the new one is now '{}'",
                object.name(),
                unique,
            );

            object.rename(unique);
        }

        let name = String::from(object.name());
        self.names.insert(name.clone(), index);

        self.objects.push(object);
        self.shape_metrics.push(ShapeMetrics::default());
        self.effect_ranges.push(EffectRange::default());
        self.pool_dirty = true;
        self.instance_uploader.invalidate_layout();
        self.effects_dirty = true;

        name
    }

    /// まだ使われていない名前を作る。`Square` → `Square 2` → `Square 3`。
    fn unique_name(&self, wanted: &str) -> String {
        unique_name(wanted, |candidate| self.names.contains_key(candidate))
    }

    /// 名前で引く。
    ///
    /// 名前は [`DrawManager::register`] がひとつに保つので、
    /// **必ず図形ひとつに決まります。**
    pub fn object(&self, name: &str) -> Option<&Object> {
        self.objects.get(*self.names.get(name)?)
    }

    /// 名前で引いて書き換える。
    ///
    /// ```no_run
    /// # use gueiz_2d::object::DrawManager;
    /// # fn run(draw_manager: &mut DrawManager) {
    /// if let Some(square) = draw_manager.object_mut("Square") {
    ///     square.z(2.0);
    /// }
    /// # }
    /// ```
    pub fn object_mut(&mut self, name: &str) -> Option<&mut Object> {
        let index = *self.names.get(name)?;

        self.objects.get_mut(index)
    }

    /// その名前の図形があるか。
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// 預かっている名前を全部。順番は決まっていない。
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
    }

    /// 預かっている図形の数。
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// 画面の外の複製を GPU で捨てるかどうか。
    pub fn set_culling(&mut self, culling: bool) {
        self.culling = culling;
    }

    pub fn culling(&self) -> bool {
        self.culling
    }

    /// 絵を貼る。シートは 1 枚だけ持てる。
    ///
    /// どの層を読むかは図形ごと（[`Object::sprite`]）に決まる。
    ///
    /// シートは**持ち主と分け合います**（[`Arc`]）。渡した側は取っ手を
    /// 持ったままでいられるので、[`crate::resource::Resources`] に預けた絵が
    /// 差した瞬間に消える、ということが起きません。
    /// `SpriteSheet` をそのまま渡しても通ります。
    ///
    /// ```no_run
    /// # use gueiz_2d::sprite::{SpriteSheet, SpriteFilter};
    /// # fn run(
    /// #     device: &gueiz_2d::wgpu::Device,
    /// #     queue: &gueiz_2d::wgpu::Queue,
    /// #     draw_manager: &mut gueiz_2d::object::DrawManager,
    /// # ) -> Result<(), gueiz_2d::error::Gueiz2DError> {
    /// let sheet = SpriteSheet::new(device, queue, 32, 32, &[&[0u8; 32 * 32 * 4]], SpriteFilter::Nearest)?;
    /// draw_manager.set_sprite_sheet(device, sheet);
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_sprite_sheet(
        &mut self,
        device: &wgpu::Device,
        sprite_sheet: impl Into<Arc<SpriteSheet>>,
    ) {
        let sprite_sheet = sprite_sheet.into();

        self.render_bind_group = build_render_bind_group(
            device,
            &self.render_bind_group_layout,
            self.pool_heap.buffer(),
            self.block_heap.buffer(),
            &self.config_buffer,
            self.object_heap.buffer(),
            &sprite_sheet,
        );

        self.sprite_sheet = sprite_sheet;
    }

    /// いま貼っている絵。
    pub fn sprite_sheet(&self) -> &SpriteSheet {
        &self.sprite_sheet
    }

    /// いま貼っている絵を分け合う。
    pub fn shared_sprite_sheet(&self) -> Arc<SpriteSheet> {
        Arc::clone(&self.sprite_sheet)
    }

    /// 描く順。z の小さい順に並べた図形の番号。
    ///
    /// 手前ほど後ろに来る。[`DrawManager::prepare`] の後に見ること。
    pub fn draw_order(&self) -> &[usize] {
        &self.draw_order
    }

    /// 自前の山の中身を差し替える。パイプラインを組み直すので、
    /// 起動時か、山を書き換えたときにだけ呼ぶ。
    ///
    /// ここに渡さなかった番号の [`crate::effect::Block::Custom`] は何もしない。
    ///
    /// ```no_run
    /// # use gueiz_2d::effect::{CustomBlock, EffectStage, CUSTOM_KIND_BASE};
    /// # fn run(device: &gueiz_2d::wgpu::Device, draw_manager: &mut gueiz_2d::object::DrawManager)
    /// #     -> Result<(), gueiz_2d::error::Gueiz2DError> {
    /// draw_manager.set_custom_blocks(device, &[CustomBlock {
    ///     kind: CUSTOM_KIND_BASE,
    ///     stage: EffectStage::Color,
    ///     body: String::from("return color * block.color_a;"),
    /// }])?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_custom_blocks(
        &mut self,
        device: &wgpu::Device,
        blocks: &[CustomBlock],
    ) -> Result<(), Gueiz2DError> {
        for block in blocks {
            if block.kind < CUSTOM_KIND_BASE {
                return Err(Gueiz2DError::ReservedBlockKindError(block.kind));
            }

            if block.stage == EffectStage::Shape {
                return Err(Gueiz2DError::UnsupportedBlockStageError);
            }
        }

        // 片方だけ差し替わった状態にならないよう、両方できてから入れ替える。
        let cull_pipeline = build_cull_pipeline(device, &self.cull_pipeline_layout, blocks)?;
        let render_pipeline = build_render_pipeline(
            device,
            &self.render_pipeline_layout,
            self.surface_format,
            self.sample_count,
            blocks,
        )?;

        self.cull_pipeline = cull_pipeline;
        self.render_pipeline = render_pipeline;

        log::info!("custom blocks: {} spliced into the shaders", blocks.len());

        Ok(())
    }

    /// エフェクトに渡す時刻（秒）。
    ///
    /// 時間で動く山（[`crate::effect::Block::Spin`] など）はこれを見る。
    /// 毎フレーム進めるだけで、インスタンスの転送は増えない。
    pub fn set_time(&mut self, time: f32) {
        self.time = time;
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    /// 今フレームに発行するドローの数。図形の数と同じ。
    ///
    /// それぞれのインスタンス数は GPU が決めるので、CPU からは分からない。
    pub fn draw_count(&self) -> u32 {
        self.draw_count
    }

    /// GPU が書いたインダイレクト引数。読み戻して生き残り数を見るときに使う。
    ///
    /// 1 図形につき [`wgpu::util::DrawIndirectArgs`] 1 件が
    /// [`DrawManager::indirect_offset`] から並ぶ。
    pub fn indirect_buffer(&self) -> &wgpu::Buffer {
        self.indirect_heap.buffer()
    }

    pub fn indirect_offset(&self) -> u64 {
        self.indirect.offset()
    }

    /// 形が変わっていればプールを積み直し、コンピュートパスを投げる。
    ///
    /// [`DrawManager::draw`] の前に呼ぶ。コンピュートパスは専用のコマンドバッファで
    /// 先に投入されるので、描画側はその結果を読むだけでよい。
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), Gueiz2DError> {
        if self.pool_dirty || self.objects.iter().any(Object::is_geometry_dirty) {
            self.rebuild_pool(queue)?;
        }

        if self.effects_dirty || self.objects.iter().any(Object::is_effects_dirty) {
            self.rebuild_effects(queue);
        }

        self.upload_frame(queue);

        if self.draw_count == 0 {
            return Ok(());
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gueiz cull"),
        });

        {
            let mut cull_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gueiz cull pass"),
                timestamp_writes: None,
            });

            cull_pass.set_pipeline(&self.cull_pipeline);
            cull_pass.set_bind_group(0, &self.cull_bind_group, &[]);

            // x = 図形内のインスタンス、y = 図形。
            let groups_x = self
                .instance_uploader
                .max_instances_per_object()
                .div_ceil(CULL_WORKGROUP_SIZE);
            cull_pass.dispatch_workgroups(groups_x.max(1), self.draw_count, 1);
        }

        queue.submit(Some(encoder.finish()));

        Ok(())
    }

    /// GPU が書いた引数どおりに描く。CPU が出すコマンドは 1 つだけ。
    pub fn draw(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        if self.draw_count == 0 {
            return;
        }

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, &self.render_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.instance_output_heap.slice(&self.instance_output));
        render_pass.multi_draw_indirect(
            self.indirect_heap.buffer(),
            self.indirect.offset(),
            self.draw_count,
        );
    }

    /// 全図形の頂点を 1 本につないで積み直す。
    ///
    /// つなぐところは共通の [`Pool`] に任せ、ここは 2D 固有の
    /// [`ShapeMetrics`]（境界円と外接矩形）だけを測る。
    fn rebuild_pool(&mut self, queue: &wgpu::Queue) -> Result<(), Gueiz2DError> {
        {
            let source = Shapes(&self.objects);
            self.shape_pool.rebuild(&source);
        }

        self.shape_metrics.resize(self.objects.len(), ShapeMetrics::default());

        for (index, object) in self.objects.iter_mut().enumerate() {
            let triangles = object.triangles();

            self.shape_metrics[index] = ShapeMetrics {
                // ローカル原点からいちばん遠い頂点までの距離。
                bounding_radius: triangles
                    .iter()
                    .map(|vertex| (vertex.x * vertex.x + vertex.y * vertex.y).sqrt())
                    .fold(0.0_f32, f32::max),
                bounds: bounding_box(triangles),
            };

            object.clear_geometry_dirty();
        }

        self.shape_pool
            .upload(queue, &self.pool_heap, &self.pool_allocation)?;

        self.pool_dirty = false;

        // 実行中に頂点を編集すると毎回ここを通るので、既定では出さない。
        log::debug!(
            "pool: {} objects, {} vertices ({} bytes)",
            self.objects.len(),
            self.shape_pool.len(),
            self.shape_pool.byte_len(),
        );

        Ok(())
    }

    /// 全図形のエフェクトを 1 本につないで積み直す。
    ///
    /// 図形ごとに Transform 段が先、Color 段が後ろ。コンピュートパスは前半を、
    /// フラグメントは後半を読む。Shape 段は CPU 側で三角形になっているので載せない。
    fn rebuild_effects(&mut self, queue: &wgpu::Queue) {
        self.block_scratch.clear();

        for (index, object) in self.objects.iter_mut().enumerate() {
            let base = self.block_scratch.len() as u32;
            let effects = object.effects();

            let transform_blocks = effects.blocks(EffectStage::Transform);
            let color_blocks = effects.blocks(EffectStage::Color);

            // 入り切らないぶんは捨てる。黙って化けるよりは出ないほうがよい。
            let room = self.max_effect_blocks.saturating_sub(base) as usize;
            let transform_count = transform_blocks.len().min(room);
            let color_count = color_blocks.len().min(room - transform_count);

            if transform_count < transform_blocks.len() || color_count < color_blocks.len() {
                log::warn!(
                    "the effect block limit ({}) is reached; some blocks on '{}' will not run",
                    self.max_effect_blocks,
                    object.name(),
                );
            }

            self.block_scratch
                .extend(transform_blocks[..transform_count].iter().map(|b| b.to_raw()));
            self.block_scratch
                .extend(color_blocks[..color_count].iter().map(|b| b.to_raw()));

            self.effect_ranges[index] = EffectRange {
                base,
                transform_count: transform_count as u32,
                color_count: color_count as u32,
            };

            object.clear_effects_dirty();
        }

        if !self.block_scratch.is_empty() {
            queue.write_buffer(
                self.block_heap.buffer(),
                self.blocks_allocation.offset(),
                bytemuck::cast_slice(&self.block_scratch),
            );
        }

        self.effects_dirty = false;

        log::debug!("effects: {} blocks", self.block_scratch.len());
    }

    /// 今フレームの図形記述・インスタンス・インダイレクト引数を上げる。
    ///
    /// 図形の記述とインダイレクト引数は図形の数ぶんしかないので毎フレーム上げる
    /// （インダイレクト引数は `instance_count` を 0 に戻す必要があるので、そもそも毎フレーム要る）。
    /// 重いのはインスタンスのほうで、こちらは書き換わったぶんだけを上げる。
    fn upload_frame(&mut self, queue: &wgpu::Queue) {
        let object_limit = self.objects.len().min(self.max_objects as usize);

        // 複製を送る。並びが変わっていなければ、書き換えられた図形のぶんだけ。
        // 仕組みは 3D と共通（[`InstanceUploader`]）。
        {
            let mut source = Instances(&mut self.objects[..object_limit]);
            self.instance_uploader.upload(
                queue,
                self.instance_input_heap.buffer(),
                self.instance_input.offset(),
                &mut source,
            );
        }

        self.object_scratch.clear();
        self.indirect_scratch.clear();

        // z の小さい順に描く。後から描いたものが上に乗る（画家のアルゴリズム）。
        // 深度バッファではなく描く順で解決しているので、半透明も正しく混ざる。
        //
        // 借用の都合で一度取り出す。容量は使い回されるので確保は起きない。
        let mut draw_order = std::mem::take(&mut self.draw_order);
        draw_order.clear();
        draw_order.extend(0..object_limit);
        // 安定ソートなので、z が同じなら登録順のまま。
        draw_order.sort_by(|&a, &b| self.objects[a].depth().total_cmp(&self.objects[b].depth()));

        for &index in &draw_order {
            let pool_range = self.shape_pool.range(index);
            let metrics = self.shape_metrics[index];
            let instance_range = self.instance_uploader.range(index);
            let effect_range = self.effect_ranges[index];
            let object = &self.objects[index];

            self.object_scratch.push(ObjectRaw {
                // カメラと図形の変換まではここで畳む。インスタンスは GPU 側。
                transform: object.view_transform().to_columns(),
                vertex_base: pool_range.base,
                vertex_count: pool_range.count,
                instance_base: instance_range.base,
                instance_count: instance_range.count,
                bounding_radius: metrics.bounding_radius,
                effect_base: effect_range.base,
                transform_count: effect_range.transform_count,
                color_count: effect_range.color_count,
                uv_rect: object.sprite_rect(),
                bounds: metrics.bounds,
                texture_layer: object.sprite_layer().unwrap_or(0),
                has_sprite: u32::from(object.sprite_layer().is_some()),
                _padding: [0; 2],
            });

            // instance_count は GPU が atomicAdd で数えるので 0 から始める。
            // first_instance は出力バッファ内の置き場所。入力と同じ範囲を使う。
            self.indirect_scratch.push(wgpu::util::DrawIndirectArgs {
                vertex_count: pool_range.count,
                instance_count: 0,
                first_vertex: 0,
                first_instance: instance_range.base,
            });
        }

        self.draw_order = draw_order;

        self.draw_count = self.object_scratch.len() as u32;

        if self.draw_count == 0 {
            return;
        }

        queue.write_buffer(
            self.object_heap.buffer(),
            self.objects_allocation.offset(),
            bytemuck::cast_slice(&self.object_scratch),
        );
        queue.write_buffer(
            self.indirect_heap.buffer(),
            self.indirect.offset(),
            draw_args_bytes(&self.indirect_scratch),
        );
        queue.write_buffer(
            &self.config_buffer,
            0,
            bytemuck::bytes_of(&CullConfig {
                object_count: self.draw_count,
                culling: u32::from(self.culling),
                time: self.time,
                _padding: 0,
            }),
        );
    }

}

/// 形の積み元。[`Pool`] が覗くだけの薄い型。
struct Shapes<'a>(&'a [Object]);

impl PoolSource for Shapes<'_> {
    type Item = Vertex;

    fn object_count(&self) -> usize {
        self.0.len()
    }

    fn elements(&self, object: usize) -> &[Vertex] {
        self.0[object].triangles()
    }
}

/// 複製の送り元。[`InstanceUploader`] が覗くだけの薄い型。
///
/// `DrawManager` 自身に実装すると `&mut self` が二重になるので、
/// 図形の列だけを借りる。
struct Instances<'a>(&'a mut [Object]);

impl InstanceSource for Instances<'_> {
    type Raw = InstanceIn;

    fn object_count(&self) -> usize {
        self.0.len()
    }

    fn instance_count(&self, object: usize) -> usize {
        self.0[object].instances().len()
    }

    fn is_dirty(&self, object: usize) -> bool {
        self.0[object].is_instances_dirty()
    }

    fn write(&self, object: usize, count: usize, out: &mut Vec<InstanceIn>) {
        let instances = self.0[object].instances();

        if instances.is_empty() {
            // 複製を 1 つも足さなかった図形。変換なしの複製で埋める。
            let default_instance = InstanceIn::from(&Instance::new());
            out.extend(std::iter::repeat_n(default_instance, count));
            return;
        }

        out.extend(instances.iter().take(count).map(InstanceIn::from));
    }

    fn clear_dirty(&mut self) {
        for object in self.0.iter_mut() {
            object.clear_instances_dirty();
        }
    }
}

/// 自前の山を `switch` の腕に組み立てる。
fn custom_cases(blocks: &[CustomBlock], stage: EffectStage) -> String {
    let mut cases = String::new();

    for block in blocks.iter().filter(|block| block.stage == stage) {
        cases.push_str(&format!(
            "        case {}u: {{\n{}\n        }}\n",
            block.kind, block.body,
        ));
    }

    cases
}

/// 差し込んだ WGSL ごとコンピュートパイプラインを組む。
fn build_cull_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    blocks: &[CustomBlock],
) -> Result<wgpu::ComputePipeline, Gueiz2DError> {
    let source = CULL_SHADER.replace(
        "//GUEIZ_CUSTOM_TRANSFORM",
        &custom_cases(blocks, EffectStage::Transform),
    );

    // 自前の WGSL が通らなかったときに、パニックではなく Err で返す。
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gueiz cull shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("gueiz cull pipeline"),
        layout: Some(layout),
        module: &module,
        entry_point: Some("cull_main"),
        compilation_options: Default::default(),
        cache: None,
    });

    check_error_scope(scope)?;

    Ok(pipeline)
}

/// 差し込んだ WGSL ごと描画パイプラインを組む。
fn build_render_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    surface_format: TextureFormat,
    sample_count: u32,
    blocks: &[CustomBlock],
) -> Result<wgpu::RenderPipeline, Gueiz2DError> {
    let source = RENDER_SHADER.replace(
        "//GUEIZ_CUSTOM_COLOR",
        &custom_cases(blocks, EffectStage::Color),
    );

    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gueiz shape shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gueiz shape pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(InstanceOut::vertex_buffer_layout())],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: sample_count,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format.into(),
                // 乗算済みアルファ。フラグメントが `rgb * a` を出すので、
                // src 係数は One でよい。重ねがけとポストで縁が暗くならない。
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    });

    check_error_scope(scope)?;

    Ok(pipeline)
}

/// 積んだエラースコープを取り出す。何も無ければ `Ok`。
fn check_error_scope(scope: wgpu::ErrorScopeGuard) -> Result<(), Gueiz2DError> {
    match pollster::block_on(scope.pop()) {
        Some(error) => Err(Gueiz2DError::ShaderCompilationError(error.to_string())),
        None => Ok(()),
    }
}

/// `DrawIndirectArgs` は `Pod` ではないので、バイト列として見る。
///
/// `#[repr(C)]` の u32 4 つで詰め物が無いことは wgpu 側で保証されている。
fn draw_args_bytes(args: &[wgpu::util::DrawIndirectArgs]) -> &[u8] {
    // SAFETY: DrawIndirectArgs は #[repr(C)] の u32 4 つで、詰め物も不正な
    // ビット列も持たない。読み出し専用のスライスとして見るだけ。
    unsafe {
        std::slice::from_raw_parts(
            args.as_ptr().cast::<u8>(),
            std::mem::size_of_val(args),
        )
    }
}

/// 描画側のバインドグループを組む。絵を差し替えるたびに組み直す。
fn build_render_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    pool: &wgpu::Buffer,
    blocks: &wgpu::Buffer,
    config: &wgpu::Buffer,
    objects: &wgpu::Buffer,
    sprite_sheet: &SpriteSheet,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gueiz shape pool bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pool.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: blocks.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: config.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: objects.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(sprite_sheet.view()),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(sprite_sheet.sampler()),
            },
        ],
    })
}

/// 頂点を囲む矩形を `[min_x, min_y, 1/幅, 1/高さ]` に直す。
///
/// 頂点座標から 0..1 の UV を作るのに使う。潰れた図形でも 0 除算しないよう、
/// 幅か高さが 0 なら 1 として扱う。
fn bounding_box(vertices: &[Vertex]) -> [f32; 4] {
    if vertices.is_empty() {
        return [0.0, 0.0, 1.0, 1.0];
    }

    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for vertex in vertices {
        min_x = min_x.min(vertex.x);
        min_y = min_y.min(vertex.y);
        max_x = max_x.max(vertex.x);
        max_y = max_y.max(vertex.y);
    }

    let width = max_x - min_x;
    let height = max_y - min_y;

    [
        min_x,
        min_y,
        if width > 0.0 { 1.0 / width } else { 1.0 },
        if height > 0.0 { 1.0 / height } else { 1.0 },
    ]
}

/// 用途 1 つぶんのバッファを作る。フレーム領域は使わない。
///
/// CPU からの書き込みは `Queue::write_buffer` が内部でステージングするので、
/// GPU が読んでいる最中のデータを壊す心配がない。
fn single_purpose_heap(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> BufferHeap {
    BufferHeap::new(
        device,
        &BufferHeapDescriptor {
            label: Some(label),
            size,
            frame_size: 0,
            frames_in_flight: 1,
            usage,
            // ストレージバッファのバインド境界に合わせる。
            alignment: 256,
        },
    )
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// まだ使われていない名前を作る。`Square` → `Square 2` → `Square 3`。
///
/// 2 から始めるのは、`Square` と `Square 2` のほうが
/// `Square 1` と `Square 2` より「どちらが元か」が分かるためです。
///
/// [`DrawManager`] は GPU が無いと組み立てられないので、
/// **名前の作り方だけをここに出して**単体で試せるようにしてあります。
fn unique_name(wanted: &str, taken: impl Fn(&str) -> bool) -> String {
    (2..)
        .map(|suffix| format!("{wanted} {suffix}"))
        .find(|candidate| !taken(candidate))
        .expect("番号は尽きない")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn taken_from(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| String::from(*name)).collect()
    }

    /// 空いていれば、いちばん小さい数が付く。
    #[test]
    fn the_first_spare_name_is_two() {
        let taken = taken_from(&["Square"]);

        assert_eq!(
            unique_name("Square", |name| taken.contains(name)),
            "Square 2",
        );
    }

    /// 2 も埋まっていれば 3 へ。飛ばさずに詰める。
    #[test]
    fn the_number_climbs_until_it_is_free() {
        let taken = taken_from(&["Square", "Square 2", "Square 3"]);

        assert_eq!(
            unique_name("Square", |name| taken.contains(name)),
            "Square 4",
        );
    }

    /// 途中が空いていればそこに入る。番号を無駄に伸ばさない。
    #[test]
    fn a_gap_in_the_numbers_gets_filled() {
        let taken = taken_from(&["Square", "Square 2", "Square 4"]);

        assert_eq!(
            unique_name("Square", |name| taken.contains(name)),
            "Square 3",
        );
    }

    /// 元の名前は残る。付け替えるのは**後から来たほう**。
    /// 先に登録したものの名前が変わると、それを持っている側が壊れる。
    #[test]
    fn the_suffix_never_touches_the_original() {
        let taken = taken_from(&["Player"]);
        let renamed = unique_name("Player", |name| taken.contains(name));

        assert_ne!(renamed, "Player");
        assert!(renamed.starts_with("Player "));
        assert!(taken.contains("Player"), "元の名前が残っていない");
    }

    /// 似ているだけの名前には引っぱられない。
    #[test]
    fn similar_names_do_not_collide() {
        let taken = taken_from(&["Square", "SquareShadow", "Squares"]);

        assert_eq!(
            unique_name("Square", |name| taken.contains(name)),
            "Square 2",
        );
    }

    /// 付けた名前をもう一度通しても、同じ名前は返らない。
    /// 返すと、順に登録したときに重なったままになる。
    #[test]
    fn feeding_the_result_back_keeps_climbing() {
        let mut taken = taken_from(&["Tile"]);

        for expected in ["Tile 2", "Tile 3", "Tile 4"] {
            let next = unique_name("Tile", |name| taken.contains(name));

            assert_eq!(next, expected);
            taken.insert(next);
        }
    }
}
