//! The display list of one `vy_frame` (SPEC-5.2.3, SPEC §6.1).
//!
//! A [`Frame`] exists only between `dl_begin` and `dl_end`, so a `dl_*` call
//! outside them (SPEC-5.3.2) cannot be written. It counts opcodes and clip
//! depth as the host does and refuses locally what the host would refuse:
//! the 65 537th opcode is `DlFull` (SPEC-6.1.6), a 33rd clip push or a pop
//! without push is `DlClip` (SPEC-6.1.4), text over 65 535 bytes and a path
//! over 65 535 commands are `Arg` (SPEC-9.3.2). Nothing is sent to the host
//! for a refused call, and a call the host refuses (every `dl_*` import
//! returns its status, SPEC-5.2.3) is not counted as an opcode either, so
//! `vy_frame` reports exactly what the list holds (SPEC-5.1.2).

use crate::error::{check, check64, VyError};
use crate::raw::{self, pl};
use crate::sem::Sem;
use crate::{from_x100, Color, Rect, ResourceId, Size, StyleId};
use alloc::vec::Vec;
use vy_abi::limits;

/// How an image is fitted to its rectangle (SPEC-5.2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Scale uniformly to cover the rectangle; crop the overflow.
    Cover = 0,
    /// Scale uniformly to fit inside; letterbox the rest.
    Contain = 1,
    /// Scale each axis to the rectangle.
    Stretch = 2,
}

/// A path for [`Frame::path`], encoded as the 9-byte commands of SPEC-C.1.2.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    bytes: Vec<u8>,
    cmds: usize,
}

impl Path {
    /// An empty path.
    pub fn new() -> Self {
        Self::default()
    }

    fn cmd(&mut self, t: u8, a: f32, b: f32) {
        self.bytes.push(t);
        self.bytes.extend_from_slice(&a.to_le_bytes());
        self.bytes.extend_from_slice(&b.to_le_bytes());
        self.cmds += 1;
    }

    /// Start a new subpath at `(x, y)`.
    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.cmd(0, x, y);
        self
    }

    /// Straight segment to `(x, y)`.
    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.cmd(1, x, y);
        self
    }

    /// Quadratic segment to `(x, y)` with control point `(cx, cy)`; two
    /// commands on the wire (SPEC-C.1.2).
    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Self {
        self.cmd(2, x, y);
        self.cmd(2, cx, cy);
        self
    }

    /// Close the current subpath.
    pub fn close(&mut self) -> &mut Self {
        self.cmd(3, 0.0, 0.0);
        self
    }

    /// Commands on the wire.
    pub fn len(&self) -> usize {
        self.cmds
    }

    /// True when no command was added.
    pub fn is_empty(&self) -> bool {
        self.cmds == 0
    }

    /// The encoded bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// The display list being built for the current `vy_frame`.
///
/// Only the export glue creates one (see [`crate::App::frame`]), so a page
/// cannot open a list outside `vy_frame` (SPEC-5.3.2, SPEC-6.1.2); native
/// tests run a page's frame with `vy_sdk::mock::run_frame`. `sem` registers
/// the semantic tree alongside (SPEC-10.1.1).
pub struct Frame {
    ops: u32,
    clip_depth: u32,
    /// The semantic tree of this frame.
    pub sem: Sem,
}

impl Frame {
    /// `dl_begin`: start a new display list (SPEC-6.1.2). The host refuses
    /// a `dl_begin` while a list is open (SPEC-5.3.2), which a `Frame` built
    /// by the export glue never does; the status is therefore not surfaced.
    pub(crate) fn begin() -> Self {
        // SAFETY: no arguments.
        let _ = unsafe { raw::dl_begin() };
        Self {
            ops: 0,
            clip_depth: 0,
            sem: Sem::new(),
        }
    }

    /// `dl_end`: close the list. Returns the opcode count `vy_frame` reports
    /// (SPEC-5.1.2); `DlClip` if clip pushes were left open (SPEC-6.1.4),
    /// in which case the host closed the list anyway.
    pub(crate) fn end(self) -> Result<u32, VyError> {
        // SAFETY: no arguments.
        check(unsafe { raw::dl_end() })?;
        if self.clip_depth != 0 {
            return Err(VyError::DlClip);
        }
        Ok(self.ops)
    }

    /// Opcodes written so far.
    pub fn ops(&self) -> u32 {
        self.ops
    }

    /// Open clip rectangles.
    pub fn clip_depth(&self) -> u32 {
        self.clip_depth
    }

    fn slot(&mut self) -> Result<(), VyError> {
        if self.ops >= limits::MAX_OPS_PER_FRAME {
            return Err(VyError::DlFull);
        }
        Ok(())
    }

    /// `dl_rect`: a rectangle with rounded corners of `radius`.
    pub fn rect(&mut self, r: Rect, radius: f32, color: Color) -> Result<(), VyError> {
        self.slot()?;
        // SAFETY: no pointers.
        check(unsafe { raw::dl_rect(r.x, r.y, r.w, r.h, radius, color.as_i32()) })?;
        self.ops += 1;
        Ok(())
    }

    /// `dl_text`: shape and wrap `text` at `max_w` from `(x, y)`; returns the
    /// laid-out width and height in logical pixels (SPEC-5.2.4, SPEC-6.4.3),
    /// so the next element can follow on the same line or below. `max_w <= 0`
    /// means no wrapping.
    pub fn text(
        &mut self,
        x: f32,
        y: f32,
        max_w: f32,
        style: StyleId,
        text: &str,
    ) -> Result<Size, VyError> {
        self.slot()?;
        if text.len() > limits::MAX_TEXT_BYTES as usize {
            return Err(VyError::Arg);
        }
        let (p, l) = pl(text.as_bytes());
        // SAFETY: `p` is valid for `l` bytes during the call.
        let packed = check64(unsafe { raw::dl_text(x, y, max_w, style.0, p, l) })?;
        self.ops += 1;
        Ok(Size {
            w: from_x100((packed >> 32) as i32),
            h: from_x100(packed as i32),
        })
    }

    /// `dl_image`: draw resource `id` fitted into `r` (SPEC-5.2.3). Id 0 is
    /// the module itself and never an image (SPEC-4.5.1); an id the package
    /// has no image for is `Resource`.
    pub fn image(&mut self, r: Rect, id: ResourceId, fit: Fit) -> Result<(), VyError> {
        self.slot()?;
        if id.0 == 0 {
            return Err(VyError::Resource);
        }
        // SAFETY: no pointers.
        check(unsafe { raw::dl_image(r.x, r.y, r.w, r.h, i32::from(id.0), fit as i32) })?;
        self.ops += 1;
        Ok(())
    }

    /// `dl_path`: fill (`stroke_w == 0`) or stroke a path.
    pub fn path(&mut self, path: &Path, color: Color, stroke_w: f32) -> Result<(), VyError> {
        self.slot()?;
        if path.len() > usize::from(u16::MAX) {
            return Err(VyError::Arg);
        }
        let (p, l) = pl(path.bytes());
        // SAFETY: `p` is valid for `l` bytes during the call.
        check(unsafe { raw::dl_path(p, l, color.as_i32(), stroke_w) })?;
        self.ops += 1;
        Ok(())
    }

    /// `dl_clip_push`: everything until the matching pop is clipped to `r`
    /// with corners rounded by `radius` (SPEC-5.2.3, SPEC-6.1.4; at most 32
    /// deep). A radius of 0 is a plain rectangle.
    pub fn clip_push(&mut self, r: Rect, radius: f32) -> Result<(), VyError> {
        self.slot()?;
        if self.clip_depth >= limits::MAX_CLIP_DEPTH {
            return Err(VyError::DlClip);
        }
        // SAFETY: no pointers.
        check(unsafe { raw::dl_clip_push(r.x, r.y, r.w, r.h, radius) })?;
        self.clip_depth += 1;
        self.ops += 1;
        Ok(())
    }

    /// `dl_clip_pop` (SPEC-6.1.4; without a push it is `DlClip`).
    pub fn clip_pop(&mut self) -> Result<(), VyError> {
        self.slot()?;
        if self.clip_depth == 0 {
            return Err(VyError::DlClip);
        }
        // SAFETY: no arguments.
        check(unsafe { raw::dl_clip_pop() })?;
        self.clip_depth -= 1;
        self.ops += 1;
        Ok(())
    }

    /// Register a container node in the semantic tree and run `f` with it as
    /// the parent of every node `f` registers (SPEC-10.1.1), drawing included.
    pub fn group<R>(
        &mut self,
        role: crate::sem::Role,
        r: Rect,
        text: &str,
        f: impl FnOnce(&mut Frame) -> Result<R, VyError>,
    ) -> Result<R, VyError> {
        self.sem.begin(role, r, text)?;
        let result = f(self);
        self.sem.end();
        result
    }

    /// Run `f` with `r` (corners rounded by `radius`) as clip, popping
    /// afterwards even when `f` fails.
    pub fn clipped<R>(
        &mut self,
        r: Rect,
        radius: f32,
        f: impl FnOnce(&mut Frame) -> Result<R, VyError>,
    ) -> Result<R, VyError> {
        self.clip_push(r, radius)?;
        let result = f(self);
        self.clip_pop()?;
        result
    }
}
