# ADR 0012: Tasks run on every CPU

- Status: accepted (amends rule 7 of ADR 0006: system calls run with interrupts enabled)
- Date: 2026-10-15
- Tracking: v0.27.0 sprint D5 (D5c-D5e); N-139, SMP-PERF-01, SCHED-PERF-01

## Context

ADR 0004 brings the other CPUs up, ADR 0006 runs every thread as a task on its own kernel stack,
and ADR 0007 defines the per-CPU run queues with their placement and balancing rules as pure
functions. Until D5 the secondaries only idled: one CPU ran every task. Running tasks everywhere
raises questions the earlier ADRs leave open:

- A CPU that runs a long system call with interrupts masked cannot take a reschedule IPI or a TLB
  shootdown IPI, so another CPU waiting for it stalls (N-139).
- A task can be woken, moved or re-pinned on one CPU while another CPU is still saving its
  registers.
- Some per-CPU registers carry task state: while ring 0 runs, the user GS base sits in
  IA32_KERNEL_GS_BASE (the entry's `swapgs`), and a task that moves keeps the old CPU's value.

## Decision

1. **Per-CPU dispatch (D5c).** Every dispatching CPU has its own run queue, current task and idle
   task (`CPUS[cpu]`, one lock each). A secondary's boot flow becomes its idle task. A task's
   `cpu` names the run queue that holds it, or the CPU that last ran it. Each CPU ticks its own
   queue. Only the timekeeper advances the clock and the load average and wakes timed sleepers.
2. **Interrupts in system calls (D5d; amends ADR 0006 rule 7).** The entry enables interrupts once
   it is on the task's kernel stack. Code shared with interrupt handlers takes its locks with
   interrupts off. The kernel stays non-preemptive: a switch still happens only at the preemption
   point on the way back to user mode, or where a task blocks.
3. **Placement (D5e).** A task that is on no run queue and on no CPU is placed by
   `balance::select_cpu`, over the dispatching CPUs its affinity allows. That covers a new task,
   a woken one, and one leaving a CPU it may no longer use. The decision reads per-CPU numbers
   that each CPU publishes under its own lock (`STATS`), so it takes no other CPU's lock.
4. **Who may move a task.** At most one party moves a given task at a time:
   - A waker moves it only after its compare-and-swap from `Blocked` to `Ready` succeeded. If the
     task is still queued (it had not blocked yet), it stays. If it is still being switched away
     from (`on_cpu`), the waker waits for that to finish, as Linux's `try_to_wake_up` does.
   - The balancer moves only queued tasks that are neither running nor `on_cpu`, holding both
     CPUs' locks taken in CPU order.
   - A running task whose CPU is no longer allowed is taken off its queue by `schedule` and placed
     by `finish_switch` once it is off the CPU. A thread that changes its own mask has moved before
     its `sched_setaffinity` returns, at the preemption point on the way out.
5. **Interrupt context only tries locks.** The timer's wake of sleepers and its periodic balance
   use `try_lock` on every lock they take. Anything they cannot lock now is done on a later tick.
   They do allocate: a run queue insert is a map insert. The kernel heap therefore takes its lock
   with interrupts masked (`IrqSafeHeap`), as Linux's allocators do. With interrupts on in system
   calls, an interrupt that allocated while its CPU held the plain spinlock deadlocked every CPU.
6. **Interrupts stay on wherever locks are taken.** A CPU spinning with interrupts off cannot
   acknowledge a TLB shootdown, so a lock that is ever spun on with interrupts off must never be
   held by a CPU that issues one. Every such lock (dispatch, heap, wait-queue lists, keyboard) is
   held only briefly and never across a shootdown. Everything else runs with interrupts on when
   the interrupted code had them on: system calls, page faults and the return to user mode. Stacks
   and page tables are freed with no process lock held (N-139).
7. **Nothing sleeps in the switch path.** `finish_switch` runs on whichever task comes next, often
   an idle task. It only queues an exited task; a `reaper` kernel thread frees it, along with its
   process's page tables when it was the last task. An idle task never sleeps: a wait on it polls.
8. **Bound tasks.** The boot flow stays on CPU 0. It is the timekeeper's first task and owns the
   console. `spawn_kernel_on` binds a kernel thread to its CPU (Linux `kthread_bind`).
9. **Task state that lives in CPU registers moves with the task.** This covers the page table
   root, FS base, user GS base (`IA32_KERNEL_GS_BASE`), entry stack, syscall scratch and XSAVE
   area. `arch_prctl(ARCH_SET_GS/ARCH_GET_GS)` sets and reads it there.
10. **Counts.** `online_cpus()` is the number of dispatching CPUs. sched_getaffinity, the processor
   count and deadline admission (95% of every CPU, ADR 0007) use it.

## Consequences

- Runnable work spreads over every CPU. A thread can be pinned with sched_setaffinity, and getcpu
  reports where it runs.
- Deadline tasks are admitted against the total bandwidth of all CPUs. They move by deadline
  rather than load (global EDF, approximately): a waking one goes where it would run soonest
  (`balance::select_cpu_dl`, Linux `find_later_rq`), and on every tick and when it goes idle a CPU
  takes a waiting one whose deadline is earlier than everything it runs (`pull_dl`, Linux
  `pull_dl_task`), keeping its reservation. There is no separate push: a task that cannot run
  where it is queued waits at most one tick for another CPU to pull it. Throttled tasks do not
  move. Admission counts every online CPU, so a deadline thread must allow all of them, as on
  Linux: sched_setattr refuses one with a narrower mask (EPERM) and sched_setaffinity refuses to
  narrow a deadline thread's mask (EBUSY).
- Waking a task that is mid-switch on another CPU spins for the rest of that switch. It is short,
  with interrupts off on the other side.
- Rule 6 has to hold for every new lock that interrupt handlers take. A CPU that stays in a
  shootdown wait for over 100 ms reports it on the console.
- User address-space flushes go only to the CPUs on that page-table root (N-262, `mm::tlb`). Every
  CR3 load must record the root first (`tlb::note_root`); a new path that loads CR3 without
  recording it would make a CPU miss flushes.
- Kernel code that assumed one CPU becomes reachable concurrently. The runtime suite runs at
  `-smp 4` in the gate, and N-51, N-52 and N-59 track what is known.
