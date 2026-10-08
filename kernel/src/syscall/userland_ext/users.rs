//! User/Group Management
//!
//! The user database: names, user IDs, homes and shells, as /etc/passwd
//! holds them, and groups as /etc/group. Passwords are not here: they are
//! the account store's (`security::auth`, /etc/shadow), and
//! `security::accounts` loads and saves both (N-131).

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

use spin::RwLock;

use super::helpers::{parse_u32, push_u32_str};
use crate::sync::once_lock::GlobalState;

static USER_DB: GlobalState<RwLock<UserDatabase>> = GlobalState::new();

/// Initialize the global persistent user database.
pub fn init_user_db() {
    let _ = USER_DB.init(RwLock::new(UserDatabase::new()));
}

/// Access the global user database immutably.
pub fn with_user_db<R, F: FnOnce(&UserDatabase) -> R>(f: F) -> Option<R> {
    USER_DB.with(|lock| f(&lock.read()))
}

/// Access the global user database mutably.
pub fn with_user_db_mut<R, F: FnOnce(&mut UserDatabase) -> R>(f: F) -> Option<R> {
    USER_DB.with(|lock| f(&mut lock.write()))
}

// ============================================================================
// Constants
// ============================================================================

/// Maximum username length
const MAX_USERNAME_LEN: usize = 32;
/// Maximum group name length
const MAX_GROUPNAME_LEN: usize = 32;
/// Root UID
const ROOT_UID: u32 = 0;
/// Root GID
const ROOT_GID: u32 = 0;
/// Default shell path
const DEFAULT_SHELL: &str = "/bin/vsh";
/// Default home directory prefix
const DEFAULT_HOME_PREFIX: &str = "/home/";

// ============================================================================
// Error Types
// ============================================================================

/// User/group management errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserGroupError {
    /// User not found
    UserNotFound,
    /// Group not found
    GroupNotFound,
    /// User already exists
    UserExists,
    /// Group already exists
    GroupExists,
    /// Invalid UID
    InvalidUid,
    /// Invalid GID
    InvalidGid,
    /// Invalid username (empty, too long, bad chars)
    InvalidUsername,
    /// Invalid group name
    InvalidGroupName,
    /// Authentication failure
    AuthFailure,
    /// Permission denied
    PermissionDenied,
    /// Parse error in config file
    ParseError,
    /// Database full (max users/groups reached)
    DatabaseFull,
    /// Password hash mismatch
    PasswordMismatch,
}

// ============================================================================
// User Entry
// ============================================================================

/// User entry (equivalent to struct passwd)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserEntry {
    /// Username
    pub username: String,
    /// User ID
    pub uid: u32,
    /// Primary group ID
    pub gid: u32,
    /// Full name / comment (GECOS field)
    pub gecos: String,
    /// Home directory
    pub home: String,
    /// Login shell
    pub shell: String,
}

impl UserEntry {
    /// Create a new user entry
    pub fn new(username: &str, uid: u32, gid: u32) -> Self {
        let home = if uid == ROOT_UID {
            String::from("/root")
        } else {
            let mut h = String::from(DEFAULT_HOME_PREFIX);
            h.push_str(username);
            h
        };
        Self {
            username: String::from(username),
            uid,
            gid,
            gecos: String::new(),
            home,
            shell: String::from(DEFAULT_SHELL),
        }
    }

    /// Serialize to /etc/passwd format
    pub fn to_passwd_line(&self) -> String {
        let mut line = String::new();
        line.push_str(&self.username);
        line.push_str(":x:");
        push_u32_str(&mut line, self.uid);
        line.push(':');
        push_u32_str(&mut line, self.gid);
        line.push(':');
        line.push_str(&self.gecos);
        line.push(':');
        line.push_str(&self.home);
        line.push(':');
        line.push_str(&self.shell);
        line
    }

    /// Parse from /etc/passwd format line
    pub fn from_passwd_line(line: &str) -> Result<Self, UserGroupError> {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 7 {
            return Err(UserGroupError::ParseError);
        }
        let uid = parse_u32(parts[2]).ok_or(UserGroupError::ParseError)?;
        let gid = parse_u32(parts[3]).ok_or(UserGroupError::ParseError)?;
        Ok(Self {
            username: String::from(parts[0]),
            uid,
            gid,
            gecos: String::from(parts[4]),
            home: String::from(parts[5]),
            shell: String::from(parts[6]),
        })
    }
}

// ============================================================================
// Group Entry
// ============================================================================

/// Group entry (equivalent to struct group)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntry {
    /// Group name
    pub name: String,
    /// Group ID
    pub gid: u32,
    /// Member usernames
    pub members: Vec<String>,
}

impl GroupEntry {
    /// Create a new group entry
    pub fn new(name: &str, gid: u32) -> Self {
        Self {
            name: String::from(name),
            gid,
            members: Vec::new(),
        }
    }

    /// Add a member to this group
    pub fn add_member(&mut self, username: &str) {
        let name = String::from(username);
        if !self.members.contains(&name) {
            self.members.push(name);
        }
    }

    /// Remove a member from this group
    pub fn remove_member(&mut self, username: &str) {
        self.members.retain(|m| m.as_str() != username);
    }

    /// Serialize to /etc/group format
    pub fn to_group_line(&self) -> String {
        let mut line = String::new();
        line.push_str(&self.name);
        line.push_str(":x:");
        push_u32_str(&mut line, self.gid);
        line.push(':');
        for (i, member) in self.members.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            line.push_str(member);
        }
        line
    }

    /// Parse from /etc/group format line
    pub fn from_group_line(line: &str) -> Result<Self, UserGroupError> {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 3 {
            return Err(UserGroupError::ParseError);
        }
        let gid = parse_u32(if parts.len() > 2 { parts[2] } else { "0" })
            .ok_or(UserGroupError::ParseError)?;
        let members = if parts.len() > 3 && !parts[3].is_empty() {
            parts[3].split(',').map(String::from).collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            name: String::from(parts[0]),
            gid,
            members,
        })
    }
}

// ============================================================================
// User Database
// ============================================================================

/// User database (/etc/passwd)
#[derive(Debug)]
pub struct UserDatabase {
    /// User entries indexed by UID
    users: BTreeMap<u32, UserEntry>,
    /// Username to UID mapping
    name_to_uid: BTreeMap<String, u32>,
    /// Next available UID
    next_uid: u32,
}

impl Default for UserDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl UserDatabase {
    /// Create a new user database with root user
    pub fn new() -> Self {
        let mut db = Self {
            users: BTreeMap::new(),
            name_to_uid: BTreeMap::new(),
            next_uid: 1000,
        };
        // Always create root user
        let root = UserEntry {
            username: String::from("root"),
            uid: ROOT_UID,
            gid: ROOT_GID,
            gecos: String::from("root"),
            home: String::from("/root"),
            shell: String::from(DEFAULT_SHELL),
        };
        db.users.insert(ROOT_UID, root);
        db.name_to_uid.insert(String::from("root"), ROOT_UID);
        db
    }

    /// Validate a username
    fn validate_username(name: &str) -> Result<(), UserGroupError> {
        if name.is_empty() || name.len() > MAX_USERNAME_LEN {
            return Err(UserGroupError::InvalidUsername);
        }
        // Must start with a letter or underscore
        let first = name.as_bytes()[0];
        if !first.is_ascii_lowercase() && first != b'_' {
            return Err(UserGroupError::InvalidUsername);
        }
        // Only alphanumeric, underscore, hyphen, dot
        for &b in name.as_bytes() {
            if !b.is_ascii_alphanumeric() && b != b'_' && b != b'-' && b != b'.' {
                return Err(UserGroupError::InvalidUsername);
            }
        }
        Ok(())
    }

    /// Add a new user (useradd)
    pub fn add_user(
        &mut self,
        username: &str,
        gid: u32,
        uid: Option<u32>,
    ) -> Result<u32, UserGroupError> {
        Self::validate_username(username)?;
        if self.name_to_uid.contains_key(username) {
            return Err(UserGroupError::UserExists);
        }
        let uid = uid.unwrap_or_else(|| {
            let u = self.next_uid;
            self.next_uid += 1;
            u
        });
        if self.users.contains_key(&uid) {
            return Err(UserGroupError::InvalidUid);
        }
        let entry = UserEntry::new(username, uid, gid);
        self.users.insert(uid, entry);
        self.name_to_uid.insert(String::from(username), uid);
        if uid >= self.next_uid {
            self.next_uid = uid + 1;
        }
        Ok(uid)
    }

    /// Remove a user (userdel)
    pub fn remove_user(&mut self, username: &str) -> Result<(), UserGroupError> {
        let uid = self
            .name_to_uid
            .remove(username)
            .ok_or(UserGroupError::UserNotFound)?;
        self.users.remove(&uid);
        Ok(())
    }

    /// Look up user by UID
    pub fn get_user_by_uid(&self, uid: u32) -> Option<&UserEntry> {
        self.users.get(&uid)
    }

    /// Look up user by username (getpwnam)
    pub fn get_user_by_name(&self, name: &str) -> Option<&UserEntry> {
        self.name_to_uid
            .get(name)
            .and_then(|uid| self.users.get(uid))
    }

    /// Get total number of users
    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// Serialize to /etc/passwd format
    pub fn to_passwd_file(&self) -> String {
        let mut output = String::new();
        for user in self.users.values() {
            output.push_str(&user.to_passwd_line());
            output.push('\n');
        }
        output
    }

    /// Parse /etc/passwd file content
    pub fn load_passwd(&mut self, content: &str) -> Result<usize, UserGroupError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let entry = UserEntry::from_passwd_line(trimmed)?;
            self.name_to_uid.insert(entry.username.clone(), entry.uid);
            if entry.uid >= self.next_uid {
                self.next_uid = entry.uid + 1;
            }
            self.users.insert(entry.uid, entry);
            count += 1;
        }
        Ok(count)
    }

    /// The user ID useradd gives next: the lowest free one from 1000.
    pub fn next_free_uid(&self) -> u32 {
        (self.next_uid.max(1000)..)
            .find(|uid| !self.users.contains_key(uid))
            .unwrap_or(self.next_uid)
    }

    /// Every (name, user ID) pair, for the account store.
    pub fn name_uid_pairs(&self) -> Vec<(String, u32)> {
        self.users
            .values()
            .map(|u| (u.username.clone(), u.uid))
            .collect()
    }

    /// List all usernames
    pub fn list_usernames(&self) -> Vec<&str> {
        self.users.values().map(|u| u.username.as_str()).collect()
    }
}

// ============================================================================
// Group Database
// ============================================================================

/// Group database (manages /etc/group)
#[derive(Debug)]
pub struct GroupDatabase {
    /// Groups indexed by GID
    groups: BTreeMap<u32, GroupEntry>,
    /// Group name to GID mapping
    name_to_gid: BTreeMap<String, u32>,
    /// Next available GID
    next_gid: u32,
}

impl Default for GroupDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupDatabase {
    /// Create a new group database with root group
    pub fn new() -> Self {
        let mut db = Self {
            groups: BTreeMap::new(),
            name_to_gid: BTreeMap::new(),
            next_gid: 1000,
        };
        let root_group = GroupEntry::new("root", ROOT_GID);
        db.groups.insert(ROOT_GID, root_group);
        db.name_to_gid.insert(String::from("root"), ROOT_GID);
        db
    }

    /// Add a new group (groupadd)
    pub fn add_group(&mut self, name: &str, gid: Option<u32>) -> Result<u32, UserGroupError> {
        if name.is_empty() || name.len() > MAX_GROUPNAME_LEN {
            return Err(UserGroupError::InvalidGroupName);
        }
        if self.name_to_gid.contains_key(name) {
            return Err(UserGroupError::GroupExists);
        }
        let gid = gid.unwrap_or_else(|| {
            let g = self.next_gid;
            self.next_gid += 1;
            g
        });
        if self.groups.contains_key(&gid) {
            return Err(UserGroupError::InvalidGid);
        }
        let entry = GroupEntry::new(name, gid);
        self.groups.insert(gid, entry);
        self.name_to_gid.insert(String::from(name), gid);
        if gid >= self.next_gid {
            self.next_gid = gid + 1;
        }
        Ok(gid)
    }

    /// Remove a group (groupdel)
    pub fn remove_group(&mut self, name: &str) -> Result<(), UserGroupError> {
        let gid = self
            .name_to_gid
            .remove(name)
            .ok_or(UserGroupError::GroupNotFound)?;
        self.groups.remove(&gid);
        Ok(())
    }

    /// Look up group by GID
    pub fn get_group_by_gid(&self, gid: u32) -> Option<&GroupEntry> {
        self.groups.get(&gid)
    }

    /// Look up group by name (getgrnam)
    pub fn get_group_by_name(&self, name: &str) -> Option<&GroupEntry> {
        self.name_to_gid
            .get(name)
            .and_then(|gid| self.groups.get(gid))
    }

    /// Add a user to a group
    pub fn add_user_to_group(
        &mut self,
        username: &str,
        group_name: &str,
    ) -> Result<(), UserGroupError> {
        let gid = *self
            .name_to_gid
            .get(group_name)
            .ok_or(UserGroupError::GroupNotFound)?;
        let group = self
            .groups
            .get_mut(&gid)
            .ok_or(UserGroupError::GroupNotFound)?;
        group.add_member(username);
        Ok(())
    }

    /// Remove a user from a group
    pub fn remove_user_from_group(
        &mut self,
        username: &str,
        group_name: &str,
    ) -> Result<(), UserGroupError> {
        let gid = *self
            .name_to_gid
            .get(group_name)
            .ok_or(UserGroupError::GroupNotFound)?;
        let group = self
            .groups
            .get_mut(&gid)
            .ok_or(UserGroupError::GroupNotFound)?;
        group.remove_member(username);
        Ok(())
    }

    /// Get all groups a user belongs to
    pub fn get_user_groups(&self, username: &str) -> Vec<u32> {
        self.groups
            .values()
            .filter(|g| g.members.iter().any(|m| m.as_str() == username))
            .map(|g| g.gid)
            .collect()
    }

    /// Get total number of groups
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Serialize to /etc/group format
    pub fn to_group_file(&self) -> String {
        let mut output = String::new();
        for group in self.groups.values() {
            output.push_str(&group.to_group_line());
            output.push('\n');
        }
        output
    }

    /// Parse /etc/group file content
    pub fn load_group_file(&mut self, content: &str) -> Result<usize, UserGroupError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let entry = GroupEntry::from_group_line(trimmed)?;
            self.name_to_gid.insert(entry.name.clone(), entry.gid);
            if entry.gid >= self.next_gid {
                self.next_gid = entry.gid + 1;
            }
            self.groups.insert(entry.gid, entry);
            count += 1;
        }
        Ok(count)
    }
}
