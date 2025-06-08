//! IPC integration tests demonstrating full functionality

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;
use veridian_kernel::{serial_print, serial_println};
use veridian_kernel::ipc::{
    Message, SmallMessage, IpcCapability, IpcPermissions,
    shared_memory::{SharedRegion, MemoryRegion, CachePolicy},
};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {}
}

#[test_case]
fn test_ipc_message_flow() {
    serial_print!("test_ipc_message_flow... ");
    
    // Create a capability for IPC
    let cap = IpcCapability::new(100, IpcPermissions::all());
    
    // Create a small message
    let msg = SmallMessage::new(cap.id(), 42)
        .with_flags(0x01) // URGENT
        .with_data(0, 1234)
        .with_data(1, 5678);
    
    // Convert to Message enum
    let message = Message::Small(msg);
    
    // Verify message properties
    assert_eq!(message.capability(), cap.id());
    assert_eq!(message.opcode(), 42);
    
    serial_println!("[ok]");
}

#[test_case]
fn test_capability_lifecycle() {
    serial_print!("test_capability_lifecycle... ");
    
    // Create parent capability with all permissions
    let parent = IpcCapability::new(200, IpcPermissions::all());
    
    // Derive child with reduced permissions
    let child_perms = IpcPermissions {
        can_send: true,
        can_receive: false,
        can_share: false,
        max_message_size: 1024,
    };
    
    let child = parent.derive(child_perms).unwrap();
    
    // Verify derivation
    assert!(child.has_permission(veridian_kernel::ipc::capability::Permission::Send));
    assert!(!child.has_permission(veridian_kernel::ipc::capability::Permission::Receive));
    
    // Test revocation
    let mut revocable = parent;
    let gen_before = revocable.generation();
    revocable.revoke();
    let gen_after = revocable.generation();
    
    assert_eq!(gen_after, gen_before + 1);
    
    serial_println!("[ok]");
}

#[test_case]
fn test_shared_memory_setup() {
    serial_print!("test_shared_memory_setup... ");
    
    // Create a shared memory region
    let region = SharedRegion::new(
        1, // owner PID
        8192, // 2 pages
        CachePolicy::WriteBack,
        Some(0), // NUMA node 0
    ).unwrap();
    
    // Verify properties
    assert_eq!(region.size(), 8192);
    assert_eq!(region.owner, 1);
    
    // Create memory region descriptor for IPC
    let mem_region = MemoryRegion::new(0x100000, region.size() as u64)
        .with_permissions(0x03) // READ | WRITE
        .with_cache_policy(0); // WRITE_BACK
    
    // Create large message with shared memory
    let large_msg = Message::large(0x5678, 100, mem_region);
    
    assert_eq!(large_msg.capability(), 0x5678);
    assert_eq!(large_msg.opcode(), 100);
    
    serial_println!("[ok]");
}

#[test_case]
fn test_message_size_limits() {
    use veridian_kernel::ipc::message::{SMALL_MESSAGE_MAX_SIZE, DATA_REGISTERS};
    
    serial_print!("test_message_size_limits... ");
    
    // Verify small message size
    assert_eq!(SMALL_MESSAGE_MAX_SIZE, 64);
    assert_eq!(DATA_REGISTERS, 4);
    
    // Small message should be exactly 48 bytes
    let size = core::mem::size_of::<SmallMessage>();
    assert_eq!(size, 48); // 8 + 4 + 4 + (8 * 4)
    
    serial_println!("[ok]");
}

#[test_case]
fn test_zero_copy_flags() {
    use veridian_kernel::ipc::zero_copy::{TransferFlags, TransferType, CachePolicy};
    
    serial_print!("test_zero_copy_flags... ");
    
    let flags = TransferFlags {
        transfer_type: TransferType::Share,
        cache_policy: CachePolicy::Default,
        numa_hint: Some(0),
    };
    
    // Verify we can create transfer flags
    match flags.transfer_type {
        TransferType::Share => {},
        _ => panic!("Wrong transfer type"),
    }
    
    serial_println!("[ok]");
}

#[test_case]
fn test_fast_path_stats() {
    use veridian_kernel::ipc::fast_path::get_fast_path_stats;
    
    serial_print!("test_fast_path_stats... ");
    
    let (count, avg_cycles) = get_fast_path_stats();
    
    // Initially should be zero
    assert_eq!(count, 0);
    assert_eq!(avg_cycles, 0);
    
    serial_println!("[ok]");
}

#[test_case]
fn test_sync_ipc_stats() {
    use veridian_kernel::ipc::sync::get_sync_stats;
    
    serial_print!("test_sync_ipc_stats... ");
    
    let stats = get_sync_stats();
    
    // Verify initial state
    assert_eq!(stats.send_count, 0);
    assert_eq!(stats.receive_count, 0);
    assert_eq!(stats.fast_path_count, 0);
    assert_eq!(stats.slow_path_count, 0);
    assert_eq!(stats.fast_path_percentage, 0);
    
    serial_println!("[ok]");
}

#[test_case]
fn test_zero_copy_stats() {
    use veridian_kernel::ipc::zero_copy::get_zero_copy_stats;
    
    serial_print!("test_zero_copy_stats... ");
    
    let stats = get_zero_copy_stats();
    
    // Verify initial state
    assert_eq!(stats.pages_transferred, 0);
    assert_eq!(stats.bytes_transferred, 0);
    assert_eq!(stats.transfer_count, 0);
    assert_eq!(stats.avg_remap_cycles, 0);
    
    serial_println!("[ok]");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}