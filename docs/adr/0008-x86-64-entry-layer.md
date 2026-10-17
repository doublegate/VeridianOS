# ADR 0008: x86_64 kernel entry layer

- Status: accepted
- Date: 2026-10-07
- Tracking: v0.27.0 sprint D (first step); N-166, N-167 (layout), N-168 (signals), N-169, N-170
  (x86_64), N-171, N-175; HX-22. Prerequisite for ADR 0006 (scheduler-dispatched tasks).

## Context

Exceptions and interrupts entered through `extern "x86-interrupt"` handlers, one per vector, and
the system call through a separate assembly routine with its own register layout:

- **GS base.** It was handled two ways. `syscall_entry` swapped it, the other handlers did not,
  and `this_arch_cpu_ptr` guessed which MSR held the per-CPU block. Code that entered user mode
  from inside a syscall had to issue compensating `swapgs` instructions in matched pairs. Once
  tasks switch, a task could return to ring 3 with the kernel GS base (N-166).
- **Stacks.** Almost every vector, the page fault and all IRQs included, ran on one of four
  shared 20 KiB IST stacks without guard pages. A nested fault overwrote the live frame, and
  code reached from a handler could not block (N-169).
- **Return to user.** `sysretq` ran with whatever RCX held (N-171). Nothing sanitised RFLAGS
  restored from user memory, so sigreturn could raise IOPL (N-170).
- **Missing handlers.** NMI, #MC, #DB, #NM, #MF and #XM had no handler, and with CR4.MCE clear
  a machine check shut the CPU down (N-175, HX-22).

## Decision

1. **One frame.** Every entry builds a `TrapFrame` (`arch/x86_64/trap.rs`): the general
   registers, the vector, the error code, then the hardware `iretq` frame. `syscall_entry` builds
   the same layout, with `vector` set to `SYSCALL_VECTOR` and the syscall number as the error
   code, and stores the handler's result into `frame.rax`. Code that redirects a return
   (exec, signal delivery, sigreturn, the dispatcher in ADR 0006) edits `rip`, `rsp` and
   `rflags` in that frame.
2. **Stubs.** 256 assembly stubs at a fixed 16-byte stride push the missing error code and the
   vector, then jump to one `trap_common`, which calls a single Rust dispatcher. At boot the
   table is checked byte by byte, so a stub that changes size cannot shift every vector. The
   IDT is built by hand: interrupt gates, DPL 3 only for `int3` and `into`.
3. **GS convention (as in Linux).** Whenever ring 0 runs, GS_BASE holds this CPU's per-CPU
   block. A ring-3 entry swaps it in and a ring-3 exit swaps it out, exactly once each:
   - `trap_common` decides from the saved CS and issues `lfence` on both paths (CVE-2019-1125);
   - `syscall_entry` swaps always;
   - every `iretq` to user swaps immediately before it, with interrupts disabled.

   No code loads the GS selector, because that clears the base on Intel.
4. **Stacks.** IST is used only for #DF, NMI and #MC; everything else runs on the current kernel
   stack, or on TSS.RSP0 when it comes from ring 3. TSS.RSP0 and the syscall stack (`kernel_rsp`)
   are one value per CPU, set by `percpu::set_entry_stack`. Each CPU's block records where its
   TSS.RSP0 lives (`rsp0_slot`, at `gs:[0x40]`), so assembly sets both. The NMI, #MC and #DF
   handlers use no GS-relative state, so they are correct even in the few instructions where GS
   holds the user value.
5. **Return checks.**
   - Every return to ring 3 goes through `sanitize_user_frame`: user selectors; RFLAGS reduced
     to the bits user code can set, with IF forced on; RIP and RSP below the user limit, or 0
     so the process faults in ring 3.
   - `sysretq` is used only when RCX and R11 still equal RIP and RFLAGS and RIP is below the
     last user page; otherwise the syscall returns with `iretq`.
   - The nested-child launcher sanitises the same fields in assembly.
6. **Exceptions from user mode** kill the process with the signal Linux sends: SIGFPE for #DE,
   #MF and #XM; SIGTRAP for #DB and #BP; SIGILL for #UD and #NM; SIGBUS for #NP, #SS and #AC;
   SIGSEGV otherwise. From kernel mode they are fatal, except a breakpoint or debug trap, which
   resumes. Fatal paths use only port I/O, because the interrupted code may hold any lock.
7. **Machine checks.** CR4.MCE is set when the CPU reports MCE. The #MC handler logs the valid
   MCA banks and stops.

## Consequences

- The dispatcher (ADR 0006) can switch tasks: GS state no longer depends on how a context was
  entered, a vector from ring 3 lands on the current thread's kernel stack, and fork, clone and
  signal delivery read and write one frame layout.
- Handlers reached from a vector can now nest and, after D3, block, because they run on a real
  kernel stack. The price is that a kernel stack overflow inside a fault handler becomes a #DF.
  That is caught on its IST stack and reported as a guard-page hit.
- The `abi_x86_interrupt` feature is no longer used by x86_64 handlers.
- Still open: a user fault without a launch context is fatal until D2 gives every user task a
  normal exit path (N-168), the frame pointer is still per CPU until frames live on per-thread
  kernel stacks (N-167), and stopping other CPUs on a fatal fault comes with SMP stage S2.
