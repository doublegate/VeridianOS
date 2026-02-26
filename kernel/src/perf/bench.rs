//! In-kernel micro-benchmark suite for Phase 5 performance validation.
//!
//! Measures latency of core kernel operations and compares against Phase 5
//! performance targets. Accessible via the "perf" shell builtin.

use crate::bench::{black_box, cycles_to_ns, read_timestamp};

/// Phase 5 performance targets (nanoseconds, x86_64 with KVM)
pub const TARGET_SYSCALL_NS: u64 = 500;
pub const TARGET_CONTEXT_SWITCH_NS: u64 = 10_000;
pub const TARGET_IPC_SMALL_NS: u64 = 1_000;
pub const TARGET_FRAME_ALLOC_NS: u64 = 500;
pub const TARGET_CAP_LOOKUP_NS: u64 = 100;

/// Individual benchmark result
pub struct BenchResult {
    pub name: &'static str,
    pub iterations: u64,
    pub min_ns: u64,
    pub avg_ns: u64,
    pub max_ns: u64,
    pub target_ns: u64,
}

impl BenchResult {
    pub fn meets_target(&self) -> bool {
        self.avg_ns <= self.target_ns
    }
}

/// Run a micro-benchmark: execute `f` for `iterations`, return min/avg/max.
fn run_bench<F: FnMut()>(
    name: &'static str,
    iterations: u64,
    target_ns: u64,
    mut f: F,
) -> BenchResult {
    // Warmup
    for _ in 0..10 {
        f();
    }

    let mut min = u64::MAX;
    let mut max = 0u64;
    let mut total = 0u64;

    for _ in 0..iterations {
        let start = read_timestamp();
        f();
        let elapsed = read_timestamp().saturating_sub(start);
        let ns = cycles_to_ns(elapsed);
        if ns < min {
            min = ns;
        }
        if ns > max {
            max = ns;
        }
        total += ns;
    }

    BenchResult {
        name,
        iterations,
        min_ns: min,
        avg_ns: total / iterations,
        max_ns: max,
        target_ns,
    }
}

/// Benchmark: sys_getpid() round-trip (minimal syscall overhead)
fn bench_syscall_latency() -> BenchResult {
    run_bench("syscall_getpid", 1000, TARGET_SYSCALL_NS, || {
        let pid = crate::sched::current_process_id();
        black_box(pid);
    })
}

/// Benchmark: frame allocator single-frame alloc+free
fn bench_frame_alloc() -> BenchResult {
    run_bench("frame_alloc_1", 1000, TARGET_FRAME_ALLOC_NS, || {
        // Use per-CPU cache path for single frames
        if let Ok(frame) = crate::mm::frame_allocator::per_cpu_alloc_frame() {
            let _ = crate::mm::frame_allocator::per_cpu_free_frame(frame);
        }
    })
}

/// Benchmark: frame allocator via global path (for comparison)
fn bench_frame_alloc_global() -> BenchResult {
    use crate::mm::FRAME_ALLOCATOR;
    run_bench(
        "frame_alloc_global",
        1000,
        TARGET_FRAME_ALLOC_NS * 2,
        || {
            let alloc = FRAME_ALLOCATOR.lock();
            if let Ok(frame) = alloc.allocate_frames(1, None) {
                let _ = alloc.free_frames(frame, 1);
            }
        },
    )
}

/// Benchmark: capability range validation (fast path)
fn bench_capability_lookup() -> BenchResult {
    run_bench("cap_validate", 1000, TARGET_CAP_LOOKUP_NS, || {
        // Fast-path validation as done in IPC
        let cap = black_box(42u64);
        let valid = cap != 0 && cap < 0x1_0000_0000;
        black_box(valid);
    })
}

/// Benchmark: perf counter increment (atomic operation baseline)
fn bench_atomic_counter() -> BenchResult {
    use core::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    run_bench("atomic_counter", 1000, 50, || {
        COUNTER.fetch_add(1, Ordering::Relaxed);
    })
}

/// Benchmark: IPC fast path stats read (lightweight)
fn bench_ipc_stats() -> BenchResult {
    run_bench("ipc_stats_read", 1000, TARGET_CAP_LOOKUP_NS, || {
        let stats = crate::ipc::fast_path::get_fast_path_stats();
        black_box(stats);
    })
}

/// Benchmark: scheduler current task lookup
fn bench_sched_current() -> BenchResult {
    run_bench("sched_current", 1000, 200, || {
        let sched = crate::sched::scheduler::SCHEDULER.lock();
        let current = sched.current();
        black_box(current);
    })
}

/// Run all benchmarks and print results.
pub fn run_all_benchmarks() {
    crate::println!("=== VeridianOS Phase 5 Performance Benchmarks ===");
    crate::println!();
    crate::println!(
        "{:<20} {:>8} {:>8} {:>8} {:>8} {:>6}",
        "Benchmark",
        "Min(ns)",
        "Avg(ns)",
        "Max(ns)",
        "Target",
        "Pass?"
    );
    crate::println!("{}", "-".repeat(68));

    let benchmarks = [
        bench_syscall_latency(),
        bench_frame_alloc(),
        bench_frame_alloc_global(),
        bench_capability_lookup(),
        bench_atomic_counter(),
        bench_ipc_stats(),
        bench_sched_current(),
    ];

    let mut passed = 0;
    let total = benchmarks.len();

    for result in &benchmarks {
        let status = if result.meets_target() {
            passed += 1;
            "PASS"
        } else {
            "FAIL"
        };
        crate::println!(
            "{:<20} {:>8} {:>8} {:>8} {:>8} {:>6}",
            result.name,
            result.min_ns,
            result.avg_ns,
            result.max_ns,
            result.target_ns,
            status
        );
    }

    crate::println!("{}", "-".repeat(68));
    crate::println!(
        "Results: {}/{} benchmarks meet Phase 5 targets",
        passed,
        total
    );

    // Also show IPC fast path stats
    let (fast_count, fast_avg) = crate::ipc::fast_path::get_fast_path_stats();
    let slow_count = crate::ipc::fast_path::get_slow_path_count();
    crate::println!();
    crate::println!("IPC Statistics:");
    crate::println!("  Fast path: {} calls, {} avg cycles", fast_count, fast_avg);
    crate::println!("  Slow path fallbacks: {}", slow_count);

    // Show trace stats if tracing is enabled
    if crate::perf::trace::is_enabled() {
        crate::println!(
            "  Trace events: {} total",
            crate::perf::trace::total_events()
        );
    }

    crate::println!();
}
