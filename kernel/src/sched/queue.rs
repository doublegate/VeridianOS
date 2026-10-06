//! Ready queue management for scheduler

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};
use core::ptr::NonNull;

#[allow(unused_imports)]
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

    /// Number of queued tasks.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Remove and return the first task (in queue order) for which `pred`
    /// holds, leaving the others in place and in order.
    pub fn take_first_where(&mut self, pred: impl Fn(&Task) -> bool) -> Option<NonNull<Task>> {
        let target = (0..self.count)
            .filter_map(|i| self.tasks[(self.head + i) % MAX_TASKS_PER_QUEUE])
            .map(|t| t.as_ptr())
            // SAFETY: queued tasks are valid while queued (scheduler
            // invariant); `pred` only reads them.
            .find(|t| pred(unsafe { t.as_ref() }))?;
        self.remove(target);
        Some(target)
    }

    /// Remove specific task from queue, keeping the others in order.
    ///
    /// Compacts in place (SCHED-PERF-03): the previous version built a
    /// second 256-entry array on the stack and copied every task into it.
    pub fn remove(&mut self, target: NonNull<Task>) -> bool {
        let Some(k) = (0..self.count).find(|&i| {
            self.tasks[(self.head + i) % MAX_TASKS_PER_QUEUE].map(|t| t.as_ptr()) == Some(target)
        }) else {
            return false;
        };
        for i in k..self.count - 1 {
            self.tasks[(self.head + i) % MAX_TASKS_PER_QUEUE] =
                self.tasks[(self.head + i + 1) % MAX_TASKS_PER_QUEUE];
        }
        self.tail = (self.tail + MAX_TASKS_PER_QUEUE - 1) % MAX_TASKS_PER_QUEUE;
        self.tasks[self.tail] = None;
        self.count -= 1;
        true
    }
}

/// Multi-level ready queue
///
/// Cache-line aligned to prevent false sharing when per-CPU ready queues
/// are stored in adjacent array slots accessed by different cores.
#[repr(align(64))]
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
    /// Tasks queued across all levels
    len: usize,
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
            len: 0,
        }
    }

    /// A new, empty queue built directly on the heap. `ReadyQueue` is ~72
    /// KiB; `Box::new(ReadyQueue::new())` would build it on the stack first
    /// (SCHED-PERF-03).
    #[cfg(feature = "alloc")]
    pub fn new_boxed() -> alloc::boxed::Box<Self> {
        // SAFETY: every field of an empty ReadyQueue is all-zero bits: the
        // indices, counts and bitmaps are 0, the bool is false and each
        // Option<TaskPtr> (a NonNull wrapper) is None, which is the null
        // niche. Checked by the `boxed_queue_is_empty` test.
        unsafe { alloc::boxed::Box::<Self>::new_zeroed().assume_init() }
    }

    /// Tasks queued across all levels.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no task is queued.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Add task to appropriate queue
    pub fn enqueue(&mut self, task: NonNull<Task>) -> bool {
        let added = self.enqueue_inner(task);
        if added {
            self.len += 1;
        }
        added
    }

    fn enqueue_inner(&mut self, task: NonNull<Task>) -> bool {
        // SAFETY: `task` is a valid NonNull<Task> provided by the scheduler.
        // We read sched_class and priority to determine which sub-queue to
        // use. The ReadyQueue is protected by a Mutex, ensuring exclusive
        // access during this operation.
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
        let task = self.dequeue_inner();
        if task.is_some() {
            self.len -= 1;
        }
        task
    }

    fn dequeue_inner(&mut self) -> Option<NonNull<Task>> {
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

    /// Remove and return the highest-priority task for which `pred` holds
    /// (FIFO within a level), leaving all others where they are.
    pub fn take_first_where(&mut self, pred: impl Fn(&Task) -> bool) -> Option<NonNull<Task>> {
        let mut rt = self.rt_bitmap;
        while rt != 0 {
            let idx = rt.trailing_zeros() as usize;
            rt &= !(1 << idx);
            if let Some(t) = self.rt_queues[idx].take_first_where(&pred) {
                if self.rt_queues[idx].is_empty() {
                    self.rt_bitmap &= !(1 << idx);
                }
                self.len -= 1;
                return Some(t);
            }
        }
        let mut normal = self.normal_bitmap;
        while normal != 0 {
            let idx = normal.trailing_zeros() as usize;
            normal &= !(1 << idx);
            if let Some(t) = self.normal_queues[idx].take_first_where(&pred) {
                if self.normal_queues[idx].is_empty() {
                    self.normal_bitmap &= !(1 << idx);
                }
                self.len -= 1;
                return Some(t);
            }
        }
        if self.idle_flag {
            if let Some(t) = self.idle_queue.take_first_where(&pred) {
                if self.idle_queue.is_empty() {
                    self.idle_flag = false;
                }
                self.len -= 1;
                return Some(t);
            }
        }
        None
    }

    /// Remove specific task from queues
    pub fn remove(&mut self, task: NonNull<Task>) -> bool {
        let removed = self.remove_inner(task);
        if removed {
            self.len -= 1;
        }
        removed
    }

    fn remove_inner(&mut self, task: NonNull<Task>) -> bool {
        // SAFETY: `task` is a valid NonNull<Task> provided by the caller
        // (e.g., migrate_task). We read sched_class and priority to find
        // the correct sub-queue for removal. The ReadyQueue Mutex ensures
        // exclusive access.
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
    /// Number of queued tasks
    len: usize,
}

#[cfg(feature = "alloc")]
impl CfsRunQueue {
    /// Create new CFS run queue
    pub fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            min_vruntime: 0,
            total_weight: 0,
            len: 0,
        }
    }

    /// Add task to CFS queue
    pub fn enqueue(&mut self, task: NonNull<Task>) {
        // SAFETY: `task` is a valid NonNull<Task>. We read vruntime and
        // priority to determine the insertion key and weight. The CFS queue
        // is protected by a Mutex ensuring exclusive access.
        unsafe {
            let task_ref = task.as_ref();
            let vruntime = task_ref.vruntime.max(self.min_vruntime);

            self.tasks
                .entry(vruntime)
                .or_default()
                .push(TaskPtr::new(task));

            self.total_weight += priority_to_weight(task_ref.priority);
            self.len += 1;
        }
    }

    /// Remove task with lowest vruntime
    pub fn dequeue(&mut self) -> Option<NonNull<Task>> {
        if let Some(&vruntime) = self.tasks.keys().next() {
            self.min_vruntime = vruntime;

            // Get mutable reference to remove task
            // get_mut cannot fail: vruntime was just retrieved from keys().next()
            let tasks = self
                .tasks
                .get_mut(&vruntime)
                .expect("vruntime key disappeared between lookup and access");
            let task = tasks.pop();

            if tasks.is_empty() {
                self.tasks.remove(&vruntime);
            }

            if let Some(task) = task {
                self.len -= 1;
                // SAFETY: task is a TaskPtr that was stored in the CFS queue.
                // We read its priority to update total_weight. The CFS queue
                // Mutex ensures exclusive access.
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

    /// Remove and return the task with the lowest vruntime for which `pred`
    /// holds, leaving all others where they are.
    pub fn take_first_where(&mut self, pred: impl Fn(&Task) -> bool) -> Option<NonNull<Task>> {
        // Remember the key the task is filed under: enqueue() clamps it to
        // min_vruntime and the task's vruntime may have moved since, so
        // looking it up again by vruntime can miss (review of the v0.26.0
        // stack, PR #11).
        let (key, target) = self
            .tasks
            .iter()
            .flat_map(|(&k, v)| v.iter().rev().map(move |t| (k, t.as_ptr()))) // dequeue() pops from the end
            // SAFETY: queued tasks are valid while queued; `pred` only reads.
            .find(|(_, t)| pred(unsafe { t.as_ref() }))?;
        // As dequeue() does: min_vruntime follows the queue's lowest key.
        if let Some(&lowest) = self.tasks.keys().next() {
            self.min_vruntime = self.min_vruntime.max(lowest);
        }
        let removed = self.remove_at(key, target);
        debug_assert!(removed, "task found by the scan must be under its key");
        Some(target)
    }

    /// Remove specific task
    pub fn remove(&mut self, target: NonNull<Task>) -> bool {
        // SAFETY: `target` is a valid NonNull<Task> provided by the caller.
        // We read vruntime to find its bucket. The CFS queue Mutex ensures
        // exclusive access.
        let vruntime = unsafe { target.as_ref().vruntime };
        if self.remove_at(vruntime, target) {
            return true;
        }
        // The task is filed under the key it was enqueued with, which differs
        // from its vruntime when enqueue() clamped it or when vruntime was
        // charged while it was queued. Fall back to a scan for its bucket.
        let key = self
            .tasks
            .iter()
            .find(|(_, v)| v.iter().any(|t| t.as_ptr() == target))
            .map(|(&k, _)| k);
        match key {
            Some(k) => self.remove_at(k, target),
            None => false,
        }
    }

    /// Remove `target` from the bucket filed under `key`, keeping `len` and
    /// `total_weight` in step. Returns false if it is not in that bucket.
    fn remove_at(&mut self, key: u64, target: NonNull<Task>) -> bool {
        let Some(tasks) = self.tasks.get_mut(&key) else {
            return false;
        };
        let Some(pos) = tasks.iter().position(|&t| t.as_ptr() == target) else {
            return false;
        };
        tasks.remove(pos);
        if tasks.is_empty() {
            self.tasks.remove(&key);
        }
        self.len -= 1;
        // SAFETY: `target` was queued, so it is a valid task; we only read
        // its priority. The CFS queue Mutex ensures exclusive access.
        let priority = unsafe { target.as_ref().priority };
        self.total_weight = self
            .total_weight
            .saturating_sub(priority_to_weight(priority));
        true
    }

    /// Update minimum vruntime
    pub fn update_min_vruntime(&mut self, current_vruntime: u64) {
        self.min_vruntime = self.min_vruntime.max(current_vruntime);
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Number of queued tasks.
    pub fn len(&self) -> usize {
        self.len
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
pub(crate) static READY_QUEUE: Mutex<ReadyQueue> = Mutex::new(ReadyQueue::new());

/// Global ready queue for RISC-V (avoiding spin::Mutex issues)
///
/// SAFETY JUSTIFICATION: This static mut is intentionally kept because:
/// 1. RISC-V early boot cannot use spin::Mutex without deadlocking
/// 2. Const-initialized in BSS (zero-init matches ReadyQueue::new())
/// 3. After boot, accessed only through get_ready_queue()
/// 4. Cannot use OnceLock because it may not be available when the scheduler
///    first needs to run
///
/// NOTE: Previously used Option<Box<ReadyQueue>> with lazy initialization,
/// but Box::new(ReadyQueue::new()) places ~72KB on the stack in debug mode
/// before heap-allocating, which corrupted the bump allocator on RISC-V.
/// Const-initializing directly in BSS avoids all heap/stack issues.
#[cfg(target_arch = "riscv64")]
#[allow(static_mut_refs)]
pub(crate) static mut READY_QUEUE_STATIC: ReadyQueue = ReadyQueue::new();

/// Get the global ready queue (architecture-specific)
#[cfg(target_arch = "riscv64")]
#[allow(static_mut_refs)]
pub fn get_ready_queue() -> &'static mut ReadyQueue {
    // SAFETY: READY_QUEUE_STATIC is a const-initialized static mut in BSS.
    // On RISC-V, we use a static mut instead of spin::Mutex to avoid
    // deadlocks during early bootstrap. Single-hart boot ensures no
    // concurrent access during initialization. The 'static lifetime is
    // valid because the static lives for the kernel's lifetime.
    unsafe { &mut READY_QUEUE_STATIC }
}

#[cfg(all(test, feature = "alloc"))]
pub(crate) mod tests {
    use alloc::{boxed::Box, string::String, vec::Vec};

    use super::*;
    use crate::process::{ProcessId, ThreadId};

    pub(crate) fn task(n: u64) -> NonNull<Task> {
        // Task::new prepares a context on the stack it is given, so give it
        // real (leaked) memory; the task is never run.
        let stack = Box::leak(alloc::vec![0u8; 4096].into_boxed_slice());
        let stack_top = stack.as_ptr() as usize + stack.len();
        let t = Box::new(Task::new(
            ProcessId(n),
            ThreadId(n),
            String::from("t"),
            0,
            stack_top,
            0,
        ));
        NonNull::from(Box::leak(t))
    }

    fn drain(q: &mut PriorityQueue) -> Vec<NonNull<Task>> {
        core::iter::from_fn(|| q.dequeue()).collect()
    }

    #[test]
    fn priority_queue_remove_keeps_order_across_wrap() {
        let tasks: Vec<_> = (0..6).map(task).collect();
        let mut q = PriorityQueue::new();
        // Move head/tail to just before the end of the ring.
        for _ in 0..MAX_TASKS_PER_QUEUE - 2 {
            assert!(q.enqueue(tasks[0]));
            q.dequeue();
        }
        for &t in &tasks {
            assert!(q.enqueue(t));
        }
        assert!(q.remove(tasks[2]));
        assert!(!q.remove(tasks[2]));
        assert_eq!(q.len(), 5);
        assert!(q.enqueue(tasks[2]));
        assert_eq!(
            drain(&mut q),
            [tasks[0], tasks[1], tasks[3], tasks[4], tasks[5], tasks[2]]
        );
    }

    #[test]
    fn boxed_queue_is_empty_and_tracks_len() {
        let mut q = ReadyQueue::new_boxed();
        assert!(q.is_empty());
        assert!(!q.has_ready_tasks());
        assert!(q.dequeue().is_none());
        let (a, b) = (task(1), task(2));
        assert!(q.enqueue(a));
        assert!(q.enqueue(b));
        assert_eq!(q.len(), 2);
        assert!(q.remove(a));
        assert_eq!(q.len(), 1);
        assert_eq!(q.dequeue(), Some(b));
        assert!(q.is_empty());
    }

    fn with_vruntime(n: u64, vruntime: u64) -> NonNull<Task> {
        let mut t = task(n);
        // SAFETY: the task was just leaked by `task` and is unshared.
        unsafe { t.as_mut().vruntime = vruntime };
        t
    }

    #[test]
    fn cfs_take_first_where_removes_clamped_task() {
        let mut q = CfsRunQueue::new();
        q.enqueue(with_vruntime(1, 1000));
        assert!(q.dequeue().is_some()); // min_vruntime = 1000
                                        // Filed under the clamped key 1000, not its own vruntime 0.
        let b = with_vruntime(2, 0);
        q.enqueue(b);
        assert_eq!(q.take_first_where(|_| true), Some(b));
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
        assert_eq!(q.dequeue(), None);

        // remove() finds it too, under the clamped key.
        q.enqueue(b);
        assert!(q.remove(b));
        assert!(!q.remove(b));
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn cfs_remove_after_vruntime_changed_while_queued() {
        let mut q = CfsRunQueue::new();
        let (a, c) = (with_vruntime(1, 10), with_vruntime(3, 30));
        q.enqueue(a);
        q.enqueue(c);
        // The task's vruntime is charged while it sits in the queue.
        // SAFETY: test-owned task; the queue only reads it.
        unsafe { a.as_ptr().as_mut().unwrap().vruntime = 500 };
        assert_eq!(q.take_first_where(|t| t.vruntime == 500), Some(a));
        assert_eq!(q.len(), 1);
        assert_eq!(q.dequeue(), Some(c));
        assert!(q.is_empty());

        q.enqueue(c);
        // SAFETY: as above.
        unsafe { c.as_ptr().as_mut().unwrap().vruntime = 7 };
        assert!(q.remove(c));
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
    }
}
