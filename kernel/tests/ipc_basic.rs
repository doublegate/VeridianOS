//! Basic IPC integration tests

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(veridian_kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;

use veridian_kernel::{
    ipc::{IpcCapability, IpcPermissions, Message, SmallMessage},
    serial_print, serial_println,
};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {}
}

#[test_case]
fn test_small_message_creation() {
    serial_print!("test_small_message_creation... ");

    let msg = SmallMessage::new(0x1234, 42);
    assert_eq!(msg.capability, 0x1234);
    assert_eq!(msg.opcode, 42);
    assert_eq!(msg.flags, 0);

    serial_println!("[ok]");
}

#[test_case]
fn test_message_builder() {
    serial_print!("test_message_builder... ");

    let msg = SmallMessage::new(0x5678, 100)
        .with_flags(0x03)
        .with_data(0, 1000)
        .with_data(1, 2000);

    assert_eq!(msg.capability, 0x5678);
    assert_eq!(msg.opcode, 100);
    assert_eq!(msg.flags, 0x03);
    assert_eq!(msg.data[0], 1000);
    assert_eq!(msg.data[1], 2000);
    assert_eq!(msg.data[2], 0); // Unset data should be 0

    serial_println!("[ok]");
}

#[test_case]
fn test_capability_creation() {
    serial_print!("test_capability_creation... ");

    let cap1 = IpcCapability::new(100, IpcPermissions::all());
    let cap2 = IpcCapability::new(200, IpcPermissions::all());

    // Each capability should have a unique ID
    assert_ne!(cap1.id(), cap2.id());

    // Both should start with generation 0
    assert_eq!(cap1.generation(), 0);
    assert_eq!(cap2.generation(), 0);

    // Targets should match what we specified
    assert_eq!(cap1.target(), 100);
    assert_eq!(cap2.target(), 200);

    serial_println!("[ok]");
}

#[test_case]
fn test_capability_permissions() {
    use veridian_kernel::ipc::capability::Permission;

    serial_print!("test_capability_permissions... ");

    // Test all permissions
    let all_perms = IpcCapability::new(1, IpcPermissions::all());
    assert!(all_perms.has_permission(Permission::Send));
    assert!(all_perms.has_permission(Permission::Receive));
    assert!(all_perms.has_permission(Permission::Share));

    // Test send-only
    let send_only = IpcCapability::new(2, IpcPermissions::send_only());
    assert!(send_only.has_permission(Permission::Send));
    assert!(!send_only.has_permission(Permission::Receive));
    assert!(!send_only.has_permission(Permission::Share));

    // Test receive-only
    let recv_only = IpcCapability::new(3, IpcPermissions::receive_only());
    assert!(!recv_only.has_permission(Permission::Send));
    assert!(recv_only.has_permission(Permission::Receive));
    assert!(!recv_only.has_permission(Permission::Share));

    serial_println!("[ok]");
}

#[test_case]
fn test_capability_derivation() {
    serial_print!("test_capability_derivation... ");

    let parent = IpcCapability::new(1, IpcPermissions::all());

    // Should be able to derive with reduced permissions
    let child = parent.derive(IpcPermissions::send_only());
    assert!(child.is_some());
    let child = child.unwrap();
    assert_eq!(child.target(), parent.target());

    // Should not be able to derive with increased permissions
    let send_only = IpcCapability::new(2, IpcPermissions::send_only());
    let invalid = send_only.derive(IpcPermissions::all());
    assert!(invalid.is_none());

    serial_println!("[ok]");
}

#[test_case]
fn test_message_enum() {
    // Removed unused import

    serial_print!("test_message_enum... ");

    // Test small message
    let small = Message::small(0x1111, 10);
    assert_eq!(small.capability(), 0x1111);
    assert_eq!(small.opcode(), 10);

    // Test large message
    let region = veridian_kernel::ipc::message::MemoryRegion::new(0x200000, 8192);
    let large = Message::large(0x2222, 20, region);
    assert_eq!(large.capability(), 0x2222);
    assert_eq!(large.opcode(), 20);

    serial_println!("[ok]");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    veridian_kernel::test_panic_handler(info)
}
