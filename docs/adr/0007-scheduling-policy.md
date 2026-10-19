# ADR 0007: Scheduling policy (deadline, real-time, EEVDF fair, idle)

- Status: accepted
- Date: 2026-10-07
- Tracking: v0.27.0 sprint D; SCHED-PERF-01, SMP-PERF-01, SCHED-PERF-02, SCHED-PERF-03,
  SCHED-INC-01 (hooks), SCHED-INC-02. Mechanism (dispatch, kernel stacks): ADR 0006.

## Context

The old scheduler had one global ready queue, a 2 KB priority array per CPU copied on every
removal, CFS/RR/priority/hybrid pick functions that rotated whole queues, an unwired EDF module
and an unreferenced per-CPU queue. None of it ever dispatched a user task (ADR 0006). The
dispatch rework is the moment to replace the policy with one that is known to work.

Linux replaced CFS with EEVDF (6.6, 2023): fairness as in CFS, plus latency control without
heuristics. SCHED_DEADLINE (EDF + Constant Bandwidth Server) and SCHED_FIFO/RR sit above it.
These are the classes POSIX and Linux programs use, so matching them also serves the Linux ABI
work (sched_setattr and friends, v0.28+).

## Decision

**Classes, in strict priority order, per CPU run queue:**

1. **Deadline** (`SCHED_DEADLINE`): EDF over non-throttled tasks. CBS rules: on wakeup, if the
   current deadline has passed or `remaining / (deadline - now) > runtime / period`, the task
   gets `deadline = now + rel_deadline` and full runtime; when runtime is used up the task is
   throttled until its deadline, then replenished (`deadline += period`, `remaining += runtime`).
   Parameters must satisfy `0 < runtime <= deadline <= period`; admission control caps the
   total bandwidth at 95% of the online CPUs (Linux default).
2. **Real-time** (`SCHED_FIFO`, `SCHED_RR`): 100 priority levels, a 128-bit bitmap plus a FIFO
   per level (O(1) pick). RR rotates after 100 ms. Real-time bandwidth is limited to 950 ms per
   1 s per CPU, so a runaway real-time task cannot lock out the rest of the system.
3. **Fair** (`SCHED_OTHER`/`SCHED_BATCH`/`SCHED_IDLE`): EEVDF.
   - Weights from nice via Linux's `sched_prio_to_weight` table (nice 0 = 1024; SCHED_IDLE = 3).
   - `vruntime += delta_exec * 1024 / weight`; `avg_vruntime` is the weight-averaged vruntime
     of the queue, kept incrementally (sum of `weight * vruntime`, 128-bit).
   - Eligible: `vruntime <= avg_vruntime`. Pick: earliest virtual deadline among eligible
     tasks, `deadline = vruntime + slice * 1024 / weight`, base slice 3 ms.
   - Lag (`avg - vruntime`) is kept across sleep and migration, clamped to two slices, and
     scaled on placement by `(W + w) / W` so it is preserved exactly; new tasks start with half
     a slice of deadline.
   - The running task keeps the CPU until its slice is used (run to parity) unless a waking task
     is eligible with an earlier deadline.
4. **Idle**: the per-CPU idle task, run only when all classes are empty.

**SMP placement and balancing** (pure policy functions, applied by the dispatcher):

- Wakeup placement: the previous CPU if it is idle and allowed; otherwise an idle allowed CPU;
  otherwise the previous CPU if the task is cache-hot (ran there in the last 5 ms); otherwise the
  least loaded allowed CPU.
- A CPU that goes idle pulls work from the busiest CPU (new-idle balance); every 4 ms each CPU
  checks for an imbalance above 25% and pulls half the difference. Affinity is always honoured
  and the running task is never migrated.
- Load is a per-CPU decaying average of runnable weight (half-life 32 ms, as PELT), integer only.

**Implementation:** `kernel/src/sched/policy/` holds the classes and balancing as pure code with
no locking or architecture dependency, keyed by an opaque task key, and is tested on the host.
The dispatcher (ADR 0006) owns one `RunQueue` per CPU behind a per-CPU lock.

## Consequences

- O(log n) fair picks (O(1) for real-time), no global queue on the hot path, no whole-queue
  rotation, no per-pick copies.
- Interactive and I/O-bound tasks get low latency from their deadlines without wakeup
  heuristics; CPU-bound tasks still share fairly by weight.
- Real-time and deadline tasks are bounded, so user code cannot starve the kernel's own tasks.
- Priority inheritance (SCHED-INC-01, sprint F) boosts the holder's class and priority through
  the same run queue interface.
- The previous `sched/queue.rs`, `percpu_queue.rs`, `deadline.rs` and the per-policy pick
  functions are retired once the dispatcher uses the new run queues.
