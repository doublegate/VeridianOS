# Scheduler Design

> **Authoritative specification**: [docs/design/SCHEDULER-DESIGN.md](https://github.com/doublegate/VeridianOS/blob/main/docs/design/SCHEDULER-DESIGN.md)
>
> **Implementation Status (v0.27.0)**: every thread is a task dispatched on its own kernel stack (ADR 0006), with an EEVDF fair class and FIFO/RR and SCHED_DEADLINE above it (ADR 0007), per-CPU run queues with wake placement and load balancing (ADR 0012). Context-switch time is not measured yet.

See the authoritative specification linked above for the full design document including multi-level feedback queues, real-time scheduling, EDF support, and priority inheritance protocol details.
