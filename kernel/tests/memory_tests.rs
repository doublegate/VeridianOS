//! Memory management integration tests

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use veridian_kernel::{
    mm::{self, MemoryRegion, FRAME_SIZE},
    serial_println,
};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}

#[test_case]
fn test_frame_allocator_init() {
    // Initialize with test memory regions
    let memory_map = [
        MemoryRegion {
            start: 0x100000,        // 1MB
            size: 16 * 1024 * 1024, // 16MB
            usable: true,
        },
        MemoryRegion {
            start: 0x1100000,       // 17MB
            size: 32 * 1024 * 1024, // 32MB
            usable: true,
        },
    ];

    mm::init(&memory_map);

    let stats = mm::FRAME_ALLOCATOR.lock().get_stats();
    assert_eq!(stats.total_frames, (48 * 1024 * 1024) / FRAME_SIZE as u64);
    assert_eq!(stats.free_frames, stats.total_frames);

    serial_println!("[ok]");
}

#[test_case]
fn test_small_allocation() {
    mm::init_default();

    // Test single frame allocation
    let _frame = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .expect("Failed to allocate single frame");

    // Allocate multiple small allocations
    let mut frames = alloc::vec::Vec::new();
    for _ in 0..100 {
        let f = mm::FRAME_ALLOCATOR
            .lock()
            .allocate_frames(10, None)
            .expect("Failed to allocate 10 frames");
        frames.push(f);
    }

    // Free all frames
    for f in frames {
        mm::FRAME_ALLOCATOR
            .lock()
            .free_frames(f, 10)
            .expect("Failed to free frames");
    }

    // Check that memory was returned
    let stats = mm::FRAME_ALLOCATOR.lock().get_stats();
    assert!(stats.free_frames > 0);

    serial_println!("[ok]");
}

#[test_case]
fn test_large_allocation() {
    mm::init_default();

    // Test buddy allocator (≥512 frames)
    let large_frame = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1024, None)
        .expect("Failed to allocate 1024 frames");

    // Free the large allocation
    mm::FRAME_ALLOCATOR
        .lock()
        .free_frames(large_frame, 1024)
        .expect("Failed to free large allocation");

    // Should be able to allocate again
    let large_frame2 = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1024, None)
        .expect("Failed to reallocate");

    assert_eq!(large_frame.as_u64(), large_frame2.as_u64());

    serial_println!("[ok]");
}

#[test_case]
fn test_numa_allocation() {
    // Initialize with multiple NUMA nodes
    let memory_map = [
        MemoryRegion {
            start: 0x100000,
            size: 16 * 1024 * 1024,
            usable: true,
        },
        MemoryRegion {
            start: 0x1100000,
            size: 16 * 1024 * 1024,
            usable: true,
        },
    ];

    mm::init(&memory_map);

    // Allocate from specific NUMA node
    let frame_node0 = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(10, Some(0))
        .expect("Failed to allocate from node 0");

    let frame_node1 = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(10, Some(1))
        .expect("Failed to allocate from node 1");

    // Frames should come from different regions
    assert!(frame_node0.as_u64() < 0x1100000 / FRAME_SIZE as u64);
    assert!(frame_node1.as_u64() >= 0x1100000 / FRAME_SIZE as u64);

    serial_println!("[ok]");
}

#[test_case]
fn test_allocation_performance() {
    mm::init_default();

    let start = veridian_kernel::bench::read_timestamp();

    // Allocate many small frames
    let mut allocations = alloc::vec::Vec::new();
    for _ in 0..1000 {
        let frame = mm::FRAME_ALLOCATOR
            .lock()
            .allocate_frames(1, None)
            .expect("Allocation failed");
        allocations.push(frame);
    }

    let alloc_time = veridian_kernel::bench::read_timestamp() - start;

    // Free all frames
    let free_start = veridian_kernel::bench::read_timestamp();
    for frame in allocations {
        mm::FRAME_ALLOCATOR
            .lock()
            .free_frames(frame, 1)
            .expect("Free failed");
    }
    let free_time = veridian_kernel::bench::read_timestamp() - free_start;

    let alloc_ns = veridian_kernel::bench::cycles_to_ns(alloc_time) / 1000;
    let free_ns = veridian_kernel::bench::cycles_to_ns(free_time) / 1000;

    serial_println!("Allocation: {} ns/op, Free: {} ns/op", alloc_ns, free_ns);

    // Should meet <1μs target
    assert!(alloc_ns < 1000, "Allocation too slow: {} ns", alloc_ns);
    assert!(free_ns < 1000, "Free too slow: {} ns", free_ns);

    serial_println!("[ok]");
}

#[test_case]
fn test_fragmentation_handling() {
    mm::init_default();

    // Create fragmentation pattern
    let mut frames = alloc::vec::Vec::new();

    // Allocate alternating pattern
    for i in 0..100 {
        let frame = mm::FRAME_ALLOCATOR
            .lock()
            .allocate_frames(if i % 2 == 0 { 1 } else { 5 }, None)
            .expect("Allocation failed");
        frames.push((frame, if i % 2 == 0 { 1 } else { 5 }));
    }

    // Free every other allocation
    for i in (0..100).step_by(2) {
        let (frame, size) = frames[i];
        mm::FRAME_ALLOCATOR
            .lock()
            .free_frames(frame, size)
            .expect("Free failed");
    }

    // Should still be able to allocate
    let _new_frame = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .expect("Allocation after fragmentation failed");

    serial_println!("[ok]");
}

#[test_case]
fn test_out_of_memory() {
    mm::init_default();

    // Get initial stats
    let initial_stats = mm::FRAME_ALLOCATOR.lock().get_stats();
    let total_frames = initial_stats.total_frames;

    // Try to allocate more than available
    let result = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(total_frames as usize + 1, None);

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), mm::FrameAllocatorError::OutOfMemory);

    serial_println!("[ok]");
}

#[test_case]
fn test_double_free_detection() {
    mm::init_default();

    // Allocate a frame
    let frame = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .expect("Allocation failed");

    // Free it once
    mm::FRAME_ALLOCATOR
        .lock()
        .free_frames(frame, 1)
        .expect("First free failed");

    // Try to free again - should fail
    let result = mm::FRAME_ALLOCATOR.lock().free_frames(frame, 1);

    assert!(result.is_err());

    serial_println!("[ok]");
}

#[test_case]
fn test_allocate_pages_interface() {
    mm::init_default();

    // Test the high-level allocate_pages function
    let pages = mm::allocate_pages(10, Some(0)).expect("Failed to allocate pages");

    assert_eq!(pages.len(), 10);

    // Verify pages are consecutive
    for i in 1..10 {
        assert_eq!(pages[i].as_u64(), pages[i - 1].as_u64() + 1);
    }

    // Free the pages
    mm::free_pages(&pages).expect("Failed to free pages");

    serial_println!("[ok]");
}

#[test_case]
fn test_buddy_allocator_merging() {
    mm::init_default();

    // Allocate two buddy blocks
    let frame1 = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(512, None)
        .expect("Failed to allocate first buddy");

    let frame2 = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(512, None)
        .expect("Failed to allocate second buddy");

    // They should be adjacent
    assert_eq!(frame2.as_u64(), frame1.as_u64() + 512);

    // Free both - they should merge
    mm::FRAME_ALLOCATOR
        .lock()
        .free_frames(frame1, 512)
        .expect("Failed to free first buddy");

    mm::FRAME_ALLOCATOR
        .lock()
        .free_frames(frame2, 512)
        .expect("Failed to free second buddy");

    // Should be able to allocate 1024 frames now
    let large_frame = mm::FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1024, None)
        .expect("Failed to allocate merged block");

    assert_eq!(large_frame.as_u64(), frame1.as_u64());

    serial_println!("[ok]");
}
