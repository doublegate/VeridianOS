//! Ready queue management for scheduler

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};
use core::ptr::NonNull;

use spin::Mutex;

use super::{
    task::{Priority, SchedClass, Task},
    task_ptr::TaskPtr,
};

/// Ready queue for a single priority level
pub struct PriorityQueue {
    /// Circular queue of task pointers
    tasks: [Option<TaskPtr>; MAX_TASKS_PER_QUEUE],
    /// Head index (next to dequeue)
    head: usize,
    /// Tail index (next to enqueue)
    tail: usize,
    /// Number of tasks in queue
    count: usize,
}

impl PriorityQueue {
    /// Create new empty priority queue
    pub const fn new() -> Self {
        Self {
            tasks: [None; MAX_TASKS_PER_QUEUE],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Check if queue is full
    pub fn is_full(&self) -> bool {
        self.count == MAX_TASKS_PER_QUEUE
    }

    /// Enqueue task
    pub fn enqueue(&mut self, task: NonNull<Task>) -> bool {
        if self.is_full() {
            return false;
        }

        self.tasks[self.tail] = Some(TaskPtr::new(task));
        self.tail = (self.tail + 1) % MAX_TASKS_PER_QUEUE;
        self.count += 1;
        true
    }

    /// Dequeue task
    pub fn dequeue(&mut self) -> Option<NonNull<Task>> {
        if self.is_empty() {
            return None;
        }

        let task = self.tasks[self.head].take();
        self.head = (self.head + 1) % MAX_TASKS_PER_QUEUE;
        self.count -= 1;
        task.map(|t| t.as_ptr())
    }

    /// Peek at next task without removing
    pub fn peek(&self) -> Option<NonNull<Task>> {
        if self.is_empty() {
            None
        } else {
            self.tasks[self.head].map(|t| t.as_ptr())
        }
    }

    /// Remove specific task from queue
    pub fn remove(&mut self, target: NonNull<Task>) -> bool {
        if self.is_empty() {
            return false;
        }

        let mut found = false;
        let mut new_tasks = [None; MAX_TASKS_PER_QUEUE];
        let mut new_count = 0;

        // Copy all tasks except target to new array
        let mut idx = self.head;
        for _ in 0..self.count {
            if let Some(task) = self.tasks[idx] {
                if task.as_ptr() != target {
                    new_tasks[new_count] = Some(task);
                    new_count += 1;
                } else {
                    found = true;
                }
            }
            idx = (idx + 1) % MAX_TASKS_PER_QUEUE;
        }

        if found {
            // Replace with new array
            self.tasks = new_tasks;
            self.head = 0;
            self.tail = new_count;
            self.count = new_count;
        }

        found
    }
}

/// Multi-level ready queue
pub struct ReadyQueue {
    /// Real-time queues by priority
    rt_queues: [PriorityQueue; NUM_RT_PRIORITIES],
    /// Normal priority queues
    normal_queues: [PriorityQueue; NUM_NORMAL_PRIORITIES],
    /// Idle queue
    idle_queue: PriorityQueue,
    /// Bitmap of non-empty real-time queues
    rt_bitmap: u32,
    /// Bitmap of non-empty normal queues
    normal_bitmap: u32,
    /// Whether idle queue has tasks
    idle_flag: bool,
}

impl ReadyQueue {
    /// Create new ready queue
    pub const fn new() -> Self {
        Self {
            rt_queues: [const { PriorityQueue::new() }; NUM_RT_PRIORITIES],
            normal_queues: [const { PriorityQueue::new() }; NUM_NORMAL_PRIORITIES],
            idle_queue: PriorityQueue::new(),
            rt_bitmap: 0,
            normal_bitmap: 0,
            idle_flag: false,
        }
    }

    /// Add task to appropriate queue
    pub fn enqueue(&mut self, task: NonNull<Task>) -> bool {
        unsafe {
            let task_ref = task.as_ref();
            match task_ref.sched_class {
                SchedClass::RealTime => {
                    let idx = (task_ref.priority as usize).min(NUM_RT_PRIORITIES - 1);
                    if self.rt_queues[idx].enqueue(task) {
                        self.rt_bitmap |= 1 << idx;
                        true
                    } else {
                        false
                    }
                }
                SchedClass::Normal => {
                    let idx = ((task_ref.priority as usize).saturating_sub(30) / 10)
                        .min(NUM_NORMAL_PRIORITIES - 1);
                    if self.normal_queues[idx].enqueue(task) {
                        self.normal_bitmap |= 1 << idx;
                        true
                    } else {
                        false
                    }
                }
                SchedClass::Idle => {
                    if self.idle_queue.enqueue(task) {
                        self.idle_flag = true;
                        true
                    } else {
                        false
                    }
                }
            }
        }
    }

    /// Dequeue highest priority task
    pub fn dequeue(&mut self) -> Option<NonNull<Task>> {
        // Check real-time queues first
        if self.rt_bitmap != 0 {
            let idx = self.rt_bitmap.trailing_zeros() as usize;
            if let Some(task) = self.rt_queues[idx].dequeue() {
                if self.rt_queues[idx].is_empty() {
                    self.rt_bitmap &= !(1 << idx);
                }
                return Some(task);
            }
        }

        // Check normal queues
        if self.normal_bitmap != 0 {
            let idx = self.normal_bitmap.trailing_zeros() as usize;
            if let Some(task) = self.normal_queues[idx].dequeue() {
                if self.normal_queues[idx].is_empty() {
                    self.normal_bitmap &= !(1 << idx);
                }
                return Some(task);
            }
        }

        // Check idle queue
        if self.idle_flag {
            if let Some(task) = self.idle_queue.dequeue() {
                if self.idle_queue.is_empty() {
                    self.idle_flag = false;
                }
                return Some(task);
            }
        }

        None
    }

    /// Remove specific task from queues
    pub fn remove(&mut self, task: NonNull<Task>) -> bool {
        unsafe {
            let task_ref = task.as_ref();
            match task_ref.sched_class {
                SchedClass::RealTime => {
                    let idx = (task_ref.priority as usize).min(NUM_RT_PRIORITIES - 1);
                    let removed = self.rt_queues[idx].remove(task);
                    if removed && self.rt_queues[idx].is_empty() {
                        self.rt_bitmap &= !(1 << idx);
                    }
                    removed
                }
                SchedClass::Normal => {
                    let idx = ((task_ref.priority as usize).saturating_sub(30) / 10)
                        .min(NUM_NORMAL_PRIORITIES - 1);
                    let removed = self.normal_queues[idx].remove(task);
                    if removed && self.normal_queues[idx].is_empty() {
                        self.normal_bitmap &= !(1 << idx);
                    }
                    removed
                }
                SchedClass::Idle => {
                    let removed = self.idle_queue.remove(task);
                    if removed && self.idle_queue.is_empty() {
                        self.idle_flag = false;
                    }
                    removed
                }
            }
        }
    }

    /// Check if any tasks are ready
    pub fn has_ready_tasks(&self) -> bool {
        self.rt_bitmap != 0 || self.normal_bitmap != 0 || self.idle_flag
    }
}

/// CFS run queue using red-black tree
#[cfg(feature = "alloc")]
pub struct CfsRunQueue {
    /// Tasks sorted by virtual runtime
    tasks: BTreeMap<u64, Vec<TaskPtr>>,
    /// Minimum virtual runtime
    min_vruntime: u64,
    /// Total weight of all tasks
    total_weight: u64,
}

#[cfg(feature = "alloc")]
impl CfsRunQueue {
    /// Create new CFS run queue
    pub fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            min_vruntime: 0,
            total_weight: 0,
        }
    }

    /// Add task to CFS queue
    pub fn enqueue(&mut self, task: NonNull<Task>) {
        unsafe {
            let task_ref = task.as_ref();
            let vruntime = task_ref.vruntime.max(self.min_vruntime);

            self.tasks
                .entry(vruntime)
                .or_default()
                .push(TaskPtr::new(task));

            self.total_weight += priority_to_weight(task_ref.priority);
        }
    }

    /// Remove task with lowest vruntime
    pub fn dequeue(&mut self) -> Option<NonNull<Task>> {
        if let Some(&vruntime) = self.tasks.keys().next() {
            self.min_vruntime = vruntime;

            // Get mutable reference to remove task
            let tasks = self.tasks.get_mut(&vruntime).unwrap();
            let task = tasks.pop();

            if tasks.is_empty() {
                self.tasks.remove(&vruntime);
            }

            if let Some(task) = task {
                unsafe {
                    let task_ref = task.as_ptr().as_ref();
                    self.total_weight = self
                        .total_weight
                        .saturating_sub(priority_to_weight(task_ref.priority));
                }
            }

            task.map(|t| t.as_ptr())
        } else {
            None
        }
    }

    /// Remove specific task
    pub fn remove(&mut self, target: NonNull<Task>) -> bool {
        unsafe {
            let task_ref = target.as_ref();
            let vruntime = task_ref.vruntime;

            if let Some(tasks) = self.tasks.get_mut(&vruntime) {
                if let Some(pos) = tasks.iter().position(|&t| t.as_ptr() == target) {
                    tasks.remove(pos);
                    self.total_weight = self
                        .total_weight
                        .saturating_sub(priority_to_weight(task_ref.priority));

                    if tasks.is_empty() {
                        self.tasks.remove(&vruntime);
                    }

                    return true;
                }
            }

            false
        }
    }

    /// Update minimum vruntime
    pub fn update_min_vruntime(&mut self, current_vruntime: u64) {
        self.min_vruntime = self.min_vruntime.max(current_vruntime);
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

impl Default for PriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for ReadyQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl Default for CfsRunQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert priority to CFS weight
fn priority_to_weight(priority: Priority) -> u64 {
    // Higher priority = higher weight = more CPU time
    match priority {
        Priority::RealTimeHigh => 0, // Not used for CFS
        Priority::RealTimeNormal => 0,
        Priority::RealTimeLow => 0,
        Priority::SystemHigh => 200,
        Priority::SystemNormal => 100,
        Priority::UserHigh => 50,
        Priority::UserNormal => 20,
        Priority::UserLow => 10,
        Priority::Idle => 1,
    }
}

/// Maximum tasks per priority queue
const MAX_TASKS_PER_QUEUE: usize = 256;

/// Number of real-time priority levels
const NUM_RT_PRIORITIES: usize = 30;

/// Number of normal priority levels
const NUM_NORMAL_PRIORITIES: usize = 4;

/// Global ready queue protected by mutex
#[cfg(not(target_arch = "riscv64"))]
pub static READY_QUEUE: Mutex<ReadyQueue> = Mutex::new(ReadyQueue::new());

/// Global ready queue for RISC-V (avoiding spin::Mutex issues)
#[cfg(target_arch = "riscv64")]
pub static mut READY_QUEUE_STATIC: Option<alloc::boxed::Box<ReadyQueue>> = None;

/// Per-CPU ready queues for SMP
#[cfg(feature = "smp")]
pub static PER_CPU_QUEUES: [Mutex<ReadyQueue>; MAX_CPUS] =
    [const { Mutex::new(ReadyQueue::new()) }; MAX_CPUS];

/// Maximum number of CPUs supported
#[cfg(feature = "smp")]
pub const MAX_CPUS: usize = 64;

/// Get the global ready queue (architecture-specific)
#[cfg(target_arch = "riscv64")]
pub fn get_ready_queue() -> &'static mut ReadyQueue {
    unsafe {
        if READY_QUEUE_STATIC.is_none() {
            // Initialize the ready queue
            #[cfg(feature = "alloc")]
            {
                let queue = alloc::boxed::Box::new(ReadyQueue::new());
                READY_QUEUE_STATIC = Some(queue);
            }
            #[cfg(not(feature = "alloc"))]
            {
                panic!("Cannot initialize ready queue without alloc feature");
            }
        }
        READY_QUEUE_STATIC.as_mut().unwrap().as_mut()
    }
}

/// Initialize the ready queue for RISC-V
#[cfg(target_arch = "riscv64")]
pub fn init_ready_queue() {
    unsafe {
        if READY_QUEUE_STATIC.is_none() {
            #[cfg(feature = "alloc")]
            {
                crate::println!("[SCHED] Initializing RISC-V ready queue...");
                let queue = alloc::boxed::Box::new(ReadyQueue::new());
                READY_QUEUE_STATIC = Some(queue);
                crate::println!("[SCHED] RISC-V ready queue initialized");
            }
        }
    }
}
