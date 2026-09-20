//! フォント・絵・自前のエフェクトをまとめて預かる。
//!
//! # 何を解いているか
//!
//! ここが無いと、使う側が 3 つの面倒を抱えます。
//!
//! | | 面倒 | ここでどうするか |
//! |---|---|---|
//! | フォント | [`Font`] はバイト列を**借りる**ので、呼ぶ側が持ち続けないといけない | 置き場が持つ。取っ手だけ配る |
//! | 絵 | [`SpriteSheet`] は差し替えるたびに作り直し。同じ絵を 2 度読みがち | 名前で引ける。2 度読まない |
//! | 自前のエフェクト | `kind` の番号を手で振って、[`Block::Custom`] と揃え続ける | **番号を自動で振る**。取っ手から引ける |
//!
//! 3 つめが特に効きます。番号を手で管理すると、増やしたときにずれて
//! **別のエフェクトが走る**という、気づきにくい壊れ方をします。
//!
//! # 使い方
//!
//! ```no_run
//! # use gueiz_2d::resource::Resources;
//! # use gueiz_2d::effect::{Block, EffectStage};
//! # use gueiz_2d::object::DrawManager;
//! # fn run(
//! #     device: &gueiz_2d::wgpu::Device,
//! #     draw_manager: &mut DrawManager,
//! # ) -> Result<(), gueiz_2d::error::Gueiz2DError> {
//! let mut resources = Resources::new();
//!
//! let arial = resources.load_font_file("arial", "C:/Windows/Fonts/arial.ttf")?;
//! let stripes = resources.add_effect(
//!     "stripes",
//!     EffectStage::Color,
//!     "return color * mix(vec4<f32>(1.0), block.color_a, step(0.5, fract(uv.x * 8.0)));",
//! );
//!
//! // 番号は置き場が振る。手で決めない。
//! let block = resources.effect_block(stripes, [0.0; 4], [1.0, 0.0, 0.0, 1.0]);
//!
//! // シェーダに差し込むのは 1 回だけ。
//! resources.apply_effects(device, draw_manager)?;
//!
//! let font = resources.font(arial).expect("読み込んである");
//! # let _ = (block, font);
//! # Ok(())
//! # }
//! ```

use std::path::Path;
use std::sync::Arc;

use gueiz_gpu::atlas::{Atlas, AtlasDescriptor, TextureRegion};
use gueiz_gpu::resource::{Handle, Store};

use crate::effect::{Block, CustomBlock, EffectStage, CUSTOM_KIND_BASE};
use crate::error::Gueiz2DError;
use crate::font::Font;
use crate::object::draw_manager::DrawManager;
use crate::sprite::SpriteSheet;

/// フォントの取っ手。
pub type FontHandle = Handle<FontData>;

/// 絵の取っ手。
///
/// シートは [`Arc`] で持ちます。[`DrawManager`] に差しても
/// **置き場から消えない**ようにするためです。
pub type SpriteSheetHandle = Handle<Arc<SpriteSheet>>;

/// 自前のエフェクトの取っ手。
pub type EffectHandle = Handle<EffectSource>;

/// 絵 1 枚の取っ手。
pub type TextureHandle = Handle<TextureEntry>;

/// 預かった絵 1 枚。
///
/// **確定する前は画素を持ち、確定したら場所だけを持ちます。**
/// 画素を抱え続けると、絵の枚数ぶんメモリが二重になります。
pub struct TextureEntry {
    width: u32,
    height: u32,
    /// 確定前だけ。確定したら捨てる。
    pixels: Option<Box<[u8]>>,
    /// 確定後だけ。どのページのどこに入ったか。
    region: Option<TextureRegion>,
}

impl TextureEntry {
    /// 元の絵の大きさ（画素）。
    pub fn size(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    /// アトラスの中の場所。まだ確定していなければ `None`。
    pub fn region(&self) -> Option<TextureRegion> {
        self.region
    }

    /// まだ GPU に上がっていないか。
    pub fn is_pending(&self) -> bool {
        self.region.is_none()
    }
}

/// フォントの中身。バイト列を持つだけ。
///
/// [`Font`] は借りものなので置き場には入れられない。こちらを預かって、
/// 引くときに [`Resources::font`] が組み立てる。
pub struct FontData {
    bytes: Box<[u8]>,
}

impl FontData {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// 自前のエフェクト 1 つ。番号は置き場が振る。
pub struct EffectSource {
    kind: u32,
    stage: EffectStage,
    body: String,
}

impl EffectSource {
    /// シェーダの `switch` に使う番号。[`Block::Custom`] に渡すもの。
    pub fn kind(&self) -> u32 {
        self.kind
    }

    pub fn stage(&self) -> EffectStage {
        self.stage
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

/// まとめて預かる置き場。
#[derive(Default)]
pub struct Resources {
    fonts: Store<FontData>,
    sheets: Store<Arc<SpriteSheet>>,
    effects: Store<EffectSource>,
    textures: Store<TextureEntry>,
    /// 絵を詰め込む先。最初に絵を預けたときに作る。
    atlas: Option<Atlas>,
    /// アトラスの作り方。絵を 1 枚でも預ける前に決めること。
    atlas_descriptor: AtlasDescriptor,
    /// まだ確定していない絵。確定の順に並べ替えるので、取っ手で持つ。
    pending_textures: Vec<TextureHandle>,
    /// 最後に [`DrawManager`] へ差したシート。
    ///
    /// 同じものを差し直すとバインドグループを組み直すだけ無駄なので、
    /// **作り直されたときだけ**差す。実行中に 1 枚ずつ足すと毎回通るところ。
    bound_sheet: Option<Arc<SpriteSheet>>,
    /// 次に振る番号。消しても戻さない（消えた番号が再利用されると、
    /// 古い [`Block::Custom`] が別のエフェクトを掴む）。
    next_kind: u32,
}

impl Resources {
    pub fn new() -> Self {
        Self {
            next_kind: CUSTOM_KIND_BASE,
            ..Default::default()
        }
    }

    // ---------------- フォント ----------------

    /// バイト列からフォントを預かる。**その場で読めるか確かめる。**
    ///
    /// 壊れたフォントを黙って預かると、使うときに初めて分かって原因を追いにくい。
    pub fn load_font(&mut self, name: &str, bytes: Vec<u8>) -> Result<FontHandle, Gueiz2DError> {
        // 読めることだけ見て、すぐ捨てる。
        Font::from_bytes(&bytes)?;

        let handle = self.fonts.insert_named(
            name,
            FontData {
                bytes: bytes.into_boxed_slice(),
            },
        );

        log::info!("font '{name}' loaded ({} bytes)", self.font_bytes(handle).len());

        Ok(handle)
    }

    /// ファイルからフォントを預かる。
    pub fn load_font_file(
        &mut self,
        name: &str,
        path: impl AsRef<Path>,
    ) -> Result<FontHandle, Gueiz2DError> {
        let path = path.as_ref();

        let bytes = std::fs::read(path).map_err(|error| {
            Gueiz2DError::FontParseError(format!("{}: {error}", path.display()))
        })?;

        self.load_font(name, bytes)
    }

    /// 預かったフォント。
    ///
    /// 取り出すたびに表を読み直します（数マイクロ秒）。**1 文字ごとではなく、
    /// 1 回の描画でまとめて**使ってください。
    pub fn font(&self, handle: FontHandle) -> Option<Font<'_>> {
        let data = self.fonts.get(handle)?;

        // 預かるときに読めることは確かめてある。
        Font::from_bytes(&data.bytes).ok()
    }

    /// 名前から引く。
    pub fn font_by_name(&self, name: &str) -> Option<Font<'_>> {
        self.font(self.fonts.handle(name)?)
    }

    pub fn font_handle(&self, name: &str) -> Option<FontHandle> {
        self.fonts.handle(name)
    }

    pub fn fonts(&self) -> &Store<FontData> {
        &self.fonts
    }

    fn font_bytes(&self, handle: FontHandle) -> &[u8] {
        self.fonts.get(handle).map_or(&[], FontData::bytes)
    }

    // ---------------- 絵 ----------------

    /// 作ってあるシートを預かる。
    pub fn add_sprite_sheet(&mut self, name: &str, sheet: SpriteSheet) -> SpriteSheetHandle {
        log::info!("sprite sheet '{name}' added ({} layers)", sheet.layer_count());

        self.sheets.insert_named(name, Arc::new(sheet))
    }

    pub fn sprite_sheet(&self, handle: SpriteSheetHandle) -> Option<&SpriteSheet> {
        self.sheets.get(handle).map(Arc::as_ref)
    }

    pub fn sprite_sheet_handle(&self, name: &str) -> Option<SpriteSheetHandle> {
        self.sheets.handle(name)
    }

    pub fn sprite_sheets(&self) -> &Store<Arc<SpriteSheet>> {
        &self.sheets
    }

    /// 預かったシートを [`DrawManager`] に差す。
    ///
    /// **置き場からは消えません。** 差したあとも同じ取っ手で引けますし、
    /// 別のシートに差し替えてから戻すこともできます。
    ///
    /// [`DrawManager`] が持てるシートは 1 枚だけなので、差した時点で
    /// 前のものは見えなくなります。消えるわけではありません。
    pub fn use_sprite_sheet(
        &mut self,
        device: &wgpu::Device,
        draw_manager: &mut DrawManager,
        handle: SpriteSheetHandle,
    ) -> bool {
        let Some(sheet) = self.sheets.get(handle).map(Arc::clone) else {
            return false;
        };

        draw_manager.set_sprite_sheet(device, sheet);
        true
    }

    // ---------------- 絵（アトラス） ----------------

    /// アトラスの作り方を決める。**絵を 1 枚でも預ける前に。**
    ///
    /// 預けたあとに変えると、すでに決まった場所と食い違うので何もしません。
    pub fn set_atlas_descriptor(&mut self, descriptor: AtlasDescriptor) -> bool {
        if self.atlas.is_some() {
            log::warn!("atlas descriptor ignored: the atlas already has images");
            return false;
        }

        self.atlas_descriptor = descriptor;
        true
    }

    pub fn atlas_descriptor(&self) -> AtlasDescriptor {
        self.atlas_descriptor
    }

    /// 詰め込んだあとのアトラス。まだ確定していなければ `None`。
    pub fn atlas(&self) -> Option<&Atlas> {
        self.atlas.as_ref()
    }

    /// 絵を預かる。**GPU は触りません。**
    ///
    /// `rgba` は `width * height * 4` バイト。**普通のアルファ**（乗算済みでない）
    /// のまま渡します。PNG を復号しただけのものがそのまま使えます。
    ///
    /// ここでは場所を決めません。[`Resources::commit_textures`] が
    /// **全部まとめて**詰め込みます。1 枚ずつ決めるより隙間が減るためです。
    pub fn load_texture(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<TextureHandle, Gueiz2DError> {
        let expected = (width as usize) * (height as usize) * 4;

        if width == 0 || height == 0 {
            return Err(Gueiz2DError::EmptySpriteSheetError);
        }

        if rgba.len() != expected {
            return Err(Gueiz2DError::SpriteSizeMismatchError {
                layer: 0,
                expected,
                found: rgba.len(),
            });
        }

        let handle = self.textures.insert_named(
            name,
            TextureEntry {
                width,
                height,
                pixels: Some(rgba.to_vec().into_boxed_slice()),
                region: None,
            },
        );

        self.pending_textures.push(handle);

        log::debug!("texture '{name}' loaded ({width}x{height})");

        Ok(handle)
    }

    /// 預かった絵をまとめて詰め込んで、GPU へ上げる。
    ///
    /// **取っ手は生きたまま**です。詰め込んだ場所は
    /// [`Resources::texture`] で引けます。上げ終わった画素は捨てるので、
    /// 同じ絵を CPU と GPU で二重に持ちません。
    ///
    /// 2 度目以降は、**前に上げたぶんはそのまま**で、増えたぶんだけを足します。
    ///
    /// ```no_run
    /// # use gueiz_2d::object::DrawManager;
    /// # use gueiz_2d::resource::Resources;
    /// # fn run(
    /// #     device: &gueiz_2d::wgpu::Device,
    /// #     queue: &gueiz_2d::wgpu::Queue,
    /// #     draw_manager: &mut DrawManager,
    /// #     grass: &[u8],
    /// #     wall: &[u8],
    /// # ) -> Result<(), gueiz_2d::error::Gueiz2DError> {
    /// let mut resources = Resources::new();
    ///
    /// let grass = resources.load_texture("grass", 64, 64, grass)?;
    /// let wall = resources.load_texture("wall", 128, 256, wall)?;
    ///
    /// // ここで初めて GPU を触る。1 回で済ませる。
    /// resources.commit_textures(device, queue, draw_manager)?;
    ///
    /// // 取っ手は生きている。
    /// let region = resources.texture(grass).expect("上げてある");
    /// # let _ = (wall, region);
    /// # Ok(())
    /// # }
    /// ```
    pub fn commit_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draw_manager: &mut DrawManager,
    ) -> Result<(), Gueiz2DError> {
        if self.pending_textures.is_empty() {
            return Ok(());
        }

        let atlas = self
            .atlas
            .get_or_insert_with(|| Atlas::new(self.atlas_descriptor));

        // 画素を取り出してから詰める。置き場を借りたままだと詰められない。
        let mut handles = Vec::with_capacity(self.pending_textures.len());
        let mut images = Vec::with_capacity(self.pending_textures.len());

        for handle in self.pending_textures.drain(..) {
            let Some(entry) = self.textures.get_mut(handle) else {
                // 確定する前に消された。詰める必要は無い。
                continue;
            };

            let Some(pixels) = entry.pixels.take() else {
                continue;
            };

            handles.push(handle);
            images.push((entry.width, entry.height, pixels));
        }

        if handles.is_empty() {
            return Ok(());
        }

        let borrowed: Vec<(u32, u32, &[u8])> = images
            .iter()
            .map(|(width, height, pixels)| (*width, *height, &pixels[..]))
            .collect();

        let regions = atlas.insert_batch(device, queue, &borrowed)?;

        for (handle, region) in handles.iter().zip(regions) {
            if let Some(entry) = self.textures.get_mut(*handle) {
                entry.region = Some(region);
            }
        }

        // シートは分け合う。差しても置き場から消えない。
        // 作り直されていなければ差さない。ページが増えなければ毎回同じもの。
        if let Some(sheet) = atlas.sheet() {
            let changed = self
                .bound_sheet
                .as_ref()
                .is_none_or(|bound| !Arc::ptr_eq(bound, sheet));

            if changed {
                draw_manager.set_sprite_sheet(device, Arc::clone(sheet));
                self.bound_sheet = Some(Arc::clone(sheet));
            }
        }

        log::info!(
            "atlas: {} images committed, {} pages, {:.0}% full",
            handles.len(),
            atlas.page_count(),
            atlas.occupancy() * 100.0,
        );

        Ok(())
    }

    /// 詰め込んだ絵の場所。まだ確定していなければ `None`。
    ///
    /// ```no_run
    /// # use gueiz_2d::object::Object;
    /// # use gueiz_2d::resource::{Resources, TextureHandle};
    /// # fn run(resources: &Resources, object: &mut Object, grass: TextureHandle) {
    /// if let Some(region) = resources.texture(grass) {
    ///     object.sprite_texture(region);
    /// }
    /// # }
    /// ```
    pub fn texture(&self, handle: TextureHandle) -> Option<TextureRegion> {
        self.textures.get(handle)?.region
    }

    /// 名前から取っ手を引く。
    pub fn texture_handle(&self, name: &str) -> Option<TextureHandle> {
        self.textures.handle(name)
    }

    /// 名前から場所を引く。
    pub fn texture_named(&self, name: &str) -> Option<TextureRegion> {
        self.texture(self.texture_handle(name)?)
    }

    pub fn textures(&self) -> &Store<TextureEntry> {
        &self.textures
    }

    /// まだ上げていない絵の数。
    pub fn pending_texture_count(&self) -> usize {
        self.pending_textures.len()
    }

    /// 絵を 1 枚捨てて、**アトラスの場所を返す。**
    ///
    /// 取っ手はその場で死にます（置き場が世代を進めるので、
    /// 古い取っ手で引いても `None`）。空いた場所は次の絵が埋めます。
    ///
    /// **貼ってある図形は勝手に剥がれません。** 消した絵を指したままの図形は、
    /// そこに入った別の絵を映します。消す前に
    /// [`crate::object::Object::no_sprite`] で剥がしてください。
    ///
    /// ```no_run
    /// # use gueiz_2d::resource::{Resources, TextureHandle};
    /// # fn run(resources: &mut Resources, old: TextureHandle) {
    /// resources.remove_texture(old);
    ///
    /// assert!(resources.texture(old).is_none());
    /// # }
    /// ```
    pub fn remove_texture(&mut self, handle: TextureHandle) -> bool {
        let Some(entry) = self.textures.remove(handle) else {
            return false;
        };

        // まだ上げていなければ、待ち行列から外すだけでよい。
        self.pending_textures.retain(|pending| *pending != handle);

        let Some(region) = entry.region else {
            return true;
        };

        match self.atlas.as_mut() {
            Some(atlas) => atlas.remove(region),
            None => false,
        }
    }

    /// 名前で捨てる。
    pub fn remove_texture_named(&mut self, name: &str) -> bool {
        match self.texture_handle(name) {
            Some(handle) => self.remove_texture(handle),
            None => false,
        }
    }

    /// 絵を全部捨てて、アトラスを空にする。
    ///
    /// **貼ってある絵は全部使えなくなります。** 出し入れを繰り返して
    /// 隙間が埋まらなくなったときに、ここからやり直します。
    pub fn clear_textures(&mut self) {
        self.textures.clear();
        self.pending_textures.clear();

        if let Some(atlas) = self.atlas.as_mut() {
            atlas.clear();
        }

        // シートも捨てたので、次は差し直す。
        self.bound_sheet = None;
    }

    // ---------------- 自前のエフェクト ----------------

    /// 自前のエフェクトを預かる。**番号は置き場が振る。**
    ///
    /// 手で振ると、増やしたときにずれて別のエフェクトが走ります。
    /// 番号は [`Resources::effect_kind`] か [`Resources::effect_block`] で引きます。
    ///
    /// [`EffectStage::Shape`] は CPU 側なので受け取りません（差しても走りません）。
    pub fn add_effect(&mut self, name: &str, stage: EffectStage, body: &str) -> EffectHandle {
        if stage == EffectStage::Shape {
            log::warn!("custom effect '{name}' is in the shape stage; it will not run on the GPU");
        }

        let kind = self.next_kind;
        // 消しても戻さない。使い回すと、古い Block::Custom が別のものを掴む。
        self.next_kind += 1;

        log::info!("custom effect '{name}' registered as kind {kind}");

        self.effects.insert_named(
            name,
            EffectSource {
                kind,
                stage,
                body: String::from(body),
            },
        )
    }

    /// そのエフェクトの番号。
    pub fn effect_kind(&self, handle: EffectHandle) -> Option<u32> {
        self.effects.get(handle).map(EffectSource::kind)
    }

    /// そのエフェクトを積むための [`Block`] を組む。番号を手で書かずに済む。
    ///
    /// 預かっていない取っ手を渡した場合は、何もしない山（番号 0）を返します。
    pub fn effect_block(
        &self,
        handle: EffectHandle,
        params: [f32; 4],
        color: [f32; 4],
    ) -> Block {
        match self.effects.get(handle) {
            Some(effect) => Block::Custom {
                stage: effect.stage(),
                kind: effect.kind(),
                params,
                color,
            },

            None => {
                log::warn!("a custom effect handle is stale; the block will do nothing");

                Block::Custom {
                    stage: EffectStage::Color,
                    kind: 0,
                    params,
                    color,
                }
            }
        }
    }

    pub fn effect_handle(&self, name: &str) -> Option<EffectHandle> {
        self.effects.handle(name)
    }

    pub fn effects(&self) -> &Store<EffectSource> {
        &self.effects
    }

    /// 預かっている自前のエフェクトを全部シェーダに差し込む。
    ///
    /// パイプラインを組み直すので、**足し終わってから 1 回**呼びます。
    pub fn apply_effects(
        &self,
        device: &wgpu::Device,
        draw_manager: &mut DrawManager,
    ) -> Result<(), Gueiz2DError> {
        let blocks: Vec<CustomBlock> = self
            .effects
            .iter()
            .map(|(_, effect)| CustomBlock {
                kind: effect.kind(),
                stage: effect.stage(),
                body: effect.body().to_string(),
            })
            .collect();

        draw_manager.set_custom_blocks(device, &blocks)
    }

    /// 全部手放す。
    pub fn clear(&mut self) {
        self.fonts.clear();
        self.sheets.clear();
        self.effects.clear();
        self.clear_textures();
        // 番号は戻さない。
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 絵を預けても、まだ GPU に上がっていない。場所も決まっていない。
    #[test]
    fn a_loaded_texture_waits_for_the_commit() {
        let mut resources = Resources::new();
        let handle = resources
            .load_texture("red", 2, 2, &[255, 0, 0, 255].repeat(4))
            .expect("預かれる");

        assert_eq!(resources.pending_texture_count(), 1);
        assert_eq!(resources.texture(handle), None, "確定前に場所があってはいけない");
        assert!(
            resources.textures().get(handle).expect("居る").is_pending(),
            "待ちのはず",
        );
        assert_eq!(resources.textures().get(handle).expect("居る").size(), [2, 2]);
    }

    /// 大きさの合わない画素は預からない。黙って受け取ると、
    /// 詰め込むときに初めて分かって原因を追いにくい。
    #[test]
    fn a_texture_with_the_wrong_byte_count_is_refused() {
        let mut resources = Resources::new();

        assert!(matches!(
            resources.load_texture("short", 4, 4, &[0; 10]),
            Err(Gueiz2DError::SpriteSizeMismatchError { .. }),
        ));
        assert!(matches!(
            resources.load_texture("empty", 0, 4, &[]),
            Err(Gueiz2DError::EmptySpriteSheetError),
        ));
        assert_eq!(resources.pending_texture_count(), 0);
    }

    #[test]
    fn textures_can_be_found_by_name() {
        let mut resources = Resources::new();
        let handle = resources
            .load_texture("grass", 1, 1, &[0, 255, 0, 255])
            .expect("預かれる");

        assert_eq!(resources.texture_handle("grass"), Some(handle));
        assert_eq!(resources.texture_handle("stone"), None);
    }

    /// 同じ名前で預け直すと入れ替わる。読み込み直しがこれで済む。
    #[test]
    fn the_same_texture_name_replaces_what_was_there() {
        let mut resources = Resources::new();
        let old = resources.load_texture("tile", 1, 1, &[1, 2, 3, 255]).expect("預かれる");
        let new = resources.load_texture("tile", 2, 2, &[0; 16]).expect("預かれる");

        assert_ne!(old, new);
        assert!(resources.textures().get(old).is_none(), "古い取っ手は死ぬ");
        assert_eq!(resources.textures().get(new).expect("居る").size(), [2, 2]);
    }

    /// 詰め方は、絵を預ける前にしか決められない。
    #[test]
    fn the_atlas_settings_are_locked_once_it_exists() {
        let mut resources = Resources::new();

        assert!(resources.set_atlas_descriptor(AtlasDescriptor {
            page_size: 512,
            ..Default::default()
        }));
        assert_eq!(resources.atlas_descriptor().page_size, 512);

        // 預けただけではまだアトラスは無い。詰めて初めてできる。
        assert!(resources.atlas().is_none());
    }

    /// 絵を捨てると、取っ手も待ち行列も空になる。
    #[test]
    fn clearing_textures_empties_everything() {
        let mut resources = Resources::new();
        let handle = resources.load_texture("a", 1, 1, &[0; 4]).expect("預かれる");

        resources.clear_textures();

        assert_eq!(resources.pending_texture_count(), 0);
        assert!(resources.textures().get(handle).is_none());
    }

    /// 上げる前に消せば、待ち行列からも外れる。
    #[test]
    fn removing_a_pending_texture_takes_it_off_the_queue() {
        let mut resources = Resources::new();
        let handle = resources.load_texture("a", 1, 1, &[0; 4]).expect("預かれる");

        assert!(resources.remove_texture(handle));

        assert_eq!(resources.pending_texture_count(), 0);
        assert!(resources.texture_handle("a").is_none(), "名前も消える");
    }

    /// 消した取っ手ではもう引けない。
    #[test]
    fn a_removed_texture_handle_stops_working() {
        let mut resources = Resources::new();
        let handle = resources.load_texture("a", 1, 1, &[0; 4]).expect("預かれる");

        resources.remove_texture(handle);

        assert!(resources.textures().get(handle).is_none());
        assert_eq!(resources.texture(handle), None);
    }

    /// 2 度消しても落ちない。2 度目は `false`。
    #[test]
    fn removing_twice_is_reported_not_fatal() {
        let mut resources = Resources::new();
        let handle = resources.load_texture("a", 1, 1, &[0; 4]).expect("預かれる");

        assert!(resources.remove_texture(handle));
        assert!(!resources.remove_texture(handle));
    }

    #[test]
    fn textures_can_be_removed_by_name() {
        let mut resources = Resources::new();
        resources.load_texture("a", 1, 1, &[0; 4]).expect("預かれる");

        assert!(resources.remove_texture_named("a"));
        assert!(!resources.remove_texture_named("a"));
        assert!(!resources.remove_texture_named("知らない"));
    }

    /// 上げる前に消された絵は、詰め込みの対象から外れる。
    /// 残すと、消えた取っ手のために場所を取ることになる。
    #[test]
    fn a_texture_removed_before_the_commit_is_dropped() {
        let mut resources = Resources::new();
        resources.load_texture("a", 1, 1, &[0; 4]).expect("預かれる");
        let doomed = resources.texture_handle("a").expect("居る");

        resources.textures.remove(doomed);

        // 待ち行列には残っているが、詰めるときに飛ばされる。
        assert_eq!(resources.pending_texture_count(), 1);
        assert_eq!(resources.textures().len(), 0);
    }

    #[test]
    fn a_fresh_store_is_empty() {
        let resources = Resources::new();

        assert!(resources.fonts().is_empty());
        assert!(resources.sprite_sheets().is_empty());
        assert!(resources.effects().is_empty());
    }

    /// 番号は自動で振られ、かぶらない。
    #[test]
    fn effect_kinds_are_assigned_in_order() {
        let mut resources = Resources::new();

        let first = resources.add_effect("a", EffectStage::Color, "return color;");
        let second = resources.add_effect("b", EffectStage::Color, "return color;");

        assert_eq!(resources.effect_kind(first), Some(CUSTOM_KIND_BASE));
        assert_eq!(resources.effect_kind(second), Some(CUSTOM_KIND_BASE + 1));
    }

    /// 番号は予約済みの範囲に入ってはいけない。入ると用意された山と衝突する。
    #[test]
    fn effect_kinds_stay_out_of_the_reserved_range() {
        let mut resources = Resources::new();

        for index in 0..8 {
            let handle = resources.add_effect(&format!("e{index}"), EffectStage::Color, "return color;");
            let kind = resources.effect_kind(handle).expect("登録した");

            assert!(kind >= CUSTOM_KIND_BASE, "{kind}");
        }
    }

    /// 消した番号を使い回してはいけない。使い回すと、古い山が
    /// 別のエフェクトを掴んで、気づきにくい壊れ方をする。
    #[test]
    fn a_removed_kind_is_never_reused() {
        let mut resources = Resources::new();

        let first = resources.add_effect("a", EffectStage::Color, "return color;");
        let first_kind = resources.effect_kind(first).expect("登録した");

        // 同じ名前で入れ替える = 前のは消える。
        let replaced = resources.add_effect("a", EffectStage::Color, "return color * 2.0;");
        let replaced_kind = resources.effect_kind(replaced).expect("登録した");

        assert_ne!(first_kind, replaced_kind, "番号が使い回されている");
        assert_eq!(resources.effect_kind(first), None, "古い取っ手は死んでいる");
    }

    /// 山を組むのに番号を手で書かない。ここが今回の主目的。
    #[test]
    fn a_block_carries_the_assigned_kind() {
        let mut resources = Resources::new();
        let handle = resources.add_effect("tint", EffectStage::Color, "return color;");

        let block = resources.effect_block(handle, [1.0, 2.0, 3.0, 4.0], [0.5; 4]);

        match block {
            Block::Custom {
                stage,
                kind,
                params,
                color,
            } => {
                assert_eq!(stage, EffectStage::Color);
                assert_eq!(kind, resources.effect_kind(handle).expect("登録した"));
                assert_eq!(params, [1.0, 2.0, 3.0, 4.0]);
                assert_eq!(color, [0.5; 4]);
            }
            other => panic!("Custom のはずが {other:?}"),
        }
    }

    /// 段も一緒に覚えている。積むときに段を書き間違えない。
    #[test]
    fn a_block_remembers_the_stage() {
        let mut resources = Resources::new();
        let handle = resources.add_effect("wobble", EffectStage::Transform, "");

        assert_eq!(
            resources.effect_block(handle, [0.0; 4], [0.0; 4]).stage(),
            EffectStage::Transform,
        );
    }

    /// 死んだ取っ手で山を組んでも落ちない。何もしない山になる。
    #[test]
    fn a_stale_handle_makes_a_block_that_does_nothing() {
        let mut resources = Resources::new();
        let handle = resources.add_effect("a", EffectStage::Color, "return color;");
        resources.clear();

        match resources.effect_block(handle, [0.0; 4], [0.0; 4]) {
            Block::Custom { kind, .. } => assert_eq!(kind, 0, "0 は何もしないに予約"),
            other => panic!("Custom のはずが {other:?}"),
        }
    }

    #[test]
    fn names_find_their_handles() {
        let mut resources = Resources::new();
        let handle = resources.add_effect("stripes", EffectStage::Color, "return color;");

        assert_eq!(resources.effect_handle("stripes"), Some(handle));
        assert_eq!(resources.effect_handle("missing"), None);
    }

    /// 壊れたバイト列は預かる時点で弾く。使うときまで黙っていると原因を追いにくい。
    #[test]
    fn a_broken_font_is_rejected_on_load() {
        let mut resources = Resources::new();

        let outcome = resources.load_font("broken", vec![0, 1, 2, 3]);

        assert!(outcome.is_err());
        assert!(resources.fonts().is_empty());
    }

    #[test]
    fn a_missing_file_is_reported() {
        let mut resources = Resources::new();

        assert!(resources.load_font_file("nope", "does/not/exist.ttf").is_err());
    }

    #[test]
    fn clearing_drops_everything() {
        let mut resources = Resources::new();
        resources.add_effect("a", EffectStage::Color, "return color;");

        resources.clear();

        assert!(resources.effects().is_empty());
        assert_eq!(resources.effect_handle("a"), None);
    }
}
