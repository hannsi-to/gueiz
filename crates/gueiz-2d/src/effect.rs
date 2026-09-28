//! エフェクトの段とブロック。VFX Graph と同じ組み立て方をする。
//!
//! # 段（Context）は固定、段の中の順番は自由
//!
//! エフェクトが効く場所は 3 つあり、**この順番はハードウェアが決めていて動かせません**。
//!
//! ```text
//! Shape（CPU・テッセレーション）→ Transform（コンピュート）→ Color（フラグメント）
//!        形を作る                      位置を動かす              色を変える
//! ```
//!
//! つまり `[Wobble, Gradient]` と `[Gradient, Wobble]` を積んでも、
//! Wobble のほうが必ず先に効きます。段が違うからです。
//!
//! そこで [`Block`] は自分がどの段で効くかを持ち（[`Block::stage`]）、
//! [`EffectStack`] は段ごとに別の列として持ちます。**順番が意味を持つのは
//! 同じ段の中だけ**で、API もそう見えるようにしてあります。
//!
//! VFX Graph の Context（Initialize / Update / Output）と Block の関係と同じです。
//!
//! # 使い方
//!
//! ```
//! # use gueiz_2d::effect::{Block, EffectStack, EffectStage};
//! let mut effects = EffectStack::new();
//!
//! // 積んだ順に効く。段は Block が自分で知っているので、指定は要らない。
//! effects.push(Block::Gradient {
//!     from: [1.0, 0.4, 0.2, 1.0],
//!     to: [0.2, 0.4, 1.0, 1.0],
//!     angle: 0.0,
//! });
//! effects.push(Block::Dissolve { threshold: 0.3, edge: 0.1, edge_color: [1.0; 4] });
//! effects.push(Block::Spin { speed: 1.5 });
//!
//! // Spin だけ別の段に入る。
//! assert_eq!(effects.blocks(EffectStage::Color).len(), 2);
//! assert_eq!(effects.blocks(EffectStage::Transform).len(), 1);
//! ```

/// エフェクトが効く場所。VFX Graph の Context にあたる。
///
/// 並び順がそのまま実行順で、**入れ替えられません**。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq, Hash)]
#[derive(Debug)]
pub enum EffectStage {
    /// 形を作る。CPU 側、三角形に開くときに効く。
    ///
    /// 描画時のコストはほぼゼロ（三角形が少し増えるだけ）。
    Shape,
    /// 位置・回転・大きさを動かす。コンピュートパスで効く。
    ///
    /// 時間の関数として書けるので、CPU からの転送は増えない。
    Transform,
    /// 色を変える。フラグメントシェーダで効く。
    ///
    /// すでに塗る画素の上でやるので、追加の画素はゼロ。
    Color,
}

impl EffectStage {
    /// 実行順に並べた全段。
    pub const ALL: [Self; 3] = [Self::Shape, Self::Transform, Self::Color];
}

/// エフェクトひと山。VFX Graph の Block にあたる。
///
/// どの段で効くかは [`Block::stage`] が持っているので、積むときに段の指定は要りません。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum Block {
    // --- Shape 段 ---
    /// 輪郭に線を足す。塗りの上に重ねて描かれる。
    Outline { width: f32, color: [f32; 4] },

    // --- Transform 段 ---
    /// 回し続ける。`speed` はラジアン毎秒。
    Spin { speed: f32 },
    /// 上下に揺らす。
    Wobble { amplitude: f32, frequency: f32 },
    /// 元の位置のまわりを回る。
    Orbit { radius: f32, speed: f32 },
    /// 大きさを脈打たせる。`amount` は増減の幅（0.2 なら ±20%）。
    Pulse { amount: f32, speed: f32 },

    // --- Color 段 ---
    /// 色を掛ける。
    Tint { color: [f32; 4] },
    /// 図形の中でグラデーションを掛ける。`angle` はラジアン。
    Gradient {
        from: [f32; 4],
        to: [f32; 4],
        angle: f32,
    },
    /// ノイズで溶かす。`threshold` が 0 で無傷、1 で消える。
    /// 溶け際は `edge` の幅だけ `edge_color` で光る。
    Dissolve {
        threshold: f32,
        edge: f32,
        edge_color: [f32; 4],
    },
    /// 時間で明滅させる。
    Flicker { amount: f32, speed: f32 },
    /// 色を何段でも置けるグラデーション。線形・放射・角度。
    ///
    /// 2 色の線形でよければ [`Block::Gradient`] のほうが短く、
    /// GPU に積む量も 1 つで済みます。
    GradientStops { gradient: Gradient },

    // --- Color 段（クリップ） ---
    //
    // 座標はどれも**ワールド**。頂点を置いたのと同じ空間なので、
    // 図形が回っても動いても、クリップは置いた場所に留まる。
    // カメラが動けば一緒に動く。
    //
    // 削るのは覆う割合（不透明度）なので、縁は 1 画素ぶんなめらかに落ちる。
    //
    // `invert` を立てると内と外が入れ替わり、**形の中が削れて外が残ります**。
    // 穴を開けたいときに使ってください。
    /// 矩形の外を削る。`radius` で角を丸められる。
    ClipRect {
        min: [f32; 2],
        max: [f32; 2],
        /// 角の丸み。0 で角のまま。半分の幅か高さを超えると、そこで頭打ちになる。
        radius: f32,
        /// 縁をぼかす幅。0 でもギザギザにはならない。
        softness: f32,
        /// 立てると矩形の中が削れる。
        invert: bool,
    },
    /// 楕円の外を削る。
    ClipEllipse {
        center: [f32; 2],
        radius_x: f32,
        radius_y: f32,
        softness: f32,
        /// 立てると楕円の中が削れる。
        invert: bool,
    },
    /// 直線の片側を削る。`normal` が向いている先が削られる。
    ///
    /// **積み重ねると凸多角形になる。** 山は順に効いてそれぞれが覆いを掛けるので、
    /// 3 つ積めば三角形、4 つで四角形にくり抜けます。
    ///
    /// 直線は `dot(normal, 点) = distance` の位置にあります。
    /// `normal` の長さは見ません（中で 1 に直します）。
    ClipHalfPlane {
        normal: [f32; 2],
        distance: f32,
        softness: f32,
        /// 立てると残る側が入れ替わる。
        invert: bool,
    },
    /// 焼いた覆いの外を削る。**式で書けない形はこれを使う。**
    ///
    /// 自分で組み立てず、[`crate::draw_manager::DrawManager::add_clip_mask`] が
    /// 返す [`crate::clip::ClipMask`] の `block()` から作ってください。
    /// 覆いの層と範囲が噛み合っていないと、何も残らないか全部残ります。
    ///
    /// 縁のなめらかさは覆いの細かさで決まるので、`softness` はありません。
    ClipMask {
        /// 覆いが覆うワールドの左下。
        min: [f32; 2],
        /// 覆いが覆うワールドの大きさ。**この外は必ず削れる。**
        size: [f32; 2],
        /// テクスチャ配列の何層目か。
        layer: u32,
        /// 立てると覆いの中が削れる。
        invert: bool,
    },
    /// 距離を焼いた覆いの外を削る。**拡大しても縁が保てる。**
    ///
    /// 焼いてあるのは覆う割合ではなく境目までの距離なので、画素のあいだを
    /// 混ぜてもほぼ距離のままです。そのぶん `softness` でぼかせます。
    ///
    /// これも自分で組み立てず、[`crate::clip::ClipMask::block_with`] から作ってください。
    ClipDistanceMask {
        /// 覆いが覆うワールドの左下。
        min: [f32; 2],
        /// 覆いが覆うワールドの大きさ。**この外は必ず削れる。**
        size: [f32; 2],
        /// テクスチャ配列の何層目か。
        layer: u32,
        /// 距離が頭打ちになるまでのワールドの長さ。焼いたときに決まる。
        spread: f32,
        /// 縁をぼかす幅。`spread` を超えると頭打ちになる。
        softness: f32,
        /// 立てると覆いの中が削れる。
        invert: bool,
    },

    // --- 自前の山 ---
    /// 自分で書いた WGSL を走らせる。
    ///
    /// `kind` は [`CUSTOM_KIND_BASE`] 以上で、
    /// [`crate::draw_manager::DrawManager::set_custom_blocks`] に
    /// 同じ番号で本体を渡しておく。`params` と `color` はシェーダに
    /// `block.params` / `block.color_a` として届く。
    Custom {
        stage: EffectStage,
        kind: u32,
        params: [f32; 4],
        color: [f32; 4],
    },
}

/// 自前の山に使える番号の下限。これより下は用意された山が使う。
pub const CUSTOM_KIND_BASE: u32 = 100;

/// 自前の山の中身。[`Block::Custom`] と `kind` で結びつく。
///
/// `body` は WGSL の断片で、そのまま `switch` の腕に埋め込まれる。
/// 段によって使えるものと返し方が違う。
///
/// | 段 | 使えるもの | 返し方 |
/// |---|---|---|
/// | [`EffectStage::Transform`] | `instance`（`ptr`）, `block`, `time`, `seed` | 返さない。`instance` を書き換える |
/// | [`EffectStage::Color`] | `color`, `block`, `uv`, `time` | `return` で `vec4<f32>` |
///
/// [`EffectStage::Shape`] は CPU 側なので、自前の山は置けない。
///
/// ```no_run
/// # use gueiz_2d::effect::{CustomBlock, EffectStage, CUSTOM_KIND_BASE};
/// let stripes = CustomBlock {
///     kind: CUSTOM_KIND_BASE,
///     stage: EffectStage::Color,
///     body: String::from(r#"
///         let stripe = step(0.5, fract(uv.x * block.params.x));
///         return color * mix(vec4<f32>(1.0), block.color_a, stripe);
///     "#),
/// };
/// ```
#[derive(Clone)]
#[derive(Debug)]
pub struct CustomBlock {
    pub kind: u32,
    pub stage: EffectStage,
    pub body: String,
}

impl Block {
    /// この山がどの段で効くか。
    pub fn stage(&self) -> EffectStage {
        match self {
            Self::Outline { .. } => EffectStage::Shape,

            Self::Spin { .. } | Self::Wobble { .. } | Self::Orbit { .. } | Self::Pulse { .. } => {
                EffectStage::Transform
            }

            Self::Tint { .. }
            | Self::Gradient { .. }
            | Self::Dissolve { .. }
            | Self::Flicker { .. }
            | Self::GradientStops { .. }
            | Self::ClipRect { .. }
            | Self::ClipEllipse { .. }
            | Self::ClipHalfPlane { .. }
            | Self::ClipMask { .. }
            | Self::ClipDistanceMask { .. } => EffectStage::Color,

            Self::Custom { stage, .. } => *stage,
        }
    }

    /// シェーダ側の `switch` に使う番号。0 は「何もしない」に予約。
    pub(crate) fn kind(&self) -> u32 {
        match self {
            Self::Outline { .. } => 0, // CPU 側で処理するので GPU には載らない
            Self::Spin { .. } => 1,
            Self::Wobble { .. } => 2,
            Self::Orbit { .. } => 3,
            Self::Pulse { .. } => 4,
            Self::Tint { .. } => 5,
            Self::Gradient { .. } => 6,
            Self::Dissolve { .. } => 7,
            Self::Flicker { .. } => 8,
            Self::GradientStops { gradient } => gradient.kind(),
            Self::ClipRect { .. } => 9,
            Self::ClipEllipse { .. } => 10,
            Self::ClipHalfPlane { .. } => 11,
            Self::ClipMask { .. } => 12,
            Self::ClipDistanceMask { .. } => 13,
            Self::Custom { kind, .. } => *kind,
        }
    }

    /// GPU に積む数。ほとんどの山は 1 つだが、
    /// [`Block::GradientStops`] は色を収める続きを従える。
    pub(crate) fn raw_count(&self) -> usize {
        match self {
            Self::GradientStops { gradient } => gradient.raw_count(),
            _ => 1,
        }
    }

    /// GPU に積む。[`Block::raw_count`] と同じ数だけ書き足す。
    pub(crate) fn write_raw(&self, out: &mut Vec<BlockRaw>) {
        let Self::GradientStops { gradient } = self else {
            out.push(self.to_raw());
            return;
        };

        out.push(gradient.head());
        out.extend(gradient.tail());
    }

    /// GPU に載せる形。`params` の意味は山ごとに違う。
    ///
    /// **1 つに収まる山だけ。** 続きを従えるものは [`Block::write_raw`] を使う。
    ///
    /// **色は [`srgb_to_linear`] を通し、寸法は通しません。** シェーダの中は
    /// すべて線形なので、色は混ぜる前にここで 1 度だけ直しておきます。
    /// クリップの山は `color_a` に色ではなく寸法を積むので、素通しです。
    pub(crate) fn to_raw(self) -> BlockRaw {
        let (params, color_a, color_b) = match self {
            Self::Outline { .. } => ([0.0; 4], [0.0; 4], [0.0; 4]),

            Self::Spin { speed } => ([speed, 0.0, 0.0, 0.0], [0.0; 4], [0.0; 4]),

            Self::Wobble {
                amplitude,
                frequency,
            } => ([amplitude, frequency, 0.0, 0.0], [0.0; 4], [0.0; 4]),

            Self::Orbit { radius, speed } => ([radius, speed, 0.0, 0.0], [0.0; 4], [0.0; 4]),

            Self::Pulse { amount, speed } => ([amount, speed, 0.0, 0.0], [0.0; 4], [0.0; 4]),

            Self::Tint { color } => ([0.0; 4], srgb_to_linear(color), [0.0; 4]),

            Self::Gradient { from, to, angle } => (
                [angle, 0.0, 0.0, 0.0],
                srgb_to_linear(from),
                srgb_to_linear(to),
            ),

            Self::Dissolve {
                threshold,
                edge,
                edge_color,
            } => (
                [threshold, edge, 0.0, 0.0],
                srgb_to_linear(edge_color),
                [0.0; 4],
            ),

            Self::Flicker { amount, speed } => ([amount, speed, 0.0, 0.0], [0.0; 4], [0.0; 4]),

            // クリップは寸法なので、sRGB を通さない。
            Self::ClipRect {
                min,
                max,
                radius,
                softness,
                invert,
            } => (
                [min[0], min[1], max[0], max[1]],
                [radius, softness, flag(invert), 0.0],
                [0.0; 4],
            ),

            Self::ClipEllipse {
                center,
                radius_x,
                radius_y,
                softness,
                invert,
            } => (
                [center[0], center[1], radius_x, radius_y],
                [softness, flag(invert), 0.0, 0.0],
                [0.0; 4],
            ),

            Self::ClipHalfPlane {
                normal,
                distance,
                softness,
                invert,
            } => (
                [normal[0], normal[1], distance, softness],
                [flag(invert), 0.0, 0.0, 0.0],
                [0.0; 4],
            ),

            Self::ClipDistanceMask {
                min,
                size,
                layer,
                spread,
                softness,
                invert,
            } => (
                [
                    min[0],
                    min[1],
                    if size[0] != 0.0 { 1.0 / size[0] } else { 0.0 },
                    if size[1] != 0.0 { 1.0 / size[1] } else { 0.0 },
                ],
                [layer as f32, flag(invert), spread, softness],
                [0.0; 4],
            ),

            Self::ClipMask {
                min,
                size,
                layer,
                invert,
            } => (
                // 幅と高さは逆数で渡す。画素ごとの割り算を省くため。
                [
                    min[0],
                    min[1],
                    if size[0] != 0.0 { 1.0 / size[0] } else { 0.0 },
                    if size[1] != 0.0 { 1.0 / size[1] } else { 0.0 },
                ],
                [layer as f32, flag(invert), 0.0, 0.0],
                [0.0; 4],
            ),

            Self::Custom { params, color, .. } => (params, srgb_to_linear(color), [0.0; 4]),

            // 続きを従えるので、1 つには収まらない。
            Self::GradientStops { gradient } => return gradient.head(),
        };

        BlockRaw {
            params,
            color_a,
            color_b,
            kind: self.kind(),
            _padding: [0; 3],
        }
    }
}

/// 真偽を GPU に送る形に。WGSL 側は `!= 0.0` で見る。
fn flag(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

/// sRGB の色を線形の光に直す。**不透明度は変換しない。**
///
/// 覆う割合であって光の量ではないので、ガンマを掛けると半透明がずれます。
///
/// シェーダ側（`srgb_to_linear`）と同じ式です。こちらは山を積むときに
/// 1 度だけ通るので、画素ごとに計算するより安く済みます。
pub(crate) fn srgb_to_linear(color: [f32; 4]) -> [f32; 4] {
    let channel = |value: f32| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };

    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        color[3],
    ]
}

/// GPU に載る 1 山。WGSL 側と並びを揃える。
///
/// `vec4<f32>` はアラインメント 16 なので、末尾の詰め物を明示している。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BlockRaw {
    pub params: [f32; 4],
    pub color_a: [f32; 4],
    pub color_b: [f32; 4],
    pub kind: u32,
    pub _padding: [u32; 3],
}

/// 図形 1 つぶんのエフェクト。段ごとに別の列で持つ。
///
/// 段の中では [`EffectStack::push`] した順に効きます。
#[derive(Clone)]
#[derive(Default)]
#[derive(Debug)]
pub struct EffectStack {
    shape: Vec<Block>,
    transform: Vec<Block>,
    color: Vec<Block>,
}

impl EffectStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// 山を積む。段は [`Block::stage`] で決まるので指定は要らない。
    ///
    /// 同じ段の中では、積んだ順に効く。
    pub fn push(&mut self, block: Block) -> &mut Self {
        self.list_mut(block.stage()).push(block);
        self
    }

    /// その段の山を、効く順に。
    pub fn blocks(&self, stage: EffectStage) -> &[Block] {
        match stage {
            EffectStage::Shape => &self.shape,
            EffectStage::Transform => &self.transform,
            EffectStage::Color => &self.color,
        }
    }

    /// その段の `index` 番目を抜く。
    pub fn remove(&mut self, stage: EffectStage, index: usize) -> Option<Block> {
        let list = self.list_mut(stage);

        if index >= list.len() {
            return None;
        }

        Some(list.remove(index))
    }

    /// その段の中で順番を入れ替える。`from` を抜いて `to` に差し込む。
    pub fn move_block(&mut self, stage: EffectStage, from: usize, to: usize) -> bool {
        let list = self.list_mut(stage);

        if from >= list.len() || to >= list.len() {
            return false;
        }

        let block = list.remove(from);
        list.insert(to, block);
        true
    }

    /// その段を空にする。
    pub fn clear(&mut self, stage: EffectStage) -> &mut Self {
        self.list_mut(stage).clear();
        self
    }

    /// 全部で何山あるか。
    pub fn len(&self) -> usize {
        self.shape.len() + self.transform.len() + self.color.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn list_mut(&mut self, stage: EffectStage) -> &mut Vec<Block> {
        match stage {
            EffectStage::Shape => &mut self.shape,
            EffectStage::Transform => &mut self.transform,
            EffectStage::Color => &mut self.color,
        }
    }
}

// --- 多段グラデーション ---

/// 1 つの山に積める色の数。
pub const MAX_GRADIENT_STOPS: usize = 8;

/// 色の止め位置。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct GradientStop {
    /// 0 が始まり、1 が終わり。並びは [`Gradient::stop`] が揃えます。
    pub position: f32,
    /// sRGB で書きます。積むときに 1 度だけ線形へ直されます。
    pub color: [f32; 4],
}

impl GradientStop {
    pub fn new(position: f32, color: [f32; 4]) -> Self {
        Self { position, color }
    }
}

/// どう流れるか。座標はどれも**図形ローカルの 0..1**（外接矩形を正規化したもの）。
///
/// 図形と一緒に動いて一緒に回るので、「この形を塗る」用途に向きます。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum GradientShape {
    /// 一方向に流れる。`angle` はラジアン、0 で左から右。
    Linear { angle: f32 },
    /// 中心から広がる。`radius` が 0.5 なら外接矩形にちょうど収まる。
    Radial {
        center: [f32; 2],
        radius_x: f32,
        radius_y: f32,
    },
    /// 中心のまわりを一周する。時計の針のように色が変わる。
    Conic {
        center: [f32; 2],
        start_angle: f32,
    },
}

/// 0..1 の外をどう埋めるか。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub enum GradientSpread {
    /// 両端の色をそのまま伸ばす。
    #[default]
    Clamp,
    /// 頭に戻って繰り返す。境目で色が飛ぶ。
    Repeat,
    /// 折り返して繰り返す。境目でも色がつながる。
    Mirror,
}

/// 色を何段でも置けるグラデーション。
///
/// 2 色の線形でよければ [`Block::Gradient`] のほうが短く書けて、
/// GPU に積む量も 1 つで済みます。**3 色以上・放射・角度**はこちらです。
///
/// ```
/// # use gueiz_2d::effect::{Block, Gradient, GradientSpread};
/// // 虹。図形ローカルで左から右へ。
/// let rainbow = Gradient::linear(0.0)
///     .stop(0.0, [1.0, 0.0, 0.0, 1.0])
///     .stop(0.5, [0.0, 1.0, 0.0, 1.0])
///     .stop(1.0, [0.0, 0.0, 1.0, 1.0]);
///
/// // 中心から広がる光。外へ行くほど透ける。
/// let glow = Gradient::radial([0.5, 0.5], 0.5, 0.5)
///     .stop(0.0, [1.0, 1.0, 0.8, 1.0])
///     .stop(1.0, [1.0, 0.6, 0.0, 0.0]);
///
/// // 角度。繰り返して縞にする。
/// let wheel = Gradient::conic([0.5, 0.5], 0.0)
///     .spread(GradientSpread::Repeat)
///     .stop(0.0, [0.0, 0.0, 0.0, 1.0])
///     .stop(1.0, [1.0, 1.0, 1.0, 1.0]);
///
/// # let _ = (Block::GradientStops { gradient: rainbow }, glow, wheel);
/// ```
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Gradient {
    shape: GradientShape,
    spread: GradientSpread,
    stops: [GradientStop; MAX_GRADIENT_STOPS],
    count: u8,
}

impl Gradient {
    fn new(shape: GradientShape) -> Self {
        Self {
            shape,
            spread: GradientSpread::default(),
            stops: [GradientStop::new(0.0, [0.0; 4]); MAX_GRADIENT_STOPS],
            count: 0,
        }
    }

    /// 一方向に流れる。`angle` はラジアン、0 で左から右。
    pub fn linear(angle: f32) -> Self {
        Self::new(GradientShape::Linear { angle })
    }

    /// 中心から広がる。座標も半径も図形ローカルの 0..1。
    pub fn radial(center: [f32; 2], radius_x: f32, radius_y: f32) -> Self {
        Self::new(GradientShape::Radial {
            center,
            radius_x,
            radius_y,
        })
    }

    /// 中心のまわりを一周する。
    pub fn conic(center: [f32; 2], start_angle: f32) -> Self {
        Self::new(GradientShape::Conic {
            center,
            start_angle,
        })
    }

    /// 色を 1 つ置く。**位置の順に並べ直される**ので、積む順は問いません。
    ///
    /// [`MAX_GRADIENT_STOPS`] を超えたぶんは捨てます。
    pub fn stop(mut self, position: f32, color: [f32; 4]) -> Self {
        if usize::from(self.count) == MAX_GRADIENT_STOPS {
            return self;
        }

        let stop = GradientStop::new(position, color);

        // 挿し込む先を探す。同じ位置なら後から来たほうを後ろに。
        let mut at = usize::from(self.count);

        while at > 0 && self.stops[at - 1].position > position {
            self.stops[at] = self.stops[at - 1];
            at -= 1;
        }

        self.stops[at] = stop;
        self.count += 1;
        self
    }

    /// 色をまとめて置く。
    pub fn colors(mut self, stops: &[GradientStop]) -> Self {
        for stop in stops {
            self = self.stop(stop.position, stop.color);
        }

        self
    }

    /// 0..1 の外をどう埋めるか。
    pub fn spread(mut self, spread: GradientSpread) -> Self {
        self.spread = spread;
        self
    }

    /// 置かれた色。位置の順。
    pub fn stops(&self) -> &[GradientStop] {
        &self.stops[..usize::from(self.count)]
    }

    pub fn shape(&self) -> GradientShape {
        self.shape
    }

    pub fn gradient_spread(&self) -> GradientSpread {
        self.spread
    }

    /// 色として成り立っているか。
    ///
    /// 1 つも置かれていないと混ぜようがないので、そのときは何もしません。
    pub fn is_usable(&self) -> bool {
        self.count >= 1
            && self
                .stops()
                .iter()
                .all(|stop| stop.position.is_finite())
    }

    /// GPU に積むときに使う数。先頭の 1 つに、色を 2 つずつ収めた続きが付く。
    fn raw_count(&self) -> usize {
        1 + usize::from(self.count).div_ceil(2)
    }

    /// シェーダ側の `switch` に使う番号。
    fn kind(&self) -> u32 {
        match self.shape {
            GradientShape::Linear { .. } => 14,
            GradientShape::Radial { .. } => 15,
            GradientShape::Conic { .. } => 16,
        }
    }

    /// 先頭の 1 つ。形と、続きの読み方。
    fn head(&self) -> BlockRaw {
        let params = match self.shape {
            GradientShape::Linear { angle } => [angle, 0.0, 0.0, 0.0],

            GradientShape::Radial {
                center,
                radius_x,
                radius_y,
            } => [center[0], center[1], radius_x, radius_y],

            GradientShape::Conic {
                center,
                start_angle,
            } => [center[0], center[1], start_angle, 0.0],
        };

        let spread = match self.spread {
            GradientSpread::Clamp => 0.0,
            GradientSpread::Repeat => 1.0,
            GradientSpread::Mirror => 2.0,
        };

        BlockRaw {
            params,
            // 色ではなく数なので、sRGB は通さない。
            color_a: [self.count as f32, spread, 0.0, 0.0],
            color_b: [0.0; 4],
            kind: self.kind(),
            _padding: [0; 3],
        }
    }

    /// 色を 2 つずつ収めた続き。
    fn tail(&self) -> impl Iterator<Item = BlockRaw> + '_ {
        self.stops().chunks(2).map(|pair| {
            let second = pair.get(1).copied().unwrap_or(pair[0]);

            BlockRaw {
                params: [pair[0].position, second.position, 0.0, 0.0],
                // 色は sRGB で書いてもらい、ここで線形に直す。
                color_a: srgb_to_linear(pair[0].color),
                color_b: srgb_to_linear(second.color),
                // 続きは `switch` に載らない。頭が索引で直に読む。
                kind: GRADIENT_STOP_KIND,
                _padding: [0; 3],
            }
        })
    }
}

/// 色を収めた続きの番号。`switch` には出てこない。
///
/// 頭の山が索引で直に読むので、**輪はここを飛ばします**。
pub(crate) const GRADIENT_STOP_KIND: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_knows_its_own_stage() {
        assert_eq!(Block::Spin { speed: 1.0 }.stage(), EffectStage::Transform);
        assert_eq!(Block::Tint { color: [1.0; 4] }.stage(), EffectStage::Color);
        assert_eq!(
            Block::Outline {
                width: 1.0,
                color: [1.0; 4],
            }
            .stage(),
            EffectStage::Shape,
        );
    }

    /// 積む側は段を意識しなくてよいが、段は勝手に分かれる。
    #[test]
    fn pushing_sorts_into_stages() {
        let mut effects = EffectStack::new();
        effects.push(Block::Tint { color: [1.0; 4] });
        effects.push(Block::Spin { speed: 1.0 });
        effects.push(Block::Flicker {
            amount: 0.5,
            speed: 2.0,
        });

        assert_eq!(effects.blocks(EffectStage::Color).len(), 2);
        assert_eq!(effects.blocks(EffectStage::Transform).len(), 1);
        assert_eq!(effects.blocks(EffectStage::Shape).len(), 0);
        assert_eq!(effects.len(), 3);
    }

    /// 同じ段の中では、積んだ順がそのまま効く順。
    #[test]
    fn order_within_a_stage_is_the_push_order() {
        let mut effects = EffectStack::new();
        effects.push(Block::Tint { color: [1.0; 4] });
        effects.push(Block::Flicker {
            amount: 0.5,
            speed: 2.0,
        });

        let color = effects.blocks(EffectStage::Color);
        assert!(matches!(color[0], Block::Tint { .. }));
        assert!(matches!(color[1], Block::Flicker { .. }));
    }

    #[test]
    fn blocks_can_be_reordered_within_a_stage() {
        let mut effects = EffectStack::new();
        effects.push(Block::Tint { color: [1.0; 4] });
        effects.push(Block::Flicker {
            amount: 0.5,
            speed: 2.0,
        });

        assert!(effects.move_block(EffectStage::Color, 1, 0));

        let color = effects.blocks(EffectStage::Color);
        assert!(matches!(color[0], Block::Flicker { .. }));
        assert!(matches!(color[1], Block::Tint { .. }));
    }

    #[test]
    fn out_of_range_moves_and_removes_do_nothing() {
        let mut effects = EffectStack::new();
        effects.push(Block::Tint { color: [1.0; 4] });

        assert!(!effects.move_block(EffectStage::Color, 0, 5));
        assert!(!effects.move_block(EffectStage::Transform, 0, 0));
        assert_eq!(effects.remove(EffectStage::Color, 3), None);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn removing_takes_the_block_out() {
        let mut effects = EffectStack::new();
        effects.push(Block::Tint { color: [1.0; 4] });
        effects.push(Block::Spin { speed: 1.0 });

        assert!(matches!(
            effects.remove(EffectStage::Color, 0),
            Some(Block::Tint { .. }),
        ));
        assert_eq!(effects.blocks(EffectStage::Color).len(), 0);
        assert_eq!(effects.blocks(EffectStage::Transform).len(), 1);
    }

    /// GPU に載せる形の並びは WGSL 側と揃っていないといけない。
    #[test]
    fn the_gpu_layout_is_what_the_shader_expects() {
        assert_eq!(size_of::<BlockRaw>(), 64);
        assert_eq!(align_of::<BlockRaw>(), 4);

        let raw = Block::Gradient {
            from: [1.0, 0.0, 0.0, 1.0],
            to: [0.0, 0.0, 1.0, 1.0],
            angle: 0.5,
        }
        .to_raw();

        assert_eq!(raw.kind, 6);
        assert_eq!(raw.params[0], 0.5);
        assert_eq!(raw.color_a, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(raw.color_b, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_custom_block_carries_its_own_stage_and_kind() {
        let block = Block::Custom {
            stage: EffectStage::Color,
            kind: CUSTOM_KIND_BASE + 3,
            params: [1.0, 2.0, 3.0, 4.0],
            color: [0.5; 4],
        };

        assert_eq!(block.stage(), EffectStage::Color);
        assert_eq!(block.kind(), CUSTOM_KIND_BASE + 3);

        let raw = block.to_raw();
        assert_eq!(raw.params, [1.0, 2.0, 3.0, 4.0]);
        // 色は sRGB で書いて、線形に直して載せる。
        assert_eq!(raw.color_a, srgb_to_linear([0.5; 4]));
        assert_eq!(raw.color_a[3], 0.5, "不透明度は変換しない");
    }

    // --- 色の空間 ---

    /// 両端は動かない。動くと原色が原色でなくなる。
    #[test]
    fn black_and_white_survive_the_conversion() {
        assert_eq!(srgb_to_linear([0.0, 0.0, 0.0, 1.0]), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(srgb_to_linear([1.0, 1.0, 1.0, 1.0]), [1.0, 1.0, 1.0, 1.0]);
    }

    /// 見た目の中間の灰色は、光の量では 5 分の 1 ほどしかない。
    /// ここを直さないと、`0.5` が明るすぎる灰色で出る。
    #[test]
    fn middle_grey_is_much_darker_in_light() {
        let [linear, ..] = srgb_to_linear([0.5, 0.5, 0.5, 1.0]);

        assert!((linear - 0.2140).abs() < 1e-3, "{linear}");
    }

    /// 不透明度は覆う割合であって光の量ではない。変換すると半透明がずれる。
    #[test]
    fn the_alpha_is_left_alone() {
        for alpha in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(srgb_to_linear([0.5, 0.5, 0.5, alpha])[3], alpha);
        }
    }

    /// 暗いほうは直線。ここを冪だけで済ませると、真っ暗付近が潰れる。
    #[test]
    fn the_dark_end_stays_on_the_straight_part() {
        assert!((srgb_to_linear([0.04, 0.0, 0.0, 1.0])[0] - 0.04 / 12.92).abs() < 1e-6);
        // 切り替わりの前後でつながっていること。
        let below = srgb_to_linear([0.04044, 0.0, 0.0, 1.0])[0];
        let above = srgb_to_linear([0.04046, 0.0, 0.0, 1.0])[0];

        assert!((above - below).abs() < 1e-5, "{below} と {above} で跳んでいる");
    }

    /// 明るいほうへ単調に増えること。逆転すると色の順番が狂う。
    #[test]
    fn the_conversion_never_goes_backwards() {
        let mut previous = -1.0_f32;

        for step in 0..=100 {
            let value = srgb_to_linear([step as f32 / 100.0, 0.0, 0.0, 1.0])[0];

            assert!(value > previous, "{step} で減った");
            previous = value;
        }
    }

    /// 自前の番号は用意された山とかぶってはいけない。
    #[test]
    fn custom_kinds_start_above_the_built_in_ones() {
        let built_in = [
            Block::Spin { speed: 0.0 },
            Block::Wobble { amplitude: 0.0, frequency: 0.0 },
            Block::Orbit { radius: 0.0, speed: 0.0 },
            Block::Pulse { amount: 0.0, speed: 0.0 },
            Block::Tint { color: [0.0; 4] },
            Block::Gradient { from: [0.0; 4], to: [0.0; 4], angle: 0.0 },
            Block::Dissolve { threshold: 0.0, edge: 0.0, edge_color: [0.0; 4] },
            Block::Flicker { amount: 0.0, speed: 0.0 },
        ];

        for block in built_in {
            assert!(block.kind() < CUSTOM_KIND_BASE, "{block:?}");
        }
    }

    /// 自前の山も、積めば段ごとに分かれる。
    #[test]
    fn custom_blocks_sort_into_their_stage() {
        let mut effects = EffectStack::new();
        effects.push(Block::Custom {
            stage: EffectStage::Transform,
            kind: CUSTOM_KIND_BASE,
            params: [0.0; 4],
            color: [0.0; 4],
        });

        assert_eq!(effects.blocks(EffectStage::Transform).len(), 1);
        assert_eq!(effects.blocks(EffectStage::Color).len(), 0);
    }

    /// 番号がかぶると別のエフェクトが走る。
    #[test]
    fn every_block_has_its_own_kind() {
        let blocks = [
            Block::Spin { speed: 0.0 },
            Block::Wobble { amplitude: 0.0, frequency: 0.0 },
            Block::Orbit { radius: 0.0, speed: 0.0 },
            Block::Pulse { amount: 0.0, speed: 0.0 },
            Block::Tint { color: [0.0; 4] },
            Block::Gradient { from: [0.0; 4], to: [0.0; 4], angle: 0.0 },
            Block::Dissolve { threshold: 0.0, edge: 0.0, edge_color: [0.0; 4] },
            Block::Flicker { amount: 0.0, speed: 0.0 },
        ];

        let mut kinds: Vec<u32> = blocks.iter().map(Block::kind).collect();
        kinds.sort_unstable();
        kinds.dedup();

        assert_eq!(kinds.len(), blocks.len());
        // 0 は「何もしない」に予約してある。
        assert!(!kinds.contains(&0));
    }

    // --- 多段グラデーション ---

    fn ramp() -> Gradient {
        Gradient::linear(0.0)
            .stop(0.0, [1.0, 0.0, 0.0, 1.0])
            .stop(0.5, [0.0, 1.0, 0.0, 1.0])
            .stop(1.0, [0.0, 0.0, 1.0, 1.0])
    }

    /// 積む順は問わない。位置の順に並べ直される。
    #[test]
    fn stops_sort_themselves() {
        let jumbled = Gradient::linear(0.0)
            .stop(1.0, [0.0, 0.0, 1.0, 1.0])
            .stop(0.0, [1.0, 0.0, 0.0, 1.0])
            .stop(0.5, [0.0, 1.0, 0.0, 1.0]);

        let positions: Vec<f32> = jumbled.stops().iter().map(|s| s.position).collect();

        assert_eq!(positions, vec![0.0, 0.5, 1.0]);
        assert_eq!(jumbled.stops(), ramp().stops());
    }

    #[test]
    fn stops_beyond_the_limit_are_dropped() {
        let mut gradient = Gradient::linear(0.0);

        for index in 0..MAX_GRADIENT_STOPS + 5 {
            gradient = gradient.stop(index as f32, [1.0; 4]);
        }

        assert_eq!(gradient.stops().len(), MAX_GRADIENT_STOPS);
    }

    #[test]
    fn a_gradient_with_no_stops_is_not_usable() {
        assert!(!Gradient::linear(0.0).is_usable());
        assert!(Gradient::linear(0.0).stop(0.0, [1.0; 4]).is_usable());
        assert!(!Gradient::linear(0.0).stop(f32::NAN, [1.0; 4]).is_usable());
    }

    /// 形ごとに別の番号。ぶつかるとシェーダが取り違える。
    #[test]
    fn each_shape_has_its_own_number() {
        let kind = |gradient: Gradient| Block::GradientStops { gradient }.kind();

        let numbers = [
            kind(Gradient::linear(0.0)),
            kind(Gradient::radial([0.5; 2], 0.5, 0.5)),
            kind(Gradient::conic([0.5; 2], 0.0)),
        ];

        assert_eq!(numbers, [14, 15, 16]);

        for number in numbers {
            assert!(number < CUSTOM_KIND_BASE);
        }
    }

    /// 色は 2 つずつ収める。頭 1 つ + 続き。
    #[test]
    fn the_stops_ride_two_to_an_entry() {
        for (count, entries) in [(1, 2), (2, 2), (3, 3), (4, 3), (5, 4), (8, 5)] {
            let mut gradient = Gradient::linear(0.0);

            for index in 0..count {
                gradient = gradient.stop(index as f32 / count as f32, [1.0; 4]);
            }

            let block = Block::GradientStops { gradient };

            assert_eq!(block.raw_count(), entries, "色 {count} 個");

            let mut out = Vec::new();
            block.write_raw(&mut out);

            assert_eq!(out.len(), entries, "色 {count} 個で積んだ数が合わない");
        }
    }

    /// 頭に色の数が載っていること。シェーダはこれを見て続きを読む。
    #[test]
    fn the_head_says_how_many_stops_follow() {
        let mut out = Vec::new();
        Block::GradientStops { gradient: ramp() }.write_raw(&mut out);

        assert_eq!(out[0].color_a[0], 3.0, "色の数");
        assert_eq!(out[0].kind, 14, "線形");
    }

    /// 色は sRGB で受けて線形に直す。位置は数なので通さない。
    #[test]
    fn stop_colours_go_linear_but_positions_do_not() {
        let mut out = Vec::new();
        Block::GradientStops {
            gradient: Gradient::linear(0.0)
                .stop(0.25, [0.5, 0.5, 0.5, 1.0])
                .stop(0.75, [1.0, 1.0, 1.0, 1.0]),
        }
        .write_raw(&mut out);

        // 位置はそのまま。
        assert_eq!(out[1].params[0], 0.25);
        assert_eq!(out[1].params[1], 0.75);

        // sRGB の 0.5 は線形では 0.214。
        assert!((out[1].color_a[0] - 0.2140).abs() < 0.001, "{}", out[1].color_a[0]);
        // 不透明度は変換しない。
        assert_eq!(out[1].color_a[3], 1.0);
        // 1.0 は動かない。
        assert_eq!(out[1].color_b[0], 1.0);
    }

    #[test]
    fn the_shape_rides_in_the_params() {
        let mut out = Vec::new();
        Block::GradientStops {
            gradient: Gradient::radial([0.25, 0.75], 0.4, 0.2).stop(0.0, [1.0; 4]),
        }
        .write_raw(&mut out);

        assert_eq!(out[0].params, [0.25, 0.75, 0.4, 0.2]);
    }

    #[test]
    fn the_spread_rides_along_too() {
        let raw = |spread| {
            let mut out = Vec::new();
            Block::GradientStops {
                gradient: ramp().spread(spread),
            }
            .write_raw(&mut out);
            out[0].color_a[1]
        };

        assert_eq!(raw(GradientSpread::Clamp), 0.0);
        assert_eq!(raw(GradientSpread::Repeat), 1.0);
        assert_eq!(raw(GradientSpread::Mirror), 2.0);
    }

    #[test]
    fn a_gradient_runs_in_the_colour_stage() {
        assert_eq!(
            Block::GradientStops { gradient: ramp() }.stage(),
            EffectStage::Color,
        );
    }

    /// ほかの山は 1 つのまま。数え方を変えても壊れていないこと。
    #[test]
    fn ordinary_blocks_still_take_one_entry() {
        for block in [
            Block::Tint { color: [1.0; 4] },
            Block::Spin { speed: 1.0 },
            Block::Gradient {
                from: [1.0; 4],
                to: [0.0; 4],
                angle: 0.0,
            },
        ] {
            assert_eq!(block.raw_count(), 1, "{block:?}");

            let mut out = Vec::new();
            block.write_raw(&mut out);

            assert_eq!(out.len(), 1);
            assert_eq!(out[0], block.to_raw());
        }
    }
}
