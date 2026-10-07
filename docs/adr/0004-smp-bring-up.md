# ADR 0004: SMP bring-up and per-CPU state

- Status: accepted
- Date: 2026-10-07
- Tracking: v0.27.0 sprint B; audit N-35, N-14, N-28; SCHED-PERF-01, SMP-PERF-01 (later stages)

## Context

Until v0.27 only the boot CPU ever ran, on every architecture. `smp::init` only printed,
`wake_up_aps` had no callers, the x86 trampoline did not exist, and `cpu_up` marked a CPU online
before it had run. Per-CPU data was never initialised, so every per-CPU path fell back to globals.

Several globals also assumed a single CPU:
- the x86 syscall frame pointer (N-35);
- the LAPIC behind a spinlock taken in every EOI;
- one tick deadline shared between CPUs;
- a global tick that every CPU would advance;
- a single TSS.

## Decision

**Two layers of per-CPU state.**

- The architecture block, `arch::percpu::ArchCpu`, is a static array of 64-byte blocks indexed by
  logical id. Each CPU reaches its own block through a register: `KernelGsBase`/`GS_BASE` on x86_64
  (whichever of the two holds a block address), `TPIDR_EL1` on AArch64, `tp` on RISC-V.
- Entry assembly uses fixed offsets, which are asserted:
  - `kernel_rsp` 0x00 and `user_rsp` 0x08 (unchanged from x86's old `PerCpuData`);
  - `syscall_frame` 0x10 (N-35);
  - `cpu_id` 0x20 and `hw_id` 0x24;
  - `timer_next`, `local_ticks` and `ipis`.
- Scheduler per-CPU state (`sched::smp::PerCpuData`) is a separate layer, initialised in stage S2.

**Logical ids are dense.** Logical CPU 0 is the CPU that booted the kernel. The boot CPU numbers the
others 1, 2, ... in firmware-table order. Hardware ids (APIC ID, MPIDR affinity, hartid) are kept in
`hw_id` and never used as indices. RISC-V's boot hart need not be hart 0: OpenSBI chooses it, and
the PLIC context follows the hart actually booted.

**One timekeeper.** Only CPU 0 advances uptime, the tick count and the timer wheel (Linux:
`tick_do_timer_cpu`). Each CPU keeps its own tick deadline, because the compare registers are per
CPU.

**Lock-free Local APIC.** EOI, IPIs and the local timer reach the calling CPU's LAPIC through an
atomic base address. Only the shared I/O APIC stays under a lock.

**Bring-up driver (`arch::smp_boot`).** The boot CPU fills one `ApBootArgs` (stack, per-CPU block,
x86 CR3/CR4), starts one CPU, and waits up to 1 s for it to report ONLINE before starting the next.
If a CPU never reports, it is left out and no further CPUs are started, because it might still read
the shared block. The boot is never failed.

**Per architecture:**

| | Enumeration | Start | Entry |
|---|---|---|---|
| x86_64 | ACPI MADT, Enabled entries only | INIT, 10 ms, SIPI, 200 us, SIPI on the TSC; ESR checked | Trampoline copied below 1 MiB with its own PML4 (real, protected, then long mode), then a higher-half stub loads the kernel CR3/CR4 and stack. Each AP gets its own GDT and TSS (a shared TSS would #GP on the second `ltr`). |
| AArch64 | device tree `/cpus`, `/psci method` | PSCI `CPU_ON` (HVC or SMC) | Stub drops EL2 to EL1 if needed, sets SP and `TPIDR_EL1` |
| RISC-V | device tree `/cpus` | SBI HSM `hart_start` | Stub sets `sp`, `tp`, `stvec` |

AArch64's link base moved to 0x4020_0000. QEMU places the device tree below a bare-metal ELF only
if it fits, and the virt DTB is 1 MiB.

**Staged enablement behind the `smp` feature:**

| Stage | Secondaries | State |
|---|---|---|
| S1 | Own vectors, interrupt controller and tick; park in an idle loop; no tasks, no scheduler lock (x86 secondaries skip the scheduler tick) | done |
| S2 | Per-CPU run queues and idle tasks; TLB shootdown | next |
| S3 | User tasks on every CPU; `smp` on by default | after the process-model work (sprint D) |

AArch64 stays at a reduced S1 until its MMU is on (N-28). With the MMU off, all memory is Device,
and load/store-exclusive across CPUs is IMPLEMENTATION DEFINED. QEMU TCG hides this. So an AArch64
secondary uses plain stores only, takes no lock, runs no timer and keeps interrupts masked.

## Consequences

- Without the `smp` feature, behaviour is unchanged. Builds on one CPU take the same paths except
  for the per-CPU register setup. The boot tests pass in both builds.
- With `FEATURES=smp`, QEMU `-smp 4` (and `-smp 8` on x86_64) brings every CPU online on all three
  architectures. Boot tests 35 (per-CPU identity and local ticks) and 36 (uptime tracks the hardware
  clock) check this, and CI runs each architecture both ways.
- **Remaining single-CPU assumptions, to fix before S3:**
  - the "boot return" context and boot-dispatch globals (`BOOT_RETURN_*`, `BOOT_CURRENT_*`);
  - `RUNNING_TASK`, `SYSCALL_WAIT_DEPTH` and `CURRENT_PROCESS_PTR`;
  - the x86 `TSS_RSP0_PTR`, which points at the boot CPU's TSS;
  - the `swapgs` rebalancing sites, which still assume the kernel per-CPU block convention of today.
- **Not supported yet:**
  - x2APIC mode (refused with a message);
  - GICv3 (AArch64 is limited to 8 CPUs on GICv2);
  - CPU hot-plug.
