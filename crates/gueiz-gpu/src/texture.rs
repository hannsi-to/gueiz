//! テクスチャの形式や次元まわりの wgpu 型ラッパー。
//!
//! `TextureFormat` は変種が 77 個あり、うち 76 個は wgpu と同名の単純な対応に
//! しかならない。手書きすると往復で 150 以上の match アームになるので、
//! 変種の一覧を 1 か所に置いてマクロで展開している。

macro_rules! texture_format {
    ($($variant:ident,)*) => {
        #[derive(Clone, Copy)]
        #[derive(Eq, PartialEq, Hash)]
        #[derive(Debug)]
        pub enum TextureFormat {
            $($variant,)*
            Astc {
                block: AstcBlock,
                channel: AstcChannel,
            },
        }

        impl TextureFormat {
            pub const ALL: &'static [TextureFormat] = &[$(TextureFormat::$variant,)*];
        }

        impl From<TextureFormat> for wgpu::TextureFormat {
            fn from(value: TextureFormat) -> Self {
                match value {
                    $(TextureFormat::$variant => wgpu::TextureFormat::$variant,)*
                    TextureFormat::Astc { block, channel } => wgpu::TextureFormat::Astc {
                        block: block.into(),
                        channel: channel.into(),
                    },
                }
            }
        }

        impl From<wgpu::TextureFormat> for TextureFormat {
            fn from(value: wgpu::TextureFormat) -> Self {
                match value {
                    $(wgpu::TextureFormat::$variant => TextureFormat::$variant,)*
                    wgpu::TextureFormat::Astc { block, channel } => TextureFormat::Astc {
                        block: block.into(),
                        channel: channel.into(),
                    },
                }
            }
        }
    };
}

texture_format! {
    R8Unorm,
    R8Snorm,
    R8Uint,
    R8Sint,
    R16Uint,
    R16Sint,
    R16Unorm,
    R16Snorm,
    R16Float,
    R32Uint,
    R32Sint,
    R32Float,
    R64Uint,

    Rg8Unorm,
    Rg8Snorm,
    Rg8Uint,
    Rg8Sint,
    Rg16Uint,
    Rg16Sint,
    Rg16Unorm,
    Rg16Snorm,
    Rg16Float,
    Rg32Uint,
    Rg32Sint,
    Rg32Float,

    Rgba8Unorm,
    Rgba8UnormSrgb,
    Rgba8Snorm,
    Rgba8Uint,
    Rgba8Sint,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgba16Uint,
    Rgba16Sint,
    Rgba16Unorm,
    Rgba16Snorm,
    Rgba16Float,
    Rgba32Uint,
    Rgba32Sint,
    Rgba32Float,

    Rgb9e5Ufloat,
    Rgb10a2Uint,
    Rgb10a2Unorm,
    Rg11b10Ufloat,

    Stencil8,
    Depth16Unorm,
    Depth24Plus,
    Depth24PlusStencil8,
    Depth32Float,
    Depth32FloatStencil8,

    NV12,
    P010,

    Bc1RgbaUnorm,
    Bc1RgbaUnormSrgb,
    Bc2RgbaUnorm,
    Bc2RgbaUnormSrgb,
    Bc3RgbaUnorm,
    Bc3RgbaUnormSrgb,
    Bc4RUnorm,
    Bc4RSnorm,
    Bc5RgUnorm,
    Bc5RgSnorm,
    Bc6hRgbUfloat,
    Bc6hRgbFloat,
    Bc7RgbaUnorm,
    Bc7RgbaUnormSrgb,

    Etc2Rgb8Unorm,
    Etc2Rgb8UnormSrgb,
    Etc2Rgb8A1Unorm,
    Etc2Rgb8A1UnormSrgb,
    Etc2Rgba8Unorm,
    Etc2Rgba8UnormSrgb,
    EacR11Unorm,
    EacR11Snorm,
    EacRg11Unorm,
    EacRg11Snorm,
}

impl TextureFormat {
    pub fn is_srgb(self) -> bool {
        wgpu::TextureFormat::from(self).is_srgb()
    }

    pub fn add_srgb_suffix(self) -> Self {
        wgpu::TextureFormat::from(self).add_srgb_suffix().into()
    }

    pub fn remove_srgb_suffix(self) -> Self {
        wgpu::TextureFormat::from(self).remove_srgb_suffix().into()
    }

    pub fn has_color_aspect(self) -> bool {
        wgpu::TextureFormat::from(self).has_color_aspect()
    }

    pub fn has_depth_aspect(self) -> bool {
        wgpu::TextureFormat::from(self).has_depth_aspect()
    }

    pub fn has_stencil_aspect(self) -> bool {
        wgpu::TextureFormat::from(self).has_stencil_aspect()
    }

    pub fn is_compressed(self) -> bool {
        wgpu::TextureFormat::from(self).is_compressed()
    }

    pub fn components(self) -> u8 {
        wgpu::TextureFormat::from(self).components()
    }

    pub fn block_dimensions(self) -> (u32, u32) {
        wgpu::TextureFormat::from(self).block_dimensions()
    }

    pub fn block_copy_size(self) -> Option<u32> {
        wgpu::TextureFormat::from(self).block_copy_size(None)
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq, Hash)]
#[derive(Debug)]
pub enum AstcBlock {
    B4x4,
    B5x4,
    B5x5,
    B6x5,
    B6x6,
    B8x5,
    B8x6,
    B8x8,
    B10x5,
    B10x6,
    B10x8,
    B10x10,
    B12x10,
    B12x12,
}

impl From<AstcBlock> for wgpu::AstcBlock {
    fn from(value: AstcBlock) -> Self {
        match value {
            AstcBlock::B4x4 => wgpu::AstcBlock::B4x4,
            AstcBlock::B5x4 => wgpu::AstcBlock::B5x4,
            AstcBlock::B5x5 => wgpu::AstcBlock::B5x5,
            AstcBlock::B6x5 => wgpu::AstcBlock::B6x5,
            AstcBlock::B6x6 => wgpu::AstcBlock::B6x6,
            AstcBlock::B8x5 => wgpu::AstcBlock::B8x5,
            AstcBlock::B8x6 => wgpu::AstcBlock::B8x6,
            AstcBlock::B8x8 => wgpu::AstcBlock::B8x8,
            AstcBlock::B10x5 => wgpu::AstcBlock::B10x5,
            AstcBlock::B10x6 => wgpu::AstcBlock::B10x6,
            AstcBlock::B10x8 => wgpu::AstcBlock::B10x8,
            AstcBlock::B10x10 => wgpu::AstcBlock::B10x10,
            AstcBlock::B12x10 => wgpu::AstcBlock::B12x10,
            AstcBlock::B12x12 => wgpu::AstcBlock::B12x12,
        }
    }
}

impl From<wgpu::AstcBlock> for AstcBlock {
    fn from(value: wgpu::AstcBlock) -> Self {
        match value {
            wgpu::AstcBlock::B4x4 => AstcBlock::B4x4,
            wgpu::AstcBlock::B5x4 => AstcBlock::B5x4,
            wgpu::AstcBlock::B5x5 => AstcBlock::B5x5,
            wgpu::AstcBlock::B6x5 => AstcBlock::B6x5,
            wgpu::AstcBlock::B6x6 => AstcBlock::B6x6,
            wgpu::AstcBlock::B8x5 => AstcBlock::B8x5,
            wgpu::AstcBlock::B8x6 => AstcBlock::B8x6,
            wgpu::AstcBlock::B8x8 => AstcBlock::B8x8,
            wgpu::AstcBlock::B10x5 => AstcBlock::B10x5,
            wgpu::AstcBlock::B10x6 => AstcBlock::B10x6,
            wgpu::AstcBlock::B10x8 => AstcBlock::B10x8,
            wgpu::AstcBlock::B10x10 => AstcBlock::B10x10,
            wgpu::AstcBlock::B12x10 => AstcBlock::B12x10,
            wgpu::AstcBlock::B12x12 => AstcBlock::B12x12,
        }
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq, Hash)]
#[derive(Debug)]
pub enum AstcChannel {
    Unorm,
    UnormSrgb,
    Hdr,
}

impl From<AstcChannel> for wgpu::AstcChannel {
    fn from(value: AstcChannel) -> Self {
        match value {
            AstcChannel::Unorm => wgpu::AstcChannel::Unorm,
            AstcChannel::UnormSrgb => wgpu::AstcChannel::UnormSrgb,
            AstcChannel::Hdr => wgpu::AstcChannel::Hdr,
        }
    }
}

impl From<wgpu::AstcChannel> for AstcChannel {
    fn from(value: wgpu::AstcChannel) -> Self {
        match value {
            wgpu::AstcChannel::Unorm => AstcChannel::Unorm,
            wgpu::AstcChannel::UnormSrgb => AstcChannel::UnormSrgb,
            wgpu::AstcChannel::Hdr => AstcChannel::Hdr,
        }
    }
}

pub enum StorageTextureAccess {
    WriteOnly,
    ReadOnly,
    ReadWrite,
    Atomic,
}

impl From<StorageTextureAccess> for wgpu::StorageTextureAccess {
    fn from(value: StorageTextureAccess) -> Self {
        match value {
            StorageTextureAccess::WriteOnly => {
                wgpu::StorageTextureAccess::WriteOnly
            }
            StorageTextureAccess::ReadOnly => {
                wgpu::StorageTextureAccess::ReadOnly
            }
            StorageTextureAccess::ReadWrite => {
                wgpu::StorageTextureAccess::ReadWrite
            }
            StorageTextureAccess::Atomic => {
                wgpu::StorageTextureAccess::Atomic
            }
        }
    }
}

impl From<wgpu::StorageTextureAccess> for StorageTextureAccess {
    fn from(value: wgpu::StorageTextureAccess) -> Self {
        match value {
            wgpu::StorageTextureAccess::WriteOnly => {
                StorageTextureAccess::WriteOnly
            }
            wgpu::StorageTextureAccess::ReadOnly => {
                StorageTextureAccess::ReadOnly
            }
            wgpu::StorageTextureAccess::ReadWrite => {
                StorageTextureAccess::ReadWrite
            }
            wgpu::StorageTextureAccess::Atomic => {
                StorageTextureAccess::Atomic
            }
        }
    }
}

pub enum TextureDimension {
    D1,
    D2,
    D3,
}

impl From<TextureDimension> for wgpu::TextureDimension {
    fn from(value: TextureDimension) -> Self {
        match value {
            TextureDimension::D1 => {
                wgpu::TextureDimension::D1
            }
            TextureDimension::D2 => {
                wgpu::TextureDimension::D2
            }
            TextureDimension::D3 => {
                wgpu::TextureDimension::D3
            }
        }
    }
}

impl From<wgpu::TextureDimension> for TextureDimension {
    fn from(value: wgpu::TextureDimension) -> Self {
        match value {
            wgpu::TextureDimension::D1 => {
                TextureDimension::D1
            }
            wgpu::TextureDimension::D2 => {
                TextureDimension::D2
            }
            wgpu::TextureDimension::D3 => {
                TextureDimension::D3
            }
        }
    }
}

pub enum TextureViewDimension {
    D1,
    D2,
    D2Array,
    Cube,
    CubeArray,
    D3,
}

impl TextureViewDimension {
    pub fn compatible_texture_dimension(self) -> TextureDimension {
        let wgpu_texture_view_dimension: wgpu::TextureViewDimension = TextureViewDimension::into(self);
        let wgpu_texture_dimension = wgpu_texture_view_dimension.compatible_texture_dimension();
        TextureDimension::from(wgpu_texture_dimension)
    }
}

impl From<TextureViewDimension> for wgpu::TextureViewDimension {
    fn from(value: TextureViewDimension) -> Self {
        match value {
            TextureViewDimension::D1 => {
                wgpu::TextureViewDimension::D1
            }
            TextureViewDimension::D2 => {
                wgpu::TextureViewDimension::D2
            }
            TextureViewDimension::D2Array => {
                wgpu::TextureViewDimension::D2Array
            }
            TextureViewDimension::Cube => {
                wgpu::TextureViewDimension::Cube
            }
            TextureViewDimension::CubeArray => {
                wgpu::TextureViewDimension::CubeArray
            }
            TextureViewDimension::D3 => {
                wgpu::TextureViewDimension::D3
            }
        }
    }
}

impl From<wgpu::TextureViewDimension> for TextureViewDimension {
    fn from(value: wgpu::TextureViewDimension) -> Self {
        match value {
            wgpu::TextureViewDimension::D1 => {
                TextureViewDimension::D1
            }
            wgpu::TextureViewDimension::D2 => {
                TextureViewDimension::D2
            }
            wgpu::TextureViewDimension::D2Array => {
                TextureViewDimension::D2Array
            }
            wgpu::TextureViewDimension::Cube => {
                TextureViewDimension::Cube
            }
            wgpu::TextureViewDimension::CubeArray => {
                TextureViewDimension::CubeArray
            }
            wgpu::TextureViewDimension::D3 => {
                TextureViewDimension::D3
            }
        }
    }
}

pub enum TextureSampleType {
    Float{
        filterable: bool,
    },
    Depth,
    Sint,
    Uint,
}

impl From<TextureSampleType> for wgpu::TextureSampleType {
    fn from(value: TextureSampleType) -> Self {
        match value {
            TextureSampleType::Float { filterable } => {
                wgpu::TextureSampleType::Float { filterable }
            }
            TextureSampleType::Depth => {
                wgpu::TextureSampleType::Depth
            }
            TextureSampleType::Sint => {
                wgpu::TextureSampleType::Sint
            }
            TextureSampleType::Uint => {
                wgpu::TextureSampleType::Uint
            }
        }
    }
}

impl From<wgpu::TextureSampleType> for TextureSampleType {
    fn from(value: wgpu::TextureSampleType) -> Self {
        match value {
            wgpu::TextureSampleType::Float { filterable } => {
                TextureSampleType::Float { filterable }
            }
            wgpu::TextureSampleType::Depth => {
                TextureSampleType::Depth
            }
            wgpu::TextureSampleType::Sint => {
                TextureSampleType::Sint
            }
            wgpu::TextureSampleType::Uint => {
                TextureSampleType::Uint
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_texture_format_round_trips() {
        for format in TextureFormat::ALL {
            let wgpu_format = wgpu::TextureFormat::from(*format);
            assert_eq!(TextureFormat::from(wgpu_format), *format, "{format:?}");
        }
    }

    #[test]
    fn every_astc_combination_round_trips() {
        const BLOCKS: [AstcBlock; 14] = [
            AstcBlock::B4x4,
            AstcBlock::B5x4,
            AstcBlock::B5x5,
            AstcBlock::B6x5,
            AstcBlock::B6x6,
            AstcBlock::B8x5,
            AstcBlock::B8x6,
            AstcBlock::B8x8,
            AstcBlock::B10x5,
            AstcBlock::B10x6,
            AstcBlock::B10x8,
            AstcBlock::B10x10,
            AstcBlock::B12x10,
            AstcBlock::B12x12,
        ];
        const CHANNELS: [AstcChannel; 3] =
            [AstcChannel::Unorm, AstcChannel::UnormSrgb, AstcChannel::Hdr];

        for block in BLOCKS {
            for channel in CHANNELS {
                let format = TextureFormat::Astc { block, channel };
                let wgpu_format = wgpu::TextureFormat::from(format);
                assert_eq!(TextureFormat::from(wgpu_format), format, "{format:?}");
            }
        }
    }

    #[test]
    fn srgb_suffixes_pair_up() {
        assert!(!TextureFormat::Bgra8Unorm.is_srgb());
        assert!(TextureFormat::Bgra8UnormSrgb.is_srgb());

        assert_eq!(
            TextureFormat::Bgra8Unorm.add_srgb_suffix(),
            TextureFormat::Bgra8UnormSrgb,
        );
        assert_eq!(
            TextureFormat::Bgra8UnormSrgb.remove_srgb_suffix(),
            TextureFormat::Bgra8Unorm,
        );
    }

    #[test]
    fn reports_layout_details() {
        assert_eq!(TextureFormat::Rgba8Unorm.components(), 4);
        assert_eq!(TextureFormat::Rgba8Unorm.block_dimensions(), (1, 1));
        assert_eq!(TextureFormat::Rgba8Unorm.block_copy_size(), Some(4));
        assert!(!TextureFormat::Rgba8Unorm.is_compressed());

        assert_eq!(TextureFormat::Bc1RgbaUnorm.block_dimensions(), (4, 4));
        assert_eq!(TextureFormat::Bc1RgbaUnorm.block_copy_size(), Some(8));
        assert!(TextureFormat::Bc1RgbaUnorm.is_compressed());
    }

    #[test]
    fn separates_colour_and_depth_aspects() {
        assert!(TextureFormat::Rgba8Unorm.has_color_aspect());
        assert!(!TextureFormat::Rgba8Unorm.has_depth_aspect());

        assert!(TextureFormat::Depth32Float.has_depth_aspect());
        assert!(!TextureFormat::Depth32Float.has_color_aspect());

        assert!(TextureFormat::Depth24PlusStencil8.has_depth_aspect());
        assert!(TextureFormat::Depth24PlusStencil8.has_stencil_aspect());
    }
}
