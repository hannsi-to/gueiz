use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum Gueiz2DError {
    UnsupportedPlatform(String),
    SurfaceCreationError(wgpu::CreateSurfaceError),
    AdapterNotFoundError(wgpu::RequestAdapterError),
    DeviceCreationError(wgpu::RequestDeviceError),
    /// adapter がこのサーフェスに対応していない。
    UnsupportedSurfaceError,
    /// 文字列からバックエンドを解決できなかった。
    UnknownBackendError(String),
    /// ヒープに連続した空き領域が足りない。
    HeapExhaustedError {
        requested: u64,
        largest_free_block: u64,
    },
    /// 二重解放、別ヒープの区画、あるいは区画をはみ出す書き込み。
    InvalidAllocationError,
    /// 今フレームのスライスに空きが足りない。
    FrameRegionExhaustedError {
        requested: u64,
        available: u64,
    },
    /// GPU の処理完了待ちに失敗した。
    DevicePollError(wgpu::PollError),
    /// 自前の山の番号が `gueiz_2d::effect::CUSTOM_KIND_BASE` より小さい。
    ReservedBlockKindError(u32),
    /// 自前の山を `gueiz_2d::effect::EffectStage::Shape` に置こうとした。
    /// 形の段は CPU 側なので、WGSL では書けない。
    UnsupportedBlockStageError,
    /// 差し込んだ WGSL が通らなかった。
    ShaderCompilationError(String),
    /// 絵が 1 枚も無い、または大きさが 0。
    EmptySpriteSheetError,
    /// その層の画素数がシートの大きさと合わない。
    SpriteSizeMismatchError {
        layer: u32,
        expected: usize,
        found: usize,
    },
    /// フォントを読めなかった。
    FontParseError(String),
    /// アトラスに空きが無い。ページを大きくするか、上限を上げる。
    AtlasFullError {
        width: u32,
        height: u32,
        pages: u32,
    },
}

impl Display for Gueiz2DError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPlatform(platform) => {
                f.write_fmt(format_args!("unsupported platform: {}", platform))
            }
            Self::SurfaceCreationError(create_surface_error) => {
                create_surface_error.fmt(f)
            }
            Self::AdapterNotFoundError(request_adapter_error) => {
                request_adapter_error.fmt(f)
            }
            Self::DeviceCreationError(request_device_error) => {
                request_device_error.fmt(f)
            }
            Self::UnsupportedSurfaceError => {
                f.write_str("the adapter does not support this surface")
            }
            Self::UnknownBackendError(backend) => {
                f.write_fmt(format_args!("unknown renderer backend: {}", backend))
            }
            Self::HeapExhaustedError { requested, largest_free_block } => {
                f.write_fmt(format_args!(
                    "heap exhausted: requested {} bytes, largest free block is {} bytes",
                    requested, largest_free_block,
                ))
            }
            Self::FrameRegionExhaustedError { requested, available } => {
                f.write_fmt(format_args!(
                    "frame region exhausted: requested {} bytes, {} bytes left in this frame's slice",
                    requested, available,
                ))
            }
            Self::DevicePollError(poll_error) => {
                poll_error.fmt(f)
            }
            Self::InvalidAllocationError => {
                f.write_str("the allocation does not belong to this heap, or was already freed")
            }
            Self::ReservedBlockKindError(kind) => {
                f.write_fmt(format_args!(
                    "block kind {} is reserved for built-in blocks; use CUSTOM_KIND_BASE or above",
                    kind,
                ))
            }
            Self::UnsupportedBlockStageError => {
                f.write_str("custom blocks cannot run in the shape stage; it runs on the CPU")
            }
            Self::ShaderCompilationError(message) => {
                f.write_fmt(format_args!("the custom block shader did not compile: {}", message))
            }
            Self::EmptySpriteSheetError => {
                f.write_str("a sprite sheet needs at least one layer with a non-zero size")
            }
            Self::SpriteSizeMismatchError { layer, expected, found } => {
                f.write_fmt(format_args!(
                    "sprite layer {} has {} bytes but the sheet needs {}",
                    layer, found, expected,
                ))
            }
            Self::FontParseError(message) => {
                f.write_fmt(format_args!("the font could not be read: {}", message))
            }
            Self::AtlasFullError { width, height, pages } => {
                f.write_fmt(format_args!(
                    "no room for a {}x{} image in the atlas ({} pages used)",
                    width, height, pages,
                ))
            }
        }
    }
}

impl Error for Gueiz2DError {}
