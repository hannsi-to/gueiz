//! 輪郭をどう三角形に開くかの指定。
//!
//! 塗りも線も三角形リストに開いてから積むので、描画は同じパイプラインで済む。
//! 線を `LineStrip` トポロジで描くとパイプラインが分かれてドローが増えるため、
//! ここでは線も多角形として開く。

/// 記録した頂点をどう塗るか。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub enum PaintType {
    /// 内部を塗る。頂点 0 から張るトライアングルファンにする。
    #[default]
    Fill,

    /// 輪郭を太さのある帯で描く。
    Stroke {
        /// 線の太さ。頂点と同じ単位（ピクセル座標なら px）。
        line_width: f32,
        /// 角のつなぎ方。
        joint_type: JointType,
        /// `true` なら開いた折れ線、`false` なら閉じた輪郭。
        strip: bool,
    },
}

impl PaintType {
    /// 既定の設定で線を引く。
    pub fn stroke(line_width: f32) -> Self {
        Self::Stroke {
            line_width,
            joint_type: JointType::Miter,
            strip: false,
        }
    }
}

/// 折れ線の角と端の処理。
///
/// `Round*` の 3 つは、丸い角に加えて**開いた折れ線の端**を丸く閉じる。
/// `strip` が `false`（閉じた輪郭）のときは端が無いので [`JointType::Round`] と同じ。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug, Default)]
pub enum JointType {
    /// 何も足さない。角の外側に隙間が空く。いちばん軽い。
    None,
    /// 外側の辺を延長して尖らせる。角が鋭すぎるときは
    /// [`JointType::Bevel`] に落ちる。
    #[default]
    Miter,
    /// 外側の角を三角形 1 枚で塞ぐ。
    Bevel,
    /// 外側の角を扇形で丸める。
    Round,
    /// 丸い角 + 始点を丸く閉じる。
    RoundStart,
    /// 丸い角 + 終点を丸く閉じる。
    RoundEnd,
    /// 丸い角 + 両端を丸く閉じる。
    RoundStartEnd,
}

impl JointType {
    /// 角を丸めるか。
    pub fn is_round(self) -> bool {
        matches!(
            self,
            Self::Round | Self::RoundStart | Self::RoundEnd | Self::RoundStartEnd
        )
    }

    /// 開いた折れ線の始点を丸く閉じるか。
    pub fn caps_start(self) -> bool {
        matches!(self, Self::RoundStart | Self::RoundStartEnd)
    }

    /// 開いた折れ線の終点を丸く閉じるか。
    pub fn caps_end(self) -> bool {
        matches!(self, Self::RoundEnd | Self::RoundStartEnd)
    }
}
