# ADR 0015: The AArch64 kernel runs in the upper half with the MMU on

- Status: accepted
- Date: 2026-10-16
- Tracking: v0.27.0 sprint E; N-28, HA-04, HA-16, HA-25

## Context

The AArch64 kernel has run since its first boot with the MMU and caches off, linked and executed
at its load address, 0x4020_0000. That has three costs:

- **Atomics.** Exclusive loads and stores (LDXR/STXR, the base of every lock) are not guaranteed
  on Device memory, which is what all memory is with the MMU off (HA-16). The tree carries
  workarounds for this: `fs::bare_lock` (a lock with no atomics), a bump global allocator, and
  secondary CPUs that take no lock and park in `wfe`.
- **User mode.** EL0 needs translation tables. The lower half of every address space belongs to
  the process, and a kernel at its physical address sits in that lower half.
- **Load address.** The kernel runs only where it is linked, while firmware may load an arm64
  Image at any 2 MiB-aligned address (HA-04).

RISC-V made the same move in E-RV3/E-RV4 (ADR 0013): linked high, paging on in boot.S, all of RAM
mapped at `PHYS_MEM_OFFSET`. Linux arm64 does the same in `__primary_switch`, with the kernel in
the TTBR1 half.

## Decision

1. **Link high, load low.** `link.ld` links the image at 0xFFFF_FFFF_8020_0000, as on x86_64 and
   RISC-V, and loads it at 0x4020_0000 (`AT()`). The ELF entry is the physical address of
   `_start`, since QEMU enters a bare ELF at `e_entry` with the MMU off. The Image header is
   unchanged; `text_offset` stays 2 MiB.
2. **One root, two halves.** boot.S builds a single level-0 table and loads it into TTBR1_EL1, so
   the kernel half (L0 entries 256-511) indexes exactly as on x86_64 and RISC-V. Every address
   space shares that half.
   - L0[256] maps all physical memory, 512 GiB in 1 GiB blocks, at 0xFFFF_8000_0000_0000
     (`PHYS_MEM_OFFSET`).
   - L0[511] maps the kernel image at its link address with 2 MiB blocks, at whatever 2 MiB-aligned
     physical address the loader chose. The early code computes that address with `adr`, so the
     image runs wherever it is loaded (HA-04, for the image itself).
   - TTBR0_EL1 holds a separate identity root (L0[0]: the same 512 GiB at their physical
     addresses). It is temporary, as in ADR 0013: the MMIO drivers still use physical addresses
     as pointers. Once they use the physical-memory map, the identity root is used only between
     enabling the MMU and the jump high, and TTBR0 is free for user address spaces.
3. **Memory types.** MAIR_EL1 attribute 0 is Normal write-back (0xFF) and attribute 1 is
   Device-nGnRE (0x04), as in `mm::pte::arm`. In the early 1 GiB blocks, the first GiB of physical
   space (QEMU `virt`'s MMIO) is Device and the rest Normal. That is right for QEMU `virt` only;
   the memory map from the device tree replaces it later (HA-17), and high PCIe windows are not
   mapped Device until then.
4. **TCR_EL1.** 48-bit virtual addresses in both halves (T0SZ = T1SZ = 16), 4 KiB granules,
   inner-shareable write-back walks, and IPS from ID_AA64MMFR0_EL1.PARange, capped at 48 bits
   (the page-table format holds 48-bit output addresses).
5. **The switch.** All boot.S code before the MMU is enabled is position-independent (`adr`,
   `adrp`; no `ldr =symbol`). After it builds the tables, boot.S invalidates their cache lines
   (they were written with the caches off, HA-25), writes MAIR, TCR and both TTBRs, invalidates
   the TLB, sets SCTLR_EL1.M, C and I, and branches to the absolute high address of the next
   instruction. It then reloads the stack pointer and VBAR_EL1 as high addresses before any Rust
   code runs.
6. **Secondary CPUs** start at the physical address of their entry (PSCI CPU_ON is given
   `kernel_phys(entry)` and the physical address of their boot block). They load the boot CPU's
   MAIR, TCR and TTBR values from a block boot.S fills, enable the MMU and jump high before reading
   the rest of their boot block. The boot CPU cleans that block to the point of coherency before
   CPU_ON, since the secondary reads it with its caches off.
7. **Linker symbols are virtual.** Code that needs the physical address of the image (the end of
   the kernel for the frame allocator) uses `aarch64::boot::kernel_phys`, which subtracts the
   offset boot.S measured. `get_kernel_page_table` returns the TTBR1 root.

## Consequences

- AArch64 runs with the MMU and caches on from the first instructions of `_start`. Exclusives work,
  so the N-28 workarounds (`bare_lock`, the bump allocator, the reduced secondary stage) can be
  removed one at a time, each with its boot test un-skipped.
- The kernel half of an address space has the same shape on all three architectures.
- The kernel image is mapped writable and executable as a whole; W^X needs 4 KiB mappings, later
  (as on RISC-V).
- Without ASIDs (HA-26), a TTBR0 switch must invalidate the TLB; that comes with EL0.
- Device and Normal memory are told apart only by the fixed QEMU `virt` rule until the device-tree
  memory map is used. On hardware the rule is wrong, and the first board port must replace it.
