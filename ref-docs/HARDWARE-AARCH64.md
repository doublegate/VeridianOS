# VeridianOS on real AArch64 hardware

Status: reference, 2026-10-07 (code references re-verified against `feat/v0.27` at c38a422).
Scope: what the AArch64 kernel assumes about QEMU `virt` (cortex-a72, direct `-kernel` ELF boot,
linked at 0x40200000, PL011 at 0x0900_0000, GICv2, MMU off) and what it must do to boot on
Raspberry Pi 4/5, Rockchip/Allwinner boards and Arm SystemReady servers. Findings carry stable IDs
(HA-nn) used by `to-dos/MASTER_TODO.md`; related audit IDs (N-nn) are in `docs/audit/`. Paths are
under `kernel/src/` unless noted.

## 1. Bottom line

The kernel boots only on QEMU `virt`. Five things each stop it on every real board:

1. The device-tree pointer passed in x0 is overwritten by the first instruction.
2. There is no arm64 Image header, so U-Boot `booti`, the Pi firmware and EFI cannot load it.
3. It is linked to run only at physical 0x40200000.
4. UART, GIC and RAM addresses are hard-coded for QEMU.
5. It runs with the MMU off: all data accesses are Device memory, so load/store-exclusive (every
   spin lock) is unreliable on real silicon, and any unaligned access faults (`+strict-align` in
   the target spec covers compiler-generated code only).

A sixth becomes blocking the moment item 5 is fixed: the kernel contains no cache-maintenance
instructions at all (HA-25), and none of the target SoCs has coherent DMA.

## 2. Target hardware classes

| Class | Boot | Interrupt controller | Console | CPUs / notes |
|---|---|---|---|---|
| Raspberry Pi 4 (BCM2711) | VideoCore firmware + armstub (+ optional U-Boot or EDK2), EL2, x0 = DTB | GIC-400 (GICv2) GICD 0xFF84_1000, GICC 0xFF84_2000 (low-peripheral map) | PL011 0xFE20_1000 (drives Bluetooth unless `dtoverlay=disable-bt`); default console is the AUX mini-UART (16550-like) at 0xFE21_5040 | 4x Cortex-A72 (ARMv8.0); RAM at 0; spin-table, release addresses 0xd8/0xe0/0xe8/0xf0 |
| Raspberry Pi 5 (BCM2712) | EEPROM bootloader + TF-A BL31 (reserves PA 0-0x80000), EL2 | GIC-400 (GICv2) GICD 0x10_7FFF_9000, GICC 0x10_7FFF_A000 | debug-connector PL011 `uart10` at 0x10_7D00_1000; 40-pin UARTs are on RP1 behind PCIe (0x1F_0003_0000 for UART0 with `enable_rp1_uart=1`) | 4x Cortex-A76 (ARMv8.2); PSCI 1.0 over SMC |
| Rockchip RK3588 | BootROM, TF-A + U-Boot (or EDK2 port) | GICv3 (GIC-600 (verify)) GICD 0xFE60_0000, GICR 0xFE68_0000, two ITS (`dma-noncoherent`) | DW APB 8250 (`snps,dw-apb-uart`, `reg-shift = 2`, `reg-io-width = 4`), `uart2` 0xFEB5_0000 on most boards | 4x A55 (MPIDR 0x000-0x300) + 4x A76 (0x400-0x700); PSCI 1.0 SMC; 4-cell GIC interrupt specifiers |
| Allwinner H6/A64 class | BootROM, SPL + TF-A + U-Boot | GIC-400 (GICv2), H6 at 0x0302_1000 | DW APB 8250 (same properties), H6 `uart0` 0x0500_0000 | 4x Cortex-A53; PSCI SMC |
| SystemReady SR/ES servers (Ampere Altra/AmpereOne, etc.) | UEFI + ACPI (BBR) | GICv3 + ITS, many CPUs | SPCR-described UART (often PL011/SBSA UART) | ACPI tables instead of DT; > 8 CPUs needs GICv3; Altra MPIDR Aff3 = socket id |

Correction to common belief: the Pi 5 uses a GIC-400 (GICv2), not GICv3 (bcm2712.dtsi `arm,gic-400`).

## 3. Findings

Severity: **B** blocks boot; **D** breaks a device; **L** latent.

| ID | Location | QEMU assumption | Real firmware/hardware | Sev | Fix |
|---|---|---|---|---|---|
| HA-01 | `arch/aarch64/boot.S:6` | x0 discarded (`mrs x0, CurrentEL` first) | booting.rst: x0 = DTB physical address (8-byte aligned, <= 2 MiB, not inside a 2 MiB block needing special attributes), x1-x3 = 0 | B | Save x0 in a callee-saved register; pass it to Rust; check x1-x3 |
| HA-02 | `arch/aarch64/smp.rs:25,38` | DTB at 0x4000_0000 (QEMU bare-ELF placement) | Firmware, U-Boot, EFI place it anywhere; Pi `device_tree_address` moves it | B | Global `boot_fdt()` from the saved x0 |
| HA-03 | `boot.S:1-4`, `arch/aarch64/link.ld` | ELF only | `booti`/Pi firmware need the 64-byte Image header ("ARM\x64" at 0x38, `text_offset`, `image_size`, flags: bit 0 endianness, bits 1-2 page size, bit 3 placement); UEFI needs PE/COFF (`res5` = PE offset) | B | Image header in `.text.boot` with `image_size` covering BSS and stack; `objcopy -O binary` artifact; optional EFI stub |
| HA-04 | `link.ld:9`, `.cargo/config.toml:12` (`relocation-model=static`), `boot.S:87-88` (`ldr =` literals) | Loaded at PA 0x40200000 | Any 2 MiB-aligned RAM address; Pi and RK3588 RAM start at 0; Pi firmware default 64-bit load address 0x200000 (`kernel_address`) | B | Position-independent early assembly (`adr`/`adrp`), then link the kernel at a high TTBR1 VA and map it in boot.S (Linux model); interim: Pi 4 (>= 2 GB) `kernel_address=0x40200000` |
| HA-05 | `boot.S:50-61`, `direct_uart.rs:11,25`, `serial.rs:49`, `mod.rs:129`, `entry.rs:15,30`, `bootstrap.rs:19`, `boot.rs:52` (all `arch/aarch64/`) | PL011 at 0x0900_0000, no setup, no TXFF polling | Pi: 0x0900_0000 is RAM (writes corrupt memory); elsewhere an external abort before vectors exist (silent hang) | B | Earlycon from `/chosen/stdout-path` (ACPI: SPCR); PL011 and ns16550/DW drivers (`reg-shift`, `reg-io-width`), FIFO polling |
| HA-06 | `drivers/input.rs:127`, `syscall/filesystem.rs:49,115`, `simple_alloc_unsafe.rs:56,91`, `test_tasks.rs` | Console I/O through the hard-coded UART | as HA-05 | D | One console driver selected at boot |
| HA-07 | `arch/aarch64/gic.rs:39,42,428,453-461` | GICD 0x0800_0000, GICC 0x0801_0000, GICv2 only | Pi 4/5, H6 addresses differ; GICv3 has no GICC MMIO (ICC_* system registers, per-CPU GICR) | B | Bases from DT (`arm,gic-400`, `arm,cortex-a15-gic`, `arm,gic-v3`) or MADT (GICC/GICD/GICR/ITS); GICv3 driver |
| HA-08 | `gic.rs:228,292` | SPIs Group 0, GICC_CTLR = 1 | Non-secure: IGROUPR RAZ/WI and CTLR bit 0 enables Group 1; works by accident (Pi armstub already puts everything in Group 1) | L | Configure Group 1 explicitly |
| HA-09 | `gic.rs:253-256` | SPIs target CPU interface 0 (`0x0101_0101`) | Boot CPU's interface number is not necessarily 0 | L | Read the banked GICD_ITARGETSR0 byte (Linux `gic_get_cpumask`) |
| HA-10 | `gic.rs:259-263` | All SPIs level-triggered | Edge-triggered devices exist (Pi 5 RP1 MSIs are edge SPIs 128-191) | D | Trigger type from DT `interrupts` flags |
| HA-11 | `sched/smp.rs:443-453` | SGI to GICD 0x0800_0000 with `1u32 << target_cpu` | GICv2 target = interface bitmap (<= 8 CPUs; the shift is also UB-panics for >= 32); GICv3 uses ICC_SGI1R_EL1 with affinity | D | GIC abstraction keyed by hardware id |
| HA-12 | `arch/timer.rs:119`, `aarch64/timer.rs:46-47` | CNTFRQ_EL0 programmed | booting.rst requires firmware to set it, but bare-metal stubs may not (verify which boards); at 0 the period clamps to 1 tick (interrupt storm) | L | DT `/timer` `clock-frequency` fallback; refuse to tick at 0 |
| HA-13 | `aarch64/exceptions.rs:153` | Virtual timer INTID 27 | True on Pi 4/5, RK3588 (PPI 11) and BSA-recommended; still a platform fact | L | From DT `/timer` (ACPI GTDT) |
| HA-14 | `boot.S:14-39` and secondary path `boot.S:111-127` (N-177) | EL2 sets only HCR, CNTHCTL, CNTVOFF, SCTLR_EL1 = 0 | booting.rst obliges firmware to initialise every writable register at or below the entry EL (incl. CPTR_EL2.TFP = 0), so conforming loaders leave defined values, but not necessarily the ones the kernel needs (MDCR_EL2/HSTR_EL2 traps, VPIDR/VMPIDR, ICC_SRE_EL2 for GICv3, RES1 bits of SCTLR_EL1); the kernel is built `+neon`, so an FP trap to EL2 hangs | L (B with a non-conforming bare-metal stub) | Port Linux `el2_setup.h` and `INIT_SCTLR_EL1_MMU_OFF/ON` for both entry paths |
| HA-15 | `boot.S:12,41`, `boot.S:127` | EL3 entry halts; EL1 paths (boot and secondary) never write SCTLR_EL1 | Most firmware enters at EL2; some SBC setups at EL3 | L | Write SCTLR_EL1 on every path; optional EL3 -> EL2 drop |
| HA-16 | whole kernel (N-28) | MMU and caches off | LDXR/STXR on Device memory not guaranteed (no global monitor): first spin lock may hang; unaligned Device access faults; uncached execution very slow | B | Sprint E: identity + high mappings, MAIR/TCR/TTBR, caches on before the first lock |
| HA-17 | `mm/mod.rs:489-498` | RAM = 0x4000_0000 + 128 MiB | Pi RAM at 0 with VideoCore carve-out, TF-A (Pi 5: 0-0x80000 `no-map`) and spin-table page; servers have UEFI maps above 4 GiB | B | `/memory` (`#address-cells`/`#size-cells`), `/memreserve/`, `reserved-memory` (`no-map`), DTB and `linux,initrd-*`; EFI memory map on UEFI |
| HA-18 | `drivers/virtio/mmio.rs:42` | 32 virtio-mmio slots at 0x0A00_0000 | No such devices: RAM on Pi, abort elsewhere | D/B | Probe only DT `virtio,mmio` nodes |
| HA-19 | `drivers/ramfb.rs:22`, `bootstrap.rs:389` | fw_cfg at 0x0902_0000 | Pi: writes and a DMA descriptor land in RAM; elsewhere abort | B/D | Gate on DT `qemu,fw-cfg-mmio` + signature; `simple-framebuffer` (Pi) or EFI GOP |
| HA-20 | `bootstrap.rs:526-532`, `drivers/pci.rs:993` | PCI x86-only | Pi 4 USB (VL805) and Pi 5 RP1 behind Broadcom PCIe (`brcm,bcm2711-pcie`/`brcm,bcm2712-pcie`, not plain ECAM, own MSI controller); servers MCFG; RK3588 DesignWare | D | Generic ECAM from DT `pci-host-ecam-generic`/MCFG; SoC root complexes later |
| HA-21 | `arch/aarch64/smp.rs:31,53,60`, `boot.rs:23,64` | MPIDR masked to Aff2..Aff0 and stored as `u32` | Aff3 is nonzero on 2-socket Ampere Altra (socket id, edk2 `AC01_GET_MPIDR`): socket-1 CPUs alias socket 0; CPU_ON needs the full MPIDR | L (D on 2P Altra) | Full 40-bit affinity (`u64`) as hardware id |
| HA-22 | `smp.rs:56`, `psci.rs` | PSCI CPU_ON only, conduit from DT | Pi 4 spin-table (no PSCI at all); Pi 5/RK3588/H6 PSCI over SMC; ACPI FADT ARM_BOOT_ARCH; PSCI 0.1 function ids from DT | L | PSCI_VERSION/FEATURES, CPU_OFF, SYSTEM_OFF/RESET, FADT conduit, spin-table (write release address, clean to PoC, `sev`) |
| HA-23 | `arch/aarch64/mod.rs:133` | `HEAP_START = 0x4100_0000` | unused, misleading | L | Delete |
| HA-24 | servers | Device tree only | SystemReady: UEFI + ACPI (MADT, GTDT, SPCR, MCFG, PPTT, IORT); > 8 CPUs needs GICv3 | B (servers) | EFI entry (PE/COFF stub or a boot protocol supplying memory map, DTB/RSDP); ACPI arm64 parsers |
| HA-25 | whole tree (no `dc`/`ic` instruction anywhere under `kernel/src`) | Caches off, QEMU coherent | MMU-on transition, ELF loading (I/D coherency), spin-table release and every DMA buffer need maintenance; no `dma-coherent` node exists in bcm2711/bcm2712/rk3588 DTs | B (once caches are on) | `arch/aarch64/cache.rs`: `dc cvac/civac/ivac/cvau`, `ic ivau/ialluis` by VA using CTR_EL0 line sizes and IDC/DIC; DMA sync API (§4) |
| HA-26 | `arch/aarch64/context.rs:236,427` | TTBR0_EL1 switched without ASID or TLBI | With the MMU on and no ASID tagging, the previous process's TLB entries remain valid after a switch | L | ASID in TTBR0[63:48] with generation rollover, or `tlbi aside1is`/`vmalle1is` on switch |
| HA-27 | `arch/timer.rs:123-130` | `set_timer` programs the EL1 physical timer (CNTP, INTID 30) | Tick uses the virtual timer (INTID 27); CNTP needs CNTHCTL_EL2.EL1PCEN, which a loader entering at EL1 may not set | L | Delete or switch to CNTV_* (dead on AArch64 today) |
| HA-28 | `arch/fdt.rs` (no interrupt parser yet) | 3-cell GIC specifiers | RK3588 GICv3 uses `#interrupt-cells = <4>` (PPI partition cell); `interrupt-map`, `interrupts-extended` and `interrupt-parent` chains on PCIe/RP1 | D | Honour `#interrupt-cells` per controller; parse `interrupt-map` |
| HA-29 | `drivers/virtio/queue.rs` and any DMA user | Bus address = PA | Pi 4 legacy DMA/EMMC see RAM through `dma-ranges` (bus 0xC000_0000 = PA 0, 1 GiB only); Pi 5 PCIe sees RAM at bus 0x10_0000_0000 | D | DMA address translation from `dma-ranges` (ACPI `_DMA`/IORT); bounce below the limit |
| HA-30 | `arch/aarch64` entry/exception paths | No speculation mitigations | A72/A76/N1 need branch-predictor/BHB hardening on EL0 -> EL1 entry (SMCCC_ARCH_WORKAROUND_1/3 or BHB loop) and SSBD (WORKAROUND_2); none present | L (security) | SMCCC discovery + per-MIDR mitigation table (§4 errata) |

Related audit items: N-28 (MMU/EL0), N-41 (FP state), N-170 (PSTATE sanitiser), N-177 (EL2/SCTLR),
N-178 (FP/SIMD in kernel), N-26 (guard pages), N-78 (DMA barriers).

## 4. Target design

**Entry (boot.S).** Position-independent: save x0-x3, install an early VBAR that prints and spins,
complete EL2 setup (CPTR_EL2 RES1 with TFP=0, MDCR_EL2, HSTR_EL2=0, VPIDR/VMPIDR from MIDR/MPIDR,
ICC_SRE_EL2, CNTHCTL, HCR_EL2.RW) and drop to EL1 with INIT_SCTLR, zero BSS through `adr`
addresses, build the early page tables and enable the MMU and caches before any Rust code runs.
Same code for the secondary entry.

**Address space (sprint E, N-28).** Kernel linked at a fixed high TTBR1 VA and mapped there from
whatever PA it was loaded at (relocation for free); identity mapping only for the MMU-enable
trampoline; direct map built from the DT `/memory` ranges; TCR.IPS from ID_AA64MMFR0_EL1.PARange
(Cortex-A72: 44 bits, A76/A55: 40); 48-bit TTBR0 (T0SZ = 16) for the user layout (mmap base
0x4000_0000_0000). MMIO mapped Device-nGnRE per DT `reg` range. Page tables written before the MMU
is on must be cleaned (or invalidated before use) since the walker reads through the cache once
TCR IRGN/ORGN say so.

**Platform discovery.** One FDT parser (`arch/fdt.rs`) extended with `#address-cells`/
`#size-cells`, `compatible` search, `reg`/`interrupts` (variable `#interrupt-cells`), `ranges` and
`dma-ranges` translation, `/chosen`, `reserved-memory` and the memreserve block, fuzzed on the
host. Overlays (`dtoverlay=` in Pi `config.txt`, U-Boot `fdt apply`) are merged by the loader; the
kernel sees one blob whose size and placement vary, so never assume either. ACPI (MADT, GTDT,
SPCR, MCFG, PPTT, IORT) on SystemReady machines via the EFI entry.

**Interrupts.** GIC trait with v2 (bases from DT, own CPU mask, trigger types, Group 1) and v3
(ICC_SRE, GICR discovery by GICR_TYPER affinity and `Last` bit, 128 KiB frames for v3 / 256 KiB
for v4, affinity routing via GICD_IROUTER, SGI1R) back ends; IPIs keyed by hardware id. GICv5 is
already in booting.rst and QEMU `virt` (verify which release) but not in shipping silicon; keep the
trait open for it.

**MSI.** GICv3 ITS: LPIs (INTID >= 8192) need a property table (GICR_PROPBASER), a 64 KiB-aligned
pending table per CPU (GICR_PENDBASER), ITS device/collection tables (GITS_BASERn) and a command
queue (MAPD, MAPC, MAPTI, INV, SYNC). DeviceID comes from the PCI requester id through DT `msi-map`
or IORT ID mappings. RK3588's two ITSes are `dma-noncoherent`: read back the shareability fields
and clean the tables when they report non-shareable (Linux `ITS_FLAGS_FORCE_NON_SHAREABLE`).
GICv2 systems use GICv2m frames or SoC MSI controllers (Pi 4/5 PCIe: Broadcom MSI in the root
complex; Pi 5 RP1: MSI to GIC SPIs 128-191 via `msi-ranges`).

**CPUs.** Full MPIDR ids; PSCI (version, features, CPU_ON with a physical entry address, CPU_OFF,
SYSTEM_OFF/RESET) or spin-table; secondaries enable their own MMU from an identity-mapped entry;
`ApBootArgs` and boot page tables cleaned to PoC before release. DynamIQ cores (A55/A76) report
MPIDR.MT = 1 with the core number in Aff1 (RK3588 `reg = <0x100>` steps), so never index CPUs by
Aff0.

**Caches and DMA.** Rules (Arm ARM D7/booting.rst):

| Case | Operation |
|---|---|
| Memory written with the MMU/caches off, then read cached (page tables, BSS, boot args) | invalidate (`dc ivac`) before first cached read, or write it cached and clean |
| Code written by the kernel (ELF load, trampolines) | `dc cvau` to PoU, `dsb ish`, `ic ivau`, `dsb ish`, `isb`; skip the clean if CTR_EL0.IDC=1, the `ic` if DIC=1 (not trusted on Neoverse-N1 erratum 1542419) |
| Data for another CPU with its MMU off (spin-table release, ApBootArgs) | `dc civac` to PoC, then `sev` |
| Buffer to a non-coherent device | `dc cvac` to PoC over the buffer, `dsb sy` |
| Buffer from a non-coherent device | `dc ivac` (or `civac`) before handing it over and again before the CPU reads; buffers aligned to CTR_EL0.CWG so no partial line is shared |
| Whole-cache (set/way) | local CPU power-down/up only; not broadcast, unsafe with system caches; use VA ops otherwise |

Coherency is per device: `dma-coherent` (DT), `_CCA` (ACPI), IORT memory-access flags. Pi 4/5,
RK3588 and Allwinner declare none, so the VideoCore mailbox, EMMC, USB, Ethernet and RP1 all need
the table above or Normal-NC (MAIR) mappings for descriptor rings. Servers are normally coherent.
Use outer-shareable `dmb oshst/oshld` for device rings (N-78) and CNTKCTL_EL1.EL0VCTEN for user
counter reads.

**IOMMU.** SystemReady servers: SMMUv3 (Ampere Altra), described by IORT (SMMUv3 nodes, RC -> SMMU
-> ITS ID mappings, RMR nodes for firmware-owned DMA such as the GOP framebuffer) or DT
`arm,smmu-v3` + `iommu-map`. SMMUv2 (`arm,mmu-500`) appears on some SoCs (verify per board).
RK3588 uses Rockchip per-master IOMMUs (`rockchip,rk3588-iommu`); Pi 5 has Broadcom-specific IOMMUs
for display/media (verify binding); Pi 4 has none. For the user-space-driver goal (C6) the IOMMU is
the only DMA isolation boundary, so on Pi 4 DMA-capable drivers stay trusted. Firmware may leave an
SMMUv3 in abort (GBPA) or bypass; probe state before relying on either.

**Timers.** CNTPCT_EL0 is the physical count; CNTVCT_EL0 = CNTPCT - CNTVOFF_EL2. An EL1 kernel uses
the virtual timer (CNTV, INTID 27), as Linux does when not in VHE; boot.S zeroes CNTVOFF_EL2 on both
paths, which satisfies booting.rst's "consistent across CPUs". Under VHE (HCR_EL2.E2H=1) EL1 timer
accesses redirect to the EL2 timers (INTIDs 26/28); not planned. Counter frequency varies (Pi 4/5
54 MHz, RK3588 24 MHz, QEMU `virt` 62.5 MHz or 1 GHz on newer machine versions; all (verify)):
convert with 128-bit multiply, never hard-code. DT timer PPIs on Pi are `IRQ_TYPE_LEVEL_LOW`.

**Power management (PSCI, DEN 0022).** Function ids (SMC64 where applicable): PSCI_VERSION
0x8400_0000, CPU_SUSPEND 0xC400_0001, CPU_OFF 0x8400_0002, CPU_ON 0xC400_0003, SYSTEM_OFF
0x8400_0008, SYSTEM_RESET 0x8400_0009, PSCI_FEATURES 0x8400_000A, SYSTEM_SUSPEND 0xC400_000E
(PSCI 1.0, optional). CPU_SUSPEND power_state format (original vs extended) comes from
PSCI_FEATURES(CPU_SUSPEND); states from DT `idle-states` (`arm,psci-suspend-param`) or ACPI `_LPI`;
`wfi` is the fallback idle. Powerdown states lose caches and EL1 state like CPU_ON, so resume uses
the secondary entry path. Pi 4 has no PSCI: reboot via the PM watchdog (`brcm,bcm2835-pm`), no
power-off.

**CPU errata (Linux `arch/arm64/kernel/cpu_errata.c`).** Linux matches each CPU's MIDR_EL1
(implementer, part, variant/revision range, sometimes REVIDR) against a capability table, applies
workarounds by patching `alternative` sites at boot, and refuses a late CPU that needs a workaround
not already applied (relevant on big.LITTLE, where clusters differ). VeridianOS can start with a
per-CPU flag table read at runtime. OS-relevant entries (from `silicon-errata.rst`/Kconfig):

| Core (board) | Erratum / issue | Effect for this kernel |
|---|---|---|
| Cortex-A53 (Allwinner) | 843419 (ADRP at page offset 0xff8/0xffc), 835769 (multiply-accumulate) | link with lld `--fix-cortex-a53-843419`; compiler fix for 835769 (verify Rust flag) |
| Cortex-A53 | 826319/827319/824069/819472 | `dc cvac`/`cvau` must be upgraded to `dc civac` (Linux `ARM64_WORKAROUND_CLEAN_CACHE`) |
| Cortex-A72 (Pi 4) | 1319367 speculative AT; Spectre-v2/BHB | AT only matters with EL2 use; BP/BHB hardening on EL0 entry (Pi 4 armstub provides no SMCCC workaround call (verify)) |
| Cortex-A76 (Pi 5, RK3588) | 1165522 speculative AT, 1286807 TLBI (r0p0-r3p0), 1463225 single-step, 3324349 MSR SSBS, 4193800 TLBI completion | repeat `tlbi`+`dsb` (`REPEAT_TLBI_SYNC`) on affected revisions; `isb`/`sb` after SSBS writes; debugger step workaround |
| Cortex-A55 (RK3588) | 1024718 DBM, 1530923 speculative AT, 2441007 TLBI | do not enable hardware dirty-bit management on A55; repeat TLBI |
| Neoverse-N1 (Altra) | 1542419 instruction fetch, 3324349, 4193800 | treat DIC as 0 (always `ic ivau`); repeat TLBI |
| AmpereOne | AC03_CPU_38, AC04_CPU_10/23 | read `silicon-errata.rst` before targeting (verify scope) |

Board revisions (Pi 4 A72 r0p3, Pi 5 A76 r4p1) decide which ranges apply (verify by MIDR on the
board). Meltdown (KPTI) does not affect A53/A55/A72/A76/N1.

**CPU features.** Read ID_AA64ISAR0/1/2, PFR0/1, MMFR0/1/2 on every CPU and use the system-wide
intersection (Linux "sanitised" registers). Worth enabling, in order:

| Feature | Present on | Use |
|---|---|---|
| FEAT_PAN (v8.1) | A55, A76, N1 (not A72/A53) | block kernel access to user pages outside copy routines |
| FEAT_LSE (v8.1) | A55, A76, N1 (not A72/A53) | CAS/LDADD atomics; LL/SC scales badly on 80+ core servers. `+lse` at compile time breaks Pi 4, so build two kernels or dispatch at runtime (outlined atomics) |
| FEAT_HAFDBS (v8.1) | A55, A76, N1 | hardware Access flag; DBM except on A55 (erratum 1024718) |
| FEAT_PAuth (v8.3) | not on these SBC cores; Neoverse V1/N2, AmpereOne | `-Z branch-protection=pac-ret`, SCTLR_EL1.EnIA, per-task keys; hint encodings are NOPs on older cores |
| FEAT_BTI (v8.5) | newer Neoverse/Cortex | GP bit in stage-1 PTEs, SCTLR_EL1.BT0/BT1, `-Z branch-protection=bti` |
| FEAT_MTE2 (v8.5) | few servers/SoCs (verify AmpereOne) | tag-checked heap; needs HCR_EL2.ATA and booting.rst MTE setup; low priority |

**Heterogeneous CPUs (EEVDF placement, ADR 0007).** RK3588 DT gives `capacity-dmips-mhz` (A55 530,
A76 1024), a `cpu-map` and `dynamic-power-coefficient`. Linux scales dmips-mhz by each cluster's
max frequency and normalises to 1024 (`topology_normalize_cpu_scale`). ACPI: PPTT supplies
topology and caches but not capacity; capacity comes from CPPC `_CPC` (verify). The scheduler needs
per-CPU capacity: scale load and lag by it, place latency-critical (earliest-deadline) work on big
cores, migrate misfit tasks, and keep per-cluster MIDR for errata.

**Raspberry Pi specifics.** VideoCore mailbox (property channel 8): Pi 4 at 0xFE00_B880, Pi 5 at
0x10_7C01_3880 (`brcm,bcm2835-mbox`); write register +0x20, read +0x00, status +0x18 (full bit 31,
empty bit 30) (verify offsets); tag buffer 16-byte aligned, passed as a bus address (Pi 4: below
1 GiB, through `dma-ranges`) with the channel in the low 4 bits, cleaned before and invalidated
after the call. Useful tags: board revision, ARM/VC memory, clock get/set rates (EMMC, UART),
framebuffer allocate (verify tag ids against the firmware wiki). Prefer the firmware-created
`simple-framebuffer` node over the mailbox framebuffer. Pi 4 VL805 USB needs a firmware "notify
xHCI reset" mailbox call after PCIe reset (verify). Pi 5: firmware loads `kernel_2712.img` (16K
pages for Linux) else `kernel8.img`, and refuses an image with no compatible DTB unless `os_check=0`.
RP1 (southbridge on PCIe x4 `pcie2`) holds GPIO, the 40-pin UARTs (`arm,pl011-axi`), Ethernet,
2x USB 3, I2C/SPI and camera/display; its peripheral space (RP1 0xC0_4000_0000) appears at CPU
0x1F_0000_0000 only while the bootloader's PCIe setup survives (`pciex4_reset=0`). Full support needs
the `brcm,bcm2712-pcie` root complex driver; use `uart10` for bring-up.

**Loaders, EFI, Secure Boot.** Paths: Pi firmware direct (Image or flat binary), U-Boot `booti`
(Image) or `bootefi` (EFI subset per EBBR, DT allowed), EDK2 (Pi 4 `pftf/RPi4`, DT or ACPI, RAM
limited to 3 GB in ACPI mode by default (verify); RK3588 `edk2-rk3588`; servers). One Image with
the Linux dual header (MZ in code0, PE offset in `res5`) serves both. On EFI: take the memory map,
DTB (`EFI_DTB_TABLE_GUID`) or RSDP from the configuration table, `ExitBootServices`, and map
runtime services if used. Secure Boot: UEFI Secure Boot verifies the PE/COFF Authenticode
signature against db/dbx (SystemReady, EDK2 on Pi); Pi 4/5 also have an EEPROM signed-boot mode
(`boot.img` signed with a key whose hash is in OTP); RK3588 BootROM verifies the loader with an
OTP key hash, then U-Boot verifies FIT signatures; TF-A TBBR covers the firmware chain.

## 5. Bring-up checklist (priority order)

1. boot.S entry: save x0-x3, early VBAR, full EL2 init (N-177), INIT_SCTLR on every path (boot and
   secondary).
2. Image header and flat binary (`image_size` covers BSS and stack; flags LE, 4K pages, placement
   anywhere); load via U-Boot `booti` under QEMU at a non-0x40200000 address.
3. Position-independent early boot with the MMU on (N-28); cache maintenance primitives (HA-25);
   caches on before the first lock; gate test: lock acquired with the MMU on, QEMU
   `-cpu cortex-a72` and `-cpu max`.
4. DT-driven discovery from the saved FDT (cells, ranges, dma-ranges, interrupt cells), parser
   fuzzed.
5. Memory from `/memory` minus kernel, DTB, initrd and reserved ranges; delete the 128 MiB
   constant and HEAP_START.
6. Console via `stdout-path`; PL011 and ns16550/DW; one console for every consumer.
7. Timer: CNTFRQ check with DT fallback; INTID from `/timer`.
8. GIC v2 and v3 back ends; IPIs by hardware id.
9. MIDR/ID-register survey per CPU: errata flags, feature intersection, capacity.
10. Gate QEMU-only devices (virtio-mmio, ramfb) on their DT nodes; `simple-framebuffer`.
11. CPUs: full MPIDR, PSCI version/features/off/reset, spin-table.
12. First hardware: Raspberry Pi 4 (GICv2, PL011 after `disable-bt` or mini-UART with
    `enable_uart=1`, EL2 with x0 = DTB, `kernel8.img`); then Pi 5 (`uart10`, `os_check=0`).
13. Then RK3588 (GICv3, ITS, big.LITTLE), then UEFI + ACPI (SystemReady), PCIe ECAM, SMMUv3 and
    the Broadcom root complex.

## 6. Validation without hardware (QEMU matrix)

| Configuration | Exercises |
|---|---|
| `-kernel Image` (flat) at two different load addresses; `-bios u-boot.bin` + `booti` | HA-01 to HA-04 |
| `-machine virt,gic-version=3` (default picks v2 only for <= 8 CPUs; also `4`, `max`) | HA-07, HA-11, ITS |
| `-machine virt,virtualization=on` | EL2 entry (HA-14); QEMU's own PSCI switches to SMC |
| `-machine virt,secure=on` with TF-A + EDK2/U-Boot firmware | QEMU PSCI disabled, secondaries start running and the firmware parks them (real PSCI, HA-22); without firmware QEMU keeps its PSCI |
| `-machine virt,iommu=smmuv3` | SMMUv3, IORT/`iommu-map` |
| `-machine virt,acpi=on -bios QEMU_EFI.fd` | EFI + ACPI path (HA-24), cheaper than sbsa-ref |
| `-m 1G`, `-m 8G` | HA-17 |
| `-cpu max` (LSE, PAuth, BTI, larger PA), `-cpu cortex-a76`, `-cpu cortex-a55`, `-cpu cortex-a53` | HA-16 IPS selection, feature and MIDR paths |
| `-machine raspi4b` (QEMU 9.0+ (verify)): 4x A72, 2 GiB, GIC-400, PL011 + AUX mini-UART, mailbox/property, framebuffer; no PCIe, no GENET | Pi 4 addresses, mini-UART, spin-table at 0xd8-0xf0 (QEMU writes its own stub; entry EL (verify)) |
| `-machine sbsa-ref` with TF-A + EDK2 (two pflash) | GICv3, AHCI, XHCI, PCIe E1000E; UEFI + ACPI only (HA-24) |

## 7. Sources

- [Linux arm64 booting.rst](https://www.kernel.org/doc/html/latest/arch/arm64/booting.html)
  (checked 2026-10-07): Image header and flags, 2 MiB placement, DTB 8-byte/2 MiB rules, x0 = DTB,
  MMU off and image cleaned to PoC by VA, CNTFRQ/CNTVOFF, "all writable registers at or below the
  entry EL initialised", CPTR_EL2.TFP, ICC_SRE_EL2 (GICv3) and GICv5, spin-table (`cpu-release-addr`,
  `/memreserve/`, `wfe`/`sev`) and PSCI.
- Linux `Documentation/arch/arm64/silicon-errata.rst`, `arch/arm64/Kconfig` (`ARM64_ERRATUM_*`),
  `arch/arm64/kernel/cpu_errata.c`, `cpufeature.c`, `include/asm/el2_setup.h`, `kernel/head.S`,
  `drivers/irqchip/irq-gic.c`, `irq-gic-v3.c`, `irq-gic-v3-its.c`, `arch_timer.c`.
- [raspberrypi/linux rpi-6.12.y](https://github.com/raspberrypi/linux/tree/rpi-6.12.y/arch/arm64/boot/dts/broadcom):
  `bcm2712.dtsi` (GIC-400 at 0x7fff9000 under `soc@107c000000` ranges, `uart10` PL011 0x7d001000,
  mailbox 0x7c013880, PSCI 1.0 smc, TF-A reserved 0-0x80000, timer PPIs, PCIe `msi-ranges`),
  `bcm2712-rpi-5-b.dts` and `rp1.dtsi` (RP1 on `pcie2`, UARTs at RP1 0xc0_40030000),
  `arch/arm/boot/dts/broadcom/bcm2711.dtsi` (GIC-400 0x40041000 via `ranges` to 0xff800000,
  spin-table 0xd8-0xf0, `dma-ranges`), `bcm283x.dtsi` (mailbox 0x7e00b880).
- [Raspberry Pi config.txt boot options](https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/config_txt/boot.adoc)
  (`kernel_2712.img`, `arm_64bit`, `enable_rp1_uart`, `pciex4_reset`, `os_check`, `uart_2ndstage`)
  and [legacy boot.adoc](https://github.com/raspberrypi/documentation/blob/master/documentation/asciidoc/computers/legacy_config_txt/boot.adoc)
  (`kernel_address`: 64-bit default 0x200000).
- [Pi forum: GIC-400 on BCM2712](https://forums.raspberrypi.com/viewtopic.php?t=371974);
  [raspberrypi/arm-trusted-firmware bcm2712](https://github.com/raspberrypi/arm-trusted-firmware/tree/bcm2712);
  [rpi-eeprom 2712 release notes](https://github.com/raspberrypi/rpi-eeprom/blob/master/firmware-2712/release-notes.md).
- Linux `arch/arm64/boot/dts/rockchip/rk3588-base.dtsi` (GICv3 0xfe600000/0xfe680000, two ITS
  `dma-noncoherent`, `#interrupt-cells = <4>`, DW UARTs `reg-shift = <2>`/`reg-io-width = <4>`,
  `capacity-dmips-mhz` 530/1024, PSCI smc) and `allwinner/sun50i-h6.dtsi` (GIC-400 0x03021000,
  DW `uart0` 0x05000000).
- [edk2-platforms AmpereAltraPkg Ac01.h](https://github.com/tianocore/edk2-platforms/blob/master/Silicon/Ampere/AmpereAltraPkg/Include/Platform/Ac01.h)
  (`AC01_GET_MPIDR`: socket << 32 = Aff3, cluster in Aff2, core in Aff1).
- QEMU `hw/arm/virt.c` (PSCI conduit selection, GIC version and CPU limits), `hw/arm/raspi.c`
  (spin-table stub), `docs/system/arm/raspi.rst`, `docs/system/arm/sbsa.rst`.
- Arm ARM (DDI 0487), GICv2 (IHI 0048) and GICv3/v4 (IHI 0069), PSCI (DEN 0022), SMCCC (DEN 0028),
  BSA (DEN 0094), SBSA (DEN 0029), BBR (DEN 0044), EBBR, Devicetree Specification.
- Still to verify against primary sources: items marked (verify) above, chiefly the counter
  frequencies, mailbox register offsets and tag ids (raspberrypi/firmware wiki), Pi board CPU
  revisions, the RK3588 GIC implementation, and the QEMU release that added `raspi4b`.
