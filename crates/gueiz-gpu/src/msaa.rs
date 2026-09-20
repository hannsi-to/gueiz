//! 縁のギザギザを均す（マルチサンプル）。2D と 3D で共通。
//!
//! # 何が起きているか
//!
//! 三角形を塗るとき、GPU は画素の中心が三角形の内側かどうかだけを見ます。
//! つまり縁の画素は**塗るか塗らないかの二択**で、中間の明るさが出ません。
//! 斜めや曲線の縁が階段状に見えるのはこれが原因です。
//!
//! マルチサンプルは、1 画素の中を複数点で判定して平均を取ります。
//! 4 点なら縁の画素が 0 / 25 / 50 / 75 / 100% の 5 段階になり、階段が目立たなくなります。
//!
//! # 値段
//!
//! 描き先が `sample_count` 倍のメモリを使い、その帯域も掛かります。
//! 4 サンプルなら 1920x1080 の描き先が 8.3 MB から 33 MB になります。
//! フラグメントシェーダは**縁の画素以外では 1 回しか走らない**ので、
//! 計算が 4 倍になるわけではありません。
//!
//! # 使い方
//!
//! 描く側（パイプライン）と描き先（アタッチメント）で**同じ数**にします。
//! 食い違うと wgpu が弾きます。
//!
//! ```ignore
//! // パイプラインを組むとき
//! multisample: wgpu::MultisampleState { count: sample_count, ..Default::default() },
//!
//! // 描くとき
//! target.ensure(device, width, height, format);
//! let attachment = target.color_attachment(&surface_view, load);
//! ```

/// ギザギザを均さない。1 画素につき 1 点。
pub const NO_MULTISAMPLE: u32 = 1;

/// どこでも使える無難な値。4 点。
pub const DEFAULT_MULTISAMPLE: u32 = 4;

/// マルチサンプルの描き先。大きさや形式が変わったら作り直す。
///
/// `sample_count` が 1 のときは何も確保しません。**そのまま描けばよい**ので、
/// 使わない人に費用が掛かりません。
pub struct MultisampleTarget {
    sample_count: u32,
    current: Option<Current>,
}

struct Current {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

impl MultisampleTarget {
    /// `sample_count` は 1 か 4。1 なら何もしない。
    pub fn new(sample_count: u32) -> Self {
        Self {
            sample_count: sample_count.max(1),
            current: None,
        }
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// 均すかどうか。
    pub fn is_enabled(&self) -> bool {
        self.sample_count > 1
    }

    /// 描き先を用意する。大きさや形式が変わっていたら作り直す。
    pub fn ensure(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        if !self.is_enabled() {
            return;
        }

        let fits = self.current.as_ref().is_some_and(|current| {
            current.width == width && current.height == height && current.format == format
        });

        if fits {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gueiz multisample target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: self.sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            // 均した結果を書き出すだけなので、読み出しには使わない。
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        log::debug!(
            "multisample target: {width}x{height} {format:?} x{}",
            self.sample_count,
        );

        self.current = Some(Current {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            width,
            height,
            format,
        });
    }

    /// 色のアタッチメントを組む。
    ///
    /// 均すときは、多点の描き先に描いて `resolve` へ書き出します。
    /// 均さないときは `resolve` に直接描きます。**呼ぶ側は同じ書き方で済みます。**
    ///
    /// [`MultisampleTarget::ensure`] を先に呼んでおくこと。
    pub fn color_attachment<'a>(
        &'a self,
        resolve: &'a wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPassColorAttachment<'a> {
        match self.current.as_ref().filter(|_| self.is_enabled()) {
            Some(current) => wgpu::RenderPassColorAttachment {
                view: &current.view,
                depth_slice: None,
                resolve_target: Some(resolve),
                ops: wgpu::Operations {
                    load,
                    // 均した結果は `resolve` に残るので、多点のほうは捨ててよい。
                    store: wgpu::StoreOp::Discard,
                },
            },

            None => wgpu::RenderPassColorAttachment {
                view: resolve,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sample_means_no_multisampling() {
        let target = MultisampleTarget::new(1);

        assert!(!target.is_enabled());
        assert_eq!(target.sample_count(), 1);
    }

    #[test]
    fn four_samples_are_enabled() {
        let target = MultisampleTarget::new(DEFAULT_MULTISAMPLE);

        assert!(target.is_enabled());
        assert_eq!(target.sample_count(), 4);
    }

    /// 0 を渡されても 1 として扱う。0 サンプルのパイプラインは作れない。
    #[test]
    fn zero_is_treated_as_one() {
        assert_eq!(MultisampleTarget::new(0).sample_count(), 1);
    }

    /// 均さないときは何も確保しない。使わない人に費用が掛かってはいけない。
    #[test]
    fn nothing_is_allocated_when_disabled() {
        let target = MultisampleTarget::new(NO_MULTISAMPLE);

        assert!(target.current.is_none());
    }
}
