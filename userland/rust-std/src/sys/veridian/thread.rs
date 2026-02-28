//! Thread operations for VeridianOS.
//!
//! Provides both low-level syscall wrappers and higher-level types:
//!
//! - Low-level: `clone`, `thread_exit`, `gettid`, `futex_wait`, `futex_wake`
//! - High-level: `Thread` (spawn/join/sleep/park/unpark)
//!
//! Syscall mappings:
//! - `clone`       -> SYS_THREAD_CLONE (46)
//! - `thread_exit` -> SYS_THREAD_EXIT (41)
//! - `gettid`      -> SYS_THREAD_GETTID (43)
//! - `futex_wait`  -> SYS_FUTEX_WAIT (201)
//! - `futex_wake`  -> SYS_FUTEX_WAKE (202)
//! - `nanosleep`   -> SYS_NANOSLEEP (162)

extern crate alloc;
use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use super::{
    syscall0, syscall1, syscall3, syscall5, syscall_result, time::Timespec, SyscallError,
    SYS_FUTEX_WAIT, SYS_FUTEX_WAKE, SYS_NANOSLEEP, SYS_THREAD_CLONE, SYS_THREAD_EXIT,
    SYS_THREAD_GETTID,
};

// ============================================================================
// Clone flags (must match kernel definitions)
// ============================================================================

/// Share virtual memory with parent.
pub const CLONE_VM: usize = 0x0000_0100;
/// Share filesystem info.
pub const CLONE_FS: usize = 0x0000_0200;
/// Share file descriptors.
pub const CLONE_FILES: usize = 0x0000_0400;
/// Share signal handlers.
pub const CLONE_SIGHAND: usize = 0x0000_0800;
/// Same thread group as parent.
pub const CLONE_THREAD: usize = 0x0001_0000;
/// Set TLS for new thread.
pub const CLONE_SETTLS: usize = 0x0008_0000;
/// Store child TID at parent_tidptr in parent.
pub const CLONE_PARENT_SETTID: usize = 0x0010_0000;
/// Clear child TID at child_tidptr on exit (for futex wake).
pub const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
/// Store child TID at child_tidptr in child.
pub const CLONE_CHILD_SETTID: usize = 0x0100_0000;

/// Default flags for creating a pthread-like thread.
pub const THREAD_FLAGS: usize =
    CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SETTLS;

// ============================================================================
// Futex operations
// ============================================================================

/// Futex wait: sleep if `*uaddr == expected`.
pub const FUTEX_WAIT: usize = 0;
/// Futex wake: wake up to `count` waiters on `uaddr`.
pub const FUTEX_WAKE: usize = 1;

// ============================================================================
// Low-level syscall wrappers
// ============================================================================

/// Create a new thread via clone.
pub fn clone(
    flags: usize,
    stack: *mut u8,
    parent_tidptr: *mut i32,
    child_tidptr: *mut i32,
    tls: usize,
) -> Result<usize, SyscallError> {
    let ret = unsafe {
        syscall5(
            SYS_THREAD_CLONE,
            flags,
            stack as usize,
            parent_tidptr as usize,
            child_tidptr as usize,
            tls,
        )
    };
    syscall_result(ret)
}

/// Exit the current thread.
pub fn thread_exit(status: usize) -> ! {
    unsafe {
        syscall1(SYS_THREAD_EXIT, status);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Get the current thread ID.
pub fn gettid() -> usize {
    unsafe { syscall0(SYS_THREAD_GETTID) as usize }
}

/// Futex wait: block the calling thread if `*uaddr == expected`.
pub fn futex_wait(
    uaddr: *const i32,
    expected: i32,
    timeout_ms: u64,
) -> Result<usize, SyscallError> {
    let timeout_ptr = if timeout_ms > 0 {
        &timeout_ms as *const u64 as usize
    } else {
        0
    };
    let timeout_size = if timeout_ms > 0 {
        core::mem::size_of::<u64>()
    } else {
        0
    };
    let ret = unsafe {
        syscall5(
            SYS_FUTEX_WAIT,
            uaddr as usize,
            expected as usize,
            timeout_ptr,
            timeout_size,
            0,
        )
    };
    syscall_result(ret)
}

/// Futex wake: wake up to `count` threads waiting on `uaddr`.
pub fn futex_wake(uaddr: *const i32, count: i32) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_FUTEX_WAKE, uaddr as usize, count as usize, 0) };
    syscall_result(ret)
}

// ============================================================================
// Thread stack allocation
// ============================================================================

/// Default thread stack size: 2 MB.
const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024;

/// Allocate a stack for a new thread.
///
/// Returns `(stack_bottom, stack_top)`.  The stack grows downward on x86_64,
/// so `stack_top` is passed to `clone`.
fn alloc_stack(size: usize) -> Result<(*mut u8, *mut u8), SyscallError> {
    use super::alloc::{MAP_ANONYMOUS, MAP_PRIVATE, PROT_READ, PROT_WRITE};
    let addr = super::alloc::mmap(
        0,
        size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )?;
    let bottom = addr as *mut u8;
    let top = unsafe { bottom.add(size) };
    Ok((bottom, top))
}

/// Free a thread stack.
fn free_stack(bottom: *mut u8, size: usize) {
    let _ = super::alloc::munmap(bottom as usize, size);
}

// ============================================================================
// Thread (high-level)
// ============================================================================

/// State shared between the spawning thread and the spawned thread.
pub(crate) struct ThreadInner {
    /// Thread ID (set by clone or by the child).
    tid: AtomicI32,
    /// Futex word for join: 0 = running, 1 = finished.
    done: AtomicI32,
    /// Stack bottom pointer (for deallocation).
    stack_bottom: *mut u8,
    /// Stack size.
    stack_size: usize,
    /// Park/unpark futex: 0 = default, 1 = unparked (token available).
    park_futex: AtomicI32,
}

// SAFETY: The thread inner is protected by atomic operations and is only
// accessed after the Arc keeps it alive.
unsafe impl Send for ThreadInner {}
unsafe impl Sync for ThreadInner {}

/// A handle to a spawned thread.
pub struct Thread {
    inner: Arc<ThreadInner>,
}

impl Thread {
    /// Spawn a new thread that runs `f`.
    ///
    /// The thread starts immediately and runs `f()` to completion, then
    /// exits.  Use `join()` to wait for it.
    pub fn spawn<F>(f: F) -> Result<Thread, SyscallError>
    where
        F: FnOnce() + Send + 'static,
    {
        Self::spawn_with_stack(DEFAULT_STACK_SIZE, f)
    }

    /// Spawn a new thread with a specific stack size.
    pub fn spawn_with_stack<F>(stack_size: usize, f: F) -> Result<Thread, SyscallError>
    where
        F: FnOnce() + Send + 'static,
    {
        let (stack_bottom, stack_top) = alloc_stack(stack_size)?;

        let inner = Arc::new(ThreadInner {
            tid: AtomicI32::new(0),
            done: AtomicI32::new(0),
            stack_bottom,
            stack_size,
            park_futex: AtomicI32::new(0),
        });

        // Pack the closure and Arc into a heap allocation that the child
        // thread will consume.
        struct ThreadStart<F> {
            f: F,
            inner: Arc<ThreadInner>,
        }

        let start = Box::into_raw(Box::new(ThreadStart {
            f,
            inner: inner.clone(),
        }));

        // Trampoline function called by the new thread.
        extern "C" fn trampoline<F: FnOnce() + Send + 'static>(arg: *mut u8) -> ! {
            // SAFETY: `arg` is a valid pointer to a `ThreadStart<F>` that
            // we exclusively own.
            let start = unsafe { Box::from_raw(arg as *mut ThreadStart<F>) };
            start.inner.tid.store(gettid() as i32, Ordering::Release);
            (start.f)();
            // Mark as done and wake any joiner.
            start.inner.done.store(1, Ordering::Release);
            let _ = futex_wake(&start.inner.done as *const AtomicI32 as *const i32, 1);
            thread_exit(0);
        }

        // We need to place the trampoline's argument (pointer to start) and
        // the entry point at the top of the stack.  The clone syscall on
        // x86_64 jumps to the child with rsp = stack and rdi = arg (if we
        // use CLONE_SETTLS to pass fn ptr via TLS, or via a wrapper).
        //
        // For VeridianOS, SYS_THREAD_CLONE takes:
        //   arg1 = flags
        //   arg2 = stack top
        //   arg3 = parent_tidptr
        //   arg4 = child_tidptr
        //   arg5 = tls
        //
        // The kernel starts the child at the function pointer stored as the
        // first item on the stack (or via entry point register, depending on
        // kernel implementation).  We set up the stack so the first return
        // address is our trampoline, and the argument is in TLS / on stack.
        //
        // Simplified approach: we put the trampoline as the entry point
        // and `start` as the argument.  VeridianOS clone takes the entry
        // function and argument similarly to pthread_create.

        let flags = THREAD_FLAGS | CLONE_CHILD_CLEARTID;
        let child_tidptr = &inner.tid as *const AtomicI32 as *mut i32;

        // Store trampoline info at top of stack (16-byte aligned).
        // The kernel will pop the entry point and argument from the stack,
        // or we rely on the clone calling convention to pass them.
        // For now, we use the stack-based convention:
        //   [stack_top - 16] = trampoline function pointer
        //   [stack_top - 8]  = argument (start pointer)
        let stack_words = stack_top as *mut usize;
        unsafe {
            // Set up the initial stack frame.
            let sp = stack_words.sub(2);
            *sp = trampoline::<F> as *const () as usize;
            *sp.add(1) = start as usize;

            match clone(flags, sp as *mut u8, core::ptr::null_mut(), child_tidptr, 0) {
                Ok(_tid) => Ok(Thread { inner }),
                Err(e) => {
                    // Clean up on failure.
                    drop(Box::from_raw(start));
                    free_stack(stack_bottom, stack_size);
                    Err(e)
                }
            }
        }
    }

    /// Wait for the thread to finish.
    pub fn join(self) -> Result<(), SyscallError> {
        loop {
            let val = self.inner.done.load(Ordering::Acquire);
            if val == 1 {
                break;
            }
            // Wait until done flag changes from 0.
            let _ = futex_wait(
                &self.inner.done as *const AtomicI32 as *const i32,
                0,
                0, // no timeout
            );
        }
        // Free the stack now that the thread is done.
        free_stack(self.inner.stack_bottom, self.inner.stack_size);
        Ok(())
    }

    /// Get the thread ID.
    pub fn id(&self) -> i32 {
        self.inner.tid.load(Ordering::Acquire)
    }

    /// Park the current thread until `unpark()` is called.
    ///
    /// If `unpark()` was called before `park()`, this returns immediately
    /// (consuming the token).
    ///
    /// # Safety
    /// The caller must provide the `ThreadInner` belonging to the current
    /// thread.
    pub(crate) fn park(inner: &Arc<ThreadInner>) {
        // Try to consume an existing token.
        if inner
            .park_futex
            .compare_exchange(1, 0, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
        // Wait for a token.
        let _ = futex_wait(&inner.park_futex as *const AtomicI32 as *const i32, 0, 0);
        // Consume the token.
        inner.park_futex.store(0, Ordering::Relaxed);
    }

    /// Unpark the thread, allowing a pending or future `park()` to return.
    pub fn unpark(&self) {
        self.inner.park_futex.store(1, Ordering::Release);
        let _ = futex_wake(&self.inner.park_futex as *const AtomicI32 as *const i32, 1);
    }

    /// Sleep the current thread for the given duration.
    pub fn sleep(secs: u64, nanos: u64) {
        let req = Timespec {
            tv_sec: secs as i64,
            tv_nsec: nanos as i64,
        };
        unsafe {
            let _ = super::syscall2(
                SYS_NANOSLEEP,
                &req as *const Timespec as usize,
                0, // rem = NULL
            );
        }
    }

    /// Sleep for the given number of milliseconds.
    pub fn sleep_ms(ms: u64) {
        Self::sleep(ms / 1000, (ms % 1000) * 1_000_000);
    }

    /// Yield the current thread's timeslice.
    pub fn yield_now() {
        let _ = super::process::sched_yield();
    }
}

impl core::fmt::Debug for Thread {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Thread").field("tid", &self.id()).finish()
    }
}

// ============================================================================
// Mutex (futex-based)
// ============================================================================

/// A mutual exclusion primitive based on futex.
///
/// State encoding:
/// - 0: unlocked
/// - 1: locked, no waiters
/// - 2: locked, with waiters
#[derive(Debug)]
pub struct Mutex {
    state: AtomicI32,
}

impl Mutex {
    /// Create a new unlocked mutex.
    pub const fn new() -> Self {
        Mutex {
            state: AtomicI32::new(0),
        }
    }

    /// Acquire the mutex, blocking if necessary.
    pub fn lock(&self) {
        // Fast path: try to acquire an unlocked mutex.
        if self
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
        self.lock_contended();
    }

    /// Slow path for contended lock acquisition.
    #[cold]
    fn lock_contended(&self) {
        loop {
            // If the state is 0, try to grab it.
            if self
                .state
                .compare_exchange(0, 2, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }

            // Set state to 2 (locked with waiters) so unlock knows to wake.
            let prev = self.state.swap(2, Ordering::Acquire);
            if prev == 0 {
                // We got the lock while setting waiters flag.
                return;
            }

            // Block on the futex until the state changes from 2.
            let _ = futex_wait(&self.state as *const AtomicI32 as *const i32, 2, 0);
        }
    }

    /// Try to acquire the mutex without blocking.
    ///
    /// Returns `true` if the lock was acquired, `false` otherwise.
    pub fn try_lock(&self) -> bool {
        self.state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    /// Release the mutex.
    pub fn unlock(&self) {
        let prev = self.state.swap(0, Ordering::Release);
        if prev == 2 {
            // There are waiters; wake one.
            let _ = futex_wake(&self.state as *const AtomicI32 as *const i32, 1);
        }
    }
}

// SAFETY: Mutex is designed for cross-thread use.
unsafe impl Send for Mutex {}
unsafe impl Sync for Mutex {}

// ============================================================================
// RwLock (futex-based)
// ============================================================================

/// A reader-writer lock based on futex.
///
/// State encoding:
/// - 0: unlocked
/// - positive N: N active readers
/// - -1: exclusively locked (writer)
#[derive(Debug)]
pub struct RwLock {
    /// Number of readers (positive) or -1 for writer.
    state: AtomicI32,
    /// Writer-waiting futex word.
    writer_wake: AtomicI32,
}

impl RwLock {
    /// Create a new unlocked reader-writer lock.
    pub const fn new() -> Self {
        RwLock {
            state: AtomicI32::new(0),
            writer_wake: AtomicI32::new(0),
        }
    }

    /// Acquire a read lock (shared).
    pub fn read_lock(&self) {
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s >= 0 {
                // No writer -- try to increment reader count.
                if self
                    .state
                    .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return;
                }
            } else {
                // Writer holds the lock -- wait.
                let _ = futex_wait(&self.state as *const AtomicI32 as *const i32, s, 0);
            }
        }
    }

    /// Try to acquire a read lock without blocking.
    pub fn try_read_lock(&self) -> bool {
        let s = self.state.load(Ordering::Relaxed);
        if s >= 0 {
            self.state
                .compare_exchange(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        } else {
            false
        }
    }

    /// Release a read lock.
    pub fn read_unlock(&self) {
        let prev = self.state.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            // Last reader -- wake a waiting writer.
            self.writer_wake.store(1, Ordering::Release);
            let _ = futex_wake(&self.writer_wake as *const AtomicI32 as *const i32, 1);
        }
    }

    /// Acquire a write lock (exclusive).
    pub fn write_lock(&self) {
        loop {
            if self
                .state
                .compare_exchange(0, -1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }

            // Someone holds the lock.  Wait on the writer futex.
            self.writer_wake.store(0, Ordering::Relaxed);
            let _ = futex_wait(&self.writer_wake as *const AtomicI32 as *const i32, 0, 0);
        }
    }

    /// Try to acquire a write lock without blocking.
    pub fn try_write_lock(&self) -> bool {
        self.state
            .compare_exchange(0, -1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    /// Release a write lock.
    pub fn write_unlock(&self) {
        self.state.store(0, Ordering::Release);
        // Wake all waiting readers and one waiting writer.
        let _ = futex_wake(&self.state as *const AtomicI32 as *const i32, i32::MAX);
        self.writer_wake.store(1, Ordering::Release);
        let _ = futex_wake(&self.writer_wake as *const AtomicI32 as *const i32, 1);
    }
}

// SAFETY: RwLock is designed for cross-thread use.
unsafe impl Send for RwLock {}
unsafe impl Sync for RwLock {}

// ============================================================================
// Condvar (futex-based)
// ============================================================================

/// A condition variable based on futex.
///
/// Used in conjunction with a `Mutex` to wait for a condition to become true.
#[derive(Debug)]
pub struct Condvar {
    /// Sequence counter -- incremented on each notify.
    seq: AtomicU32,
}

impl Condvar {
    /// Create a new condition variable.
    pub const fn new() -> Self {
        Condvar {
            seq: AtomicU32::new(0),
        }
    }

    /// Wait on the condition variable.
    ///
    /// The caller must hold `mutex`.  The mutex is released before blocking
    /// and re-acquired before returning.
    pub fn wait(&self, mutex: &Mutex) {
        let seq = self.seq.load(Ordering::Relaxed);
        mutex.unlock();

        // Block until the sequence number changes.
        let _ = futex_wait(&self.seq as *const AtomicU32 as *const i32, seq as i32, 0);

        mutex.lock();
    }

    /// Wait on the condition variable with a timeout (in milliseconds).
    ///
    /// Returns `true` if notified, `false` if timed out.
    pub fn wait_timeout(&self, mutex: &Mutex, timeout_ms: u64) -> bool {
        let seq = self.seq.load(Ordering::Relaxed);
        mutex.unlock();

        let result = futex_wait(
            &self.seq as *const AtomicU32 as *const i32,
            seq as i32,
            timeout_ms,
        );

        mutex.lock();

        !matches!(result, Err(SyscallError::TimedOut))
    }

    /// Wake one waiting thread.
    pub fn notify_one(&self) {
        self.seq.fetch_add(1, Ordering::Release);
        let _ = futex_wake(&self.seq as *const AtomicU32 as *const i32, 1);
    }

    /// Wake all waiting threads.
    pub fn notify_all(&self) {
        self.seq.fetch_add(1, Ordering::Release);
        let _ = futex_wake(&self.seq as *const AtomicU32 as *const i32, i32::MAX);
    }
}

// SAFETY: Condvar is designed for cross-thread use.
unsafe impl Send for Condvar {}
unsafe impl Sync for Condvar {}

// ============================================================================
// Once (run-once initialization)
// ============================================================================

/// State values for `Once`.
const ONCE_INCOMPLETE: i32 = 0;
const ONCE_RUNNING: i32 = 1;
const ONCE_COMPLETE: i32 = 2;

/// A synchronization primitive that runs a closure at most once.
///
/// Multiple threads may call `call_once` concurrently; exactly one will
/// execute the closure and the others will block until it completes.
#[derive(Debug)]
pub struct Once {
    state: AtomicI32,
}

impl Once {
    /// Create a new `Once` value.
    pub const fn new() -> Self {
        Once {
            state: AtomicI32::new(ONCE_INCOMPLETE),
        }
    }

    /// Call the given closure at most once, blocking other callers until
    /// the first call completes.
    pub fn call_once<F: FnOnce()>(&self, f: F) {
        if self.state.load(Ordering::Acquire) == ONCE_COMPLETE {
            return;
        }
        self.call_once_slow(f);
    }

    #[cold]
    fn call_once_slow<F: FnOnce()>(&self, f: F) {
        if self
            .state
            .compare_exchange(
                ONCE_INCOMPLETE,
                ONCE_RUNNING,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            // We won the race -- run the initialization.
            f();
            self.state.store(ONCE_COMPLETE, Ordering::Release);
            let _ = futex_wake(&self.state as *const AtomicI32 as *const i32, i32::MAX);
            return;
        }

        // Someone else is running initialization -- wait for them.
        loop {
            let s = self.state.load(Ordering::Acquire);
            if s == ONCE_COMPLETE {
                return;
            }
            let _ = futex_wait(
                &self.state as *const AtomicI32 as *const i32,
                ONCE_RUNNING,
                0,
            );
        }
    }

    /// Returns `true` if `call_once` has been completed.
    pub fn is_completed(&self) -> bool {
        self.state.load(Ordering::Acquire) == ONCE_COMPLETE
    }
}

// SAFETY: Once is designed for cross-thread use.
unsafe impl Send for Once {}
unsafe impl Sync for Once {}
