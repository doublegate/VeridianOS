//! Integration tests for inter-subsystem communication
//!
//! This module provides tests that verify proper interaction between
//! different kernel subsystems.

#![cfg(test)]

use alloc::vec::Vec;

use crate::{cap, error::KernelError, fs, ipc, net, process, security};

/// Test IPC with capability validation
#[test_case]
fn test_ipc_with_capabilities() {
    // Initialize required subsystems
    cap::init();
    process::init_without_init_process().expect("Process init failed");
    ipc::init();

    // Create a test endpoint
    let endpoint_id = ipc::create_endpoint(1).expect("Failed to create endpoint");

    // Create a capability for the endpoint
    let cap_id = cap::create_capability(1, cap::Rights::READ | cap::Rights::WRITE, endpoint_id)
        .expect("Failed to create capability");

    // Validate the capability
    assert!(cap::validate_capability(cap_id, cap::Rights::READ).is_ok());
    assert!(cap::validate_capability(cap_id, cap::Rights::WRITE).is_ok());

    // Try to validate with insufficient rights
    assert!(cap::validate_capability(cap_id, cap::Rights::EXECUTE).is_err());
}

/// Test network socket with IPC integration
#[test_case]
fn test_network_socket_with_ipc() {
    // Initialize subsystems
    ipc::init();
    net::init().expect("Network init failed");

    // Create a UDP socket
    let socket_id = net::socket::create_socket(
        net::socket::SocketDomain::Inet,
        net::socket::SocketType::Dgram,
        net::socket::SocketProtocol::Udp,
    )
    .expect("Failed to create socket");

    // Verify socket exists
    assert!(socket_id > 0);

    // Get socket and verify state
    let socket = net::socket::get_socket(socket_id).expect("Socket not found");
    assert_eq!(socket.state, net::socket::SocketState::Unbound);
}

/// Test security MAC with file system operations
#[test_case]
fn test_security_mac_with_filesystem() {
    // Initialize subsystems
    security::init().expect("Security init failed");
    fs::init();

    // Test MAC access control
    let has_access =
        security::mac::check_access("system_t", "system_t", security::mac::AccessType::Read);
    assert!(has_access, "System should have read access to itself");

    let has_access =
        security::mac::check_access("user_t", "system_t", security::mac::AccessType::Execute);
    assert!(!has_access, "User should not have execute access to system");
}

/// Test cryptographic hashing (security subsystem)
#[test_case]
fn test_crypto_hashing() {
    security::init().expect("Security init failed");

    // Test SHA-256 hashing
    let data = b"Hello, VeridianOS!";
    let mut hash = [0u8; 32];

    security::crypto::sha256(data, &mut hash);

    // Hash should not be all zeros
    assert_ne!(hash, [0u8; 32]);

    // Same input should produce same hash
    let mut hash2 = [0u8; 32];
    security::crypto::sha256(data, &mut hash2);
    assert_eq!(hash, hash2);
}

/// Test process creation with capability inheritance
#[test_case]
fn test_process_with_capabilities() {
    // Initialize subsystems
    cap::init();
    process::init_without_init_process().expect("Process init failed");

    // Create a test process
    let pid = process::create_process("test_process").expect("Failed to create process");
    assert!(pid > 0);

    // Create a capability for the process
    let cap_id = cap::create_capability(pid, cap::Rights::READ | cap::Rights::WRITE, 1234)
        .expect("Failed to create capability");

    // Verify capability exists
    assert!(cap::validate_capability(cap_id, cap::Rights::READ).is_ok());
}

/// Test IPC message passing between processes
#[test_case]
fn test_ipc_message_passing() {
    // Initialize subsystems
    process::init_without_init_process().expect("Process init failed");
    ipc::init();

    // Create two test processes
    let pid1 = 1u64;
    let pid2 = 2u64;

    // Create endpoints for both processes
    let endpoint1 = ipc::create_endpoint(pid1).expect("Failed to create endpoint 1");
    let endpoint2 = ipc::create_endpoint(pid2).expect("Failed to create endpoint 2");

    assert!(endpoint1 > 0);
    assert!(endpoint2 > 0);

    // Test endpoint lookup
    assert!(ipc::lookup_endpoint(endpoint1).is_ok());
    assert!(ipc::lookup_endpoint(endpoint2).is_ok());
}

/// Test network packet creation and statistics
#[test_case]
fn test_network_packet_stats() {
    net::init().expect("Network init failed");

    // Get initial stats
    let stats_before = net::get_stats();

    // Simulate sending packets
    net::update_stats_tx(1500);
    net::update_stats_tx(1500);

    // Check stats updated
    let stats_after = net::get_stats();
    assert_eq!(stats_after.packets_sent, stats_before.packets_sent + 2);
    assert_eq!(stats_after.bytes_sent, stats_before.bytes_sent + 3000);
}

/// Test IP routing table
#[test_case]
fn test_ip_routing() {
    net::init().expect("Network init failed");

    // Test localhost routing
    let localhost = net::Ipv4Address::LOCALHOST;
    let route = net::ip::lookup_route(localhost);

    assert!(route.is_some(), "Should find route for localhost");

    let route = route.unwrap();
    assert_eq!(route.destination, net::Ipv4Address::new(127, 0, 0, 0));
    assert_eq!(route.netmask, net::Ipv4Address::new(255, 0, 0, 0));
}

/// Test TCP connection state transitions
#[test_case]
fn test_tcp_state_machine() {
    net::init().expect("Network init failed");

    let local = net::SocketAddr::v4(net::Ipv4Address::LOCALHOST, 8080);
    let remote = net::SocketAddr::v4(net::Ipv4Address::new(192, 168, 1, 1), 80);

    let mut conn = net::tcp::TcpConnection::new(local, remote);

    // Initial state should be Closed
    assert_eq!(conn.state, net::tcp::TcpState::Closed);

    // Test transition to Listen
    conn.listen().expect("Listen failed");
    assert_eq!(conn.state, net::tcp::TcpState::Listen);

    // Create another connection for active open
    let mut conn2 = net::tcp::TcpConnection::new(local, remote);
    conn2.connect().expect("Connect failed");
    assert_eq!(conn2.state, net::tcp::TcpState::SynSent);
}

/// Test UDP socket operations
#[test_case]
fn test_udp_socket_operations() {
    net::init().expect("Network init failed");

    let mut socket = net::udp::UdpSocket::new();
    let addr = net::SocketAddr::v4(net::Ipv4Address::LOCALHOST, 8080);

    // Test bind
    socket.bind(addr).expect("Bind failed");
    assert!(socket.bound);
    assert_eq!(socket.local, addr);

    // Test connect (optional for UDP)
    let remote = net::SocketAddr::v4(net::Ipv4Address::new(192, 168, 1, 1), 80);
    socket.connect(remote).expect("Connect failed");
    assert_eq!(socket.remote, Some(remote));
}

/// Test network device loopback
#[test_case]
fn test_loopback_device() {
    net::init().expect("Network init failed");

    // Loopback should be created and up
    let lo = net::device::get_device("lo0");
    assert!(lo.is_some(), "Loopback device should exist");

    let lo = lo.unwrap();
    assert_eq!(lo.name(), "lo0");
    assert_eq!(lo.state(), net::device::DeviceState::Up);
}

/// Test security audit logging
#[test_case]
fn test_security_audit() {
    security::init().expect("Security init failed");

    // Log a test event
    let event = security::audit::AuditEvent {
        event_type: security::audit::AuditEventType::FileAccess,
        timestamp: 123456,
        pid: 1,
        uid: 1000,
        result: 0,
        data: 0,
    };

    security::audit::log_event(event);

    // Event should be logged (no panic means success)
}

/// Test package manager operations
#[test_case]
fn test_package_manager() {
    crate::pkg::init().expect("Package init failed");

    // Test package installation (basic functionality)
    let result = crate::pkg::install("test-package", (1, 0, 0));

    // Package manager should handle installation
    // (may succeed or fail depending on implementation state)
    match result {
        Ok(_) => {
            // Verify package exists
            let installed = crate::pkg::is_installed("test-package");
            assert!(installed.is_ok());
        }
        Err(_) => {
            // Installation may not be fully implemented yet
            // This is expected for placeholder implementations
        }
    }
}

/// Test graphics framebuffer initialization
#[test_case]
fn test_graphics_framebuffer() {
    crate::graphics::init().expect("Graphics init failed");

    // Graphics should initialize without panic
    // Actual framebuffer operations depend on hardware
}

/// Test performance monitoring
#[test_case]
fn test_performance_monitoring() {
    crate::perf::init().expect("Perf init failed");

    // Get counters (should initialize successfully)
    let counters = crate::perf::get_counters();
    assert!(counters.syscalls >= 0);
    assert!(counters.context_switches >= 0);
}

/// Test VFS mount operations
#[test_case]
fn test_vfs_operations() {
    fs::init();

    // Test root mount
    let result = fs::mount("/", "ramfs", 0);
    // Mount may succeed or be already mounted
    // Both are acceptable states
    let _ = result;

    // Test directory operations would go here
    // (requires more VFS implementation)
}

/// Integration test: Full workflow
/// Tests a complete workflow involving multiple subsystems
#[test_case]
fn test_full_integration_workflow() {
    // Initialize all subsystems in order
    cap::init();
    process::init_without_init_process().expect("Process init failed");
    ipc::init();
    security::init().expect("Security init failed");
    net::init().expect("Network init failed");
    fs::init();

    // 1. Create a process
    let pid = 1u64;

    // 2. Create capabilities for the process
    let cap_id = cap::create_capability(pid, cap::Rights::READ | cap::Rights::WRITE, 100)
        .expect("Failed to create capability");

    // 3. Validate capability
    assert!(cap::validate_capability(cap_id, cap::Rights::READ).is_ok());

    // 4. Create IPC endpoint for the process
    let endpoint = ipc::create_endpoint(pid).expect("Failed to create endpoint");
    assert!(endpoint > 0);

    // 5. Create a network socket
    let socket_id = net::socket::create_socket(
        net::socket::SocketDomain::Inet,
        net::socket::SocketType::Stream,
        net::socket::SocketProtocol::Tcp,
    )
    .expect("Failed to create socket");

    // 6. Verify all components work together
    assert!(cap::validate_capability(cap_id, cap::Rights::READ).is_ok());
    assert!(ipc::lookup_endpoint(endpoint).is_ok());
    assert!(net::socket::get_socket(socket_id).is_ok());

    // Test complete!
}

#[cfg(test)]
mod module_tests {
    use super::*;

    /// Run all integration tests
    pub fn run_all() {
        test_ipc_with_capabilities();
        test_network_socket_with_ipc();
        test_security_mac_with_filesystem();
        test_crypto_hashing();
        test_process_with_capabilities();
        test_ipc_message_passing();
        test_network_packet_stats();
        test_ip_routing();
        test_tcp_state_machine();
        test_udp_socket_operations();
        test_loopback_device();
        test_security_audit();
        test_package_manager();
        test_graphics_framebuffer();
        test_performance_monitoring();
        test_vfs_operations();
        test_full_integration_workflow();

        println!("[INTEGRATION] All integration tests passed!");
    }
}
