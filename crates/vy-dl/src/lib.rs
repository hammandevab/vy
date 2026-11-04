//! Display list buffer (SPEC appendix C).
//!
//! The module describes, the host draws (SPEC-6.1.1). A `DisplayList` is the
//! byte buffer handed from the tab process to the main process: a flat sequence
//! of opcodes, each with fixed little-endian fields. This crate owns encoding,
//! validation of the limits in SPEC §6 and decoding for the rasterizer.
#![deny(missing_docs)]

use vy_abi::{err, limits, op};

/// One path command (SPEC-C.1.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCmd {
    /// Start a new subpath.
    MoveTo(f32, f32),
    /// Straight segment.
    LineTo(f32, f32),
    /// Quadratic segment with a control point.
    QuadTo {
        /// Control point.
        ctrl: (f32, f32),
        /// End point.
        to: (f32, f32),
    },
    /// Close the current subpath.
    Close,
}

/// Image fit mode for `IMAGE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Fit {
    /// Scale to cover the rectangle, cropping.
    Cover = 0,
    /// Scale to fit inside the rectangle, letterboxing.
    Contain = 1,
    /// Stretch to the rectangle.
    Stretch = 2,
}

/// A decoded opcode.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Rounded rectangle.
    Rect {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
        /// Corner radius.
        radius: f32,
        /// `0xRRGGBBAA`.
        color: u32,
    },
    /// Text run laid out by the host.
    Text {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Wrap width.
        max_w: f32,
        /// Style id from `text_style`.
        style_id: u16,
        /// UTF-8 bytes.
        text: Vec<u8>,
    },
    /// Image resource.
    Image {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
        /// Resource index.
        resource_id: u16,
        /// Fit mode.
        fit: u8,
    },
    /// Filled or stroked path.
    Path {
        /// `0xRRGGBBAA`.
        color: u32,
        /// 0 = fill, otherwise stroke width.
        stroke_w: f32,
        /// Commands.
        cmds: Vec<PathCmd>,
    },
    /// Push a clip rectangle.
    ClipPush {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
    },
    /// Pop the last clip rectangle.
    ClipPop,
    /// Line segment.
    Line {
        /// Start x.
        x1: f32,
        /// Start y.
        y1: f32,
        /// End x.
        x2: f32,
        /// End y.
        y2: f32,
        /// Stroke width.
        w: f32,
        /// `0xRRGGBBAA`.
        color: u32,
    },
    /// Drop shadow under a rounded rectangle.
    Shadow {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
        /// Corner radius.
        radius: f32,
        /// Blur radius.
        blur: f32,
        /// `0xRRGGBBAA`.
        color: u32,
    },
}

/// Encoder with the validation rules of SPEC §5.3 and §6.1.
///
/// Every method that can fail returns the ABI status code from `vy_abi::err`.
#[derive(Debug, Default)]
pub struct DisplayList {
    buf: Vec<u8>,
    ops: u32,
    clip_depth: u32,
    shadows: u32,
    text_bytes: usize,
    open: bool,
}

impl DisplayList {
    /// Empty, closed list.
    pub fn new() -> Self {
        Self::default()
    }

    /// `dl_begin`: discard the previous frame and start a new list (SPEC-6.1.2).
    pub fn begin(&mut self) {
        self.buf.clear();
        self.ops = 0;
        self.clip_depth = 0;
        self.shadows = 0;
        self.text_bytes = 0;
        self.open = true;
    }

    /// `dl_end`: write `END` and close the list.
    ///
    /// Returns `E_DL_STATE` if no list is open and `E_DL_CLIP` if clip pushes
    /// are unbalanced (the list is still closed in that case).
    pub fn end(&mut self) -> Result<(), i32> {
        if !self.open {
            return Err(err::E_DL_STATE);
        }
        self.buf.push(op::END);
        self.open = false;
        if self.clip_depth != 0 {
            return Err(err::E_DL_CLIP);
        }
        Ok(())
    }

    /// True between `begin` and `end`.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Number of opcodes written in the current list (excluding `END`).
    pub fn op_count(&self) -> u32 {
        self.ops
    }

    /// Encoded bytes of the last completed or current list.
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// UTF-8 payload bytes of every `TEXT` op in the current list; the host
    /// counts them against `vy_abi::limits::MAX_FRAME_TEXT_BYTES`.
    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }

    fn gate(&mut self) -> Result<(), i32> {
        if !self.open {
            return Err(err::E_DL_STATE);
        }
        if self.ops >= limits::MAX_OPS_PER_FRAME {
            return Err(err::E_DL_FULL);
        }
        Ok(())
    }

    fn f(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// `RECT`.
    pub fn rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: u32,
    ) -> Result<(), i32> {
        self.gate()?;
        self.buf.push(op::RECT);
        for v in [x, y, w, h, radius] {
            self.f(v);
        }
        self.u32(color);
        self.ops += 1;
        Ok(())
    }

    /// `TEXT`. Text longer than 65 535 bytes is `E_ARG`.
    pub fn text(
        &mut self,
        x: f32,
        y: f32,
        max_w: f32,
        style_id: u16,
        text: &[u8],
    ) -> Result<(), i32> {
        self.gate()?;
        let len = u16::try_from(text.len()).map_err(|_| err::E_ARG)?;
        self.buf.push(op::TEXT);
        for v in [x, y, max_w] {
            self.f(v);
        }
        self.u16(style_id);
        self.u16(len);
        self.buf.extend_from_slice(text);
        self.text_bytes += text.len();
        self.ops += 1;
        Ok(())
    }

    /// `IMAGE`.
    pub fn image(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        resource_id: u16,
        fit: Fit,
    ) -> Result<(), i32> {
        self.gate()?;
        self.buf.push(op::IMAGE);
        for v in [x, y, w, h] {
            self.f(v);
        }
        self.u16(resource_id);
        self.buf.push(fit as u8);
        self.ops += 1;
        Ok(())
    }

    /// `PATH`. `QuadTo` occupies two commands on the wire (SPEC-C.1.2).
    pub fn path(&mut self, color: u32, stroke_w: f32, cmds: &[PathCmd]) -> Result<(), i32> {
        self.gate()?;
        let wire_count: usize = cmds
            .iter()
            .map(|c| {
                if matches!(c, PathCmd::QuadTo { .. }) {
                    2
                } else {
                    1
                }
            })
            .sum();
        let n = u16::try_from(wire_count).map_err(|_| err::E_ARG)?;
        self.buf.push(op::PATH);
        self.u32(color);
        self.f(stroke_w);
        self.u16(n);
        for c in cmds {
            match *c {
                PathCmd::MoveTo(x, y) => {
                    self.buf.push(0);
                    self.f(x);
                    self.f(y);
                }
                PathCmd::LineTo(x, y) => {
                    self.buf.push(1);
                    self.f(x);
                    self.f(y);
                }
                PathCmd::QuadTo { ctrl, to } => {
                    self.buf.push(2);
                    self.f(to.0);
                    self.f(to.1);
                    self.buf.push(2);
                    self.f(ctrl.0);
                    self.f(ctrl.1);
                }
                PathCmd::Close => {
                    self.buf.push(3);
                    self.f(0.0);
                    self.f(0.0);
                }
            }
        }
        self.ops += 1;
        Ok(())
    }

    /// `CLIP_PUSH`. Depth above 32 is `E_DL_CLIP` (SPEC-6.1.4).
    pub fn clip_push(&mut self, x: f32, y: f32, w: f32, h: f32) -> Result<(), i32> {
        self.gate()?;
        if self.clip_depth >= limits::MAX_CLIP_DEPTH {
            return Err(err::E_DL_CLIP);
        }
        self.buf.push(op::CLIP_PUSH);
        for v in [x, y, w, h] {
            self.f(v);
        }
        self.clip_depth += 1;
        self.ops += 1;
        Ok(())
    }

    /// `CLIP_POP`. Pop without push is `E_DL_CLIP` (SPEC-6.1.4).
    pub fn clip_pop(&mut self) -> Result<(), i32> {
        self.gate()?;
        if self.clip_depth == 0 {
            return Err(err::E_DL_CLIP);
        }
        self.buf.push(op::CLIP_POP);
        self.clip_depth -= 1;
        self.ops += 1;
        Ok(())
    }

    /// `LINE`.
    pub fn line(
        &mut self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        w: f32,
        color: u32,
    ) -> Result<(), i32> {
        self.gate()?;
        self.buf.push(op::LINE);
        for v in [x1, y1, x2, y2, w] {
            self.f(v);
        }
        self.u32(color);
        self.ops += 1;
        Ok(())
    }

    /// `SHADOW`. More than 64 per frame is `E_DL_FULL` (SPEC-C.1.3).
    #[allow(clippy::too_many_arguments)]
    pub fn shadow(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        blur: f32,
        color: u32,
    ) -> Result<(), i32> {
        self.gate()?;
        if self.shadows >= limits::MAX_SHADOWS_PER_FRAME {
            return Err(err::E_DL_FULL);
        }
        self.buf.push(op::SHADOW);
        for v in [x, y, w, h, radius, blur] {
            self.f(v);
        }
        self.u32(color);
        self.shadows += 1;
        self.ops += 1;
        Ok(())
    }
}

/// Why a buffer failed to decode.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Buffer ended inside an opcode.
    Truncated,
    /// Unknown opcode byte at the given offset.
    UnknownOp(usize, u8),
    /// No `END` marker.
    MissingEnd,
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, DecodeError> {
        let v = *self.b.get(self.i).ok_or(DecodeError::Truncated)?;
        self.i += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        let s = self
            .b
            .get(self.i..self.i + 2)
            .ok_or(DecodeError::Truncated)?;
        self.i += 2;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        let s = self
            .b
            .get(self.i..self.i + 4)
            .ok_or(DecodeError::Truncated)?;
        self.i += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.u32()?.to_le_bytes()))
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let s = self
            .b
            .get(self.i..self.i + n)
            .ok_or(DecodeError::Truncated)?;
        self.i += n;
        Ok(s)
    }
}

/// Size of one path command on the wire (SPEC-C.1.2).
pub const PATH_CMD_BYTES: usize = 9;

/// Decode a sequence of 9-byte path commands (SPEC-C.1.2): the payload of a
/// `PATH` op and also the buffer a module hands to `dl_path`.
///
/// `UnknownOp(offset, byte)` reports a bad command type; `Truncated` a length
/// that is not a multiple of 9 or a `quadTo` without its control point.
pub fn decode_path_cmds(bytes: &[u8]) -> Result<Vec<PathCmd>, DecodeError> {
    if !bytes.len().is_multiple_of(PATH_CMD_BYTES) {
        return Err(DecodeError::Truncated);
    }
    let mut c = Cursor { b: bytes, i: 0 };
    let mut cmds = Vec::with_capacity(bytes.len() / PATH_CMD_BYTES);
    while c.i < bytes.len() {
        let at = c.i;
        let t = c.u8()?;
        let a = c.f32()?;
        let b = c.f32()?;
        cmds.push(match t {
            0 => PathCmd::MoveTo(a, b),
            1 => PathCmd::LineTo(a, b),
            2 => {
                // control point follows as a second command
                let _t2 = c.u8()?;
                let cx = c.f32()?;
                let cy = c.f32()?;
                PathCmd::QuadTo {
                    ctrl: (cx, cy),
                    to: (a, b),
                }
            }
            3 => PathCmd::Close,
            other => return Err(DecodeError::UnknownOp(at, other)),
        });
    }
    Ok(cmds)
}

/// Decode a buffer produced by [`DisplayList`] into opcodes, in order (SPEC-6.1.3).
pub fn decode(bytes: &[u8]) -> Result<Vec<Op>, DecodeError> {
    let mut c = Cursor { b: bytes, i: 0 };
    let mut out = Vec::new();
    loop {
        let at = c.i;
        let code = c.u8()?;
        match code {
            op::END => return Ok(out),
            op::RECT => out.push(Op::Rect {
                x: c.f32()?,
                y: c.f32()?,
                w: c.f32()?,
                h: c.f32()?,
                radius: c.f32()?,
                color: c.u32()?,
            }),
            op::TEXT => {
                let x = c.f32()?;
                let y = c.f32()?;
                let max_w = c.f32()?;
                let style_id = c.u16()?;
                let len = c.u16()? as usize;
                let text = c.bytes(len)?.to_vec();
                out.push(Op::Text {
                    x,
                    y,
                    max_w,
                    style_id,
                    text,
                });
            }
            op::IMAGE => out.push(Op::Image {
                x: c.f32()?,
                y: c.f32()?,
                w: c.f32()?,
                h: c.f32()?,
                resource_id: c.u16()?,
                fit: c.u8()?,
            }),
            op::PATH => {
                let color = c.u32()?;
                let stroke_w = c.f32()?;
                let n = c.u16()? as usize;
                let raw = c.bytes(n * PATH_CMD_BYTES)?;
                let cmds = decode_path_cmds(raw).map_err(|e| match e {
                    DecodeError::UnknownOp(at, t) => {
                        DecodeError::UnknownOp(at + c.i - raw.len(), t)
                    }
                    other => other,
                })?;
                out.push(Op::Path {
                    color,
                    stroke_w,
                    cmds,
                });
            }
            op::CLIP_PUSH => out.push(Op::ClipPush {
                x: c.f32()?,
                y: c.f32()?,
                w: c.f32()?,
                h: c.f32()?,
            }),
            op::CLIP_POP => out.push(Op::ClipPop),
            op::LINE => out.push(Op::Line {
                x1: c.f32()?,
                y1: c.f32()?,
                x2: c.f32()?,
                y2: c.f32()?,
                w: c.f32()?,
                color: c.u32()?,
            }),
            op::SHADOW => out.push(Op::Shadow {
                x: c.f32()?,
                y: c.f32()?,
                w: c.f32()?,
                h: c.f32()?,
                radius: c.f32()?,
                blur: c.f32()?,
                color: c.u32()?,
            }),
            other => return Err(DecodeError::UnknownOp(at, other)),
        }
        if c.i >= bytes.len() {
            return Err(DecodeError::MissingEnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> DisplayList {
        let mut dl = DisplayList::new();
        dl.begin();
        dl
    }

    /// SPEC-C.1.1: RECT is 25 bytes, CLIP_PUSH 17, CLIP_POP 1, LINE 25, SHADOW 29, END 1.
    #[test]
    fn spec_c_1_1_fixed_sizes() {
        let mut dl = open();
        dl.rect(0.0, 0.0, 1.0, 1.0, 0.0, 0xFF00_00FF).unwrap();
        assert_eq!(dl.bytes().len(), 25);
        dl.clip_push(0.0, 0.0, 1.0, 1.0).unwrap();
        assert_eq!(dl.bytes().len(), 25 + 17);
        dl.clip_pop().unwrap();
        assert_eq!(dl.bytes().len(), 25 + 17 + 1);
        dl.line(0.0, 0.0, 1.0, 1.0, 1.0, 0).unwrap();
        assert_eq!(dl.bytes().len(), 25 + 17 + 1 + 25);
        dl.shadow(0.0, 0.0, 1.0, 1.0, 0.0, 2.0, 0).unwrap();
        assert_eq!(dl.bytes().len(), 25 + 17 + 1 + 25 + 29);
        dl.end().unwrap();
        assert_eq!(dl.bytes().len(), 25 + 17 + 1 + 25 + 29 + 1);
        assert_eq!(*dl.bytes().last().unwrap(), op::END);
    }

    /// SPEC-C.1.1: TEXT is 17 + len and IMAGE is 20.
    #[test]
    fn spec_c_1_1_text_and_image_sizes() {
        let mut dl = open();
        dl.text(0.0, 0.0, 100.0, 7, b"hej").unwrap();
        assert_eq!(dl.bytes().len(), 17 + 3);
        dl.image(0.0, 0.0, 1.0, 1.0, 2, Fit::Contain).unwrap();
        assert_eq!(dl.bytes().len(), 17 + 3 + 20);
    }

    /// SPEC-C.1.2: path commands are 9 bytes and quadTo takes two.
    #[test]
    fn spec_c_1_2_path_encoding_roundtrip() {
        let mut dl = open();
        let cmds = [
            PathCmd::MoveTo(0.0, 0.0),
            PathCmd::LineTo(10.0, 0.0),
            PathCmd::QuadTo {
                ctrl: (10.0, 10.0),
                to: (0.0, 10.0),
            },
            PathCmd::Close,
        ];
        dl.path(0x1122_33FF, 0.0, &cmds).unwrap();
        // 1 + 4 + 4 + 2 header, 5 wire commands * 9
        assert_eq!(dl.bytes().len(), 11 + 9 * 5);
        dl.end().unwrap();
        let ops = decode(dl.bytes()).unwrap();
        assert_eq!(
            ops,
            vec![Op::Path {
                color: 0x1122_33FF,
                stroke_w: 0.0,
                cmds: cmds.to_vec()
            }]
        );
        // The same 9-byte encoding is what a module hands to dl_path.
        let raw = &dl.bytes()[11..11 + 9 * 5];
        assert_eq!(decode_path_cmds(raw).unwrap(), cmds.to_vec());
        assert_eq!(decode_path_cmds(&raw[..10]), Err(DecodeError::Truncated));
        let mut bad = raw.to_vec();
        bad[0] = 7;
        assert_eq!(decode_path_cmds(&bad), Err(DecodeError::UnknownOp(0, 7)));
    }

    /// SPEC-C.1.3: at most 64 shadows per frame.
    #[test]
    fn spec_c_1_3_shadow_limit() {
        let mut dl = open();
        for _ in 0..64 {
            dl.shadow(0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0).unwrap();
        }
        assert_eq!(
            dl.shadow(0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0),
            Err(err::E_DL_FULL)
        );
    }

    /// SPEC-5.3.2: dl_* outside begin/end is E_DL_STATE and ignored.
    #[test]
    fn spec_5_3_2_calls_outside_begin_end() {
        let mut dl = DisplayList::new();
        assert_eq!(dl.rect(0.0, 0.0, 1.0, 1.0, 0.0, 0), Err(err::E_DL_STATE));
        assert_eq!(dl.end(), Err(err::E_DL_STATE));
        assert!(dl.bytes().is_empty());
        dl.begin();
        dl.end().unwrap();
        assert_eq!(dl.clip_pop(), Err(err::E_DL_STATE));
    }

    /// SPEC-6.1.2: begin discards the previous frame.
    #[test]
    fn spec_6_1_2_begin_resets() {
        let mut dl = open();
        dl.rect(0.0, 0.0, 1.0, 1.0, 0.0, 0).unwrap();
        dl.end().unwrap();
        dl.begin();
        assert!(dl.bytes().is_empty());
        assert_eq!(dl.op_count(), 0);
    }

    /// SPEC-6.1.3: ops decode in the order they were written.
    #[test]
    fn spec_6_1_3_order_preserved() {
        let mut dl = open();
        dl.rect(1.0, 2.0, 3.0, 4.0, 5.0, 6).unwrap();
        dl.text(1.0, 2.0, 3.0, 4, b"ab").unwrap();
        dl.line(0.0, 0.0, 1.0, 1.0, 2.0, 7).unwrap();
        dl.end().unwrap();
        let ops = decode(dl.bytes()).unwrap();
        assert!(matches!(ops[0], Op::Rect { .. }));
        assert!(matches!(ops[1], Op::Text { .. }));
        assert!(matches!(ops[2], Op::Line { .. }));
        assert_eq!(ops.len(), 3);
    }

    /// SPEC-6.1.4: clip depth is at most 32; pop without push is E_DL_CLIP.
    #[test]
    fn spec_6_1_4_clip_depth() {
        let mut dl = open();
        assert_eq!(dl.clip_pop(), Err(err::E_DL_CLIP));
        for _ in 0..32 {
            dl.clip_push(0.0, 0.0, 1.0, 1.0).unwrap();
        }
        assert_eq!(dl.clip_push(0.0, 0.0, 1.0, 1.0), Err(err::E_DL_CLIP));
        for _ in 0..32 {
            dl.clip_pop().unwrap();
        }
        dl.end().unwrap();
    }

    /// SPEC-6.1.4: unbalanced clips at end are reported.
    #[test]
    fn spec_6_1_4_unbalanced_at_end() {
        let mut dl = open();
        dl.clip_push(0.0, 0.0, 1.0, 1.0).unwrap();
        assert_eq!(dl.end(), Err(err::E_DL_CLIP));
        assert!(!dl.is_open());
    }

    /// SPEC-6.1.5: color is passed through as 0xRRGGBBAA.
    #[test]
    fn spec_6_1_5_color_passthrough() {
        let mut dl = open();
        dl.rect(0.0, 0.0, 1.0, 1.0, 0.0, 0x1234_56FF).unwrap();
        dl.end().unwrap();
        match &decode(dl.bytes()).unwrap()[0] {
            Op::Rect { color, .. } => assert_eq!(*color, 0x1234_56FF),
            other => panic!("{other:?}"),
        }
    }

    /// SPEC-6.1.6: at most 65 536 ops per frame; the 65 537th is E_DL_FULL.
    #[test]
    fn spec_6_1_6_op_limit() {
        let mut dl = open();
        for _ in 0..65_536 {
            dl.clip_pop().err(); // cheap failing call must not count
            dl.rect(0.0, 0.0, 1.0, 1.0, 0.0, 0).unwrap();
        }
        assert_eq!(dl.op_count(), 65_536);
        assert_eq!(dl.rect(0.0, 0.0, 1.0, 1.0, 0.0, 0), Err(err::E_DL_FULL));
    }

    /// SPEC-9.3.1, SPEC-C.1.1: a text run longer than the `len: u16` field is
    /// E_ARG and writes nothing; the list counts the text bytes it holds and
    /// forgets them at the next `begin`.
    #[test]
    fn spec_9_3_1_text_bytes_counted() {
        let mut dl = open();
        assert_eq!(dl.text_bytes(), 0);
        dl.text(0.0, 0.0, 1.0, 1, b"hej").unwrap();
        dl.text(0.0, 0.0, 1.0, 1, b"och hopp").unwrap();
        assert_eq!(dl.text_bytes(), 3 + 8);
        let before = dl.bytes().len();
        let too_long = vec![b'x'; usize::from(u16::MAX) + 1];
        assert_eq!(dl.text(0.0, 0.0, 1.0, 1, &too_long), Err(err::E_ARG));
        assert_eq!(dl.bytes().len(), before, "nothing written");
        assert_eq!(dl.text_bytes(), 3 + 8);
        assert_eq!(dl.op_count(), 2);
        let max = vec![b'x'; usize::from(u16::MAX)];
        dl.text(0.0, 0.0, 1.0, 1, &max).unwrap();
        assert_eq!(dl.text_bytes(), 3 + 8 + usize::from(u16::MAX));
        dl.end().unwrap();
        dl.begin();
        assert_eq!(dl.text_bytes(), 0);
    }

    /// SPEC-C.1.1: a truncated or unterminated buffer is rejected.
    #[test]
    fn spec_c_1_1_decode_rejects_truncated() {
        let mut dl = open();
        dl.rect(0.0, 0.0, 1.0, 1.0, 0.0, 0).unwrap();
        assert_eq!(decode(dl.bytes()), Err(DecodeError::MissingEnd));
        assert_eq!(decode(&dl.bytes()[..10]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x42, 0xFF]), Err(DecodeError::UnknownOp(0, 0x42)));
    }
}
