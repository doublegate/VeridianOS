# ADR 0019: Streaming DMA cache maintenance, coherency per device

- Status: accepted
- Date: 2026-10-18
- Tracking: HA-25 (`ref-docs/HARDWARE-AARCH64.md`), the RISC-V `dma-noncoherent` boards in
  `ref-docs/HARDWARE-RISCV64.md`; v0.27.0 sprint E

## Context

A device that is not coherent with the CPU caches reads and writes memory, not the caches. The
CPU's writes to a buffer must reach memory before the device reads it, and the CPU must not read
stale cached lines after the device wrote it. A dirty line the CPU still holds must also not be
written back later over what the device wrote.

QEMU keeps DMA coherent, and its device trees say so: every AArch64 `virtio,mmio` node and the
RISC-V `soc` bus carry `dma-coherent`. Real boards differ:

- AArch64: the Raspberry Pi 4 and 5, RK3588 and Allwinner trees mark no device `dma-coherent`;
  servers normally are coherent.
- RISC-V: most systems are coherent; T-Head C9xx (D1, TH1520, SG2042) and SpacemiT K1 trees mark
  their devices `dma-noncoherent`.

Before this change the kernel did no maintenance for DMA buffers on either architecture
(`docs/KNOWN-LIMITATIONS.md`). AArch64 already maintained the caches for its boot page tables and
for code it writes (HA-25 part).

## Decision

1. **Linux's streaming-DMA model (`mm::dma`).** A transfer has a direction (to the device, from
   it, both). Before the device uses a buffer, `sync_for_device` cleans its lines to the point of
   coherency in every direction, as Linux's arm64 and RISC-V `arch_sync_dma_for_device` do, so no
   dirty line is evicted over the device's data later. Once the device is done, `sync_for_cpu`
   cleans and invalidates them unless the device only read the buffer (`arch_sync_dma_for_cpu`).
   Clean-and-invalidate rather than invalidate, so that a line the buffer shares with other data
   loses nothing. Each call orders against the device with `dma_wmb`/`dma_rmb`.
2. **Coherency per device, from the device tree, as Linux's `of_dma_is_coherent`.** A node's own
   `dma-coherent` or `dma-noncoherent`, else its nearest ancestor's. Without either the
   architecture's default applies, Linux's: AArch64 non-coherent, RISC-V and x86_64 coherent.
   PCI devices (x86_64 only today) are coherent.
3. **Instructions.** AArch64: `dc cvac` and `dc civac` by line (CTR_EL0.DminLine), then `dsb sy`.
   RISC-V: Zicbom's `cbo.clean` and `cbo.flush` by block (`riscv,cbom-block-size`), found in the
   cpu node's ISA strings; firmware enables them for S-mode. x86_64 needs none.
4. **Users.** The virtqueue (descriptors and available ring written back before the index that
   publishes them, then the index; used-ring lines dropped before each read; the zeroed rings
   written back once when the queue is marked non-coherent), virtio-blk's request frames,
   virtio-net's buffers and the fw_cfg DMA descriptor used for ramfb.

## Consequences

- The same drivers work on non-coherent boards, once their other blockers are gone (device
  discovery, PCIe, interrupts; HA-05, HA-20, HR-04..06, HR-34).
- QEMU models no caches, so stale data cannot be observed under it. The boot test
  `dma_cache_maintenance` runs the non-coherent path on a frame (the instructions execute, the
  data survives), and a one-time run with every device forced non-coherent passed the AArch64 and
  RISC-V runtime suites. Correctness against real caches needs hardware.
- A RISC-V device marked `dma-noncoherent` on a hart without Zicbom gets no maintenance: the T-Head
  vendor instructions (XTheadCmo) and Svpbmt non-cacheable mappings (HR-11) are not implemented.
- Descriptor rings could instead live in non-cacheable memory (Normal-NC on AArch64, Svpbmt NC on
  RISC-V), saving the maintenance on every exchange. Not done: it needs per-mapping memory types
  in the physical map.
- Drivers for PCIe devices on AArch64 and RISC-V (NVMe, e1000, AHCI, xHCI) must use the same calls
  when PCIe arrives there; today they run on x86_64 only.
