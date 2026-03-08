//! Session management for multi-user support
//!
//! Provides session groups, virtual terminal assignment, session isolation,
//! and login/logout lifecycle management. Each session represents a user's
//! login context with its own process group, VT assignment, and state.
//!
//! Sessions are identified by a `SessionId` (u64) and tracked by a global
//! `SessionManager` protected by a spin mutex. A maximum of 8 concurrent
//! sessions is enforced.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::{error::KernelError, process::ProcessId};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of concurrent sessions.
const MAX_SESSIONS: usize = 8;

/// Base VT number for graphical sessions (session 0 -> VT7, session 1 -> VT8,
/// ...).
const VT_BASE: u8 = 7;

// ---------------------------------------------------------------------------
// SessionId
// ---------------------------------------------------------------------------

/// Unique session identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u64);

/// Monotonic session ID allocator.
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn alloc_session_id() -> SessionId {
    SessionId(NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// SessionState
// ---------------------------------------------------------------------------

/// Runtime state of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Session is the foreground (active) session receiving input.
    Active,
    /// Session is locked -- requires re-authentication to resume.
    Locked,
    /// Session is running but in the background (another session is active).
    Background,
    /// Session is in the process of logging out.
    LoggingOut,
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// A user session containing process groups, VT assignment, and metadata.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session identifier.
    pub id: SessionId,
    /// User ID of the session owner.
    pub user_id: u64,
    /// Username of the session owner.
    pub username: String,
    /// Virtual terminal number assigned to this session.
    pub vt_number: u8,
    /// Current state.
    pub state: SessionState,
    /// Process group ID that currently owns the terminal (foreground).
    pub foreground_group: u64,
    /// Background process group IDs.
    pub background_groups: Vec<u64>,
    /// TSC tick count at login time.
    pub login_time: u64,
    /// Process IDs belonging to this session.
    pub process_ids: Vec<u64>,
}

#[cfg(feature = "alloc")]
impl Session {
    /// Create a new session.
    fn new(id: SessionId, user_id: u64, username: &str, vt_number: u8) -> Self {
        Self {
            id,
            user_id,
            username: String::from(username),
            vt_number,
            state: SessionState::Active,
            foreground_group: 0,
            background_groups: Vec::new(),
            login_time: 0,
            process_ids: Vec::new(),
        }
    }

    /// Check whether the given process belongs to this session.
    pub fn contains_process(&self, pid: u64) -> bool {
        self.process_ids.contains(&pid)
    }

    /// Return the number of processes in this session.
    pub fn process_count(&self) -> usize {
        self.process_ids.len()
    }
}

// ---------------------------------------------------------------------------
// SessionManager
// ---------------------------------------------------------------------------

/// Manages all active sessions.
#[cfg(feature = "alloc")]
pub struct SessionManager {
    /// Active sessions indexed by position (max `MAX_SESSIONS`).
    sessions: Vec<Session>,
    /// Currently active (foreground) session ID.
    active_session: Option<SessionId>,
}

#[cfg(feature = "alloc")]
impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl SessionManager {
    /// Create a new, empty session manager.
    pub const fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active_session: None,
        }
    }

    /// Create a new session for a user.
    ///
    /// Assigns the next available virtual terminal starting at `VT_BASE`.
    /// Returns `Err` if the maximum number of sessions is reached.
    pub fn create_session(
        &mut self,
        user_id: u64,
        username: &str,
        vt: u8,
    ) -> Result<SessionId, KernelError> {
        if self.sessions.len() >= MAX_SESSIONS {
            return Err(KernelError::InvalidState {
                expected: "fewer than 8 sessions",
                actual: "max sessions reached",
            });
        }

        let id = alloc_session_id();
        let vt_number = if vt > 0 {
            vt
        } else {
            VT_BASE + self.sessions.len() as u8
        };

        let mut session = Session::new(id, user_id, username, vt_number);

        // Read TSC as login time (or 0 if unavailable).
        #[cfg(target_arch = "x86_64")]
        {
            session.login_time = unsafe { core::arch::x86_64::_rdtsc() };
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            session.login_time = 0;
        }

        // If no session is active, make this one active.
        if self.active_session.is_none() {
            session.state = SessionState::Active;
            self.active_session = Some(id);
        } else {
            session.state = SessionState::Background;
        }

        self.sessions.push(session);
        Ok(id)
    }

    /// Destroy a session and remove it from tracking.
    pub fn destroy_session(&mut self, id: SessionId) {
        self.sessions.retain(|s| s.id != id);
        if self.active_session == Some(id) {
            // Promote the first remaining session to active.
            self.active_session = self.sessions.first().map(|s| s.id);
            if let Some(new_active) = self.active_session {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == new_active) {
                    s.state = SessionState::Active;
                }
            }
        }
    }

    /// Get a reference to a session by ID.
    pub fn get_session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    /// Get a mutable reference to a session by ID.
    pub fn get_session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    /// Return the currently active (foreground) session ID.
    pub fn get_active_session(&self) -> Option<SessionId> {
        self.active_session
    }

    /// Switch to a different session.
    ///
    /// The previously active session is moved to `Background`; the new
    /// session becomes `Active`.
    pub fn switch_session(&mut self, id: SessionId) {
        // Put current active session into background.
        if let Some(current_id) = self.active_session {
            if let Some(s) = self.sessions.iter_mut().find(|s| s.id == current_id) {
                if s.state == SessionState::Active {
                    s.state = SessionState::Background;
                }
            }
        }

        // Activate the target session.
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            s.state = SessionState::Active;
            self.active_session = Some(id);
        }
    }

    /// Lock a session (requires re-authentication to resume).
    pub fn lock_session(&mut self, id: SessionId) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            s.state = SessionState::Locked;
        }
    }

    /// Unlock a previously locked session, returning it to `Active`.
    pub fn unlock_session(&mut self, id: SessionId) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            if s.state == SessionState::Locked {
                s.state = SessionState::Active;
            }
        }
    }

    /// Add a process to a session.
    pub fn add_process(&mut self, session_id: SessionId, pid: ProcessId) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            if !s.process_ids.contains(&pid.0) {
                s.process_ids.push(pid.0);
            }
        }
    }

    /// Remove a process from a session.
    pub fn remove_process(&mut self, session_id: SessionId, pid: ProcessId) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            s.process_ids.retain(|&p| p != pid.0);
        }
    }

    /// Find the session assigned to a given VT number.
    pub fn get_session_for_vt(&self, vt: u8) -> Option<SessionId> {
        self.sessions
            .iter()
            .find(|s| s.vt_number == vt)
            .map(|s| s.id)
    }

    /// Return a list of all session IDs.
    pub fn list_sessions(&self) -> Vec<SessionId> {
        self.sessions.iter().map(|s| s.id).collect()
    }

    /// Number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Check whether a process in one session may signal a process in another.
    ///
    /// Session isolation: cross-session signaling is denied unless the
    /// source process belongs to the same session as the target.
    pub fn can_signal(&self, source_pid: u64, target_pid: u64) -> bool {
        let source_session = self
            .sessions
            .iter()
            .find(|s| s.process_ids.contains(&source_pid));
        let target_session = self
            .sessions
            .iter()
            .find(|s| s.process_ids.contains(&target_pid));

        match (source_session, target_session) {
            (Some(src), Some(tgt)) => src.id == tgt.id,
            // If either process is not in any session, allow (kernel process).
            _ => true,
        }
    }

    /// Begin logout for a session: set state to `LoggingOut`.
    pub fn begin_logout(&mut self, id: SessionId) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            s.state = SessionState::LoggingOut;
        }
    }

    /// Get all process IDs belonging to a session (for cleanup on logout).
    pub fn get_session_processes(&self, id: SessionId) -> Vec<u64> {
        self.sessions
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.process_ids.clone())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Global session manager
// ---------------------------------------------------------------------------

/// Global session manager, protected by a spin mutex.
#[cfg(feature = "alloc")]
pub static SESSION_MANAGER: Mutex<SessionManager> = Mutex::new(SessionManager::new());

/// Initialize the session subsystem.
#[cfg(feature = "alloc")]
pub fn init() {
    // Force the lazy initialization of the session manager.
    let _guard = SESSION_MANAGER.lock();
    // Nothing else to do -- the manager starts empty.
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn test_session_id_allocation() {
        let id1 = alloc_session_id();
        let id2 = alloc_session_id();
        assert_ne!(id1, id2);
        assert!(id2.0 > id1.0);
    }

    #[test]
    fn test_create_session() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(1000, "alice", 0).unwrap();
        assert_eq!(mgr.session_count(), 1);
        assert_eq!(mgr.get_active_session(), Some(id));

        let session = mgr.get_session(id).unwrap();
        assert_eq!(session.user_id, 1000);
        assert_eq!(session.username, "alice");
        assert_eq!(session.state, SessionState::Active);
    }

    #[test]
    fn test_max_sessions() {
        let mut mgr = SessionManager::new();
        for i in 0..MAX_SESSIONS {
            let name = alloc::format!("user{}", i);
            mgr.create_session(i as u64, &name, 0).unwrap();
        }
        let result = mgr.create_session(100, "overflow", 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_destroy_session() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(1, "bob", 0).unwrap();
        assert_eq!(mgr.session_count(), 1);
        mgr.destroy_session(id);
        assert_eq!(mgr.session_count(), 0);
        assert_eq!(mgr.get_active_session(), None);
    }

    #[test]
    fn test_switch_session() {
        let mut mgr = SessionManager::new();
        let id1 = mgr.create_session(1, "alice", 0).unwrap();
        let id2 = mgr.create_session(2, "bob", 0).unwrap();

        assert_eq!(mgr.get_active_session(), Some(id1));
        assert_eq!(
            mgr.get_session(id2).unwrap().state,
            SessionState::Background
        );

        mgr.switch_session(id2);
        assert_eq!(mgr.get_active_session(), Some(id2));
        assert_eq!(
            mgr.get_session(id1).unwrap().state,
            SessionState::Background
        );
        assert_eq!(mgr.get_session(id2).unwrap().state, SessionState::Active);
    }

    #[test]
    fn test_lock_unlock_session() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(1, "charlie", 0).unwrap();

        mgr.lock_session(id);
        assert_eq!(mgr.get_session(id).unwrap().state, SessionState::Locked);

        mgr.unlock_session(id);
        assert_eq!(mgr.get_session(id).unwrap().state, SessionState::Active);
    }

    #[test]
    fn test_add_remove_process() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(1, "dave", 0).unwrap();

        mgr.add_process(id, ProcessId(42));
        mgr.add_process(id, ProcessId(43));
        assert_eq!(mgr.get_session(id).unwrap().process_count(), 2);
        assert!(mgr.get_session(id).unwrap().contains_process(42));

        mgr.remove_process(id, ProcessId(42));
        assert_eq!(mgr.get_session(id).unwrap().process_count(), 1);
        assert!(!mgr.get_session(id).unwrap().contains_process(42));
    }

    #[test]
    fn test_session_isolation() {
        let mut mgr = SessionManager::new();
        let id1 = mgr.create_session(1, "alice", 0).unwrap();
        let id2 = mgr.create_session(2, "bob", 0).unwrap();

        mgr.add_process(id1, ProcessId(10));
        mgr.add_process(id2, ProcessId(20));

        // Same session: allowed
        mgr.add_process(id1, ProcessId(11));
        assert!(mgr.can_signal(10, 11));

        // Cross-session: denied
        assert!(!mgr.can_signal(10, 20));
    }

    #[test]
    fn test_get_session_for_vt() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(1, "eve", 9).unwrap();
        assert_eq!(mgr.get_session_for_vt(9), Some(id));
        assert_eq!(mgr.get_session_for_vt(10), None);
    }

    #[test]
    fn test_list_sessions() {
        let mut mgr = SessionManager::new();
        let id1 = mgr.create_session(1, "a", 0).unwrap();
        let id2 = mgr.create_session(2, "b", 0).unwrap();
        let list = mgr.list_sessions();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&id1));
        assert!(list.contains(&id2));
    }

    #[test]
    fn test_begin_logout() {
        let mut mgr = SessionManager::new();
        let id = mgr.create_session(1, "frank", 0).unwrap();
        mgr.add_process(id, ProcessId(100));
        mgr.begin_logout(id);
        assert_eq!(mgr.get_session(id).unwrap().state, SessionState::LoggingOut);

        let pids = mgr.get_session_processes(id);
        assert_eq!(pids, vec![100u64]);
    }

    #[test]
    fn test_destroy_promotes_next() {
        let mut mgr = SessionManager::new();
        let id1 = mgr.create_session(1, "a", 0).unwrap();
        let id2 = mgr.create_session(2, "b", 0).unwrap();
        assert_eq!(mgr.get_active_session(), Some(id1));

        mgr.destroy_session(id1);
        // id2 should be promoted to active
        assert_eq!(mgr.get_active_session(), Some(id2));
        assert_eq!(mgr.get_session(id2).unwrap().state, SessionState::Active);
    }
}
