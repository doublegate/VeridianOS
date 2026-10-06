//! Intel E1000 (82540EM) Network Driver
//!
//! Supports the Intel E1000 Gigabit Ethernet controller found in QEMU and
//! VirtualBox virtual machines.
//!
//! DMA rules (DRV-SEC-02): descriptor rings and packet buffers live in frames
//! from the frame allocator and the NIC is only given their physical
//! addresses. The previous driver kept rings and buffers inside the driver
//! struct -- built on the stack and then moved -- and programmed their
//! virtual addresses into the NIC, so the device wrote to arbitrary memory.
//! Descriptor fields the device writes are accessed with volatile
//! operations, and the device is reset before any DMA memory is freed.

use alloc::vec::Vec;
use core::mem::ManuallyDrop;

use crate::{
    drivers::dma_frame::DmaFrame,
    error::KernelError,
    net::{
        device::{DeviceCapabilities, DeviceState, DeviceStatistics, NetworkDevice},
        MacAddress, Packet,
    },
};

/// E1000 PCI vendor and device IDs
pub const E1000_VENDOR_ID: u16 = 0x8086;
pub const E1000_DEVICE_ID: u16 = 0x100E;

/// E1000 register offsets
const REG_CTRL: usize = 0x0000; // Device Control
const REG_EEPROM: usize = 0x0014; // EEPROM Read
const REG_ICR: usize = 0x00C0; // Interrupt Cause Read
const REG_IMC: usize = 0x00D8; // Interrupt Mask Clear
const REG_RCTL: usize = 0x0100; // Receive Control
const REG_TCTL: usize = 0x0400; // Transmit Control
const REG_RDBAL: usize = 0x2800; // RX Descriptor Base Low
const REG_RDBAH: usize = 0x2804; // RX Descriptor Base High
const REG_RDLEN: usize = 0x2808; // RX Descriptor Length
const REG_RDH: usize = 0x2810; // RX Descriptor Head
const REG_RDT: usize = 0x2818; // RX Descriptor Tail
const REG_TDBAL: usize = 0x3800; // TX Descriptor Base Low
const REG_TDBAH: usize = 0x3804; // TX Descriptor Base High
const REG_TDLEN: usize = 0x3808; // TX Descriptor Length
const REG_TDH: usize = 0x3810; // TX Descriptor Head
const REG_TDT: usize = 0x3818; // TX Descriptor Tail
const REG_MTA: usize = 0x5200; // Multicast Table Array

/// CTRL: device reset.
const CTRL_RST: u32 = 1 << 26;
/// CTRL: set link up.
const CTRL_SLU: u32 = 1 << 6;
/// RCTL: enable, accept broadcast, strip CRC (2048-byte buffers).
const RCTL_VALUE: u32 = (1 << 1) | (1 << 15) | (1 << 26);
/// TCTL: enable, pad short packets, collision threshold.
const TCTL_VALUE: u32 = (1 << 1) | (1 << 3) | (0x10 << 4);

/// Number of RX/TX descriptors (ring lengths must be multiples of 128
/// bytes, i.e. of 8 descriptors).
const NUM_RX_DESC: usize = 32;
const NUM_TX_DESC: usize = 8;
/// Bytes per packet buffer (RCTL buffer size 2048).
const BUF_SIZE: usize = 2048;
/// Descriptor status: descriptor done.
const STATUS_DD: u8 = 1;
/// TX command: end of packet, insert FCS, report status.
const TX_CMD: u8 = (1 << 0) | (1 << 1) | (1 << 3);

/// Legacy receive descriptor (16 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct RxDescriptor {
    addr: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

/// Legacy transmit descriptor (16 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct TxDescriptor {
    addr: u64,
    length: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

/// E1000 driver state.
pub struct E1000Driver {
    mmio_base: usize,
    mac_address: MacAddress,
    /// One frame: RX descriptors at offset 0, TX descriptors after them.
    /// `ManuallyDrop` so `Drop` can leak it when the device did not leave
    /// reset and may still DMA into it.
    rings: ManuallyDrop<DmaFrame>,
    /// Packet buffers, two per frame: buffer `i` is half `i % 2` of frame
    /// `i / 2`.
    rx_frames: Vec<DmaFrame>,
    tx_frames: Vec<DmaFrame>,
    rx_current: usize,
    tx_current: usize,
    name: alloc::string::String,
    state: DeviceState,
    stats: DeviceStatistics,
}

// SAFETY: the driver is used only behind the network device registry lock;
// the DMA memory it points at is owned by it.
unsafe impl Send for E1000Driver {}

const TX_RING_OFFSET: usize = NUM_RX_DESC * core::mem::size_of::<RxDescriptor>();

impl E1000Driver {
    /// Initialize an E1000 whose registers are mapped at `mmio_base`.
    pub fn new(mmio_base: usize) -> Result<Self, KernelError> {
        let rings = DmaFrame::alloc()?;
        let rx_frames = (0..NUM_RX_DESC / 2)
            .map(|_| DmaFrame::alloc())
            .collect::<Result<Vec<_>, _>>()?;
        let tx_frames = (0..NUM_TX_DESC / 2)
            .map(|_| DmaFrame::alloc())
            .collect::<Result<Vec<_>, _>>()?;

        let mut driver = Self {
            mmio_base,
            mac_address: MacAddress::ZERO,
            rings: ManuallyDrop::new(rings),
            rx_frames,
            tx_frames,
            rx_current: 0,
            tx_current: 0,
            name: crate::net::device::alloc_ethernet_name(),
            state: DeviceState::Down,
            stats: DeviceStatistics::default(),
        };
        driver.initialize()?;
        Ok(driver)
    }

    fn read_reg(&self, offset: usize) -> u32 {
        // SAFETY: mmio_base maps the controller's BAR0 register window and
        // `offset` is a register offset within it.
        unsafe { core::ptr::read_volatile((self.mmio_base + offset) as *const u32) }
    }

    fn write_reg(&self, offset: usize, value: u32) {
        // SAFETY: as read_reg.
        unsafe { core::ptr::write_volatile((self.mmio_base + offset) as *mut u32, value) }
    }

    fn rx_desc(&self, i: usize) -> *mut RxDescriptor {
        debug_assert!(i < NUM_RX_DESC);
        (self.rings.virt + i * core::mem::size_of::<RxDescriptor>()) as *mut RxDescriptor
    }

    fn tx_desc(&self, i: usize) -> *mut TxDescriptor {
        debug_assert!(i < NUM_TX_DESC);
        (self.rings.virt + TX_RING_OFFSET + i * core::mem::size_of::<TxDescriptor>())
            as *mut TxDescriptor
    }

    /// (virtual, physical) address of buffer `i` within `frames`.
    fn buffer(frames: &[DmaFrame], i: usize) -> (usize, u64) {
        let f = &frames[i / 2];
        let off = (i % 2) * BUF_SIZE;
        (f.virt + off, f.phys + off as u64)
    }

    fn read_mac_address(&self) -> MacAddress {
        let mut mac = [0u8; 6];
        for i in 0usize..3 {
            let word = self.eeprom_read(i as u8);
            mac[i * 2] = (word & 0xFF) as u8;
            mac[i * 2 + 1] = (word >> 8) as u8;
        }
        MacAddress(mac)
    }

    fn eeprom_read(&self, addr: u8) -> u16 {
        self.write_reg(REG_EEPROM, 1 | ((addr as u32) << 8));
        // Bounded wait for the DONE bit.
        for _ in 0..100_000 {
            let result = self.read_reg(REG_EEPROM);
            if result & (1 << 4) != 0 {
                return ((result >> 16) & 0xFFFF) as u16;
            }
            core::hint::spin_loop();
        }
        0
    }

    /// Reset the controller, which stops all DMA. Returns whether the
    /// controller confirmed the reset by clearing `CTRL_RST`; on `false` the
    /// device may still own the rings and buffers (review of the v0.26.0
    /// stack, PR #10).
    fn reset(&self) -> bool {
        self.write_reg(REG_IMC, 0xFFFF_FFFF);
        self.write_reg(REG_RCTL, 0);
        self.write_reg(REG_TCTL, 0);
        self.write_reg(REG_CTRL, self.read_reg(REG_CTRL) | CTRL_RST);
        let done = poll_until_clear(|| self.read_reg(REG_CTRL), CTRL_RST, RESET_SPINS);
        self.write_reg(REG_IMC, 0xFFFF_FFFF);
        self.read_reg(REG_ICR);
        done
    }

    fn initialize(&mut self) -> Result<(), KernelError> {
        if !self.reset() {
            // Programming rings into a controller that never left reset
            // would hand it DMA addresses it may act on later.
            return Err(KernelError::Timeout {
                operation: "e1000 reset",
                duration_ms: 0,
            });
        }
        self.write_reg(REG_CTRL, self.read_reg(REG_CTRL) | CTRL_SLU);
        self.mac_address = self.read_mac_address();

        // RX ring: every descriptor owns a buffer.
        for i in 0..NUM_RX_DESC {
            let (_, phys) = Self::buffer(&self.rx_frames, i);
            // SAFETY: rx_desc(i) is within the ring frame owned by `self`.
            unsafe {
                core::ptr::write_volatile(
                    self.rx_desc(i),
                    RxDescriptor {
                        addr: phys,
                        length: 0,
                        checksum: 0,
                        status: 0,
                        errors: 0,
                        special: 0,
                    },
                );
            }
        }
        // TX ring: all descriptors start done (free).
        for i in 0..NUM_TX_DESC {
            let (_, phys) = Self::buffer(&self.tx_frames, i);
            // SAFETY: tx_desc(i) is within the ring frame owned by `self`.
            unsafe {
                core::ptr::write_volatile(
                    self.tx_desc(i),
                    TxDescriptor {
                        addr: phys,
                        length: 0,
                        cso: 0,
                        cmd: 0,
                        status: STATUS_DD,
                        css: 0,
                        special: 0,
                    },
                );
            }
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        let rx_base = self.rings.phys;
        self.write_reg(REG_RDBAL, rx_base as u32);
        self.write_reg(REG_RDBAH, (rx_base >> 32) as u32);
        self.write_reg(REG_RDLEN, (NUM_RX_DESC * 16) as u32);
        self.write_reg(REG_RDH, 0);
        // The NIC owns descriptors from head up to (not including) tail.
        self.write_reg(REG_RDT, (NUM_RX_DESC - 1) as u32);

        let tx_base = self.rings.phys + TX_RING_OFFSET as u64;
        self.write_reg(REG_TDBAL, tx_base as u32);
        self.write_reg(REG_TDBAH, (tx_base >> 32) as u32);
        self.write_reg(REG_TDLEN, (NUM_TX_DESC * 16) as u32);
        self.write_reg(REG_TDH, 0);
        self.write_reg(REG_TDT, 0);

        for i in 0..128 {
            self.write_reg(REG_MTA + i * 4, 0);
        }
        self.write_reg(REG_RCTL, RCTL_VALUE);
        self.write_reg(REG_TCTL, TCTL_VALUE);

        let m = self.mac_address.0;
        println!(
            "[E1000] Initialized with MAC: {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            m[0], m[1], m[2], m[3], m[4], m[5]
        );
        self.state = DeviceState::Up;
        Ok(())
    }

    fn transmit_raw(&mut self, packet: &[u8]) -> Result<(), KernelError> {
        if packet.is_empty() || packet.len() > BUF_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "packet_size",
                value: "empty or too large",
            });
        }
        let idx = self.tx_current;
        let desc = self.tx_desc(idx);
        // SAFETY: desc is in our ring; the NIC sets DD when it is done.
        let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*desc).status)) };
        if status & STATUS_DD == 0 {
            self.stats.tx_dropped += 1;
            return Err(KernelError::WouldBlock);
        }

        let (virt, phys) = Self::buffer(&self.tx_frames, idx);
        // SAFETY: the buffer is BUF_SIZE bytes of our DMA memory and the NIC
        // is done with it (DD set).
        unsafe {
            core::ptr::copy_nonoverlapping(packet.as_ptr(), virt as *mut u8, packet.len());
            core::ptr::write_volatile(
                desc,
                TxDescriptor {
                    addr: phys,
                    length: packet.len() as u16,
                    cso: 0,
                    cmd: TX_CMD,
                    status: 0,
                    css: 0,
                    special: 0,
                },
            );
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        self.tx_current = (idx + 1) % NUM_TX_DESC;
        self.write_reg(REG_TDT, self.tx_current as u32);
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += packet.len() as u64;
        Ok(())
    }

    fn receive_raw(&mut self) -> Result<Option<Packet>, KernelError> {
        let idx = self.rx_current;
        let desc = self.rx_desc(idx);
        // SAFETY: desc is in our ring; the NIC writes it (volatile read).
        let d = unsafe { core::ptr::read_volatile(desc) };
        if d.status & STATUS_DD == 0 {
            return Ok(None);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        let packet = if d.errors != 0 {
            self.stats.rx_errors += 1;
            None
        } else {
            // Device-reported length is untrusted: clamp to the buffer.
            let len = (d.length as usize).min(BUF_SIZE);
            let (virt, _) = Self::buffer(&self.rx_frames, idx);
            // SAFETY: virt points to BUF_SIZE bytes of our DMA memory that the
            // NIC has finished writing (DD set).
            let data = unsafe { core::slice::from_raw_parts(virt as *const u8, len) };
            self.stats.rx_packets += 1;
            self.stats.rx_bytes += len as u64;
            Some(Packet::from_bytes(data))
        };

        // Give the descriptor back: clear its status and make it the new
        // tail (the previous code advanced the tail past a descriptor it had
        // not processed yet).
        // SAFETY: desc is in our ring and we own it until RDT passes it back.
        unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!((*desc).status), 0) };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.write_reg(REG_RDT, idx as u32);
        self.rx_current = (idx + 1) % NUM_RX_DESC;
        Ok(packet)
    }

    /// Get MAC address
    pub fn mac_address(&self) -> MacAddress {
        self.mac_address
    }
}

impl Drop for E1000Driver {
    fn drop(&mut self) {
        // Stop all DMA before the rings and buffers are freed.
        if self.reset() {
            // SAFETY: the controller confirmed the reset, so it no longer
            // DMAs into the rings; `rings` is never used after this.
            unsafe { ManuallyDrop::drop(&mut self.rings) };
        } else {
            // The device did not leave reset and may still write the rings
            // and packet buffers: leak them rather than hand the frames back
            // to the allocator (dma_frame.rs contract). `rings` stays
            // `ManuallyDrop` and is never dropped.
            core::mem::forget(core::mem::take(&mut self.rx_frames));
            core::mem::forget(core::mem::take(&mut self.tx_frames));
            println!("[E1000] reset timed out; leaking its DMA frames");
        }
    }
}

impl NetworkDevice for E1000Driver {
    fn name(&self) -> &str {
        &self.name
    }

    fn mac_address(&self) -> MacAddress {
        self.mac_address
    }

    fn capabilities(&self) -> DeviceCapabilities {
        DeviceCapabilities {
            max_transmission_unit: 1500,
            supports_vlan: false,
            supports_checksum_offload: false,
            supports_tso: false,
            supports_lro: false,
        }
    }

    fn state(&self) -> DeviceState {
        self.state
    }

    fn set_state(&mut self, state: DeviceState) -> Result<(), KernelError> {
        match state {
            DeviceState::Up => {
                self.write_reg(REG_RCTL, RCTL_VALUE);
                self.write_reg(REG_TCTL, TCTL_VALUE);
            }
            DeviceState::Down => {
                self.write_reg(REG_RCTL, 0);
                self.write_reg(REG_TCTL, 0);
            }
            _ => {}
        }
        self.state = state;
        Ok(())
    }

    fn statistics(&self) -> DeviceStatistics {
        self.stats
    }

    fn transmit(&mut self, packet: &Packet) -> Result<(), KernelError> {
        if self.state != DeviceState::Up {
            self.stats.tx_dropped += 1;
            return Err(KernelError::InvalidState {
                expected: "up",
                actual: "not_up",
            });
        }
        self.transmit_raw(packet.data())
    }

    fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        if self.state != DeviceState::Up {
            return Ok(None);
        }
        self.receive_raw()
    }
}

/// Initialize E1000 driver
/// Upper bound on `CTRL_RST` polls during a reset.
const RESET_SPINS: usize = 100_000;

/// Poll `read` until every bit of `mask` reads clear, at most `spins` times.
/// Returns whether the bits cleared.
fn poll_until_clear(mut read: impl FnMut() -> u32, mask: u32, spins: usize) -> bool {
    for _ in 0..spins {
        if read() & mask == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

pub fn init() -> Result<(), KernelError> {
    println!("[E1000] Intel E1000 network driver module loaded");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_layout_matches_hardware() {
        assert_eq!(core::mem::size_of::<RxDescriptor>(), 16);
        assert_eq!(core::mem::size_of::<TxDescriptor>(), 16);
        // Ring lengths must be multiples of 128 bytes and fit in one frame.
        assert_eq!((NUM_RX_DESC * 16) % 128, 0);
        assert_eq!((NUM_TX_DESC * 16) % 128, 0);
        assert!(TX_RING_OFFSET + NUM_TX_DESC * 16 <= crate::mm::FRAME_SIZE);
        assert_eq!(NUM_RX_DESC % 2, 0);
        assert_eq!(NUM_TX_DESC % 2, 0);
    }

    #[test]
    fn reset_poll_reports_timeout_when_bit_never_clears() {
        assert!(!poll_until_clear(|| CTRL_RST, CTRL_RST, 1000));
    }

    #[test]
    fn reset_poll_reports_success_when_bit_clears() {
        let mut reads = 0;
        let read = || {
            reads += 1;
            if reads < 5 {
                CTRL_RST
            } else {
                0
            }
        };
        assert!(poll_until_clear(read, CTRL_RST, 1000));
    }
}
