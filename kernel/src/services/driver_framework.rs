//! Driver Framework Implementation
//!
//! Provides driver registration, device enumeration, and driver-device binding.

#![allow(clippy::unwrap_or_default)]

use alloc::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::RwLock;

use crate::error::KernelError;

/// Device event for hot-plug notifications.
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    /// A new device was added to the system.
    Added(DeviceInfo),
    /// A device was removed from the system.
    Removed(u64),
    /// A device's status changed.
    StateChanged {
        device_id: u64,
        old: DeviceStatus,
        new: DeviceStatus,
    },
}

/// Trait for receiving device hot-plug events.
///
/// Implementors register with the driver framework via
/// [`DriverFramework::register_event_listener`] and will receive callbacks
/// whenever devices are added, removed, or change state.
pub trait DeviceEventListener: Send + Sync {
    /// Called when a device event occurs.
    fn on_event(&self, event: &DeviceEvent);
}

/// Device class
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Character,
    Block,
    Network,
    Input,
    Display,
    Audio,
    USB,
    PCI,
    Storage,
    Serial,
    Other,
}

/// Device identifier
#[derive(Debug, Clone)]
pub struct DeviceId {
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
}

/// Device information
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: u64,
    pub name: String,
    pub class: DeviceClass,
    pub device_id: Option<DeviceId>,
    pub driver: Option<String>,
    pub bus: String,
    pub address: u64,
    pub irq: Option<u8>,
    pub dma_channels: Vec<u8>,
    pub io_ports: Vec<(u16, u16)>,       // (start, end)
    pub memory_regions: Vec<(u64, u64)>, // (start, size)
    pub status: DeviceStatus,
}

/// Device status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    Uninitialized,
    Probing,
    Active,
    Suspended,
    Failed,
    Removed,
}

/// Driver operations trait
pub trait Driver: Send + Sync {
    /// Get driver name
    fn name(&self) -> &str;

    /// Get supported device classes
    fn supported_classes(&self) -> Vec<DeviceClass>;

    /// Check if driver supports a device
    fn supports_device(&self, device: &DeviceInfo) -> bool;

    /// Probe device
    fn probe(&mut self, device: &DeviceInfo) -> Result<(), KernelError>;

    /// Attach to device
    fn attach(&mut self, device: &DeviceInfo) -> Result<(), KernelError>;

    /// Detach from device
    fn detach(&mut self, device: &DeviceInfo) -> Result<(), KernelError>;

    /// Suspend device
    fn suspend(&mut self) -> Result<(), KernelError>;

    /// Resume device
    fn resume(&mut self) -> Result<(), KernelError>;

    /// Handle interrupt
    fn handle_interrupt(&mut self, irq: u8) -> Result<(), KernelError>;

    /// Read from device
    fn read(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize, KernelError>;

    /// Write to device
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<usize, KernelError>;

    /// Device control (ioctl)
    fn ioctl(&mut self, cmd: u32, arg: u64) -> Result<u64, KernelError>;
}

/// Bus operations trait
pub trait Bus: Send + Sync {
    /// Get bus name
    fn name(&self) -> &str;

    /// Scan for devices
    fn scan(&mut self) -> Vec<DeviceInfo>;

    /// Read configuration space
    fn read_config(&self, device: &DeviceInfo, offset: u16, size: u8) -> Result<u32, KernelError>;

    /// Write configuration space
    fn write_config(
        &mut self,
        device: &DeviceInfo,
        offset: u16,
        value: u32,
        size: u8,
    ) -> Result<(), KernelError>;

    /// Enable device
    fn enable_device(&mut self, device: &DeviceInfo) -> Result<(), KernelError>;

    /// Disable device
    fn disable_device(&mut self, device: &DeviceInfo) -> Result<(), KernelError>;
}

/// Driver framework
pub struct DriverFramework {
    /// Registered drivers
    drivers: RwLock<BTreeMap<String, Box<dyn Driver>>>,

    /// Registered buses
    buses: RwLock<BTreeMap<String, Box<dyn Bus>>>,

    /// Discovered devices
    devices: RwLock<BTreeMap<u64, DeviceInfo>>,

    /// Driver-device bindings
    bindings: RwLock<BTreeMap<u64, String>>, // device_id -> driver_name

    /// Next device ID
    next_device_id: AtomicU64,

    /// IRQ handlers
    irq_handlers: RwLock<BTreeMap<u8, Vec<String>>>, // IRQ -> driver names

    /// Event listeners for device hot-plug notifications
    event_listeners: RwLock<Vec<Arc<dyn DeviceEventListener>>>,
}

impl DriverFramework {
    /// Create a new driver framework
    pub fn new() -> Self {
        Self {
            drivers: RwLock::new(BTreeMap::new()),
            buses: RwLock::new(BTreeMap::new()),
            devices: RwLock::new(BTreeMap::new()),
            bindings: RwLock::new(BTreeMap::new()),
            next_device_id: AtomicU64::new(1),
            irq_handlers: RwLock::new(BTreeMap::new()),
            event_listeners: RwLock::new(Vec::new()),
        }
    }
}

impl Default for DriverFramework {
    fn default() -> Self {
        Self::new()
    }
}

impl DriverFramework {
    /// Register a driver
    pub fn register_driver(&self, driver: Box<dyn Driver>) -> Result<(), KernelError> {
        let name: String = driver.name().into();

        if self.drivers.read().contains_key(&name) {
            return Err(KernelError::AlreadyExists {
                resource: "driver",
                id: 0,
            });
        }

        crate::println!("[DRIVER_FRAMEWORK] Registering driver: {}", name);
        self.drivers.write().insert(name.clone(), driver);

        // Try to bind to existing devices
        self.probe_driver(&name)?;

        Ok(())
    }

    /// Unregister a driver
    pub fn unregister_driver(&self, name: &str) -> Result<(), KernelError> {
        // Detach from all devices
        let devices_to_detach: Vec<u64> = self
            .bindings
            .read()
            .iter()
            .filter(|(_, driver)| driver == &name)
            .map(|(device_id, _)| *device_id)
            .collect();

        for device_id in devices_to_detach {
            self.unbind_device(device_id)?;
        }

        self.drivers.write().remove(name);
        crate::println!("[DRIVER_FRAMEWORK] Unregistered driver: {}", name);

        Ok(())
    }

    /// Register a bus
    pub fn register_bus(&self, bus: Box<dyn Bus>) -> Result<(), KernelError> {
        let name = bus.name().into();

        if self.buses.read().contains_key(&name) {
            return Err(KernelError::AlreadyExists {
                resource: "bus",
                id: 0,
            });
        }

        crate::println!("[DRIVER_FRAMEWORK] Registering bus: {}", name);
        self.buses.write().insert(name, bus);

        Ok(())
    }

    /// Scan all buses for devices
    pub fn scan_buses(&self) -> Result<usize, KernelError> {
        let mut total_devices = 0;

        let mut buses = self.buses.write();
        let bus_names: Vec<String> = buses.keys().cloned().collect();

        for bus_name in bus_names {
            if let Some(bus) = buses.get_mut(&bus_name) {
                crate::println!("[DRIVER_FRAMEWORK] Scanning bus: {}", bus_name);
                let devices = bus.scan();

                for mut device in devices {
                    device.id = self.next_device_id.fetch_add(1, Ordering::SeqCst);
                    device.bus = bus_name.clone();

                    crate::println!(
                        "[DRIVER_FRAMEWORK] Found device: {} on {}",
                        device.name,
                        bus_name
                    );

                    self.devices.write().insert(device.id, device.clone());
                    total_devices += 1;

                    // Try to find a driver for this device
                    self.probe_device(device.id)?;
                }
            }
        }

        Ok(total_devices)
    }

    /// Probe a device with all drivers
    fn probe_device(&self, device_id: u64) -> Result<(), KernelError> {
        let device = self
            .devices
            .read()
            .get(&device_id)
            .cloned()
            .ok_or(KernelError::NotFound {
                resource: "device",
                id: device_id,
            })?;

        let mut drivers = self.drivers.write();

        for (driver_name, driver) in drivers.iter_mut() {
            if driver.supports_device(&device) {
                crate::println!(
                    "[DRIVER_FRAMEWORK] Driver {} supports device {}",
                    driver_name,
                    device.name
                );

                // Try to probe
                if driver.probe(&device).is_ok() {
                    // Attach driver
                    if driver.attach(&device).is_ok() {
                        self.bindings.write().insert(device_id, driver_name.clone());

                        // Update device status
                        if let Some(dev) = self.devices.write().get_mut(&device_id) {
                            dev.driver = Some(driver_name.clone());
                            dev.status = DeviceStatus::Active;
                        }

                        // Register IRQ handler if device has IRQ
                        if let Some(irq) = device.irq {
                            self.irq_handlers
                                .write()
                                .entry(irq)
                                .or_insert_with(Vec::new)
                                .push(driver_name.clone());
                        }

                        crate::println!(
                            "[DRIVER_FRAMEWORK] Bound driver {} to device {}",
                            driver_name,
                            device.name
                        );
                        return Ok(());
                    }
                }
            }
        }

        Ok(())
    }

    /// Probe a driver with all devices
    fn probe_driver(&self, driver_name: &str) -> Result<(), KernelError> {
        let devices: Vec<(u64, DeviceInfo)> = self
            .devices
            .read()
            .iter()
            .filter(|(_, dev)| dev.driver.is_none())
            .map(|(id, dev)| (*id, dev.clone()))
            .collect();

        // Check if driver exists first
        {
            let drivers = self.drivers.read();
            if !drivers.contains_key(driver_name) {
                return Err(KernelError::NotFound {
                    resource: "driver",
                    id: 0,
                });
            }
        }

        for (device_id, device) in devices {
            // Check if driver supports device
            let supports = {
                let mut drivers = self.drivers.write();
                if let Some(driver) = drivers.get_mut(driver_name) {
                    driver.supports_device(&device)
                } else {
                    false
                }
            };

            if supports {
                // Try to probe the device
                let probe_ok = {
                    let mut drivers = self.drivers.write();
                    if let Some(driver) = drivers.get_mut(driver_name) {
                        driver.probe(&device).is_ok()
                    } else {
                        false
                    }
                };

                if probe_ok {
                    // Try to attach the device
                    let attach_ok = {
                        let mut drivers = self.drivers.write();
                        if let Some(driver) = drivers.get_mut(driver_name) {
                            driver.attach(&device).is_ok()
                        } else {
                            false
                        }
                    };

                    if attach_ok {
                        self.bindings.write().insert(device_id, driver_name.into());

                        if let Some(dev) = self.devices.write().get_mut(&device_id) {
                            dev.driver = Some(driver_name.into());
                            dev.status = DeviceStatus::Active;
                        }

                        if let Some(irq) = device.irq {
                            self.irq_handlers
                                .write()
                                .entry(irq)
                                .or_insert_with(Vec::new)
                                .push(driver_name.into());
                        }

                        crate::println!(
                            "[DRIVER_FRAMEWORK] Bound driver {} to device {}",
                            driver_name,
                            device.name
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Unbind device from driver
    fn unbind_device(&self, device_id: u64) -> Result<(), KernelError> {
        if let Some(driver_name) = self.bindings.write().remove(&device_id) {
            let device =
                self.devices
                    .read()
                    .get(&device_id)
                    .cloned()
                    .ok_or(KernelError::NotFound {
                        resource: "device",
                        id: device_id,
                    })?;

            if let Some(driver) = self.drivers.write().get_mut(&driver_name) {
                driver.detach(&device)?;
            }

            if let Some(dev) = self.devices.write().get_mut(&device_id) {
                dev.driver = None;
                dev.status = DeviceStatus::Uninitialized;
            }

            // Remove IRQ handler
            if let Some(irq) = device.irq {
                if let Some(handlers) = self.irq_handlers.write().get_mut(&irq) {
                    handlers.retain(|name| name != &driver_name);
                }
            }

            crate::println!(
                "[DRIVER_FRAMEWORK] Unbound device {} from driver {}",
                device.name,
                driver_name
            );
        }

        Ok(())
    }

    /// Handle interrupt
    pub fn handle_interrupt(&self, irq: u8) -> Result<(), KernelError> {
        let handler_names = self
            .irq_handlers
            .read()
            .get(&irq)
            .cloned()
            .unwrap_or_default();

        let mut drivers = self.drivers.write();

        for handler_name in handler_names {
            if let Some(driver) = drivers.get_mut(&handler_name) {
                driver.handle_interrupt(irq)?;
            }
        }

        Ok(())
    }

    /// Get device information
    pub fn get_device(&self, device_id: u64) -> Option<DeviceInfo> {
        self.devices.read().get(&device_id).cloned()
    }

    /// List all devices
    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        self.devices.read().values().cloned().collect()
    }

    /// Get driver for device
    pub fn get_device_driver(&self, device_id: u64) -> Option<String> {
        self.bindings.read().get(&device_id).cloned()
    }

    /// Enable device
    pub fn enable_device(&self, device_id: u64) -> Result<(), KernelError> {
        let device = self
            .devices
            .read()
            .get(&device_id)
            .cloned()
            .ok_or(KernelError::NotFound {
                resource: "device",
                id: device_id,
            })?;

        if let Some(bus) = self.buses.write().get_mut(&device.bus) {
            bus.enable_device(&device)?;

            if let Some(dev) = self.devices.write().get_mut(&device_id) {
                if dev.status == DeviceStatus::Suspended {
                    dev.status = DeviceStatus::Active;
                }
            }
        }

        Ok(())
    }

    /// Disable device
    pub fn disable_device(&self, device_id: u64) -> Result<(), KernelError> {
        let device = self
            .devices
            .read()
            .get(&device_id)
            .cloned()
            .ok_or(KernelError::NotFound {
                resource: "device",
                id: device_id,
            })?;

        if let Some(bus) = self.buses.write().get_mut(&device.bus) {
            bus.disable_device(&device)?;

            if let Some(dev) = self.devices.write().get_mut(&device_id) {
                dev.status = DeviceStatus::Suspended;
            }
        }

        Ok(())
    }

    /// Get all registered drivers
    pub fn get_drivers(&self) -> Vec<String> {
        self.drivers.read().keys().cloned().collect()
    }

    /// Register an event listener for device hot-plug notifications.
    pub fn register_event_listener(&self, listener: Arc<dyn DeviceEventListener>) {
        self.event_listeners.write().push(listener);
    }

    /// Unregister all event listeners (used during shutdown).
    pub fn clear_event_listeners(&self) {
        self.event_listeners.write().clear();
    }

    /// Notify all registered listeners of a device event.
    fn notify_listeners(&self, event: &DeviceEvent) {
        let listeners = self.event_listeners.read();
        for listener in listeners.iter() {
            listener.on_event(event);
        }
    }

    /// Add a new device to the system (hot-plug).
    ///
    /// Assigns a unique device ID, registers the device, fires an `Added`
    /// event, and attempts to auto-probe a matching driver.
    pub fn add_device(&self, mut device: DeviceInfo) -> Result<u64, KernelError> {
        device.id = self.next_device_id.fetch_add(1, Ordering::SeqCst);
        let device_id = device.id;

        crate::println!(
            "[DRIVER_FRAMEWORK] Hot-plug: adding device {} (id={})",
            device.name,
            device_id
        );

        self.devices.write().insert(device_id, device.clone());

        // Fire Added event
        self.notify_listeners(&DeviceEvent::Added(device));

        // Try to find a driver for this device
        self.probe_device(device_id)?;

        Ok(device_id)
    }

    /// Remove a device from the system (hot-unplug).
    ///
    /// Unbinds any attached driver, fires a `Removed` event, and removes the
    /// device from the registry.
    pub fn remove_device(&self, device_id: u64) -> Result<(), KernelError> {
        // Unbind driver if bound
        if self.bindings.read().contains_key(&device_id) {
            self.unbind_device(device_id)?;
        }

        // Remove device and fire event
        let removed = self.devices.write().remove(&device_id);

        if let Some(_dev) = removed {
            crate::println!(
                "[DRIVER_FRAMEWORK] Hot-unplug: removed device (id={})",
                device_id
            );
            self.notify_listeners(&DeviceEvent::Removed(device_id));
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "device",
                id: device_id,
            })
        }
    }

    /// Update a device's status and fire a StateChanged event.
    pub fn update_device_status(
        &self,
        device_id: u64,
        new_status: DeviceStatus,
    ) -> Result<(), KernelError> {
        let old_status = {
            let mut devices = self.devices.write();
            let dev = devices.get_mut(&device_id).ok_or(KernelError::NotFound {
                resource: "device",
                id: device_id,
            })?;
            let old = dev.status;
            dev.status = new_status;
            old
        };

        if old_status != new_status {
            self.notify_listeners(&DeviceEvent::StateChanged {
                device_id,
                old: old_status,
                new: new_status,
            });
        }

        Ok(())
    }

    /// Read from a device through its bound driver.
    pub fn read_device(
        &self,
        device_id: u64,
        offset: u64,
        buffer: &mut [u8],
    ) -> Result<usize, KernelError> {
        let driver_name =
            self.bindings
                .read()
                .get(&device_id)
                .cloned()
                .ok_or(KernelError::NotFound {
                    resource: "device binding",
                    id: device_id,
                })?;

        let mut drivers = self.drivers.write();
        let driver = drivers.get_mut(&driver_name).ok_or(KernelError::NotFound {
            resource: "driver",
            id: 0,
        })?;

        driver.read(offset, buffer)
    }

    /// Write to a device through its bound driver.
    pub fn write_device(
        &self,
        device_id: u64,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, KernelError> {
        let driver_name =
            self.bindings
                .read()
                .get(&device_id)
                .cloned()
                .ok_or(KernelError::NotFound {
                    resource: "device binding",
                    id: device_id,
                })?;

        let mut drivers = self.drivers.write();
        let driver = drivers.get_mut(&driver_name).ok_or(KernelError::NotFound {
            resource: "driver",
            id: 0,
        })?;

        driver.write(offset, data)
    }

    /// Get statistics
    pub fn get_statistics(&self) -> DriverFrameworkStats {
        let devices = self.devices.read();
        let mut active = 0;
        let mut failed = 0;
        let mut suspended = 0;

        for device in devices.values() {
            match device.status {
                DeviceStatus::Active => active += 1,
                DeviceStatus::Failed => failed += 1,
                DeviceStatus::Suspended => suspended += 1,
                _ => {}
            }
        }

        DriverFrameworkStats {
            total_drivers: self.drivers.read().len(),
            total_buses: self.buses.read().len(),
            total_devices: devices.len(),
            bound_devices: self.bindings.read().len(),
            active_devices: active,
            failed_devices: failed,
            suspended_devices: suspended,
        }
    }
}

/// Driver framework statistics
#[derive(Debug)]
pub struct DriverFrameworkStats {
    pub total_drivers: usize,
    pub total_buses: usize,
    pub total_devices: usize,
    pub bound_devices: usize,
    pub active_devices: usize,
    pub failed_devices: usize,
    pub suspended_devices: usize,
}

/// Global driver framework using OnceLock for safe initialization.
static DRIVER_FRAMEWORK: crate::sync::once_lock::OnceLock<DriverFramework> =
    crate::sync::once_lock::OnceLock::new();

/// Initialize the driver framework
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    println!("[DRIVER_FRAMEWORK] Creating new DriverFramework...");
    match DRIVER_FRAMEWORK.set(DriverFramework::new()) {
        Ok(()) => println!("[DRIVER_FRAMEWORK] Driver framework initialized"),
        Err(_) => println!("[DRIVER_FRAMEWORK] Already initialized, skipping..."),
    }
}

/// Try to get the global driver framework without panicking.
///
/// Returns `None` if the driver framework has not been initialized via
/// [`init`].
pub fn try_get_driver_framework() -> Option<&'static DriverFramework> {
    DRIVER_FRAMEWORK.get()
}

/// Get the global driver framework.
///
/// Panics if the driver framework has not been initialized via [`init`].
/// Prefer [`try_get_driver_framework`] in contexts where a panic is
/// unacceptable.
pub fn get_driver_framework() -> &'static DriverFramework {
    DRIVER_FRAMEWORK
        .get()
        .expect("Driver framework not initialized: init() was not called")
}
