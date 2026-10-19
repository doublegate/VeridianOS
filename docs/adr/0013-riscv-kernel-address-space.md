# ADR 0013: The RISC-V kernel runs in the upper half

- Status: accepted; the layout (physical map at L4[256]) is replaced by ADR 0018, which keeps
  everything inside Sv39's kernel half and adds Sv39 paging for harts without Sv48
- Date: 2026-10-16
- Tracking: v0.27.0 sprint E (E-RV3); N-14, HR-02, HR-10

## Context

Until sprint E the RISC-V kernel ran with paging off (`satp` Bare), linked and executed at its
load address, 0x8020_0000. User mode needs paging, and the lower half of every address space
belongs to the process (as on x86_64, where the kernel half is L4 entries 256-511 and is shared
by every address space). A kernel at its physical address sits in that lower half: RAM on QEMU
`virt` starts at 2 GiB, so the kernel and every physical address it touches would collide with
user mappings. The x86 format the page tables used (HR-10) was also invalid on RISC-V; `mm::pte`
replaced it before this step.

Linux on RISC-V links the kernel high and turns paging on in its first instructions
(`setup_vm`), mapping all of RAM at a fixed offset.

## Decision

1. **Link high, load low.** `link.ld` links the image at 0xFFFF_FFFF_8020_0000 (the same place
   as the x86_64 kernel) and loads it at 0x8020_0000 (`AT()`); `KERNEL_OFFSET` is the
   difference. OpenSBI and QEMU load each segment at its physical address and enter the lowest
   one, so `_start` stays first and nothing about loading changes.
2. **Paging in boot.S, before Rust.** The early code is position-independent (`la` is
   PC-relative under the medany code model), so it runs at the physical address. It zeroes
   `.bss`, builds three tables there and turns paging on:
   - L4[256]: all physical memory (512 GiB in 1 GiB pages) at 0xFFFF_8000_0000_0000, the
     physical-memory map that `mm::phys_to_virt_addr` uses (`PHYS_MEM_OFFSET`);
   - L4[511]: the kernel image at its link address (L3 510-511, physical 2-4 GiB);
   - L4[0]: the same 512 GiB at their physical addresses (identity). This is temporary: the
     MMIO drivers still use physical addresses as pointers, and the map goes once they use the
     physical-memory map (E-RV4), which frees the lower half for user space.

   It then writes `satp` (Sv48), checks that the hart accepted it (satp is WARL: without Sv48 it
   stays 0, and the kernel stops with a message instead of faulting), and jumps to the high
   address of the next instruction. Leaves are global, readable, writable, executable, with A and
   D set.
3. **Secondary harts** start at the physical address of their entry (SBI `hart_start` is given
   `kernel_phys(entry)`), load the boot hart's `satp` (`veridian_kernel_satp`) and jump high
   before they read their boot block, a kernel virtual address.
4. **Linker symbols are virtual.** Code that needs a physical address of the image (the end of
   the kernel for the frame allocator) uses `riscv64::boot::kernel_phys`.

## Consequences

- RISC-V runs with paging on from the first instructions of `_start`, and the kernel half of an
  address space (L4 256-511) is the same shape as on x86_64, ready to be shared with user
  address spaces.
- Only Sv48 is used, with 1 GiB pages. QEMU `virt` offers Sv57; most boards are Sv39-only
  (HR-09), which needs a three-level walker and a different early map.
- The kernel image mapping is writable and executable as a whole (one 1 GiB page); splitting
  text, rodata and data permissions (W^X) needs 4 KiB or 2 MiB mappings, later. (Done in v0.27:
  `arch::riscv64::kmap` remaps the image in 4 KiB pages once the frame allocator is up, and the
  physical map is not executable.)
- Memory attributes come from the platform's PMAs, so mapping holes and MMIO cacheable in the
  page tables is harmless on RISC-V without Svpbmt; with Svpbmt (HR-11) the physical map will
  need the right types.
- The temporary identity map overlapped the user half until E-RV4 confined it to the trampoline
  (amendment below).

## Amendment (E-RV4, 2026-10-16): no identity map after the trampoline

The identity map of decision 2 is now confined to a trampoline root, used only between `csrw satp`
at the physical address and the jump to the upper half. boot.S builds two roots from the same
tables: the trampoline root (L4[0], L4[256], L4[511]) and the kernel root (L4[256] and L4[511]
only). The boot hart and each secondary hart load the trampoline root, jump high, then load the
kernel root (`veridian_kernel_satp`). The drivers that used physical addresses as pointers (PLIC,
virtio-mmio slots, virtio queue rings, the early UART writes, ramfb and fw_cfg) use the
physical-memory map, and the secure-boot hash reads the image at its link address. New address
spaces copy the kernel root's upper half. Boot tests: `kernel_root_lower_half_empty`,
`user_half_starts_empty` (now on RISC-V too, and checking that the upper half is shared).
