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
//! # 色は sRGB で受け取る
//!
//! 頂点の色も、複製の色味も、エフェクトの色も **sRGB** で書きます。
//! `#808080` と書けば、画面でも `#808080` の灰色が出ます。
//!
//! 受け取った色は**頂点シェーダの入口で線形の光に直します**。掛け算も
//! 混ぜも合成も線形で行い、出すときに GPU が sRGB へ戻します。
//! 絵（`Rgba8UnormSrgb`）もサンプルした時点で線形なので、同じ空間で揃います。
//!
//! 直さないと `0.5` が真ん中の灰色ではなく、かなり明るい灰色（sRGB 188）で
//! 出ます。**両端（0 と 1）は動かない**ので、原色だけ見ていると気づけません。
//!
//! **不透明度は変換しません。** 光の量ではなく覆う割合なので、
//! ガンマを掛けると半透明がずれます。
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
use std::ops::Range;
use std::sync::Arc;

use fxhash::FxHashMap;

use gueiz_gpu::instance::{InstanceSource, InstanceUploader};
use gueiz_gpu::msaa::NO_MULTISAMPLE;
use gueiz_gpu::pool::{Pool, PoolSource};

use crate::clip::{self, ClipMask, ClipMaskKind};
use crate::effect::{Block, BlockRaw, CustomBlock, EffectStage, CUSTOM_KIND_BASE};
use crate::error::Gueiz2DError;
use crate::instance::Instance;
use crate::object::Object;
use crate::post::PostChain;
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
    /// クリップの覆い 1 枚の辺の長さ（画素）。縁のなめらかさがこれで決まる。
    ///
    /// 覆いは 1 枚ごとにテクスチャ配列の 1 層を使うので、
    /// 1 枚あたり `辺 * 辺` バイト。256 なら 64KB。
    pub clip_mask_resolution: u32,
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
            clip_mask_resolution: DEFAULT_CLIP_MASK_RESOLUTION,
        }
    }
}

/// クリップの覆いの既定の辺の長さ。
pub const DEFAULT_CLIP_MASK_RESOLUTION: u32 = 256;

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
    /// カメラだけの逆行列。クリップ位置からワールド座標に戻すのに使う。
    ///
    /// 図形ごとに 1 つなので、インスタンスを増やしても増えない。
    /// カメラが逆行列を持てないときは単位行列（クリップ空間 = ワールド）。
    camera_inverse: [[f32; 4]; 4],
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
    camera_inverse: mat4x4<f32>,
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
    camera_inverse: mat4x4<f32>,
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

// 焼いたクリップの覆い。1 枚が 1 層。
@group(0) @binding(6)
var clip_masks: texture_2d_array<f32>;

@group(0) @binding(7)
var clip_sampler: sampler;

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
/// クリップの距離から、その画素がどれだけ残るかを出す。
///
/// `distance` は外を正とする符号付き距離（ワールド単位）。
/// `softness` が 0 でも、1 画素ぶんはなめらかに落とす。
fn clip_coverage(distance: f32, softness: f32, world_per_pixel: f32) -> f32 {
    let feather = max(softness, 0.0) * 0.5 + world_per_pixel;

    return 1.0 - smoothstep(-feather, feather, distance);
}

/// 裏返す指定が立っていれば、内と外を入れ替える。
fn clip_flip(coverage: f32, invert: f32) -> f32 {
    return select(coverage, 1.0 - coverage, invert != 0.0);
}

/// 覆いを掛けた色を返す。出すのは乗算前なので、不透明度だけを削る。
fn clip_apply(color: vec4<f32>, coverage: f32) -> vec4<f32> {
    return vec4<f32>(color.rgb, color.a * coverage);
}

/// 角の丸い矩形までの符号付き距離。中が負、外が正。
fn clip_rect_distance(point: vec2<f32>, low: vec2<f32>, high: vec2<f32>, radius: f32) -> f32 {
    let half = (high - low) * 0.5;
    let center = (high + low) * 0.5;

    // 丸みは半分の幅・高さを超えられない。
    let corner = clamp(radius, 0.0, min(half.x, half.y));
    let offset = abs(point - center) - (half - vec2<f32>(corner));

    return length(max(offset, vec2<f32>(0.0))) + min(max(offset.x, offset.y), 0.0) - corner;
}

/// 楕円までの距離。円のときは厳密で、細長いほど縁の幅がわずかにずれる。
fn clip_ellipse_distance(point: vec2<f32>, center: vec2<f32>, radii: vec2<f32>) -> f32 {
    let safe = max(radii, vec2<f32>(1e-6));
    let offset = (point - center) / safe;

    return (length(offset) - 1.0) * min(safe.x, safe.y);
}

/// その山が積みで何個ぶんを占めるか。**必ず 1 以上を返す。**
///
/// これは節約であって、無くても絵は変わらない。色を収めた続きは
/// `switch` に無い番号なので、踏んでも素通りするだけ。
/// ただし**多く見積もるとあとの山が飛ばされる**ので、
/// `Block::raw_count` と同じ数え方であること。
///
/// 0 を返すと、色を混ぜる輪が進まなくなる。
fn block_span(block: EffectBlock) -> u32 {
    // グラデーションは色を 2 つずつ収めた続きを従える。
    if (block.kind == 14u || block.kind == 15u || block.kind == 16u) {
        return 1u + (u32(block.color_a.x) + 1u) / 2u;
    }

    return 1u;
}

/// 色を置いた並びから、`ratio` のところの色を引く。
///
/// 色は積むときに線形へ直してあるので、ここでの混ぜは光の量どうしの混ぜ。
fn gradient_color(base: u32, count: u32, spread: f32, ratio: f32) -> vec4<f32> {
    var position = ratio;

    // 0..1 の外をどう埋めるか。
    if (spread == 1.0) {
        position = fract(position);
    } else if (spread == 2.0) {
        let folded = fract(position * 0.5) * 2.0;
        position = select(folded, 2.0 - folded, folded > 1.0);
    } else {
        position = clamp(position, 0.0, 1.0);
    }

    // 色は 2 つずつ収まっている。`index` 番目を引く。
    let stop = func_stop(base, 0u);
    var low = stop;
    var high = stop;

    for (var index = 1u; index < count; index = index + 1u) {
        let current = func_stop(base, index);

        if (current.position <= position) {
            low = current;
        } else {
            high = current;
            break;
        }

        high = current;
    }

    let span = high.position - low.position;

    // 同じ位置に重なっていたら混ぜようがない。後ろを採る。
    if (span <= 0.0) {
        return high.color;
    }

    return mix(low.color, high.color, clamp((position - low.position) / span, 0.0, 1.0));
}

struct GradientStop {
    position: f32,
    color: vec4<f32>,
}

/// 色を収めた続きから `index` 番目を取り出す。2 つで 1 つぶん。
fn func_stop(base: u32, index: u32) -> GradientStop {
    let entry = blocks[base + 1u + index / 2u];

    var stop: GradientStop;

    if (index % 2u == 0u) {
        stop.position = entry.params.x;
        stop.color = entry.color_a;
    } else {
        stop.position = entry.params.y;
        stop.color = entry.color_b;
    }

    return stop;
}

fn apply_color_block(
    color: vec4<f32>,
    at: u32,
    uv: vec2<f32>,
    time: f32,
    world: vec2<f32>,
    world_per_pixel: f32,
) -> vec4<f32> {
    let block = blocks[at];

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
        // ClipRect: 矩形の外を削る。
        case 9u: {
            let distance = clip_rect_distance(
                world,
                block.params.xy,
                block.params.zw,
                block.color_a.x,
            );
            let coverage = clip_coverage(distance, block.color_a.y, world_per_pixel);

            return clip_apply(color, clip_flip(coverage, block.color_a.z));
        }
        // ClipEllipse: 楕円の外を削る。
        case 10u: {
            let distance = clip_ellipse_distance(world, block.params.xy, block.params.zw);
            let coverage = clip_coverage(distance, block.color_a.x, world_per_pixel);

            return clip_apply(color, clip_flip(coverage, block.color_a.y));
        }
        // ClipHalfPlane: 直線の向こう側を削る。積むと凸多角形になる。
        case 11u: {
            let normal = block.params.xy;
            let span = length(normal);

            // 向きが決まらなければ削らない。
            if (span < 1e-6) {
                return color;
            }

            let distance = dot(normal / span, world) - block.params.z / span;
            let coverage = clip_coverage(distance, block.params.w, world_per_pixel);

            return clip_apply(color, clip_flip(coverage, block.color_a.x));
        }
        // ClipDistanceMask: 距離を焼いた覆い。拡大しても縁が保てる。
        case 13u: {
            let uv = (world - block.params.xy) * block.params.zw;
            let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));

            let sampled = textureSampleLevel(
                clip_masks,
                clip_sampler,
                clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)),
                i32(block.color_a.x),
                0.0,
            ).r;

            // 0.5 が境目で、大きいほうが中。外を正にして距離に戻す。
            let distance = (0.5 - sampled) * 2.0 * block.color_a.z;
            let coverage = clip_coverage(distance, block.color_a.w, world_per_pixel);

            return clip_apply(color, clip_flip(select(0.0, coverage, inside), block.color_a.y));
        }
        // ClipMask: 焼いた覆いの外を削る。式で書けない形はこれ。
        case 12u: {
            // params.zw は幅と高さの逆数。画素ごとの割り算を省いてある。
            let uv = (world - block.params.xy) * block.params.zw;

            // 覆いの外は必ず削る。`clip::cover` が縁を 1 升ぶん内側に逃がしている
            // ので、いまは端を引いても 0 が返る。手で組んだ山が範囲とずれていても
            // 漏らさないための備えで、外から見て違いは出ない。
            let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));

            // ミップは要らないので段を指定して引く。
            // 微分を使わないぶん、一様でない制御フローの中でも安全。
            let sampled = textureSampleLevel(
                clip_masks,
                clip_sampler,
                clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)),
                i32(block.color_a.x),
                0.0,
            ).r;

            let coverage = select(0.0, sampled, inside);

            return clip_apply(color, clip_flip(coverage, block.color_a.y));
        }
        // GradientStops（線形）: 一方向に流れる。
        case 14u: {
            let direction = vec2<f32>(cos(block.params.x), sin(block.params.x));
            let ratio = dot(uv - 0.5, direction) + 0.5;

            return color * gradient_color(at, u32(block.color_a.x), block.color_a.y, ratio);
        }
        // GradientStops（放射）: 中心から広がる。
        case 15u: {
            let radii = max(abs(block.params.zw), vec2<f32>(1e-6));
            let ratio = length((uv - block.params.xy) / radii);

            return color * gradient_color(at, u32(block.color_a.x), block.color_a.y, ratio);
        }
        // GradientStops（角度）: 中心のまわりを一周する。
        case 16u: {
            let offset = uv - block.params.xy;
            let angle = atan2(offset.y, offset.x) - block.params.z;
            let ratio = fract(angle / TAU + 1.0);

            return color * gradient_color(at, u32(block.color_a.x), block.color_a.y, ratio);
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
    /// ワールド座標。頂点を置いたのと同じ空間。クリップが使う。
    @location(7) world_position: vec2<f32>,
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

    // 色は sRGB で受け取り、ここで線形に直す。以降の掛け算・混ぜ・合成は
    // 全部線形で行う。絵も `Rgba8UnormSrgb` なので、読んだ時点で線形。
    // 不透明度は光の量ではなく覆う割合なので、変換しない。
    let vertex_color = vec4<f32>(
        srgb_to_linear(vec3<f32>(vertex.r, vertex.g, vertex.b)),
        vertex.a,
    );
    let tint = vec4<f32>(srgb_to_linear(instance.tint.rgb), instance.tint.a);

    output.color = tint * vertex_color;

    // テッセレータは UV を書かないので、外接矩形から作る。
    // 四角なら 0..1 がちょうど四隅に来るので、絵が素直に収まる。
    let object = objects[instance.object_index];
    let uv = (vec2<f32>(vertex.x, vertex.y) - object.bounds.xy) * object.bounds.zw;

    // カメラだけ巻き戻してワールドに戻す。
    // 頂点に掛かったのは camera * object * instance なので、
    // カメラを取り除けば object * instance * 頂点、つまりワールド座標になる。
    let world = object.camera_inverse * output.clip_position;
    output.world_position = world.xy / world.w;

    output.uv = uv;
    // 切り出し範囲に写す。コマ送りはここを動かすだけで済む。
    output.sprite_uv = uv * object.uv_rect.zw + object.uv_rect.xy;

    output.color_base = instance.color_base;
    output.color_count = instance.color_count;
    output.texture_layer = object.texture_layer;
    output.has_sprite = object.has_sprite;
    return output;
}

const TAU: f32 = 6.2831855;

/// sRGB の値を線形の光に直す。
///
/// 描き先が sRGB なので、出すときは GPU が逆向きに直します。
/// ここで直しておかないと、`0.5` が真ん中の灰色ではなく
/// かなり明るい灰色（sRGB 188）で出ます。
///
/// 折れ線ではなく規格どおりの式を使うのは、暗いほうの取りこぼしを
/// 避けるためです。`0.0` と `1.0` は変換しても動きません。
fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let cutoff = color <= vec3<f32>(0.04045);
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));

    return select(high, low, cutoff);
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

    // 画素 1 つぶんがワールドで何単位か。縁をなめらかに落とすのに使う。
    //
    // 微分は一様な制御フローの中でしか取れないので、山の輪に入る前にここで 1 度だけ求める。
    // クリップの距離場はどれもワールド単位の真の距離なので、これがそのまま縁の幅になる。
    let world_per_pixel = max(
        max(abs(dpdx(input.world_position.x)), abs(dpdy(input.world_position.y))),
        1e-6,
    );

    // Color 段。積まれた順に掛ける。
    //
    // 1 山が 2 つ以上を占めることがある（グラデーションは色を収める続きを従える）。
    // 山が自分で「いくつ使ったか」を言うので、そのぶんまとめて進む。
    var step = 0u;

    loop {
        if (step >= input.color_count) {
            break;
        }

        let at = input.color_base + step;

        color = apply_color_block(
            color,
            at,
            input.uv,
            config.time,
            input.world_position,
            world_per_pixel,
        );

        step = step + block_span(blocks[at]);
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
    /// 焼いたクリップの覆い。使っていなくても 1 層だけ持っておく。
    clip_masks: ClipMaskSheet,
    /// 使われている層と、それが覆うワールドの範囲。
    clip_mask_bounds: FxHashMap<u32, [f32; 4]>,

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
    /// 1 回のパスで描けるまとまり。描く順に並ぶ。
    ///
    /// 並べ替えたあと、**掛けるエフェクトが変わるところで切って**作る。
    /// 同じものが続くあいだは 1 回で描けるので、パスは必要な数しか出ない。
    passes: Vec<PassGroup>,
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
                    // クリップの覆い。図形がひとつも使っていなくても繋いでおく。
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 7,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        // 絵を指定していない図形もここを読む。真っ白なので掛けても変わらない。
        let sprite_sheet = Arc::new(SpriteSheet::white(device, queue));

        // 覆いを 1 枚も使わなくても、束ねる先は要る。最初は 1 層だけ。
        let clip_masks = ClipMaskSheet::new(device, descriptor.clip_mask_resolution, 1);

        let render_bind_group = build_render_bind_group(
            device,
            RenderBindings {
                layout: &render_bind_group_layout,
                pool: pool_heap.buffer(),
                blocks: block_heap.buffer(),
                config: &config_buffer,
                objects: object_heap.buffer(),
                sprite_sheet: &sprite_sheet,
                clip_masks: &clip_masks,
            },
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
            clip_masks,
            clip_mask_bounds: FxHashMap::default(),
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
            passes: Vec::new(),
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
    /// # use gueiz_2d::draw_manager::DrawManager;
    /// # use gueiz_2d::object;
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
    /// # use gueiz_2d::draw_manager::DrawManager;
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
    /// #     draw_manager: &mut gueiz_2d::draw_manager::DrawManager,
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
            RenderBindings {
                layout: &self.render_bind_group_layout,
                pool: self.pool_heap.buffer(),
                blocks: self.block_heap.buffer(),
                config: &self.config_buffer,
                objects: self.object_heap.buffer(),
                sprite_sheet: &sprite_sheet,
                clip_masks: &self.clip_masks,
            },
        );

        self.sprite_sheet = sprite_sheet;
    }

    /// 図形を焼いてクリップの覆いにする。返った [`ClipMask`] の `block()` を
    /// 別の図形に積むと、その形の外が削れます。
    ///
    /// 焼くのは**そのときの形だけ**です。図形を組み替えたら
    /// [`DrawManager::update_clip_mask`] で焼き直してください。
    /// 位置・回転・拡大（[`Object::translate`] など）や複製は見ません。
    /// 頂点を置いた座標そのままで覆います。
    ///
    /// ```no_run
    /// # fn run(
    /// #     device: &gueiz_2d::wgpu::Device,
    /// #     queue: &gueiz_2d::wgpu::Queue,
    /// #     draw_manager: &mut gueiz_2d::draw_manager::DrawManager,
    /// #     star: &gueiz_2d::object::Object,
    /// #     photo: &mut gueiz_2d::object::Object,
    /// # ) {
    /// use gueiz_2d::clip::ClipMaskKind;
    ///
    /// let mask = draw_manager.add_clip_mask(device, queue, star, ClipMaskKind::Coverage);
    /// photo.effect(mask.block());
    /// # }
    /// ```
    pub fn add_clip_mask(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shape: &Object,
        kind: ClipMaskKind,
    ) -> ClipMask {
        let layer = match self.clip_masks.take() {
            Some(layer) => layer,
            None => {
                self.clip_masks.grow(device, queue);
                self.rebuild_render_bind_group(device);

                // 増やした直後なので必ず空きがある。
                self.clip_masks.take().expect("層を増やしたのに空きがない")
            }
        };

        let mask = self.bake(queue, layer, shape, kind);

        self.clip_mask_bounds.insert(layer, mask.bounds());
        mask
    }

    /// 焼き直す。層はそのままなので、積んである山はそのまま使えます。
    ///
    /// **覆う範囲は変わります。** 形が動いたなら、積み直すか
    /// [`ClipMask::block`] を取り直してください。
    pub fn update_clip_mask(
        &mut self,
        queue: &wgpu::Queue,
        mask: ClipMask,
        shape: &Object,
    ) -> ClipMask {
        let updated = self.bake(queue, mask.layer(), shape, mask.kind());

        self.clip_mask_bounds.insert(mask.layer(), updated.bounds());
        updated
    }

    /// 覆いを捨てて層を空ける。空いた層は次の [`DrawManager::add_clip_mask`] が使います。
    ///
    /// **捨てた覆いの山を積んだままにしないこと。** 層が使い回されると
    /// 別の形で削られます。
    pub fn remove_clip_mask(&mut self, queue: &wgpu::Queue, mask: ClipMask) {
        let blank = vec![0u8; (self.clip_masks.resolution * self.clip_masks.resolution) as usize];

        self.clip_masks.write(queue, mask.layer(), &blank);
        self.clip_masks.give_back(mask.layer());
        self.clip_mask_bounds.remove(&mask.layer());
    }

    /// いま使われている覆いの枚数。
    pub fn clip_mask_count(&self) -> usize {
        self.clip_mask_bounds.len()
    }

    /// 覆い 1 枚の辺の長さ（画素）。
    pub fn clip_mask_resolution(&self) -> u32 {
        self.clip_masks.resolution
    }

    /// 用意してある層の数。足りなくなると倍に増える。
    pub fn clip_mask_capacity(&self) -> u32 {
        self.clip_masks.layers
    }

    /// 形を焼いて層に載せる。
    fn bake(
        &self,
        queue: &wgpu::Queue,
        layer: u32,
        shape: &Object,
        kind: ClipMaskKind,
    ) -> ClipMask {
        let resolution = self.clip_masks.resolution;
        let triangles = shape.triangles();

        match kind {
            ClipMaskKind::Coverage => {
                // 縁が升目の境目に乗ると薄くなるので、1 画素ぶん外に逃がす。
                let bounds = clip::cover(triangles, resolution, 1.0, false);

                self.clip_masks
                    .write(queue, layer, &clip::rasterise(triangles, bounds, resolution));

                ClipMask::new(layer, bounds, kind, 0.0)
            }

            ClipMaskKind::Distance => {
                // 距離が頭打ちになる手前まで余白を取り、縦横の縮尺を揃える。
                let bounds = clip::cover(triangles, resolution, clip::SPREAD, true);
                let (pixels, spread) = clip::distance_field(triangles, bounds, resolution);

                self.clip_masks.write(queue, layer, &pixels);

                ClipMask::new(layer, bounds, kind, spread)
            }
        }
    }

    fn rebuild_render_bind_group(&mut self, device: &wgpu::Device) {
        self.render_bind_group = build_render_bind_group(
            device,
            RenderBindings {
                layout: &self.render_bind_group_layout,
                pool: self.pool_heap.buffer(),
                blocks: self.block_heap.buffer(),
                config: &self.config_buffer,
                objects: self.object_heap.buffer(),
                sprite_sheet: &self.sprite_sheet,
                clip_masks: &self.clip_masks,
            },
        );
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
    /// # fn run(device: &gueiz_2d::wgpu::Device, draw_manager: &mut gueiz_2d::draw_manager::DrawManager)
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
        self.draw_indirect_range(render_pass, 0..self.draw_count);
    }

    /// 1 回のパスで描けるまとまりを、描く順に。
    ///
    /// **掛けるエフェクトが変わるところで切ってあります。** まとまりごとに
    /// 別の絵へ描いて、その鎖を通してから重ねると、図形ごとのぼかしになります。
    ///
    /// ```no_run
    /// # use gueiz_2d::draw_manager::DrawManager;
    /// # fn run(draw_manager: &DrawManager) {
    /// for pass in draw_manager.passes() {
    ///     // pass.range を描いて、pass.chain を通して重ねる。
    ///     let _ = (&pass.range, pass.chain);
    /// }
    /// # }
    /// ```
    pub fn passes(&self) -> impl Iterator<Item = DrawPass<'_>> {
        self.passes.iter().map(|group| DrawPass {
            layer: group.layer,
            range: group.range.clone(),
            chain: &group.chain,
        })
    }

    /// インダイレクトの一部だけ描く。[`DrawManager::passes`] の範囲を渡す。
    pub fn draw_range(&self, render_pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        self.draw_indirect_range(render_pass, range);
    }

    fn draw_indirect_range(&self, render_pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        // 描く数が 0 のときにインダイレクトを呼ぶと、実装によっては怒られる。
        if self.draw_count == 0 || range.start >= range.end {
            return;
        }

        let stride = size_of::<wgpu::util::DrawIndirectArgs>() as u64;

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, &self.render_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.instance_output_heap.slice(&self.instance_output));
        render_pass.multi_draw_indirect(
            self.indirect_heap.buffer(),
            self.indirect.offset() + range.start as u64 * stride,
            range.end - range.start,
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
            //
            // 数えるのは**積む数**であって山の数ではない。グラデーションは
            // 色を収める続きを従えるので、1 山で 2 つ以上になることがある。
            let room = self.max_effect_blocks.saturating_sub(base) as usize;

            let transform_count = fit(transform_blocks, room, &mut self.block_scratch);
            let color_count = fit(
                color_blocks,
                room - transform_count.written,
                &mut self.block_scratch,
            );

            if transform_count.dropped || color_count.dropped {
                log::warn!(
                    "the effect block limit ({}) is reached; some blocks on '{}' will not run",
                    self.max_effect_blocks,
                    object.name(),
                );
            }

            self.effect_ranges[index] = EffectRange {
                base,
                transform_count: transform_count.written as u32,
                color_count: color_count.written as u32,
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
        // **層が先、その中で z。** 層ごとにまとめて描けるよう、
        // 同じ層が続きになるように並べる。
        // 安定ソートなので、どちらも同じなら登録順のまま。
        draw_order.sort_by(|&a, &b| {
            let (left, right) = (&self.objects[a], &self.objects[b]);

            left.in_layer()
                .cmp(&right.in_layer())
                .then_with(|| left.depth().total_cmp(&right.depth()))
        });

        // パスのまとまりを作る。`passes` がここを見る。
        //
        // 層が変わるか、掛けるエフェクトが変わったところで切る。
        // 同じものが続くあいだは 1 回で描けるので、無駄なパスが出ない。
        self.passes.clear();

        for (position, &index) in draw_order.iter().enumerate() {
            let object = &self.objects[index];
            let (layer, chain) = (object.in_layer(), object.post_chain());
            let position = position as u32;

            match self.passes.last_mut() {
                Some(last) if last.layer == layer && &last.chain == chain => {
                    last.range.end = position + 1;
                }

                _ => self.passes.push(PassGroup {
                    layer,
                    range: position..position + 1,
                    chain: chain.clone(),
                }),
            }
        }

        for &index in &draw_order {
            let pool_range = self.shape_pool.range(index);
            let metrics = self.shape_metrics[index];
            let instance_range = self.instance_uploader.range(index);
            let effect_range = self.effect_ranges[index];
            let object = &self.objects[index];

            self.object_scratch.push(ObjectRaw {
                // カメラと図形の変換まではここで畳む。インスタンスは GPU 側。
                transform: object.view_transform().to_columns(),
                // クリップ位置からワールドに戻す道。潰れたカメラなら素通しにする。
                camera_inverse: object
                    .view_camera()
                    .view_projection()
                    .inverse_2d()
                    .unwrap_or_default()
                    .to_columns(),
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
/// 描画側のバインドグループに繋ぐもの。
struct RenderBindings<'a> {
    layout: &'a wgpu::BindGroupLayout,
    pool: &'a wgpu::Buffer,
    blocks: &'a wgpu::Buffer,
    config: &'a wgpu::Buffer,
    objects: &'a wgpu::Buffer,
    sprite_sheet: &'a SpriteSheet,
    clip_masks: &'a ClipMaskSheet,
}

fn build_render_bind_group(
    device: &wgpu::Device,
    bindings: RenderBindings<'_>,
) -> wgpu::BindGroup {
    let RenderBindings {
        layout,
        pool,
        blocks,
        config,
        objects,
        sprite_sheet,
        clip_masks,
    } = bindings;

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
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&clip_masks.view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Sampler(&clip_masks.sampler),
            },
        ],
    })
}

/// 焼いたクリップの覆いを収めるテクスチャ配列。1 枚が 1 層。
///
/// 層が足りなくなったら倍に増やし、古い中身をそのまま写します。
/// どれだけ増えても 1 枚のテクスチャなので、**ドローの数は増えません**。
struct ClipMaskSheet {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    /// 1 層の辺の長さ（画素）。
    resolution: u32,
    /// 用意してある層の数。
    layers: u32,
    /// 次に配る層。
    next: u32,
    /// 返された層。先に使い回す。
    free: Vec<u32>,
}

impl ClipMaskSheet {
    fn new(device: &wgpu::Device, resolution: u32, layers: u32) -> Self {
        let resolution = resolution.max(1);
        let layers = layers.max(1);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gueiz clip masks"),
            size: wgpu::Extent3d {
                width: resolution,
                height: resolution,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // 覆う割合しか要らないので 1 チャンネル。
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("gueiz clip mask sampler"),
                // 縁の外に漏れないよう、端は 0 で止める。
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            texture,
            resolution,
            layers,
            next: 0,
            free: Vec::new(),
        }
    }

    /// 層を 1 つ確保する。空きが無ければ `None`。
    fn take(&mut self) -> Option<u32> {
        if let Some(layer) = self.free.pop() {
            return Some(layer);
        }

        if self.next < self.layers {
            let layer = self.next;
            self.next += 1;
            return Some(layer);
        }

        None
    }

    /// 層を返す。中身は消してから返す側で消す。
    fn give_back(&mut self, layer: u32) {
        if layer < self.next && !self.free.contains(&layer) {
            self.free.push(layer);
        }
    }

    /// 層を倍に増やし、古い中身を写す。
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let grown = Self::new(device, self.resolution, self.layers * 2);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gueiz clip mask grow"),
        });

        encoder.copy_texture_to_texture(
            self.texture.as_image_copy(),
            grown.texture.as_image_copy(),
            wgpu::Extent3d {
                width: self.resolution,
                height: self.resolution,
                depth_or_array_layers: self.layers,
            },
        );

        queue.submit(Some(encoder.finish()));

        let (next, free) = (self.next, std::mem::take(&mut self.free));
        *self = grown;
        self.next = next;
        self.free = free;
    }

    /// 1 層を書き換える。`pixels` は `resolution * resolution` バイト。
    fn write(&self, queue: &wgpu::Queue, layer: u32, pixels: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.resolution),
                rows_per_image: Some(self.resolution),
            },
            wgpu::Extent3d {
                width: self.resolution,
                height: self.resolution,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// 1 回のパスで描くまとまり。持ち主は [`DrawManager`]。
struct PassGroup {
    layer: u32,
    range: Range<u32>,
    chain: PostChain,
}

/// [`DrawManager::passes`] が返す、1 回ぶんのパス。
pub struct DrawPass<'a> {
    /// どの層か。同じ層でも、掛けるものが違えば分かれる。
    pub layer: u32,
    /// [`DrawManager::draw_range`] に渡す範囲。
    pub range: Range<u32>,
    /// このまとまりに掛けるもの。空なら素のまま重ねるだけ。
    pub chain: &'a PostChain,
}

/// 積めるだけ積む。
struct Fitted {
    /// 実際に積んだ数。山の数ではなく、GPU に載る数。
    written: usize,
    /// 入り切らずに捨てたものがあるか。
    dropped: bool,
}

/// 山を、入るぶんだけ積む。
///
/// 1 山が 2 つ以上になることがあるので、**山の途中で切らない**。
/// 半端に積むと、色を収める続きだけが残って化ける。
fn fit(blocks: &[Block], room: usize, out: &mut Vec<BlockRaw>) -> Fitted {
    let mut written = 0;

    for (index, block) in blocks.iter().enumerate() {
        let needed = block.raw_count();

        if written + needed > room {
            return Fitted {
                written,
                dropped: index < blocks.len(),
            };
        }

        block.write_raw(out);
        written += needed;
    }

    Fitted {
        written,
        dropped: false,
    }
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
