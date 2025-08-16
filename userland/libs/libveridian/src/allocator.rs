//! User-space memory allocator
//!
//! A buddy allocator implementation for user-space programs.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use core::mem;

const PAGE_SIZE: usize = 4096;
const MIN_BLOCK_SIZE: usize = 32; // Minimum allocation size
const MAX_ORDER: usize = 10; // Maximum order (32B to 16KB)
const HEAP_SIZE: usize = 16 * 1024 * 1024; // 16MB heap

/// Buddy allocator for user-space
pub struct BuddyAllocator {
    heap_start: usize,
    heap_end: usize,
    free_lists: [Option<NonNull<FreeBlock>>; MAX_ORDER + 1],
    initialized: bool,
}

#[repr(C)]
struct FreeBlock {
    next: Option<NonNull<FreeBlock>>,
    order: usize,
}

impl BuddyAllocator {
    /// Create a new buddy allocator
    pub const fn new() -> Self {
        Self {
            heap_start: 0,
            heap_end: 0,
            free_lists: [None; MAX_ORDER + 1],
            initialized: false,
        }
    }
    
    /// Initialize the allocator with a heap region
    pub unsafe fn init(&mut self, heap_start: *mut u8, heap_size: usize) {
        if self.initialized {
            return;
        }
        
        self.heap_start = heap_start as usize;
        self.heap_end = self.heap_start + heap_size;
        
        // Initialize all memory as one large free block
        let order = self.size_to_order(heap_size);
        let block = heap_start as *mut FreeBlock;
        (*block).next = None;
        (*block).order = order;
        
        self.free_lists[order] = NonNull::new(block);
        self.initialized = true;
    }
    
    /// Calculate the order for a given size
    fn size_to_order(&self, size: usize) -> usize {
        let mut order = 0;
        let mut block_size = MIN_BLOCK_SIZE;
        
        while block_size < size && order < MAX_ORDER {
            block_size *= 2;
            order += 1;
        }
        
        order
    }
    
    /// Calculate the size for a given order
    fn order_to_size(&self, order: usize) -> usize {
        MIN_BLOCK_SIZE << order
    }
    
    /// Split a block of given order
    unsafe fn split_block(&mut self, order: usize) -> Option<NonNull<FreeBlock>> {
        if order == 0 || order > MAX_ORDER {
            return None;
        }
        
        // Try to get a block of higher order
        let block = if let Some(block) = self.free_lists[order].take() {
            block
        } else {
            // Recursively split a larger block
            self.split_block(order + 1)?
        };
        
        // Split the block
        let block_ptr = block.as_ptr();
        let block_size = self.order_to_size(order);
        let buddy_ptr = (block_ptr as usize + block_size / 2) as *mut FreeBlock;
        
        // Initialize buddy block
        (*buddy_ptr).next = self.free_lists[order - 1];
        (*buddy_ptr).order = order - 1;
        
        // Add buddy to free list
        self.free_lists[order - 1] = NonNull::new(buddy_ptr);
        
        // Update original block
        (*block_ptr).order = order - 1;
        
        Some(NonNull::new_unchecked(block_ptr))
    }
    
    /// Allocate a block of memory
    unsafe fn alloc_block(&mut self, order: usize) -> Option<NonNull<u8>> {
        if order > MAX_ORDER {
            return None;
        }
        
        // Try to find a free block of the requested order
        if let Some(mut block) = self.free_lists[order] {
            // Remove block from free list
            self.free_lists[order] = block.as_mut().next;
            return Some(NonNull::new_unchecked(block.as_ptr() as *mut u8));
        }
        
        // Split a larger block
        if order < MAX_ORDER {
            if let Some(block) = self.split_block(order + 1) {
                // Remove block from free list
                self.free_lists[order] = block.as_ref().next;
                return Some(NonNull::new_unchecked(block.as_ptr() as *mut u8));
            }
        }
        
        None
    }
    
    /// Free a block of memory
    unsafe fn free_block(&mut self, ptr: NonNull<u8>, order: usize) {
        let block = ptr.as_ptr() as *mut FreeBlock;
        
        // Try to coalesce with buddy
        let block_addr = block as usize;
        let block_size = self.order_to_size(order);
        let buddy_addr = block_addr ^ block_size;
        
        // Check if buddy is within heap bounds
        if buddy_addr >= self.heap_start && buddy_addr < self.heap_end {
            // Search for buddy in free list
            let mut prev: Option<NonNull<FreeBlock>> = None;
            let mut current = self.free_lists[order];
            
            while let Some(mut curr_block) = current {
                if curr_block.as_ptr() as usize == buddy_addr {
                    // Found buddy - remove it from free list
                    if let Some(mut p) = prev {
                        p.as_mut().next = curr_block.as_ref().next;
                    } else {
                        self.free_lists[order] = curr_block.as_ref().next;
                    }
                    
                    // Coalesce blocks
                    let coalesced_addr = block_addr.min(buddy_addr);
                    let coalesced_ptr = NonNull::new_unchecked(coalesced_addr as *mut u8);
                    
                    // Free the coalesced block at higher order
                    if order < MAX_ORDER {
                        self.free_block(coalesced_ptr, order + 1);
                    }
                    return;
                }
                
                prev = Some(curr_block);
                current = curr_block.as_ref().next;
            }
        }
        
        // No coalescing - add block to free list
        (*block).next = self.free_lists[order];
        (*block).order = order;
        self.free_lists[order] = NonNull::new(block);
    }
}

unsafe impl GlobalAlloc for BuddyAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(layout.align());
        let order = (self as *const Self as *mut Self).as_mut().unwrap().size_to_order(size);
        
        if let Some(ptr) = (self as *const Self as *mut Self).as_mut().unwrap().alloc_block(order) {
            ptr.as_ptr()
        } else {
            ptr::null_mut()
        }
    }
    
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }
        
        let size = layout.size().max(layout.align());
        let order = (self as *const Self as *mut Self).as_mut().unwrap().size_to_order(size);
        
        if let Some(ptr) = NonNull::new(ptr) {
            (self as *const Self as *mut Self).as_mut().unwrap().free_block(ptr, order);
        }
    }
}

/// Global allocator instance
#[global_allocator]
static mut ALLOCATOR: BuddyAllocator = BuddyAllocator::new();

/// Static heap buffer
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// Initialize the global allocator
pub fn init() {
    unsafe {
        ALLOCATOR.init(HEAP.as_mut_ptr(), HEAP_SIZE);
    }
}

/// Allocation error handler
#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!("Allocation error: {:?}", layout);
}