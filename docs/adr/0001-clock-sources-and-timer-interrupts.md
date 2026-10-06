# ADR 0001: Clock sources and timer interrupts per architecture

- Status: accepted
- Date: 2026-10-06
- Tracking: W-13, audit verification N-21..N-23

## Context

The audit runtime suite found that no timed wait worked from user space. `nanosleep`, `poll` and
`epoll` timeouts, timerfd and futex timeouts never ended, and `CLOCK_MONOTONIC` always read 0. The
cause was that `timer::get_uptime_ms` read a counter that only `timer::timer_tick` advanced, and
`timer_tick` had no callers.

Fixing the clock also exposed the following:

- **x86_64:**
  - The TSC frequency was hard-coded at 2 GHz.
  - The timer interrupt was a periodic LAPIC timer calibrated against the PIT.
- **AArch64:**
  - `VBAR_EL1` was never written, so any exception or interrupt vectored to address 0.
  - The generic timer was never armed and its interrupt never enabled.
- **RISC-V:** the trap vector panicked on every trap, so no timer interrupt could be taken. The
  timebase was hard-coded at 10 MHz, and the device tree that firmware passes in `a1` was lost
  because `boot.S` cleared every register.
- **All architectures:** the tick path took the timer wheel's outer lock. It would have deadlocked
  as soon as an interrupt arrived while that lock was held. It did exactly that, during
  `timer::init`, the first time RISC-V interrupts were enabled.

## Decision

Each architecture uses its architectural clock as the time base, and the timer device paired with
that clock for the tick.

| Arch | Clock source | Frequency | Tick |
|---|---|---|---|
| x86_64 | TSC (`rdtsc`) | CPUID 0x15 (+0x16 base frequency when the crystal is not enumerated); else the hypervisor leaf 0x40000010 (KVM, Nitro, VMware); else PIT channel 2 calibration (median of 5 x 25 ms) | LAPIC in **TSC-deadline mode** when CPUID.1:ECX[24]; else periodic LAPIC calibrated against the PIT |
| AArch64 | `CNTVCT_EL0`, read after an `ISB` | `CNTFRQ_EL0` | **EL1 virtual timer** (`CNTV_CVAL_EL0`, PPI 11 = INTID 27) via GICv2 |
| RISC-V | `time` CSR (`rdtime`) | device tree `/cpus/timebase-frequency` | **Sstc `stimecmp`** when the device tree lists `sstc`; else SBI `set_timer` |

Supporting decisions:

- **`timer::get_uptime_ms` and `timer::monotonic_ns` read the clock source directly.** Time
  advances whether or not interrupts run, and missed ticks cause no drift. `clock_gettime` and
  timerfd use nanoseconds.
- **The tick runs at 1000 Hz on every architecture.** Deadlines advance by whole periods. When
  ticks are missed the timer skips to the next future deadline instead of firing a burst.
- **The tick handler never spins on a lock.** It drives the timer wheel through
  `GlobalState::try_with_mut` plus `try_lock`, carrying the elapsed time over when the wheel is
  busy. Wheel callbacks run in interrupt context.
- **On x86_64, a syscall halting only so the clock can advance does not get scheduled by the
  tick** (`sched::wait_for_interrupt_in_syscall`, W-13). On AArch64 and RISC-V the tick does not
  call the scheduler at all yet. Preempting kernel code from an interrupt needs the process-model
  work (C5).
- **AArch64 IRQ entry saves the full caller-visible context,** including q0-q31 and FPCR/FPSR,
  because the kernel runs with FP/SIMD enabled. Synchronous exceptions, FIQ, SError and anything
  from EL0 are reported with ESR/ELR/FAR and stop the system.
- **The AArch64 IRQ path reads GICC_IAR/EOIR directly,** since they are banked per CPU, and never
  takes the GIC mutex.
- **RISC-V traps save the caller-saved registers plus `sepc`/`sstatus`.** `sscratch` is 0 in the
  kernel, so a trap from U-mode is detected and treated as fatal. Interrupts other than the
  supervisor timer interrupt are fatal.
- **The EL2 boot path zeroes `CNTVOFF_EL2`,** which has no defined reset value, and grants EL1
  access to the physical counter.

Why these choices, from current kernel practice:

- **TSC-deadline is the most efficient x86 clockevent.** The tick is programmed in clock-source
  units, so it needs no LAPIC bus-clock calibration and cannot drift from the clock. The `WRMSR`
  is preceded by `MFENCE; LFENCE`, as in Linux.
- **CPUID 0x15/0x16 and the 0x40000010 timing leaf are the frequency sources Linux and other
  guests trust.** PIT calibration is the universal fallback.
- **An EL1 kernel uses the virtual counter and timer,** as Linux's `arm_arch_timer` does when not
  running at EL2. The `ISB` keeps the counter read ordered with the surrounding code.
- **Sstc lets S-mode program the timer without trapping to M-mode,** which costs roughly 800
  cycles per tick. Linux prefers it and falls back to SBI.

## Consequences

- Timed waits work. They are runtime-tested by `nanosleep_waits` and `poll_times_out` in
  `audit_runtime_test`, under both the periodic and the TSC-deadline x86 modes.
- Boot test 34 (`timer_interrupts_and_clock`) proves on all three architectures that the clock
  advances and that the interrupt is delivered.
- **AArch64 and RISC-V now run with interrupts enabled.** Code that assumed interrupts were masked
  is exposed. The boot tests, the BusyBox/rootfs suite and DHCP all pass, but new interrupt-context
  code must follow the no-spin rule above.
- **QEMU's default x86 CPU model advertises neither TSC-deadline nor an invariant TSC,** so CI uses
  the periodic LAPIC and PIT calibration. `-cpu host,+invtsc` exercises the TSC-deadline and
  hypervisor-leaf paths, and the hypervisor leaf matched PIT calibration to within 60 ppm.
- **Not done:**
  - HPET as a calibration reference.
  - A tickless (one-shot only) design.
  - Per-CPU timers for SMP.
  - Preemption from the tick on AArch64/RISC-V.
  - External interrupts (PLIC, GIC SPIs) beyond the timer.

## References

- [x86/tsc: Use CPUID.0x16 to calculate missing crystal frequency](https://github.com/torvalds/linux/commit/604dc9170f2435d27da5039a3efd757dceadc684)
- [KVM: x86: Provide TSC frequency in "generic" timing information CPUID leaf](https://lkml.iu.edu/hypermail/linux/kernel/2512.2/01237.html)
- [x86/kvmclock: Get TSC frequency from CPUID when it is available](https://www.mail-archive.com/xen-devel@lists.xenproject.org/msg184081.html)
- [x86, apic: use tsc deadline for oneshot when available](https://lkml.iu.edu/hypermail/linux/kernel/1210.2/03717.html)
- [OSDev Wiki: APIC Timer](https://wiki.osdev.org/APIC_timer)
- [linux/arch/arm64/include/asm/arch_timer.h](https://github.com/torvalds/linux/blob/master/arch/arm64/include/asm/arch_timer.h)
- [AArch64 Programmer's Guide: Generic Timer](https://tc.gts3.org/cs3210/2020/spring/r/aarch64-generic-timer.pdf)
- [RISC-V: Prefer sstc extension if available](https://patchew.org/linux/20220426185245.281182-1-atishp@rivosinc.com/20220426185245.281182-4-atishp@rivosinc.com/)
- [FreeBSD D40241: riscv timer: use stimecmp CSR when available](https://reviews.freebsd.org/D40241)
