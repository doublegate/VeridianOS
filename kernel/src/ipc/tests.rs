//! IPC subsystem tests

use super::*;
use crate::ipc::message::{flags, permissions, LargeMessage, SmallMessage};

#[test]
fn test_small_message_creation() {
    let msg = SmallMessage::new(0x1234, 42)
        .with_flags(flags::URGENT | flags::NEEDS_ACK)
        .with_data(0, 100)
        .with_data(1, 200);

    assert_eq!(msg.capability, 0x1234);
    assert_eq!(msg.opcode, 42);
    assert_eq!(msg.flags, flags::URGENT | flags::NEEDS_ACK);
    assert_eq!(msg.data[0], 100);
    assert_eq!(msg.data[1], 200);
}

#[test]
fn test_message_enum() {
    let small = Message::small(0x1000, 1);
    assert_eq!(small.capability(), 0x1000);
    assert_eq!(small.opcode(), 1);

    let region =
        MemoryRegion::new(0x2000, 4096).with_permissions(permissions::READ | permissions::WRITE);
    let large = Message::large(0x2000, 2, region);
    assert_eq!(large.capability(), 0x2000);
    assert_eq!(large.opcode(), 2);
}

#[test]
fn test_capability_generation() {
    let cap1 = IpcCapability::new(100, IpcPermissions::all());
    let cap2 = IpcCapability::new(200, IpcPermissions::all());

    // Should have different IDs
    assert_ne!(cap1.id(), cap2.id());

    // Should have same generation (0)
    assert_eq!(cap1.generation(), 0);
    assert_eq!(cap2.generation(), 0);
}

#[test]
fn test_capability_permissions() {
    let perms = IpcPermissions {
        can_send: true,
        can_receive: false,
        can_share: false,
        max_message_size: 1024,
    };

    let cap = IpcCapability::new(1, perms);
    assert!(cap.has_permission(Permission::Send));
    assert!(!cap.has_permission(Permission::Receive));
    assert!(!cap.has_permission(Permission::Share));
}

#[test]
fn test_capability_derivation() {
    let parent = IpcCapability::new(1, IpcPermissions::all());

    // Valid derivation - reducing permissions
    let child_perms = IpcPermissions::send_only();
    let child = parent.derive(child_perms);
    assert!(child.is_some());

    // Invalid derivation - trying to add permissions
    let invalid_perms = IpcPermissions {
        can_send: true,
        can_receive: true,
        can_share: true,
        max_message_size: usize::MAX,
    };
    let send_only = IpcCapability::new(2, IpcPermissions::send_only());
    let invalid = send_only.derive(invalid_perms);
    assert!(invalid.is_none());
}

#[test]
fn test_endpoint_async_communication() {
    let endpoint = Endpoint::new(1);

    // Send multiple messages
    for i in 0..5 {
        let msg = Message::small(1000 + i, i as u32);
        assert!(endpoint.send_async(msg).is_ok());
    }

    // Receive them in order
    for i in 0..5 {
        let msg = endpoint.try_receive().unwrap();
        assert_eq!(msg.capability(), 1000 + i);
        assert_eq!(msg.opcode(), i as u32);
    }

    // Queue should be empty
    assert_eq!(endpoint.try_receive().err(), Some(IpcError::ChannelEmpty));
}

#[test]
fn test_channel_bidirectional() {
    let channel = Channel::new(1, 100);

    // Send on one end
    let msg1 = Message::small(0x1111, 10);
    assert!(channel.send(msg1).is_ok());

    // Receive on the other
    let received = channel.receive().unwrap();
    assert_eq!(received.capability(), 0x1111);
    assert_eq!(received.opcode(), 10);
}

#[test]
fn test_shared_region_mapping() {
    let region = SharedRegion::new(1, 8192, CachePolicy::WriteBack, None).unwrap();

    // Map to process 2
    let vaddr = VirtualAddress::new(0x1000_0000);
    assert!(region.map(2, vaddr, Permission::Read).is_ok());

    // Should be able to get mapping
    assert_eq!(region.get_mapping(2), Some(vaddr));

    // Can't map same process twice
    let vaddr2 = VirtualAddress::new(0x2000_0000);
    assert!(region.map(2, vaddr2, Permission::Write).is_err());
}

#[test]
fn test_memory_region_permissions() {
    assert_eq!(Permission::Read as u32, 0b001);
    assert_eq!(Permission::Write as u32, 0b011);
    assert_eq!(Permission::Execute as u32, 0b100);
    assert_eq!(Permission::ReadWriteExecute as u32, 0b111);
}

#[test]
fn test_ipc_error_codes() {
    assert_eq!(IpcError::InvalidCapability.to_errno(), -1);
    assert_eq!(IpcError::ProcessNotFound.to_errno(), -2);
    assert_eq!(IpcError::Timeout.to_errno(), -8);
}
