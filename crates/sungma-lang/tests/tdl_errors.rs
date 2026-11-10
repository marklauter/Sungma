//! A hostile document's errors, measured by what parsing it allocates.
//! The allocator here counts every allocation, so this binary holds only
//! the tests that read it.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    time::{Duration, Instant},
};

use sungma_lang::tdl::{
    self, ErrorKind, MAX_DOCUMENT_BYTES, MAX_ERRORS, MAX_NAMESPACE_BYTES, Place,
};

/// The system allocator, counting the bytes each thread holds and the most
/// it has held.
struct Counting;

thread_local! {
    static HELD: Cell<usize> = const { Cell::new(0) };
    static PEAK: Cell<usize> = const { Cell::new(0) };
}

fn grow(bytes: usize) {
    let _ = HELD.try_with(|held| {
        let now = held.get() + bytes;
        held.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}

fn shrink(bytes: usize) {
    let _ = HELD.try_with(|held| held.set(held.get().saturating_sub(bytes)));
}

// SAFETY: each call is passed to `System` as is; the counts only read sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grow(layout.size());
        // SAFETY: the caller's layout, passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        shrink(layout.size());
        // SAFETY: the caller's pointer and layout, passed on.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        grow(size);
        shrink(layout.size());
        // SAFETY: the caller's pointer, layout and size, passed on.
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The most bytes this thread held above what it held before `run`.
fn peak<T>(run: impl FnOnce() -> T) -> (T, usize) {
    let before = HELD.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    let value = run();
    (value, PEAK.with(Cell::get) - before)
}

/// Generous for a linear pass in a debug build.
const BOUND: Duration = Duration::from_secs(60);

#[test]
fn errors_past_the_cap_arent_kept() {
    // Each line is a relation whose constraint list fails, and declares the
    // relation again: two errors a line, as many as fit, each naming the
    // longest namespace.
    let line = "  a<\n";
    let header = format!("namespace: {}\n", "n".repeat(MAX_NAMESPACE_BYTES));
    let lines = (MAX_DOCUMENT_BYTES - header.len()) / line.len();
    let text = format!("{header}{}", line.repeat(lines));
    let start = Instant::now();
    let (errors, held) = peak(|| tdl::parse(&text).expect_err("a hostile document"));
    assert!(start.elapsed() < BOUND);
    assert_eq!(errors.len(), MAX_ERRORS + 1);
    assert!(matches!(errors[MAX_ERRORS].kind, ErrorKind::TooManyErrors));
    // In document order: at each line from the third, the constraint list
    // the line before left open, found first, then the relation declared
    // again, found by the namespace's checks.
    for (at, error) in errors[..MAX_ERRORS].iter().enumerate() {
        let line = 3 + at / 2;
        assert_eq!(error.place, Some(Place { line, column: 3 }), "{error}");
        if at % 2 == 0 {
            assert!(matches!(error.kind, ErrorKind::Syntax { .. }), "{error}");
        } else {
            assert!(matches!(error.kind, ErrorKind::Namespace(_)), "{error}");
        }
    }
    // Lexing the document, and declaring and checking its relations, hold
    // about 130 bytes for each of its bytes; keeping every error held about
    // 190.
    assert!(
        held < 150 * text.len(),
        "held {held} bytes for a {} byte document",
        text.len()
    );
}
