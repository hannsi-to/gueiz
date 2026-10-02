//! ウィジェットを入れる木。
//!
//! # 入れ子の所有ではなく、平らな枠と鍵
//!
//! 「親が子を `Vec<Box<dyn Widget>>` で持つ」形は素直ですが、
//! 実際に組むと次で詰まります。
//!
//! - **親を触りながら子を触れない。** 親の `&mut` を握ったまま子へ降りられないので、
//!   レイアウトのように「親に聞いて子に渡す」周回が書けません。
//! - **焦点と掴みが置けない。** 「いま押されているウィジェット」は
//!   フレームをまたいで覚える必要があり、所有の木では指す手段がありません。
//! - **外から掴めない。** コードの側から「音量つまみ」を触るたびに
//!   木を辿り直すことになります。
//!
//! そこで**ウィジェットは平らな枠に並べ、親子は [`WidgetId`] のつながりで持ちます。**
//! 枠の中身は変わっても鍵は変わらないので、焦点も掴みも鍵 1 つで覚えられます。
//!
//! # ウィジェットは周回のあいだ枠から抜ける
//!
//! [`Widget::measure`](crate::gui::Widget::measure) には木への `&mut` を渡します。
//! 子へ降りるために要るのですが、ウィジェット自身が木の中にいるので
//! そのままでは二重に借りることになります。
//!
//! そこで**呼ぶ直前に枠から抜き、終わったら戻します**
//! （[`WidgetTree::take_widget`] / [`WidgetTree::put_widget`]）。
//! 抜けているあいだ [`WidgetTree::get`] は `None` を返します。
//! つまり**自分自身を木から引こうとしても取れません。** 自分のことは
//! `self` で分かるので、これで困ることはありません。
//!
//! # 測り直しの印は根まで上げる
//!
//! 子の大きさが変われば親の大きさも変わりうるので、
//! [`WidgetTree::request_layout`] は印を**先祖すべて**に立てます。
//! 立っていない枝はそのフレームで測り直されません。

use fxhash::FxHashMap;
use std::num::NonZeroU32;

use crate::gui::geometry::{Point, Rect, Size};
use crate::gui::id::{Tag, WidgetId};
use crate::gui::layout::LayoutCache;
use crate::gui::widget::{Behavior, Widget, WidgetState};

/// 枠 1 つ。
enum Slot {
    /// 空き。次に入るときはこの世代を使う。
    Vacant { generation: NonZeroU32 },
    Occupied { generation: NonZeroU32, node: Node },
}

impl Slot {
    fn generation(&self) -> NonZeroU32 {
        match self {
            Self::Vacant { generation } | Self::Occupied { generation, .. } => *generation,
        }
    }
}

/// ウィジェット 1 つぶんの持ち物。
pub struct Node {
    /// 周回のあいだだけ抜ける。
    widget: Option<Box<dyn Widget>>,
    parent: WidgetId,
    children: Vec<WidgetId>,
    tag: Option<Tag>,

    /// **画面の左上を原点とした**矩形。[`WidgetTree::arrange_done`] が書く。
    bounds: Rect,
    /// 直近の [`Widget::measure`](crate::gui::Widget::measure) の答え。
    desired: Size,
    cache: LayoutCache,

    state: WidgetState,
    behavior: Behavior,

    /// 測り直しが要る。
    needs_layout: bool,
}

impl Node {
    /// 画面上の矩形。
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// 画面上の左上。
    pub fn origin(&self) -> Point {
        self.bounds.origin()
    }

    pub fn size(&self) -> Size {
        self.bounds.size()
    }

    /// 直近に測った「欲しい大きさ」。
    pub fn desired(&self) -> Size {
        self.desired
    }

    pub fn parent(&self) -> WidgetId {
        self.parent
    }

    pub fn children(&self) -> &[WidgetId] {
        &self.children
    }

    pub fn tag(&self) -> Option<Tag> {
        self.tag
    }

    pub fn state(&self) -> WidgetState {
        self.state
    }

    pub fn behavior(&self) -> Behavior {
        self.behavior
    }

    pub fn needs_layout(&self) -> bool {
        self.needs_layout
    }

    /// 中のウィジェット。周回中は `None`。
    pub fn widget(&self) -> Option<&dyn Widget> {
        self.widget.as_deref()
    }

    pub fn widget_mut(&mut self) -> Option<&mut (dyn Widget + 'static)> {
        self.widget.as_deref_mut()
    }
}

/// ウィジェットの木。
pub struct WidgetTree {
    slots: Vec<Slot>,
    /// 空いた枠の番号。後ろから使う。
    free: Vec<u32>,
    root: WidgetId,
    tags: FxHashMap<Tag, WidgetId>,
}

impl Default for WidgetTree {
    fn default() -> Self {
        Self::new()
    }
}

impl WidgetTree {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            root: WidgetId::NONE,
            tags: FxHashMap::default(),
        }
    }

    /// 根。まだ無ければ [`WidgetId::NONE`]。
    pub fn root(&self) -> WidgetId {
        self.root
    }

    /// 入っているウィジェットの数。
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 根を据える。すでに根があれば**古い木ごと捨てます**。
    pub fn set_root(&mut self, widget: Box<dyn Widget>) -> WidgetId {
        if self.root.is_some() {
            self.remove(self.root);
        }

        let id = self.allocate(widget, WidgetId::NONE);
        self.root = id;

        id
    }

    /// 子として足す。末尾に付きます。
    ///
    /// `parent` が無効なら何も足さず [`WidgetId::NONE`] を返します。
    pub fn add_child(&mut self, parent: WidgetId, widget: Box<dyn Widget>) -> WidgetId {
        if !self.is_valid(parent) {
            log::warn!("add_child to a stale parent {parent}; ignored");
            return WidgetId::NONE;
        }

        let id = self.allocate(widget, parent);

        if let Some(node) = self.node_mut(parent) {
            node.children.push(id);
        }

        self.request_layout(parent);

        id
    }

    /// `index` の位置に差し込む。範囲を超えていれば末尾。
    pub fn insert_child(
        &mut self,
        parent: WidgetId,
        index: usize,
        widget: Box<dyn Widget>,
    ) -> WidgetId {
        if !self.is_valid(parent) {
            log::warn!("insert_child to a stale parent {parent}; ignored");
            return WidgetId::NONE;
        }

        let id = self.allocate(widget, parent);

        if let Some(node) = self.node_mut(parent) {
            let index = index.min(node.children.len());
            node.children.insert(index, id);
        }

        self.request_layout(parent);

        id
    }

    /// 消す。**子孫もまとめて消えます。**
    ///
    /// 消えた枠を指していた古い鍵は、以後 [`WidgetTree::get`] で `None` になります。
    pub fn remove(&mut self, id: WidgetId) {
        if !self.is_valid(id) {
            return;
        }

        // 親の子一覧から外す。先にやらないと、消えた鍵が残る。
        let parent = self.node(id).map(|node| node.parent).unwrap_or(WidgetId::NONE);

        if let Some(node) = self.node_mut(parent) {
            node.children.retain(|child| *child != id);
        }

        if parent.is_some() {
            self.request_layout(parent);
        }

        if self.root == id {
            self.root = WidgetId::NONE;
        }

        self.remove_subtree(id);
    }

    /// 子を全部消す。自分は残る。
    pub fn clear_children(&mut self, id: WidgetId) {
        let children = match self.node_mut(id) {
            Some(node) => std::mem::take(&mut node.children),
            None => return,
        };

        for child in children {
            self.remove_subtree(child);
        }

        self.request_layout(id);
    }

    pub fn is_valid(&self, id: WidgetId) -> bool {
        self.node(id).is_some()
    }

    pub fn get(&self, id: WidgetId) -> Option<&Node> {
        self.node(id)
    }

    pub fn get_mut(&mut self, id: WidgetId) -> Option<&mut Node> {
        self.node_mut(id)
    }

    /// 具体型で引く。
    pub fn get_as<W: Widget>(&self, id: WidgetId) -> Option<&W> {
        self.node(id)?.widget()?.downcast_ref::<W>()
    }

    /// 具体型で引いて書き換える。
    ///
    /// **形が変わるような書き換えをしたら
    /// [`WidgetTree::request_layout`] を呼んでください。**
    /// 呼ばないと古い大きさのまま出ます。
    pub fn get_as_mut<W: Widget>(&mut self, id: WidgetId) -> Option<&mut W> {
        self.node_mut(id)?.widget_mut()?.downcast_mut::<W>()
    }

    pub fn parent(&self, id: WidgetId) -> WidgetId {
        self.node(id).map(|node| node.parent).unwrap_or(WidgetId::NONE)
    }

    pub fn children(&self, id: WidgetId) -> &[WidgetId] {
        self.node(id).map(Node::children).unwrap_or(&[])
    }

    /// 自分を含めず、親から根まで。
    pub fn ancestors(&self, id: WidgetId) -> impl Iterator<Item = WidgetId> + '_ {
        let mut current = self.parent(id);

        std::iter::from_fn(move || {
            if current.is_none() {
                return None;
            }

            let yielded = current;
            current = self.parent(current);

            Some(yielded)
        })
    }

    /// 札を付ける。同じ札を付け直すと古いほうの結び付きは消えます。
    pub fn set_tag(&mut self, id: WidgetId, tag: Tag) {
        if !self.is_valid(id) {
            return;
        }

        if let Some(old) = self.node_mut(id).and_then(|node| node.tag.replace(tag)) {
            // 付け替えなら、古い札の結び付きは消す。
            self.tags.remove(&old);
        }

        self.tags.insert(tag, id);
    }

    /// 札から引く。
    pub fn find(&self, tag: Tag) -> Option<WidgetId> {
        let id = *self.tags.get(&tag)?;

        // 消えたあとに同じ札が残っていることがあるので、生きているかを見る。
        self.is_valid(id).then_some(id)
    }

    /// 測り直しが要ると印を立てる。**先祖すべてに立ちます。**
    pub fn request_layout(&mut self, id: WidgetId) {
        let mut current = id;

        while current.is_some() {
            let Some(node) = self.node_mut(current) else {
                break;
            };

            // すでに立っているなら、その上も立っている。
            if node.needs_layout {
                break;
            }

            node.needs_layout = true;
            node.cache.invalidate();

            current = node.parent;
        }
    }

    /// 木のどこかに測り直しが要るか。
    pub fn needs_layout(&self) -> bool {
        self.node(self.root).is_some_and(Node::needs_layout)
    }

    pub fn state(&self, id: WidgetId) -> WidgetState {
        self.node(id).map(Node::state).unwrap_or_default()
    }

    /// 状態を書き換える。[`crate::gui::Gui`] が呼びます。
    pub(crate) fn set_state(&mut self, id: WidgetId, state: WidgetState) {
        if let Some(node) = self.node_mut(id) {
            node.state = state;
        }
    }

    /// 無効かどうかを切り替える。無効なウィジェットは入力を受け取りません。
    pub fn set_disabled(&mut self, id: WidgetId, disabled: bool) {
        if let Some(node) = self.node_mut(id) {
            node.state.disabled = disabled;

            if disabled {
                // 無効にした瞬間に触られていた印を落とす。
                node.state.hovered = false;
                node.state.pressed = false;
                node.state.focused = false;
            }
        }
    }

    /// 画面上の矩形。
    pub fn bounds(&self, id: WidgetId) -> Rect {
        self.node(id).map(Node::bounds).unwrap_or(Rect::ZERO)
    }

    /// 周回のためにウィジェットを抜く。抜けているあいだ
    /// [`WidgetTree::get`] の `widget()` は `None`。
    pub(crate) fn take_widget(&mut self, id: WidgetId) -> Option<Box<dyn Widget>> {
        self.node_mut(id)?.widget.take()
    }

    /// 抜いたウィジェットを戻す。
    pub(crate) fn put_widget(&mut self, id: WidgetId, widget: Box<dyn Widget>) {
        match self.node_mut(id) {
            Some(node) => {
                // 戻すついでに扱いを取り直す。中身の状態で変わることがある。
                node.behavior = widget.behavior();
                node.widget = Some(widget);
            }
            // 周回のあいだに消された。戻す先が無いので捨てる。
            None => log::debug!("widget {id} was removed while in flight; dropped"),
        }
    }

    /// 測った結果を覚える。
    pub(crate) fn measure_done(&mut self, id: WidgetId, size: Size) {
        if let Some(node) = self.node_mut(id) {
            node.desired = size;
        }
    }

    /// 置き場所を覚え、印を落とす。`bounds` は**画面座標**。
    pub(crate) fn arrange_done(&mut self, id: WidgetId, bounds: Rect) {
        if let Some(node) = self.node_mut(id) {
            node.bounds = bounds;
            node.needs_layout = false;
        }
    }

    pub(crate) fn cache(&self, id: WidgetId) -> LayoutCache {
        self.node(id).map(|node| node.cache).unwrap_or_default()
    }

    pub(crate) fn set_cache(&mut self, id: WidgetId, cache: LayoutCache) {
        if let Some(node) = self.node_mut(id) {
            node.cache = cache;
        }
    }

    fn allocate(&mut self, widget: Box<dyn Widget>, parent: WidgetId) -> WidgetId {
        let behavior = widget.behavior();

        let node = Node {
            widget: Some(widget),
            parent,
            children: Vec::new(),
            tag: None,
            bounds: Rect::ZERO,
            desired: Size::ZERO,
            cache: LayoutCache::new(),
            state: WidgetState::default(),
            behavior,
            // 入れたばかりなので、まだ測っていない。
            needs_layout: true,
        };

        match self.free.pop() {
            Some(index) => {
                let generation = self.slots[index as usize].generation();
                self.slots[index as usize] = Slot::Occupied { generation, node };

                WidgetId::new(index, generation)
            }
            None => {
                let index = self.slots.len() as u32;
                let generation = NonZeroU32::new(1).expect("1 is not zero");

                self.slots.push(Slot::Occupied { generation, node });

                WidgetId::new(index, generation)
            }
        }
    }

    /// 自分と子孫を枠から外す。親の子一覧は触らない。
    fn remove_subtree(&mut self, id: WidgetId) {
        let Some(node) = self.node(id) else {
            return;
        };

        // 借用を切るために写す。木は深くないので写しは安い。
        let children = node.children.clone();
        let tag = node.tag;

        for child in children {
            self.remove_subtree(child);
        }

        if let Some(tag) = tag {
            // 同じ札が別のウィジェットに付け替わっていることがある。
            if self.tags.get(&tag) == Some(&id) {
                self.tags.remove(&tag);
            }
        }

        let index = id.index();

        // 世代を進める。これで古い鍵が合わなくなる。
        let next = self.slots[index]
            .generation()
            .checked_add(1)
            // 42 億回使い回したら、そこから先は使わない枠にする。
            .unwrap_or(NonZeroU32::MAX);

        self.slots[index] = Slot::Vacant { generation: next };

        if next != NonZeroU32::MAX {
            self.free.push(index as u32);
        }
    }

    fn node(&self, id: WidgetId) -> Option<&Node> {
        let generation = id.generation()?;

        match self.slots.get(id.index())? {
            Slot::Occupied {
                generation: slot,
                node,
            } if *slot == generation => Some(node),
            _ => None,
        }
    }

    fn node_mut(&mut self, id: WidgetId) -> Option<&mut Node> {
        let generation = id.generation()?;

        match self.slots.get_mut(id.index())? {
            Slot::Occupied {
                generation: slot,
                node,
            } if *slot == generation => Some(node),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe(u32);
    impl Widget for Probe {}

    fn tree_with_root() -> (WidgetTree, WidgetId) {
        let mut tree = WidgetTree::new();
        let root = tree.set_root(Box::new(Probe(0)));

        (tree, root)
    }

    #[test]
    fn children_hang_off_the_parent() {
        let (mut tree, root) = tree_with_root();

        let a = tree.add_child(root, Box::new(Probe(1)));
        let b = tree.add_child(root, Box::new(Probe(2)));

        assert_eq!(tree.children(root), &[a, b]);
        assert_eq!(tree.parent(a), root);
        assert_eq!(tree.len(), 3);
    }

    #[test]
    fn insert_child_respects_the_index() {
        let (mut tree, root) = tree_with_root();

        let a = tree.add_child(root, Box::new(Probe(1)));
        let b = tree.insert_child(root, 0, Box::new(Probe(2)));
        let c = tree.insert_child(root, 99, Box::new(Probe(3)));

        assert_eq!(tree.children(root), &[b, a, c]);
    }

    #[test]
    fn removing_takes_the_whole_subtree() {
        let (mut tree, root) = tree_with_root();

        let parent = tree.add_child(root, Box::new(Probe(1)));
        let child = tree.add_child(parent, Box::new(Probe(2)));
        let grandchild = tree.add_child(child, Box::new(Probe(3)));

        tree.remove(parent);

        assert!(!tree.is_valid(parent));
        assert!(!tree.is_valid(child));
        assert!(!tree.is_valid(grandchild));
        assert_eq!(tree.children(root), &[]);
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn a_stale_id_never_points_at_the_replacement() {
        let (mut tree, root) = tree_with_root();

        let old = tree.add_child(root, Box::new(Probe(1)));
        tree.remove(old);

        // 枠は使い回されるが、世代が違うので古い鍵では引けない。
        let new = tree.add_child(root, Box::new(Probe(2)));

        assert_eq!(old.index(), new.index(), "枠は使い回される");
        assert_ne!(old, new);
        assert!(!tree.is_valid(old));
        assert!(tree.is_valid(new));
    }

    #[test]
    fn layout_flag_climbs_to_the_root() {
        let (mut tree, root) = tree_with_root();

        let middle = tree.add_child(root, Box::new(Probe(1)));
        let leaf = tree.add_child(middle, Box::new(Probe(2)));

        // 入れたぶんの印を落としておく。
        for id in [root, middle, leaf] {
            tree.arrange_done(id, Rect::ZERO);
        }
        assert!(!tree.needs_layout());

        tree.request_layout(leaf);

        assert!(tree.get(leaf).unwrap().needs_layout());
        assert!(tree.get(middle).unwrap().needs_layout());
        assert!(tree.needs_layout(), "根まで上がる");
    }

    #[test]
    fn ancestors_walk_up_to_the_root() {
        let (mut tree, root) = tree_with_root();

        let middle = tree.add_child(root, Box::new(Probe(1)));
        let leaf = tree.add_child(middle, Box::new(Probe(2)));

        let chain: Vec<_> = tree.ancestors(leaf).collect();

        assert_eq!(chain, vec![middle, root]);
        assert_eq!(tree.ancestors(root).count(), 0);
    }

    #[test]
    fn tags_find_widgets_and_die_with_them() {
        let (mut tree, root) = tree_with_root();
        let tag = Tag::new("volume");

        let slider = tree.add_child(root, Box::new(Probe(1)));
        tree.set_tag(slider, tag);

        assert_eq!(tree.find(tag), Some(slider));

        tree.remove(slider);
        assert_eq!(tree.find(tag), None);
    }

    #[test]
    fn downcasting_through_the_tree() {
        let (mut tree, root) = tree_with_root();
        let probe = tree.add_child(root, Box::new(Probe(7)));

        assert_eq!(tree.get_as::<Probe>(probe).unwrap().0, 7);

        tree.get_as_mut::<Probe>(probe).unwrap().0 = 9;
        assert_eq!(tree.get_as::<Probe>(probe).unwrap().0, 9);
    }

    #[test]
    fn a_widget_in_flight_cannot_be_reached() {
        let (mut tree, root) = tree_with_root();

        let taken = tree.take_widget(root).expect("root has a widget");

        assert!(tree.is_valid(root), "枠は残っている");
        assert!(tree.get(root).unwrap().widget().is_none(), "中身は抜けている");
        assert!(tree.get_as::<Probe>(root).is_none());

        tree.put_widget(root, taken);
        assert!(tree.get_as::<Probe>(root).is_some());
    }

    #[test]
    fn disabling_clears_the_touch_flags() {
        let (mut tree, root) = tree_with_root();

        tree.set_state(
            root,
            WidgetState {
                hovered: true,
                pressed: true,
                focused: true,
                disabled: false,
            },
        );

        tree.set_disabled(root, true);

        let state = tree.state(root);
        assert!(state.disabled);
        assert!(state.is_idle(), "無効にしたら触られていない扱い");
    }

    #[test]
    fn setting_a_new_root_drops_the_old_tree() {
        let (mut tree, root) = tree_with_root();
        let child = tree.add_child(root, Box::new(Probe(1)));

        let fresh = tree.set_root(Box::new(Probe(2)));

        assert!(!tree.is_valid(root));
        assert!(!tree.is_valid(child));
        assert_eq!(tree.root(), fresh);
        assert_eq!(tree.len(), 1);
    }
}
