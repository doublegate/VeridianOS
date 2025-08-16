//! Driver Framework Test Program
//!
//! Tests driver registration, device enumeration, and device operations.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::{TestProgram, TestResult};
use crate::services::driver_framework::{get_driver_framework, DeviceClass};
use crate::drivers::storage::get_storage_manager;
use crate::drivers::console::get_console_driver;

pub struct DriverTest;

impl DriverTest {
    pub fn new() -> Self {
        Self
    }
    
    fn test_driver_framework(&mut self) -> bool {
        let driver_framework = get_driver_framework();
        
        // Get registered drivers
        let drivers = driver_framework.get_drivers();
        crate::println!("[DRIVER] Found {} registered drivers", drivers.len());
        
        for driver in drivers {
            crate::println!("[DRIVER] Driver: {}", driver.name);
            crate::println!("[DRIVER]   Supported classes: {:?}", driver.supported_classes);
        }
        
        // Should have at least some drivers registered
        !drivers.is_empty()
    }
    
    fn test_device_enumeration(&mut self) -> bool {
        let driver_framework = get_driver_framework();
        
        // Scan for devices
        match driver_framework.scan_devices() {
            Ok(device_count) => {
                crate::println!("[DRIVER] Device scan found {} devices", device_count);
                
                // Get all devices
                let devices = driver_framework.get_devices();
                crate::println!("[DRIVER] Total devices in system: {}", devices.len());
                
                for device in &devices {
                    crate::println!("[DRIVER] Device: {} ({})", device.name, device.bus);
                    crate::println!("[DRIVER]   Class: {:?}, Status: {:?}", device.class, device.status);
                    if let Some(ref driver) = device.driver {
                        crate::println!("[DRIVER]   Driver: {}", driver);
                    }
                }
                
                true
            }
            Err(e) => {
                crate::println!("[DRIVER] Device scan failed: {}", e);
                false
            }
        }
    }
    
    fn test_storage_devices(&mut self) -> bool {
        let storage_manager = get_storage_manager();
        let storage = storage_manager.lock();
        
        // List storage devices
        let devices = storage.list_devices();
        crate::println!("[DRIVER] Found {} storage devices", devices.len());
        
        for device in &devices {
            crate::println!("[DRIVER] Storage device: {}", device.model);
            crate::println!("[DRIVER]   Capacity: {} MB", device.capacity / (1024 * 1024));
            crate::println!("[DRIVER]   Sector size: {} bytes", device.sector_size);
            crate::println!("[DRIVER]   Interface: {:?}", device.interface);
        }
        
        // Get total capacity
        let total_capacity = storage.get_total_capacity();
        crate::println!("[DRIVER] Total storage capacity: {} MB", total_capacity / (1024 * 1024));
        
        true // Storage devices may or may not be present
    }
    
    fn test_console_devices(&mut self) -> bool {
        let console_driver = get_console_driver();
        let mut console = console_driver.lock();
        
        // Test console operations
        if let Some(device) = console.get_active_device() {
            let (width, height) = device.dimensions();
            crate::println!("[DRIVER] Console dimensions: {}x{}", width, height);
            
            let (x, y) = device.get_cursor();
            crate::println!("[DRIVER] Cursor position: ({}, {})", x, y);
            
            // Test writing to console
            match device.write_string(0, 0, "Driver Test Output", 0x07) {
                Ok(_) => {
                    crate::println!("[DRIVER] Console write test successful");
                    true
                }
                Err(e) => {
                    crate::println!("[DRIVER] Console write test failed: {}", e);
                    false
                }
            }
        } else {
            crate::println!("[DRIVER] No active console device");
            false
        }
    }
    
    fn test_device_classes(&mut self) -> bool {
        let driver_framework = get_driver_framework();
        let devices = driver_framework.get_devices();
        
        // Count devices by class
        let mut class_counts = alloc::collections::BTreeMap::new();
        for device in &devices {
            *class_counts.entry(device.class).or_insert(0) += 1;
        }
        
        crate::println!("[DRIVER] Devices by class:");
        for (class, count) in class_counts {
            crate::println!("[DRIVER]   {:?}: {} devices", class, count);
        }
        
        true
    }
    
    fn test_bus_operations(&mut self) -> bool {
        let driver_framework = get_driver_framework();
        
        // Get registered buses
        let buses = driver_framework.get_buses();
        crate::println!("[DRIVER] Found {} registered buses", buses.len());
        
        for bus in buses {
            crate::println!("[DRIVER] Bus: {}", bus.name);
        }
        
        // Should have at least PCI and USB buses
        buses.len() >= 2
    }
}

impl TestProgram for DriverTest {
    fn name(&self) -> &str {
        "driver_test"
    }
    
    fn description(&self) -> &str {
        "Driver framework and device management test"
    }
    
    fn run(&mut self) -> TestResult {
        let mut passed = true;
        let mut messages = Vec::new();
        
        // Test driver framework
        if self.test_driver_framework() {
            messages.push("✓ Driver framework");
        } else {
            messages.push("✗ Driver framework");
            passed = false;
        }
        
        // Test device enumeration
        if self.test_device_enumeration() {
            messages.push("✓ Device enumeration");
        } else {
            messages.push("✗ Device enumeration");
            passed = false;
        }
        
        // Test storage devices
        if self.test_storage_devices() {
            messages.push("✓ Storage devices");
        } else {
            messages.push("✗ Storage devices");
            passed = false;
        }
        
        // Test console devices
        if self.test_console_devices() {
            messages.push("✓ Console devices");
        } else {
            messages.push("✗ Console devices");
            passed = false;
        }
        
        // Test device classes
        if self.test_device_classes() {
            messages.push("✓ Device classes");
        } else {
            messages.push("✗ Device classes");
            passed = false;
        }
        
        // Test bus operations
        if self.test_bus_operations() {
            messages.push("✓ Bus operations");
        } else {
            messages.push("✗ Bus operations");
            passed = false;
        }
        
        TestResult {
            name: self.name().to_string(),
            passed,
            message: messages.join(", "),
        }
    }
}