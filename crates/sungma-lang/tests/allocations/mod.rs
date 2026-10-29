//! Counts the bytes each thread allocates, so a test bounds the work a parse
//! does by what it allocates rather than by how long it takes.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    /// The bytes this thread has asked for, a reallocation's whole new size
    /// included.
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

fn count(bytes: usize) {
    // A thread's count is gone while the thread exits, and what it
    // allocates then goes uncounted.
    let _ = ALLOCATED.try_with(|allocated| allocated.set(allocated.get() + bytes));
}

/// The system allocator, counting as it goes.
struct Counting;

// SAFETY: every call passes straight to the system allocator, and the count
// beside it is a thread-local cell that never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: the caller keeps the system allocator's contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: the caller keeps the system allocator's contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller keeps the system allocator's contract.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(size);
        // SAFETY: the caller keeps the system allocator's contract.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// Runs `work` on `input`, failing if it allocated more than `per_byte`
/// bytes for each byte of the input. Linear work stays within a small
/// multiple of its input; quadratic work, or worse, grows past any multiple
/// as the input does.
pub fn linear<T>(input: &str, per_byte: usize, work: impl FnOnce() -> T) -> T {
    let before = ALLOCATED.get();
    let result = work();
    let allocated = ALLOCATED.get() - before;
    assert!(
        allocated <= per_byte * input.len(),
        "{allocated} bytes allocated for {} bytes of input",
        input.len()
    );
    result
}
