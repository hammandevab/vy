//! The semantic tree, registered node by node while drawing (SPEC-10.1.1,
//! SPEC-5.2.13).
//!
//! `Sem` hands out ids in call order starting at 1, keeps a stack of open
//! parents, and refuses locally what the host would: the 65 537th node is
//! `DlFull` and a text over 4 KB is `Arg` (SPEC-9.3.2). Ids are stable across
//! frames as long as the draw order is, which is what `KIND_FOCUS` events
//! rely on (SPEC-10.3.1); the host walks the focusable nodes in the order
//! they were registered (SPEC-10.3.2), so register the whole tree every
//! frame, on screen or not (SPEC-10.1.1). An id used twice in one frame is
//! refused by the host as `Arg` (SPEC-5.2.13) and surfaces here as `Err`.

use crate::error::{check, VyError};
use crate::raw::{self, pl};
use crate::Rect;
use alloc::vec::Vec;
use vy_abi::{limits, role};

/// Id of a registered node, as `KIND_FOCUS` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(pub i32);

/// The fixed roles of SPEC-A.3.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Role {
    /// Plain text.
    Text = role::TEXT,
    /// Heading.
    Heading = role::HEADING,
    /// Link.
    Link = role::LINK,
    /// Button.
    Button = role::BUTTON,
    /// Text input; requires capability `ime` (SPEC-10.4.2).
    Textbox = role::TEXTBOX,
    /// Checkbox.
    Checkbox = role::CHECKBOX,
    /// Radio button.
    Radio = role::RADIO,
    /// List.
    List = role::LIST,
    /// List item.
    ListItem = role::LISTITEM,
    /// Image with alt text.
    Image = role::IMAGE,
    /// Table.
    Table = role::TABLE,
    /// Table row.
    Row = role::ROW,
    /// Table cell.
    Cell = role::CELL,
    /// Navigation region.
    Nav = role::NAV,
    /// Main content.
    Main = role::MAIN,
    /// Generic region.
    Region = role::REGION,
    /// Dialog.
    Dialog = role::DIALOG,
    /// Menu.
    Menu = role::MENU,
    /// Menu item.
    MenuItem = role::MENUITEM,
    /// Tab.
    Tab = role::TAB,
    /// Tab panel.
    TabPanel = role::TABPANEL,
    /// Progress indicator.
    Progress = role::PROGRESS,
    /// Slider.
    Slider = role::SLIDER,
    /// Separator.
    Separator = role::SEPARATOR,
}

/// Builder for the semantic tree of one frame.
#[derive(Debug, Default)]
pub struct Sem {
    next_id: i32,
    parents: Vec<i32>,
    count: u32,
}

impl Sem {
    /// An empty tree; the first node gets id 1.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            parents: Vec::new(),
            count: 0,
        }
    }

    /// The id the next node registered without an explicit id will get.
    pub fn next_id(&self) -> NodeId {
        NodeId(self.next_id)
    }

    /// Nodes registered so far.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Id of the innermost open parent, 0 at the root.
    pub fn parent(&self) -> i32 {
        self.parents.last().copied().unwrap_or(0)
    }

    /// Register a node under the current parent with the next free id.
    pub fn node(&mut self, role: Role, rect: Rect, text: &str) -> Result<NodeId, VyError> {
        let id = self.next_id;
        self.node_with_id(id, role, rect, text)
    }

    /// Register a node with an id of the page's choosing. Later automatic
    /// ids continue above it.
    pub fn node_with_id(
        &mut self,
        id: i32,
        role: Role,
        rect: Rect,
        text: &str,
    ) -> Result<NodeId, VyError> {
        if self.count >= limits::MAX_SEM_NODES_PER_FRAME {
            return Err(VyError::DlFull);
        }
        if text.len() > limits::MAX_SEM_TEXT_BYTES as usize {
            return Err(VyError::Arg);
        }
        let (p, l) = pl(text.as_bytes());
        // SAFETY: `p` is valid for `l` bytes during the call.
        check(unsafe {
            raw::sem_node(
                id,
                self.parent(),
                role as i32,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                p,
                l,
            )
        })?;
        self.count += 1;
        self.next_id = self.next_id.max(id.saturating_add(1));
        Ok(NodeId(id))
    }

    /// Register a node and make it the parent of what follows until [`Sem::end`].
    pub fn begin(&mut self, role: Role, rect: Rect, text: &str) -> Result<NodeId, VyError> {
        let id = self.node(role, rect, text)?;
        self.parents.push(id.0);
        Ok(id)
    }

    /// Close the innermost [`Sem::begin`]. Without one it does nothing.
    pub fn end(&mut self) {
        self.parents.pop();
    }

    /// Register a container node and run `f` with it as parent.
    pub fn group<R>(
        &mut self,
        role: Role,
        rect: Rect,
        text: &str,
        f: impl FnOnce(&mut Sem) -> Result<R, VyError>,
    ) -> Result<R, VyError> {
        self.begin(role, rect, text)?;
        let r = f(self);
        self.end();
        r
    }

    /// A `text` node.
    pub fn text(&mut self, rect: Rect, text: &str) -> Result<NodeId, VyError> {
        self.node(Role::Text, rect, text)
    }

    /// A `heading` node.
    pub fn heading(&mut self, rect: Rect, text: &str) -> Result<NodeId, VyError> {
        self.node(Role::Heading, rect, text)
    }

    /// A `link` node named `text`.
    pub fn link(&mut self, rect: Rect, text: &str) -> Result<NodeId, VyError> {
        self.node(Role::Link, rect, text)
    }

    /// A `button` node named `text`.
    pub fn button(&mut self, rect: Rect, text: &str) -> Result<NodeId, VyError> {
        self.node(Role::Button, rect, text)
    }

    /// A `textbox` node whose text is its current value.
    pub fn textbox(&mut self, rect: Rect, value: &str) -> Result<NodeId, VyError> {
        self.node(Role::Textbox, rect, value)
    }

    /// A `checkbox` node named `text`.
    pub fn checkbox(&mut self, rect: Rect, text: &str) -> Result<NodeId, VyError> {
        self.node(Role::Checkbox, rect, text)
    }

    /// An `image` node with alt text.
    pub fn image(&mut self, rect: Rect, alt: &str) -> Result<NodeId, VyError> {
        self.node(Role::Image, rect, alt)
    }
}
