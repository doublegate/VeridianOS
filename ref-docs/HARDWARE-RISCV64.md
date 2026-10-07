# VeridianOS on real RISC-V (RV64) hardware

Status: reference, 2026-10-07 (code references re-verified against `feat/v0.27` at c38a422). Scope:
what the RISC-V kernel assumes about QEMU `virt` (OpenSBI `-bios default`, ELF at 0x80200000,
16550 UART at 0x1000_0000, PLIC at 0x0C00_0000, Sv48, MMU off today) and what it must do to boot
on real boards. Findings carry stable IDs (HR-nn) for citation from `to-dos/MASTER_TODO.md`
(none cited there yet); related audit IDs (N-nn) are in `docs/audit/`. Paths are under
`kernel/src/`. The build target is the built-in `riscv64gc-unknown-none-elf` (lp64d hard-float,
`medany`, static relocation; `build-kernel.sh:93`), linked by `arch/riscv64/link.ld` via
`kernel/build.rs:45-46`.

## 1. Bottom line

The kernel boots only because QEMU `virt` matches a dozen hard-coded assumptions. It runs with the
MMU off (`satp = 0`) at a fixed physical address, its page-table code writes x86-format entries,
and U-Boot cannot load it. Four items block boot on any real board: BSS is never zeroed, there is
no Image header, the load address is fixed, and raw stores go to 0x1000_0000 (the PRCI clock
controller on FU740, the PLIC on D1). Beyond boot, there is no PCIe on RISC-V at all (HR-34), the
trap vector treats external interrupts as fatal (HR-31), and IPIs address logical CPU ids as hart
ids (HR-33). What already works: boot hart id from a0 and DTB from a1, timebase from the DT, SMP
enumeration that skips disabled and `riscv,none` harts, HSM/RFENCE probing, RFENCE-based TLB
shootdown.

## 2. Target hardware

Facts from mainline Linux 6.19 DTS (local tree `arch/riscv/boot/dts/`), cross-checked with QEMU
11.1.2 `dumpdtb`. RAM base comes from `/memory` (board .dts or firmware); OpenSBI reserves its own
region via `/reserved-memory` (`no-map`, from `fdt_reserved_memory_fixup`).

| Board (SoC) | Harts | MMU | RAM base | Console UART | PLIC base, `riscv,ndev` | Timebase | Notes |
|---|---|---|---|---|---|---|---|
| StarFive VisionFive 2 (JH7110) | cpu@0 S7 (`sifive,s7`, rv64imac, no `mmu-type`, `status = "disabled"`) + cpu@1-4 U74 (`sifive,u74-mc`) | Sv39 | 0x4000_0000 (2/4/8 GiB) | `snps,dw-apb-uart` at 0x1000_0000, `reg-shift = 2`, `reg-io-width = 4` | 0x0C00_0000, 136 | 4 MHz | PLIC contexts: 0 = hart 0 M, then M/S pairs for harts 1-4 |
| SiFive HiFive Unmatched (FU740) | cpu@0 S7 + cpu@1-4 U74, all `sifive,bullet0`; cpu@0 rv64imac, no `mmu-type`, disabled | Sv39 | 0x8000_0000 (16 GiB) | `sifive,uart0` (not 8250) at 0x1001_0000 | 0x0C00_0000, 69 | 1 MHz | 0x1000_0000 is `sifive,fu740-c000-prci` |
| Allwinner D1 (C906) | 1, `thead,c906` | Sv39, XTheadMae | 0x4000_0000 (512 MiB-2 GiB, from U-Boot) | `snps,dw-apb-uart` at 0x0250_0000, shift 2 | 0x1000_0000, 175, `thead,c900-plic`, 2-cell specifiers | 24 MHz | `dma-noncoherent`; `xtheadvector`, `thead,vlenb = <16>` |
| T-Head TH1520 (4 x C910) | 4 | Sv39, XTheadMae | 0x0 | DW 8250 at 0xFF_E701_4000, shift 2 | 0xFF_D800_0000, 240, `thead,c900-plic` | 3 MHz | `dma-noncoherent`; `xtheadvector` |
| Sophgo SG2042 (64 x C920) | 64 | Sv39, XTheadMae | 0x0 (firmware-provided, verify) | DW 8250 at 0x70_4000_0000, shift 2 | 0x70_9000_0000, 224, `thead,c900-plic` | 50 MHz | `dma-noncoherent`; per-cluster ACLINT mtimer |
| SpacemiT K1 (8 x X60, e.g. BPI-F3) | 8 | Sv39, Svpbmt | 0x0 (verify) | `intel,xscale-uart` at 0xD401_7000 | 0xE000_0000, 159 | 24 MHz | RVV 1.0, Sstc, Zicbom (`riscv,cbom-block-size = <64>`), Svnapot, Svinval, Sscofpmf; `dma-noncoherent` |
| ESWIN EIC7700 (4 x P550, HiFive Premier) | 4 | Sv48 | (verify) | DW 8250 at 0x5090_0000 | 0x0C00_0000, 520 | 1 MHz | `dma-noncoherent`; H extension |
| RVA23 SoCs (2025+) | | Sv39 mandatory, Sv48/57 optional | | | APLIC/IMSIC expected (verify per SoC; none upstream in 6.19) | | Svpbmt, Sstc, Svade, Svinval, Svnapot, Zicbom, V mandatory (section 4.9) |

None of these cores implements Sstc except K1 and RVA23 parts; none has Svpbmt except K1 and
RVA23 parts (T-Head uses XTheadMae instead).

### 2.1 Boot chains

| Board | Chain | DTB source | Kernel entry |
|---|---|---|---|
| VisionFive 2 | mask ROM (boot-mode switches: QSPI/SD/eMMC/UART) -> U-Boot SPL (DDR init, M-mode) -> FIT `u-boot.itb` = OpenSBI `fw_dynamic` + U-Boot proper (S-mode) | U-Boot loads `starfive/jh7110-starfive-visionfive-2-v1.{2a,3b}.dtb` (`fdtfile`) or uses its control DT; OpenSBI fixes it up | `booti` (Image header), `bootefi` (PE/COFF), extlinux; `kernel_addr_r = 0x40200000` (verify `fdt_addr_r`) |
| HiFive Unmatched | ZSBL in mask ROM -> U-Boot SPL (SD or SPI flash) -> FIT with OpenSBI `fw_dynamic` + U-Boot | U-Boot (`hifive-unmatched-a00.dtb`) | `booti`/`bootefi`; `kernel_addr_r = 0x84000000` (verify) |
| D1 boards | BROM -> U-Boot SPL (sunxi, M-mode) -> OpenSBI `fw_dynamic` -> U-Boot | U-Boot | `booti`/`bootefi` |
| TH1520 (LicheePi 4A) | BROM -> vendor U-Boot SPL -> OpenSBI -> U-Boot (mainline SPL in progress, verify) | U-Boot | `booti`/`bootefi` |
| SG2042 (Milk-V Pioneer) | ZSBL -> OpenSBI `fw_dynamic` -> LinuxBoot (Linux + u-root) or EDK2 (verify) | ZSBL/OpenSBI | kexec of an Image, or EFI from EDK2 |
| QEMU `virt` | `-bios default` (OpenSBI `fw_dynamic`) -> `-kernel` | QEMU generates; OpenSBI fixes up | ELF at its link address; a flat Image at RAM + 2 MiB |

Every real board reaches the kernel through U-Boot or EFI, so HR-02/HR-03 are mandatory. The DTB
is OpenSBI-fixed-up: `fdt_cpu_fixup` disables harts outside the root domain or without an MMU and
appends `zicntr` when it emulates `time`; `fdt_reserved_memory_fixup` adds `no-map` nodes for its
PMP regions. An EFI stub gets the boot hart from `RISCV_EFI_BOOT_PROTOCOL`, not a0.

## 3. Findings

Severity: **B** blocks boot; **D** breaks a device or subsystem; **L** latent.

| ID | Location | QEMU assumption | Real hardware | Sev | Fix |
|---|---|---|---|---|---|
| HR-01 | `arch/riscv64/boot.S:48-84`; `arch/riscv64/link.ld:26-34` | BSS already zero (`__bss_start/__bss_end` defined, never used) | A flat Image (booti, fw_payload) has no BSS contents: GlobalState, atomics, the 8 MiB heap start as garbage | B | Zero BSS in boot.S before `_start_rust` |
| HR-02 | `link.ld:6`; target is static | Runs at link address 0x80200000, MMU off | Linux protocol: any 2 MiB-aligned address; VisionFive 2 uses 0x40200000; D1/TH1520/SG2042 RAM does not even contain 0x8020_0000 on small configs | B | `medany` code is PC-relative already; absolute data (vtables, fn-pointer statics) needs PIE + `R_RISCV_RELATIVE` self-relocation, or an early Sv39 map at a fixed high VA (Linux `setup_vm`) |
| HR-03 | `boot.S:16-19` | ELF loaded by OpenSBI | U-Boot `booti` and EFI need the 64-byte header (`boot-image-header.rst`): `code0/code1` 0x0/0x4 (`code0` = "MZ" for EFI), `text_offset` 0x8 (Linux: 0x200000), `image_size` 0x10 (mandatory), `flags` 0x18 (bit 0 = BE), `version` 0x20 (0.2), `magic` "RISCV\0\0\0" 0x30 (deprecated), `magic2` "RSC\x05" 0x38, `res3` 0x3C = PE offset | B | Header in `.text.boot`; `Image` (objcopy -O binary) artifact; optional PE/COFF for `bootefi`/EDK2 |
| HR-04 | `main.rs:207`, `bootstrap.rs:230,750` | Raw byte stores to 0x1000_0000 (16550 THR) | FU740: PRCI clock/PLL registers; JH7110: DW UART needs 32-bit accesses and a clock enabled by firmware; D1: PLIC priority 0 | B/D | Delete (SBI console works); DT `stdout-path` early console later |
| HR-05 | `drivers/virtio/mmio.rs:43-44` (via `bootstrap.rs:547`, `drivers/virtio/blk.rs:622-640`) | 8 virtio-mmio slots from 0x1000_1000 | None on hardware; on FU740/JH7110 0x1000_1000 is inside another UART/peripheral block, on D1 inside the PLIC | B (likely) | Probe only DT `virtio,mmio` nodes |
| HR-06 | `drivers/ramfb.rs:22` (via `bootstrap.rs:389`) | fw_cfg at 0x0902_0000 (the AArch64 address; comment claims both match) | Wrong even on QEMU riscv: `fw-cfg@10100000` (`qemu,fw-cfg-mmio`, confirmed by dumpdtb); on hardware an unmapped access | D/B | DT `qemu,fw-cfg-mmio` + "QEMU" signature, else skip |
| HR-07 | `mm/mod.rs:499-500` | RAM = 0x8000_0000 + 128 MiB; no reservations | RAM base 0x4000_0000 / 0x8000_0000 / 0x0, 0.5-16 GiB (SG2042 up to 128 GiB); OpenSBI's `no-map` region, DTB and initrd can be handed out | B/L | `/memory`, `/reserved-memory`, memreserve block, DTB and initrd ranges; copy the DTB early |
| HR-08 | `arch/riscv64/mod.rs:103,116`; `arch/fdt.rs:87` | `mmu-type` read from the first `/cpus/cpu` node; Sv48 assumed when absent | First node is the S7 monitor core (no `mmu-type`), so `SV48` stays true on Sv39 hardware; an unsupported `satp` MODE is WARL-ignored, so the MMU silently stays off. Reproducible on QEMU `sifive_u` (cpu@0 = E51, no `mmu-type`) | B (once MMU on) | Read the node whose `reg` == boot hart; default Sv39; write MODE 9, read back, fall back to 8 (and probe 10) |
| HR-09 | `mm/page_table.rs:328-343` | Always 4-level (Sv48), panics otherwise | Sv39 is the RVA20/22/23 baseline; JH7110, FU740, D1, TH1520, SG2042, K1 are Sv39 only | B | Runtime 3/4/5-level tables and per-mode kernel VA layout (Linux `set_satp_mode()`) |
| HR-10 | `mm/page_table.rs:64-65` (`frame << 12`), `mm/mod.rs:148-160` | x86 PTE layout | RISC-V: V=0 R=1 W=2 X=3 U=4 G=5 A=6 D=7, RSW 8-9, PPN at bit 10, PBMT 61-62, N 63; Svade cores (all listed, and mandatory in RVA23) fault on clear A/D | B (with MMU) | Per-arch PTE encoder; set A/D on kernel mappings; leaf = any of R/W/X |
| HR-11 | PTE path, no errata | PMA only | Svpbmt: PBMT 1 = NC, 2 = IO, enabled by `menvcfg.PBMTE` (firmware). XTheadMae (C906/C910/C920, detected by `mvendorid` 0x5B7, `marchid` = `mimpid` = 0 and `th.sxstatus.MAEE`, CSR 0x5C0): PMA = bits 62/61/60, NC = 61/60, IO = 63/60, mask includes 59 (Linux `pgtable-64.h`) | D/B (T-Head) | Attribute table from `riscv,isa-extensions` (`svpbmt`) and SBI BASE `mvendorid/marchid/mimpid` |
| HR-12 | `riscv/plic.rs:37,86,124-126,392` | Fixed base, 128 sources, `s_context = 2*hart+1` | On JH7110/FU740 context 0 is hart 0 M only, so hart 1's S context is 2 and the formula picks hart 2's M context; `ndev` 136/175/240/224 exceed 127, so high sources can never be enabled; D1/TH1520/SG2042/K1 bases differ | D | PLIC from DT (`riscv,plic0`, `sifive,plic-1.0.0`, `thead,c900-plic`): `reg`, `riscv,ndev`, context index = position in `interrupts-extended` of this hart's entry with irq 9 (skip 0xffffffff placeholders) |
| HR-13 | `plic.rs:70-77` | UART IRQ 10, virtio 1-8 | Per device node; T-Head PLICs use `#interrupt-cells = <2>` (irq, type) and need edge handling (Linux `PLIC_QUIRK_EDGE_INTERRUPT`); OpenSBI grants S-mode access to the C900 PLIC (verify) | D | DT `interrupts` / `interrupt-parent`, 1- or 2-cell specifiers |
| HR-14 | none | PLIC always present | QEMU `virt,aia=aplic-imsic` replaces the PLIC with APLICs at 0x0C00_0000 (M) / 0x0D00_0000 (S) and IMSICs at 0x2400_0000 (M) / 0x2800_0000 (S) (dumpdtb); RVA23 SoCs likely the same | D/L | Detect `riscv,aplic`/`riscv,imsics`; direct-mode APLIC first, IMSIC (MSI, needs Ssaia `stopei`) later |
| HR-15 | `riscv/timer.rs:22,69,74-79` | 10 MHz fallback; Sstc from first cpu node | Timebases 1/3/4/24/50 MHz; Sstc absent on U74/C906/C910 (SBI path); Sstc also needs `menvcfg.STCE` from firmware. QEMU 11 `rv64` advertises Sstc by default | L | Boot-hart node; no guessed frequency; Sstc only when the DT says so (OpenSBI sets STCE when it detects Sstc) |
| HR-16 | `arch/timer.rs:163` | `set_timer` always via SBI | Inconsistent with the Sstc path | L | Route through `riscv::timer::arm` |
| HR-17 | `boot.S:25-46`, `boot.rs:88-93`, `entry.rs:40-46`, `riscv64/serial.rs:40-45`, `sbi.rs:141-148` | SBI v0.1 legacy console; a0 declared input-only in the inline asm | Legacy extensions optional since SBI 0.2 and deprecated; DBCN (0x4442434E, SBI 2.0) replaces them; legacy calls return in a0, so `in("a0")` is UB as written | L/B | Probe DBCN, legacy fallback; `inlateout("a0")`; DT UART driver later |
| HR-18 | `sbi.rs:135-138,152-180` | SBI >= 0.2 | BBL/SBI 0.1 has no BASE extension; `probe_extension` ignores the error | L | Read spec version (BASE fid 0); require TIME, IPI, RFENCE, HSM with clear errors |
| HR-19 | `boot.S:19-23,81` | Only the boot hart enters | Without HSM (`RISCV_BOOT_SPINWAIT` firmware) all harts enter together and share `__stack_top` | L | Hart lottery (atomic in `.data`), losers park |
| HR-20 | `riscv/context.rs:392-404` (`csrs mstatus`), `412-433` (`csrr misa`) | FP usable; `mstatus`/`misa` readable | `mstatus`/`misa` are M-mode CSRs (illegal from S-mode). The kernel is built lp64d, so `sstatus.FS` must be non-Off before any FP instruction; OpenSBI enables FS at hart init when F/D exist (verify), U-Boot may not | B/L | Soft-float kernel (N-178); `sstatus` and DT `riscv,isa-extensions` |
| HR-21 | `perf/pmu.rs:249,260` | `mcycle`/`minstret` readable | Illegal from S-mode | L | `rdcycle`/`rdinstret` (needs `mcounteren`) or the SBI PMU extension (section 4.8) |
| HR-22 | `arch/entropy.rs:49`, `security/stack_canary.rs:198`, `security/kaslr.rs:247` | `rdcycle` is a seed | Needs `mcounteren.CY`; low entropy (N-149) | L | Zkr `seed` (with `mseccfg.SSEED`), virtio-rng, or a board TRNG |
| HR-23 | `riscv/timer.rs:51-58` | `time` CSR in hardware | Resolved: SiFive U54/U74 do not implement `time`; `rdtime` traps and OpenSBI emulates it (`sbi_emulate_csr.c`, `CSR_TIME` -> `sbi_timer_value()`), an M-mode round trip per read. C9xx implement it (verify) | L (perf) | Keep `rdtime` out of hot paths (scheduler accounting); use the tick where resolution allows |
| HR-24 | no cache maintenance | DMA coherent | D1, TH1520, SG2042, K1, EIC7700 are `dma-noncoherent`; Zicbom boards give `riscv,cbom-block-size`; T-Head uses `th.dcache.{c,i,ci}{pa,va}`; JH7100 used the SiFive ccache flush register | D (future drivers) | DMA API honouring `dma-noncoherent`: Zicbom `cbo.*`, T-Head CMO, SiFive ccache, or Svpbmt/XTheadMae NC mappings |
| HR-25 | `riscv64/mod.rs:282-290`; `mm/heap.rs:53` | `serial_init` at 0x1000_0000; `HEAP_START = 0x8100_0000` | QEMU map (cf. N-134 for the `vas.rs` copy) | L | Delete or DT-driven |
| HR-26 | `mm/cache_topology.rs:439-461` | U74-like caches hard-coded | DT `{i,d}-cache-{size,sets,block-size}`, `next-level-cache`, `cache-level` | L | Parse the DT |
| HR-27 | `riscv64/smp.rs:55-60` | Entry VA == PA | HSM `hart_start` takes a physical address | L | Pass PAs once the kernel runs at a high VA |
| HR-28 | `drivers/input.rs:146-160` | Keyboard via SBI legacy getchar | Optional (HR-17) | D | DBCN read or the UART driver |
| HR-29 | `riscv64/mod.rs:294-301`, `mm/page_table.rs:858-867` | `sfence.vma addr` flushes that page | SiFive CIP-1200: on `marchid` 0x8000000000000007 with `mimpid[23:0]` <= 0x200630 (U54/U74 class) the page-specific form is unreliable; Linux replaces it with a full flush | D (with MMU) | Errata check via SBI BASE `marchid`/`mimpid`; full `sfence.vma` on affected cores |
| HR-30 | `mm/page_fault.rs:450-474` | `stval` is a canonical VA | SiFive CIP-453 (same `marchid`, `mimpid` 0x20181004-0x20191105): `stval` not sign-extended on instruction page faults | L | Sign-extend from the active VA width on affected cores |
| HR-31 | `riscv64/mod.rs:139-153` | Only timer (5) and software (1) interrupts arrive | Supervisor external interrupt (scause 9, PLIC/APLIC) and every U-mode trap go to `riscv_fatal_trap`: no device interrupt can ever be serviced | D | Claim/complete loop for scause 9; U-mode entry with `sscratch` (N-14) |
| HR-32 | `mm/page_table.rs:335` | ASID always 0 | Fine with one address space; per-process `satp` then needs a full flush on every switch | L (perf) | Probe ASIDLEN (write all-ones to `satp.ASID`, read back; may be 0), allocate ASIDs with generations, `sfence.vma va, asid` for user flushes |
| HR-33 | `sched/smp.rs:462`, `sched/smp.rs:239-246` | Logical CPU n is hart n; one CPU | JH7110/FU740 boot harts 1-4 (hart 0 is the S7), OpenSBI may pick any boot hart: `1 << target_cpu` IPIs the wrong hart; `detect_riscv` reports 1 CPU | D (SMP) | IPI via the per-CPU hart id (`percpu::install`), `hart_mask_base` for hart ids >= 64 (SG2042); topology from the DT |
| HR-34 | `drivers/pci.rs:599-625`, `bootstrap.rs:525-532` | PCI = x86 port I/O, gated to x86_64 | No PCIe on RISC-V at all: QEMU `virt` gpex (`pci-host-ecam-generic`), JH7110 PLDA (`starfive,jh7110-pcie`, 2 x Gen2 x1, cfg at 0x9_4000_0000/0x9_C000_0000), FU740 DWC (`sifive,fu740-pcie`, Gen3 x8), SG2042 4 x `sophgo,sg2042-pcie-host`. NVMe and most USB on these boards sit behind PCIe | D | ECAM from DT `ranges`/`reg`; per-SoC host-bridge init (PHY, reset, link training); MSI via PLIC-attached MSI controllers or IMSIC |
| HR-35 | `riscv64/mod.rs:273-280` | `fence.i` is a speculation barrier | It only synchronises the local hart's instruction fetch; RISC-V has no architectural speculation barrier. Cross-hart code patching needs SBI RFENCE `remote_fence_i` (fid 0) | L | Rename to `icache_sync_local`; add the remote variant for module loading |
| HR-36 | `arch/fdt.rs:259-262`; `timer.rs:74-79` | Extensions from one node, `_`-separated tokens | Single-letter extensions in the base token of `riscv,isa` (`v`, `h`) never match; harts can differ; `xtheadvector` is not `v` | L | Prefer `riscv,isa-extensions`; usable ISA = intersection over enabled harts (as Linux) |
| HR-37 | none (no `sstatus.VS` handling) | No vector unit | K1/RVA23: RVV 1.0, `sstatus.VS` bits 9-10, state = 32 x VLEN + `vstart/vtype/vl/vcsr`. C9xx: XTheadVector (RVV 0.7.1 encoding, VS at bits 23-24, Linux `SR_VS_THEAD`), and GhostWrite (CVE-2024-44067: some vector stores bypass the MMU) | L (sprint E) | VS Off in the kernel; lazy per-task save; enable XTheadVector never (Linux marks the whole C9xx class vulnerable) |
| HR-38 | `sbi.rs:16` (`SBI_EXT_SRST`, unused) | QEMU exits on its own | No shutdown/reboot path on RISC-V | D (power) | SBI SRST `system_reset(type 0/1/2, reason)`; legacy 0x08 fallback |
| HR-39 | `riscv64/mod.rs:264-268`, `riscv64/smp.rs:79-84` | `wfi` idle is enough | Correct but shallow; deeper states need SBI HSM `hart_suspend` with DT `riscv,idle-state` params | L | Retentive suspend first; non-retentive (bit 31) resumes at a PA with the MMU off, so it needs the HR-27 path |
| HR-40 | `drivers/iommu.rs:590` (non-x86 stub) | No IOMMU needed | None of the listed SoCs has an IOMMU; the RISC-V IOMMU (spec v1.0) appears only on newer SoCs and QEMU (`-device riscv-iommu-pci`, `virt,iommu-sys=on`, both verified) | L (design) | User-space drivers with DMA (critique C6) cannot be isolated on these boards; document, and add a `riscv,iommu` driver for QEMU/RVA23 |

Related audit items: N-14 (U-mode, `tp` via `sscratch`), N-170 (`sstatus` SPP sanitiser), N-178
(soft-float), N-26 (guard pages), N-149 (entropy), N-134 (dead `HEAP_START` copy).

## 4. Target design

**4.1 Entry.** boot.S: hart lottery, zero BSS, set `sstatus.FS` per the float decision (N-178), set
up an early Sv39 mapping that places the kernel at a fixed high VA from wherever it was loaded
(PC-relative code only before `satp` is written, Linux `-fno-pie -mcmodel=medany` rule), then call
Rust. Image header at the start of the image. Accept a0 = hart id, a1 = DTB PA, `satp = 0`.

**4.2 MMU and TLB.** Probe the mode: write MODE 9 (Sv48), read back, fall back to 8 (Sv39); probe 10
(Sv57) when wanted. Page-table depth and kernel VA layout follow the mode. A RISC-V PTE encoder sets
A/D on kernel mappings (Svade; Svadu needs `menvcfg.ADUE`) and maps memory types through Svpbmt or
XTheadMae. Rules: after changing a valid PTE, `sfence.vma va, asid` locally (rs2 = x0 only for
global mappings) and RFENCE `remote_sfence_vma_asid` (fid 2) for other harts; after making an
invalid PTE valid a fence is still required unless Svvptc is present (otherwise expect one
spurious fault); no fence is needed on `satp` writes when switching to an ASID that holds no stale
entries; CIP-1200 cores use full flushes. Svinval (`sinval.vma` between `sfence.w.inval` and
`sfence.inval.ir`) batches unmaps. Svnapot gives 64 KiB contiguous mappings (N bit 63,
`ppn[3:0] = 0b1000`).

**4.3 Platform discovery.** FDT parsing for `/memory`, `/reserved-memory`, `/chosen`
(`stdout-path`, initrd), the boot hart's cpu node (`mmu-type`, `riscv,isa-extensions`,
`riscv,cbom-block-size`, `riscv,cboz-block-size`), `timebase-frequency`, PLIC/APLIC/IMSIC,
virtio-mmio, fw_cfg, PCIe hosts and caches. Vendor ids from SBI BASE (fids 4-6) for errata.

**4.4 SBI.** Spec version and extension probing up front; DBCN console with legacy fallback; HSM
with physical entry addresses; RFENCE required for SMP (already checked); IPI by hart id; SRST for
power; SUSP (system suspend, SBI 2.0) later; FWFT (`MISALIGNED_EXC_DELEG`) when present.

**4.5 Interrupts.** PLIC context map from `interrupts-extended`; T-Head 2-cell specifiers and edge
quirk; APLIC direct mode, then IMSIC with per-hart interrupt files (MSI for PCIe).

**4.6 DMA and IOMMU.** One DMA API per architecture; on RISC-V it uses Zicbom (`cbo.clean`,
`cbo.inval`, `cbo.flush`, enabled for S-mode by `menvcfg.CBIE/CBCFE`) or vendor cache maintenance
for `dma-noncoherent` devices, or NC/IO mappings. Without an IOMMU, DMA-capable drivers stay in the
TCB on these boards.

**4.7 Useful extensions.** Zicboz `cbo.zero` for page zeroing; Zihintpause `pause` in spin loops
(a HINT encoding, safe to emit unconditionally); Zawrs `wrs.nto`/`wrs.sto` for lock waiting;
Zkr `seed` for entropy; Svnapot/Svinval as above. Misaligned accesses: U74 traps and OpenSBI
emulates (hundreds of cycles, verify), C9xx handle them in hardware when firmware sets
`mxstatus.MM` (verify), RVA23 Zicclsm only promises they work, not that they are fast. LLVM for
`riscv64gc` without `+unaligned-scalar-mem` already splits `read_unaligned`; never cast unaligned
pointers in packet or descriptor parsing.

**4.8 PMU.** Counters through the SBI PMU extension (0x504D55: `num_counters`, `counter_get_info`,
`counter_config_matching`, `start/stop`, `fw_read`); overflow interrupts need Sscofpmf (LCOFI, irq
13, `scountovf` 0xDA0) or the T-Head variant (irq 17, CSR 0x5C5); DT `riscv,pmu`
`riscv,event-to-mhpmcounters` maps events (D1).

**4.9 RVA23 profile.** RVA23S64 mandates (as modelled by QEMU `-cpu rva23s64`) Sv39, Svade,
Svpbmt, Svinval, Svnapot, Sstc, Sscofpmf, Ssnpm, H, plus RVA23U64's V, Zicbom/Zicbop/Zicboz,
Zawrs, Zihintpause, Zicond, Zba/Zbb/Zbs and Zicclsm. Sv48/Sv57, Svadu, Ssaia and Zkr are
optional. An RVA23 target can assume Sstc, Svpbmt and Zicbom and drop the SBI timer and vendor
paths.

**4.10 Scheduler (ADR 0007 placement).** Exclude monitor harts (S7: no MMU, no S-mode) before
assigning logical ids. SG2042 has 16 clusters of 4 cores sharing an L2 and several NUMA nodes
(`numa-node-id`, `distance-map`; verify): the "cache-hot previous CPU" and idle-pull rules should
prefer the same cluster, then the same node. ADR 0007 has no capacity model; heterogeneous RISC-V
SoCs (big and little clusters, DT `capacity-dmips-mhz`) need capacity-aware wakeup placement, and
harts with differing ISAs (for example only some with V) need affinity for tasks that use the
extension.

## 5. Bring-up checklist (priority order)

1. boot.S: zero BSS, `sstatus.FS` (or soft-float, N-178), hart lottery.
2. Delete raw 0x1000_0000 stores; gate virtio-mmio and ramfb on DT nodes; fix the riscv fw_cfg
   address.
3. Console: SBI DBCN with legacy fallback, fix the a0 clobber; later DT `stdout-path` drivers for
   8250/DW (`reg-shift`/`reg-io-width`) and `sifive,uart0`.
4. Image header and `Image` artifact; position-independent or MMU-early boot so 0x40200000 works;
   test with QEMU `-kernel Image` and U-Boot `booti`.
5. Memory map from the DT, with reservations and a DTB copy.
6. Boot-hart cpu node for `mmu-type` and extensions; Sv39 default; `satp` MODE probe.
7. RISC-V PTE encoder (A/D), 3/4/5-level tables, Svpbmt/XTheadMae attributes, CIP-1200 flushes.
8. PLIC from the DT (base, `ndev`, context from `interrupts-extended`, per-device IRQs), scause 9
   handling (HR-31); APLIC detection.
9. SBI hygiene: spec version, required extensions, HSM with physical addresses, IPI by hart id
   (HR-33), SRST.
10. Remove M-mode CSR use (`mstatus`, `misa`, `mcycle`, `minstret`).
11. DMA API with Zicbom, T-Head and SiFive cache maintenance before any real NIC/storage driver.
12. PCIe: ECAM on QEMU `virt` first, then the JH7110 PLDA host (NVMe, USB).
13. Cache topology and entropy (Zkr) from the DT; no 10 MHz fallback.

Sprint E (U-mode, N-14) depends on items 6-8: today's tables are x86-encoded and 4-level, so the
MMU bring-up is E's first task, Sv39-first.

### 5.1 Board peripherals (for driver planning)

| SoC | Ethernet | SD/eMMC | USB | PCIe |
|---|---|---|---|---|
| JH7110 | 2 x `snps,dwmac-5.20` (0x1603_0000, 0x1604_0000) | 2 x `starfive,jh7110-mmc` (DW MSHC) | `cdns,usb3` at 0x1010_0000; board USB-A via a PCIe xHCI (verify) | 2 x PLDA XpressRICH, Gen2 x1 |
| FU740 | `sifive,fu540-c000-gem` (Cadence GEM) at 0x1009_0000 | `mmc-spi-slot` on `spi0` (SPI mode) | via PCIe (verify) | DWC Gen3 x8 |
| D1 | `allwinner,sun20i-d1-emac` | `allwinner,sun20i-d1-mmc`/`-emmc` | MUSB OTG + EHCI/OHCI | none |
| TH1520 | 2 x `thead,th1520-gmac` (`snps,dwmac-3.70a`) | 3 x `thead,th1520-dwcmshc` | DWC3 (verify) | (not in mainline DT, verify) |
| SG2042 | `snps,dwmac-5.00a` | 2 x `sophgo,sg2042-dwcmshc` | via PCIe (verify) | 4 x `sophgo,sg2042-pcie-host` |

## 6. Validation without hardware (QEMU matrix)

All options below were accepted by QEMU 11.1.2 (`qemu-system-riscv64`, checked 2026-10-07); the
`mmu-type` results come from `-M virt,dumpdtb=`. `xtheadvector` is not a QEMU CPU property.

| Configuration | Exercises |
|---|---|
| `-M virt -cpu rv64,sv48=off` (DT says `riscv,sv39`; `sv57=off` gives Sv48) | HR-08, HR-09 |
| `-M sifive_u -smp 5` (FU540 model: cpu@0 E51 without `mmu-type`, `sifive,uart0` at 0x1001_0000, PRCI at 0x1000_0000, PLIC 53 sources, 1 MHz) | HR-04, HR-08, HR-12, HR-33 |
| `-M virt,aia=aplic` and `aia=aplic-imsic` (optionally `aia-guests=N`) | HR-12 to HR-14, HR-31 |
| `-kernel Image` (flat, loaded at RAM + 2 MiB) and U-Boot `qemu-riscv64_smode` with `booti`; `-m 1G`, `-m 8G` | HR-01 to HR-03, HR-07 |
| `-bios` OpenSBI built without legacy extensions (`CONFIG_SBI_ECALL_LEGACY=n`, verify name) | HR-17, HR-18 |
| `-smp 4 -cpu rv64,svpbmt=on,zicbom=on,sstc=off` (default `rv64` has Sstc and Svadu, not Svpbmt) | HR-11, HR-15, HR-24 |
| `-cpu rv64,svadu=off,svade=on` | HR-10 (A/D faults) |
| `-cpu rva23s64` (Sv39, V, Svpbmt, Svnapot, Svinval, Sscofpmf) | sections 4.7-4.9 |
| `-cpu thead-c906` (XThead* ISA string, no Sstc/Svpbmt; MAE PTE bits not modelled, verify) | HR-11 detection paths, HR-36 |
| `-cpu rv64,v=on,vlen=256` | HR-37 |
| `-device riscv-iommu-pci` or `-M virt,iommu-sys=on` | HR-40 |
| `-M virt` + `-device nvme` / `qemu-xhci` on the gpex bus | HR-34 |

Hardware order: VisionFive 2 (Sv39, 4 MHz timebase, RAM at 0x4000_0000, hart 0 = monitor core,
PCIe NVMe), then HiFive Unmatched (PRCI at 0x1000_0000, SiFive UART, CIP-1200), then a T-Head
board (XTheadMae, non-coherent DMA, 2-cell PLIC), then a K1 or RVA23 board (Svpbmt, Sstc, RVV 1.0).

## 7. Sources

- RISC-V Privileged Architecture, SBI specification v2.0/v3.0, RVA22/RVA23 profiles, AIA and
  IOMMU v1.0 specifications.
- Linux 6.19 (local tree): `Documentation/arch/riscv/boot.rst` and `boot-image-header.rst`;
  `arch/riscv/kernel/head.S` (header fields, `text_offset` 0x200000); `arch/riscv/include/asm/`
  `pgtable-64.h` (PBMT and T-Head bits), `csr.h` (`SR_VS_THEAD`, `ENVCFG_*`), `sbi.h`;
  `arch/riscv/errata/sifive/errata.c` (CIP-453, CIP-1200), `errata/thead/errata.c` (MAE, CMO,
  PMU, GhostWrite); `arch/riscv/mm/init.c` (`set_satp_mode`); `drivers/irqchip/irq-sifive-plic.c`,
  `irq-riscv-aplic-*`, `irq-riscv-imsic-*`.
- Mainline DTS: `starfive/jh7110.dtsi`, `jh7110-common.dtsi`; `sifive/fu740-c000.dtsi`,
  `hifive-unmatched-a00.dts`; `allwinner/sun20i-d1s.dtsi`, `sunxi-d1s-t113.dtsi`;
  `thead/th1520.dtsi`; `sophgo/sg2042.dtsi`, `sg2042-cpus.dtsi`; `spacemit/k1.dtsi`;
  `eswin/eic7700.dtsi`.
- OpenSBI master: `lib/sbi/sbi_emulate_csr.c` (`time` emulation), `lib/utils/fdt/fdt_fixup.c`
  (cpu and reserved-memory fixups); firmware docs (fw_dynamic, fw_jump, fw_payload).
- QEMU 11.1.2: `-M virt,dumpdtb=`, `-M sifive_u`, CPU properties as tested in section 6.
- rdtime on SiFive cores: U74 Core Complex Manual 21G3; Z. Yedidia, "Bare-metal development on
  the VisionFive 2".
- Items marked "(verify)" were not confirmed against primary sources.
