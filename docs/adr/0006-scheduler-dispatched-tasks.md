# ADR 0006: Scheduler-dispatched tasks on per-thread kernel stacks

- Status: accepted
- Date: 2026-10-07
- Tracking: v0.27.0 sprint D (critique C5, plan X0.1); N-41, N-46, N-50, N-51, N-52, N-54;
  SCHED-PERF-01, SMP-PERF-01 (stages S2/S3 of ADR 0004)

## Context

On x86_64 the scheduler is never initialised and never dispatches a user task. Every user
program, forked child and clone thread runs nested on the stack of whatever launched it (boot
code, the shell, or a parent's syscall), entered with `enter_usermode_returnable` and left
through one set of globals (`BOOT_RETURN_RSP/CR3`, `BOOT_CURRENT_PID/TID`). In particular:

- A syscall cannot block. Waits spin on `sti; hlt; cli`, and progress for other processes is
  manufactured inside the waiter: `wait` and `read` on a pipe run a forked child to completion
  nested under the parent's syscall, and a futex wait runs sibling threads one syscall at a time.
- Nothing preempts ring 3. `Scheduler::tick` acts only when `current` is set, which it never is.
- The per-thread kernel stacks (now guard-paged, N-26) are allocated and never used.
- Every process and thread also gets a `Task` that sits on the ready queue and is never
  dispatched; a `yield_cpu()` from a syscall would `load_context` one of them and abandon the
  caller's stack.
- `schedule()` switches while its caller holds the `SCHEDULER` lock, so a task that starts
  fresh would never release it.
- No FPU/vector state is switched (N-41), and signals are never delivered on return to user.

## Decision

Adopt the conventional model (Linux, the BSDs): **every thread is a scheduler task with its own
kernel stack, and all switching happens between kernel stacks.**

1. **Kernel stacks.** Each user thread runs its syscalls and interrupts on its own guarded kernel
   stack (`mm::kstack`). On every switch the next task's stack top is written to the per-CPU
   `kernel_rsp` (syscall entry) and to TSS.RSP0 (interrupts and exceptions from ring 3).
2. **Switch primitive.** `switch_stacks(&mut prev.saved_sp, next.saved_sp)` pushes the
   callee-saved registers, swaps the stack pointer and pops them. Everything else a task needs
   (user registers, syscall frame) is already on its own kernel stack.
3. **New tasks** get a kernel stack seeded with a switch frame that "returns" into a start
   trampoline. Kernel threads then call their entry function; user threads return to ring 3
   through an `iretq` frame built on their kernel stack (fork: the parent's syscall frame with
   `rax = 0`; clone: plus the new stack and TLS; exec/launch: entry point and stack).
4. **No lock across a switch.** `schedule()` takes the scheduler lock only to requeue the
   previous task and pick the next one, and drops it before switching, with interrupts off.
   A per-task `on_cpu` flag, cleared by the next task after the switch completes, keeps another
   CPU from running a task whose registers are still being saved (Linux `p->on_cpu`).
5. **The boot flow becomes a task.** The code running at the end of boot (the kernel shell)
   becomes CPU 0's first task, and every CPU gets an idle task. The shell launches a program
   by creating its process and then waiting for it, like any parent.
6. **Blocking.** A syscall blocks by marking its task blocked on a wait queue and calling
   `schedule()`; the waker makes it ready. Sleeps are woken by the tick. The nested boot
   dispatch (`boot_run_forked_child`, `boot_futex_spin`, `BOOT_CLONE_YIELD_PENDING`, the
   `BOOT_RETURN_*` globals) is removed once every wait uses this.
7. **Preemption.** The kernel stays non-preemptive: syscalls run with interrupts masked except
   while they block. The timer marks the running task as needing a reschedule; ring 3 is
   preempted on the way back to user mode, from the syscall exit and from interrupts taken in
   ring 3. Device interrupts therefore move off the shared IST stack onto the current kernel
   stack (IST stays for #DF, NMI and #MC).
8. **Exit and reaping.** An exiting thread releases what it can, becomes a zombie and switches
   away for good. Its kernel stack and task are freed by whoever reaps it (`wait`, or the
   kernel reaper for threads), after `on_cpu` shows it has left the CPU.
9. **FPU/vector state (N-41).** The kernel no longer uses vector registers (soft-float build),
   so user extended state is switched eagerly with XSAVE/XRSTOR into a per-thread area sized
   from CPUID leaf 0Dh.

**Staging** (each stage boots on every architecture and passes the runtime suite):

| Stage | Content |
|---|---|
| D1 | Switch primitive, task kernel stacks, scheduler started with the boot task and idle; kernel threads; boot test |
| D2 | User tasks dispatched by the scheduler: launch from boot/shell/KDE, syscalls on the thread's kernel stack, exit + reap, fork + wait, blocking pipes |
| D3 | Timer preemption of ring 3; device IRQs off IST; sleep queue; futex wait queues; clone threads; poll/epoll/timerfd blocking; XSAVE |
| D4 | Nested boot dispatch removed; N-46, N-50, N-54 |
| D5 | SMP S2/S3: per-CPU current task and run queues, idle and work stealing on every CPU, `smp` on by default; N-51, N-52 |

User mode exists only on x86_64 until sprint E, so D1-D4 implement the user side for x86_64;
the switch primitive and kernel threads are added for RISC-V with D5 and for AArch64 with N-28.

## Consequences

- A blocked syscall no longer stalls the machine, a forked child runs concurrently with its
  parent, and threads run concurrently.
- Kernel code that assumed it runs alone (one user process at a time) becomes reachable from
  several tasks; the audit items listed above are the known cases.
- The kernel shell is no longer special: it is a task that waits for its children.
- Signal delivery on return to user mode gets a natural hook (the return path), used by the
  v0.28 signal tier.
