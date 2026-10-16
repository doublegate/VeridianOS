# ADR 0014: RISC-V user mode entry and exit

- Status: accepted
- Date: 2026-10-16
- Tracking: v0.27.0 sprint E (E-RV6); N-14, ARCH-MEM-03; follows ADR 0006 (dispatcher), ADR 0008
  (the x86_64 entry layer), ADR 0013 (the RISC-V kernel address space)

## Context

With paging on, the kernel in the upper half and the dispatcher running on RISC-V (ADR 0013, E-RV5),
user programs need a way in and out of U-mode. RISC-V gives a trap one vector (`stvec`) for both
privilege levels and one scratch register (`sscratch`). The kernel keeps its per-CPU pointer in `tp`,
which user code owns as its TLS pointer (N-14). The kernel may touch a user page only with
`sstatus.SUM` set. A system call is `ecall` with the number in `a7` and arguments in `a0`-`a5`,
numbered by Linux's generic table (ADR 0009 amendment).

## Decision

1. **`sscratch` says where the trap came from** (Linux's convention): 0 while the kernel runs; the
   per-CPU block while user code runs. The vector swaps `tp` and `sscratch`: a non-zero `tp` after
   the swap is a user trap, and `tp` is the kernel's per-CPU pointer from the first instruction on
   (N-14). A kernel trap swaps back and continues on the interrupted stack as before.
2. **User traps land on the task's kernel stack.** The per-CPU block has x86_64's layout: 0x00 the
   kernel stack of the running task (switched with the task, as `kernel_rsp` is on x86_64), 0x08 a
   scratch slot for the user `sp`, 0x10 the user frame of the trap in progress. The vector saves
   the whole register file plus `sepc` and `sstatus` as a `switch::UserFrame` (272 bytes), sets
   `sscratch` to 0 and calls `riscv_user_trap`; `veridian_riscv_return_to_user` restores the frame
   and sets `sscratch` to the per-CPU block again. A new user task starts there (`user_trampoline`).
3. **The handler**: `ecall` is a system call (`sepc` past the instruction, interrupts on, as x86_64
   system calls since N-139; `syscall::riscv64_syscall` decodes with `Syscall::from_riscv64` and
   shares the rest of the dispatch with x86_64; the sixth argument is read from the frame); timer
   and IPI interrupts; page faults through `mm::page_fault`, SIGSEGV when unresolved; illegal
   instructions SIGILL, breakpoints SIGTRAP, anything else SIGBUS. Every return goes through
   `exit_to_user`: preemption, a pending fatal signal, and an `sstatus` that can only return to
   U-mode with interrupts on, SUM clear and FP off. A return address outside user space kills the
   process.
4. **User copies are the only code that sets SUM** (`arch/riscv64/usercopy.rs`, the RISC-V
   counterpart of x86's SMAP/STAC), and they are fault-tolerant (ARCH-MEM-03): their two memory
   instructions are listed, a kernel page fault there is offered to demand paging and otherwise
   resumes at a fixup that returns EFAULT.

## Consequences

- RISC-V runs programs in U-mode as dispatcher tasks (boot tests `riscv_user_syscall_round_trip`,
  `riscv_user_fault_kills_process`, `riscv_user_mode_is_unprivileged`, `usercopy_fault_returns_efault`).
- ELF programs from the VFS run through the loader (boot tests `riscv_elf_program_runs`,
  `riscv_embedded_init_runs`). Not yet: a RISC-V C library and TLS; user FP state (FP is off in
  U-mode, so an FP instruction is SIGILL); signal handlers (only fatal signals act); a
  fault-tolerant user compare-and-exchange (futexes). These follow in sprint E. Secondary harts
  dispatch too (the loader fences the new code on every hart, SBI `remote_fence_i`).
- The user copy goes a byte at a time; faster copies wait for a measurement.
- A user `sp` is never trusted: the kernel never runs on it, and the trap does not read user memory.

## Amendment (2026-10-16): user FP state is switched

User FP state is no longer off in user mode: each user task carries an FP area, saved and restored
eagerly by `sched::dispatch::arch_switch` (as XSAVE on x86_64). A new program starts zeroed; fork
and clone copy the caller's live registers. RISC-V saves f0-f31 and fcsr with `sstatus.FS` on only for the save and
restore, turns FS off at every trap from U-mode (the user's FS is in the frame and comes back with
the `sret`), returns to U-mode with FS Clean, and learns F/D from the device tree (`riscv64::fpu`;
`misa` and `mstatus` are M-mode CSRs, HR-20/21).

## Amendment (2026-10-16): signal handlers

Signals reach handlers, with the decision loop x86_64 uses (`process::signals`: default actions,
stops, SA_RESTART, SA_ONSTACK, SA_NODEFER, SA_RESETHAND, the mask). A fault with a handler that does
not block it is queued for that handler (`signals::queue_fault`, shared with x86_64), as on Linux,
instead of ending the process. SA_RESTART puts the call's first argument back (the return value
overwrote it) and rewinds the 4-byte instruction; `rt_sigreturn` itself is never restarted. RISC-V uses Linux's RISC-V `rt_sigframe` (`siginfo`, then a `ucontext` whose `sigcontext` holds pc,
x1-x31 and the D state). Its uapi has no SA_RESTORER, so `struct sigaction` is 24 bytes (handler,
flags, mask); Linux returns from handlers through its vDSO. Here the kernel maps one read-execute
page holding `li a7, 139; ecall` into the process, from its mmap area, the first time a handler is
entered (`Process::sigtramp`, re-checked before each use so exec cannot leave it stale).
The boot test `user_signal_handler_round_trip` has the handler rewrite a saved register in its
`ucontext` and checks that `rt_sigreturn` restores it; it fails with the restore skipped.
