# ADR 0002: User address-space layout

- Status: accepted
- Date: 2026-10-06

## Context

The user/kernel boundary was defined in four places with four different values:

| Value | Where |
|---|---|
| `0x0000_7FFF_FFFF_0000` | `process::memory::layout` |
| `0x0000_7FFF_FFFF_FFFF` | the syscall validators (used as both an inclusive and an exclusive end) |
| `0x0000_8000_0000_0000` | `clone` |
| `0x7FFF_FFFF_F000` | the ELF loader's stack top |

Three problems came with this:

- **`mmap(MAP_FIXED)` had no upper bound,** so a fixed mapping could be requested in the kernel half.
- **On RISC-V, the kernel built 4-level page tables but activated them with `satp` MODE 8,** which is Sv39 (3-level). Under Sv39 user space ends at `0x40_0000_0000`, below the user stacks this kernel places.
- **No `sfence.vma` followed the `satp` write.**

Each architecture's documented convention:

| Arch | User range | Notes |
|---|---|---|
| x86_64, 4-level paging | `0` - `0x7FFF_FFFF_EFFF` | `0x7FFF_FFFF_F000`-`0x7FFF_FFFF_FFFF` is a 4 KiB guard hole. Linux `TASK_SIZE_MAX = (1 << 47) - PAGE_SIZE`. |
| AArch64, 48-bit VA | `0` - `0x0000_FFFF_FFFF_FFFF` (TTBR0) | Bit 63 selects TTBR0 or TTBR1. The kernel lives in the TTBR1 half. |
| RISC-V Sv39 | `0` - `0x3F_FFFF_FFFF` | `satp` MODE 8 |
| RISC-V Sv48 | `0` - `0x7FFF_FFFF_FFFF` | `satp` MODE 9 (Sv57 is 10) |

**Why x86 has the guard page.** On Intel, a SYSCALL instruction in the highest canonical page enters
the kernel with a non-canonical return address, and SYSRET then faults in ring 0 on the user's stack.
On AMD Ryzen, executing from that page speculates past the end of canonical space. This kernel returns
to user mode with SYSRETQ.

All three ABIs (SysV AMD64, AAPCS64, RISC-V psABI) require a 16-byte-aligned stack pointer. The
initial user stack built in `process::creation` already meets this: `sp & !0xF`, with `argc` at `sp`.

## Decision

- **One definition.** `mm::user_layout` defines user space as `[0x1000, 0x7FFF_FFFF_F000)` on every
  architecture, with `USER_SPACE_END` as an exclusive end and `is_user_range(start, len)` checking
  ranges with overflow rejected.
  - On x86_64 this is exactly Linux's `TASK_SIZE_MAX`.
  - On AArch64 it is a subset of TTBR0's range.
  - On RISC-V it is the Sv48 range minus the same top page, which keeps one layout for all three.
- **Every check uses it.** The syscall pointer validators, the user accessors, `clone`'s stack check,
  `is_user_addr_valid`, the loader's stack top and the process layout all reference it.
  - `mmap(MAP_FIXED)` now requires the whole page-rounded range to satisfy `is_user_range`.
  - Code that only classifies which canonical half an address is in (fault handlers, fork) keeps
    `0x0000_8000_0000_0000`, which is the correct value for that question.
- **RISC-V `satp` uses MODE 9 (Sv48), followed by `sfence.vma`.** The device tree's `mmu-type` is read
  at boot. Activating the 4-level tables on a hart without Sv48 is refused with a clear panic instead
  of mistranslating.

## Consequences

- The top x86 page can no longer be mapped. This is runtime-tested by `map_fixed_user_limits`: the
  highest legitimate page maps and is writable, while the reserved page, a kernel-half address and a
  range running past the end are all rejected.
- RISC-V user mode, when it arrives, starts from a correct translation mode.
- **Not done, planned with the process-model work (C5): kernel stacks have no guard pages.** They come
  from the direct map, so an overflow silently corrupts the neighbouring frame. Linux maps them in a
  dedicated region with an unmapped guard page (`CONFIG_VMAP_STACK`). Doing that here needs the
  kernel-half page tables to be shared with every address space. Kernel L4 entries are copied when a
  process is created, so a new top-level entry added later would not reach existing processes.

## References

- [Linux Documentation/arch/x86/x86_64/mm.rst](https://github.com/torvalds/linux/blob/master/Documentation/arch/x86/x86_64/mm.rst)
- [x86_64: Add a comment explaining the TASK_SIZE_MAX guard page](https://lkml.rescloud.iu.edu/1411.0/02498.html)
- [Memory Layout on AArch64 Linux](https://www.kernel.org/doc/html/v5.8/arm64/memory.html)
- [Virtual Memory Layout on RISC-V Linux](https://www.kernel.org/doc/html/v6.6/riscv/vm-layout.html)
- [RISC-V Supervisor-Level ISA: satp MODE encoding](https://docs.riscv.org/reference/isa/v20260120/priv/supervisor.html)
- [System V AMD64 ABI](https://refspecs.linuxbase.org/elf/x86_64-abi-0.98.pdf)
- [Procedure Call Standard for the Arm 64-bit Architecture](https://student.cs.uwaterloo.ca/~cs452/docs/rpi4b/aapcs64.pdf)
