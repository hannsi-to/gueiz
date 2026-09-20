//! 画面全体に掛けるエフェクト。
//!
//! # 段としての位置づけ
//!
//! `gueiz_2d::effect` の 3 段（Shape / Transform / Color）は**図形ごと**に効き、
//! 塗る画素の上でしか走りません。ここはその後ろに付く 4 つめの段で、
//! **画面が 1 枚できあがってから**掛かります。
//!
//! 分かれ目は「すでに描かれた画素を読む必要があるか」です。
//! ぼかしやにじみは隣の画素を読むので、1 枚できあがるまで計算できません。
//! 逆にグラデーションや色掛けは図形ごとに済むので、こちらに置く理由はありません。
//!
//! # 値段
//!
//! **1 パスにつき画面 1 枚ぶんの読み書き**が固定で掛かります。図形が何個あっても、
//! 画面のどこに効くかにも関係なく同じです。図形ごとの段は「塗る画素だけ」なので、
//! 小さい図形に掛けるなら図形ごとの段のほうが桁違いに安くなります。
//!
//! [`PostEffect::Blur`] だけは縦横に分けるので 2 パス使います
//! （[`PostChain::pass_count`]）。
//!
//! # 順番
//!
//! パスの実行順がそのままエフェクトの順で、[`PostChain::push`] した順に走ります。
//! 図形ごとの段と違い、ここは全部が同じ「段」なので順番に制約はありません。

/// 画面全体に掛ける 1 つ。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum PostEffect {
    /// 明るさ・彩度・色味。`exposure` は掛け算、`saturation` は 0 で白黒、1 で元のまま。
    ColorGrade {
        exposure: f32,
        saturation: f32,
        tint: [f32; 4],
    },
    /// 周辺を落とす。`amount` が落とす深さ、`softness` が効き始める位置。
    Vignette { amount: f32, softness: f32 },
    /// ぼかす。`radius` は画素。**縦横に分けるので 2 パス使う。**
    Blur { radius: f32 },
    /// 明るいところをにじませる。
    ///
    /// 1 パスで済ませるため、ピラミッドを使う本来のブルームではなく
    /// 広めに散らして足すだけの近似。半径を大きくすると粗が見える。
    Glow {
        threshold: f32,
        intensity: f32,
        radius: f32,
    },
}

impl PostEffect {
    /// シェーダ側の `switch` に使う番号。
    pub(crate) fn kind(&self) -> u32 {
        match self {
            Self::ColorGrade { .. } => 1,
            Self::Vignette { .. } => 2,
            Self::Blur { .. } => 3,
            Self::Glow { .. } => 4,
        }
    }

    /// これが使うパスの数。
    pub fn pass_count(&self) -> usize {
        match self {
            // ぼかしは縦と横に分ける。まとめてやると 1 画素あたりの
            // サンプル数が半径の 2 乗で増えてしまう。
            Self::Blur { .. } => 2,
            _ => 1,
        }
    }

    /// `pass` 番目のパスに渡す値。`Blur` だけ `pass` で向きが変わる。
    pub(crate) fn to_raw(self, pass: usize, texel: [f32; 2]) -> PostUniform {
        let (params, color, direction) = match self {
            Self::ColorGrade {
                exposure,
                saturation,
                tint,
            } => ([exposure, saturation, 0.0, 0.0], tint, [0.0, 0.0]),

            Self::Vignette { amount, softness } => {
                ([amount, softness, 0.0, 0.0], [0.0; 4], [0.0, 0.0])
            }

            Self::Blur { radius } => {
                let direction = if pass == 0 { [1.0, 0.0] } else { [0.0, 1.0] };
                ([radius, 0.0, 0.0, 0.0], [0.0; 4], direction)
            }

            Self::Glow {
                threshold,
                intensity,
                radius,
            } => (
                [threshold, intensity, radius, 0.0],
                [0.0; 4],
                [0.0, 0.0],
            ),
        };

        PostUniform {
            params,
            color,
            texel,
            direction,
            kind: self.kind(),
            _padding: [0; 3],
        }
    }
}

/// 1 パスぶんの設定。ユニフォームなので 16 バイト境界に揃える。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PostUniform {
    pub params: [f32; 4],
    pub color: [f32; 4],
    /// 画素 1 つぶんの UV。`1 / 幅`, `1 / 高さ`。
    pub texel: [f32; 2],
    /// ぼかしの向き。
    pub direction: [f32; 2],
    pub kind: u32,
    pub _padding: [u32; 3],
}

/// 画面全体に掛けるエフェクトの列。**積んだ順に走る。**
///
/// ```
/// # use gueiz_gpu::post::{PostChain, PostEffect};
/// let mut chain = PostChain::new();
/// chain.push(PostEffect::Glow { threshold: 0.7, intensity: 0.8, radius: 4.0 });
/// chain.push(PostEffect::Blur { radius: 2.0 });
/// chain.push(PostEffect::Vignette { amount: 0.5, softness: 0.4 });
///
/// // ぼかしだけ 2 パス使うので、合計 4 パス。
/// assert_eq!(chain.pass_count(), 4);
/// ```
#[derive(Clone)]
#[derive(Default)]
#[derive(Debug)]
pub struct PostChain {
    effects: Vec<PostEffect>,
}

impl PostChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// 後ろに積む。積んだ順に走る。
    pub fn push(&mut self, effect: PostEffect) -> &mut Self {
        self.effects.push(effect);
        self
    }

    pub fn effects(&self) -> &[PostEffect] {
        &self.effects
    }

    pub fn remove(&mut self, index: usize) -> Option<PostEffect> {
        if index >= self.effects.len() {
            return None;
        }

        Some(self.effects.remove(index))
    }

    /// 順番を入れ替える。`from` を抜いて `to` に差し込む。
    pub fn move_effect(&mut self, from: usize, to: usize) -> bool {
        if from >= self.effects.len() || to >= self.effects.len() {
            return false;
        }

        let effect = self.effects.remove(from);
        self.effects.insert(to, effect);
        true
    }

    pub fn clear(&mut self) -> &mut Self {
        self.effects.clear();
        self
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// 走るパスの数。これがそのまま「画面何枚ぶん読み書きするか」。
    pub fn pass_count(&self) -> usize {
        self.effects.iter().map(PostEffect::pass_count).sum()
    }

    /// 走る順に `(エフェクト, そのエフェクトの中で何番目のパスか)` を並べる。
    pub(crate) fn passes(&self) -> impl Iterator<Item = (PostEffect, usize)> + '_ {
        self.effects
            .iter()
            .flat_map(|effect| (0..effect.pass_count()).map(move |pass| (*effect, pass)))
    }
}


/// 画面全体のパスを走らせる側。
///
/// [`crate::renderer::Renderer`] が 1 つ持っていて、普通はそちら経由で使う。
/// サーフェスではなく自前のテクスチャに掛けたいときだけ、直接組む。
///
/// ping-pong する 2 枚のテクスチャを行き来し、**最後の 1 パスだけ直接
/// サーフェスに書く**ので、余分な転送が 1 回減る。
///
/// パイプラインは 1 本で、どのエフェクトかはユニフォームの `kind` で切り替える。
/// Unity の uber shader と同じ形。パスごとにパイプラインを分けると、
/// 切り替えのぶんだけ無駄になる。
pub struct PostProcessor {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// ユニフォーム 1 つぶんの間隔。GPU が要求する境界に合わせて広げてある。
    stride: u32,
    /// 収まるパスの数。
    capacity: u32,
    targets: Option<Targets>,
}

/// ping-pong する 2 枚と、それを読むためのバインドグループ。
struct Targets {
    views: [wgpu::TextureView; 2],
    bind_groups: [wgpu::BindGroup; 2],
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

/// 収められるパスの上限。1 パス 256 バイト程度なので、多めに取っても安い。
const MAX_PASSES: u32 = 64;

impl PostProcessor {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gueiz post shader"),
            source: wgpu::ShaderSource::Wgsl(POST_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gueiz post layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        // パスごとに読む場所をずらす。バインドグループは 1 つで済む。
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(size_of::<PostUniform>() as u64),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gueiz post pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gueiz post pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // 前のパスの結果を丸ごと置き換える。混ぜる必要はない。
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gueiz post sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // 動的オフセットは GPU の要求する境界に揃っていないといけない。
        let alignment = device.limits().min_uniform_buffer_offset_alignment;
        let stride = (size_of::<PostUniform>() as u32).max(alignment).div_ceil(alignment) * alignment;

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gueiz post uniforms"),
            size: (stride * MAX_PASSES) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
            uniforms,
            stride,
            capacity: MAX_PASSES,
            targets: None,
        }
    }

    /// 描き先を用意する。大きさや形式が変わっていたら作り直す。
    fn ensure_targets(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        let fits = self.targets.as_ref().is_some_and(|targets| {
            targets.width == width && targets.height == height && targets.format == format
        });

        if fits {
            return;
        }

        let mut views = Vec::with_capacity(2);
        let mut bind_groups = Vec::with_capacity(2);
        let mut textures = Vec::with_capacity(2);

        for index in 0..2 {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gueiz post target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            views.push(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            textures.push(texture);
            let _ = index;
        }

        for view in &views {
            bind_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gueiz post bind group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.uniforms,
                            offset: 0,
                            size: wgpu::BufferSize::new(size_of::<PostUniform>() as u64),
                        }),
                    },
                ],
            }));
        }

        let views: [wgpu::TextureView; 2] = views.try_into().ok().expect("2 枚作った");
        let bind_groups: [wgpu::BindGroup; 2] =
            bind_groups.try_into().ok().expect("2 つ作った");

        log::debug!("post targets: {width}x{height} {format:?}");

        self.targets = Some(Targets {
            views,
            bind_groups,
            width,
            height,
            format,
        });
    }

    /// 場面を描き込む先。ここに描いてから [`PostProcessor::run`] を呼ぶ。
    pub fn scene_view(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> &wgpu::TextureView {
        self.ensure_targets(device, width, height, format);

        &self
            .targets
            .as_ref()
            .expect("ensure_targets が用意する")
            .views[0]
    }

    /// パスを順に走らせる。**最後の 1 パスだけ `destination` に直接書く。**
    ///
    /// [`PostProcessor::scene_view`] に場面を描いた後に呼ぶ。
    pub fn run(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        chain: &PostChain,
        destination: &wgpu::TextureView,
    ) {
        let Some(targets) = self.targets.as_ref() else {
            return;
        };

        let total = chain.pass_count();
        if total == 0 {
            return;
        }

        if total > self.capacity as usize {
            log::warn!(
                "the post chain has {} passes but only {} fit; the rest will not run",
                total,
                self.capacity,
            );
        }

        let total = total.min(self.capacity as usize);
        let texel = [1.0 / targets.width as f32, 1.0 / targets.height as f32];

        // 設定は先に全部上げる。パスの間でバッファを書き換えると順番が保証されない。
        for (index, (effect, pass)) in chain.passes().take(total).enumerate() {
            queue.write_buffer(
                &self.uniforms,
                (index as u32 * self.stride) as u64,
                bytemuck::bytes_of(&effect.to_raw(pass, texel)),
            );
        }

        for index in 0..total {
            let source = index % 2;
            let target = if index == total - 1 {
                destination
            } else {
                &targets.views[(index + 1) % 2]
            };

            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gueiz post pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // 画面を丸ごと塗り替えるので、消す必要はない。
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(
                0,
                &targets.bind_groups[source],
                &[index as u32 * self.stride],
            );
            // 画面を覆う三角形 1 枚。頂点バッファは要らない。
            render_pass.draw(0..3, 0..1);
        }
    }
}

const POST_SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct PostConfig {
    params: vec4<f32>,
    color: vec4<f32>,
    texel: vec2<f32>,
    direction: vec2<f32>,
    kind: u32,
    padding0: u32,
    padding1: u32,
    padding2: u32,
}

@group(0) @binding(2) var<uniform> config: PostConfig;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// 画面を覆う三角形 1 枚。四角 2 枚より、境目で二重に塗る画素が出ない。
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));

    var output: VertexOutput;
    output.uv = uv;
    // UV は左上原点、クリップ空間は下から上。y を裏返す。
    output.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    return output;
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// 9 点のガウスぼかし。向きは `direction` で決まる。
fn blur(uv: vec2<f32>) -> vec4<f32> {
    let step = config.direction * config.texel * config.params.x;
    let weights = array<f32, 5>(0.2270, 0.1945, 0.1216, 0.0540, 0.0162);

    var sum = textureSample(source, source_sampler, uv) * weights[0];

    for (var tap = 1; tap < 5; tap = tap + 1) {
        let offset = step * f32(tap);
        sum = sum + textureSample(source, source_sampler, uv + offset) * weights[tap];
        sum = sum + textureSample(source, source_sampler, uv - offset) * weights[tap];
    }

    return sum;
}

/// 明るいところを拾って散らす。1 パスで済ませるための近似。
fn glow(uv: vec2<f32>, base: vec4<f32>) -> vec4<f32> {
    let threshold = config.params.x;
    let intensity = config.params.y;
    let radius = config.params.z;

    var gathered = vec3<f32>(0.0);

    // 2 重の輪に 8 点ずつ。回転をずらして縞が出ないようにする。
    for (var ring = 1; ring <= 2; ring = ring + 1) {
        let distance = radius * f32(ring);

        for (var step = 0; step < 8; step = step + 1) {
            let angle = (f32(step) + f32(ring) * 0.5) * 0.7853981;
            let offset = vec2<f32>(cos(angle), sin(angle)) * distance * config.texel;
            let sampled = textureSample(source, source_sampler, uv + offset).rgb;

            gathered = gathered + max(sampled - vec3<f32>(threshold), vec3<f32>(0.0));
        }
    }

    return vec4<f32>(base.rgb + gathered / 16.0 * intensity, base.a);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(source, source_sampler, input.uv);

    switch config.kind {
        // ColorGrade
        case 1u: {
            var graded = base.rgb * config.params.x;
            let gray = luminance(graded);
            graded = mix(vec3<f32>(gray), graded, config.params.y);
            return vec4<f32>(graded * config.color.rgb, base.a);
        }
        // Vignette
        case 2u: {
            // 中心からの距離を、角で 1 になるように正規化する。
            let offset = input.uv - vec2<f32>(0.5);
            let distance = length(offset) * 1.41421356;
            let fade = 1.0 - smoothstep(config.params.y, 1.0, distance) * config.params.x;
            return vec4<f32>(base.rgb * fade, base.a);
        }
        // Blur
        case 3u: {
            return blur(input.uv);
        }
        // Glow
        case 4u: {
            return glow(input.uv, base);
        }
        default: {
            return base;
        }
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_runs_in_push_order() {
        let mut chain = PostChain::new();
        chain.push(PostEffect::Vignette {
            amount: 0.5,
            softness: 0.4,
        });
        chain.push(PostEffect::ColorGrade {
            exposure: 1.0,
            saturation: 1.0,
            tint: [1.0; 4],
        });

        let kinds: Vec<u32> = chain.passes().map(|(effect, _)| effect.kind()).collect();
        assert_eq!(kinds, vec![2, 1]);
    }

    /// ぼかしだけ 2 パス使う。パス数がそのまま値段になるので、数え間違えると困る。
    #[test]
    fn blur_costs_two_passes() {
        assert_eq!(PostEffect::Blur { radius: 1.0 }.pass_count(), 2);
        assert_eq!(
            PostEffect::Vignette {
                amount: 0.0,
                softness: 0.0,
            }
            .pass_count(),
            1,
        );

        let mut chain = PostChain::new();
        chain.push(PostEffect::Blur { radius: 1.0 });
        chain.push(PostEffect::Vignette {
            amount: 0.0,
            softness: 0.0,
        });

        assert_eq!(chain.pass_count(), 3);
        assert_eq!(chain.passes().count(), 3);
    }

    /// ぼかしの 2 パスは向きが違う。同じ向きで 2 回掛けても縦にぼけない。
    #[test]
    fn the_two_blur_passes_go_in_different_directions() {
        let blur = PostEffect::Blur { radius: 4.0 };
        let texel = [0.001, 0.002];

        assert_eq!(blur.to_raw(0, texel).direction, [1.0, 0.0]);
        assert_eq!(blur.to_raw(1, texel).direction, [0.0, 1.0]);
    }

    #[test]
    fn effects_can_be_reordered() {
        let mut chain = PostChain::new();
        chain.push(PostEffect::Blur { radius: 1.0 });
        chain.push(PostEffect::Vignette {
            amount: 0.0,
            softness: 0.0,
        });

        assert!(chain.move_effect(1, 0));
        assert_eq!(chain.effects()[0].kind(), 2);

        assert!(!chain.move_effect(0, 9));
        assert_eq!(chain.remove(9), None);
    }

    #[test]
    fn an_empty_chain_runs_nothing() {
        let chain = PostChain::new();

        assert!(chain.is_empty());
        assert_eq!(chain.pass_count(), 0);
        assert_eq!(chain.passes().count(), 0);
    }

    /// WGSL 側の `switch` と番号が合っていないと別のエフェクトが走る。
    #[test]
    fn every_effect_has_its_own_kind() {
        let effects = [
            PostEffect::ColorGrade {
                exposure: 0.0,
                saturation: 0.0,
                tint: [0.0; 4],
            },
            PostEffect::Vignette {
                amount: 0.0,
                softness: 0.0,
            },
            PostEffect::Blur { radius: 0.0 },
            PostEffect::Glow {
                threshold: 0.0,
                intensity: 0.0,
                radius: 0.0,
            },
        ];

        let mut kinds: Vec<u32> = effects.iter().map(PostEffect::kind).collect();
        kinds.sort_unstable();
        kinds.dedup();

        assert_eq!(kinds.len(), effects.len());
        assert!(!kinds.contains(&0), "0 は「何もしない」に予約");
    }

    #[test]
    fn the_gpu_layout_is_what_the_shader_expects() {
        assert_eq!(size_of::<PostUniform>(), 64);
    }
}
