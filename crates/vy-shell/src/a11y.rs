//! Accessibility (SPEC-10.2.1, SPEC-10.3.1, SPEC-10.4.1): the page's
//! semantic tree and the chrome's own address field, exposed to the
//! platform's accessibility API through AccessKit. Everything here is a
//! pure conversion from a [`Snapshot`] of the shell to an
//! [`accesskit::TreeUpdate`], and from an [`accesskit::ActionRequest`] to
//! what the shell should do, so it runs and is tested without a window;
//! `window.rs` owns the platform adapter and pushes an update only when
//! [`Snapshot::digest`] changed.
//!
//! The tree: the window (root) holds the address field and the page (a
//! document named after the page, clipping its children to the page area
//! as SPEC-10.1.1 asks the host to). The page holds the module's nodes as
//! the module nested them, with one rule that makes any tree a page sends
//! well-formed: a node is the child of its `parent_id` only when that node
//! comes earlier in document order, else of the page. Cycles, unknown
//! parents and self-parents therefore cannot detach anything from the root.
//! A page without a tree holds one label saying so ("otillgänglig").

use crate::chrome::Rect;
use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use vy_abi::role;
use vy_runtime::SemNode;

/// The window, the tree's root.
pub const WINDOW_ID: NodeId = NodeId(1);
/// The address field.
pub const ADDRESS_ID: NodeId = NodeId(2);
/// The page area.
pub const PAGE_ID: NodeId = NodeId(3);
/// The label shown for a page without a tree (SPEC-10.4.1).
pub const INACCESSIBLE_ID: NodeId = NodeId(4);
/// Semantic nodes live above this: the module's `i32` id in the low bits.
const SEM_BASE: u64 = 1 << 32;

/// What the platform is told about a semantic node with module id `id`.
pub fn node_id(id: i32) -> NodeId {
    NodeId(SEM_BASE | u64::from(id as u32))
}

/// The module id of a semantic node's [`NodeId`], if it is one.
pub fn sem_id(id: NodeId) -> Option<i32> {
    (id.0 >> 32 == 1).then_some(id.0 as u32 as i32)
}

/// The AccessKit role for a role of SPEC-A.3.1.
pub fn role_of(r: i32) -> Role {
    match r {
        role::TEXT => Role::Label,
        role::HEADING => Role::Heading,
        role::LINK => Role::Link,
        role::BUTTON => Role::Button,
        role::TEXTBOX => Role::TextInput,
        role::CHECKBOX => Role::CheckBox,
        role::RADIO => Role::RadioButton,
        role::LIST => Role::List,
        role::LISTITEM => Role::ListItem,
        role::IMAGE => Role::Image,
        role::TABLE => Role::Table,
        role::ROW => Role::Row,
        role::CELL => Role::Cell,
        role::NAV => Role::Navigation,
        role::MAIN => Role::Main,
        role::REGION => Role::Region,
        role::DIALOG => Role::Dialog,
        role::MENU => Role::Menu,
        role::MENUITEM => Role::MenuItem,
        role::TAB => Role::Tab,
        role::TABPANEL => Role::TabPanel,
        role::PROGRESS => Role::ProgressIndicator,
        role::SLIDER => Role::Slider,
        role::SEPARATOR => Role::Splitter,
        _ => Role::GenericContainer,
    }
}

/// What the shell shows, as the accessibility tree needs it. Coordinates
/// are logical window pixels (chrome) and the page's own logical pixels
/// (`sem`); `scale` makes them physical, which is what the platform gets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// The window title.
    pub title: String,
    /// Window width, logical pixels.
    pub width: f32,
    /// Window height.
    pub height: f32,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// The address field's rectangle.
    pub address_rect: Rect,
    /// What the address field shows.
    pub address: String,
    /// True while the address field has keyboard focus.
    pub address_focused: bool,
    /// The page area.
    pub page: Rect,
    /// The page's font scale (Ctrl+Plus/Minus).
    pub zoom: f32,
    /// The page's name (the manifest's, or the address).
    pub page_title: String,
    /// The page's tree, in document order (SPEC-10.1.1).
    pub sem: Vec<SemNode>,
    /// The node with keyboard focus (SPEC-10.3.1).
    pub focus: Option<i32>,
    /// The page runs without a tree (SPEC-10.4.1).
    pub inaccessible: bool,
}

impl Snapshot {
    /// A digest of everything the tree is made from: the window pushes an
    /// update only when it changes.
    pub fn digest(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.title.hash(&mut h);
        self.address.hash(&mut h);
        self.address_focused.hash(&mut h);
        self.page_title.hash(&mut h);
        self.focus.hash(&mut h);
        self.inaccessible.hash(&mut h);
        for v in [self.width, self.height, self.scale, self.zoom] {
            v.to_bits().hash(&mut h);
        }
        for r in [self.address_rect, self.page] {
            for v in [r.x, r.y, r.w, r.h] {
                v.to_bits().hash(&mut h);
            }
        }
        self.sem.len().hash(&mut h);
        for n in &self.sem {
            (n.id, n.parent_id, n.role).hash(&mut h);
            for v in [n.x, n.y, n.w, n.h] {
                v.to_bits().hash(&mut h);
            }
            n.text.hash(&mut h);
        }
        h.finish()
    }
}

/// A rectangle in physical window pixels, or nothing for one that is not
/// finite.
fn bounds(x: f32, y: f32, w: f32, h: f32, scale: f32) -> Option<accesskit::Rect> {
    let v = [x, y, w, h, scale];
    v.iter().all(|f| f.is_finite()).then(|| {
        let s = f64::from(scale);
        accesskit::Rect::new(
            f64::from(x) * s,
            f64::from(y) * s,
            f64::from(x + w.max(0.0)) * s,
            f64::from(y + h.max(0.0)) * s,
        )
    })
}

/// The full tree for `snap` (AccessKit's proxy mode wants a full tree in
/// every update).
pub fn tree_update(snap: &Snapshot) -> TreeUpdate {
    let mut nodes = Vec::with_capacity(snap.sem.len() + 4);
    let mut window = Node::new(Role::Window);
    window.set_label(snap.title.clone());
    if let Some(b) = bounds(0.0, 0.0, snap.width, snap.height, snap.scale) {
        window.set_bounds(b);
    }
    window.set_children(vec![ADDRESS_ID, PAGE_ID]);
    nodes.push((WINDOW_ID, window));

    let mut address = Node::new(Role::TextInput);
    address.set_label("Adress");
    address.set_value(snap.address.clone());
    let a = snap.address_rect;
    if let Some(b) = bounds(a.x, a.y, a.w, a.h, snap.scale) {
        address.set_bounds(b);
    }
    address.add_action(Action::Focus);
    address.add_action(Action::Click);
    nodes.push((ADDRESS_ID, address));

    let mut page = Node::new(Role::Document);
    page.set_label(snap.page_title.clone());
    let p = snap.page;
    if let Some(b) = bounds(p.x, p.y, p.w, p.h, snap.scale) {
        page.set_bounds(b);
    }
    // The tree holds nodes outside the window too (SPEC-10.1.1); the page
    // clips them to its area for the platform.
    page.set_clips_children();

    // Parents: only a node earlier in document order (see the module doc).
    let index: HashMap<i32, usize> = snap
        .sem
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let mut children: Vec<Vec<NodeId>> = vec![Vec::new(); snap.sem.len()];
    let mut top = Vec::new();
    for (i, n) in snap.sem.iter().enumerate() {
        match index.get(&n.parent_id) {
            Some(&pi) if n.parent_id != 0 && pi < i => children[pi].push(node_id(n.id)),
            _ => top.push(node_id(n.id)),
        }
    }
    if snap.inaccessible {
        top.push(INACCESSIBLE_ID);
        let mut label = Node::new(Role::Label);
        label.set_value("Sidan har inget semantiskt träd (otillgänglig).");
        nodes.push((INACCESSIBLE_ID, label));
    }
    page.set_children(top);
    nodes.push((PAGE_ID, page));

    for (n, kids) in snap.sem.iter().zip(children) {
        let r = role_of(n.role);
        let mut node = Node::new(r);
        if !n.text.is_empty() {
            match r {
                Role::Label | Role::TextInput => node.set_value(n.text.clone()),
                _ => node.set_label(n.text.clone()),
            }
        }
        if let Some(b) = bounds(
            p.x + n.x * snap.zoom,
            p.y + n.y * snap.zoom,
            n.w * snap.zoom,
            n.h * snap.zoom,
            snap.scale,
        ) {
            node.set_bounds(b);
        }
        if role::is_focusable(n.role) {
            node.add_action(Action::Focus);
            node.add_action(Action::Click);
        }
        if !kids.is_empty() {
            node.set_children(kids);
        }
        nodes.push((node_id(n.id), node));
    }

    let focus = if snap.address_focused {
        ADDRESS_ID
    } else {
        snap.focus
            .filter(|f| index.contains_key(f))
            .map_or(WINDOW_ID, node_id)
    };
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo {
            root: WINDOW_ID,
            toolkit_name: Some("Vy".into()),
            toolkit_version: Some(env!("CARGO_PKG_VERSION").into()),
        }),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// What an assistive technology asked for, as the shell does it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Focus the address field.
    FocusAddress,
    /// Move the host's focus to a node, as Tab would (SPEC-10.3.1).
    Focus(i32),
    /// Press and release the primary button at a node's centre.
    Click(i32),
    /// Anything else (`SetValue`, scrolling, …): not supported; the page
    /// owns its values and its scrolling.
    Unsupported,
}

/// What `req` asks the shell to do.
pub fn request(req: &ActionRequest) -> Request {
    if req.target_tree != TreeId::ROOT {
        return Request::Unsupported;
    }
    match (req.action, req.target_node) {
        (Action::Focus | Action::Click, ADDRESS_ID) => Request::FocusAddress,
        (Action::Focus, n) => sem_id(n).map_or(Request::Unsupported, Request::Focus),
        (Action::Click, n) => sem_id(n).map_or(Request::Unsupported, Request::Click),
        _ => Request::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i32, parent_id: i32, role_id: i32, text: &str) -> SemNode {
        SemNode {
            id,
            parent_id,
            role: role_id,
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 30.0,
            text: text.into(),
        }
    }

    fn snap(sem: Vec<SemNode>) -> Snapshot {
        Snapshot {
            title: "Anmälan – Vy".into(),
            width: 800.0,
            height: 600.0,
            scale: 2.0,
            address_rect: Rect::new(114.0, 41.0, 678.0, 30.0),
            address: "vy://form.example/".into(),
            address_focused: false,
            page: Rect::new(0.0, 78.0, 800.0, 522.0),
            zoom: 1.0,
            page_title: "Anmälan".into(),
            sem,
            focus: None,
            inaccessible: false,
        }
    }

    /// SPEC-10.2.1, SPEC-A.3.1: every role of 1.0 has a platform role,
    /// and a module's ids, including negative ones, round-trip through the
    /// platform's node ids without meeting the shell's own.
    #[test]
    fn spec_10_2_1_roles_and_ids() {
        for r in 1..=vy_abi::role::MAX {
            assert_ne!(role_of(r), Role::GenericContainer, "role {r}");
        }
        assert_eq!(role_of(0), Role::GenericContainer);
        for id in [0, 1, -1, -10, i32::MAX, i32::MIN] {
            assert_eq!(sem_id(node_id(id)), Some(id));
            assert!(![WINDOW_ID, ADDRESS_ID, PAGE_ID, INACCESSIBLE_ID].contains(&node_id(id)));
        }
        for own in [WINDOW_ID, ADDRESS_ID, PAGE_ID, INACCESSIBLE_ID] {
            assert_eq!(sem_id(own), None);
        }
    }

    /// SPEC-10.2.1, SPEC-10.1.1: a hostile tree (a cycle, a self-parent,
    /// a parent that comes later, an unknown parent, non-finite numbers)
    /// still gives a tree where every node hangs off the root exactly once.
    #[test]
    fn spec_10_2_1_any_tree_is_well_formed() {
        let mut bad = node(6, 0, role::TEXT, "nan");
        bad.x = f32::NAN;
        let sem = vec![
            node(1, 2, role::REGION, "a"),
            node(2, 1, role::REGION, "b"),
            node(3, 3, role::TEXT, "self"),
            node(4, 99, role::TEXT, "orphan"),
            node(5, 2, role::BUTTON, "ok"),
            bad,
        ];
        let update = tree_update(&snap(sem));
        let by_id: HashMap<NodeId, &Node> = update.nodes.iter().map(|(i, n)| (*i, n)).collect();
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![WINDOW_ID];
        while let Some(id) = stack.pop() {
            assert!(seen.insert(id), "{id:?} reached twice");
            stack.extend(by_id[&id].children().iter().copied());
        }
        assert_eq!(seen.len(), update.nodes.len(), "every node reachable");
        assert!(by_id[&node_id(2)].children().contains(&node_id(5)));
        assert!(by_id[&node_id(6)].bounds().is_none());
    }
}
