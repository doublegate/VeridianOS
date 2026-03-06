//! Cloud-Init Service
//!
//! Provides instance metadata retrieval and user-data processing
//! for cloud environment initialization (hostname, users, SSH keys,
//! packages, commands, file creation).

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Metadata Service
// ---------------------------------------------------------------------------

/// Instance metadata source (link-local address 169.254.169.254).
#[derive(Debug)]
pub struct MetadataService {
    /// Base URL for the metadata service.
    base_url: String,
    /// Cached metadata.
    cache: BTreeMap<String, String>,
}

impl Default for MetadataService {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataService {
    /// Standard metadata service IP.
    pub const DEFAULT_BASE_URL: &'static str = "169.254.169.254";

    /// Create a new metadata service.
    pub fn new() -> Self {
        MetadataService {
            base_url: String::from(Self::DEFAULT_BASE_URL),
            cache: BTreeMap::new(),
        }
    }

    /// Create with a custom base URL.
    pub fn with_url(base_url: String) -> Self {
        MetadataService {
            base_url,
            cache: BTreeMap::new(),
        }
    }

    /// Get the instance ID.
    pub fn get_instance_id(&self) -> Option<&str> {
        self.cache.get("instance-id").map(|s| s.as_str())
    }

    /// Get the hostname.
    pub fn get_hostname(&self) -> Option<&str> {
        self.cache.get("hostname").map(|s| s.as_str())
    }

    /// Get public SSH keys.
    pub fn get_public_keys(&self) -> Vec<&str> {
        self.cache
            .iter()
            .filter(|(k, _)| k.starts_with("public-key-"))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// Get a metadata value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.cache.get(key).map(|s| s.as_str())
    }

    /// Set a metadata value (for testing or manual override).
    pub fn set(&mut self, key: String, value: String) {
        self.cache.insert(key, value);
    }

    /// Get the base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Simulate fetching metadata from the link-local service.
    ///
    /// In a real implementation this would make HTTP GET requests to
    /// `http://169.254.169.254/latest/meta-data/{path}`.
    pub fn fetch_metadata(&mut self) -> Result<(), CloudInitError> {
        // Populate default metadata for testing
        if self.cache.is_empty() {
            self.cache
                .insert(String::from("instance-id"), String::from("i-0000000000"));
            self.cache
                .insert(String::from("hostname"), String::from("veridian-node"));
            self.cache
                .insert(String::from("local-ipv4"), String::from("10.0.0.2"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// User Data Types
// ---------------------------------------------------------------------------

/// User account configuration.
#[derive(Debug, Clone)]
pub struct UserConfig {
    /// Username.
    pub name: String,
    /// Groups the user belongs to.
    pub groups: Vec<String>,
    /// SSH authorized keys.
    pub ssh_authorized_keys: Vec<String>,
    /// Login shell.
    pub shell: String,
    /// Sudo configuration (e.g., "ALL=(ALL) NOPASSWD:ALL").
    pub sudo: String,
}

impl Default for UserConfig {
    fn default() -> Self {
        UserConfig {
            name: String::new(),
            groups: Vec::new(),
            ssh_authorized_keys: Vec::new(),
            shell: String::from("/bin/sh"),
            sudo: String::new(),
        }
    }
}

/// File to write during cloud-init.
#[derive(Debug, Clone)]
pub struct WriteFile {
    /// Absolute file path.
    pub path: String,
    /// File content.
    pub content: String,
    /// File permissions (octal string, e.g., "0644").
    pub permissions: String,
    /// File owner (e.g., "root:root").
    pub owner: String,
}

impl Default for WriteFile {
    fn default() -> Self {
        WriteFile {
            path: String::new(),
            content: String::new(),
            permissions: String::from("0644"),
            owner: String::from("root:root"),
        }
    }
}

/// User data configuration.
#[derive(Debug, Clone, Default)]
pub struct UserData {
    /// Desired hostname.
    pub hostname: String,
    /// Users to create.
    pub users: Vec<UserConfig>,
    /// SSH keys to install (global).
    pub ssh_keys: Vec<String>,
    /// Packages to install.
    pub packages: Vec<String>,
    /// Commands to run.
    pub runcmd: Vec<String>,
    /// Files to write.
    pub write_files: Vec<WriteFile>,
}

impl UserData {
    /// Parse user data from a simple key=value format.
    ///
    /// This is a simplified parser. Real cloud-init uses YAML.
    /// Recognized keys:
    /// - `hostname=value`
    /// - `ssh_key=value`
    /// - `package=value`
    /// - `runcmd=value`
    /// - `user=name:groups:shell:sudo`
    /// - `write_file=path:permissions:owner:content`
    pub fn from_key_value(input: &str) -> Self {
        let mut data = UserData::default();

        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "hostname" => data.hostname = String::from(value),
                    "ssh_key" => data.ssh_keys.push(String::from(value)),
                    "package" => data.packages.push(String::from(value)),
                    "runcmd" => data.runcmd.push(String::from(value)),
                    "user" => {
                        let parts: Vec<&str> = value.splitn(4, ':').collect();
                        let mut user = UserConfig::default();
                        if !parts.is_empty() {
                            user.name = String::from(parts[0]);
                        }
                        if parts.len() > 1 && !parts[1].is_empty() {
                            user.groups = parts[1]
                                .split(',')
                                .map(|g| String::from(g.trim()))
                                .collect();
                        }
                        if parts.len() > 2 && !parts[2].is_empty() {
                            user.shell = String::from(parts[2]);
                        }
                        if parts.len() > 3 {
                            user.sudo = String::from(parts[3]);
                        }
                        data.users.push(user);
                    }
                    "write_file" => {
                        let parts: Vec<&str> = value.splitn(4, ':').collect();
                        let mut file = WriteFile::default();
                        if !parts.is_empty() {
                            file.path = String::from(parts[0]);
                        }
                        if parts.len() > 1 && !parts[1].is_empty() {
                            file.permissions = String::from(parts[1]);
                        }
                        if parts.len() > 2 && !parts[2].is_empty() {
                            file.owner = String::from(parts[2]);
                        }
                        if parts.len() > 3 {
                            file.content = String::from(parts[3]);
                        }
                        data.write_files.push(file);
                    }
                    _ => {}
                }
            }
        }

        data
    }
}

// ---------------------------------------------------------------------------
// Cloud-Init Error
// ---------------------------------------------------------------------------

/// Cloud-init error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudInitError {
    /// Metadata service unreachable.
    MetadataUnavailable,
    /// User data fetch failed.
    UserDataFetchFailed,
    /// Failed to apply hostname.
    HostnameApplyFailed(String),
    /// Failed to create user.
    UserCreateFailed(String),
    /// Failed to write file.
    WriteFileFailed(String),
    /// Command execution failed.
    CommandFailed(String),
    /// Package installation failed.
    PackageInstallFailed(String),
}

// ---------------------------------------------------------------------------
// Cloud-Init Runner
// ---------------------------------------------------------------------------

/// Execution log entry.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Stage name.
    pub stage: String,
    /// Success or failure.
    pub success: bool,
    /// Detail message.
    pub detail: String,
}

/// Cloud-Init runner: orchestrates the full initialization sequence.
#[derive(Debug)]
pub struct CloudInitRunner {
    /// Metadata service.
    metadata: MetadataService,
    /// Parsed user data.
    user_data: Option<UserData>,
    /// Execution log.
    log: Vec<LogEntry>,
    /// Whether cloud-init has already run.
    completed: bool,
    /// Applied hostname.
    applied_hostname: Option<String>,
    /// Created users.
    created_users: Vec<String>,
    /// Written files.
    written_files: Vec<String>,
    /// Executed commands.
    executed_commands: Vec<String>,
    /// Installed packages.
    installed_packages: Vec<String>,
}

impl Default for CloudInitRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudInitRunner {
    /// Create a new cloud-init runner.
    pub fn new() -> Self {
        CloudInitRunner {
            metadata: MetadataService::new(),
            user_data: None,
            log: Vec::new(),
            completed: false,
            applied_hostname: None,
            created_users: Vec::new(),
            written_files: Vec::new(),
            executed_commands: Vec::new(),
            installed_packages: Vec::new(),
        }
    }

    /// Create with custom metadata service.
    pub fn with_metadata(metadata: MetadataService) -> Self {
        CloudInitRunner {
            metadata,
            user_data: None,
            log: Vec::new(),
            completed: false,
            applied_hostname: None,
            created_users: Vec::new(),
            written_files: Vec::new(),
            executed_commands: Vec::new(),
            installed_packages: Vec::new(),
        }
    }

    /// Fetch metadata from the metadata service.
    pub fn fetch_metadata(&mut self) -> Result<(), CloudInitError> {
        self.metadata.fetch_metadata()?;
        self.log.push(LogEntry {
            stage: String::from("fetch_metadata"),
            success: true,
            detail: String::from("metadata fetched"),
        });
        Ok(())
    }

    /// Fetch and parse user data.
    pub fn fetch_userdata(&mut self, raw_data: &str) -> Result<(), CloudInitError> {
        let data = UserData::from_key_value(raw_data);
        self.user_data = Some(data);
        self.log.push(LogEntry {
            stage: String::from("fetch_userdata"),
            success: true,
            detail: String::from("userdata parsed"),
        });
        Ok(())
    }

    /// Apply the configured hostname.
    pub fn apply_hostname(&mut self) -> Result<(), CloudInitError> {
        let hostname = if let Some(ref data) = self.user_data {
            if !data.hostname.is_empty() {
                data.hostname.clone()
            } else {
                self.metadata
                    .get_hostname()
                    .map(String::from)
                    .unwrap_or_else(|| String::from("veridian"))
            }
        } else {
            self.metadata
                .get_hostname()
                .map(String::from)
                .unwrap_or_else(|| String::from("veridian"))
        };

        self.applied_hostname = Some(hostname.clone());
        self.log.push(LogEntry {
            stage: String::from("apply_hostname"),
            success: true,
            detail: hostname,
        });
        Ok(())
    }

    /// Create user accounts.
    pub fn create_users(&mut self) -> Result<(), CloudInitError> {
        let users = match &self.user_data {
            Some(data) => data.users.clone(),
            None => return Ok(()),
        };

        for user in &users {
            if user.name.is_empty() {
                continue;
            }
            self.created_users.push(user.name.clone());
            self.log.push(LogEntry {
                stage: String::from("create_users"),
                success: true,
                detail: alloc::format!("created user: {}", user.name),
            });
        }
        Ok(())
    }

    /// Install SSH keys.
    pub fn install_ssh_keys(&mut self) -> Result<(), CloudInitError> {
        let keys = match &self.user_data {
            Some(data) => &data.ssh_keys,
            None => return Ok(()),
        };

        for key in keys {
            self.log.push(LogEntry {
                stage: String::from("install_ssh_keys"),
                success: true,
                detail: alloc::format!("installed key: {}...", &key[..key.len().min(20)]),
            });
        }
        Ok(())
    }

    /// Run commands.
    pub fn run_commands(&mut self) -> Result<(), CloudInitError> {
        let commands = match &self.user_data {
            Some(data) => data.runcmd.clone(),
            None => return Ok(()),
        };

        for cmd in &commands {
            self.executed_commands.push(cmd.clone());
            self.log.push(LogEntry {
                stage: String::from("run_commands"),
                success: true,
                detail: alloc::format!("ran: {}", cmd),
            });
        }
        Ok(())
    }

    /// Write files.
    pub fn write_files(&mut self) -> Result<(), CloudInitError> {
        let files = match &self.user_data {
            Some(data) => data.write_files.clone(),
            None => return Ok(()),
        };

        for file in &files {
            if file.path.is_empty() {
                continue;
            }
            self.written_files.push(file.path.clone());
            self.log.push(LogEntry {
                stage: String::from("write_files"),
                success: true,
                detail: alloc::format!("wrote: {} ({} bytes)", file.path, file.content.len()),
            });
        }
        Ok(())
    }

    /// Install packages.
    pub fn install_packages(&mut self) -> Result<(), CloudInitError> {
        let packages = match &self.user_data {
            Some(data) => data.packages.clone(),
            None => return Ok(()),
        };

        for pkg in &packages {
            self.installed_packages.push(pkg.clone());
            self.log.push(LogEntry {
                stage: String::from("install_packages"),
                success: true,
                detail: alloc::format!("installed: {}", pkg),
            });
        }
        Ok(())
    }

    /// Execute the full cloud-init sequence.
    pub fn execute(&mut self, raw_userdata: &str) -> Result<(), CloudInitError> {
        if self.completed {
            return Ok(());
        }

        self.fetch_metadata()?;
        self.fetch_userdata(raw_userdata)?;
        self.apply_hostname()?;
        self.create_users()?;
        self.install_ssh_keys()?;
        self.write_files()?;
        self.install_packages()?;
        self.run_commands()?;

        self.completed = true;
        self.log.push(LogEntry {
            stage: String::from("complete"),
            success: true,
            detail: String::from("cloud-init complete"),
        });
        Ok(())
    }

    /// Get the execution log.
    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    /// Check if cloud-init has completed.
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    /// Get the applied hostname.
    pub fn applied_hostname(&self) -> Option<&str> {
        self.applied_hostname.as_deref()
    }

    /// Get created users.
    pub fn created_users(&self) -> &[String] {
        &self.created_users
    }

    /// Get written files.
    pub fn written_files(&self) -> &[String] {
        &self.written_files
    }

    /// Get executed commands.
    pub fn executed_commands(&self) -> &[String] {
        &self.executed_commands
    }

    /// Get installed packages.
    pub fn installed_packages(&self) -> &[String] {
        &self.installed_packages
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn test_metadata_service_default() {
        let mut svc = MetadataService::new();
        svc.fetch_metadata().unwrap();
        assert_eq!(svc.get_instance_id(), Some("i-0000000000"));
        assert_eq!(svc.get_hostname(), Some("veridian-node"));
    }

    #[test]
    fn test_metadata_set_get() {
        let mut svc = MetadataService::new();
        svc.set(String::from("custom-key"), String::from("custom-val"));
        assert_eq!(svc.get("custom-key"), Some("custom-val"));
    }

    #[test]
    fn test_metadata_public_keys() {
        let mut svc = MetadataService::new();
        svc.set(
            String::from("public-key-0"),
            String::from("ssh-rsa AAAA..."),
        );
        svc.set(
            String::from("public-key-1"),
            String::from("ssh-ed25519 BBBB..."),
        );
        let keys = svc.get_public_keys();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_userdata_parse() {
        let input = "\
hostname=my-node
ssh_key=ssh-rsa AAAAB3...
package=nginx
package=vim
runcmd=echo hello
user=admin:sudo,docker:/bin/bash:ALL=(ALL) NOPASSWD:ALL
write_file=/etc/motd:0644:root:Welcome to VeridianOS
";
        let data = UserData::from_key_value(input);
        assert_eq!(data.hostname, "my-node");
        assert_eq!(data.ssh_keys.len(), 1);
        assert_eq!(data.packages.len(), 2);
        assert_eq!(data.runcmd.len(), 1);
        assert_eq!(data.users.len(), 1);
        assert_eq!(data.users[0].name, "admin");
        assert_eq!(data.users[0].groups, ["sudo", "docker"]);
        assert_eq!(data.users[0].shell, "/bin/bash");
        assert_eq!(data.write_files.len(), 1);
        assert_eq!(data.write_files[0].path, "/etc/motd");
    }

    #[test]
    fn test_userdata_empty() {
        let data = UserData::from_key_value("");
        assert!(data.hostname.is_empty());
        assert!(data.users.is_empty());
    }

    #[test]
    fn test_userdata_comments() {
        let input = "# comment\nhostname=test\n# another\n";
        let data = UserData::from_key_value(input);
        assert_eq!(data.hostname, "test");
    }

    #[test]
    fn test_runner_execute() {
        let mut runner = CloudInitRunner::new();
        let userdata = "hostname=cloud-node\nuser=admin:::\npackage=curl\nruncmd=uname -a\n";
        runner.execute(userdata).unwrap();
        assert!(runner.is_completed());
        assert_eq!(runner.applied_hostname(), Some("cloud-node"));
        assert_eq!(runner.created_users(), &["admin"]);
        assert_eq!(runner.installed_packages(), &["curl"]);
        assert_eq!(runner.executed_commands(), &["uname -a"]);
    }

    #[test]
    fn test_runner_idempotent() {
        let mut runner = CloudInitRunner::new();
        runner.execute("hostname=test").unwrap();
        let log_len = runner.log().len();
        runner.execute("hostname=test").unwrap(); // should be no-op
        assert_eq!(runner.log().len(), log_len);
    }

    #[test]
    fn test_runner_apply_hostname_from_metadata() {
        let mut runner = CloudInitRunner::new();
        runner.fetch_metadata().unwrap();
        runner.apply_hostname().unwrap();
        assert_eq!(runner.applied_hostname(), Some("veridian-node"));
    }

    #[test]
    fn test_runner_write_files() {
        let mut runner = CloudInitRunner::new();
        let userdata = "write_file=/etc/test:0755:root:file content\n";
        runner.fetch_metadata().unwrap();
        runner.fetch_userdata(userdata).unwrap();
        runner.write_files().unwrap();
        assert_eq!(runner.written_files(), &["/etc/test"]);
    }

    #[test]
    fn test_runner_no_userdata() {
        let mut runner = CloudInitRunner::new();
        runner.fetch_metadata().unwrap();
        // These should all succeed with no user data
        runner.apply_hostname().unwrap();
        runner.create_users().unwrap();
        runner.install_ssh_keys().unwrap();
        runner.write_files().unwrap();
        runner.run_commands().unwrap();
        assert!(runner.created_users().is_empty());
    }

    #[test]
    fn test_runner_install_ssh_keys() {
        let mut runner = CloudInitRunner::new();
        let userdata = "ssh_key=ssh-rsa AAAAB3NzaC1yc2E\nssh_key=ssh-ed25519 AABBCC\n";
        runner.fetch_metadata().unwrap();
        runner.fetch_userdata(userdata).unwrap();
        runner.install_ssh_keys().unwrap();
        // Check log has entries
        let key_logs: Vec<_> = runner
            .log()
            .iter()
            .filter(|e| e.stage == "install_ssh_keys")
            .collect();
        assert_eq!(key_logs.len(), 2);
    }

    #[test]
    fn test_write_file_default() {
        let file = WriteFile::default();
        assert_eq!(file.permissions, "0644");
        assert_eq!(file.owner, "root:root");
    }

    #[test]
    fn test_user_config_default() {
        let user = UserConfig::default();
        assert_eq!(user.shell, "/bin/sh");
        assert!(user.groups.is_empty());
    }
}
