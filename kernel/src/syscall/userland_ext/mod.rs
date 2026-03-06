//! Shell/Userland Extensions for VeridianOS (Phase 7.5 Wave 8)
//!
//! Implements six subsystems for advanced userland support:
//! 1. io_uring - Async I/O submission/completion ring interface
//! 2. ptrace - Process tracing and debugging
//! 3. Core dump - ELF core file generation
//! 4. User/Group management - /etc/passwd, shadow, group
//! 5. sudo/su privilege elevation - sudoers, authentication
//! 6. Crontab scheduler - Periodic job scheduling

#![allow(dead_code)]

// Submodules
pub mod coredump;
pub mod cron;
mod helpers;
pub mod io_uring;
pub mod privilege;
pub mod ptrace;
pub mod users;

// Re-export all public types from submodules for API compatibility
#[allow(unused_imports)]
pub use coredump::{CoreDumpError, CoreDumpWriter, CoreMemorySegment, PrPsInfo, PrStatus};
#[allow(unused_imports)]
pub use cron::{
    CronDaemon, CronEntry, CronError, CronField, CronFieldItem, CronJobExecution, CronSchedule,
    CronSpecial, CronTab,
};
#[allow(unused_imports)]
pub use io_uring::{
    CqEntry, IoUring, IoUringError, IoUringId, IoUringManager, IoUringOpcode, IoUringParams,
    IoUringState, PendingOp, RingBuffer, SqEntry, SqeFlag, IORING_SETUP_IOPOLL,
    IORING_SETUP_SQPOLL, IORING_SETUP_SQ_AFF,
};
#[allow(unused_imports)]
pub use privilege::{
    PrivilegeError, PrivilegeManager, SuSwitchResult, SudoExecResult, SudoSession, SudoersParser,
    SudoersRule,
};
#[allow(unused_imports)]
pub use ptrace::{
    PtraceError, PtraceManager, PtraceOptions, PtraceRequest, RegisterState, SigInfo, TraceeState,
};
#[allow(unused_imports)]
pub use users::{GroupDatabase, GroupEntry, ShadowEntry, UserDatabase, UserEntry, UserGroupError};

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::{string::String, vec, vec::Vec};

    use super::*;

    // --- io_uring tests ---

    #[test]
    fn test_ring_buffer_basic() {
        let mut rb: RingBuffer<u32> = RingBuffer::new(4).unwrap();
        assert!(rb.is_empty());
        assert_eq!(rb.capacity(), 4);
        assert_eq!(rb.available(), 4);
        rb.push(1).unwrap();
        rb.push(2).unwrap();
        assert_eq!(rb.len(), 2);
        assert_eq!(rb.pop(), Some(1));
        assert_eq!(rb.pop(), Some(2));
        assert!(rb.is_empty());
    }

    #[test]
    fn test_ring_buffer_full() {
        let mut rb: RingBuffer<u32> = RingBuffer::new(2).unwrap();
        rb.push(10).unwrap();
        rb.push(20).unwrap();
        assert!(rb.is_full());
        assert!(matches!(
            rb.push(30),
            Err(IoUringError::SubmissionQueueFull)
        ));
    }

    #[test]
    fn test_ring_buffer_invalid_capacity() {
        let result: Result<RingBuffer<u32>, _> = RingBuffer::new(3);
        assert!(matches!(result, Err(IoUringError::InvalidEntries)));
        let result2: Result<RingBuffer<u32>, _> = RingBuffer::new(0);
        assert!(matches!(result2, Err(IoUringError::InvalidEntries)));
    }

    #[test]
    fn test_io_uring_create_and_submit() {
        let params = IoUringParams::default();
        let mut ring = IoUring::new(1, params, 100).unwrap();
        assert_eq!(ring.state(), IoUringState::Idle);

        let sqe = SqEntry::nop(42);
        ring.submit(sqe).unwrap();
        assert_eq!(ring.sq_pending(), 1);
        assert_eq!(ring.state(), IoUringState::Submitting);
    }

    #[test]
    fn test_io_uring_process_and_reap() {
        let params = IoUringParams::default();
        let mut ring = IoUring::new(1, params, 100).unwrap();

        ring.submit(SqEntry::nop(1)).unwrap();
        ring.submit(SqEntry::readv(5, 0x1000, 4, 0, 2)).unwrap();

        let processed = ring.process_submissions();
        assert_eq!(processed, 2);
        assert_eq!(ring.cq_ready(), 2);

        let cqe1 = ring.reap_completion().unwrap();
        assert_eq!(cqe1.user_data, 1);
        assert_eq!(cqe1.result, 0); // NOP returns 0

        let cqe2 = ring.reap_completion().unwrap();
        assert_eq!(cqe2.user_data, 2);
        assert_eq!(cqe2.result, 4); // READV returns len
    }

    #[test]
    fn test_io_uring_batch_submit() {
        let params = IoUringParams::default();
        let mut ring = IoUring::new(1, params, 100).unwrap();

        let sqes = vec![SqEntry::nop(1), SqEntry::nop(2), SqEntry::nop(3)];
        let submitted = ring.submit_batch(&sqes).unwrap();
        assert_eq!(submitted, 3);
        assert_eq!(ring.total_submissions(), 3);
    }

    #[test]
    fn test_io_uring_shutdown() {
        let params = IoUringParams::default();
        let mut ring = IoUring::new(1, params, 100).unwrap();
        ring.submit(SqEntry::nop(1)).unwrap();
        ring.shutdown();
        assert_eq!(ring.state(), IoUringState::Shutdown);
        assert!(matches!(
            ring.submit(SqEntry::nop(2)),
            Err(IoUringError::Shutdown)
        ));
    }

    #[test]
    fn test_io_uring_manager() {
        let mut mgr = IoUringManager::new();
        let id = mgr.setup(IoUringParams::default(), 100).unwrap();
        assert_eq!(mgr.active_rings(), 1);
        assert!(mgr.get_ring(id).is_some());
        mgr.destroy(id).unwrap();
        assert_eq!(mgr.active_rings(), 0);
    }

    #[test]
    fn test_io_uring_register_files() {
        let mut ring = IoUring::new(1, IoUringParams::default(), 100).unwrap();
        ring.register_files(&[1, 2, 3]).unwrap();
        ring.unregister_files();
    }

    #[test]
    fn test_sqe_constructors() {
        let fsync = SqEntry::fsync(10, true, 99);
        assert_eq!(fsync.opcode, IoUringOpcode::Fsync as u8);
        assert_eq!(fsync.fd, 10);
        assert_eq!(fsync.op_flags, 1);
        assert_eq!(fsync.user_data, 99);

        let poll = SqEntry::poll_add(5, 0x01, 50);
        assert_eq!(poll.opcode, IoUringOpcode::PollAdd as u8);
    }

    // --- ptrace tests ---

    #[test]
    fn test_ptrace_attach_detach() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        assert!(mgr.is_traced(2));
        assert_eq!(mgr.get_tracer(2), Some(1));
        mgr.detach(1, 2).unwrap();
        assert!(!mgr.is_traced(2));
    }

    #[test]
    fn test_ptrace_cannot_trace_self() {
        let mut mgr = PtraceManager::new();
        assert!(matches!(
            mgr.attach(1, 1),
            Err(PtraceError::PermissionDenied)
        ));
    }

    #[test]
    fn test_ptrace_double_attach() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        assert!(matches!(mgr.attach(3, 2), Err(PtraceError::AlreadyTraced)));
    }

    #[test]
    fn test_ptrace_cont_and_state() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        assert!(matches!(
            mgr.get_tracee_state(2),
            Some(TraceeState::Stopped(19))
        ));
        mgr.cont(1, 2, 0).unwrap();
        assert!(matches!(
            mgr.get_tracee_state(2),
            Some(TraceeState::Running)
        ));
    }

    #[test]
    fn test_ptrace_single_step() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        mgr.single_step(1, 2).unwrap();
        assert!(matches!(
            mgr.get_tracee_state(2),
            Some(TraceeState::SingleStep)
        ));
    }

    #[test]
    fn test_ptrace_get_set_regs() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        let mut regs = mgr.get_regs(1, 2).unwrap();
        regs.rip = 0x4000;
        regs.rax = 42;
        mgr.set_regs(1, 2, regs).unwrap();
        let updated = mgr.get_regs(1, 2).unwrap();
        assert_eq!(updated.rip, 0x4000);
        assert_eq!(updated.rax, 42);
    }

    #[test]
    fn test_ptrace_peek_poke() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        mgr.poke_data(1, 2, 0x1000, 0xDEAD).unwrap();
        let val = mgr.peek_data(1, 2, 0x1000).unwrap();
        assert_eq!(val, 0xDEAD);
    }

    #[test]
    fn test_ptrace_on_signal() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        mgr.cont(1, 2, 0).unwrap();
        mgr.on_signal(2, 11, 0xBAD); // SIGSEGV
        assert!(matches!(
            mgr.get_tracee_state(2),
            Some(TraceeState::Stopped(11))
        ));
        let info = mgr.get_sig_info(1, 2).unwrap();
        assert_eq!(info.signo, 11);
        assert_eq!(info.fault_addr, 0xBAD);
    }

    #[test]
    fn test_ptrace_tracer_exit_cleanup() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        mgr.attach(1, 3).unwrap();
        assert_eq!(mgr.active_traces(), 2);
        mgr.on_tracer_exit(1);
        assert_eq!(mgr.active_traces(), 0);
    }

    // --- Core Dump tests ---

    #[test]
    fn test_core_dump_basic() {
        let mut writer = CoreDumpWriter::new();
        writer.prstatus.pid = 42;
        writer.prstatus.signal = 11;
        writer.prpsinfo.set_fname("test_prog");
        writer.add_segment(0x400000, 5, vec![0xCC; 64]); // R+X
        writer.add_segment(0x600000, 6, vec![0; 128]); // R+W

        let dump = writer.write_core_dump().unwrap();
        // Check ELF magic
        assert_eq!(&dump[0..4], &[0x7F, b'E', b'L', b'F']);
        // Check it's 64-bit
        assert_eq!(dump[4], 2); // ELFCLASS64
                                // Check it's a core file
        assert_eq!(dump[16], 4); // ET_CORE low byte
        assert_eq!(dump[17], 0);
        assert!(dump.len() > 64); // > ELF64_EHDR_SIZE
    }

    #[test]
    fn test_core_dump_empty_segments() {
        let mut writer = CoreDumpWriter::new();
        writer.prstatus.pid = 1;
        let dump = writer.write_core_dump().unwrap();
        assert_eq!(&dump[0..4], &[0x7F, b'E', b'L', b'F']);
    }

    #[test]
    fn test_prpsinfo_fname() {
        let mut info = PrPsInfo::default();
        info.set_fname("hello_world");
        assert_eq!(&info.fname[..11], b"hello_world");
        assert_eq!(info.fname[11], 0);
    }

    // --- User/Group tests ---

    #[test]
    fn test_user_database_creation() {
        let db = UserDatabase::new();
        assert_eq!(db.user_count(), 1); // root
        let root = db.get_user_by_uid(0).unwrap();
        assert_eq!(root.username, "root");
    }

    #[test]
    fn test_user_add_remove() {
        let mut db = UserDatabase::new();
        let uid = db.add_user("alice", 1000, None).unwrap();
        assert_eq!(uid, 1000);
        assert_eq!(db.user_count(), 2);
        assert!(db.get_user_by_name("alice").is_some());
        db.remove_user("alice").unwrap();
        assert_eq!(db.user_count(), 1);
    }

    #[test]
    fn test_user_duplicate() {
        let mut db = UserDatabase::new();
        db.add_user("bob", 1000, None).unwrap();
        assert!(matches!(
            db.add_user("bob", 1000, None),
            Err(UserGroupError::UserExists)
        ));
    }

    #[test]
    fn test_passwd_serialization() {
        let user = UserEntry::new("alice", 1000, 1000);
        let line = user.to_passwd_line();
        assert!(line.contains("alice:x:1000:1000:"));
        let parsed = UserEntry::from_passwd_line(&line).unwrap();
        assert_eq!(parsed.username, "alice");
        assert_eq!(parsed.uid, 1000);
    }

    #[test]
    fn test_shadow_entry() {
        let shadow = ShadowEntry::new_locked("alice");
        assert!(shadow.is_locked());
        let shadow2 = ShadowEntry::with_password("alice", "$6$salt$hash");
        assert!(!shadow2.is_locked());
    }

    #[test]
    fn test_group_database() {
        let mut db = GroupDatabase::new();
        let gid = db.add_group("developers", None).unwrap();
        assert_eq!(gid, 1000);
        db.add_user_to_group("alice", "developers").unwrap();
        let groups = db.get_user_groups("alice");
        assert_eq!(groups, vec![1000]);
    }

    #[test]
    fn test_group_serialization() {
        let mut group = GroupEntry::new("staff", 100);
        group.add_member("alice");
        group.add_member("bob");
        let line = group.to_group_line();
        assert!(line.contains("staff:x:100:alice,bob"));
        let parsed = GroupEntry::from_group_line(&line).unwrap();
        assert_eq!(parsed.members.len(), 2);
    }

    #[test]
    fn test_username_validation() {
        let mut db = UserDatabase::new();
        assert!(matches!(
            db.add_user("", 1000, None),
            Err(UserGroupError::InvalidUsername)
        ));
        assert!(matches!(
            db.add_user("1bad", 1000, None),
            Err(UserGroupError::InvalidUsername)
        ));
        assert!(db.add_user("_valid-name.1", 1000, None).is_ok());
    }

    // --- sudo/su tests ---

    #[test]
    fn test_sudoers_rule_match() {
        let rule = SudoersRule::new("alice", "ALL", "ALL", "ALL");
        assert!(rule.matches_user("alice", &[]));
        assert!(!rule.matches_user("bob", &[]));
        assert!(rule.matches_runas("root"));
        assert!(rule.matches_command("/bin/ls"));
    }

    #[test]
    fn test_sudoers_group_match() {
        let rule = SudoersRule::new("%wheel", "ALL", "ALL", "ALL");
        let groups = vec![String::from("wheel")];
        assert!(rule.matches_user("anyone", &groups));
        assert!(!rule.matches_user("anyone", &[]));
    }

    #[test]
    fn test_privilege_manager_check() {
        let mut mgr = PrivilegeManager::new();
        mgr.sudoers
            .add_rule(SudoersRule::new("alice", "ALL", "ALL", "ALL"));
        let result = mgr
            .check_sudo_permission("alice", &[], "root", "/bin/ls")
            .unwrap();
        assert!(result);
        let result2 = mgr
            .check_sudo_permission("bob", &[], "root", "/bin/ls")
            .unwrap();
        assert!(!result2);
    }

    #[test]
    fn test_sudo_nopasswd() {
        let mut mgr = PrivilegeManager::new();
        mgr.sudoers
            .add_rule(SudoersRule::new_nopasswd("alice", "ALL", "ALL", "ALL"));
        assert!(mgr.is_nopasswd("alice", &[], "root", "/bin/ls"));
    }

    #[test]
    fn test_sudo_session_timeout() {
        let session = SudoSession::new(1000, 1, 100);
        assert!(session.is_valid(200)); // within 300s
        assert!(session.is_valid(399)); // edge
        assert!(!session.is_valid(400)); // expired
    }

    #[test]
    fn test_su_switch_root_no_password() {
        let mgr = PrivilegeManager::new();
        let result = mgr.su_switch(0, "alice", None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_su_switch_non_root_needs_password() {
        let mgr = PrivilegeManager::new();
        let result = mgr.su_switch(1000, "root", None);
        assert!(matches!(result, Err(PrivilegeError::AuthFailed)));
    }

    #[test]
    fn test_password_hash_stub() {
        let hash1 = PrivilegeManager::hash_password_stub(b"password", b"salt", 10);
        let hash2 = PrivilegeManager::hash_password_stub(b"password", b"salt", 10);
        assert_eq!(hash1, hash2); // deterministic
        let hash3 = PrivilegeManager::hash_password_stub(b"different", b"salt", 10);
        assert_ne!(hash1, hash3);
    }

    // --- Crontab tests ---

    #[test]
    fn test_cron_field_any() {
        let field = CronField::parse("*", 0, 59).unwrap();
        assert!(matches!(field, CronField::Any));
        assert!(field.matches(0));
        assert!(field.matches(59));
    }

    #[test]
    fn test_cron_field_value() {
        let field = CronField::parse("15", 0, 59).unwrap();
        assert!(field.matches(15));
        assert!(!field.matches(16));
    }

    #[test]
    fn test_cron_field_range() {
        let field = CronField::parse("10-20", 0, 59).unwrap();
        assert!(field.matches(10));
        assert!(field.matches(15));
        assert!(field.matches(20));
        assert!(!field.matches(9));
        assert!(!field.matches(21));
    }

    #[test]
    fn test_cron_field_step() {
        let field = CronField::parse("*/15", 0, 59).unwrap();
        assert!(field.matches(0));
        assert!(field.matches(15));
        assert!(field.matches(30));
        assert!(field.matches(45));
        assert!(!field.matches(10));
    }

    #[test]
    fn test_cron_field_list() {
        let field = CronField::parse("1,5,10", 0, 59).unwrap();
        assert!(field.matches(1));
        assert!(field.matches(5));
        assert!(field.matches(10));
        assert!(!field.matches(2));
    }

    #[test]
    fn test_cron_schedule_parse() {
        let sched = CronSchedule::parse("30 2 * * *").unwrap();
        assert!(sched.matches(30, 2, 15, 6, 3));
        assert!(!sched.matches(0, 2, 15, 6, 3));
        assert!(!sched.matches(30, 3, 15, 6, 3));
    }

    #[test]
    fn test_cron_special_strings() {
        let daily = CronSchedule::parse("@daily").unwrap();
        assert!(daily.matches(0, 0, 1, 1, 0));
        assert!(!daily.matches(1, 0, 1, 1, 0));

        let reboot = CronSchedule::parse("@reboot").unwrap();
        assert!(reboot.is_reboot);
        assert!(!reboot.matches(0, 0, 1, 1, 0));
    }

    #[test]
    fn test_cron_entry_parse() {
        let entry = CronEntry::parse(1, "0 3 * * 1 /usr/bin/backup", "root").unwrap();
        assert_eq!(entry.command, "/usr/bin/backup");
        assert_eq!(entry.owner, "root");
    }

    #[test]
    fn test_crontab_add_remove() {
        let mut tab = CronTab::new("alice");
        let id = tab.add_line("0 * * * * /bin/echo hello").unwrap();
        assert_eq!(tab.entry_count(), 1);
        tab.remove_entry(id).unwrap();
        assert_eq!(tab.entry_count(), 0);
    }

    #[test]
    fn test_cron_daemon_tick() {
        let mut daemon = CronDaemon::new();
        daemon.start();
        let tab = daemon.get_or_create_crontab("alice");
        tab.add_line("30 * * * * /bin/echo tick").unwrap();

        let queued = daemon.tick(30, 12, 15, 6, 3, 1000);
        assert_eq!(queued, 1);

        let jobs = daemon.drain_queue();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].command, "/bin/echo tick");
    }

    #[test]
    fn test_cron_daemon_reboot_jobs() {
        let mut daemon = CronDaemon::new();
        daemon.start();
        let tab = daemon.get_or_create_crontab("root");
        tab.add_line("@reboot /etc/init.d/startup").unwrap();

        daemon.fire_reboot_jobs(0);
        let jobs = daemon.drain_queue();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].command, "/etc/init.d/startup");

        // Should not fire again
        daemon.fire_reboot_jobs(100);
        assert!(daemon.drain_queue().is_empty());
    }

    #[test]
    fn test_cron_next_run() {
        let sched = CronSchedule::parse("0 12 * * *").unwrap();
        // From 10:30, next should be 12:00 same day
        let next = sched.next_run(30, 10, 15, 6, 2026);
        assert!(next.is_some());
        let (min, hour, _day, _month, _year) = next.unwrap();
        assert_eq!(min, 0);
        assert_eq!(hour, 12);
    }

    #[test]
    fn test_day_of_week_calculation() {
        use cron::day_of_week;
        // 2026-03-05 is a Thursday (4)
        assert_eq!(day_of_week(2026, 3, 5), 4);
        // 2026-01-01 is a Thursday (4)
        assert_eq!(day_of_week(2026, 1, 1), 4);
    }

    #[test]
    fn test_days_in_month() {
        use cron::days_in_month;
        assert_eq!(days_in_month(2, 2024), 29); // leap year
        assert_eq!(days_in_month(2, 2025), 28); // not leap
        assert_eq!(days_in_month(1, 2025), 31);
        assert_eq!(days_in_month(4, 2025), 30);
    }

    #[test]
    fn test_helper_parse_functions() {
        use helpers::{parse_u32, parse_u64, parse_u8};
        assert_eq!(parse_u8("0"), Some(0));
        assert_eq!(parse_u8("255"), Some(255));
        assert_eq!(parse_u8("256"), None);
        assert_eq!(parse_u32("4294967295"), Some(u32::MAX));
        assert_eq!(parse_u64("0"), Some(0));
        assert_eq!(parse_u64("abc"), None);
    }
}
