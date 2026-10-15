//! Process lifecycle management
//!
//! This module handles process creation, termination, and state transitions.
//! The implementation is split into focused submodules:
//! - [`creation`](super::creation): Process creation, exec, and stack setup
//! - [`fork`](super::fork): Process forking with address space cloning
//! - [`exit`](super::exit): Process exit, cleanup, signals, wait, and
//!   statistics

// Re-export everything from submodules to maintain existing API
// Re-export wait_process (used by parent module re-exports)
#[cfg(feature = "alloc")]
#[allow(unused_imports)]
pub(crate) use super::creation::{
    read_executable, setup_exec_stack as setup_exec_stack_pub, setup_static_tls,
};
#[cfg(feature = "alloc")]
pub use super::exit::wait_process;
pub use super::{
    creation::{
        create_process, create_process_with_options, exec_process, parse_shebang, search_path,
        ProcessCreateOptions, DEFAULT_KERNEL_STACK_SIZE, DEFAULT_USER_STACK_SIZE,
    },
    exit::{
        cleanup_thread, default_signal_action, exit_process, get_process_stats, kill_process,
        reap_zombie_threads, signals, ProcessStats, SignalAction, WaitOptions,
    },
    fork::fork_process,
};
