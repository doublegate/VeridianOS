//! Core scheduler implementation
//!
//! Contains the `Scheduler` struct which implements the scheduling algorithms
//! (round-robin, priority, CFS, hybrid), task enqueueing/dequeueing, context
//! switching, and the global scheduler instance.

#[cfg(feature = "alloc")]
extern crate alloc;

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use spin::Mutex;

#[cfg(not(target_arch = "riscv64"))]
use super::queue::READY_QUEUE;
use super::{
    queue::CfsRunQueue,
    task::{Priority, SchedClass, Task},
    task_ptr::TaskPtr,
    ProcessState,
};

/// Scheduler state
pub struct Scheduler {
    /// Currently running task
    pub current: Option<TaskPtr>,
    /// Idle task
    pub idle_task: Option<TaskPtr>,
    /// Scheduling algorithm
    pub algorithm: SchedAlgorithm,
    /// Preemption enabled
    pub preemption_enabled: AtomicBool,
    /// Scheduler lock count
    pub lock_count: AtomicU64,
    /// CPU ID this scheduler is running on
    pub cpu_id: u8,
    /// CFS run queue (if using CFS)
    #[cfg(feature = "alloc")]
    pub cfs_queue: Option<Mutex<CfsRunQueue>>,
}

/// Scheduling algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedAlgorithm {
    /// Simple round-robin
    RoundRobin,
    /// Priority-based
    Priority,
    /// Completely Fair Scheduler
    Cfs,
    /// Real-time + CFS hybrid
    Hybrid,
}

impl Scheduler {
    /// Create new scheduler
    pub const fn new() -> Self {
        Self {
            current: None,
            idle_task: None,
            algorithm: SchedAlgorithm::Priority,
            preemption_enabled: AtomicBool::new(true),
            lock_count: AtomicU64::new(0),
            cpu_id: 0, // Default to CPU 0, will be set per-CPU
            #[cfg(feature = "alloc")]
            cfs_queue: None,
        }
    }

    /// Initialize scheduler with idle task
    pub fn init(&mut self, idle_task: NonNull<Task>) {
        let idle_ptr = TaskPtr::new(idle_task);
        self.idle_task = Some(idle_ptr);
        self.current = Some(idle_ptr);

        #[cfg(feature = "alloc")]
        if self.algorithm == SchedAlgorithm::Cfs || self.algorithm == SchedAlgorithm::Hybrid {
            self.cfs_queue = Some(Mutex::new(CfsRunQueue::new()));
        }
    }

    /// Get current task
    pub fn current(&self) -> Option<NonNull<Task>> {
        self.current.map(|t| t.as_ptr())
    }

    /// Disable preemption
    pub fn disable_preemption(&self) {
        self.lock_count.fetch_add(1, Ordering::Acquire);
        self.preemption_enabled.store(false, Ordering::Release);
    }

    /// Enable preemption
    pub fn enable_preemption(&self) {
        let count = self.lock_count.fetch_sub(1, Ordering::Release);
        if count == 1 {
            self.preemption_enabled.store(true, Ordering::Release);
        }
    }

    /// Check if preemption is enabled
    pub fn is_preemptible(&self) -> bool {
        self.preemption_enabled.load(Ordering::Acquire)
    }

    /// Add task to ready queue
    pub fn enqueue(&self, task: NonNull<Task>) {
        // SAFETY: `task` is a valid NonNull<Task> provided by the caller
        // (scheduler.schedule, wake_up_process, etc.). We read task fields
        // (cpu_affinity, sched_class) to determine the correct queue. The
        // caller ensures the task is not concurrently modified.
        let task_ref = unsafe { task.as_ref() };

        // Check if task can run on this CPU
        if !task_ref.can_run_on(self.cpu_id) {
            // Task can't run on this CPU, try to find a suitable CPU
            self.enqueue_on_suitable_cpu(task);
            return;
        }

        // SAFETY: `task` is a valid NonNull<Task>. We update its state to
        // Ready before placing it in a queue. We hold the scheduler lock
        // (callers acquire it before calling enqueue), ensuring exclusive
        // access to the task's state field.
        unsafe {
            let task_mut = task.as_ptr();
            (*task_mut).state = ProcessState::Ready;
        }

        // Use per-CPU queue if available
        if let Some(cpu_data) = super::smp::per_cpu(self.cpu_id) {
            match self.algorithm {
                SchedAlgorithm::RoundRobin | SchedAlgorithm::Priority => {
                    cpu_data.cpu_info.ready_queue.lock().enqueue(task);
                    cpu_data.cpu_info.nr_running.fetch_add(1, Ordering::Relaxed);
                    cpu_data.cpu_info.update_load();
                }
                #[cfg(feature = "alloc")]
                SchedAlgorithm::Cfs => {
                    if let Some(ref cfs) = self.cfs_queue {
                        cfs.lock().enqueue(task);
                        cpu_data.cpu_info.nr_running.fetch_add(1, Ordering::Relaxed);
                        cpu_data.cpu_info.update_load();
                    }
                }
                #[cfg(feature = "alloc")]
                SchedAlgorithm::Hybrid => {
                    if task_ref.sched_class == SchedClass::RealTime {
                        cpu_data.cpu_info.ready_queue.lock().enqueue(task);
                    } else if let Some(ref cfs) = self.cfs_queue {
                        cfs.lock().enqueue(task);
                    }
                    cpu_data.cpu_info.nr_running.fetch_add(1, Ordering::Relaxed);
                    cpu_data.cpu_info.update_load();
                }
            }
        } else {
            // Fallback to global queue
            match self.algorithm {
                SchedAlgorithm::RoundRobin | SchedAlgorithm::Priority => {
                    #[cfg(not(target_arch = "riscv64"))]
                    READY_QUEUE.lock().enqueue(task);
                    #[cfg(target_arch = "riscv64")]
                    super::queue::get_ready_queue().enqueue(task);
                }
                #[cfg(feature = "alloc")]
                SchedAlgorithm::Cfs => {
                    if let Some(ref cfs) = self.cfs_queue {
                        cfs.lock().enqueue(task);
                    }
                }
                #[cfg(feature = "alloc")]
                SchedAlgorithm::Hybrid => {
                    if task_ref.sched_class == SchedClass::RealTime {
                        #[cfg(not(target_arch = "riscv64"))]
                        READY_QUEUE.lock().enqueue(task);
                        #[cfg(target_arch = "riscv64")]
                        super::queue::get_ready_queue().enqueue(task);
                    } else if let Some(ref cfs) = self.cfs_queue {
                        cfs.lock().enqueue(task);
                    }
                }
            }
        }
    }

    /// Enqueue task on a suitable CPU that matches its affinity
    fn enqueue_on_suitable_cpu(&self, task: NonNull<Task>) {
        // SAFETY: `task` is a valid NonNull<Task>. We read cpu_affinity and
        // tid fields to find a suitable CPU and log warnings. The task is not
        // concurrently modified because the caller holds the scheduler lock.
        let task_ref = unsafe { task.as_ref() };

        // Find first CPU that matches task affinity
        for cpu in 0..super::smp::MAX_CPUS as u8 {
            if task_ref.can_run_on(cpu) {
                // Schedule on that CPU
                super::scheduler::schedule_on_cpu(cpu, task);
                return;
            }
        }

        // No suitable CPU found, this is an error
        println!(
            "[SCHED] Warning: Task {} has no valid CPU affinity!",
            task_ref.tid
        );
    }

    /// Select next task to run
    pub fn pick_next(&self) -> Option<NonNull<Task>> {
        match self.algorithm {
            SchedAlgorithm::RoundRobin => self.pick_next_rr(),
            SchedAlgorithm::Priority => self.pick_next_priority(),
            #[cfg(feature = "alloc")]
            SchedAlgorithm::Cfs => self.pick_next_cfs(),
            #[cfg(feature = "alloc")]
            SchedAlgorithm::Hybrid => self.pick_next_hybrid(),
        }
    }

    /// Round-robin task selection
    fn pick_next_rr(&self) -> Option<NonNull<Task>> {
        let current_cpu = self.cpu_id;

        // Try per-CPU queue first
        if let Some(cpu_data) = super::smp::per_cpu(self.cpu_id) {
            let mut queue = cpu_data.cpu_info.ready_queue.lock();

            // Find a task that can run on this CPU
            while let Some(task_ptr) = queue.dequeue() {
                // SAFETY: task_ptr was just dequeued from the ready queue
                // where it was stored as a valid NonNull<Task>. We read
                // cpu_affinity via can_run_on to check compatibility. Tasks
                // in the queue are not deallocated while enqueued.
                unsafe {
                    let task = task_ptr.as_ref();
                    if task.can_run_on(current_cpu) {
                        cpu_data.cpu_info.nr_running.fetch_sub(1, Ordering::Relaxed);
                        cpu_data.cpu_info.update_load();
                        return Some(task_ptr);
                    }
                    // Task can't run on this CPU, re-queue it
                    queue.enqueue(task_ptr);
                }
            }
        } else {
            // Fallback to global queue
            #[cfg(not(target_arch = "riscv64"))]
            {
                let mut queue = READY_QUEUE.lock();

                while let Some(task_ptr) = queue.dequeue() {
                    // SAFETY: Same as above -- task_ptr is valid from the
                    // global ready queue.
                    unsafe {
                        let task = task_ptr.as_ref();
                        if task.can_run_on(current_cpu) {
                            return Some(task_ptr);
                        }
                        // Task can't run on this CPU, re-queue it
                        queue.enqueue(task_ptr);
                    }
                }
            }
            #[cfg(target_arch = "riscv64")]
            {
                let queue = super::queue::get_ready_queue();

                while let Some(task_ptr) = queue.dequeue() {
                    // SAFETY: Same as above -- task_ptr is valid from the
                    // RISC-V global ready queue.
                    unsafe {
                        let task = task_ptr.as_ref();
                        if task.can_run_on(current_cpu) {
                            return Some(task_ptr);
                        }
                        // Task can't run on this CPU, re-queue it
                        queue.enqueue(task_ptr);
                    }
                }
            }
        }

        // Work-stealing: try to steal from the busiest neighbor CPU
        {
            let mut best_cpu = None;
            let mut best_load = 0u32;
            for cpu in 0..super::smp::MAX_CPUS as u8 {
                if cpu == current_cpu {
                    continue;
                }
                if let Some(data) = super::smp::per_cpu(cpu) {
                    let load = data.cpu_info.nr_running.load(Ordering::Relaxed);
                    if load > best_load && load >= 2 {
                        best_load = load;
                        best_cpu = Some(cpu);
                    }
                }
            }

            if let Some(victim_cpu) = best_cpu {
                if let Some(victim_data) = super::smp::per_cpu(victim_cpu) {
                    let mut queue = victim_data.cpu_info.ready_queue.lock();
                    if let Some(task_ptr) = queue.dequeue() {
                        // SAFETY: task_ptr is valid from the victim's ready queue.
                        unsafe {
                            if task_ptr.as_ref().can_run_on(current_cpu) {
                                victim_data
                                    .cpu_info
                                    .nr_running
                                    .fetch_sub(1, Ordering::Relaxed);
                                return Some(task_ptr);
                            }
                            // Can't run here, put it back
                            queue.enqueue(task_ptr);
                        }
                    }
                }
            }
        }

        // No runnable task found, use idle task
        self.idle_task.map(|t| t.as_ptr())
    }

    /// Priority-based task selection
    fn pick_next_priority(&self) -> Option<NonNull<Task>> {
        let current_cpu = self.cpu_id;

        // Try per-CPU queue first
        if let Some(cpu_data) = super::smp::per_cpu(self.cpu_id) {
            let mut queue = cpu_data.cpu_info.ready_queue.lock();

            // The ReadyQueue already maintains priority order with bitmaps
            // Just dequeue the highest priority task that can run on this CPU
            let mut requeue_count = 0;
            let max_attempts = 10; // Prevent infinite loop

            while requeue_count < max_attempts {
                match queue.dequeue() {
                    Some(task_ptr) => {
                        // SAFETY: task_ptr was just dequeued from the per-CPU
                        // ready queue where it was stored as a valid
                        // NonNull<Task>. We read cpu_affinity via can_run_on.
                        unsafe {
                            let task = task_ptr.as_ref();
                            if task.can_run_on(current_cpu) {
                                cpu_data.cpu_info.nr_running.fetch_sub(1, Ordering::Relaxed);
                                cpu_data.cpu_info.update_load();
                                return Some(task_ptr);
                            } else {
                                // Task can't run on this CPU, re-queue it
                                queue.enqueue(task_ptr);
                                requeue_count += 1;
                            }
                        }
                    }
                    None => break, // No more tasks
                }
            }
        } else {
            // Fallback to global queue
            #[cfg(not(target_arch = "riscv64"))]
            {
                let mut queue = READY_QUEUE.lock();

                let mut requeue_count = 0;
                let max_attempts = 10;

                while requeue_count < max_attempts {
                    match queue.dequeue() {
                        // SAFETY: task_ptr is valid from the global ready queue.
                        Some(task_ptr) => unsafe {
                            let task = task_ptr.as_ref();
                            if task.can_run_on(current_cpu) {
                                return Some(task_ptr);
                            } else {
                                queue.enqueue(task_ptr);
                                requeue_count += 1;
                            }
                        },
                        None => break,
                    }
                }
            }
            #[cfg(target_arch = "riscv64")]
            {
                let queue = super::queue::get_ready_queue();

                let mut requeue_count = 0;
                let max_attempts = 10;

                while requeue_count < max_attempts {
                    match queue.dequeue() {
                        // SAFETY: task_ptr is valid from RISC-V ready queue.
                        Some(task_ptr) => unsafe {
                            let task = task_ptr.as_ref();
                            if task.can_run_on(current_cpu) {
                                return Some(task_ptr);
                            } else {
                                queue.enqueue(task_ptr);
                                requeue_count += 1;
                            }
                        },
                        None => break,
                    }
                }
            }
        }

        // No runnable task found, use idle task
        self.idle_task.map(|t| t.as_ptr())
    }

    /// CFS task selection
    #[cfg(feature = "alloc")]
    fn pick_next_cfs(&self) -> Option<NonNull<Task>> {
        let current_cpu = self.cpu_id;

        if let Some(ref cfs) = self.cfs_queue {
            let mut queue = cfs.lock();

            // Find task with lowest vruntime that can run on this CPU
            while let Some(task_ptr) = queue.dequeue() {
                // SAFETY: task_ptr is valid from the CFS run queue. We read
                // cpu_affinity to check if the task can run on this CPU.
                unsafe {
                    let task = task_ptr.as_ref();
                    if task.can_run_on(current_cpu) {
                        return Some(task_ptr);
                    }
                    // Task can't run on this CPU, re-queue it
                    queue.enqueue(task_ptr);
                }
            }
        }

        self.idle_task.map(|t| t.as_ptr())
    }

    /// Hybrid scheduler task selection
    #[cfg(feature = "alloc")]
    fn pick_next_hybrid(&self) -> Option<NonNull<Task>> {
        let current_cpu = self.cpu_id;

        // Check real-time tasks first
        {
            #[cfg(not(target_arch = "riscv64"))]
            {
                let mut queue = READY_QUEUE.lock();
                while let Some(task_ptr) = queue.dequeue() {
                    // SAFETY: task_ptr is valid from the global ready queue.
                    // We check affinity before returning it.
                    unsafe {
                        let task = task_ptr.as_ref();
                        if task.can_run_on(current_cpu) {
                            return Some(task_ptr);
                        }
                        // Task can't run on this CPU, re-queue it
                        queue.enqueue(task_ptr);
                    }
                }
            }
            #[cfg(target_arch = "riscv64")]
            {
                let queue = super::queue::get_ready_queue();
                while let Some(task_ptr) = queue.dequeue() {
                    // SAFETY: Same as above for RISC-V ready queue.
                    unsafe {
                        let task = task_ptr.as_ref();
                        if task.can_run_on(current_cpu) {
                            return Some(task_ptr);
                        }
                        // Task can't run on this CPU, re-queue it
                        queue.enqueue(task_ptr);
                    }
                }
            }
        }

        // Then check CFS queue
        if let Some(ref cfs) = self.cfs_queue {
            let mut queue = cfs.lock();
            while let Some(task_ptr) = queue.dequeue() {
                // SAFETY: task_ptr is valid from the CFS queue.
                unsafe {
                    let task = task_ptr.as_ref();
                    if task.can_run_on(current_cpu) {
                        return Some(task_ptr);
                    }
                    // Task can't run on this CPU, re-queue it
                    queue.enqueue(task_ptr);
                }
            }
        }

        self.idle_task.map(|t| t.as_ptr())
    }

    /// Update task runtime statistics
    pub fn update_runtime(&self, task: NonNull<Task>, runtime: u64) {
        // SAFETY: `task` is a valid NonNull<Task> passed by the caller. We
        // update statistics (runtime via atomic operations) and vruntime for
        // CFS scheduling. The caller holds the scheduler lock ensuring no
        // concurrent modification of the vruntime field.
        unsafe {
            let task_ref = task.as_ref();
            task_ref.update_runtime(runtime);

            // Update vruntime for CFS
            if self.algorithm == SchedAlgorithm::Cfs
                || (self.algorithm == SchedAlgorithm::Hybrid
                    && task_ref.sched_class != SchedClass::RealTime)
            {
                let task_mut = task.as_ptr();
                let weight = priority_to_weight(task_ref.priority);
                (*task_mut).vruntime += runtime * 1024 / weight;
            }
        }
    }

    /// Handle timer tick
    pub fn tick(&mut self) {
        if let Some(current) = self.current {
            // SAFETY: `current` is a TaskPtr stored in the scheduler which
            // points to a valid Task. We are called from the timer interrupt
            // handler with the scheduler lock held. We decrement the time
            // slice and potentially trigger a reschedule if it expires.
            unsafe {
                let task_mut = current.as_raw();

                // Decrement time slice
                if (*task_mut).time_slice > 0 {
                    (*task_mut).time_slice -= 1;
                }

                // Check if time slice expired
                if (*task_mut).time_slice == 0 && self.is_preemptible() {
                    (*task_mut).time_slice = DEFAULT_TIME_SLICE;
                    self.schedule();
                }
            }
        }
    }

    /// Perform scheduling decision
    pub fn schedule(&mut self) {
        let start_cycles = super::metrics::read_tsc();

        // Disable interrupts
        let _guard = crate::arch::disable_interrupts();

        // Check if we can schedule
        if !self.is_preemptible() {
            return;
        }

        // Get current task
        let current = match self.current {
            Some(task) => task,
            None => {
                // No current task, pick one
                if let Some(next) = self.pick_next() {
                    self.switch_to(next);
                }
                super::metrics::SCHEDULER_METRICS
                    .record_scheduler_overhead(super::metrics::read_tsc() - start_cycles);
                return;
            }
        };

        // Check if current task should continue
        // SAFETY: `current` is a TaskPtr from self.current which points to a
        // valid Task. We hold the scheduler lock and interrupts are disabled,
        // so the task is not concurrently modified. We read its state and
        // compare with the next candidate task to decide on preemption.
        unsafe {
            let current_ref = current.as_ptr().as_ref();
            if current_ref.state == ProcessState::Running {
                // Task is still runnable, check if we should preempt
                if let Some(next) = self.pick_next() {
                    let next_ref = next.as_ref();

                    // Preempt if next task has higher priority
                    if should_preempt(current_ref, next_ref) {
                        // Re-queue current task
                        self.enqueue(current.as_ptr());
                        self.switch_to(next);
                        super::metrics::SCHEDULER_METRICS
                            .record_scheduler_overhead(super::metrics::read_tsc() - start_cycles);
                        return;
                    }
                }
                // Current task continues
                super::metrics::SCHEDULER_METRICS
                    .record_scheduler_overhead(super::metrics::read_tsc() - start_cycles);
                return;
            }
        }

        // Current task is not runnable, must switch
        if let Some(next) = self.pick_next() {
            self.switch_to(next);
        }

        super::metrics::SCHEDULER_METRICS
            .record_scheduler_overhead(super::metrics::read_tsc() - start_cycles);
    }

    /// Switch to new task
    fn switch_to(&mut self, next: NonNull<Task>) {
        let next_ptr = TaskPtr::new(next);
        if self.current == Some(next_ptr) {
            return; // Already running
        }

        crate::perf::count_context_switch();
        let start_cycles = super::metrics::read_tsc();

        // Trace: record context switch
        // SAFETY: next is a valid NonNull<Task>.
        unsafe {
            let next_ref = next.as_ref();
            if let Some(current) = self.current {
                let current_ref = current.as_ptr().as_ref();
                crate::trace!(
                    crate::perf::trace::TraceEventType::SchedSwitchOut,
                    current_ref.pid.0,
                    current_ref.tid.0
                );
            }
            crate::trace!(
                crate::perf::trace::TraceEventType::SchedSwitchIn,
                next_ref.pid.0,
                next_ref.tid.0
            );
        }
        // SAFETY: If self.current is Some, its TaskPtr points to a valid Task.
        // We read the state field to determine if the context switch is
        // voluntary (task blocked/sleeping) or involuntary (preemption).
        // The scheduler lock is held ensuring exclusive access.
        let is_voluntary = unsafe {
            if let Some(current) = self.current {
                let current_ref = current.as_ptr().as_ref();
                current_ref.state == ProcessState::Blocked
                    || current_ref.state == ProcessState::Sleeping
            } else {
                false
            }
        };

        // SAFETY: Both `self.current` (if Some) and `next` point to valid
        // Tasks. We hold the scheduler lock and interrupts are disabled
        // (from schedule()), ensuring exclusive access. We update:
        // - current task: state to Ready, clear current_cpu
        // - next task: state to Running, set current_cpu, mark_scheduled
        // - Load context for the first task if no current task exists
        // The context pointers passed to load_context are derived from the
        // task's context field which is valid for the task's lifetime.
        unsafe {
            // Update states
            if let Some(current) = self.current {
                let current_raw = current.as_raw();
                if (*current_raw).state == ProcessState::Running {
                    (*current_raw).state = ProcessState::Ready;
                }
                (*current_raw).current_cpu = None;
            }

            let next_mut = next.as_ptr();
            let next_ref = next.as_ref();

            // Check if scheduling idle task
            if next_ref.sched_class == SchedClass::Idle {
                super::metrics::SCHEDULER_METRICS.record_idle_scheduled();
            }

            (*next_mut).state = ProcessState::Running;
            (*next_mut).current_cpu = Some(current_cpu());
            (*next_mut).mark_scheduled(current_cpu(), is_voluntary);

            // Update TSS kernel stack for Ring 3 -> Ring 0 transitions
            #[cfg(target_arch = "x86_64")]
            {
                let kernel_sp = (*next.as_ptr()).kernel_stack;
                if kernel_sp != 0 {
                    crate::arch::x86_64::gdt::set_kernel_stack(kernel_sp as u64);
                }
            }

            // Lazy TLB: skip CR3 reload when switching to kernel threads
            // (has_user_mappings == false). Kernel threads share the same
            // kernel page table mappings, so CR3 reload is unnecessary.
            // This saves ~100-300 cycles per kernel-to-kernel switch.
            if (*next.as_ptr()).has_user_mappings {
                let next_pt = (*next.as_ptr()).page_table;
                if next_pt != 0 {
                    // Only reload CR3 if switching to a different address space
                    let current_pt = if let Some(current) = self.current {
                        (*current.as_raw()).page_table
                    } else {
                        0
                    };
                    if next_pt != current_pt {
                        #[cfg(target_arch = "x86_64")]
                        {
                            // SAFETY: next_pt is a valid page table physical address
                            // from the Task struct, set during process creation.
                            // Already inside an unsafe block from the parent scope.
                            core::arch::asm!(
                                "mov cr3, {}",
                                in(reg) next_pt,
                                options(nostack, preserves_flags)
                            );
                        }
                    }
                }
            }

            // Perform context switch
            if let Some(current) = self.current {
                // Update current pointer BEFORE context_switch, since
                // context_switch saves current registers and jumps to
                // next's saved RIP. When this task resumes later,
                // execution continues after this point with the correct
                // self.current already set.
                self.current = Some(next_ptr);

                // Save current context and restore next context.
                // This call does NOT return until this task is scheduled
                // again -- at that point, execution resumes here.
                let current_raw = current.as_raw();
                context_switch(&mut (*current_raw).context, &(*next.as_ptr()).context);

                // Resumed: Record metrics and report RCU quiescent state.
                // A context switch is a quiescent point: no RCU read-side
                // references from before the switch can be held.
                crate::sync::rcu::rcu_quiescent();
                let switch_cycles = super::metrics::read_tsc() - start_cycles;
                super::metrics::SCHEDULER_METRICS
                    .record_context_switch(switch_cycles, is_voluntary);
                return;
            } else {
                // First task, load its context directly
                // This happens when scheduler starts with bootstrap task
                #[cfg(target_arch = "x86_64")]
                {
                    crate::arch::x86_64::idt::raw_serial_str(b"[SCHED] DISPATCH_FIRST pid=0x");
                    crate::arch::x86_64::idt::raw_serial_hex((*next.as_ptr()).pid.0);
                    crate::arch::x86_64::idt::raw_serial_str(b"\n");
                }
                match &(*next.as_ptr()).context {
                    #[cfg(target_arch = "x86_64")]
                    crate::sched::task::TaskContext::X86_64(ctx) => {
                        // SAFETY: ctx is a reference to a valid X86_64Context
                        // within the task. load_context restores CPU registers
                        // from this context structure.
                        crate::arch::x86_64::context::load_context(ctx as *const _);
                    }
                    #[cfg(target_arch = "aarch64")]
                    crate::sched::task::TaskContext::AArch64(ctx) => {
                        // SAFETY: ctx is a reference to a valid AArch64Context.
                        // load_context restores registers from it.
                        crate::arch::aarch64::context::load_context(ctx as *const _);
                    }
                    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
                    crate::sched::task::TaskContext::RiscV(ctx) => {
                        // SAFETY: ctx is a reference to a valid RiscVContext.
                        // load_context restores registers from it.
                        crate::arch::riscv::context::load_context(ctx as *const _);
                    }
                }
            }

            // Update current task
            self.current = Some(next_ptr);
        }

        // Record context switch metrics and RCU quiescent state.
        crate::sync::rcu::rcu_quiescent();
        let switch_cycles = super::metrics::read_tsc() - start_cycles;
        super::metrics::SCHEDULER_METRICS.record_context_switch(switch_cycles, is_voluntary);
    }
}

/// Check if should preempt current task
pub fn should_preempt(current: &Task, next: &Task) -> bool {
    // Never preempt idle task except for any real task
    if current.sched_class == SchedClass::Idle && next.sched_class != SchedClass::Idle {
        return true;
    }

    // Real-time tasks always preempt non-real-time
    if next.sched_class == SchedClass::RealTime && current.sched_class != SchedClass::RealTime {
        return true;
    }

    // Within same class, check effective priority
    if next.sched_class == current.sched_class {
        let next_prio = next.effective_priority();
        let curr_prio = current.effective_priority();

        // For real-time tasks, always preempt if higher priority
        if current.sched_class == SchedClass::RealTime {
            return next_prio < curr_prio;
        }

        // For normal tasks, only preempt if significantly higher priority
        // or current task has used up its time slice
        if current.sched_class == SchedClass::Normal {
            return next_prio < curr_prio || (next_prio == curr_prio && current.time_slice == 0);
        }
    }

    false
}

/// Convert priority to weight for CFS
fn priority_to_weight(priority: Priority) -> u64 {
    match priority {
        Priority::SystemHigh => 200,
        Priority::SystemNormal => 100,
        Priority::UserHigh => 50,
        Priority::UserNormal => 20,
        Priority::UserLow => 10,
        Priority::Idle => 1,
        _ => 20, // Default for real-time (not used in CFS)
    }
}

/// Architecture-specific context switch
#[cfg(target_arch = "x86_64")]
fn context_switch(current: &mut super::task::TaskContext, next: &super::task::TaskContext) {
    use super::task::TaskContext;
    match (current, next) {
        // SAFETY: Both curr and next point to valid context structures within
        // their respective Tasks. context_switch saves curr's registers and
        // restores next's. The scheduler lock and disabled interrupts ensure
        // no concurrent access to these contexts.
        (TaskContext::X86_64(curr), TaskContext::X86_64(next)) => unsafe {
            crate::arch::x86_64::context::context_switch(curr as *mut _, next as *const _);
        },
    }
}

#[cfg(target_arch = "aarch64")]
fn context_switch(current: &mut super::task::TaskContext, next: &super::task::TaskContext) {
    use super::task::TaskContext;
    match (current, next) {
        // SAFETY: Same as x86_64 -- valid context structures, scheduler lock
        // held, interrupts disabled.
        (TaskContext::AArch64(curr), TaskContext::AArch64(next)) => unsafe {
            crate::arch::aarch64::context::context_switch(curr as *mut _, next as *const _);
        },
    }
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
fn context_switch(current: &mut super::task::TaskContext, next: &super::task::TaskContext) {
    use super::task::TaskContext;
    match (current, next) {
        // SAFETY: Same as x86_64 -- valid context structures, scheduler lock
        // held, interrupts disabled.
        (TaskContext::RiscV(curr), TaskContext::RiscV(next)) => unsafe {
            crate::arch::riscv::context::context_switch(curr as *mut _, next as *const _);
        },
    }
}

/// Load context for first task
#[cfg(target_arch = "x86_64")]
fn load_context(context: &super::task::TaskContext) {
    use crate::arch::x86_64::context::X86_64Context;

    match context {
        // SAFETY: ctx is a valid reference to an X86_64Context within the
        // bootstrap task. load_context restores CPU registers from this
        // structure. This is called once during scheduler startup.
        super::task::TaskContext::X86_64(ctx) => unsafe {
            crate::arch::x86_64::context::load_context(ctx as *const X86_64Context);
        },
    }
}

#[cfg(target_arch = "aarch64")]
fn load_context(context: &super::task::TaskContext) {
    use crate::arch::aarch64::context::AArch64Context;

    match context {
        // SAFETY: Same as x86_64 -- valid context reference for the bootstrap
        // task. load_context restores CPU registers.
        super::task::TaskContext::AArch64(ctx) => unsafe {
            crate::arch::aarch64::context::load_context(ctx as *const AArch64Context);
        },
        #[allow(unreachable_patterns)]
        _ => unreachable!("Invalid context type for AArch64"),
    }
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
fn load_context(context: &super::task::TaskContext) {
    use crate::arch::riscv::context::RiscVContext;

    match context {
        // SAFETY: Same as x86_64 -- valid context reference for the bootstrap
        // task. load_context restores CPU registers.
        super::task::TaskContext::RiscV(ctx) => unsafe {
            crate::arch::riscv::context::load_context(ctx as *const RiscVContext);
        },
        #[allow(unreachable_patterns)]
        _ => unreachable!("Invalid context type for RISC-V"),
    }
}

/// Get current CPU ID
fn current_cpu() -> u8 {
    super::smp::current_cpu_id()
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Default time slice
const DEFAULT_TIME_SLICE: u32 = 10;

/// Global scheduler instance (for BSP/CPU0)
pub static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::new());

// ---- Global PID-to-Task registry for O(log n) lookup ----

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;

/// Wrapper for NonNull<Task> that implements Send+Sync.
///
/// SAFETY: Task pointers in the registry are only accessed under the
/// TASK_REGISTRY mutex. Tasks are pinned in memory (Box::leak) and
/// outlive their registry entries. Cross-CPU access is serialized
/// by the mutex.
#[derive(Clone, Copy)]
struct SendTaskPtr(NonNull<Task>);
unsafe impl Send for SendTaskPtr {}
unsafe impl Sync for SendTaskPtr {}

/// Global PID-to-Task pointer registry.
///
/// Enables O(log n) task lookup by PID without holding the scheduler lock.
/// Tasks are registered on creation and unregistered on exit.
#[cfg(feature = "alloc")]
static TASK_REGISTRY: Mutex<Option<BTreeMap<u64, SendTaskPtr>>> = Mutex::new(None);

/// Register a task in the global PID-to-Task registry.
#[cfg(feature = "alloc")]
pub fn register_task(pid: u64, task: NonNull<Task>) {
    let mut registry = TASK_REGISTRY.lock();
    let map = registry.get_or_insert_with(BTreeMap::new);
    map.insert(pid, SendTaskPtr(task));
}

/// Unregister a task from the global PID-to-Task registry.
#[cfg(feature = "alloc")]
pub fn unregister_task(pid: u64) {
    let mut registry = TASK_REGISTRY.lock();
    if let Some(map) = registry.as_mut() {
        map.remove(&pid);
    }
}

/// Look up a task pointer by PID. O(log n) via BTreeMap.
///
/// Returns the NonNull<Task> if the PID is registered. This is used by the
/// IPC fast path for direct task-to-task message transfer without iterating
/// the scheduler's run queues.
#[cfg(feature = "alloc")]
pub fn get_task_ptr(pid: u64) -> Option<NonNull<Task>> {
    let registry = TASK_REGISTRY.lock();
    registry.as_ref().and_then(|map| map.get(&pid).map(|p| p.0))
}

/// Get scheduler for current CPU
pub fn current_scheduler() -> &'static Mutex<Scheduler> {
    let cpu_id = current_cpu();

    // Try to get per-CPU scheduler
    if let Some(cpu_data) = super::smp::per_cpu(cpu_id) {
        // Return reference to per-CPU scheduler
        &cpu_data.cpu_info.scheduler
    } else {
        // Fallback to global scheduler for BSP
        &SCHEDULER
    }
}

/// Schedule on specific CPU
pub fn schedule_on_cpu(cpu_id: u8, task: NonNull<Task>) {
    // SAFETY: `task` is a valid NonNull<Task> provided by the caller. We
    // read the task's cpu_affinity and tid fields to verify it can run on
    // the specified CPU. The task is not concurrently modified because the
    // caller manages its lifecycle.
    unsafe {
        let task_ref = task.as_ref();
        if !task_ref.can_run_on(cpu_id) {
            println!(
                "[SCHED] Warning: Task {} cannot run on CPU {} due to affinity mask",
                task_ref.tid, cpu_id
            );
            return;
        }
    }

    if let Some(cpu_data) = super::smp::per_cpu(cpu_id) {
        // Add to per-CPU ready queue
        cpu_data.cpu_info.ready_queue.lock().enqueue(task);
        cpu_data.cpu_info.nr_running.fetch_add(1, Ordering::Relaxed);
        cpu_data.cpu_info.update_load();

        // Send IPI if needed
        if cpu_data.cpu_info.is_idle() {
            super::smp::send_ipi(cpu_id, 0); // Wake up CPU
        }
    }
}
