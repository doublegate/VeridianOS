//! Process exit, cleanup, signals, and wait
//!
//! Handles process termination, resource cleanup, signal delivery,
//! zombie reaping, and parent-child wait semantics. Also provides
//! system-wide process statistics.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use super::{
    pcb::{Process, ProcessState},
    table,
    thread::ThreadId,
    ProcessId,
};
#[allow(unused_imports)]
use crate::{error::KernelError, println, sched};

/// Exit current process
pub fn exit_process(exit_code: i32) {
    if let Some(process) = super::current_process() {
        // Audit log: process exit
        crate::security::audit::log_process_exit(process.pid.0, exit_code);

        println!(
            "[PROCESS] Process {} exiting with code {}",
            process.pid.0, exit_code
        );

        // Set exit code
        process.set_exit_code(exit_code);

        // Mark all threads as exited
        #[cfg(feature = "alloc")]
        {
            let threads = process.threads.lock();
            for (_, thread) in threads.iter() {
                thread.set_state(super::thread::ThreadState::Zombie);
            }
        }

        // Clean up resources
        cleanup_process(&process);

        // Mark process as zombie (parent needs to reap)
        process.set_state(ProcessState::Zombie);

        // Notify parent: send SIGCHLD and wake if blocked
        if let Some(parent_pid) = process.parent() {
            if let Some(parent) = table::get_process(parent_pid) {
                // Send SIGCHLD to parent (POSIX: delivered on child exit)
                if let Err(_e) = {
                    if exit_signal(&process) != 0 {
                        super::signals::notify(&parent, exit_signal(&process));
                    }
                    Ok::<(), KernelError>(())
                } {
                    println!(
                        "[PROCESS] Warning: Failed to send SIGCHLD to parent {}: {:?}",
                        parent_pid.0, _e
                    );
                }

                // Wake parent if it is blocked (e.g. in waitpid)
                let parent_state = parent.get_state();
                if parent_state == ProcessState::Blocked {
                    parent.set_state(ProcessState::Ready);
                    sched::wake_up_process(parent_pid);
                }
            }
        }

        // Schedule another process
        sched::exit_task(exit_code);
    }
}

/// Wait for child process to exit
#[cfg(feature = "alloc")]
pub fn wait_process(pid: Option<ProcessId>) -> Result<(ProcessId, i32), KernelError> {
    wait_process_with_options(pid, WaitOptions::default())
}

/// Wait options for wait_process_with_options
#[derive(Debug, Clone, Copy, Default)]
pub struct WaitOptions {
    /// Don't block if no child has exited (WNOHANG)
    pub no_hang: bool,
    /// Also return if a child has stopped (WUNTRACED)
    pub untraced: bool,
    /// Also return if a stopped child has been resumed (WCONTINUED)
    pub continued: bool,
    /// Do not report exited children (waitid without WEXITED)
    pub skip_exited: bool,
    /// Report without reaping the child or consuming its stop/continue
    /// report (waitid WNOWAIT), so a later wait sees it again
    pub keep: bool,
}

impl WaitOptions {
    /// Non-blocking wait
    pub fn no_hang() -> Self {
        Self {
            no_hang: true,
            ..Self::default()
        }
    }
}

/// Which children a wait is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitFilter {
    /// Any child (`pid == -1`, `P_ALL`).
    Any,
    /// One child (`pid > 0`, `P_PID`).
    Pid(ProcessId),
    /// Children in a process group (`pid == 0` or `pid < -1`, `P_PGID`;
    /// N-99).
    Group(u64),
}

impl WaitFilter {
    fn matches(self, child: ProcessId) -> bool {
        match self {
            Self::Any => true,
            Self::Pid(p) => p == child,
            Self::Group(g) => {
                table::get_process(child).is_some_and(|c| c.pgid.load(Ordering::Acquire) == g)
            }
        }
    }
}

/// The stop or continue report of `child` that `options` asks for, if any
/// (not consumed).
fn job_report_for(child: &super::Process, options: WaitOptions) -> Option<u32> {
    let r = child.job_report.load(Ordering::Acquire);
    let stop = r != 0 && r & 0xff == 0x7f;
    let cont = r == 0xffff;
    ((options.untraced && stop) || (options.continued && cont)).then_some(r)
}

/// Wait for child process with options
#[cfg(feature = "alloc")]
pub fn wait_process_with_options(
    pid: Option<ProcessId>,
    options: WaitOptions,
) -> Result<(ProcessId, i32), KernelError> {
    wait_children(pid.map_or(WaitFilter::Any, WaitFilter::Pid), options)
}

/// Wait for a child selected by `filter` to change state: exit (unless
/// `skip_exited`), stop (`untraced`) or continue (`continued`). Each stop
/// or continue is reported once.
#[cfg(feature = "alloc")]
pub fn wait_children(
    filter: WaitFilter,
    options: WaitOptions,
) -> Result<(ProcessId, i32), KernelError> {
    let current = super::current_process().ok_or(KernelError::NotInitialized {
        subsystem: "current process",
    })?;
    let current_pid = current.pid;

    loop {
        // Check for zombie children
        let children = table::PROCESS_TABLE.find_children(current_pid);

        // No children at all
        if children.is_empty() {
            return Err(KernelError::NotFound {
                resource: "child process",
                id: 0,
            });
        }

        // Check if any matching child exists
        let mut matching_child_exists = false;

        for child_pid in &children {
            // Check if this child matches our pid filter
            if !filter.matches(*child_pid) {
                continue;
            }

            matching_child_exists = true;

            if let Some(child) = table::get_process(*child_pid) {
                let child_state = child.get_state();

                // Check for zombie (exited)
                if child_state == ProcessState::Zombie && !options.skip_exited {
                    if options.keep {
                        return Ok((*child_pid, child.wait_status()));
                    }
                    // Reap the zombie
                    let exit_code = child.get_exit_code();

                    // Remove from children list
                    current.children.lock().retain(|&p| p != *child_pid);

                    // Remove from process table
                    table::remove_process(*child_pid);

                    println!(
                        "[PROCESS] Process {} reaped child {} (exit code {})",
                        current_pid.0, child_pid.0, exit_code
                    );

                    // Encode as POSIX wait status:
                    // Normal exit: low 7 bits = 0, bits 8-15 = exit code
                    // WIFEXITED(s) = (s & 0x7f) == 0
                    // WEXITSTATUS(s) = (s >> 8) & 0xff
                    // Exit code or terminating signal (N-99).
                    let wait_status = child.wait_status();
                    return Ok((*child_pid, wait_status));
                }

                // A stop (WUNTRACED) or continue (WCONTINUED), reported
                // once: whoever clears it reports it (N-99).
                if let Some(r) = job_report_for(&child, options) {
                    if options.keep {
                        return Ok((*child_pid, r as i32));
                    }
                    if child
                        .job_report
                        .compare_exchange(r, 0, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        return Ok((*child_pid, r as i32));
                    }
                }
            }
        }

        // No matching child found
        if !matching_child_exists {
            return match filter {
                WaitFilter::Pid(p) => Err(KernelError::ProcessNotFound { pid: p.0 }),
                // No child in the group: ECHILD, as for no children.
                _ => Err(KernelError::NotFound {
                    resource: "child process",
                    id: 0,
                }),
            };
        }

        // No zombie children found
        if options.no_hang {
            // WNOHANG: return immediately with (0, 0) to indicate no child changed state
            return Ok((ProcessId(0), 0));
        }

        // A dispatched process waits for a child to change state; its
        // children run as tasks of their own. A fatal signal for this
        // process ends the wait (EINTR; the thread then exits on its way
        // back to user mode).
        #[cfg(feature = "alloc")]
        if crate::sched::dispatch::current_owner().is_some() {
            sched::dispatch::PROCESS_EVENTS.wait_until(|| {
                super::wait_interrupted()
                    || table::PROCESS_TABLE
                        .find_children(current_pid)
                        .iter()
                        .filter(|c| filter.matches(**c))
                        .any(|c| {
                            // A zombie is reapable once its last task is off
                            // the CPU and its page tables are gone.
                            table::get_process(*c).is_none_or(|p| {
                                (p.get_state() == ProcessState::Zombie
                                    && !options.skip_exited
                                    && sched::dispatch::tasks_of(c.0).is_empty())
                                    || job_report_for(&p, options).is_some()
                            })
                        })
            });
            // A signal to act on (fatal, or caught: EINTR, or a restart
            // with SA_RESTART).
            if super::wait_interrupted() {
                return Err(KernelError::WouldBlock);
            }
            continue;
        }

        // Boot execution model: no preemptive scheduler to context-switch
        // to the child. If we find a Ready child (just forked, never run),
        // run it inline to completion. After it exits, loop back to reap
        // the zombie.
        #[cfg(target_arch = "x86_64")]
        {
            let mut ran_child = false;
            for child_pid in &children {
                if !filter.matches(*child_pid) {
                    continue;
                }
                if let Some(child) = table::get_process(*child_pid) {
                    if child.get_state() == ProcessState::Ready {
                        // Get parent's thread ID for BOOT_CURRENT restore
                        let parent_tid = current
                            .threads
                            .lock()
                            .values()
                            .next()
                            .map(|t| t.tid)
                            .unwrap_or(super::thread::ThreadId(current_pid.0));
                        ran_child = crate::bootstrap::boot_run_forked_child(
                            *child_pid,
                            current_pid,
                            parent_tid,
                        );
                        break;
                    }
                }
            }
            if ran_child {
                // Child ran to completion, loop back to reap
                continue;
            }
        }

        // Normal scheduler path: block and wait for child to wake us.
        println!(
            "[PROCESS] Process {} blocking in wait() for {:?}",
            current_pid.0, filter
        );

        sched::block_process(current_pid);

        // When we wake up, loop back to check for zombie children.
        current.set_state(ProcessState::Running);

        // Check if we were interrupted by a signal
        if let Some(signum) = current.get_next_pending_signal() {
            // Clear the signal and return EINTR
            current.clear_pending_signal(signum);
            return Err(KernelError::WouldBlock);
        }
    }
}

// ============================================================================
// Signals
// ============================================================================

/// Standard signal numbers (POSIX)
pub mod signals {
    pub const SIGHUP: i32 = 1; // Hangup
    pub const SIGINT: i32 = 2; // Interrupt
    pub const SIGQUIT: i32 = 3; // Quit
    pub const SIGILL: i32 = 4; // Illegal instruction
    pub const SIGTRAP: i32 = 5; // Trace trap
    pub const SIGABRT: i32 = 6; // Abort
    pub const SIGBUS: i32 = 7; // Bus error
    pub const SIGFPE: i32 = 8; // Floating point exception
    pub const SIGKILL: i32 = 9; // Kill (cannot be caught)
    pub const SIGUSR1: i32 = 10; // User signal 1
    pub const SIGSEGV: i32 = 11; // Segmentation violation
    pub const SIGUSR2: i32 = 12; // User signal 2
    pub const SIGPIPE: i32 = 13; // Broken pipe
    pub const SIGALRM: i32 = 14; // Alarm clock
    pub const SIGTERM: i32 = 15; // Termination
    pub const SIGSTKFLT: i32 = 16; // Stack fault
    pub const SIGCHLD: i32 = 17; // Child status changed
    pub const SIGCONT: i32 = 18; // Continue
    pub const SIGSTOP: i32 = 19; // Stop (cannot be caught)
    pub const SIGTSTP: i32 = 20; // Terminal stop
    pub const SIGTTIN: i32 = 21; // Background read from tty
    pub const SIGTTOU: i32 = 22; // Background write to tty
    pub const SIGURG: i32 = 23; // Urgent data on socket
    pub const SIGXCPU: i32 = 24; // CPU time limit exceeded
    pub const SIGXFSZ: i32 = 25; // File size limit exceeded
    pub const SIGVTALRM: i32 = 26; // Virtual timer expired
    pub const SIGPROF: i32 = 27; // Profiling timer expired
    pub const SIGWINCH: i32 = 28; // Window size changed
    pub const SIGIO: i32 = 29; // I/O possible
    pub const SIGPWR: i32 = 30; // Power failure
    pub const SIGSYS: i32 = 31; // Bad system call
}

/// Signal action types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    /// Default action for signal
    Default,
    /// Ignore signal
    Ignore,
    /// Terminate process
    Terminate,
    /// Terminate and dump core
    CoreDump,
    /// Stop process
    Stop,
    /// Continue stopped process
    Continue,
    /// Call user handler at given address
    Handler(usize),
}

/// Get default action for a signal
pub fn default_signal_action(signal: i32) -> SignalAction {
    use signals::*;
    match signal {
        SIGHUP | SIGINT | SIGKILL | SIGPIPE | SIGALRM | SIGTERM | SIGUSR1 | SIGUSR2 => {
            SignalAction::Terminate
        }
        SIGQUIT | SIGILL | SIGABRT | SIGFPE | SIGSEGV | SIGBUS | SIGSYS | SIGTRAP | SIGXCPU
        | SIGXFSZ => SignalAction::CoreDump,
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => SignalAction::Stop,
        SIGCONT => SignalAction::Continue,
        SIGCHLD | SIGURG | SIGWINCH | SIGIO => SignalAction::Ignore,
        _ => SignalAction::Terminate, // Unknown signals terminate by default
    }
}

/// Send a signal to a process (kill syscall)
pub fn kill_process(pid: ProcessId, signal: i32) -> Result<(), KernelError> {
    // Validate signal number
    if !(0..=super::signals::NSIG as i32).contains(&signal) {
        return Err(KernelError::InvalidArgument {
            name: "signal",
            value: "signal number out of range (0-64)",
        });
    }

    // Special case: signal 0 is used to check if process exists
    if signal == 0 {
        if table::get_process(pid).is_some() {
            return Ok(());
        } else {
            return Err(KernelError::ProcessNotFound { pid: pid.0 });
        }
    }

    let process = table::get_process(pid).ok_or(KernelError::ProcessNotFound { pid: pid.0 })?;

    if !process.is_alive() {
        return Err(KernelError::InvalidState {
            expected: "alive",
            actual: "dead",
        });
    }

    println!("[PROCESS] Sending signal {} to process {}", signal, pid.0);

    // A dispatched process takes its signals on its own threads (sprint
    // D3): queued, ignored ones dropped, SIGKILL at once.
    #[cfg(feature = "alloc")]
    if process.dispatched.load(Ordering::Acquire)
        && super::signals::send_to_process(&process, signal as usize)?
    {
        return Ok(());
    }

    // Queue the signal to the process
    process.send_signal(signal as usize)?;

    // Determine the action to take
    let handler = process.get_signal_handler(signal as usize).unwrap_or(0);
    let action = if handler == 0 {
        default_signal_action(signal)
    } else if handler == 1 {
        SignalAction::Ignore
    } else {
        SignalAction::Handler(handler as usize)
    };

    // Handle uncatchable signals immediately
    match signal {
        signals::SIGKILL => {
            // SIGKILL always terminates immediately
            force_terminate_process(&process, signal)?;
        }
        signals::SIGSTOP => {
            // SIGSTOP always stops immediately
            process.set_state(ProcessState::Blocked);
            sched::block_process(pid);
            println!("[PROCESS] Process {} stopped by SIGSTOP", pid.0);

            // Notify parent with SIGCHLD (POSIX: child stopped)
            notify_parent_sigchld(&process, signals::SIGCHLD as usize);
        }
        _ => {
            // Handle based on action
            match action {
                SignalAction::Ignore => {
                    // Clear the pending signal since we're ignoring it
                    process.clear_pending_signal(signal as usize);
                }
                SignalAction::Terminate | SignalAction::CoreDump => {
                    // For default terminate/core dump actions, do it now
                    force_terminate_process(&process, signal)?;
                }
                SignalAction::Stop => {
                    process.set_state(ProcessState::Blocked);
                    sched::block_process(pid);
                    println!("[PROCESS] Process {} stopped by signal {}", pid.0, signal);

                    // Notify parent with SIGCHLD (POSIX: child stopped)
                    notify_parent_sigchld(&process, signals::SIGCHLD as usize);
                }
                SignalAction::Continue => {
                    if process.get_state() == ProcessState::Blocked {
                        process.set_state(ProcessState::Ready);
                        sched::wake_up_process(pid);
                        println!("[PROCESS] Process {} continued by signal {}", pid.0, signal);

                        // Notify parent with SIGCHLD (POSIX: child continued)
                        notify_parent_sigchld(&process, signals::SIGCHLD as usize);
                    }
                    process.clear_pending_signal(signal as usize);
                }
                SignalAction::Handler(_addr) => {
                    // Signal will be delivered when process returns to user mode
                    // The signal handling is done in the syscall return path
                    println!(
                        "[PROCESS] Signal {} queued for process {}, handler at {:#x}",
                        signal, pid.0, _addr
                    );

                    // Wake up process if blocked so it can handle the signal
                    if process.get_state() == ProcessState::Blocked {
                        process.set_state(ProcessState::Ready);
                        sched::wake_up_process(pid);
                    }
                }
                SignalAction::Default => {
                    // Should not reach here, but handle it as terminate
                    force_terminate_process(&process, signal)?;
                }
            }
        }
    }

    Ok(())
}

/// `kill_pending` value for an `exit_group` by another thread: leave with
/// the process's exit code, not as killed by a signal.
pub const GROUP_EXIT_PENDING: u32 = u32::MAX;

/// Exit the running thread of a dispatched process (ADR 0006 stage D2).
/// Never returns.
///
/// `group` ends the whole process (`exit`/`exit_group`): the other threads
/// leave at their next return to user mode or wakeup. Otherwise only this
/// thread ends (`pthread_exit`). The last thread to leave tears the process
/// down -- files, memory, capabilities -- makes it a zombie and tells the
/// parent. Its page tables, which this thread is still running on, are
/// freed once its task is off the CPU (`user_task_reaped`).
#[cfg(feature = "alloc")]
pub fn exit_dispatched(exit_code: i32, group: bool) -> ! {
    use crate::sched::dispatch;

    if let (Some(process), Some(thread)) = (super::current_process(), super::current_thread()) {
        // Robust futexes the thread still holds (N-225), then
        // CLONE_CHILD_CLEARTID: clear the TID word and wake a joiner.
        super::robust_list::exit_thread(&thread);
        crate::syscall::scheduling::thread_exit(&thread);
        let clear_ptr = thread.clear_tid.load(Ordering::Acquire);
        if clear_ptr != 0 {
            let _ = crate::syscall::userspace::write_user(clear_ptr, 0u32);
            let _ = crate::syscall::sys_futex_wake(clear_ptr, 1, 0);
        }

        let pending = process.kill_pending.load(Ordering::Acquire);
        if group && pending == 0 {
            // First thread out of an exit_group decides the exit code and
            // sends the others after it.
            process.set_exit_code(exit_code);
            process
                .kill_pending
                .store(GROUP_EXIT_PENDING, Ordering::Release);
            for task in dispatch::tasks_of(process.pid.0) {
                dispatch::wake(&task);
            }
        } else if !group && pending == 0 {
            process.set_exit_code(exit_code);
        }
        thread.set_exited(exit_code);

        let others_alive = process.threads.lock().values().any(|t| {
            t.tid != thread.tid
                && !matches!(
                    t.get_state(),
                    super::thread::ThreadState::Zombie | super::thread::ThreadState::Dead
                )
        });
        if !others_alive {
            cleanup_process(&process);
            process.set_state(ProcessState::Zombie);
            notify_parent_sigchld(&process, exit_signal(&process));
        }
        dispatch::PROCESS_EVENTS.wake_all();
    }
    dispatch::exit_current_task()
}

// ============================================================================
// SIGCHLD notification helper
// ============================================================================

/// The signal a parent gets when `process` exits: SIGCHLD, or what clone
/// asked for (its low byte; 0 for none, N-210).
fn exit_signal(process: &Process) -> usize {
    process.exit_signal.load(Ordering::Acquire) as usize
}

/// Send `sig` (SIGCHLD, or the child's exit signal; 0 sends nothing) to
/// the parent of `process` and wake it if blocked.
///
/// Called when a child process exits, stops, or continues. This is the
/// unified notification path: `exit_process` and `kill_process` (for Stop
/// and Continue actions) both funnel through here.
fn notify_parent_sigchld(process: &Process, sig: usize) {
    if let Some(parent_pid) = process.parent() {
        if let Some(parent) = table::get_process(parent_pid) {
            // Signal the parent (POSIX: delivered on child state change)
            if let Err(_e) = {
                if sig != 0 {
                    super::signals::notify(&parent, sig);
                }
                Ok::<(), KernelError>(())
            } {
                println!(
                    "[PROCESS] Warning: Failed to send SIGCHLD to parent {}: {:?}",
                    parent_pid.0, _e
                );
            }

            // Wake parent if it is blocked (e.g. in waitpid)
            if parent.get_state() == ProcessState::Blocked {
                parent.set_state(ProcessState::Ready);
                sched::wake_up_process(parent_pid);
            }
        }
    }
}

// ============================================================================
// Process Cleanup
// ============================================================================

/// Force terminate a process (used by SIGKILL and unhandled fatal signals)
fn force_terminate_process(process: &Process, signal: i32) -> Result<(), KernelError> {
    // A dispatched process is never torn down under its running threads:
    // they act on the signal themselves (at their next return to user mode
    // or wakeup), and the last one out does the teardown (stage D2).
    #[cfg(feature = "alloc")]
    if process.dispatched.load(Ordering::Acquire) {
        let tasks = sched::dispatch::tasks_of(process.pid.0);
        if !tasks.is_empty() {
            let _ = process.kill_pending.compare_exchange(
                0,
                signal as u32,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            for task in &tasks {
                sched::dispatch::wake(task);
            }
            sched::dispatch::PROCESS_EVENTS.wake_all();
            return Ok(());
        }
    }
    let _pid = process.pid;
    println!("[PROCESS] Force terminating process {}", _pid.0);
    process.set_term_signal(signal as u32);

    // Mark all threads as exited
    #[cfg(feature = "alloc")]
    {
        let threads = process.threads.lock();
        for (_, thread) in threads.iter() {
            thread.set_state(super::thread::ThreadState::Zombie);

            // Remove from scheduler if scheduled
            if let Some(task_ptr) = thread.get_task_ptr() {
                // SAFETY: task_ptr is a NonNull<Task> stored in the thread.
                // We mark the task as Dead so the scheduler will not run it
                // again. The threads lock is held, preventing concurrent
                // modification of the thread's task pointer.
                unsafe {
                    let task = task_ptr.as_ptr();
                    (*task).state = ProcessState::Dead;
                    // Note: The scheduler will clean up dead tasks
                }
            }
        }
    }

    // Clean up and mark as zombie
    cleanup_process(process);
    process.set_state(ProcessState::Zombie);

    // Wake up parent if waiting
    if let Some(parent_pid) = process.parent() {
        if let Some(parent) = table::get_process(parent_pid) {
            // Send SIGCHLD to parent
            if let Err(_e) = {
                if exit_signal(process) != 0 {
                    super::signals::notify(&parent, exit_signal(process));
                }
                Ok::<(), KernelError>(())
            } {
                println!(
                    "[PROCESS] Warning: Failed to send SIGCHLD to parent {}: {:?}",
                    parent_pid.0, _e
                );
            }

            if parent.get_state() == ProcessState::Blocked {
                parent.set_state(ProcessState::Ready);
                sched::wake_up_process(parent_pid);
            }
        }
    }

    Ok(())
}

/// Clean up process resources
pub fn cleanup_process(process: &Process) {
    println!(
        "[PROCESS] Cleaning up resources for process {}",
        process.pid.0
    );

    // A parent waiting in vfork for this child may resume.
    process.release_vfork_parent();

    // Release memory (VAS-tracked data frames + page table subtrees)
    {
        let mut memory_space = process.memory_space.lock();
        // Clear all mappings
        memory_space.clear();
    }

    // Free kernel stack frames for all threads.
    //
    // Kernel stacks are allocated by ThreadBuilder::build() from the frame
    // allocator but are NOT tracked in the VAS (they are used through the
    // kernel's direct physical map). Without this, every process exit leaks 16
    // frames (64 KB) of kernel stack per thread -- 630 processes during
    // BusyBox compilation would leak ~40 MB.
    //
    // User stack frames are managed by the VAS (allocated via map_page() and
    // freed by clear() above), so they do not need separate cleanup here.
    #[cfg(feature = "alloc")]
    {
        let threads = process.threads.lock();
        for (_, thread) in threads.iter() {
            let frame_num = thread.kernel_stack.phys_frame.load(Ordering::Acquire);
            let page_count = thread.kernel_stack.phys_page_count.load(Ordering::Acquire);
            if frame_num != 0 && page_count > 0 {
                // Unmaps the guard-paged stack (N-26) on every CPU, then
                // returns its frames.
                crate::mm::kstack::free(crate::mm::kstack::KernelStack {
                    base: thread.kernel_stack.base,
                    frame: crate::mm::FrameNumber::new(frame_num),
                    pages: page_count,
                });
                // Mark as freed to prevent double-free
                thread.kernel_stack.phys_frame.store(0, Ordering::Release);
                thread
                    .kernel_stack
                    .phys_page_count
                    .store(0, Ordering::Release);
            }
        }
    }

    // Release capabilities
    {
        let cap_space = process.capability_space.lock();
        // Clear all capabilities
        cap_space.clear();
    }

    // Close IPC endpoints
    #[cfg(feature = "alloc")]
    {
        use crate::ipc;

        // Remove all endpoints owned by this process from the global registry
        match ipc::remove_process_endpoints(process.pid) {
            Ok(count) => {
                if count > 0 {
                    println!(
                        "[PROCESS] Removed {} IPC endpoints for process {}",
                        count, process.pid.0
                    );
                }
            }
            Err(_e) => {
                println!(
                    "[PROCESS] Warning: Failed to remove IPC endpoints for process {}: {:?}",
                    process.pid.0, _e
                );
            }
        }

        // Clear the local endpoint map
        process.ipc_endpoints.lock().clear();
    }

    // Drop DRM page-flip events and pending flips this process owned, so
    // departed DRM masters do not accumulate queued events. Independent of
    // the IPC cleanup above.
    crate::graphics::gpu_accel::purge_drm_owner(process.pid.0);

    // Close all open file descriptors.
    // Log a warning if the process has more than 3 fds open (stdin/stdout/stderr).
    // This helps identify fd leaks during heavy workloads like BusyBox compilation
    // where 213+ sequential processes must not leak fds.
    {
        let file_table = process.file_table.lock();
        let open_fds = file_table.count_open();
        if open_fds > 3 {
            println!(
                "[PROCESS] Warning: process {} exiting with {} open fds (expected <= 3)",
                process.pid.0, open_fds
            );
        }
        file_table.close_all();
        // flock/fcntl locks die with their owner (N-120).
        crate::fs::flock::cleanup_process_locks(process.pid.0);
    }

    // An exiting tracer detaches from everything it traced (as on Linux),
    // so a later process with the same pid inherits no tracing rights.
    #[cfg(feature = "alloc")]
    table::PROCESS_TABLE.for_each(|p| {
        let _ = p.tracer.compare_exchange(
            process.pid.0,
            0,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
        );
    });

    // Reparent children to init if not zombie
    #[cfg(feature = "alloc")]
    {
        let children: Vec<ProcessId> = process.children.lock().clone();
        if !children.is_empty() && process.get_state() != ProcessState::Zombie {
            if let Some(init_process) = table::get_process(ProcessId(1)) {
                for child_pid in children {
                    if let Some(child) = table::get_process(child_pid) {
                        child.set_parent(Some(ProcessId(1)));
                        init_process.children.lock().push(child_pid);
                        println!("[PROCESS] Reparented process {} to init", child_pid);
                    }
                }
            }
            process.children.lock().clear();
        }
    }

    // Update CPU time statistics
    let _cpu_time = process.cpu_time.load(Ordering::Relaxed);
    println!(
        "[PROCESS] Process {} used {} microseconds of CPU time",
        process.pid.0, _cpu_time
    );
}

// ============================================================================
// Thread Cleanup
// ============================================================================

/// Clean up a dead thread
#[cfg(feature = "alloc")]
pub fn cleanup_thread(process: &Process, tid: ThreadId) -> Result<(), KernelError> {
    // Remove thread from process
    let mut threads = process.threads.lock();

    if let Some(thread) = threads.remove(&tid) {
        println!("[PROCESS] Cleaning up thread {}", tid.0);

        // Make sure thread is marked as dead
        thread.set_state(super::thread::ThreadState::Dead);

        // Clean up scheduler task if exists
        if let Some(task_ptr) = thread.get_task_ptr() {
            // SAFETY: task_ptr is a NonNull<Task> stored in the thread.
            // We clear the thread reference and mark the task as Dead
            // for scheduler cleanup. The thread has already been marked
            // as Dead above, so no scheduler will attempt to run it.
            unsafe {
                let task = task_ptr.as_ptr();

                // Clear thread reference in task
                (*task).thread_ref = None;

                // Mark task for cleanup
                (*task).state = ProcessState::Dead;

                // The scheduler will eventually free the task memory
            }
        }

        // Free user stack pages from the VAS. User stack frames are managed
        // by the VAS (allocated via map_page), so unmap them to free the
        // physical frames. If cleanup_process already called clear(), the
        // mappings are gone and unmap will fail harmlessly.
        if thread.user_stack.size > 0 {
            let stack_base = thread.user_stack.base;
            let stack_size = thread.user_stack.size;

            let memory_space = process.memory_space.lock();
            // Try to unmap each page individually since map_page creates
            // per-page entries in the BTreeMap.
            let num_pages = stack_size / 0x1000;
            for i in 0..num_pages {
                let page_addr = stack_base + i * 0x1000;
                let _ = memory_space.unmap(page_addr, 0x1000);
            }
            println!(
                "[PROCESS] Freed user stack at {:#x}, size {}",
                stack_base, stack_size
            );
        }

        // Free kernel stack frames using the stored physical frame info.
        // ThreadBuilder::build() records the frame number and page count
        // in the Stack struct for exactly this purpose.
        {
            let frame_num = thread
                .kernel_stack
                .phys_frame
                .load(core::sync::atomic::Ordering::Acquire);
            let page_count = thread
                .kernel_stack
                .phys_page_count
                .load(core::sync::atomic::Ordering::Acquire);
            if frame_num != 0 && page_count > 0 {
                crate::mm::kstack::free(crate::mm::kstack::KernelStack {
                    base: thread.kernel_stack.base,
                    frame: crate::mm::FrameNumber::new(frame_num),
                    pages: page_count,
                });
                println!(
                    "[PROCESS] Freed kernel stack for tid {} ({} frames)",
                    tid.0, page_count
                );
                // Mark as freed to prevent double-free
                thread
                    .kernel_stack
                    .phys_frame
                    .store(0, core::sync::atomic::Ordering::Release);
                thread
                    .kernel_stack
                    .phys_page_count
                    .store(0, core::sync::atomic::Ordering::Release);
            }
        }

        // Clean up TLS area
        {
            let tls = thread.tls.lock();
            if tls.base != 0 && tls.size > 0 {
                // Unmap TLS from process's virtual address space
                let memory_space = process.memory_space.lock();
                if let Err(_e) = memory_space.unmap(tls.base, tls.size) {
                    println!(
                        "[PROCESS] Warning: Failed to unmap TLS at {:#x}: {}",
                        tls.base, _e
                    );
                } else {
                    println!(
                        "[PROCESS] Freed TLS area at {:#x}, size {}",
                        tls.base, tls.size
                    );
                }
            }
        }

        Ok(())
    } else {
        Err(KernelError::ThreadNotFound { tid: tid.0 })
    }
}

/// Reap zombie threads in a process
#[cfg(feature = "alloc")]
pub fn reap_zombie_threads(process: &Process) -> Vec<(ThreadId, i32)> {
    let mut reaped = Vec::new();
    let threads = process.threads.lock();

    // Find all zombie threads
    let zombies: Vec<ThreadId> = threads
        .iter()
        .filter(|(_, thread)| thread.get_state() == super::thread::ThreadState::Zombie)
        .map(|(tid, _)| *tid)
        .collect();

    drop(threads);

    // Clean up each zombie thread
    for tid in zombies {
        if let Ok(()) = cleanup_thread(process, tid) {
            // Get exit code before cleanup
            if let Some(thread) = process.get_thread(tid) {
                let exit_code = thread.exit_code.load(Ordering::Acquire) as i32;
                reaped.push((tid, exit_code));
            }
        }
    }

    reaped
}

// ============================================================================
// Process Statistics
// ============================================================================

/// Process statistics
#[cfg(feature = "alloc")]
pub struct ProcessStats {
    pub total_processes: usize,
    pub running_processes: usize,
    pub blocked_processes: usize,
    pub zombie_processes: usize,
    pub total_threads: usize,
    pub total_cpu_time: u64,
    pub total_memory_usage: u64,
}

/// Get system-wide process statistics
#[cfg(feature = "alloc")]
pub fn get_process_stats() -> ProcessStats {
    let mut stats = ProcessStats {
        total_processes: 0,
        running_processes: 0,
        blocked_processes: 0,
        zombie_processes: 0,
        total_threads: 0,
        total_cpu_time: 0,
        total_memory_usage: 0,
    };

    table::PROCESS_TABLE.for_each(|process| {
        stats.total_processes += 1;
        stats.total_threads += process.thread_count();
        stats.total_cpu_time += process.get_cpu_time();
        stats.total_memory_usage += process
            .memory_stats
            .virtual_size
            .load(core::sync::atomic::Ordering::Relaxed);

        match process.get_state() {
            ProcessState::Running => stats.running_processes += 1,
            ProcessState::Blocked => stats.blocked_processes += 1,
            ProcessState::Zombie => stats.zombie_processes += 1,
            _ => {}
        }
    });

    stats
}
