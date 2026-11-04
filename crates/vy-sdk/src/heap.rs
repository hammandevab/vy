//! Global allocator for modules: a first-fit free list over pages grown
//! with `memory.grow`, so `alloc::vec::Vec` and `alloc::string::String`
//! work in a page. Nothing in the module's initial memory is touched; the
//! heap lives only in pages grown after instantiation, and growth stops at
//! the manifest's `memory_max` (SPEC-4.2.4), where allocation fails and the
//! page traps rather than corrupts.
//!
//! Blocks carry a header of `HEADER` bytes (size and free-list link); sizes
//! are multiples of `ALIGN`; adjacent free blocks are merged on free. Only
//! alignments up to `ALIGN` are served, which covers every type a page uses.

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

#[repr(C)]
struct Block {
    /// Whole block including this header.
    size: usize,
    /// Next free block by address, null at the end.
    next: *mut Block,
}

/// Where fresh memory comes from.
pub trait Grow {
    /// Reserve `pages` more pages; returns the address of the first new
    /// byte, or `None` when the memory cap is reached.
    fn grow(&mut self, pages: usize) -> Option<usize>;
}

/// The allocator state, generic over its memory source so it can be tested
/// natively.
pub struct Heap<G> {
    free: *mut Block,
    grow: G,
}

impl<G: Grow> Heap<G> {
    /// An empty heap.
    pub const fn new(grow: G) -> Self {
        Self {
            free: ptr::null_mut(),
            grow,
        }
    }

    fn block_size(layout: Layout) -> usize {
        let payload = layout.size().max(1);
        (payload + HEADER).div_ceil(ALIGN) * ALIGN
    }

    /// Insert a free block at `addr` of `size` bytes, merging neighbours.
    ///
    /// # Safety
    /// `addr..addr+size` must be memory this heap owns and nobody uses.
    unsafe fn insert(&mut self, addr: usize, size: usize) {
        let block = addr as *mut Block;
        (*block).size = size;
        (*block).next = ptr::null_mut();
        // Sorted by address: find prev (< block) and next (> block).
        let mut prev: *mut Block = ptr::null_mut();
        let mut cur = self.free;
        while !cur.is_null() && (cur as usize) < addr {
            prev = cur;
            cur = (*cur).next;
        }
        (*block).next = cur;
        if prev.is_null() {
            self.free = block;
        } else {
            (*prev).next = block;
        }
        // Merge with next.
        if !cur.is_null() && addr + size == cur as usize {
            (*block).size += (*cur).size;
            (*block).next = (*cur).next;
        }
        // Merge with prev.
        if !prev.is_null() && prev as usize + (*prev).size == addr {
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
        }
    }

    /// Take the first free block of at least `size` bytes, splitting off the
    /// remainder when it can hold another block.
    unsafe fn take(&mut self, size: usize) -> *mut u8 {
        let mut prev: *mut Block = ptr::null_mut();
        let mut cur = self.free;
        while !cur.is_null() {
            if (*cur).size >= size {
                let rest = (*cur).size - size;
                let next = if rest >= HEADER + ALIGN {
                    let split = (cur as usize + size) as *mut Block;
                    (*split).size = rest;
                    (*split).next = (*cur).next;
                    (*cur).size = size;
                    split
                } else {
                    (*cur).next
                };
                if prev.is_null() {
                    self.free = next;
                } else {
                    (*prev).next = next;
                }
                return (cur as *mut u8).add(HEADER);
            }
            prev = cur;
            cur = (*cur).next;
        }
        ptr::null_mut()
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
        let size = Self::block_size(layout);
        let p = self.take(size);
        if !p.is_null() {
            return p;
        }
        let pages = size.div_ceil(PAGE);
        let Some(addr) = self.grow.grow(pages) else {
            return ptr::null_mut();
        };
        self.insert(addr, pages * PAGE);
        self.take(size)
    }

    /// Free a block from [`Heap::alloc`].
    ///
    /// # Safety
    /// `p` must come from this heap's `alloc` and not be freed twice.
    pub unsafe fn dealloc(&mut self, p: *mut u8) {
        let block = p.sub(HEADER) as *mut Block;
        let size = (*block).size;
        self.insert(block as usize, size);
    }

    /// Bytes on the free list, for tests.
    pub fn free_bytes(&self) -> usize {
        let mut total = 0;
        let mut cur = self.free;
        // SAFETY: the list only holds blocks this heap owns.
        unsafe {
            while !cur.is_null() {
                total += (*cur).size;
                cur = (*cur).next;
            }
        }
        total
    }

    /// Free blocks, for tests: 1 after everything was freed and merged.
    pub fn free_blocks(&self) -> usize {
        let mut n = 0;
        let mut cur = self.free;
        // SAFETY: as above.
        unsafe {
            while !cur.is_null() {
                n += 1;
                cur = (*cur).next;
            }
        }
        n
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
}

#[cfg(all(target_arch = "wasm32", not(test)))]
#[global_allocator]
static HEAP: ModuleHeap = ModuleHeap(UnsafeCell::new(Heap::new(LinearMemory)));
