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
        /// 線を刻む模様。`None` なら 1 本につながる。
        ///
        /// **刻むと破片はどれも開いた折れ線になります。** 閉じた輪でも
        /// 切れ目ができるので、端の閉じ方（[`JointType`] の `Round` 系）が
        /// 破片ごとに効きます。
        dash: Option<Dash>,
    },
}

impl PaintType {
    /// 既定の設定で線を引く。
    pub fn stroke(line_width: f32) -> Self {
        Self::Stroke {
            line_width,
            joint_type: JointType::Miter,
            strip: false,
            dash: None,
        }
    }

    /// 既定の設定で破線を引く。
    ///
    /// 点線にするなら [`Dash::dots`] と `Round` の付く [`JointType`] を
    /// 直に組んでください。角を尖らせたままだと点が現れません。
    pub fn dashed(line_width: f32, dash: Dash) -> Self {
        Self::Stroke {
            line_width,
            joint_type: JointType::Miter,
            strip: false,
            dash: Some(dash),
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

    /// 両端を丸く閉じる形にする。
    ///
    /// 角の繋ぎ方と端の閉じ方を 1 つの型で表しているので、
    /// **角の繋ぎ方も丸に寄ります**。[`Dash::round_ends`] から使います。
    pub fn with_round_ends(self) -> Self {
        Self::RoundStartEnd
    }
}

/// 線を刻む模様。描く長さと空ける長さを交互に並べる。
///
/// # 点線にするには
///
/// 点は「ごく短い破線に丸い端を付けたもの」です。端を丸めないと
/// 見えないほど細い棒になるので、[`JointType::RoundStartEnd`] と
/// 組み合わせてください。[`Dash::dots`] がその形です。
///
/// ```
/// # use gueiz_2d::paint_type::{Dash, JointType, PaintType};
/// // 8 描いて 4 空ける
/// let dashed = PaintType::dashed(2.0, Dash::new(8.0, 4.0));
///
/// // 点線。端を丸めるので、線の太さが点の直径になる。
/// let dotted = PaintType::Stroke {
///     line_width: 4.0,
///     joint_type: JointType::RoundStartEnd,
///     strip: true,
///     dash: Some(Dash::dots(10.0)),
/// };
/// # let _ = (dashed, dotted);
/// ```
///
/// # 模様を動かす
///
/// [`Dash::offset`] を時間で動かすと、模様が線に沿って流れます。
/// 選択範囲の「行進するアリ」はこれです。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Dash {
    /// 描く・空けるの長さ。前から [`Dash::len`] 個だけ使う。
    pattern: [f32; MAX_DASH],
    len: u8,
    /// 模様をずらす長さ。動かすと流れる。
    pub offset: f32,
    /// 破片の端を丸めるか。
    ///
    /// 刻んでできた端は元の線の途中なので、[`JointType`] の
    /// `RoundStart` / `RoundEnd`（＝**折れ線全体**の端）では届きません。
    /// 破片ごとの端はここで決めます。
    round_ends: bool,
}

/// 模様に並べられる長さの数。
pub const MAX_DASH: usize = 8;

/// [`Dash::dots`] が使う、点 1 つぶんの長さ。間隔に対する割合。
///
/// 0 だと向きが決まらず消えてしまうので、丸い端だけが残る長さにする。
/// **割合にしてあるのは、模様の切り替わり際を丸め誤差で潰さないため。**
/// 決め打ちの長さにすると、間隔を広げたときに飲み込まれて点が消える。
const DOT_RATIO: f32 = 1e-3;

impl Dash {
    /// 描く長さと空ける長さ。いちばん短い形。
    pub fn new(on: f32, off: f32) -> Self {
        Self::pattern(&[on, off])
    }

    /// 点線。`gap` は点と点の間隔で、点の直径は線の太さになります。
    ///
    /// 点は「長さのない破線の端を丸めたもの」なので、
    /// **端の丸めは自分で立てます**。[`JointType`] は何でも構いません。
    ///
    /// `gap` は**線の太さより広く**取ってください。同じだと点どうしが接して
    /// 1 本の線に見えます。太さの 2〜3 倍が目安です。
    pub fn dots(gap: f32) -> Self {
        Self::pattern(&[gap * DOT_RATIO, gap]).round_ends(true)
    }

    /// 好きな並び。描く・空けるの順に、[`MAX_DASH`] 個まで。
    ///
    /// 奇数個なら、もう一周ぶん繰り返して偶数に均します（`[4, 2, 1]` は
    /// `[4, 2, 1, 4, 2, 1]` と同じ）。繰り返すと入りきらないときは、
    /// 入るところまでで切ります。
    ///
    /// 0 以下や数でない長さが混ざっていると**模様として使えない**ので、
    /// そのときは刻まずに 1 本の線になります。
    pub fn pattern(lengths: &[f32]) -> Self {
        let mut pattern = [0.0; MAX_DASH];
        let mut len = 0;

        // 奇数だと描くと空けるが一周ごとに入れ替わる。繰り返して均す。
        let rounds = if lengths.len().is_multiple_of(2) { 1 } else { 2 };

        for _ in 0..rounds {
            for length in lengths {
                if len == MAX_DASH {
                    break;
                }

                pattern[len] = *length;
                len += 1;
            }
        }

        // 切った結果が奇数になったら、末尾を落として偶数に戻す。
        if !len.is_multiple_of(2) {
            len -= 1;
        }

        Self {
            pattern,
            len: len as u8,
            offset: 0.0,
            round_ends: false,
        }
    }

    /// 模様をずらす。動かすと線に沿って流れる。
    pub fn offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    /// 破片ごとに端を丸める。
    ///
    /// **元の線の途中にできた端も丸まります。** [`JointType`] の
    /// `RoundStart` / `RoundEnd` は折れ線全体の端しか見ないので、
    /// 刻んだ破片の端はここでしか丸められません。
    ///
    /// いまの [`JointType`] は角の繋ぎ方と端の閉じ方を 1 つで表しているため、
    /// これを立てると**破片の角も丸まります**。
    pub fn round_ends(mut self, round_ends: bool) -> Self {
        self.round_ends = round_ends;
        self
    }

    /// 破片ごとに端を丸めるか。
    pub fn has_round_ends(&self) -> bool {
        self.round_ends
    }

    /// 使う長さの並び。
    pub fn lengths(&self) -> &[f32] {
        &self.pattern[..self.len as usize]
    }

    /// 模様ひと回りの長さ。
    pub fn period(&self) -> f32 {
        self.lengths().iter().sum()
    }

    /// 刻む模様として成り立っているか。
    ///
    /// 長さが 1 つでも 0 以下や数でないと、どこまで進んでも模様が変わらず
    /// 止まらなくなります。そういうものは刻まずに 1 本の線として描きます。
    pub fn is_usable(&self) -> bool {
        self.len >= 2
            && self.offset.is_finite()
            && self
                .lengths()
                .iter()
                .all(|length| length.is_finite() && *length > 0.0)
    }

    /// 先頭から `distance` 進んだところが、描くところか。
    ///
    /// 返るのは（描くか, 次に切り替わるまでの長さ）。
    /// **必ず正の長さを返します。** 0 を返すと、刻む側が進めなくなります。
    pub(crate) fn at(&self, distance: f32) -> (bool, f32) {
        let period = self.period();

        // 区間の切り替わり際ちょうどに立つと、丸め誤差で「あと 0.0000001」の
        // ような答えになり、刻む側がいつまでも進まなくなる。
        // ひと回りに対して無視できる長さなら、次の区間の頭に立っていると見なす。
        let epsilon = period * 1e-5;
        let mut position = (distance + self.offset).rem_euclid(period);

        for (index, length) in self.lengths().iter().enumerate() {
            if position < length - epsilon {
                return (index.is_multiple_of(2), length - position);
            }

            position = (position - length).max(0.0);
        }

        // 最後までこぼれたら、ひと回りして先頭の区間の頭。
        (true, self.pattern[0])
    }
}
