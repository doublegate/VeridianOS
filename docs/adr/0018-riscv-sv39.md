# ADR 0018: RISC-V on Sv39 and Sv48, one address layout for both

- Status: accepted (stage A: the layout; stage B: Sv39 paging, in progress)
- Date: 2026-10-17
- Tracking: HR-08, HR-09 (`ref-docs/HARDWARE-RISCV64.md`); amends ADR 0002 (the RISC-V user range)
  and ADR 0013 (the RISC-V kernel layout)

## Context

The kernel turns on Sv48 (satp MODE 9) and stops on a hart without it (ADR 0013). Sv39 is the
baseline of the RVA20, RVA22 and RVA23 profiles, and most boards implement nothing else: StarFive
JH7110, SiFive FU740, Allwinner D1, T-Head TH1520, Sophgo SG2042, SpacemiT K1. QEMU `virt`
implements Sv57 and every mode below it.

Sv39 has 39-bit virtual addresses, sign-extended, giving two 256 GiB halves:

- user addresses are `[0, 0x40_0000_0000)`;
- kernel addresses are `[0xFFFF_FFC0_0000_0000, 2^64)`.

The current layout fits neither half:

- **Kernel half.** The physical map is at `0xFFFF_8000_0000_0000` and the kernel stacks at
  `0xFFFF_E000_0000_0000`. Only the image (`0xFFFF_FFFF_8020_0000`) is inside Sv39's kernel half.
- **User half.** The layout is shared with x86_64 and AArch64: the main stack at
  `0x7FFF_FFFF_0000`, thread stacks at `0x7FFE_0000_0000`, the mmap area from
  `0x4000_0000_0000`, the PIE base at `0x5555_5555_4000`, and `USER_SPACE_END` at
  `0x7FFF_FFFF_F000`.
- **Walker.** The page-table walker always has four levels.

Linux handles this with:

- `set_satp_mode()`, which writes satp with Sv57, then Sv48, then Sv39, and keeps the first one
  the hart reads back;
- a kernel layout per mode;
- since 6.6, a hint-less mmap that stays inside the Sv39 range even on a Sv48 hart, so programs
  that keep tags in the upper address bits run on every hart.

## Decision

1. **One kernel layout for RISC-V, inside Sv39's kernel half, used in every mode.**

   | Range | Size | Use | Sv39 root entry | Sv48 entry |
   |---|---|---|---|---|
   | `0xFFFF_FFC0_0000_0000` | 128 GiB | physical map (1 GiB pages) | 256..383 | L4[511], L3[256..383] |
   | `0xFFFF_FFE0_0000_0000` | 16 GiB | kernel stacks (`mm::kstack`) | 384..399 | L4[511], L3[384..399] |
   | `0xFFFF_FFFF_8000_0000` | 2 GiB | kernel image (link address + 2 MiB) | 510..511 | L4[511], L3[510..511] |

   Under Sv48 all of it hangs off one top-level entry, L4[511]. Its table is exactly the upper half
   of the Sv39 root, so stage B can use the same table as the Sv39 root.

   RAM above 128 GiB is not used. The device tree's memory is clamped to the physical map
   (`arch::riscv64::boot::PHYS_MAP_SIZE`; AArch64 gets the same clamp at its 512 GiB).

2. **Default user placement inside Sv39's user half in every mode; the user limit per mode.**
   The places the kernel chooses keep x86_64's order and stay below 256 GiB on every hart, as
   Linux keeps hint-less mappings in the Sv39 range:

   | Range | Use |
   |---|---|
   | from `0x10_0000_0000` (64 GiB), up to the PIE base | mmap area |
   | `0x2A_AAAA_A000` (Linux's `ELF_ET_DYN_BASE` for Sv39, page-aligned) | PIE base; brk follows the program |
   | below `0x3E_0000_0000` | thread stacks |
   | `0x3F_FFFF_0000` | main stack top |

   The user limit (`USER_SPACE_END`, what `is_user_range` checks) follows the paging mode, less the
   top page: `0x3F_FFFF_F000` under Sv39 and `0x7FFF_FFFF_F000` under Sv48. So on a Sv48 hart a
   program that asks for an address above 256 GiB (an mmap hint, MAP_FIXED) gets it, as on Linux
   (decided 2026-10-17, for full Linux compatibility). The limit is a value set once at boot
   from satp's mode (`mm::user_layout::user_space_end`); Sv39's until then.

   The values are defined once in `mm::user_layout` for each architecture; x86_64 and AArch64
   keep theirs.

3. **The mmap area has an upper bound:** the PIE base, on every architecture. The mmap cursor only
   moves forward (until the v0.29 range-indexed mappings), so a long-lived program could otherwise
   run into the program. On RISC-V that leaves 106 GiB for mmap; on x86_64 the bound is 21 TiB
   away.

4. **Stage B: Sv39 paging.**
   - boot.S probes Sv48 and then Sv39 by writing satp and reading it back (Linux's order; Sv57 is
     not used), and records the mode.
   - Under Sv39 the kernel root is the shared upper table itself, and the trampoline root adds the
     identity map's first 256 GiB.
   - The walker takes the number of levels from the recorded mode: under Sv39 the root is indexed
     by VA[38:30] and the walk starts one level down. The kernel/user split stays at root entry 256
     in both modes.
   - Each GiB of the kernel-stack region is its own Sv39 root entry, so all of them are created
     before the first address space is.

## Consequences

- The same kernel boots on Sv39-only boards and on Sv48 and Sv57 harts (stage B). The gate boots
  RISC-V with `-cpu rv64,sv48=off` as well as the default.
- On RISC-V, what the kernel places is in the low 256 GiB on every hart; on Sv48 hardware a
  program can still map above it by asking, as on Linux. Programs that need more than about
  100 GiB of hint-less mappings are out of reach, as they are on Sv39 Linux.
- RAM above 128 GiB is not used on RISC-V (Linux's Sv39 linear map is about 124 GiB). Sizing the
  physical map by mode for larger Sv48 machines is left for later (decided 2026-10-17).
- ADR 0002's single user range for all three architectures becomes one range per architecture, in
  one place.
- ADR 0013's layout (the physical map at L4[256]) is replaced by the table above. Its other
  decisions (the link address, the trampoline, no identity map after boot) stand.
