use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

pub(crate) struct CountingAllocator;

impl CountingAllocator {
    #[expect(
        clippy::unused_self,
        reason = "method syntax associates the thread-local count with the installed allocator"
    )]
    pub(crate) fn allocation_count(&self) -> usize { ALLOCATIONS.try_with(Cell::get).unwrap_or(0) }

    #[expect(
        clippy::unused_self,
        reason = "method syntax associates thread-local recording with the installed allocator"
    )]
    fn record_allocation(&self) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

// SAFETY: Every allocation operation delegates to `System` with the arguments received from the
// allocator caller.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.record_allocation();

        // SAFETY: `layout` is provided by the allocator caller and `System` is the process
        // allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.record_allocation();

        // SAFETY: `layout` is provided by the allocator caller and `System` is the process
        // allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` and `layout` are provided by the allocator caller and `System` owns
        // the matching allocation.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        self.record_allocation();

        // SAFETY: `pointer`, `layout`, and `size` are provided by the allocator caller and
        // `System` owns the matching allocation.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
pub(crate) static TEST_ALLOCATOR: CountingAllocator = CountingAllocator;
