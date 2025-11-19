//! Core scheduler implementation

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
        let task_ref = unsafe { task.as_ref() };

        // Check if task can run on this CPU
        if !task_ref.can_run_on(self.cpu_id) {
            // Task can't run on this CPU, try to find a suitable CPU
            self.enqueue_on_suitable_cpu(task);
            return;
        }

        // Update task state
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

        let start_cycles = super::metrics::read_tsc();
        let is_voluntary = unsafe {
            if let Some(current) = self.current {
                let current_ref = current.as_ptr().as_ref();
                current_ref.state == ProcessState::Blocked
                    || current_ref.state == ProcessState::Sleeping
            } else {
                false
            }
        };

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

            // Perform context switch
            if let Some(_current) = self.current {
                // Save current context and switch to next
                // For now, we'll just update the task pointer
                // TODO: Implement actual context save/restore
            } else {
                // First task, load its context directly
                // This happens when scheduler starts with bootstrap task
                match &(*next.as_ptr()).context {
                    #[cfg(target_arch = "x86_64")]
                    crate::sched::task::TaskContext::X86_64(ctx) => {
                        crate::arch::x86_64::context::load_context(ctx as *const _);
                    }
                    #[cfg(target_arch = "aarch64")]
                    crate::sched::task::TaskContext::AArch64(ctx) => {
                        crate::arch::aarch64::context::load_context(ctx as *const _);
                    }
                    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
                    crate::sched::task::TaskContext::RiscV(ctx) => {
                        crate::arch::riscv::context::load_context(ctx as *const _);
                    }
                }
            }

            // Update current task
            self.current = Some(next_ptr);
        }

        // Record context switch metrics
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
        (TaskContext::X86_64(curr), TaskContext::X86_64(next)) => unsafe {
            crate::arch::x86_64::context::context_switch(curr as *mut _, next as *const _);
        },
    }
}

#[cfg(target_arch = "aarch64")]
fn context_switch(current: &mut super::task::TaskContext, next: &super::task::TaskContext) {
    use super::task::TaskContext;
    match (current, next) {
        (TaskContext::AArch64(curr), TaskContext::AArch64(next)) => unsafe {
            crate::arch::aarch64::context::context_switch(curr as *mut _, next as *const _);
        },
    }
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
fn context_switch(current: &mut super::task::TaskContext, next: &super::task::TaskContext) {
    use super::task::TaskContext;
    match (current, next) {
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
        super::task::TaskContext::X86_64(ctx) => unsafe {
            // Load the initial context
            crate::arch::x86_64::context::load_context(ctx as *const X86_64Context);
        },
    }
}

#[cfg(target_arch = "aarch64")]
fn load_context(context: &super::task::TaskContext) {
    use crate::arch::aarch64::context::AArch64Context;

    match context {
        super::task::TaskContext::AArch64(ctx) => unsafe {
            // Load the initial context
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
        super::task::TaskContext::RiscV(ctx) => unsafe {
            // Load the initial context
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
    // Check if task can run on the specified CPU
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
