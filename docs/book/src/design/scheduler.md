# Scheduler Design

> **Authoritative specification**: [docs/design/SCHEDULER-DESIGN.md](https://github.com/doublegate/VeridianOS/blob/main/docs/design/SCHEDULER-DESIGN.md)
>
> **Implementation Status**: Complete as of v0.25.1. CFS with SMP, NUMA-aware load balancing, CPU hotplug, work-stealing. Context switch <10us. Benchmarked at 77ns sched_current.

See the authoritative specification linked above for the full design document including multi-level feedback queues, real-time scheduling, EDF support, and priority inheritance protocol details.
