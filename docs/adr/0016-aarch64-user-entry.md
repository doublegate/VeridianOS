# ADR 0016: AArch64 user mode entry and exit

- Status: accepted
- Date: 2026-10-16
- Tracking: v0.27.0 sprint E (E-A6); N-28 follow-up, ARCH-MEM-03; follows ADR 0006 (dispatcher),
  ADR 0014 (the RISC-V counterpart), ADR 0015 (the AArch64 kernel address space)

## Context

With the MMU on, the kernel in the TTBR1 half and the dispatcher running on every CPU (ADR 0015,
E-A5), AArch64 user programs need a way in and out of EL0. An exception from EL0 is taken at EL1 on
SP_EL1, through the lower-EL entries of the vector table (VBAR_EL1 + 0x400). The exception syndrome
(ESR_EL1) gives its class. The kernel keeps its per-CPU pointer in TPIDR_EL1, which EL0 cannot
access; user code owns TPIDR_EL0 for TLS. A system call is `svc #0` with the number in x8 and
arguments in x0-x5, numbered by Linux's generic table with arm64's ABIs (common, 64, renameat,
rlimit, memfd_secret): RISC-V's numbers plus `renameat` (38), without the two RISC-V-only calls.

## Decision

1. **Exceptions from EL0 land on the task's kernel stack with no extra state.** SP_EL1 is the
   kernel's own stack pointer, and the return to EL0 (`veridian_aarch64_return_to_user`) pops the
   user frame and erets with SP_EL1 at the top of the running task's kernel stack. The next exception
   from EL0 therefore starts there. RISC-V needs `sscratch` and a per-CPU kernel-stack slot for
   this (ADR 0014 decisions 1 and 2); AArch64 needs neither.
2. **The frame.** The lower-EL synchronous and IRQ vectors save the whole register file, SP_EL0,
   ELR_EL1 and SPSR_EL1 as a `switch::UserFrame` (272 bytes) and record it in the per-CPU block
   (`syscall_frame`, offset 0x10; switched with the task, so `syscall_arg6` reads x5 from it).
   They then call `aarch64_user_exception`. A new user task starts at the same return path
   (`user_trampoline`).
3. **The handler**, by ESR_EL1.EC:
   - SVC64 is a system call (ELR is already past the `svc`; IRQs on, as on x86_64 and RISC-V;
     `syscall::aarch64_syscall` decodes with `Syscall::from_aarch64`).
   - Instruction and data aborts from EL0 go through `mm::page_fault`; SIGSEGV when unresolved.
   - Undefined instructions, trapped system-register accesses, illegal execution state and FP/SIMD
     traps are SIGILL; BRK is SIGTRAP; alignment faults and anything else are SIGBUS.
   - IRQs from EL0 are serviced as from EL1.

   Every return goes through `exit_to_user`: preemption, a pending fatal signal, and an SPSR that
   can only return to EL0t with D, A, I and F unmasked (`switch::user_spsr`). A return address
   outside user space kills the process.
4. **User state switched with the task**: TTBR0 (the address space; there are no ASIDs yet, so the
   TLB is invalidated, HA-26), TPIDR_EL0 (TLS) and the per-CPU frame pointer. FP/SIMD is not switched
   yet, so it traps at EL0 (CPACR_EL1.FPEN = 01), as RISC-V keeps FS off. The kernel is soft-float
   and uses none.
5. **User copies are fault-tolerant** (`arch/aarch64/usercopy.rs`, ARCH-MEM-03). EL1 synchronous
   exceptions now have a real handler: a data abort at one of the copy loop's two memory
   instructions is offered to demand paging and otherwise resumes at the fixup, which returns
   EFAULT. ARMv8.0 has no PAN, so the kernel reaches user pages directly (AP[1] grants EL0 access
   and EL1 keeps it); user pages are never executable at EL1 (PXN, `mm::pte::arm`).
6. **Code written for EL0 is made fetchable**: after loading a program, `sync_icache_all`
   invalidates the instruction caches to the inner-shareable domain. This relies on CTR_EL0.IDC
   (QEMU has it); a core without IDC also needs the written range cleaned to the point of
   unification first (HA-25).

## Consequences

- AArch64 runs programs at EL0 as dispatcher tasks on every CPU. Boot tests:
  - `aarch64_user_syscall_round_trip`, `aarch64_user_fault_kills_process`,
    `aarch64_user_mode_is_unprivileged` and `aarch64_user_fp_is_trapped`;
  - `aarch64_elf_program_runs` (through the loader, from the VFS, with arguments) and
    `aarch64_embedded_init_runs`;
  - `usercopy_fault_returns_efault`, now on AArch64 too.

  The embedded init and shell used x86_64's system call numbers (write 1, exit_group 231) and now
  use arm64's (64, 94).
- Not yet: user FP state switching, signal handlers (only fatal signals act), a fault-tolerant user
  compare-and-exchange (futexes), ASIDs, an AArch64 C library and rootfs. These follow in sprint
  E with RISC-V's.
- The user copy goes a byte at a time; faster copies wait for a measurement.
- A user stack pointer is never trusted: the kernel never runs on SP_EL0, and the entry does not read
  user memory.

## Amendment (2026-10-16): user FP state is switched

User FP state is no longer off in user mode: each user task carries an FP area, saved and restored
eagerly by `sched::dispatch::arch_switch` (as XSAVE on x86_64). A new program starts zeroed; fork
and clone copy the caller's live registers. AArch64 saves v0-v31, FPCR and FPSR, and CPACR_EL1.FPEN is 11 on every CPU.
The boot test `aarch64_user_fp_is_trapped` is replaced by `user_fp_state_survives_switches`, which
fails with the switch disabled.

## Amendment (2026-10-16): signal handlers

Signals reach handlers, with the decision loop x86_64 uses (`process::signals`: default actions,
stops, SA_RESTART, SA_ONSTACK, SA_NODEFER, SA_RESETHAND, the mask). A fault with a handler that does
not block it is queued for that handler (`signals::queue_fault`, shared with x86_64), as on Linux,
instead of ending the process. SA_RESTART puts the call's first argument back (the return value
overwrote it) and rewinds the 4-byte instruction; `rt_sigreturn` itself is never restarted. AArch64 uses Linux's arm64 `rt_sigframe` (`siginfo`, then a `ucontext` whose `sigcontext` holds
x0-x30, sp, pc and pstate, with an `fpsimd_context` record and a terminator in `__reserved`), followed
by a frame record; handlers return to the C library's SA_RESTORER, which musl always passes.
The boot test `user_signal_handler_round_trip` has the handler rewrite a saved register in its
`ucontext` and checks that `rt_sigreturn` restores it; it fails with the restore skipped.

## Amendment (2026-10-16): ASIDs (HA-26)

Decision 4's TLB invalidation on every address-space switch is gone. Each user page-table root
gets an ASID (`mm::asid`; 16-bit when ID_AA64MMFR0_EL1.ASIDBits says so, TCR_EL1.AS set at boot,
else 8-bit), and a task saves the whole TTBR0 value, so it comes back with its ASID; the switch
writes TTBR0 and leaves the TLB alone. Entries of other spaces stay, tagged with other ASIDs.

- Nothing needs a generation rollover: an ASID belongs to its root until the root is freed
  (`free_user_page_table_frames`, after no CPU runs in it), when it is released under the
  allocator lock with `tlbi aside1is` (every CPU) before it can be handed out again.
- Changes inside a live address space were already invalidated on every CPU (`tlbi ...is`), so
  entries an earlier CPU keeps stay correct.
- ASID 0 is never allocated: the empty root's (it maps nothing), and the fallback when all ASIDs
  are in use, for which the switch invalidates ASID 0's entries on the CPU (`tlbi aside1`), as
  before.
- User pages are already non-global (nG) and kernel pages global (`mm::pte::arm`).

The boot test `aarch64_asids_isolate_address_spaces` runs three processes that keep their pid at
the same stack address while switching; with one ASID shared by every root, two of them read
another's word.
