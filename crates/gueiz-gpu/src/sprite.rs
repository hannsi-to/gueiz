//! スプライトの絵。複数枚を 1 本の配列テクスチャにまとめて持つ。
//!
//! # なぜ配列テクスチャか
//!
//! 絵を 1 枚ずつ別のテクスチャにすると、絵が変わるたびにバインドグループを
//! 差し替えることになり、**ドローが絵の数だけ割れます**。このクレートは
//! `multi_draw_indirect` 1 回に畳むのが前提なので、それでは前提が壊れます。
//!
//! 配列テクスチャなら**バインドは 1 回**で、どの層を読むかは図形ごとの番号で
//! 決まります。`shape_pool` に全図形の頂点を積んで `vertex_base` で引くのと
//! 同じやり方です。
//!
//! 代わりに**全部の層が同じ大きさ**でなければなりません。大きさの違う絵は、
//! 大きめの層に並べて置いて、図形ごとに切り出し範囲
//! （`gueiz_2d::object::Object::sprite_region`）で指定します。
//!
//! # 重なり順
//!
//! 絵の重なりは Z で決まります。`gueiz_2d::object::Object` の z が大きいほど
//! 手前に描かれます。詳しくは `gueiz_2d::object::draw_manager` を参照。

use crate::error::Gueiz2DError;

/// 1 画素あたりのバイト数。RGBA8 固定。
const BYTES_PER_PIXEL: u32 = 4;

/// 絵の拡大縮小のしかた。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub enum SpriteFilter {
    /// なめらかに混ぜる。写真や大きな絵向き。
    #[default]
    Linear,
    /// いちばん近い画素をそのまま。ドット絵はこちらでないと滲む。
    Nearest,
}

impl SpriteFilter {
    fn to_wgpu(self) -> wgpu::FilterMode {
        match self {
            Self::Linear => wgpu::FilterMode::Linear,
            Self::Nearest => wgpu::FilterMode::Nearest,
        }
    }
}

/// 同じ大きさの絵を何枚か束ねたもの。
///
/// ```no_run
/// # use gueiz_gpu::sprite::{SpriteSheet, SpriteFilter};
/// # fn run(device: &gueiz_gpu::wgpu::Device, queue: &gueiz_gpu::wgpu::Queue)
/// #     -> Result<(), gueiz_gpu::error::Gueiz2DError> {
/// // 32x32 の絵を 3 枚。どれも RGBA8 で 32*32*4 バイト。
/// let sheet = SpriteSheet::new(
///     device,
///     queue,
///     32,
///     32,
///     &[&[0u8; 32 * 32 * 4], &[0u8; 32 * 32 * 4], &[0u8; 32 * 32 * 4]],
///     SpriteFilter::Nearest,
/// )?;
///
/// assert_eq!(sheet.layer_count(), 3);
/// # Ok(())
/// # }
/// ```
pub struct SpriteSheet {
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    width: u32,
    height: u32,
    layer_count: u32,
}

impl SpriteSheet {
    /// 絵をまとめて載せる。`layers` はどれも `width * height * 4` バイトの RGBA8。
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        layers: &[&[u8]],
        filter: SpriteFilter,
    ) -> Result<Self, Gueiz2DError> {
        if width == 0 || height == 0 || layers.is_empty() {
            return Err(Gueiz2DError::EmptySpriteSheetError);
        }

        let expected = (width * height * BYTES_PER_PIXEL) as usize;

        for (index, pixels) in layers.iter().enumerate() {
            if pixels.len() != expected {
                return Err(Gueiz2DError::SpriteSizeMismatchError {
                    layer: index as u32,
                    expected,
                    found: pixels.len(),
                });
            }
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gueiz sprite sheet"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layers.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        for (index, pixels) in layers.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: index as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * BYTES_PER_PIXEL),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }

        // 層が 1 枚でも配列として見る。シェーダ側を 1 本にできる。
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("gueiz sprite sheet view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gueiz sprite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter.to_wgpu(),
            min_filter: filter.to_wgpu(),
            ..Default::default()
        });

        log::info!(
            "sprite sheet: {}x{}, {} layers ({} bytes)",
            width,
            height,
            layers.len(),
            expected * layers.len(),
        );

        Ok(Self {
            view,
            sampler,
            width,
            height,
            layer_count: layers.len() as u32,
        })
    }

    /// すでにあるテクスチャから、見る口とサンプラーだけを作る。
    ///
    /// [`crate::atlas::Atlas`] のように、**中身を自分で書き込む**側が使う。
    /// テクスチャは呼ぶ側が持ち続ける。
    pub fn from_texture(
        device: &wgpu::Device,
        texture: &wgpu::Texture,
        filter: SpriteFilter,
        mipmapped: bool,
    ) -> Self {
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("gueiz sprite sheet view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gueiz sprite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter.to_wgpu(),
            min_filter: filter.to_wgpu(),
            // 段のあいだも混ぜないと、縮小したときに段の切り替わりが見える。
            mipmap_filter: if mipmapped {
                wgpu::MipmapFilterMode::Linear
            } else {
                wgpu::MipmapFilterMode::Nearest
            },
            ..Default::default()
        });

        Self {
            view,
            sampler,
            width: texture.width(),
            height: texture.height(),
            layer_count: texture.depth_or_array_layers(),
        }
    }

    /// 真っ白な 1x1 を 1 枚だけ持つシート。
    ///
    /// 絵を指定していない図形もこれを読む。掛け算して 1 倍になるので、
    /// 頂点色がそのまま出る。シェーダを分岐させずに済む。
    pub fn white(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::new(device, queue, 1, 1, &[&[255, 255, 255, 255]], SpriteFilter::Nearest)
            .expect("1x1 の白は必ず作れる")
    }

    pub fn layer_count(&self) -> u32 {
        self.layer_count
    }

    /// 1 層の大きさ（画素）。
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 画素の範囲を、切り出し範囲（`[u, v, 幅, 高さ]`、いずれも 0..1）に直す。
    ///
    /// ドット絵を並べたシートから 1 コマを取り出すときに使う。
    ///
    /// ```no_run
    /// # use gueiz_gpu::sprite::SpriteSheet;
    /// # fn run(sheet: &SpriteSheet) {
    /// // 128x128 のシートの、左上から 32x32 のコマ。
    /// let region = sheet.region(0, 0, 32, 32);
    /// # }
    /// ```
    pub fn region(&self, x: u32, y: u32, width: u32, height: u32) -> [f32; 4] {
        [
            x as f32 / self.width as f32,
            y as f32 / self.height as f32,
            width as f32 / self.width as f32,
            height as f32 / self.height as f32,
        ]
    }

    /// バインドグループに差すための取っ手。
    ///
    /// 束ねるところ（`DrawManager`）は次元ごとに別のクレートにあるので、
    /// ここは公開しておく必要がある。
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }
}

/// 切り出し範囲の既定。層をまるごと使う。
pub const WHOLE_LAYER: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

#[cfg(test)]
mod tests {
    use super::*;

    /// 画素の範囲から切り出し範囲への変換。ここを間違えると絵がずれる。
    #[test]
    fn a_region_is_normalised_against_the_sheet() {
        // テクスチャを作らずに変換だけ確かめたいので、値を直接組む。
        let sheet_size = (128.0_f32, 128.0_f32);

        let to_region = |x: u32, y: u32, width: u32, height: u32| {
            [
                x as f32 / sheet_size.0,
                y as f32 / sheet_size.1,
                width as f32 / sheet_size.0,
                height as f32 / sheet_size.1,
            ]
        };

        assert_eq!(to_region(0, 0, 128, 128), WHOLE_LAYER);
        assert_eq!(to_region(0, 0, 32, 32), [0.0, 0.0, 0.25, 0.25]);
        assert_eq!(to_region(96, 96, 32, 32), [0.75, 0.75, 0.25, 0.25]);
    }

    #[test]
    fn filters_map_to_wgpu() {
        assert_eq!(SpriteFilter::Nearest.to_wgpu(), wgpu::FilterMode::Nearest);
        assert_eq!(SpriteFilter::Linear.to_wgpu(), wgpu::FilterMode::Linear);
        // ドット絵が滲むほうが既定だと事故りやすいが、一般の絵は Linear が自然。
        assert_eq!(SpriteFilter::default(), SpriteFilter::Linear);
    }
}
