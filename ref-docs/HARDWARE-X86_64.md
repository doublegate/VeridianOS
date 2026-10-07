# VeridianOS on real x86_64 hardware

Status: reference, 2026-10-07 (code references re-verified on `feat/v0.27`). Scope: what the x86_64 kernel assumes about QEMU (q35/pc, OVMF, `bootloader` crate 0.11) and what
it must do instead to boot and run on real PCs, laptops and servers. Findings carry stable IDs
(HX-nn) used by `to-dos/MASTER_TODO.md`; related audit IDs (N-nn) are in `docs/audit/`. Paths are
under `kernel/src/`. "(verify)" marks a claim not checked against a primary source.

## 1. Target hardware classes

| Class | Firmware | Notable differences from QEMU |
|---|---|---|
| Desktop/laptop (Intel 8th gen and later, AMD Zen) | UEFI, often no CSM | No COM1, no 8042 or emulated by SMM via USB, 8254 PIT may be clock-gated, no HPET table on some, USB-only keyboards, GOP framebuffer only; laptops add an Embedded Controller, battery/lid/AC via AML, and often no S3 (Modern Standby only) |
| Server (Xeon, EPYC) | UEFI + ACPI, IPMI SOL | x2APIC handover, APIC IDs > 255 and sparse, multiple PCI segments, SOL on COM2 or MMIO UART (SPCR), MAXPHYADDR up to 52 bits, 64-bit BARs above 4 GiB, IOMMU possibly pre-enabled, machine checks and NMIs (BMC, PCIe SERR) actually delivered |
| Small boards (Atom/N-series) | UEFI | Less than 2 GiB RAM, PIT clock-gated and HPET possibly absent (Apollo Lake and later) |

## 2. Findings

Severity: **B** blocks boot on some real machines; **D** breaks a device or feature; **L** latent
or limits scale.

| ID | Location | QEMU assumption | Real hardware | Sev | Fix |
|---|---|---|---|---|---|
| HX-01 | `arch/x86_64/tsc.rs:127-152` (`pit_window`, wait at 147-149), `apic.rs:938-990` (wait at 983) | 8254 PIT counts; port 0x61 OUT2 loops have no timeout. TSC already tries CPUID 15h/16h and leaf 0x40000010 first (`tsc.rs:81-125`); the periodic-LAPIC path always calibrates with the PIT | Skylake/Apollo Lake and later PCHs can gate the PIT clock (ITSSPRC): registers program normally but nothing counts, so OUT2 never rises and both loops hang | B | Bounded loops; TSC from CPUID 15h/16h, then `MSR_PLATFORM_INFO` (0xCE, Intel only, #GP on AMD) x 100 MHz, HPET, ACPI PM timer, PIT last; calibrate the LAPIC timer against the TSC |
| HX-02 | `mm/heap.rs:39,47` | 1 GiB static heap in BSS | Bootloader must allocate and zero it: machines below ~1.5 GiB cannot load the kernel | B | Small static bootstrap heap; grow from frames sized by the memory map |
| HX-03 | `drivers/input.rs:56-121`, `fs/devfs.rs:108-300` | COM1 at 0x3F8 and an 8042 controller | Absent ports read 0xFF, so LSR "data ready" is always set and the shell reads endless 0xFF; many machines have neither | B | Probe the UART (scratch/loopback, LSR != 0xFF); console from ACPI SPCR, then DBG2, then GOP; gate 8042 on FADT `IAPC_BOOT_ARCH` bit 1 plus self-test (0xAA -> 0x55) |
| HX-04 | `arch/x86_64/mod.rs:129-145`, `apic.rs:567-645`, `apic.rs:112` | xAPIC MMIO works; APIC before ACPI; I/O APIC at 0xFEC00000; APIC base below 4 GiB (`apic.rs:577` masks with 0xFFFF_F000) | Firmware may hand over in x2APIC mode; xAPIC MMIO then decodes nothing (no tick, no EOI). (`smp.rs:152` only skips APs) | B | x2APIC (N-174); ACPI before APIC; I/O APICs from MADT; full-width base mask |
| HX-05 | `arch/x86_64/smp.rs:142`, `apic.rs:804`, `acpi.rs:42,413-436`, `sched/smp.rs:275` | 8-bit APIC IDs (CPUID.1 EBX[31:24], `start_ap(u8)`), `MAX_CPUS` = 16; MADT type 0 only; disabled entries take slots (filtered later, `smp.rs:161`) | Server APIC IDs > 255 and sparse; a 20-thread desktop already exceeds 16; > 255 CPUs need x2APIC plus interrupt remapping (DMAR flag bit 0) so I/O APIC and MSI can target them | B (servers) | 32-bit IDs (CPUID 0x1F/0x0B, MADT type 9); store enabled/online-capable entries only; runtime-sized per-CPU data; IR |
| HX-06 | `smp.rs:171-182,211-215` | 0-2 MiB identity-mapped with one 2 MiB WB page. (Trampoline base now comes from bootloader Usable regions, `boot.rs:78`) | A large page spanning ranges of different fixed-range MTRR memory types is undefined (SDM 3A, large page size considerations) | L | Map 0-1 MiB with 4 KiB pages; reserve the trampoline in the frame allocator explicitly; fail loudly if none |
| HX-07 | `mm/mod.rs:316-317,350-377,445-470` | Only the 8 largest Usable regions, clipped above the kernel; region index = NUMA node | UEFI maps have 50-150 descriptors; nodes come from ACPI SRAT | L | Dynamic region list with exclusions; nodes from SRAT proximity domains |
| HX-08 | `main.rs:56`, `mm/kstack.rs:40`, heap slot | All physical addresses below 64 TiB (direct map 0xFFFF_8000_0000_0000, heap 0xFFFF_C000_...) | MAXPHYADDR up to 52 bits; 64-bit BARs placed high; `map_mmio` could hit the heap or stack slots | L (servers) | Read CPUID 0x80000008; size the direct map and a separate MMIO window from it; refuse BARs outside |
| HX-09 | bootloader direct map; `msr.rs:57` (`phys_to_virt`) | Everything up to max(RAM, 4 GiB) mapped WB with 2 MiB pages, MMIO included | Works only because MTRRs make MMIO UC; a WC framebuffer alias of a WB mapping violates the SDM | L | Kernel-owned tables: RAM and ACPI regions in the direct map; MMIO through UC `ioremap`; framebuffer only WC |
| HX-10 | `pat.rs:60-68`; called on BSP (`bootstrap.rs:249`) and APs (`smp.rs:288`) | PAT rewritten in place with WRMSR only | SDM 3A requires identical PAT on all CPUs; Linux reprograms it inside the MTRR-update sequence (CR0.CD=1, WBINVD, write, flush TLB, CR0.CD=0) | L | Use that sequence on every CPU before any WC mapping |
| HX-11 | `drivers/pci.rs:592-618`; ECAM helpers `pci.rs:994-1040` unused; `acpi.rs:45` | Configuration mechanism #1 only; MCFG capped at 4 entries | 256-byte config space, segment 0 only; no extended capabilities (AER, ATS, resizable BAR); servers have several segments | D | ECAM per MCFG entry (UC-mapped, not through the WB direct map), 0xCF8 as segment-0 fallback; drop the 4-entry cap |
| HX-12 | `pci.rs:528-566` | BAR sizing with decode on, low dword only, `(!mask + 1) & ~0xF` inverts flags with size | Live devices decode the probe value; 64-bit BARs and prefetchable flags mis-sized | D | Disable MEM/IO decode while sizing, size both dwords, mask flags before inverting, 16-bit I/O mask |
| HX-13 | `pci.rs:728-732`; LINT0 masked | Interrupt Line register = PIC IRQ | With an I/O APIC routing comes from `_PRT` (AML) or MSI/MSI-X | D | MSI/MSI-X first; ISA IRQs via MADT ISOs; `_PRT` when AML exists (HX-28) |
| HX-14 | `acpi_pm.rs:160-163,388-397` | PIIX4 PM ports 0x600/0x604, SLP_TYP 5/6/7, 32-bit FADT fields | Values come from DSDT `\_S5`/`\_S3`; hardware-reduced ACPI (FADT flag bit 20) uses SLEEP_CONTROL_REG; X_ GAS fields may be MMIO; many laptops have no `\_S3` at all (FADT flag bit 21, low-power S0 idle) | D | No defaults; X_ fields; `\_S5` from AML; S3 needs FACS waking vector and resume |
| HX-15 | `services/shell/commands/system.rs:1851,1898`, `services/init_system.rs:456-462` | Power-off via QEMU `isa-debug-exit` (0xF4); reboot via 8042 0xFE | Real machines ignore 0xF4; legacy-free machines have no 8042 | D | S5 via PM1/SLEEP_CONTROL; reset like Linux: FADT RESET_REG, 8042, EFI `ResetSystem` (first on hardware-reduced), 0xCF9 (write `(v & ~code) \| 2`, then `\| code`; code 0x06 warm, 0x0E cold), triple fault |
| HX-16 | `arch/x86_64/rtc.rs:105` | Century at CMOS 0x32 | FADT `CENTURY` index (0 = none); `IAPC_BOOT_ARCH` bit 5 = no CMOS RTC | L | Use the FADT index; EFI `GetTime` without CMOS |
| HX-17 | `main.rs:141-148`, `arch/x86_64/boot.rs:28-35`, `vga.rs:147`, `drivers/console.rs:115`, `multiboot.rs:67-70`, `drivers/gpu.rs:371` | VGA text at 0xB8000; std-vga framebuffer at 0xFD000000 | UEFI without CSM often decodes no legacy VGA (FADT `IAPC_BOOT_ARCH` bit 2 "VGA not present") | L | Remove or gate on a BIOS path; GOP or serial only |
| HX-18 | `drivers/usb/xhci.rs:1392-1406` (HCCPARAMS1 read, xECP never walked) | No xHCI BIOS-to-OS handoff | Until claimed via USBLEGSUP (ext. cap ID 1), SMM may own the controller (8042 emulation) | D | Set OS Owned (bit 24), wait for BIOS Owned (bit 16) to clear (timeout), clear SMI enables in USBLEGCTLSTS (+4), reset; USB HID keyboard |
| HX-19 | `drivers/iommu.rs:460-477` (`init_iommu_unit` reads no registers) | IOMMU off; DMAR parsed only (N-77) | Firmware with pre-boot DMA protection may leave VT-d translation on; RMRRs must stay identity-mapped; DMAR flag bit 2 (platform opt-in) asks the OS to keep DMA protection for Thunderbolt/USB4 | L | Read GSTS.TES; adopt or disable without a gap; honour RMRRs; AMD IVRS likewise (section 3.8) |
| HX-20 | `drivers/e1000.rs:27-28` (8086:100E, 82540EM), GPU drivers | QEMU device models | Real NICs: I219 (e1000e), I225/I226 (igc), Realtek, Broadcom | D | Publish a supported-hardware list; e1000e/igc drivers later |
| HX-21 | `tsc.rs:93-104,171` | CPUID 16h base MHz when 15h ECX = 0; TSC-deadline trusted from CPUID.1 ECX[24] | Coarse; microcode errata for TSC-deadline (Linux `apic_check_deadline_errata`); TSC sync unchecked | L | Errata list; cross-check with PM timer/HPET; `IA32_TSC_ADJUST` and per-AP sync (N-179) |
| HX-22 | `arch/x86_64/idt.rs:36-120` (no NMI or #MC gate) | NMI and #MC never arrive | BMC/IPMI, PCIe SERR and watchdogs raise NMI; with CR4.MCE=0 a machine check shuts the CPU down (SDM 3B ch. 16): a silent reset | D | Handlers on own IST; enable MCA banks (section 3.3, N-175) |
| HX-23 | `arch/x86_64/kpti.rs:246-257`, `security/spectre.rs:393` | No speculative-execution mitigation needed | KPTI never switches CR3, `RETPOLINE_ENABLED = false`, no IBPB/VERW/RSB fill (N-145, N-146); exposure depends on the CPU model | L (security) | Per-model selection from `IA32_ARCH_CAPABILITIES` and CPUID (section 3.2) |
| HX-24 | (none) | No microcode loading | Fixes for HX-23, TSC-deadline errata and Zen hangs ship as microcode; firmware may carry an old revision | L | Early load per vendor (Intel MSR 0x79, revision 0x8B; AMD 0xC0010020), section 3.1 |
| HX-25 | `arch/x86_64/cpufreq.rs:199-250,463-471` | Legacy EIST via `IA32_PERF_CTL`; invented 8..30 ratio fallback | Once firmware sets `IA32_PM_ENABLE` (0x770, sticky until reset) HWP owns P-states and `PERF_CTL` writes are ignored; AMD uses CPPC | D | HWP, AMD CPPC, ACPI `_CPC`/`_PSS`; report "unknown", not invented ratios |
| HX-26 | `arch/x86_64/mod.rs:190-196`; no idle driver | Idle is HLT; LAPIC timer always runs | Without ARAT (CPUID.06H:EAX[2]) the LAPIC timer stops in deep C-states; package C-states and S0ix need MWAIT hints | L | Section 3.4; broadcast timer when no ARAT |
| HX-27 | `cpufreq.rs:268` (CPUID 06H only) | No thermal monitoring | Hardware self-throttles (PROCHOT, THERMTRIP) but trip and fan policy live in ACPI thermal zones | L | DTS MSRs, thermal LVT, ACPI `_TMP`/`_CRT` (section 3.4) |
| HX-28 | `acpi_pm.rs:128-134` (comments name `\_Sx`; no AML evaluated) | No AML interpreter | `_PRT`, `\_S5`/`\_S3`, `_OSC`, EC, battery, lid, power button, `_CST`/`_CPC` all live in AML | D | Adopt an interpreter (section 3.5) |
| HX-29 | `drivers/ahci.rs:72,1130-1170` | BOHC defined, never used; `CAP.S64A` recorded but ignored for DMA | Firmware may own the HBA until BIOS/OS handoff; without S64A buffers above 4 GiB are unreachable | D | BOHC handoff with timeout before HBA reset; DMA below 4 GiB when S64A is clear |
| HX-30 | `drivers/nvme.rs` | Polling; no shutdown notification; `CAP.MPSMIN` unchecked | Power-off without `CC.SHN` and `CSTS.SHST` is an unsafe shutdown (volatile write cache) | L | Normal shutdown on S5/reset; MSI-X; check MPSMIN |
| HX-31 | `drivers/gpu_i915.rs`, `gpu_amdgpu.rs` (device-ID tables) | virtio-gpu/ramfb | iGPU modesetting needs per-generation display code and firmware (Intel DMC/GuC, AMD DMCUB via PSP) | D | GOP framebuffer only (Linux `simpledrm` model) |
| HX-32 | `bootloader_api` 0.11.15 `BootInfo` | No EFI system table, SMBIOS or memory attributes | `ResetSystem`, `GetTime`, variables unavailable; memory kinds collapsed to Usable/Bootloader/Unknown | L | Loader change and runtime-call safeguards (section 3.9) |
| HX-33 | (none) | No SMBIOS | DMI strings drive quirk tables on real hardware (reboot, `_OSI`, backlight) | L | Parse SMBIOS 3 (`_SM3_`) from the UEFI configuration table |
| HX-34 | panic path (serial only) | Serial always visible | Laptops have no UART; early panics are lost | L | GOP panic output plus a persistent RAM log (section 3.10) |

Related audit items: N-41 (kernel built without SSE), N-145/N-146 (KPTI and Spectre claims),
N-172 (APs skip `init_syscall`/XSETBV), N-173 (IPI destinations and vectors), N-174 (x2APIC,
MADT-driven I/O APIC, generic vectors), N-175 (NMI/#MC/#XM handlers), N-176 (per-fault serial
output), N-179 (timer advance, TSC invariance), N-180 (AP stacks, failed start), N-77 (IOMMU
parse-only).

## 3. Target design

**Discovery order at boot.** UEFI memory map -> RSDP -> XSDT -> FADT (flags, PM timer, reset,
century, sleep), MADT (LAPIC/x2APIC, I/O APICs, ISOs, NMI), MCFG (ECAM), HPET, SPCR/DBG2
(console), SRAT/SLIT (NUMA), DMAR/IVRS (IOMMU), ECDT (early EC), then interrupt controllers, timers
and CPUs; AML namespace after the heap and timers. Nothing QEMU-specific is assumed; each device
either comes from a table or is probed safely.

**Console.** SPCR names the console (I/O port or MMIO, register width, baud). Without SPCR, probe
COM1-COM4 with the scratch register and LSR != 0xFF; otherwise GOP text only. Input from a probed
UART, an 8042 that passed self-test and that the FADT says exists, or a USB HID keyboard after
xHCI handoff.

**Time.** TSC as the clock source when invariant (CPUID 0x80000007 EDX[8]), frequency from CPUID
15h, then MSR 0xCE (Intel), then a measured reference (HPET, PM timer, PIT, each with timeouts);
LAPIC timer in TSC-deadline mode unless errata say otherwise, else periodic calibrated against the
TSC; a broadcast timer when ARAT is absent.

**Memory.** The kernel builds its own page tables: direct map of usable RAM and ACPI regions
(WB), MMIO only through `ioremap` (UC or WC), the framebuffer through one WC mapping, PAT
programmed per the SDM. MAXPHYADDR sizes the windows. The real-mode trampoline page is reserved
from the memory map before the frame allocator starts. The heap grows from frames.

**Interrupts.** x2APIC when available (required above 255 CPUs, with interrupt remapping),
xAPIC otherwise; I/O APIC(s) from the MADT; MSI/MSI-X for PCI devices; generic vector stubs for
64-239 (N-174); NMI and #MC on dedicated IST stacks.

### 3.1 CPU identification, errata and microcode

Identify vendor, family/model/stepping and microcode revision on every CPU before choosing any
feature path. Load microcode early (Linux layout:
`kernel/x86/microcode/{GenuineIntel,AuthenticAMD}.bin` in a cpio before the initrd), on the BSP
before feature detection and on each AP first, since microcode changes CPUID and
`ARCH_CAPABILITIES` bits. Late loading is unsafe with other threads running; skip it.
Keep a small, sourced errata table keyed by model and minimum microcode (TSC-deadline, ARAT,
MONITOR/MWAIT), modelled on Linux `apic_check_deadline_errata` and the "Old Microcode" page.

### 3.2 Speculative-execution mitigations by CPU (HX-23, N-145, N-146)

Decision input: CPUID.7.0:EDX (IBRS/IBPB bit 26, STIBP 27, `ARCH_CAPABILITIES` 29, SSBD 31,
MD_CLEAR 10), `IA32_ARCH_CAPABILITIES` (0x10A: RDCL_NO bit 0, IBRS_ALL bit 1, MDS_NO 5, TAA_NO 8,
BHI_NO 20, GDS_NO 26, RFDS_NO 27, ITS_NO 62), AMD CPUID 0x80000008 EBX and 0x80000021 EAX
(AutoIBRS bit 8). Control MSRs: `IA32_SPEC_CTRL` 0x48 (IBRS 0, STIBP 1, SSBD 2), `IA32_PRED_CMD`
0x49 (IBPB 0), `IA32_FLUSH_CMD` 0x10B (L1D flush); AMD AutoIBRS is EFER bit 21.

| CPU group | Needed at minimum |
|---|---|
| Intel without RDCL_NO (Skylake-era client, pre-Cascade Lake server) | KPTI with PCID; retpoline or IBRS; IBPB and RSB fill on switch; VERW on return to user (MD_CLEAR); L1TF-safe non-present PTEs |
| Intel with IBRS_ALL (eIBRS) and RDCL_NO | No KPTI; eIBRS on; BHI mitigation unless BHI_NO; IBPB on switch; check GDS/RFDS/ITS bits and microcode |
| AMD Zen 1-3 | No KPTI (not Meltdown-affected); retpoline/IBPB; Retbleed (Zen 1/2) and SRSO (Zen 1-4) return-thunk mitigations or IBPB-on-entry (verify per family) |
| AMD Zen 4 and later | AutoIBRS; SRSO as applicable; IBPB on switch |

Report the state per vulnerability and say "not mitigated" rather than claim coverage (N-145,
N-146). Per-issue logic: Linux `arch/x86/kernel/cpu/bugs.c`.

### 3.3 Machine check and NMI (HX-22)

Enable CR4.MCE after zeroing `IA32_MCi_STATUS` (0x401 + 4i) and setting `IA32_MCi_CTL`
(0x400 + 4i) for each bank in `IA32_MCG_CAP[7:0]`; AMD Scalable MCA uses its own bank MSRs (verify
range before use). The #MC handler runs on its own IST, reads `IA32_MCG_STATUS` (0x17A) and
banks, logs to the persistent buffer (3.10), and panics when RIPV is clear or the error is
uncorrected. NMI: own IST, no locks, per-CPU reason check (panic-stop IPI, perf
overflow, external). An NMI watchdog (PMI routed as NMI, Linux hardlockup model) comes later.

### 3.4 Power management: idle, P-states, thermal (HX-25 to HX-27)

Idle: HLT always works; MWAIT (CPUID.01H:ECX[3], sub-states in CPUID.05H) reaches deeper
C-states from a per-model table (Linux `intel_idle`) or ACPI `_CST`/`_LPI` (Linux `acpi_idle`,
used on AMD). Performance: Intel HWP (`IA32_HWP_CAPABILITIES` 0x771, `IA32_HWP_REQUEST` 0x774);
AMD CPPC (MSRs 0xC00102B1 enable, 0xC00102B3 request) or ACPI `_CPC`; legacy `_PSS`/`PERF_CTL`
last. Thermal: DTS readout in `IA32_THERM_STATUS` (temperature = TjMax from 0x1A2 - readout);
ACPI zones (`_TMP` in tenths of Kelvin, `_CRT`, `_HOT`, `_PSV`) once AML runs; `_CRT` -> S5.

### 3.5 ACPI namespace and AML (HX-28)

Options: ACPICA (C, BSD-3/GPLv2 or Intel licence; the reference, used by Linux and FreeBSD; large
OS-services layer), uACPI (C, MIT; events, sleep, `_PRT`, global lock, NT-compatible semantics;
used by Managarm, Ironclad and other hobby kernels), rust-osdev `acpi` crate (Rust, MIT/Apache-2.0;
AML interpreter behind `alloc`, real-DSDT maturity unmeasured, verify). Recommendation: uACPI or
ACPICA behind one FFI shim (module 90 rules), with the Rust crate evaluated against a corpus of
dumped DSDTs. `_OSI`: answer like ACPICA's default list ("Windows 2000" through "Windows 2022",
not "Linux"), because firmware is tested against Windows; SMBIOS quirks (HX-33) override. `_OSC`:
claim native PCIe hotplug/PME/AER/LTR only when implemented. Needed first: `_PRT` (HX-13),
`\_S5` (HX-14), EC `_REG`, GPE and fixed events (power button `PWRBTN_STS` or PNP0C0C).

### 3.6 Embedded Controller and laptop devices

EC (PNP0C09; ports from ECDT before the namespace loads, usually 0x66/0x62), ACPI chapter 12
protocol: status OBF bit 0, IBF bit 1, SCI_EVT bit 5; commands 0x80 read, 0x81 write, 0x84 query
(then `_Qxx`). Via EC and AML: lid (PNP0C0D `_LID`), battery (PNP0C0A `_BIX`/`_BST`), AC
(ACPI0003 `_PSR`), hotkeys, fans. FADT flag bit 21 without `\_S3` means S0ix only: suspend is
idle states plus device D3.

### 3.7 Storage and graphics (HX-29 to HX-31)

AHCI also needs COMRESET via `PxSCTL` and staggered spin-up when `CAP.SSS`; laptops shipped in
Intel RST "RAID" mode hide NVMe drives (verify), so document "AHCI mode in firmware". Display stays
the GOP framebuffer (fixed mode, no vblank or hotplug, WC); native iGPU drivers are out of scope
for v1 and the stub files should say so.

### 3.8 DMA protection, IOMMU and Thunderbolt (HX-19)

With DMAR flag bit 2 (Linux `DMAR_PLATFORM_OPT_IN`) or Windows-style Kernel DMA Protection, the
firmware expects the OS to keep the IOMMU translating for external PCIe (Thunderbolt/USB4).
Required: adopt the pre-boot translation state without a window, identity-map RMRRs only for
their devices, default-deny DMA domains for hot-plugged devices, interrupt remapping. Until then,
document that Thunderbolt and USB4 PCIe tunnels are unsupported and should be disabled in
firmware.

### 3.9 Boot loader, EFI runtime services, Secure Boot (HX-32, HX-33)

A future UEFI loader (or Limine) should hand over the raw UEFI memory map with attributes, RSDP,
SMBIOS 3, GOP mode, the EFI system table, the memory-attributes and RT-properties tables, command
line, initrd and TPM event log. Runtime services: `SetVirtualAddressMap` once, regions in a
separate address space (Linux `efi_mm`), one lock around all calls, FCW/MXCSR set and FPU state
saved around each call (UEFI 2.3.4, Linux `efi_fpu_begin`; required since the kernel has no SSE,
N-41), boot-services memory kept until after the switch, no variable writes (verify bricking
reports). Prefer ACPI reset and the RTC; EFI `ResetSystem` on hardware-reduced platforms.

Secure Boot: shim signed by Microsoft's third-party UEFI CA plus a VeridianOS key via MOK, or a
vendor key after shim-review, which expects signature enforcement for ring-0 code (verify current
rules). The Microsoft UEFI CA 2011 expired 2026-06-27; new shims carry a UEFI CA 2023 signature, so
firmware lacking that CA in `db` rejects them.

### 3.10 Crash capture (HX-34)

Kexec/kdump (reserved region, second kernel) is not a v1 item. Cheaper: a RAM log ring at a
fixed physical address (magic plus checksum) that survives a warm reset and is read back at next
boot (Linux pstore/ramoops model); ACPI ERST as an optional backend.

## 4. Bring-up checklist (priority order)

1. Console: UART probe, SPCR/DBG2, GOP fallback; no reads from absent ports (HX-03).
2. Timers: bounded PIT loops; CPUID 15h/16h, MSR 0xCE, HPET, PM timer, PIT; LAPIC against TSC
   (HX-01, HX-21).
3. ACPI before APIC; I/O APICs from MADT; x2APIC; NMI and #MC handlers (HX-04, HX-22, N-174).
4. Memory: full memory map, runtime heap, trampoline reservation, MAXPHYADDR, kernel-owned page
   tables with UC MMIO and one WC framebuffer, PAT sequence (HX-02, HX-06 to HX-10).
5. SMP: 32-bit APIC IDs, enabled entries, dynamic CPU count (HX-05, N-172, N-180).
6. PCI: ECAM on all segments, BAR sizing, MSI/MSI-X (HX-11 to HX-13).
7. Input and storage: xHCI handoff, USB HID, FADT-gated PS/2, AHCI handoff, NVMe shutdown (HX-18,
   HX-29, HX-30).
8. Power: reset and S5, then an AML interpreter (HX-14, HX-15, HX-28).
9. CPU hygiene: microcode, mitigations, idle and P-states (HX-23 to HX-26).
10. Then IOMMU, SRAT NUMA, NIC drivers, SMBIOS quirks, crash log (HX-19, HX-07, HX-20, HX-33,
    HX-34).

## 5. Validation without hardware (QEMU matrix)

| Configuration | Exercises |
|---|---|
| `-machine q35 -global ICH9-LPC.disable_s3=0`, OVMF | baseline UEFI path |
| `-no-hpet`, `-machine pit=off` (or `-global kvm-pit.lost_tick_policy` variants) | HX-01 calibration fallbacks |
| `-serial none` with `-device pci-serial` or no serial | HX-03 console selection |
| `-smp 64,maxcpus=288 -machine q35,kernel-irqchip=split -device intel-iommu,intremap=on` with `-cpu host,+x2apic` | HX-04, HX-05, x2APIC above 255 |
| `-m 1G`, `-m 64G`, `-cpu host,phys-bits=52` (KVM permitting) | HX-02, HX-07, HX-08 |
| `-device pxb-pcie` (extra PCI bus/segment), `-device nvme` with large BAR | HX-11, HX-12 |
| `-device qemu-xhci -device usb-kbd`, no PS/2 (`-machine q35,i8042=off`) | HX-18 |
| `-device e1000e`, `-device igb` | HX-20 |
| HMP `nmi`, HMP `mce <cpu> <bank> <status> <mcgstatus> <addr> <misc>` | HX-22 |
| `-cpu Skylake-Client-v1`, `-cpu EPYC-v1`; `-device ich9-ahci` with `ide-hd` | HX-23 detection, HX-29 |
| OVMF `OVMF_CODE.secboot.fd` with enrolled keys, `-machine q35,smm=on` | Secure Boot path (3.9) |

Real-hardware order: one Intel laptop or NUC (PIT-less, USB keyboard, no COM1, EC, S0ix), one AMD
desktop, then a server with IPMI SOL. Dump each machine's ACPI tables and SMBIOS under Linux first
(`acpidump`, `dmidecode`) as the HX-28/HX-33 corpus.

## 6. Sources

- Linux: [c8c4076 "x86/timer: Skip PIT initialization on modern chipsets"](https://github.com/torvalds/linux/commit/c8c4076723daca08bf35ccd68f22ea1c6219e207);
  [LKML: no PIT and no HPET on Intel N3350](https://lkml.kernel.org/lkml/alpine.DEB.2.21.1906280707020.32342@nanos.tec.linutronix.de/T/);
  [`reboot.c`](https://github.com/torvalds/linux/blob/master/arch/x86/kernel/reboot.c) (reset order, 0xCF9);
  [`msr-index.h`](https://github.com/torvalds/linux/blob/master/arch/x86/include/asm/msr-index.h) (all MSRs and bits cited);
  [`xhci-ext-caps.h`](https://github.com/torvalds/linux/blob/master/drivers/usb/host/xhci-ext-caps.h);
  [`dmar.h`](https://github.com/torvalds/linux/blob/master/include/linux/dmar.h),
  [`actbl.h`](https://github.com/torvalds/linux/blob/master/include/acpi/actbl.h),
  [`actbl1.h`](https://github.com/torvalds/linux/blob/master/include/acpi/actbl1.h) (DMAR, FADT, ECDT);
  [`asm/efi.h`](https://github.com/torvalds/linux/blob/master/arch/x86/include/asm/efi.h) (`efi_fpu_begin`);
  `tsc.c`, `apic.c`, `cpu/bugs.c`, `cpu/mce/`, `intel_idle.c`, `acpi/ec.c`;
  [hw-vuln docs](https://docs.kernel.org/admin-guide/hw-vuln/index.html);
  [microcode loader](https://docs.kernel.org/arch/x86/microcode.html).
- [Xen: ITSSPRC static PIT clock gating](https://patchew.org/Xen/20210127150615.641-1-andrew.cooper3@citrix.com/)
- Intel SDM Vol. 3A (MP init, APIC/x2APIC, MTRR/PAT, large pages), Vol. 3B (MCA, thermal, HWP);
  AMD APM Vol. 2 and per-family PPRs (verify per family); Intel VT-d specification.
- [ACPI 6.5](https://uefi.org/specs/ACPI/6.5/): FADT (5.2.9), MADT, GAS, ECDT, `\_Sx` (ch. 7),
  thermal (ch. 11), EC (ch. 12). [ACPICA `utosi.c`](https://github.com/acpica/acpica/blob/master/source/components/utilities/utosi.c);
  [uACPI](https://github.com/uACPI/uACPI); [rust-osdev `acpi`](https://github.com/rust-osdev/acpi).
- PCI Firmware 3.3 (MCFG); Microsoft SPCR/DBG2; xHCI 1.2 (sec. 7.1); AHCI 1.3.1; NVMe base;
  UEFI 2.10 (runtime services, 2.3.4 calling convention, GOP, memory attributes); SMBIOS 3.x
  (DMTF DSP0134).
- Secure Boot: [shim issue 679](https://github.com/rhboot/shim/issues/679);
  [Red Hat on the 2026 certificate expiry](https://www.redhat.com/en/blog/expiration-secure-boot-signing-certificates-2026).
