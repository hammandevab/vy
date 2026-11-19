//! Global allocator for modules: free blocks kept by size over pages
//! grown with `memory.grow`, so `alloc::vec::Vec` and
//! `alloc::string::String` work in a page. Nothing in the module's initial
//! memory is touched; the heap lives only in pages grown after
//! instantiation, and growth stops at the manifest's `memory_max`
//! (SPEC-4.2.4), where allocation fails and the page traps rather than
//! corrupts.
//!
//! What an allocation and a free cost does not depend on how many holes
//! the heap has. A page that lets thousands of small strings grow leaves
//! thousands of holes, and the first allocator here, one list in address
//! order, walked them at every free and every allocation: `kalkyl` went
//! over the frame budget on it and stopped (granskning 0018:1). Now:
//!
//! - Every block has a header of [`HEADER`] bytes: its size, with a bit
//!   that says it is free, and the size of the block before it in memory.
//!   The two give a freed block both its neighbours without a search, so
//!   adjacent free blocks are always merged.
//! - Free blocks are on lists by size: one list for each size up to
//!   [`SMALL_MAX`] (the sizes are multiples of [`ALIGN`]) and one for
//!   everything larger. A set bit per non-empty list finds the smallest
//!   block that fits in one step. Only a request above `SMALL_MAX` looks
//!   through a list, the one of large blocks, for the first that fits.
//! - One free block, the current one, is on no list. A small request that
//!   no free block has exactly the size of is cut from its front, and a
//!   block freed next to it goes back into it, so a page that allocates
//!   and frees in a heap without holes pays for moving a pointer and not
//!   for a list. When it is too small the smallest free block that fits is
//!   used, and a large one of those becomes the current block.
//! - A block grows in place when the block after it is free, and the
//!   heap's last block grows into new pages ([`Heap::realloc`]), so a `Vec`
//!   pushed to at the end of the heap does not move.
//! - The heap grows by the pages a request is short of: what is free at
//!   its end already counts.
//!
//! Sizes are multiples of `ALIGN`. Only alignments up to `ALIGN` are
//! served, which covers every type a page uses.

#[cfg(target_arch = "wasm32")]
use core::alloc::GlobalAlloc;
use core::alloc::Layout;
#[cfg(target_arch = "wasm32")]
use core::cell::UnsafeCell;
use core::ptr;

/// Alignment of every block and the largest alignment served.
pub const ALIGN: usize = 16;
/// Bytes of bookkeeping before each block's payload.
pub const HEADER: usize = 16;
/// Bytes per wasm page.
pub const PAGE: usize = 65_536;
/// The smallest block: a header and room for a free block's two links.
const MIN: usize = HEADER + ALIGN;
/// Lists of one size each: blocks of `MIN`, `MIN + ALIGN`, … bytes.
const SMALL_LISTS: usize = 63;
/// The largest block that has a list of its own size.
pub const SMALL_MAX: usize = MIN + (SMALL_LISTS - 1) * ALIGN;
/// Set in a header's size while the block is free.
const FREE: usize = 1;

#[repr(C)]
struct Block {
    /// Whole block including this header; [`FREE`] is set while it is free.
    /// Zero in the marker that ends the heap, which is never free.
    size: usize,
    /// Size of the block before this one in memory; zero for the first.
    before: usize,
}

/// What a free block keeps in its payload: its neighbours on its list.
#[repr(C)]
struct Links {
    next: *mut Block,
    prev: *mut Block,
}

// A header holds a block's two sizes, and the smallest block's payload a
// free block's two links, on wasm32 and where the tests run.
const _: () = assert!(
    core::mem::size_of::<Block>() <= HEADER && core::mem::size_of::<Links>() <= MIN - HEADER
);

/// Where fresh memory comes from.
pub trait Grow {
    /// Reserve `pages` more pages; returns the address of the first new
    /// byte, or `None` when the memory cap is reached.
    fn grow(&mut self, pages: usize) -> Option<usize>;
}

/// The allocator state, generic over its memory source so it can be tested
/// natively.
pub struct Heap<G> {
    /// The free lists: `SMALL_LISTS` of one size each, then the large one.
    lists: [*mut Block; SMALL_LISTS + 1],
    /// Bit `i` is set while `lists[i]` has a block.
    filled: u64,
    /// The current block: free, on no list, at least `MIN` bytes; or null.
    current: *mut Block,
    /// The address after the heap's last page, where the next pages
    /// continue it; zero before the first.
    end: usize,
    grow: G,
    /// Set when pages once did not continue the heap: something else grows
    /// the memory too, and new pages are not counted on to continue it.
    apart: bool,
    /// Blocks looked at and passed over, for the tests: only the list of
    /// large blocks is ever searched.
    #[cfg(test)]
    pub passed: usize,
    /// Blocks put on a list or taken off one, for the tests.
    #[cfg(test)]
    pub listed: usize,
    /// Where each part of the heap begins, for the tests' walk over it:
    /// one part unless pages did not continue it.
    #[cfg(test)]
    parts: alloc::vec::Vec<usize>,
}

/// A block's size without the free bit.
unsafe fn size_of(b: *mut Block) -> usize {
    (*b).size & !FREE
}

unsafe fn is_free(b: *mut Block) -> bool {
    (*b).size & FREE != 0
}

unsafe fn links(b: *mut Block) -> *mut Links {
    (b as *mut u8).add(HEADER) as *mut Links
}

/// The block that follows a block of `size` bytes at `b`.
unsafe fn after(b: *mut Block, size: usize) -> *mut Block {
    (b as *mut u8).add(size) as *mut Block
}

/// The list a free block of `size` bytes is on.
fn list_of(size: usize) -> usize {
    if size <= SMALL_MAX {
        size / ALIGN - MIN / ALIGN
    } else {
        SMALL_LISTS
    }
}

impl<G: Grow> Heap<G> {
    /// An empty heap.
    pub const fn new(grow: G) -> Self {
        Self {
            lists: [ptr::null_mut(); SMALL_LISTS + 1],
            filled: 0,
            current: ptr::null_mut(),
            end: 0,
            grow,
            apart: false,
            #[cfg(test)]
            passed: 0,
            #[cfg(test)]
            listed: 0,
            #[cfg(test)]
            parts: alloc::vec::Vec::new(),
        }
    }

    /// The block a payload of `bytes` needs.
    fn block_size(bytes: usize) -> usize {
        (bytes.max(1) + HEADER).div_ceil(ALIGN) * ALIGN
    }

    /// Mark `size` bytes at `b` free and put them first on their list.
    ///
    /// # Safety
    /// The bytes are this heap's, nobody uses them, the blocks on both
    /// sides are in use, and `(*b).before` is right.
    unsafe fn put(&mut self, b: *mut Block, size: usize) {
        #[cfg(test)]
        {
            self.listed += 1;
        }
        (*b).size = size | FREE;
        (*after(b, size)).before = size;
        let i = list_of(size);
        let head = self.lists[i];
        (*links(b)).next = head;
        (*links(b)).prev = ptr::null_mut();
        if !head.is_null() {
            (*links(head)).prev = b;
        }
        self.lists[i] = b;
        self.filled |= 1 << i;
    }

    /// Mark `size` bytes at `b` free and make them the current block.
    ///
    /// # Safety
    /// As [`Heap::put`], and `size` is at least `MIN`; the block that was
    /// current is on a list or a part of this one.
    unsafe fn hold(&mut self, b: *mut Block, size: usize) {
        (*b).size = size | FREE;
        (*after(b, size)).before = size;
        self.current = b;
    }

    /// Take the free block `b` off its list; it stays marked free.
    unsafe fn unlink(&mut self, b: *mut Block) {
        #[cfg(test)]
        {
            self.listed += 1;
        }
        let i = list_of(size_of(b));
        let (next, prev) = ((*links(b)).next, (*links(b)).prev);
        if prev.is_null() {
            self.lists[i] = next;
            if next.is_null() {
                self.filled &= !(1 << i);
            }
        } else {
            (*links(prev)).next = next;
        }
        if !next.is_null() {
            (*links(next)).prev = prev;
        }
    }

    /// Make the `have` bytes at `b`, which are on no list, a block in use
    /// of `size` bytes; the payload. What is left over is a free block when
    /// it can be one: the current block when `current` says that the bytes
    /// were the current block's or are to be it, otherwise on its list.
    ///
    /// Kept out of line: it is used in several places, and every page
    /// carries the allocator.
    #[inline(never)]
    unsafe fn carve(&mut self, b: *mut Block, have: usize, size: usize, current: bool) -> *mut u8 {
        let rest = have - size;
        if rest >= MIN {
            (*b).size = size;
            let r = after(b, size);
            (*r).before = size;
            if current {
                self.hold(r, rest);
            } else {
                self.put(r, rest);
            }
        } else {
            (*b).size = have;
            (*after(b, have)).before = have;
            if current {
                self.current = ptr::null_mut();
            }
        }
        (b as *mut u8).add(HEADER)
    }

    /// A block of at least `size` bytes of the free ones, or null.
    unsafe fn take(&mut self, size: usize) -> *mut u8 {
        let current = self.current;
        let fits = !current.is_null() && size_of(current) >= size;
        if size > SMALL_MAX {
            // The first large block that fits; the current one after them,
            // which keeps it for the small requests.
            let mut b = self.lists[SMALL_LISTS];
            while !b.is_null() {
                let have = size_of(b);
                if have >= size {
                    self.unlink(b);
                    return self.carve(b, have, size, false);
                }
                #[cfg(test)]
                {
                    self.passed += 1;
                }
                b = (*links(b)).next;
            }
            if fits {
                return self.carve(current, size_of(current), size, true);
            }
            return ptr::null_mut();
        }
        // A free block of exactly this size.
        let from = list_of(size);
        let b = self.lists[from];
        if !b.is_null() {
            self.unlink(b);
            (*b).size = size;
            return (b as *mut u8).add(HEADER);
        }
        if fits {
            return self.carve(current, size_of(current), size, true);
        }
        // The smallest free block that is larger: the first list from this
        // size up that has one. The list of large blocks is the last bit,
        // and any of its blocks fits; that one becomes the current block.
        let filled = self.filled >> from;
        if filled == 0 {
            return ptr::null_mut();
        }
        let list = from + filled.trailing_zeros() as usize;
        let b = self.lists[list];
        self.unlink(b);
        if list < SMALL_LISTS {
            return self.carve(b, size_of(b), size, false);
        }
        if !current.is_null() {
            self.put(current, size_of(current));
        }
        self.carve(b, size_of(b), size, true)
    }

    /// The bytes of the free block the heap ends with; 0 when a block in
    /// use ends it.
    ///
    /// # Safety
    /// The heap has pages.
    unsafe fn tail(&self) -> usize {
        let marker = (self.end - HEADER) as *mut Block;
        let last = (marker as *mut u8).sub((*marker).before) as *mut Block;
        if is_free(last) {
            size_of(last)
        } else {
            0
        }
    }

    /// Grow the heap by the pages a free block of `size` bytes at its end
    /// is short of; false at the memory cap. New pages continue the heap:
    /// what is free at its end already counts, and its end marker becomes
    /// the header of the new block. `size` is more than what is free there.
    ///
    /// If the pages do not continue the heap (something else grew the
    /// memory between), the block they make can be short of `size`; the
    /// caller asks again, and from then on pages are asked for as if they
    /// never continued it.
    unsafe fn extend(&mut self, size: usize) -> bool {
        let need = if self.end == 0 || self.apart {
            size + HEADER
        } else {
            size - self.tail()
        };
        let pages = need.div_ceil(PAGE);
        let Some(addr) = self.grow.grow(pages) else {
            return false;
        };
        let bytes = pages * PAGE;
        let (b, before, size) = if self.end != 0 && addr == self.end {
            let marker = (addr - HEADER) as *mut Block;
            (marker, (*marker).before, bytes)
        } else {
            self.apart = self.end != 0;
            #[cfg(test)]
            self.parts.push(addr);
            (addr as *mut Block, 0, bytes - HEADER)
        };
        self.end = addr + bytes;
        (*after(b, size)).size = 0;
        (*b).before = before;
        (*b).size = size;
        // Freed as a block in use is: merged with a free block before it.
        self.dealloc((b as *mut u8).add(HEADER));
        true
    }

    /// Allocate for `layout`; null when the memory cap is reached or the
    /// alignment is above [`ALIGN`].
    ///
    /// # Safety
    /// Single-threaded use only.
    pub unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        if layout.align() > ALIGN {
            return ptr::null_mut();
        }
        let size = Self::block_size(layout.size());
        // Once more only after pages that did not continue the heap.
        loop {
            let p = self.take(size);
            if !p.is_null() || !self.extend(size) {
                return p;
            }
        }
    }

    /// Free a block from [`Heap::alloc`], merging it with a free block on
    /// either side. Next to the current block it becomes a part of that.
    ///
    /// # Safety
    /// `p` must come from this heap's `alloc` and not be freed twice.
    pub unsafe fn dealloc(&mut self, p: *mut u8) {
        let mut b = p.sub(HEADER) as *mut Block;
        let mut size = (*b).size;
        let mut current = false;
        let next = after(b, size);
        if is_free(next) {
            if next == self.current {
                current = true;
            } else {
                self.unlink(next);
            }
            size += size_of(next);
        }
        let before = (*b).before;
        if before != 0 {
            let prev = (b as *mut u8).sub(before) as *mut Block;
            if is_free(prev) {
                b = prev;
                size += before;
                if prev == self.current {
                    current = true;
                } else if !current && before > SMALL_MAX {
                    // A large block stays where it is on its list, larger.
                    (*b).size = size | FREE;
                    (*after(b, size)).before = size;
                    return;
                } else {
                    self.unlink(prev);
                }
            }
        }
        if current {
            self.hold(b, size);
        } else {
            self.put(b, size);
        }
    }

    /// Make the block at `p` hold `bytes` without moving it: true when it
    /// already does (what it no longer needs is freed when that is a
    /// block) or when the block after it is free and large enough to grow
    /// into.
    unsafe fn resize(&mut self, p: *mut u8, bytes: usize) -> bool {
        let b = p.sub(HEADER) as *mut Block;
        let have = (*b).size;
        let size = Self::block_size(bytes);
        if size <= have {
            if have - size >= MIN {
                (*b).size = size;
                let rest = after(b, size);
                (*rest).before = size;
                (*rest).size = have - size;
                self.dealloc((rest as *mut u8).add(HEADER));
            }
            return true;
        }
        let next = after(b, have);
        if is_free(next) && have + size_of(next) >= size {
            let current = next == self.current;
            if !current {
                self.unlink(next);
            }
            self.carve(b, have + size_of(next), size, current);
            return true;
        }
        false
    }

    /// Give the block at `p`, which holds `old` bytes, room for `bytes`:
    /// in place when it can be, which the heap's last block can as long as
    /// there are pages, otherwise in a new block the old bytes are copied
    /// to. Null, with the block untouched, at the memory cap.
    ///
    /// # Safety
    /// As [`Heap::dealloc`]; `old` is the size the block was asked for.
    pub unsafe fn realloc(&mut self, p: *mut u8, old: usize, bytes: usize) -> *mut u8 {
        if self.resize(p, bytes) {
            return p;
        }
        // The heap's last block, with what is free after it, grows into
        // new pages where it is.
        let b = p.sub(HEADER) as *mut Block;
        let have = (*b).size;
        let mut next = after(b, have);
        if is_free(next) {
            next = after(next, size_of(next));
        }
        if !self.apart
            && next as usize + HEADER == self.end
            && self.extend(Self::block_size(bytes) - have)
            && self.resize(p, bytes)
        {
            return p;
        }
        let q = self.alloc(Layout::from_size_align_unchecked(bytes, ALIGN));
        if !q.is_null() {
            ptr::copy_nonoverlapping(p, q, old.min(bytes));
            self.dealloc(p);
        }
        q
    }

    /// Bytes in free blocks, for tests.
    pub fn free_bytes(&self) -> usize {
        self.free().0
    }

    /// Free blocks, for tests: 1 after everything was freed and merged.
    pub fn free_blocks(&self) -> usize {
        self.free().1
    }

    /// Bytes and blocks that are free: the current block and the lists.
    fn free(&self) -> (usize, usize) {
        let (mut bytes, mut blocks) = (0, 0);
        if !self.current.is_null() {
            // SAFETY: the current block is this heap's.
            bytes = unsafe { size_of(self.current) };
            blocks = 1;
        }
        for head in self.lists {
            let mut b = head;
            // SAFETY: the lists only hold blocks this heap owns.
            unsafe {
                while !b.is_null() {
                    bytes += size_of(b);
                    blocks += 1;
                    b = (*links(b)).next;
                }
            }
        }
        (bytes, blocks)
    }

    /// Walk the heap from its first block to its end marker and the lists
    /// from end to end, and panic at anything that does not hold: a size
    /// that is not a block's, a block that does not know the one before
    /// it, two free blocks side by side, a free block on the wrong list or
    /// on none that is not the current block, a current block that is on a
    /// list or is not a free block of the heap, a list bit that is wrong.
    /// The bytes in use, for the tests.
    #[cfg(test)]
    pub fn check(&self) -> usize {
        let (mut used, mut free_bytes, mut free_blocks) = (0, 0, 0);
        let mut current = self.current.is_null();
        // SAFETY: the walk follows the sizes the heap wrote into memory it
        // owns; a wrong one is what the asserts are for.
        unsafe {
            for (part, start) in self.parts.iter().enumerate() {
                let mut b = *start as *mut Block;
                let mut before = 0;
                let mut after_free = false;
                while (*b).size != 0 {
                    let size = size_of(b);
                    assert!(size >= MIN && size.is_multiple_of(ALIGN), "block of {size}");
                    assert_eq!((*b).before, before, "the block before");
                    assert!(!(after_free && is_free(b)), "two free blocks");
                    after_free = is_free(b);
                    if after_free {
                        free_bytes += size;
                        free_blocks += 1;
                        current |= b == self.current;
                        let mut on = self.lists[list_of(size)];
                        while on != b && b != self.current {
                            assert!(!on.is_null(), "a free block on no list");
                            on = (*links(on)).next;
                        }
                    } else {
                        used += size;
                    }
                    before = size;
                    b = after(b, size);
                }
                assert_eq!((*b).before, before, "the end marker");
                if part + 1 == self.parts.len() {
                    assert_eq!(b as usize + HEADER, self.end, "the heap's end");
                }
            }
            for (i, head) in self.lists.iter().enumerate() {
                assert_eq!(self.filled >> i & 1 == 1, !head.is_null(), "bit {i}");
                let mut prev = ptr::null_mut();
                let mut b = *head;
                while !b.is_null() {
                    assert!(is_free(b) && list_of(size_of(b)) == i);
                    assert!(b != self.current, "the current block on a list");
                    assert_eq!((*links(b)).prev, prev);
                    prev = b;
                    b = (*links(b)).next;
                }
            }
        }
        assert!(current, "the current block is not a free block");
        assert_eq!(self.free(), (free_bytes, free_blocks), "the lists");
        used
    }

    /// Whether a request for `bytes` can be served with `pages` more pages
    /// that continue the heap: a free block is large enough, or what is
    /// free at the heap's end and the pages are together. For the tests.
    #[cfg(test)]
    pub fn can_serve(&self, bytes: usize, pages: usize) -> bool {
        let size = Self::block_size(bytes);
        // SAFETY: the lists and the current block are this heap's.
        unsafe {
            let mut largest = 0;
            if !self.current.is_null() {
                largest = size_of(self.current);
            }
            for head in self.lists {
                let mut b = head;
                while !b.is_null() {
                    largest = largest.max(size_of(b));
                    b = (*links(b)).next;
                }
            }
            let grown = if self.end == 0 {
                (pages * PAGE).saturating_sub(HEADER)
            } else {
                self.tail() + pages * PAGE
            };
            largest >= size || grown >= size
        }
    }

    /// The memory source, for the tests.
    #[cfg(test)]
    pub fn source(&self) -> &G {
        &self.grow
    }

    /// The parts the heap is in, for the tests.
    #[cfg(test)]
    pub fn parts(&self) -> usize {
        self.parts.len()
    }
}

/// Linear memory as the source (wasm32).
#[cfg(target_arch = "wasm32")]
pub struct LinearMemory;

#[cfg(target_arch = "wasm32")]
impl Grow for LinearMemory {
    fn grow(&mut self, pages: usize) -> Option<usize> {
        let old = core::arch::wasm32::memory_grow(0, pages);
        if old == usize::MAX {
            None
        } else {
            Some(old * PAGE)
        }
    }
}

/// The global allocator of every module (wasm32).
#[cfg(target_arch = "wasm32")]
pub struct ModuleHeap(UnsafeCell<Heap<LinearMemory>>);

// SAFETY: modules are single-threaded (SPEC-4.2.3).
#[cfg(target_arch = "wasm32")]
unsafe impl Sync for ModuleHeap {}

#[cfg(target_arch = "wasm32")]
unsafe impl GlobalAlloc for ModuleHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        (*self.0.get()).alloc(layout)
    }
    unsafe fn dealloc(&self, p: *mut u8, _layout: Layout) {
        (*self.0.get()).dealloc(p)
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        (*self.0.get()).realloc(p, layout.size(), bytes)
    }
}

#[cfg(all(target_arch = "wasm32", not(test)))]
#[global_allocator]
static HEAP: ModuleHeap = ModuleHeap(UnsafeCell::new(Heap::new(LinearMemory)));
