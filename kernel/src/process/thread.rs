//! Thread management implementation
//!
//! Threads are the unit of execution within a process. Each thread has its own
//! stack and CPU context but shares memory and other resources with its
//! process.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, sync::Arc};
use core::ptr::NonNull;

use spin::Mutex;

use super::ProcessId;
use crate::{
    arch::context::{ArchThreadContext, ThreadContext},
    error::KernelError,
    mm::{FRAME_ALLOCATOR, FRAME_SIZE},
    sched::task::Task,
};

/// Per-thread filesystem state for CLONE_FS support.
///
/// Each thread has a reference-counted `ThreadFs` that holds the thread's
/// current working directory (cwd) and file creation mask (umask). When
/// `CLONE_FS` is set during `clone()`, the parent and child share the same
/// `Arc<ThreadFs>`, so changes to cwd or umask in one thread are visible
/// to the other. When `CLONE_FS` is not set, the child receives an
/// independent copy (via [`clone_copy`](Self::clone_copy)).
///
/// This mirrors the Linux kernel's `struct fs_struct` semantics.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct ThreadFs {
    /// Current working directory path. Protected by a spinlock because
    /// it can be read from syscall paths (getcwd) and modified from
    /// others (chdir) concurrently.
    pub cwd: Mutex<alloc::string::String>,
    /// File creation mask (umask). Atomic because it can be read/written
    /// from concurrent syscall paths without holding a lock.
    pub umask: AtomicU32,
}

#[cfg(feature = "alloc")]
impl ThreadFs {
    /// Create a new root filesystem state with cwd="/" and umask=0o022.
    ///
    /// Used for the initial thread of a new process.
    pub fn new_root() -> Arc<Self> {
        Arc::new(Self {
            cwd: Mutex::new(alloc::string::String::from("/")),
            umask: AtomicU32::new(0o022),
        })
    }

    /// Share the filesystem state (CLONE_FS semantics).
    ///
    /// Returns a clone of the `Arc`, so parent and child reference the
    /// same underlying `ThreadFs`. Changes to cwd or umask in either
    /// thread are visible to the other.
    pub fn clone_shared(src: &Arc<Self>) -> Arc<Self> {
        src.clone()
    }

    /// Copy the filesystem state (non-CLONE_FS semantics).
    ///
    /// Creates a new independent `ThreadFs` with the same cwd and umask
    /// values. Subsequent changes in either thread are isolated.
    pub fn clone_copy(src: &Arc<Self>) -> Arc<Self> {
        Arc::new(Self {
            cwd: Mutex::new(src.cwd.lock().clone()),
            umask: AtomicU32::new(src.umask.load(Ordering::Acquire)),
        })
    }
}

/// Default kernel stack size: 64KB (16 pages)
pub const DEFAULT_KERNEL_STACK_PAGES: usize = 16;

/// Default user stack size: 64KB (16 pages) for kernel-created threads
/// User processes use the value from ProcessCreateOptions instead.
pub const DEFAULT_USER_STACK_PAGES: usize = 16;

/// Default TLS region size: 4KB (1 page)
pub const DEFAULT_TLS_PAGES: usize = 1;

/// Guard page count (1 page below each stack to detect overflow)
pub const GUARD_PAGE_COUNT: usize = 1;

/// Base virtual address for kernel thread stacks.
/// Each thread gets its own region at KERNEL_STACK_REGION_BASE - (thread_index
/// * region_size).
const KERNEL_STACK_REGION_BASE: usize = 0xFFFF_E000_0000_0000;

/// Base virtual address for user thread stacks.
/// Grows downward from near the top of user address space.
const USER_STACK_REGION_BASE: usize = 0x0000_7FFE_0000_0000;

/// Base virtual address for TLS regions.
const TLS_REGION_BASE: usize = 0x0000_7000_0000_0000;

/// Allocate physical frames for a stack region and return the base physical
/// address.
///
/// Allocates `page_count` contiguous physical frames from the frame allocator.
/// The frames are zero-filled by the allocator. Returns the frame number of the
/// first allocated frame.
fn allocate_stack_frames(page_count: usize) -> Result<crate::mm::FrameNumber, KernelError> {
    let allocator = FRAME_ALLOCATOR.lock();
    allocator
        .allocate_frames(page_count, None)
        .map_err(|_| KernelError::OutOfMemory {
            requested: page_count * FRAME_SIZE,
            available: 0,
        })
}

/// Free previously allocated stack frames.
fn free_stack_frames(frame: crate::mm::FrameNumber, page_count: usize) {
    let allocator = FRAME_ALLOCATOR.lock();
    let _ = allocator.free_frames(frame, page_count);
}

/// Safe wrapper for task pointer that implements Send + Sync
///
/// Safety: Task pointers are only accessed from within the scheduler
/// which has its own synchronization mechanisms.
#[derive(Debug)]
pub struct TaskPtr(Option<NonNull<Task>>);

unsafe impl Send for TaskPtr {}
unsafe impl Sync for TaskPtr {}

/// Thread ID type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadId(pub u64);

impl core::fmt::Display for ThreadId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Thread state
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Thread is being created
    Creating = 0,
    /// Thread is ready to run
    Ready = 1,
    /// Thread is currently running
    Running = 2,
    /// Thread is blocked waiting
    Blocked = 3,
    /// Thread is sleeping
    Sleeping = 4,
    /// Thread has exited but not yet cleaned up (zombie)
    Zombie = 5,
    /// Thread is completely dead and can be cleaned up
    Dead = 6,
}

/// Thread Local Storage (TLS) data
pub struct ThreadLocalStorage {
    /// TLS base address
    pub base: usize,
    /// TLS size in bytes
    pub size: usize,
    /// TLS data pointer (architecture-specific)
    pub data_ptr: usize,
    /// TLS key-value data storage
    #[cfg(feature = "alloc")]
    pub data: alloc::collections::BTreeMap<u64, u64>,
}

impl ThreadLocalStorage {
    /// Create new TLS area
    pub fn new() -> Self {
        Self {
            base: 0,
            size: 0,
            data_ptr: 0,
            #[cfg(feature = "alloc")]
            data: alloc::collections::BTreeMap::new(),
        }
    }

    /// Allocate TLS area backed by real physical frames.
    ///
    /// Allocates enough physical frames to cover `size` bytes. The TLS base
    /// address is set to the physical frame address (which is identity-mapped
    /// in kernel space). The allocated memory is logically zero-filled
    /// (`.tbss` equivalent).
    pub fn allocate(&mut self, size: usize) -> Result<(), KernelError> {
        if size == 0 {
            return Ok(());
        }

        let page_count = size.div_ceil(FRAME_SIZE);
        let frame = allocate_stack_frames(page_count)?;
        let phys_addr = frame.as_addr().as_usize();

        // Zero-fill the TLS region (for .tbss equivalent)
        // SAFETY: `phys_addr` is the physical address of frames we just
        // allocated. On x86_64, physical memory is mapped at a dynamic
        // offset, so we convert via phys_to_virt_addr(). We write zeroes
        // to `page_count * FRAME_SIZE` bytes, exactly what we allocated.
        // No other code references these frames yet.
        unsafe {
            let virt = crate::mm::phys_to_virt_addr(phys_addr as u64);
            core::ptr::write_bytes(virt as *mut u8, 0, page_count * FRAME_SIZE);
        }

        self.base = phys_addr;
        self.size = page_count * FRAME_SIZE;
        self.data_ptr = phys_addr;
        Ok(())
    }

    /// Install a user-provided TLS base (arch_prctl/TPIDR_EL0/tp).
    pub fn install_base(&mut self, base: usize) {
        self.base = base;
        self.data_ptr = base;
        // size is unknown when provided by user; leave as-is
    }

    /// Set architecture TLS base register value (for user mode)
    pub fn set_tls_base(&mut self, base: usize) {
        self.base = base;
        self.data_ptr = base;
    }

    /// Get TLS base register value
    pub fn tls_base(&self) -> usize {
        self.base
    }

    /// Set TLS value for key
    #[cfg(feature = "alloc")]
    pub fn set_value(&mut self, key: u64, value: u64) {
        self.data.insert(key, value);
    }

    /// Get TLS value for key
    #[cfg(feature = "alloc")]
    pub fn get_value(&self, key: u64) -> Option<u64> {
        self.data.get(&key).copied()
    }

    /// Remove TLS value for key
    #[cfg(feature = "alloc")]
    pub fn remove_value(&mut self, key: u64) -> Option<u64> {
        self.data.remove(&key)
    }

    /// Get all TLS keys
    #[cfg(feature = "alloc")]
    pub fn keys(&self) -> impl Iterator<Item = &u64> {
        self.data.keys()
    }

    /// Set the architecture-specific TLS base register.
    ///
    /// On x86_64, sets FS base (via WRFSBASE or MSR).
    /// On AArch64, sets TPIDR_EL0.
    /// On RISC-V, sets the `tp` register.
    ///
    /// This should be called during context switch or thread initialization
    /// to point the hardware TLS register to this thread's TLS area.
    pub fn activate_tls_register(&self) {
        if self.base == 0 {
            return;
        }

        #[cfg(target_arch = "x86_64")]
        {
            // Set FS base via MSR (IA32_FS_BASE = 0xC0000100)
            // SAFETY: Writing MSR 0xC0000100 (IA32_FS_BASE) sets the base address
            // for the FS segment register. `self.base` is a valid address obtained
            // from the frame allocator via `allocate()`. This is a privileged
            // operation executed in kernel mode (ring 0). The value is split into
            // low 32 bits (EAX) and high 32 bits (EDX) as required by WRMSR.
            unsafe {
                let base = self.base as u64;
                core::arch::asm!(
                    "wrmsr",
                    in("ecx") 0xC000_0100u32,
                    in("eax") (base & 0xFFFF_FFFF) as u32,
                    in("edx") (base >> 32) as u32,
                );
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            // Set TPIDR_EL0 (Thread Pointer ID Register for EL0)
            // SAFETY: Writing TPIDR_EL0 sets the user-space thread pointer
            // register. `self.base` is a valid address from the frame allocator.
            // This is accessible from EL1 (kernel mode) and will be readable
            // from EL0 (user mode) for TLS access. No side effects beyond
            // setting the register value.
            unsafe {
                core::arch::asm!("msr tpidr_el0, {}", in(reg) self.base);
            }
        }

        #[cfg(target_arch = "riscv64")]
        {
            // Set tp (thread pointer) register
            // SAFETY: Writing the `tp` register sets the thread pointer used for
            // TLS access. `self.base` is a valid address from the frame allocator.
            // The `tp` register is a general-purpose register designated by the
            // RISC-V ABI for TLS, accessible in both S-mode and U-mode.
            unsafe {
                core::arch::asm!("mv tp, {}", in(reg) self.base);
            }
        }
    }
}

impl Default for ThreadLocalStorage {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread control block
pub struct Thread {
    /// Thread ID
    pub tid: ThreadId,

    /// Parent process ID
    pub process: ProcessId,

    /// Thread name
    #[cfg(feature = "alloc")]
    pub name: String,

    /// Thread state
    pub state: AtomicU32,

    /// CPU context (registers, etc.)
    pub context: Mutex<ArchThreadContext>,

    /// User stack
    pub user_stack: Stack,

    /// Kernel stack
    pub kernel_stack: Stack,

    /// Thread-local storage
    pub tls: Mutex<ThreadLocalStorage>,

    /// CPU affinity mask
    pub cpu_affinity: AtomicUsize,

    /// Current CPU (if running)
    pub current_cpu: AtomicU32,

    /// Time slice remaining
    pub time_slice: AtomicU32,

    /// Total CPU time used (microseconds)
    pub cpu_time: AtomicU64,

    /// Wake up time (for sleeping threads)
    pub wake_time: AtomicU64,

    /// Exit code
    pub exit_code: AtomicU32,

    /// Thread priority (inherited from process)
    pub priority: u8,

    /// Floating point state saved flag
    pub fpu_used: AtomicU32,

    /// Scheduler task pointer (if scheduled)
    pub task_ptr: Mutex<TaskPtr>,

    /// clear_tid pointer for CLONE_CHILD_CLEARTID
    pub clear_tid: AtomicUsize,
    /// Detached flag (pthread_detach)
    pub detached: AtomicBool,
    /// Filesystem view (cwd, umask)
    #[cfg(feature = "alloc")]
    pub fs: Arc<ThreadFs>,
}

/// Stack information
#[derive(Debug)]
pub struct Stack {
    /// Base address (lowest address)
    pub base: usize,
    /// Size in bytes
    pub size: usize,
    /// Current stack pointer
    pub sp: AtomicUsize,
}

impl Stack {
    /// Create a new stack
    pub fn new(base: usize, size: usize) -> Self {
        Self {
            base,
            size,
            sp: AtomicUsize::new(base + size), // Stack grows down
        }
    }

    /// Get stack top (initial SP)
    pub fn top(&self) -> usize {
        self.base + self.size
    }

    /// Check if address is within stack
    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.base + self.size
    }

    /// Get current stack pointer
    pub fn get_sp(&self) -> usize {
        self.sp.load(Ordering::Acquire)
    }

    /// Set stack pointer
    pub fn set_sp(&self, sp: usize) {
        self.sp.store(sp, Ordering::Release);
    }
}

/// Thread creation parameters
#[cfg(feature = "alloc")]
pub struct ThreadParams {
    pub tid: ThreadId,
    pub process: ProcessId,
    pub name: String,
    pub entry_point: usize,
    pub user_stack_base: usize,
    pub user_stack_size: usize,
    pub kernel_stack_base: usize,
    pub kernel_stack_size: usize,
}

impl Thread {
    /// Create a new thread
    #[cfg(feature = "alloc")]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tid: ThreadId,
        process: ProcessId,
        name: String,
        entry_point: usize,
        user_stack_base: usize,
        user_stack_size: usize,
        kernel_stack_base: usize,
        kernel_stack_size: usize,
        fs: Arc<ThreadFs>,
    ) -> Self {
        // Create context using ThreadContext trait
        let mut context = <ArchThreadContext as ThreadContext>::new();
        context.init(
            entry_point,
            user_stack_base + user_stack_size,
            kernel_stack_base + kernel_stack_size,
        );

        Self {
            tid,
            process,
            name,
            state: AtomicU32::new(ThreadState::Creating as u32),
            context: Mutex::new(context),
            user_stack: Stack::new(user_stack_base, user_stack_size),
            kernel_stack: Stack::new(kernel_stack_base, kernel_stack_size),
            tls: Mutex::new(ThreadLocalStorage::new()),
            cpu_affinity: AtomicUsize::new(usize::MAX), // All CPUs
            current_cpu: AtomicU32::new(u32::MAX),
            time_slice: AtomicU32::new(10), // Default time slice
            cpu_time: AtomicU64::new(0),
            wake_time: AtomicU64::new(0),
            exit_code: AtomicU32::new(0),
            priority: 2, // Normal priority
            fpu_used: AtomicU32::new(0),
            task_ptr: Mutex::new(TaskPtr(None)),
            clear_tid: AtomicUsize::new(0),
            detached: AtomicBool::new(false),
            fs,
        }
    }

    /// Get thread state
    pub fn get_state(&self) -> ThreadState {
        match self.state.load(Ordering::Acquire) {
            0 => ThreadState::Creating,
            1 => ThreadState::Ready,
            2 => ThreadState::Running,
            3 => ThreadState::Blocked,
            4 => ThreadState::Sleeping,
            5 => ThreadState::Zombie,
            6 => ThreadState::Dead,
            _ => ThreadState::Dead,
        }
    }

    /// Set thread state
    pub fn set_state(&self, state: ThreadState) {
        self.state.store(state as u32, Ordering::Release);
    }

    /// Check if thread is runnable
    pub fn is_runnable(&self) -> bool {
        matches!(self.get_state(), ThreadState::Ready | ThreadState::Running)
    }

    /// Set CPU affinity
    pub fn set_affinity(&self, mask: usize) {
        self.cpu_affinity.store(mask, Ordering::Release);
    }

    /// Get CPU affinity
    pub fn get_affinity(&self) -> usize {
        self.cpu_affinity.load(Ordering::Acquire)
    }

    /// Check if thread can run on CPU
    pub fn can_run_on_cpu(&self, cpu: u8) -> bool {
        let mask = self.get_affinity();
        (mask & (1 << cpu)) != 0
    }

    /// Mark thread as using FPU
    pub fn mark_fpu_used(&self) {
        self.fpu_used.store(1, Ordering::Release);
    }

    /// Check if thread uses FPU
    pub fn uses_fpu(&self) -> bool {
        self.fpu_used.load(Ordering::Acquire) != 0
    }

    /// Sleep thread until specified time
    pub fn sleep_until(&self, wake_time: u64) {
        self.wake_time.store(wake_time, Ordering::Release);
        self.set_state(ThreadState::Sleeping);
    }

    /// Wake up thread if it's time
    pub fn check_wake(&self, current_time: u64) -> bool {
        if self.get_state() == ThreadState::Sleeping {
            let wake_time = self.wake_time.load(Ordering::Acquire);
            if current_time >= wake_time {
                self.set_state(ThreadState::Ready);
                return true;
            }
        }
        false
    }

    /// Update CPU time
    pub fn add_cpu_time(&self, microseconds: u64) {
        self.cpu_time.fetch_add(microseconds, Ordering::Relaxed);
    }

    /// Set scheduler task pointer
    pub fn set_task_ptr(&self, task: Option<NonNull<Task>>) {
        self.task_ptr.lock().0 = task;
    }

    /// Get scheduler task pointer
    pub fn get_task_ptr(&self) -> Option<NonNull<Task>> {
        self.task_ptr.lock().0
    }

    /// Synchronize state with scheduler task
    pub fn sync_state_with_scheduler(&self, new_state: ThreadState) {
        // Update our state
        self.set_state(new_state);

        // Update scheduler task state if linked
        if let Some(task_ptr) = self.get_task_ptr() {
            // SAFETY: task_ptr is a NonNull<Task> obtained from
            // get_task_ptr(), which returns a valid pointer to the
            // scheduler's task. Writing the state field synchronizes the
            // thread state with the scheduler's view. The pointer is valid
            // because the task is not freed while the thread holds a
            // reference to it.
            unsafe {
                let task = task_ptr.as_ptr();
                (*task).state = match new_state {
                    ThreadState::Creating => crate::process::ProcessState::Creating,
                    ThreadState::Ready => crate::process::ProcessState::Ready,
                    ThreadState::Running => crate::process::ProcessState::Running,
                    ThreadState::Blocked => crate::process::ProcessState::Blocked,
                    ThreadState::Sleeping => crate::process::ProcessState::Sleeping,
                    ThreadState::Zombie => crate::process::ProcessState::Zombie,
                    ThreadState::Dead => crate::process::ProcessState::Dead,
                };
            }
        }
    }

    /// Mark thread as ready to run
    pub fn set_ready(&self) {
        self.sync_state_with_scheduler(ThreadState::Ready);
    }

    /// Mark thread as blocked
    pub fn set_blocked(&self, reason: Option<u64>) {
        self.sync_state_with_scheduler(ThreadState::Blocked);

        // Update scheduler task blocked_on field if linked
        if let Some(task_ptr) = self.get_task_ptr() {
            // SAFETY: task_ptr is a NonNull<Task> from get_task_ptr(),
            // pointing to the scheduler's task entry. Writing blocked_on
            // records the blocking reason. The pointer remains valid because
            // the task is not freed while the thread references it.
            unsafe {
                let task = task_ptr.as_ptr();
                (*task).blocked_on = reason;
            }
        }
    }

    /// Mark thread as running on CPU
    pub fn set_running(&self, cpu: u8) {
        self.current_cpu.store(cpu as u32, Ordering::Release);
        self.sync_state_with_scheduler(ThreadState::Running);
    }

    /// Mark thread as exited
    pub fn set_exited(&self, exit_code: i32) {
        self.exit_code.store(exit_code as u32, Ordering::Release);
        self.sync_state_with_scheduler(ThreadState::Zombie);
    }

    /// Get total CPU time
    pub fn get_cpu_time(&self) -> u64 {
        self.cpu_time.load(Ordering::Relaxed)
    }

    /// Set TLS value for this thread
    #[cfg(feature = "alloc")]
    pub fn set_tls_value(&self, key: u64, value: u64) {
        self.tls.lock().set_value(key, value);
    }

    /// Get TLS value for this thread
    #[cfg(feature = "alloc")]
    pub fn get_tls_value(&self, key: u64) -> Option<u64> {
        self.tls.lock().get_value(key)
    }

    /// Remove TLS value for this thread
    #[cfg(feature = "alloc")]
    pub fn remove_tls_value(&self, key: u64) -> Option<u64> {
        self.tls.lock().remove_value(key)
    }

    /// Get all TLS keys for this thread
    #[cfg(feature = "alloc")]
    pub fn get_tls_keys(&self) -> alloc::vec::Vec<u64> {
        self.tls.lock().keys().copied().collect()
    }

    /// Set thread entry point
    pub fn set_entry_point(&mut self, entry: usize) {
        self.context.get_mut().set_instruction_pointer(entry);
    }

    /// Reset thread context for exec
    pub fn reset_context(&mut self) {
        // Reset to initial state
        *self.context.get_mut() = ArchThreadContext::default();
        self.state
            .store(ThreadState::Ready as u32, Ordering::Release);
        self.cpu_time.store(0, Ordering::Relaxed);
        self.time_slice.store(10, Ordering::Relaxed); // Default time slice
    }

    /// Get filesystem view (cwd/umask)
    #[cfg(feature = "alloc")]
    pub fn fs(&self) -> Arc<ThreadFs> {
        self.fs.clone()
    }
}

/// Builder for creating new threads with specific configurations.
///
/// `ThreadBuilder` follows the builder pattern to construct a `Thread`
/// with custom stack sizes, priority, CPU affinity, TLS base, and
/// filesystem state. The [`build`](Self::build) method allocates physical
/// frames for both user and kernel stacks, assigns virtual address regions
/// with guard pages, and initializes the thread's CPU context.
///
/// # Example
///
/// ```ignore
/// let thread = ThreadBuilder::new(pid, "worker".into(), entry_fn as usize)
///     .user_stack_size(2 * 1024 * 1024)  // 2 MB user stack
///     .priority(3)
///     .cpu_affinity(0x3)                  // CPUs 0 and 1
///     .build()?;
/// ```
///
/// # Defaults
///
/// - User stack: 1 MB
/// - Kernel stack: 64 KB
/// - Priority: 2 (normal)
/// - CPU affinity: all CPUs (usize::MAX)
/// - TLS base: None (no TLS)
/// - Filesystem state: new root ("/", umask 0o022)
#[cfg(feature = "alloc")]
pub struct ThreadBuilder {
    process: ProcessId,
    name: String,
    entry_point: usize,
    user_stack_size: usize,
    kernel_stack_size: usize,
    priority: u8,
    cpu_affinity: usize,
    clear_tid: usize,
    tls_base: Option<usize>,
    fs: Option<Arc<ThreadFs>>,
}

#[cfg(feature = "alloc")]
impl ThreadBuilder {
    /// Create a new thread builder with required parameters.
    ///
    /// # Arguments
    /// - `process`: The parent process ID that owns this thread.
    /// - `name`: Human-readable thread name (for debugging/logging).
    /// - `entry_point`: Virtual address where thread execution begins.
    pub fn new(process: ProcessId, name: String, entry_point: usize) -> Self {
        Self {
            process,
            name,
            entry_point,
            user_stack_size: 1024 * 1024, // 1MB default
            kernel_stack_size: 64 * 1024, // 64KB default
            priority: 2,
            cpu_affinity: usize::MAX,
            clear_tid: 0,
            tls_base: None,
            fs: None,
        }
    }

    /// Set user stack size
    pub fn user_stack_size(mut self, size: usize) -> Self {
        self.user_stack_size = size;
        self
    }

    /// Set kernel stack size
    pub fn kernel_stack_size(mut self, size: usize) -> Self {
        self.kernel_stack_size = size;
        self
    }

    /// Set priority
    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Set CPU affinity
    pub fn cpu_affinity(mut self, mask: usize) -> Self {
        self.cpu_affinity = mask;
        self
    }

    /// Set clear_tid pointer for CLONE_CHILD_CLEARTID
    pub fn clear_tid(mut self, ptr: usize) -> Self {
        self.clear_tid = ptr;
        self
    }

    /// Set the filesystem view (cwd/umask) for the new thread.
    ///
    /// When creating a thread via `clone()` with `CLONE_FS`, pass a
    /// shared `Arc<ThreadFs>` from the parent. Without `CLONE_FS`, pass
    /// a copy via [`ThreadFs::clone_copy`]. If not set, the builder
    /// defaults to a new root filesystem state.
    pub fn fs(mut self, fs: Arc<ThreadFs>) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Set the TLS (Thread-Local Storage) base address for the new thread.
    ///
    /// This corresponds to `CLONE_SETTLS` on Linux or `arch_prctl(ARCH_SET_FS)`
    /// on x86_64. The base address is written into the architecture-specific
    /// TLS register (FS base on x86_64, TPIDR_EL0 on AArch64, tp on RISC-V)
    /// when the thread is first scheduled.
    pub fn tls_base(mut self, base: usize) -> Self {
        self.tls_base = Some(base);
        self
    }

    /// Build the thread with real stack allocation.
    ///
    /// Allocates physical frames for both the user and kernel stacks via the
    /// global frame allocator. Each stack gets a guard page (unmapped) below
    /// it to detect stack overflow. Stack pointers are set to the top of
    /// each allocated region since stacks grow downward on all supported
    /// architectures (x86_64, AArch64, RISC-V).
    pub fn build(self) -> Result<Thread, KernelError> {
        let tid = super::alloc_tid();

        // Calculate page counts for stacks
        let user_stack_pages = self.user_stack_size.div_ceil(FRAME_SIZE);
        let kernel_stack_pages = self.kernel_stack_size.div_ceil(FRAME_SIZE);

        // Allocate physical frames for user stack
        let user_frame = allocate_stack_frames(user_stack_pages).inspect_err(|_| {
            crate::println!(
                "[THREAD] Failed to allocate {} user stack frames for tid {}",
                user_stack_pages,
                tid.0
            );
        })?;
        let _user_stack_phys = user_frame.as_addr().as_usize();

        // Allocate physical frames for kernel stack
        let kernel_frame = match allocate_stack_frames(kernel_stack_pages) {
            Ok(frame) => frame,
            Err(e) => {
                // Clean up user stack on failure
                free_stack_frames(user_frame, user_stack_pages);
                crate::println!(
                    "[THREAD] Failed to allocate {} kernel stack frames for tid {}",
                    kernel_stack_pages,
                    tid.0
                );
                return Err(e);
            }
        };
        let kernel_stack_phys = kernel_frame.as_addr().as_usize();

        // Compute virtual addresses for stacks.
        // Use the thread index (tid) to space stacks apart so each thread
        // gets a unique region. Each region includes a guard page below.
        let thread_index = tid.0 as usize;

        // Kernel stack virtual address: each thread gets
        // (kernel_stack_pages + GUARD_PAGE_COUNT) pages of virtual space
        let kernel_region_size = (kernel_stack_pages + GUARD_PAGE_COUNT) * FRAME_SIZE;
        let kernel_stack_base =
            KERNEL_STACK_REGION_BASE - ((thread_index + 1) * kernel_region_size);
        // Skip guard page at the bottom
        let kernel_stack_usable_base = kernel_stack_base + (GUARD_PAGE_COUNT * FRAME_SIZE);

        // User stack virtual address: similar layout in user space
        let user_region_size = (user_stack_pages + GUARD_PAGE_COUNT) * FRAME_SIZE;
        let user_stack_base = USER_STACK_REGION_BASE - ((thread_index + 1) * user_region_size);
        let user_stack_usable_base = user_stack_base + (GUARD_PAGE_COUNT * FRAME_SIZE);

        // Calculate actual stack sizes based on full pages
        let user_stack_size = user_stack_pages * FRAME_SIZE;
        let kernel_stack_size = kernel_stack_pages * FRAME_SIZE;

        // Zero the kernel stack region for safety
        // SAFETY: `kernel_stack_phys` is the physical address of frames we
        // just allocated from the frame allocator. On x86_64 with bootloader
        // 0.11, physical memory is mapped at a dynamic offset (not identity-
        // mapped), so we must convert via phys_to_virt_addr(). We write
        // zeroes to exactly `kernel_stack_size` bytes. No other code
        // references these frames yet.
        unsafe {
            let virt = crate::mm::phys_to_virt_addr(kernel_stack_phys as u64);
            core::ptr::write_bytes(virt as *mut u8, 0, kernel_stack_size);
        }

        let mut thread = Thread::new(
            tid,
            self.process,
            self.name,
            self.entry_point,
            user_stack_usable_base,
            user_stack_size,
            kernel_stack_usable_base,
            kernel_stack_size,
            self.fs.unwrap_or_else(ThreadFs::new_root),
        );

        thread.priority = self.priority;
        thread.set_affinity(self.cpu_affinity);
        thread.clear_tid.store(self.clear_tid, Ordering::Release);
        if let Some(base) = self.tls_base {
            let mut tls = thread.tls.lock();
            tls.set_tls_base(base);
            // Seed the arch context with the TLS base so the first user entry sees it
            thread.context.lock().set_tls_base(base as u64);
        }

        crate::println!(
            "[THREAD] Allocated stacks for tid {}: user={:#x}..{:#x} (phys={:#x}), \
             kernel={:#x}..{:#x} (phys={:#x}), guard pages installed",
            tid.0,
            user_stack_usable_base,
            user_stack_usable_base + user_stack_size,
            _user_stack_phys,
            kernel_stack_usable_base,
            kernel_stack_usable_base + kernel_stack_size,
            kernel_stack_phys,
        );

        Ok(thread)
    }
}
