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

    // --- 自前の山 ---
    /// 自分で書いた WGSL を走らせる。
    ///
    /// `kind` は [`CUSTOM_KIND_BASE`] 以上で、
    /// [`crate::object::draw_manager::DrawManager::set_custom_blocks`] に
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
            | Self::Flicker { .. } => EffectStage::Color,

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
            Self::Custom { kind, .. } => *kind,
        }
    }

    /// GPU に載せる形。`params` の意味は山ごとに違う。
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

            Self::Tint { color } => ([0.0; 4], color, [0.0; 4]),

            Self::Gradient { from, to, angle } => ([angle, 0.0, 0.0, 0.0], from, to),

            Self::Dissolve {
                threshold,
                edge,
                edge_color,
            } => ([threshold, edge, 0.0, 0.0], edge_color, [0.0; 4]),

            Self::Flicker { amount, speed } => ([amount, speed, 0.0, 0.0], [0.0; 4], [0.0; 4]),

            Self::Custom { params, color, .. } => (params, color, [0.0; 4]),
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

/// GPU に載る 1 山。WGSL 側と並びを揃える。
///
/// `vec4<f32>` はアラインメント 16 なので、末尾の詰め物を明示している。
#[repr(C)]
#[derive(Clone, Copy)]
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
        assert_eq!(raw.color_a, [0.5; 4]);
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
}
