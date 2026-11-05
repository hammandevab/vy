//! The buffer behind `vy_alloc` (SPEC-5.1.6): a bump arena the host writes
//! committed text into before delivering `TEXT_INPUT` or `IME_COMPOSE`.
//!
//! The host hands the pointer straight back in the event, so a buffer only
//! has to live until the event that carries it has been dispatched
//! (SPEC-5.1.6), and that is exactly how long a page can see it: the bytes
//! are borrowed from a [`Dispatch`], which the export glue creates for one
//! `vy_event` and drops when it returns. The arena is `ARENA_BYTES` large
//! and wraps to its start when full, which never happens within one event.
//! On wasm32 the handle is the buffer's address in linear memory; natively,
//! where pointers do not fit an `i32`, it is the offset into the arena, and
//! [`Dispatch::bytes`] understands both.

use core::cell::UnsafeCell;

/// Size of the arena: enough for any committed text the host sends at once.
pub const ARENA_BYTES: usize = 64 * 1024;

struct Arena {
    buf: UnsafeCell<[u8; ARENA_BYTES]>,
    used: UnsafeCell<usize>,
    last: UnsafeCell<(usize, usize)>,
}

// SAFETY: modules are single-threaded (SPEC-4.2.3) and the host never
// re-enters an export (SPEC-5.3.4); native tests run one host per thread
// and touch the arena only from that thread.
unsafe impl Sync for Arena {}

static ARENA: Arena = Arena {
    buf: UnsafeCell::new([0; ARENA_BYTES]),
    used: UnsafeCell::new(0),
    last: UnsafeCell::new((0, 0)),
};

fn base() -> *mut u8 {
    ARENA.buf.get().cast()
}

/// The arena's start as the host sees it: its address on wasm32; natively
/// a fixed non-zero offset, so a successful `alloc` never returns 0
/// (SPEC-5.1.6: 0 means failure).
fn origin() -> usize {
    if cfg!(target_arch = "wasm32") {
        base() as usize
    } else {
        ARENA_BYTES
    }
}

/// `vy_alloc`: a handle to `size` writable bytes, or 0 when `size` is
/// negative or larger than the arena.
pub fn alloc(size: i32) -> i32 {
    let Ok(size) = usize::try_from(size) else {
        return 0;
    };
    if size > ARENA_BYTES {
        return 0;
    }
    // SAFETY: single-threaded access, see `Sync` above.
    unsafe {
        let used = &mut *ARENA.used.get();
        let start = (*used + 7) & !7;
        let start = if start + size > ARENA_BYTES { 0 } else { start };
        *used = start + size;
        *ARENA.last.get() = (start, size);
        (origin() + start) as i32
    }
}

/// One `vy_event` dispatch's view of the arena. The export glue makes one
/// per event; the bytes behind a handle are borrowed from it and so cannot
/// outlive the event that carried them (SPEC-5.1.6).
pub struct Dispatch(());

impl Dispatch {
    pub(crate) fn new() -> Self {
        Self(())
    }

    /// The bytes behind a handle from [`alloc`] and a length, or `None` when
    /// the range is not inside the arena.
    pub fn bytes(&self, handle: i32, len: i32) -> Option<&[u8]> {
        let (Ok(handle), Ok(len)) = (usize::try_from(handle), usize::try_from(len)) else {
            return None;
        };
        let start = handle.checked_sub(origin())?;
        let end = start.checked_add(len)?;
        if end > ARENA_BYTES {
            return None;
        }
        // SAFETY: the range is inside the arena, which lives for the
        // program; the host writes to it only between events (SPEC-5.3.4),
        // and the slice cannot outlive this dispatch.
        Some(unsafe { core::slice::from_raw_parts(base().add(start), len) })
    }

    /// [`Dispatch::bytes`] as UTF-8, or `None` when it is not.
    pub fn str(&self, handle: i32, len: i32) -> Option<&str> {
        core::str::from_utf8(self.bytes(handle, len)?).ok()
    }
}

/// Write `data` into a fresh buffer as the host would before a
/// `TEXT_INPUT`, returning the `(ptr, len)` to pass to `vy_event`. For native
/// tests.
pub fn write(data: &[u8]) -> Option<(i32, i32)> {
    let len = i32::try_from(data.len()).ok()?;
    let handle = alloc(len);
    if handle == 0 && !data.is_empty() {
        return None;
    }
    // SAFETY: `alloc` just reserved `len` bytes at this handle.
    unsafe {
        let (start, _) = *ARENA.last.get();
        core::ptr::copy_nonoverlapping(data.as_ptr(), base().add(start), data.len());
    }
    Some((handle, len))
}

/// The last `(offset, size)` handed out, for tests.
pub fn last() -> (usize, usize) {
    // SAFETY: single-threaded access, see `Sync` above.
    unsafe { *ARENA.last.get() }
}
