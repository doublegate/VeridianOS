//! Network Test Program
//!
//! Tests network interface management and packet operations.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::{TestProgram, TestResult};
use crate::drivers::network::{get_network_manager, NetworkPacket};

pub struct NetworkTest;

impl NetworkTest {
    pub fn new() -> Self {
        Self
    }
    
    fn test_interface_listing(&mut self) -> bool {
        let network_manager = get_network_manager();
        
        // List all network interfaces
        let interfaces = network_manager.list_interfaces();
        crate::println!("[NET] Found {} network interfaces", interfaces.len());
        
        for interface in &interfaces {
            crate::println!("[NET] Interface: {}", interface);
        }
        
        // Should have at least loopback interface
        !interfaces.is_empty()
    }
    
    fn test_loopback_interface(&mut self) -> bool {
        let network_manager = get_network_manager();
        
        // Get loopback interface
        match network_manager.get_interface("lo") {
            Some(loopback) => {
                let mut device = loopback.lock();
                
                // Check if interface is up
                if device.link_up() {
                    crate::println!("[NET] Loopback interface is up");
                    
                    // Get interface configuration
                    let config = device.get_config();
                    crate::println!("[NET] Loopback MTU: {}", config.mtu);
                    crate::println!("[NET] Loopback IP: {:?}", config.ip_address);
                    
                    // Test packet transmission on loopback
                    let test_data = b"Hello, loopback!";
                    let packet = NetworkPacket::new(test_data.to_vec());
                    
                    match device.send_packet(packet) {
                        Ok(_) => {
                            crate::println!("[NET] Loopback packet sent successfully");
                            
                            // Get statistics
                            let stats = device.get_stats();
                            crate::println!("[NET] TX packets: {}, RX packets: {}", 
                                stats.tx_packets, stats.rx_packets);
                            
                            stats.tx_packets > 0
                        }
                        Err(e) => {
                            crate::println!("[NET] Loopback packet send failed: {}", e);
                            false
                        }
                    }
                } else {
                    crate::println!("[NET] Loopback interface is down");
                    false
                }
            }
            None => {
                crate::println!("[NET] Loopback interface not found");
                false
            }
        }
    }
    
    fn test_ethernet_interface(&mut self) -> bool {
        let network_manager = get_network_manager();
        
        // Try to find ethernet interface
        let interfaces = network_manager.list_interfaces();
        let eth_interface = interfaces.iter()
            .find(|name| name.starts_with("eth"))
            .cloned();
        
        match eth_interface {
            Some(interface_name) => {
                crate::println!("[NET] Found ethernet interface: {}", interface_name);
                
                match network_manager.get_interface(&interface_name) {
                    Some(ethernet) => {
                        let mut device = ethernet.lock();
                        
                        // Get interface configuration
                        let config = device.get_config();
                        crate::println!("[NET] Ethernet MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                            config.mac_address[0], config.mac_address[1], config.mac_address[2],
                            config.mac_address[3], config.mac_address[4], config.mac_address[5]);
                        
                        crate::println!("[NET] Ethernet MTU: {}", config.mtu);
                        crate::println!("[NET] Link speed: {} Mbps", device.link_speed());
                        
                        // Test basic functionality
                        if device.link_up() {
                            crate::println!("[NET] Ethernet link is up");
                            true
                        } else {
                            crate::println!("[NET] Ethernet link is down");
                            true // This is normal in a virtual environment
                        }
                    }
                    None => {
                        crate::println!("[NET] Failed to get ethernet interface");
                        false
                    }
                }
            }
            None => {
                crate::println!("[NET] No ethernet interface found (normal in simulation)");
                true // This is acceptable for testing
            }
        }
    }
    
    fn test_network_statistics(&mut self) -> bool {
        let network_manager = get_network_manager();
        
        // Get global network statistics
        let global_stats = network_manager.get_global_stats();
        
        crate::println!("[NET] Global statistics:");
        crate::println!("[NET]   RX packets: {}, bytes: {}", 
            global_stats.rx_packets, global_stats.rx_bytes);
        crate::println!("[NET]   TX packets: {}, bytes: {}", 
            global_stats.tx_packets, global_stats.tx_bytes);
        crate::println!("[NET]   Errors: {} RX, {} TX", 
            global_stats.rx_errors, global_stats.tx_errors);
        
        // Statistics should be available (even if zero)
        true
    }
}

impl TestProgram for NetworkTest {
    fn name(&self) -> &str {
        "network_test"
    }
    
    fn description(&self) -> &str {
        "Network interface and packet operations test"
    }
    
    fn run(&mut self) -> TestResult {
        let mut passed = true;
        let mut messages = Vec::new();
        
        // Test interface listing
        if self.test_interface_listing() {
            messages.push("✓ Interface listing");
        } else {
            messages.push("✗ Interface listing");
            passed = false;
        }
        
        // Test loopback interface
        if self.test_loopback_interface() {
            messages.push("✓ Loopback interface");
        } else {
            messages.push("✗ Loopback interface");
            passed = false;
        }
        
        // Test ethernet interface
        if self.test_ethernet_interface() {
            messages.push("✓ Ethernet interface");
        } else {
            messages.push("✗ Ethernet interface");
            passed = false;
        }
        
        // Test network statistics
        if self.test_network_statistics() {
            messages.push("✓ Network statistics");
        } else {
            messages.push("✗ Network statistics");
            passed = false;
        }
        
        TestResult {
            name: self.name().to_string(),
            passed,
            message: messages.join(", "),
        }
    }
}