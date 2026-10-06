//! VirtIO Network Driver
//!
//! Paravirtualized NIC (virtio device ID 1) over the shared virtio transport
//! (legacy PCI I/O ports or virtio-mmio) and the frame-backed [`VirtQueue`].
//!
//! DMA rules (DRV-SEC-01, DRV-SEC-02): the device is only ever given
//! physical addresses of frames from the frame allocator -- ring memory comes
//! from [`VirtQueue`], packet buffers are one frame each -- and a TX
//! descriptor and its buffer are reused only after the device has returned
//! it through the used ring. The previous driver handed the device heap
//! virtual addresses as DMA addresses and freed TX descriptors before the
//! device had consumed them, and could not drive the legacy (I/O-port) PCI
//! device QEMU exposes on x86_64 at all.

use alloc::vec::Vec;

use crate::{
    drivers::{
        dma_frame::DmaFrame,
        virtio::{
            mmio::VirtioMmioTransport, queue::VirtQueue, VirtioPciTransport, VirtioTransport,
        },
    },
    error::KernelError,
    mm::FRAME_SIZE,
    net::{
        device::{DeviceCapabilities, DeviceState, DeviceStatistics, NetworkDevice},
        MacAddress, Packet,
    },
};

/// Device has a MAC address in config space.
const VIRTIO_NET_F_MAC: u32 = 1 << 5;
/// Device supports checksum offload (not negotiated).
const VIRTIO_NET_F_CSUM: u32 = 1 << 0;

/// virtio device ID of a network card.
const VIRTIO_DEVICE_ID_NET: u32 = 1;

/// Queue indices.
const RX_QUEUE: u16 = 0;
const TX_QUEUE: u16 = 1;

/// Receive buffers kept posted, and transmit buffers in flight at most.
const RX_BUFFERS: usize = 32;
const TX_BUFFERS: usize = 32;

/// Descriptor flag: buffer is device-writable.
const VIRTQ_DESC_F_WRITE: u16 = 2;

/// Largest Ethernet frame we send or accept (without FCS).
const MAX_FRAME: usize = 1514;

/// VirtIO network driver.
pub struct VirtioNetDriver {
    transport: VirtioTransport,
    rx: VirtQueue,
    tx: VirtQueue,
    /// RX buffers, with the descriptor each one is posted on.
    rx_bufs: Vec<(u16, DmaFrame)>,
    /// TX buffers; `tx_busy[i]` is the descriptor using buffer `i`.
    tx_bufs: Vec<DmaFrame>,
    tx_busy: Vec<Option<u16>>,
    /// virtio-net header length: 12 with VIRTIO_F_VERSION_1, else 10.
    hdr_len: usize,
    mac_address: MacAddress,
    /// Interface name ("eth1", "eth2", ... -- unique per NIC).
    name: alloc::string::String,
    features: u32,
    state: DeviceState,
    stats: DeviceStatistics,
}

// SAFETY: the driver is only used behind the network device registry's
// lock; the raw ring pointers inside VirtQueue are owned by the driver.
unsafe impl Send for VirtioNetDriver {}

impl VirtioNetDriver {
    /// Probe a virtio-mmio slot; fails unless it holds a network device.
    pub fn new(mmio_base: usize) -> Result<Self, KernelError> {
        let mmio = VirtioMmioTransport::new(mmio_base);
        if !mmio.matches_device(VIRTIO_DEVICE_ID_NET) {
            return Err(KernelError::NotFound {
                resource: "virtio-net (mmio)",
                id: mmio_base as u64,
            });
        }
        Self::init(VirtioTransport::Mmio(mmio))
    }

    /// Initialize a legacy virtio-net PCI device at I/O port base `io_base`.
    pub fn new_pci(io_base: u16) -> Result<Self, KernelError> {
        Self::init(VirtioTransport::Pci(VirtioPciTransport::new(io_base)))
    }

    fn init(transport: VirtioTransport) -> Result<Self, KernelError> {
        let fail = |code| KernelError::HardwareError {
            device: "virtio-net",
            code,
        };

        // virtio spec 3.1.1: reset, ACKNOWLEDGE, DRIVER, features.
        transport.begin_init();
        let features = transport.read_device_features() & VIRTIO_NET_F_MAC;
        transport.write_guest_features(features);
        let version_1 = transport.negotiate_version_1();
        if !transport.set_features_ok() && version_1 {
            transport.set_failed();
            return Err(fail(1));
        }

        // On failure, reset the device before the queues created so far
        // are dropped: it must not keep using ring memory being freed.
        let rx =
            Self::setup_queue(&transport, RX_QUEUE).inspect_err(|_| transport.reset_device())?;
        let tx =
            Self::setup_queue(&transport, TX_QUEUE).inspect_err(|_| transport.reset_device())?;

        let mut mac = [0u8; 6];
        if features & VIRTIO_NET_F_MAC != 0 {
            for (i, byte) in mac.iter_mut().enumerate() {
                *byte = transport.read_device_config_u8(i as u16);
            }
        }

        let mut driver = Self {
            transport,
            rx,
            tx,
            rx_bufs: Vec::with_capacity(RX_BUFFERS),
            tx_bufs: Vec::with_capacity(TX_BUFFERS),
            tx_busy: Vec::with_capacity(TX_BUFFERS),
            hdr_len: if version_1 { 12 } else { 10 },
            mac_address: MacAddress(mac),
            name: crate::net::device::alloc_ethernet_name(),
            features,
            state: DeviceState::Down,
            stats: DeviceStatistics::default(),
        };

        // Post receive buffers before DRIVER_OK so the device can deliver
        // as soon as it is live.
        for _ in 0..RX_BUFFERS.min(driver.rx.size() as usize) {
            let buf = DmaFrame::alloc()?;
            let desc = driver.rx.alloc_desc().ok_or(fail(2))?;
            driver.post_rx(desc, &buf);
            driver.rx_bufs.push((desc, buf));
        }
        for _ in 0..TX_BUFFERS.min(driver.tx.size() as usize) {
            driver.tx_bufs.push(DmaFrame::alloc()?);
            driver.tx_busy.push(None);
        }

        driver.transport.set_driver_ok();
        driver.transport.notify_queue(RX_QUEUE);
        driver.state = DeviceState::Up;

        crate::println!(
            "[VIRTIO-NET] Initialized: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, {} RX / {} \
             TX buffers, {}-byte header",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5],
            driver.rx_bufs.len(),
            driver.tx_bufs.len(),
            driver.hdr_len
        );
        Ok(driver)
    }

    fn setup_queue(transport: &VirtioTransport, index: u16) -> Result<VirtQueue, KernelError> {
        transport.select_queue(index);
        let size = transport.read_queue_size();
        if size == 0 {
            return Err(KernelError::HardwareError {
                device: "virtio-net",
                code: 3,
            });
        }
        let queue = VirtQueue::new(size)?;
        transport.set_queue_size(queue.size());
        transport.write_queue_address(queue.pfn());
        transport.write_queue_phys(queue.phys_desc(), queue.phys_avail(), queue.phys_used());
        transport.set_queue_ready();
        Ok(queue)
    }

    /// (Re)post an RX buffer on descriptor `desc`.
    fn post_rx(&mut self, desc: u16, buf: &DmaFrame) {
        // SAFETY: `desc` is a descriptor this driver allocated; `buf` is a
        // whole frame of DMA memory that stays allocated while posted.
        unsafe {
            self.rx
                .write_desc(desc, buf.phys, FRAME_SIZE as u32, VIRTQ_DESC_F_WRITE, 0);
        }
        self.rx.push_avail(desc);
    }

    /// Return completed TX descriptors and their buffers to the free pool.
    fn reclaim_tx(&mut self) {
        while self.tx.has_used() {
            // Only a descriptor we have in flight is freed: a bogus or
            // repeated id from the device must not corrupt the free list.
            let Some((desc, _)) = self.tx.poll_used() else {
                self.stats.tx_errors += 1;
                continue;
            };
            if let Some(slot) = self.tx_busy.iter().position(|d| *d == Some(desc)) {
                self.tx_busy[slot] = None;
                self.tx.free_desc(desc);
            } else {
                self.stats.tx_errors += 1;
            }
        }
    }

    /// Transmit one Ethernet frame.
    pub fn transmit(&mut self, frame: &[u8]) -> Result<(), KernelError> {
        if self.state != DeviceState::Up {
            return Err(KernelError::InvalidState {
                expected: "up",
                actual: "down",
            });
        }
        if frame.is_empty() || frame.len() > MAX_FRAME {
            return Err(KernelError::InvalidArgument {
                name: "frame length",
                value: "empty or larger than MTU",
            });
        }

        self.reclaim_tx();
        let Some(slot) = self.tx_busy.iter().position(Option::is_none) else {
            self.stats.tx_dropped += 1;
            return Err(KernelError::WouldBlock);
        };
        let Some(desc) = self.tx.alloc_desc() else {
            self.stats.tx_dropped += 1;
            return Err(KernelError::WouldBlock);
        };

        let hdr_len = self.hdr_len;
        let total = hdr_len + frame.len();
        let buf = &mut self.tx_bufs[slot];
        let bytes = buf.bytes_mut();
        bytes[..hdr_len].fill(0); // no offloads requested
        bytes[hdr_len..total].copy_from_slice(frame);
        let phys = buf.phys;

        // SAFETY: `desc` was just allocated; the buffer stays owned by this
        // descriptor (tx_busy) until the device returns it via the used ring.
        unsafe { self.tx.write_desc(desc, phys, total as u32, 0, 0) };
        self.tx_busy[slot] = Some(desc);
        self.tx.push_avail(desc);
        self.transport.notify_queue(TX_QUEUE);

        self.stats.tx_packets += 1;
        self.stats.tx_bytes += frame.len() as u64;
        Ok(())
    }

    /// Receive one frame if the device has completed an RX buffer.
    pub fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        if self.state != DeviceState::Up {
            return Ok(None);
        }
        let Some((desc, len)) = self.rx.poll_used() else {
            return Ok(None);
        };
        let Some(index) = self.rx_bufs.iter().position(|(d, _)| *d == desc) else {
            // Not one of ours: drop it rather than trust the device's id.
            self.stats.rx_errors += 1;
            return Ok(None);
        };

        // The device-reported length is untrusted: clamp to the buffer.
        let len = (len as usize).min(FRAME_SIZE);
        let packet = if len > self.hdr_len {
            let frame = &self.rx_bufs[index].1.bytes()[self.hdr_len..len];
            self.stats.rx_packets += 1;
            self.stats.rx_bytes += frame.len() as u64;
            Some(Packet::from_bytes(frame))
        } else {
            self.stats.rx_errors += 1;
            None
        };

        // Repost the same buffer on the same descriptor.
        let (_, buf) = self.rx_bufs.swap_remove(index);
        self.post_rx(desc, &buf);
        self.rx_bufs.push((desc, buf));
        self.transport.notify_queue(RX_QUEUE);

        Ok(packet)
    }

    /// Get MAC address
    pub fn mac_address(&self) -> MacAddress {
        self.mac_address
    }
}

impl NetworkDevice for VirtioNetDriver {
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
            supports_checksum_offload: self.features & VIRTIO_NET_F_CSUM != 0,
            supports_tso: false,
            supports_lro: false,
        }
    }

    fn state(&self) -> DeviceState {
        self.state
    }

    fn set_state(&mut self, state: DeviceState) -> Result<(), KernelError> {
        // Taking the device down only stops the driver using it; resetting
        // the device would discard the posted buffers and queues.
        self.state = state;
        Ok(())
    }

    fn statistics(&self) -> DeviceStatistics {
        self.stats
    }

    fn transmit(&mut self, packet: &Packet) -> Result<(), KernelError> {
        VirtioNetDriver::transmit(self, packet.data())
    }

    fn receive(&mut self) -> Result<Option<Packet>, KernelError> {
        VirtioNetDriver::receive(self)
    }
}

impl Drop for VirtioNetDriver {
    fn drop(&mut self) {
        // Stop the device before the fields drop: the posted RX buffers,
        // in-flight TX buffers and the rings are about to be returned to
        // the frame allocator, and a live device would keep writing them.
        self.transport.reset_device();
    }
}

/// Initialize VirtIO-Net driver module
pub fn init() -> Result<(), KernelError> {
    crate::println!("[VIRTIO-NET] VirtIO Network driver module loaded");
    Ok(())
}
